//! M3 "Cofre externo": nuvem cifrada entre duas máquinas e importação do Ludusavi.

use sha1::{Digest, Sha1};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
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
        .env("RELICARIO_KDF_FAST", "1")
        .env("RELICARIO_P2P_PORT", "0")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("RELICARIO_PASSWORD")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("APPDATA")
        .env_remove("LOCALAPPDATA");
    c
}

fn run(cfg: &Path, env: &[(&str, &str)], args: &[&str]) -> (bool, String) {
    let out = cmd(cfg)
        .envs(env.iter().copied())
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn ok(cfg: &Path, env: &[(&str, &str)], args: &[&str]) -> String {
    let (s, t) = run(cfg, env, args);
    assert!(s, "relicario {args:?} falhou:\n{t}");
    t
}

fn all_files(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            v.extend(all_files(&p));
        } else {
            v.push(p);
        }
    }
    v
}

#[test]
fn nuvem_cifrada_entre_duas_maquinas() {
    let t = tempfile::tempdir().unwrap();
    let (pc, note, nuvem) = (
        t.path().join("pc"),
        t.path().join("note"),
        t.path().join("nuvem"),
    );
    let (save_pc, save_note) = (t.path().join("save-pc"), t.path().join("save-note"));
    fs::create_dir_all(&save_pc).unwrap();
    fs::create_dir_all(&save_note).unwrap();
    let pw = [("RELICARIO_PASSWORD", "senha comprida")];
    for (cfg, save, name) in [(&pc, &save_pc, "PC"), (&note, &save_note, "Notebook")] {
        ok(
            cfg,
            &[],
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
            &[],
            &["game", "add", "hookshot", "--path", save.to_str().unwrap()],
        );
    }
    fs::write(save_pc.join("caption.sav"), "SEGREDO capitulo 1").unwrap();
    ok(&pc, &[], &["snapshot"]);

    let out = ok(
        &pc,
        &pw,
        &["cloud", "add", "nuvem", "--pasta", nuvem.to_str().unwrap()],
    );
    assert!(out.contains("CHAVE DE RECUPERAÇÃO"), "{out}");
    let recovery = out
        .lines()
        .map(str::trim)
        .find(|l| l.len() > 50 && l.split('-').count() == 13)
        .unwrap()
        .to_string();
    for f in all_files(&nuvem) {
        let b = fs::read(&f).unwrap();
        assert!(
            !b.windows(7).any(|w| w == b"SEGREDO"),
            "{} vazou o save",
            f.display()
        );
    }

    // O notebook entra no mesmo cofre: senha errada não, a certa sim.
    let (s, t2) = run(
        &note,
        &[("RELICARIO_PASSWORD", "errada")],
        &["cloud", "add", "nuvem", "--pasta", nuvem.to_str().unwrap()],
    );
    assert!(!s && t2.contains("senha"), "{t2}");
    assert!(
        ok(
            &note,
            &pw,
            &["cloud", "add", "nuvem", "--pasta", nuvem.to_str().unwrap()]
        )
        .contains("já existia")
    );
    ok(&note, &[], &["sync"]);
    assert_eq!(
        fs::read_to_string(save_note.join("caption.sav")).unwrap(),
        "SEGREDO capitulo 1"
    );

    // Volta: o notebook joga, o PC traz.
    fs::write(save_note.join("caption.sav"), "SEGREDO capitulo 2").unwrap();
    ok(&note, &[], &["sync"]);
    ok(&pc, &[], &["sync"]);
    assert_eq!(
        fs::read_to_string(save_pc.join("caption.sav")).unwrap(),
        "SEGREDO capitulo 2"
    );

    let c = ok(&pc, &[], &["cloud", "check"]);
    assert!(c.contains("restaurado e conferido"), "{c}");

    // Esqueceu a senha: a chave de recuperação define outra.
    ok(
        &pc,
        &[
            ("RELICARIO_RECOVERY", &recovery),
            ("RELICARIO_PASSWORD", "outra senha boa"),
        ],
        &["cloud", "unlock", "nuvem", "--recovery"],
    );
    ok(&pc, &[], &["cloud", "sync"]);
    let (s, _) = run(&note, &[], &["cloud", "sync"]);
    assert!(
        !s,
        "a senha antiga guardada no notebook não deveria mais abrir"
    );
    ok(
        &note,
        &[("RELICARIO_PASSWORD", "outra senha boa")],
        &["cloud", "unlock", "nuvem"],
    );
    ok(&note, &[], &["cloud", "sync"]);
}

