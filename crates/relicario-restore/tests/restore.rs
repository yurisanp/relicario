//! O `relicario-restore` lê um repositório (em claro ou cifrado) sem mudar nada nele.

use relicario_model::{GameId, MachineId, SnapshotReason};
use relicario_store::crypto::Encrypted;
use relicario_store::ops::{self, GameSpec, Local};
use relicario_store::replicate::{read_config, replicate};
use relicario_store::{Filter, Repository, Source};
use relicario_transport::{LocalFolder, Remote};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

fn game(save: &Path) -> GameSpec {
    GameSpec {
        id: GameId::new("hookshot"),
        sources: vec![Source {
            root: "save".into(),
            path: save.to_path_buf(),
            filter: Filter::new(&[], &[]).unwrap(),
            mirrors: vec![],
        }],
    }
}

fn snapshot(repo: &Path, save: &Path) {
    let mut r = Repository::open(Arc::new(LocalFolder::new(repo))).unwrap();
    let mut l = Local {
        repo: &mut r,
        machine: MachineId::new("pc"),
        machine_name: "PC".into(),
    };
    ops::take_snapshot(&mut l, &game(save), SnapshotReason::GameExit, false).unwrap();
}

/// Conteúdo de todos os arquivos da pasta, para ver que nada mudou.
fn tree_of(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(
                    p.strip_prefix(dir).unwrap().to_string_lossy().into(),
                    fs::read(&p).unwrap(),
                );
            }
        }
    }
    out
}

fn run(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_relicario-restore"))
        .args(args)
        .env_remove("RELICARIO_SENHA")
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

#[test]
fn lista_extrai_e_nao_muda_nada_em_claro_e_cifrado() {
    let t = tempfile::tempdir().unwrap();
    let (repo, save, cloud) = (
        t.path().join("repo"),
        t.path().join("save"),
        t.path().join("nuvem"),
    );
    fs::create_dir_all(save.join("slot1")).unwrap();
    Repository::init(Arc::new(LocalFolder::new(&repo))).unwrap();
    fs::write(save.join("slot1/caption.sav"), b"capitulo 1").unwrap();
    snapshot(&repo, &save);
    fs::write(save.join("slot1/caption.sav"), b"capitulo 2").unwrap();
    fs::write(save.join("opcoes.ini"), b"volume=3").unwrap();
    snapshot(&repo, &save);

    // Cópia cifrada, como a da nuvem.
    let cfg = read_config(&LocalFolder::new(&repo)).unwrap();
    let raw: Arc<dyn Remote> = Arc::new(LocalFolder::new(&cloud));
    let (enc, recovery) = Encrypted::init(raw, &cfg, "senha boa").unwrap();
    replicate(&LocalFolder::new(&repo), &enc, &MachineId::new("pc")).unwrap();
    drop(enc);

    let before = (tree_of(&repo), tree_of(&cloud));
    let r = repo.to_str().unwrap();
    let (ok, out) = run(&[r, "jogos"]);
    assert!(ok, "{out}");
    assert!(out.contains("hookshot: 2 versão(ões)"), "{out}");
    let (ok, out) = run(&[r, "versoes", "hookshot"]);
    assert!(ok && out.contains("atual em PC"), "{out}");
    let first = out
        .lines()
        .last()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();

    // A mais nova do jogo, e uma versão antiga pelo id.
    let dest = t.path().join("recuperado");
    let (ok, out) = run(&[r, "extrair", "hookshot", dest.to_str().unwrap()]);
    assert!(ok, "{out}");
    assert_eq!(
        fs::read(dest.join("save/slot1/caption.sav")).unwrap(),
        b"capitulo 2"
    );
    assert_eq!(fs::read(dest.join("save/opcoes.ini")).unwrap(), b"volume=3");
    let old = t.path().join("antigo");
    let (ok, out) = run(&[r, "extrair", &first, old.to_str().unwrap()]);
    assert!(ok, "{out}");
    assert_eq!(
        fs::read(old.join("save/slot1/caption.sav")).unwrap(),
        b"capitulo 1"
    );
    assert!(!old.join("save/opcoes.ini").exists());
    let (ok, out) = run(&[r, "conferir"]);
    assert!(ok && out.contains("tudo inteiro"), "{out}");

    // A cópia cifrada: senha errada recusa, senha e chave de recuperação abrem.
    let c = cloud.to_str().unwrap();
    let (ok, _) = run(&[c, "--senha", "errada", "jogos"]);
    assert!(!ok);
    let dest2 = t.path().join("da-nuvem");
    let (ok, out) = run(&[
        c,
        "--senha",
        "senha boa",
        "extrair",
        "hookshot",
        dest2.to_str().unwrap(),
    ]);
    assert!(ok, "{out}");
    assert_eq!(
        fs::read(dest2.join("save/slot1/caption.sav")).unwrap(),
        b"capitulo 2"
    );
    let (ok, out) = run(&[c, "--recuperacao", &recovery, "jogos"]);
    assert!(ok && out.contains("hookshot"), "{out}");

    assert_eq!(
        before,
        (tree_of(&repo), tree_of(&cloud)),
        "o repositório não pode mudar"
    );
    let (ok, out) = run(&[t.path().join("nada").to_str().unwrap(), "jogos"]);
    assert!(!ok && out.contains("não é uma pasta"), "{out}");
}
