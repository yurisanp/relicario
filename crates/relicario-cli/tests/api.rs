//! O que a interface pede ao daemon (pelo `relicario api`, que usa o mesmo caminho).

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

fn cmd(cfg: &Path) -> Command {
    let home = cfg.join("home");
    fs::create_dir_all(&home).unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_relicario"));
    c.env("RELICARIO_CONFIG_DIR", cfg)
        .env("RELICARIO_DATA_DIR", cfg.join("data"))
        .env("RELICARIO_CACHE_DIR", cfg.join("cache"))
        .env("RELICARIO_NO_NOTIFY", "1")
        .env("RELICARIO_NO_KEYRING", "1")
        .env("RELICARIO_P2P_PORT", "0")
        .env("RELICARIO_OFFLINE", "1")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("APPDATA")
        .env_remove("LOCALAPPDATA");
    c
}

fn ok(cfg: &Path, args: &[&str]) -> String {
    let out = cmd(cfg).args(args).output().unwrap();
    let t = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "relicario {args:?} falhou:\n{t}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn api(cfg: &Path, call: Value) -> Value {
    let out = ok(cfg, &["api", &call.to_string()]);
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"))
}

fn api_err(cfg: &Path, call: Value) -> String {
    let out = cmd(cfg).args(["api", &call.to_string()]).output().unwrap();
    assert!(!out.status.success());
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn biblioteca_conflito_resolucao_e_atividade() {
    let t = tempfile::tempdir().unwrap();
    let (pc, note, nuvem) = (
        t.path().join("pc"),
        t.path().join("note"),
        t.path().join("nuvem"),
    );
    let (save_pc, save_note) = (t.path().join("save-pc"), t.path().join("save-note"));
    fs::create_dir_all(save_pc.join("slot1")).unwrap();
    fs::create_dir_all(save_note.join("slot1")).unwrap();
    for (cfg, save, name) in [(&pc, &save_pc, "PC"), (&note, &save_note, "Notebook")] {
        ok(
            cfg,
            &[
                "init",
                "--repo",
                cfg.join("repo").to_str().unwrap(),
                "--name",
                name,
            ],
        );
        ok(
            cfg,
            &["game", "add", "hookshot", "--path", save.to_str().unwrap()],
        );
        ok(
            cfg,
            &[
                "cloud",
                "add",
                "pasta",
                "--pasta",
                nuvem.to_str().unwrap(),
                "--sem-criptografia",
            ],
        );
    }
    fs::write(
        save_pc.join("slot1/caption.sav"),
        r#"{"location": "Vila do Porto", "level": 3}"#,
    )
    .unwrap();
    ok(&pc, &["sync"]);
    ok(&note, &["sync"]);
    // As duas jogam sem se ver.
    fs::write(
        save_pc.join("slot1/caption.sav"),
        r#"{"location": "Templo da Floresta", "level": 4}"#,
    )
    .unwrap();
    fs::write(
        save_note.join("slot1/caption.sav"),
        r#"{"location": "Cavernas de Sal", "level": 5}"#,
    )
    .unwrap();
    ok(&pc, &["snapshot"]);
    ok(&note, &["sync"]);
    // Conflito: o `sync` avisa e sai com erro, sem restaurar nada.
    let out = cmd(&pc).arg("sync").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("CONFLITO"));

    let ov = api(&pc, serde_json::json!({"call": "overview"}));
    assert_eq!(ov["machine"]["name"], "PC");
    let g = ov["games"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["id"] == "hookshot")
        .unwrap();
    assert_eq!(g["snapshots"], 3, "{g}");

    let d = api(&pc, serde_json::json!({"call": "game", "id": "hookshot"}));
    assert_eq!(d["plan"]["action"]["kind"], "conflito", "{d:#}");
    let tips = d["plan"]["action"]["tips"].as_array().unwrap();
    let theirs = tips.iter().find(|t| t["local"] == false).unwrap();
    assert_eq!(theirs["name"], "Notebook");
    let theirs = theirs["snapshot"].as_str().unwrap().to_string();

    // O leitor do Hookshot mostra o que tem em cada versão.
    let s = api(
        &pc,
        serde_json::json!({"call": "snapshot", "game": "hookshot", "snapshot": theirs}),
    );
    assert_eq!(s["meta"]["reader"], "Hookshot", "{s:#}");
    let fields = s["meta"]["fields"].as_array().unwrap();
    assert!(
        fields
            .iter()
            .any(|f| f[0] == "local" && f[1] == "Cavernas de Sal"),
        "{fields:?}"
    );

    // Snapshot que não é ponta do conflito é recusado.
    let first = d["snapshots"].as_array().unwrap().last().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let e = api_err(
        &pc,
        serde_json::json!({"call": "resolve", "game": "hookshot", "keep": first}),
    );
    assert!(e.contains("pontas"), "{e}");

    let r = api(
        &pc,
        serde_json::json!({"call": "resolve", "game": "hookshot", "keep": theirs, "keep_both": true}),
    );
    assert_eq!(r["ok"], true, "{r}");
    assert!(
        fs::read_to_string(save_pc.join("slot1/caption.sav"))
            .unwrap()
            .contains("Cavernas de Sal")
    );
    let d = api(&pc, serde_json::json!({"call": "game", "id": "hookshot"}));
    assert_eq!(d["plan"]["action"]["kind"], "nada", "{d:#}");
    let pinned: Vec<&Value> = d["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["pinned"].is_string())
        .collect();
    assert_eq!(pinned.len(), 1);
    assert_eq!(pinned[0]["pinned"], "versão de PC");

    // O notebook recebe a resolução sem ver conflito.
    ok(&pc, &["cloud", "sync"]);
    ok(&note, &["sync"]);
    let d = api(&note, serde_json::json!({"call": "game", "id": "hookshot"}));
    let k = d["plan"]["action"]["kind"].as_str().unwrap();
    assert!(k == "nada" || k == "adotar", "{d:#}");

    // Restaurar um ponto antigo e exportar.
    let r = api(
        &pc,
        serde_json::json!({"call": "restore", "game": "hookshot", "snapshot": first}),
    );
    assert_eq!(r["ok"], true);
    assert!(
        fs::read_to_string(save_pc.join("slot1/caption.sav"))
            .unwrap()
            .contains("Vila do Porto")
    );
    let zip = t.path().join("export.zip");
    let r = api(
        &pc,
        serde_json::json!({"call": "export_zip", "game": "hookshot", "snapshot": first, "dest": zip}),
    );
    assert_eq!(r["files"], 1);
    assert!(fs::metadata(&zip).unwrap().len() > 20);

    let act = api(&pc, serde_json::json!({"call": "activity"}));
    let kinds: Vec<&str> = act
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"resolvido"), "{kinds:?}");
    assert!(kinds.contains(&"restaurado"), "{kinds:?}");

    // Ajustes e modo do jogo.
    api(
        &pc,
        serde_json::json!({"call": "save_settings", "settings": {"machine_name": "Desktop", "retention": {"all_days": 2, "daily_days": 10, "monthly_days": 100}}}),
    );
    let st = api(&pc, serde_json::json!({"call": "settings"}));
    assert_eq!(st["machine_name"], "Desktop");
    assert_eq!(st["retention"]["daily_days"], 10);
    api(
        &pc,
        serde_json::json!({"call": "set_mode", "game": "hookshot", "mode": "historico"}),
    );
    let d = api(&pc, serde_json::json!({"call": "game", "id": "hookshot"}));
    assert_eq!(d["history_only"], true);
    assert_eq!(d["mode"], "historico");

    // Arte gerada (sem rede) vem como data URL.
    let a = api(
        &pc,
        serde_json::json!({"call": "art", "game": "hookshot", "kind": "capa"}),
    );
    assert!(a["url"].as_str().unwrap().starts_with("data:image/"), "{a}");
    let dv = api(&pc, serde_json::json!({"call": "devices"}));
    assert_eq!(dv["clouds"][0]["kind"], "pasta");
}
