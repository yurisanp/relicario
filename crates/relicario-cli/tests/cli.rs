//! M1 "Funciona na CLI": duas máquinas simuladas com pastas de configuração separadas.

use std::fs;
use std::path::Path;
use std::process::Command;

fn run(cfg: &Path, args: &[&str]) -> (i32, String) {
    // Cada "máquina" tem a própria pasta de usuário, dados e cache.
    let home = cfg.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_relicario"))
        .args(args)
        .env("RELICARIO_CONFIG_DIR", cfg)
        .env("RELICARIO_DATA_DIR", cfg.join("data"))
        .env("RELICARIO_CACHE_DIR", cfg.join("cache"))
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("APPDATA")
        .env_remove("LOCALAPPDATA")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

fn ok(cfg: &Path, args: &[&str]) -> String {
    let (code, text) = run(cfg, args);
    assert_eq!(code, 0, "relicario {args:?} falhou:\n{text}");
    text
}

#[test]
fn hookshot_entre_pc_e_notebook() {
    let t = tempfile::tempdir().unwrap();
    let repo = t.path().join("repo");
    let (pc, note) = (t.path().join("cfg-pc"), t.path().join("cfg-note"));
    let (save_pc, save_note) = (t.path().join("hookshot-pc"), t.path().join("hookshot-note"));
    fs::create_dir_all(save_pc.join("save")).unwrap();
    fs::write(save_pc.join("save/caption.sav"), b"capitulo 1").unwrap();
    fs::write(save_pc.join("hookshot_options.ini"), b"volume=10").unwrap();
    let r = repo.to_str().unwrap();

    ok(&pc, &["init", "--repo", r, "--name", "PC"]);
    ok(&note, &["init", "--repo", r, "--name", "Notebook"]);
    for (cfg, save) in [(&pc, &save_pc), (&note, &save_note)] {
        ok(
            cfg,
            &[
                "game",
                "add",
                "hookshot",
                "--name",
                "Hookshot",
                "--path",
                save.to_str().unwrap(),
                "--exclude",
                "save-backups/**",
                "--exclude",
                "hookshot_options.ini",
            ],
        );
    }

    assert!(ok(&pc, &["snapshot"]).contains("snapshot"));
    assert!(ok(&pc, &["snapshot", "hookshot"]).contains("sem mudanças"));
    assert!(ok(&note, &["plan"]).contains("PC está à frente"));
    ok(&note, &["sync"]);
    assert_eq!(
        fs::read(save_note.join("save/caption.sav")).unwrap(),
        b"capitulo 1"
    );
    assert!(
        !save_note.join("hookshot_options.ini").exists(),
        "arquivo ignorado não viaja"
    );

    // Joga no notebook e volta para o PC.
    fs::write(save_note.join("save/caption.sav"), b"capitulo 2").unwrap();
    ok(&note, &["sync"]);
    assert!(ok(&pc, &["plan"]).contains("Notebook está à frente"));
    ok(&pc, &["sync"]);
    assert_eq!(
        fs::read(save_pc.join("save/caption.sav")).unwrap(),
        b"capitulo 2"
    );
    assert_eq!(
        fs::read(save_pc.join("hookshot_options.ini")).unwrap(),
        b"volume=10"
    );

    // Os dois jogam sem sincronizar: conflito, e nada é sobrescrito.
    fs::write(save_pc.join("save/caption.sav"), b"capitulo 3 pc").unwrap();
    fs::write(save_note.join("save/caption.sav"), b"capitulo 3 note").unwrap();
    ok(&pc, &["sync"]);
    let (code, text) = run(&note, &["sync"]);
    assert_eq!(code, 4, "{text}");
    assert!(text.contains("CONFLITO"));
    assert_eq!(
        fs::read(save_note.join("save/caption.sav")).unwrap(),
        b"capitulo 3 note"
    );

    // Resolve escolhendo o do PC pelo log; o do notebook fica no histórico.
    let log = ok(&note, &["log", "hookshot"]);
    let pc_head = log.lines().find(|l| l.contains("head de PC")).unwrap();
    let id = pc_head.split_whitespace().next().unwrap();
    let out = ok(&note, &["restore", "hookshot", id]);
    assert_eq!(
        fs::read(save_note.join("save/caption.sav")).unwrap(),
        b"capitulo 3 pc"
    );
    assert!(
        !out.contains("cópia de segurança"),
        "o estado do note já estava guardado pelo sync"
    );

    // Pin, verify e prune.
    let note_snap = ok(&note, &["log", "hookshot"])
        .lines()
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    ok(&note, &["pin", &note_snap, "--name", "final alternativo"]);
    assert!(ok(&note, &["log", "hookshot"]).contains("fixado: final alternativo"));
    assert!(ok(&pc, &["verify", "--read-data"]).contains("repositório íntegro"));
    ok(&pc, &["prune"]);
    assert!(ok(&note, &["verify", "--read-data"]).contains("repositório íntegro"));
    assert!(ok(&pc, &["doctor"]).contains("repositório abre: sim"));
}

#[test]
fn save_corrompido_nao_e_promovido() {
    let t = tempfile::tempdir().unwrap();
    let cfg = t.path().join("cfg");
    let save = t.path().join("save");
    fs::create_dir_all(&save).unwrap();
    fs::write(save.join("a.sav"), vec![7u8; 50_000]).unwrap();
    ok(
        &cfg,
        &["init", "--repo", t.path().join("repo").to_str().unwrap()],
    );
    ok(
        &cfg,
        &["game", "add", "jogo", "--path", save.to_str().unwrap()],
    );
    ok(&cfg, &["snapshot"]);
    fs::write(save.join("a.sav"), vec![0u8; 50_000]).unwrap();
    let (code, text) = run(&cfg, &["snapshot"]);
    assert_eq!(code, 3, "{text}");
    assert!(text.contains("virou só zeros"));
    ok(&cfg, &["snapshot", "--force"]);
}

#[test]
fn scan_encontra_jogo_pelo_manifesto_e_emulador() {
    let t = tempfile::tempdir().unwrap();
    let cfg = t.path().join("cfg");
    let home = cfg.join("home");
    // Manifesto mínimo já em cache.
    let manifest = r#"{"Jogo Teste":{"files":{"<home>/JogoTeste/saves":{"tags":["save"]}}}}"#;
    fs::create_dir_all(cfg.join("cache/manifest")).unwrap();
    fs::write(cfg.join("cache/manifest/manifest.json"), manifest).unwrap();
    fs::create_dir_all(home.join("JogoTeste/saves")).unwrap();
    fs::write(home.join("JogoTeste/saves/slot1.dat"), b"progresso").unwrap();
    let ra = if cfg!(windows) {
        "AppData/Roaming/RetroArch/saves"
    } else if cfg!(target_os = "macos") {
        "Library/Application Support/RetroArch/saves"
    } else {
        ".config/retroarch/saves"
    };
    fs::create_dir_all(home.join(ra)).unwrap();
    fs::write(home.join(ra).join("zelda.srm"), b"z").unwrap();

    ok(
        &cfg,
        &[
            "init",
            "--repo",
            t.path().join("repo").to_str().unwrap(),
            "--name",
            "PC",
        ],
    );
    let out = ok(&cfg, &["scan", "--offline"]);
    assert!(out.contains("Jogo Teste (jogo-teste)"), "{out}");
    assert!(out.contains("RetroArch (retroarch)"), "{out}");
    let list = ok(&cfg, &["game", "list"]);
    assert!(list.contains("jogo-teste"), "{list}");
    ok(&cfg, &["snapshot", "jogo-teste"]);
    assert!(ok(&cfg, &["log", "jogo-teste"]).contains("head de PC"));
    ok(&cfg, &["game", "ignore", "retroarch"]);
    assert!(!ok(&cfg, &["game", "list"]).contains("retroarch"));
    let art = ok(&cfg, &["art", "jogo-teste"]);
    assert!(art.contains("fonte: gerada"), "{art}");
    let json = ok(&cfg, &["scan", "--offline", "--json"]);
    assert!(json.trim_start().starts_with('['));
}
