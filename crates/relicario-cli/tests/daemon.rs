//! M2 "Funciona sozinho": `relicario run` e o daemon vigiando um jogo de mentira.

use std::fs;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn cmd(cfg: &Path) -> Command {
    let home = cfg.join("home");
    fs::create_dir_all(&home).unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_relicario"));
    c.env("RELICARIO_CONFIG_DIR", cfg)
        .env("RELICARIO_DATA_DIR", cfg.join("data"))
        .env("RELICARIO_CACHE_DIR", cfg.join("cache"))
        .env("RELICARIO_NO_NOTIFY", "1")
        .env("RELICARIO_SETTLE_MS", "100")
        .env("RELICARIO_POLL_MS", "100")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("SteamAppId")
        .env_remove("SteamGameId")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("APPDATA")
        .env_remove("LOCALAPPDATA");
    c
}

fn ok(cfg: &Path, args: &[&str]) -> String {
    let out = cmd(cfg).args(args).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "relicario {args:?} falhou:\n{text}");
    text
}

fn setup(t: &Path, name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let cfg = t.join(format!("cfg-{name}"));
    let save = t.join(format!("save-{name}"));
    fs::create_dir_all(&save).unwrap();
    let repo = t.join("repo");
    ok(
        &cfg,
        &["init", "--repo", repo.to_str().unwrap(), "--name", name],
    );
    ok(
        &cfg,
        &[
            "game",
            "add",
            "hookshot",
            "--name",
            "Hookshot",
            "--path",
            save.to_str().unwrap(),
        ],
    );
    (cfg, save)
}

/// Comando que imita o jogo: copia o save atual para `lido.txt` e grava um save novo.
fn fake_game(save: &Path, new: &str) -> Vec<String> {
    let s = save.join("caption.sav");
    let lido = save.parent().unwrap().join("lido.txt");
    if cfg!(windows) {
        vec![
            "cmd".into(),
            "/C".into(),
            format!(
                "type \"{}\" > \"{}\" & echo {new}> \"{}\"",
                s.display(),
                lido.display(),
                s.display()
            ),
        ]
    } else {
        vec![
            "sh".into(),
            "-c".into(),
            format!(
                "cat '{}' > '{}'; printf '{new}' > '{}'",
                s.display(),
                lido.display(),
                s.display()
            ),
        ]
    }
}

#[test]
fn run_traz_o_save_antes_e_guarda_depois() {
    let t = tempfile::tempdir().unwrap();
    let (pc, save_pc) = setup(t.path(), "PC");
    let (note, save_note) = setup(t.path(), "Notebook");
    fs::write(save_pc.join("caption.sav"), "capitulo 1").unwrap();
    ok(&pc, &["snapshot"]);

    // No notebook, o jogo abre pelo `run`: o save do PC chega antes de o jogo ler.
    let mut args = vec!["run", "--game", "hookshot", "--"];
    let game = fake_game(&save_note, "capitulo 2");
    args.extend(game.iter().map(String::as_str));
    let out = ok(&note, &args);
    assert!(out.contains("trouxe"), "{out}");
    let lido = fs::read_to_string(t.path().join("lido.txt")).unwrap();
    assert_eq!(lido.trim(), "capitulo 1");

    // Ao fechar, o save novo foi guardado e o PC vê o notebook à frente.
    assert!(ok(&note, &["log", "hookshot"]).contains("GameExit"));
    assert!(ok(&pc, &["plan"]).contains("Notebook está à frente"));
    ok(&pc, &["sync"]);
    assert_eq!(
        fs::read_to_string(save_pc.join("caption.sav"))
            .unwrap()
            .trim(),
        "capitulo 2"
    );
}

#[test]
fn run_sem_configuracao_ainda_abre_o_jogo() {
    let t = tempfile::tempdir().unwrap();
    let cfg = t.path().join("vazio");
    let save = t.path().join("s");
    fs::create_dir_all(&save).unwrap();
    fs::write(save.join("caption.sav"), "x").unwrap();
    let mut args = vec!["run", "--"];
    let game = fake_game(&save, "y");
    args.extend(game.iter().map(String::as_str));
    let out = ok(&cfg, &args);
    assert!(out.contains("sem sincronizar"), "{out}");
    assert_eq!(
        fs::read_to_string(save.join("caption.sav")).unwrap().trim(),
        "y"
    );
}

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for(what: &str, timeout: Duration, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < timeout, "esperei demais: {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// O daemon reconhece o jogo pelo nome do executável (associado à mão), guarda o save quando o
/// processo fecha e soma o tempo de jogo.
#[cfg(target_os = "linux")]
#[test]
fn daemon_guarda_quando_o_jogo_fecha() {
    let t = tempfile::tempdir().unwrap();
    let (pc, save) = setup(t.path(), "PC");
    fs::write(save.join("caption.sav"), "capitulo 1").unwrap();
    let exe = t.path().join("hookshot-fake");
    fs::copy("/bin/sleep", &exe).unwrap();
    ok(&pc, &["associate", "hookshot", "--exe", "hookshot-fake"]);

    let _d = Daemon(cmd(&pc).args(["daemon", "run"]).spawn().unwrap());
    wait_for("daemon subir", Duration::from_secs(20), || {
        cmd(&pc)
            .args(["daemon", "status"])
            .output()
            .unwrap()
            .status
            .success()
    });

    let mut game = Command::new(&exe).arg("2").spawn().unwrap();
    wait_for("daemon ver o jogo", Duration::from_secs(10), || {
        ok(&pc, &["daemon", "status"]).contains("abertos: hookshot")
    });
    let refused = cmd(&pc).args(["restore", "hookshot"]).output().unwrap();
    assert!(
        !refused.status.success(),
        "restaurar com o jogo aberto deveria falhar"
    );
    fs::write(save.join("caption.sav"), "capitulo 2").unwrap();
    game.wait().unwrap();

    wait_for("snapshot de fim de jogo", Duration::from_secs(20), || {
        ok(&pc, &["log", "hookshot"]).contains("GameExit")
    });
    assert!(ok(&pc, &["daemon", "status"]).contains("nenhum jogo aberto"));
    ok(&pc, &["daemon", "stop"]);
}
