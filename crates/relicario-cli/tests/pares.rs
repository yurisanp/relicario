//! Duas máquinas com repositórios próprios, pareadas pela CLI e sincronizando pelo daemon.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn cmd(cfg: &Path, port: u16) -> Command {
    let home = cfg.join("home");
    fs::create_dir_all(&home).unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_relicario"));
    c.env("RELICARIO_CONFIG_DIR", cfg)
        .env("RELICARIO_DATA_DIR", cfg.join("data"))
        .env("RELICARIO_CACHE_DIR", cfg.join("cache"))
        .env("RELICARIO_NO_NOTIFY", "1")
        .env("RELICARIO_SETTLE_MS", "100")
        .env("RELICARIO_POLL_MS", "200")
        .env("RELICARIO_P2P_PORT", port.to_string())
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("APPDATA")
        .env_remove("LOCALAPPDATA");
    c
}

fn ok(cfg: &Path, port: u16, args: &[&str]) -> String {
    let out = cmd(cfg, port).args(args).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "relicario {args:?} falhou:\n{text}");
    text
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn parear_pela_cli_e_sincronizar_pelo_daemon() {
    let t = tempfile::tempdir().unwrap();
    let (pc, note) = (t.path().join("pc"), t.path().join("note"));
    let (save_pc, save_note) = (t.path().join("save-pc"), t.path().join("save-note"));
    fs::create_dir_all(&save_pc).unwrap();
    fs::create_dir_all(&save_note).unwrap();
    let (port_pc, port_note) = (free_port(), free_port());
    // Cada máquina com o próprio repositório.
    for (cfg, port, name, save) in [
        (&pc, port_pc, "PC", &save_pc),
        (&note, port_note, "Notebook", &save_note),
    ] {
        let repo = cfg.join("repo");
        ok(
            cfg,
            port,
            &["init", "--repo", repo.to_str().unwrap(), "--name", name],
        );
        ok(
            cfg,
            port,
            &["game", "add", "hookshot", "--path", save.to_str().unwrap()],
        );
    }

    // O PC mostra o código; o notebook entra com ele.
    let mut host = Kill(
        cmd(&pc, port_pc)
            .args(["pair", "--wait", "30"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut out = BufReader::new(host.0.stdout.take().unwrap());
    let mut code = String::new();
    let start = Instant::now();
    while code.is_empty() {
        let mut line = String::new();
        assert!(
            out.read_line(&mut line).unwrap() > 0,
            "pair terminou sem mostrar o código"
        );
        if let Some(rest) = line.trim().strip_prefix("relicario pair ") {
            code = rest.replace(' ', "");
        }
        assert!(start.elapsed() < Duration::from_secs(20));
    }
    let addr = format!("127.0.0.1:{port_pc}");
    let j = ok(&note, port_note, &["pair", &code, "--addr", &addr]);
    assert!(j.contains("pareado com PC"), "{j}");
    assert!(host.0.wait().unwrap().success());
    assert!(ok(&pc, port_pc, &["peers", "list"]).contains("Notebook"));
    assert!(ok(&note, port_note, &["peers", "list"]).contains("PC"));

    // Joga no PC com o daemon rodando; o notebook sincroniza e recebe.
    let _d = Kill(cmd(&pc, port_pc).args(["daemon", "run"]).spawn().unwrap());
    let start = Instant::now();
    while !cmd(&pc, port_pc)
        .args(["daemon", "status"])
        .output()
        .unwrap()
        .status
        .success()
    {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "daemon não subiu"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    fs::write(save_pc.join("caption.sav"), "capitulo 1").unwrap();
    ok(&pc, port_pc, &["snapshot"]);
    let s = ok(&note, port_note, &["sync"]);
    assert!(s.contains("PC:"), "{s}");
    assert_eq!(
        fs::read_to_string(save_note.join("caption.sav")).unwrap(),
        "capitulo 1"
    );

    // O notebook joga; o daemon do PC recebe o head e traz o save sozinho.
    fs::write(save_note.join("caption.sav"), "capitulo 2").unwrap();
    ok(&note, port_note, &["snapshot"]);
    ok(&note, port_note, &["peers", "sync"]);
    let start = Instant::now();
    while fs::read_to_string(save_pc.join("caption.sav")).unwrap() != "capitulo 2" {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "o PC não trouxe o save do notebook"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}