fn sha1_hex(b: &[u8]) -> String {
    Sha1::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn importa_backups_do_ludusavi() {
    let t = tempfile::tempdir().unwrap();
    let cfg = t.path().join("cfg");
    let save = t.path().join("save");
    fs::create_dir_all(&save).unwrap();
    ok(
        &cfg,
        &[],
        &[
            "init",
            "--repo",
            cfg.join("repo").to_str().unwrap(),
            "--name",
            "PC",
        ],
    );
    ok(
        &cfg,
        &[],
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

    let orig = save.to_string_lossy().replace('\\', "/");
    let (drive, rest) = if cfg!(windows) {
        (orig[..2].to_string(), orig[2..].to_string())
    } else {
        (String::new(), orig.clone())
    };
    let folder = if cfg!(windows) {
        format!("drive-{}", &drive[..1])
    } else {
        "drive-0".to_string()
    };
    let back = t.path().join("ludusavi");
    let g = back.join("Hookshot");
    let put = |backup: &str, name: &str, data: &[u8]| {
        let p = g
            .join(backup)
            .join(&folder)
            .join(rest.trim_start_matches('/'))
            .join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    };
    let a1 = vec![7u8; 300_000];
    put("backup-20240101T000000Z", "a.sav", &a1);
    put("backup-20240101T000000Z", "b.sav", b"b1");
    put("backup-20240102T000000Z-diff", "a.sav", b"a2");
    let mapping = format!(
        "name: Hookshot\ndrives:\n  {folder}: \"{drive}\"\nbackups:\n  - name: backup-20240101T000000Z\n    when: \"2024-01-01T00:00:00Z\"\n    files:\n      \"{orig}/a.sav\":\n        hash: {}\n        size: {}\n      \"{orig}/b.sav\":\n        hash: {}\n        size: 2\n    children:\n      - name: backup-20240102T000000Z-diff\n        when: \"2024-01-02T00:00:00Z\"\n        files:\n          \"{orig}/a.sav\":\n            hash: {}\n            size: 2\n          \"{orig}/b.sav\": ~\n",
        sha1_hex(&a1),
        a1.len(),
        sha1_hex(b"b1"),
        sha1_hex(b"a2")
    );
    fs::write(g.join("mapping.yaml"), mapping).unwrap();

    // Um segundo jogo, desconhecido, em zip.
    let z = back.join("Jogo Antigo");
    fs::create_dir_all(&z).unwrap();
    let mut w =
        zip::ZipWriter::new(fs::File::create(z.join("backup-20230505T101010Z.zip")).unwrap());
    w.start_file(
        format!("{folder}/{}/x.sav", "jogos/antigo"),
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    w.write_all(b"xx").unwrap();
    w.finish().unwrap();
    let abs = if cfg!(windows) {
        format!("{drive}/jogos/antigo/x.sav")
    } else {
        "/jogos/antigo/x.sav".to_string()
    };
    fs::write(
        z.join("mapping.yaml"),
        format!("name: Jogo Antigo\ndrives:\n  {folder}: \"{drive}\"\nbackups:\n  - name: backup-20230505T101010Z.zip\n    when: \"2023-05-05T10:10:10Z\"\n    files:\n      \"{abs}\":\n        hash: {}\n        size: 2\n", sha1_hex(b"xx")),
    )
    .unwrap();

    let out = ok(
        &cfg,
        &[],
        &[
            "import",
            "ludusavi",
            "--backups",
            back.to_str().unwrap(),
            "--sem-personalizados",
        ],
    );
    assert!(
        out.contains("Hookshot → hookshot: 2 snapshots, 3 arquivos"),
        "{out}"
    );
    assert!(
        out.contains("Jogo Antigo → jogo-antigo: 1 snapshots, 1 arquivos"),
        "{out}"
    );
    assert!(out.contains("tudo conferido"), "{out}");
    let log = ok(&cfg, &[], &["log", "hookshot"]);
    assert_eq!(log.matches("Import").count(), 2, "{log}");
    assert!(log.contains("importado do Ludusavi"), "{log}");
    assert!(!save.join("a.sav").exists(), "importar não restaura nada");

    // Restaurar o primeiro backup devolve os arquivos originais.
    let first = log
        .lines()
        .find(|l| l.contains("Import") && l.contains("2024-01-01"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    ok(&cfg, &[], &["restore", "hookshot", &first]);
    assert_eq!(fs::read(save.join("a.sav")).unwrap(), a1);
    assert_eq!(fs::read(save.join("b.sav")).unwrap(), b"b1");
}
