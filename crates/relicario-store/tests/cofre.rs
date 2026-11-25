//! Cópia cifrada na "nuvem" (uma pasta vista só pelo lado de fora).

use relicario_engine::Action;
use relicario_model::{GameId, MachineId, SnapshotReason};
use relicario_store::crypto::Encrypted;
use relicario_store::ops::{self, GameSpec, Local};
use relicario_store::replicate::{read_config, replicate};
use relicario_store::{Error, Filter, Repository, Source, verify};
use relicario_transport::{Kind, LocalFolder, Remote};
use std::fs;
use std::path::Path;
use std::sync::Arc;

fn spec(save: &Path) -> GameSpec {
    GameSpec {
        id: GameId::new("bg3"),
        sources: vec![Source {
            root: "save".into(),
            path: save.to_path_buf(),
            filter: Filter::new(&[], &[]).unwrap(),
            mirrors: vec![],
        }],
    }
}

fn ruido(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

fn snap(dir: &Path, machine: &str, save: &Path) {
    let mut r = Repository::open(Arc::new(LocalFolder::new(dir))).unwrap();
    let mut l = Local {
        repo: &mut r,
        machine: MachineId::new(machine),
        machine_name: machine.into(),
    };
    ops::take_snapshot(&mut l, &spec(save), SnapshotReason::GameExit, true).unwrap();
}

#[test]
fn nuvem_ilegivel_sem_chave_e_envio_incremental_pequeno() {
    let t = tempfile::tempdir().unwrap();
    let (a, nuvem, b) = (
        t.path().join("a"),
        t.path().join("nuvem"),
        t.path().join("b"),
    );
    let (save_a, save_b) = (t.path().join("save-a"), t.path().join("save-b"));
    fs::create_dir_all(&save_a).unwrap();
    fs::create_dir_all(&save_b).unwrap();
    let marca = b"SEGREDO-DO-SAVE-0123456789".repeat(4);
    let mut big = ruido(20_000_000, 3);
    big[1_000..1_000 + marca.len()].copy_from_slice(&marca);
    fs::write(save_a.join("Honour.lsv"), &big).unwrap();

    let ra = Repository::init(Arc::new(LocalFolder::new(&a))).unwrap();
    let cfg = read_config(&LocalFolder::new(&a)).unwrap();
    drop(ra);
    let raw: Arc<dyn Remote> = Arc::new(LocalFolder::new(&nuvem));
    let (enc, recovery) = Encrypted::init(raw.clone(), &cfg, "senha boa").unwrap();
    assert_eq!(recovery.split('-').count(), 13);

    snap(&a, "pc", &save_a);
    let r1 = replicate(&LocalFolder::new(&a), &enc, &MachineId::new("pc")).unwrap();
    assert!(r1.bytes_out > 10_000_000, "{r1:?}");

    // Nada legível no destino: nem o conteúdo do save, nem o JSON dos snapshots.
    for kind in [Kind::Packs, Kind::Index, Kind::Snapshots, Kind::Heads] {
        for n in raw.list(kind).unwrap() {
            let bytes = raw.get(kind, &n).unwrap().unwrap();
            assert!(
                !bytes.windows(marca.len()).any(|w| w == &marca[..]),
                "{kind:?}/{n} vazou o save"
            );
            assert!(
                !bytes.windows(4).any(|w| w == b"\"bg3"),
                "{kind:?}/{n} vazou metadados"
            );
        }
    }

    // Autosave: muda um pedaço pequeno; o envio novo é bem menor que o save.
    big[5_000_000..5_010_000].copy_from_slice(&ruido(10_000, 99));
    fs::write(save_a.join("Honour.lsv"), &big).unwrap();
    snap(&a, "pc", &save_a);
    let r2 = replicate(&LocalFolder::new(&a), &enc, &MachineId::new("pc")).unwrap();
    assert!(
        r2.bytes_out * 10 < big.len() as u64,
        "incremental grande demais: {} de {}",
        r2.bytes_out,
        big.len()
    );

    // Senha errada não abre; a certa e a de recuperação abrem.
    assert!(matches!(
        Encrypted::open_with_password(raw.clone(), "senha ruim"),
        Err(Error::WrongPassword)
    ));
    let enc2 = Encrypted::open_with_password(raw.clone(), "senha boa").unwrap();
    let enc3 = Encrypted::open_with_recovery(raw.clone(), &recovery.to_lowercase()).unwrap();
    enc3.set_password("senha nova").unwrap();
    assert!(Encrypted::open_with_password(raw.clone(), "senha boa").is_err());
    let enc4 = Encrypted::open_with_password(raw.clone(), "senha nova").unwrap();
    drop((enc2, enc4));

    // Outra máquina, só com a nuvem: traz tudo, verifica e restaura byte a byte.
    Repository::init(Arc::new(LocalFolder::new(&b))).unwrap();
    replicate(&LocalFolder::new(&b), &enc3, &MachineId::new("note")).unwrap();
    let mut rb = Repository::open(Arc::new(LocalFolder::new(&b))).unwrap();
    assert!(verify::verify(&rb, true).unwrap().ok());
    let mut l = Local {
        repo: &mut rb,
        machine: MachineId::new("note"),
        machine_name: "Notebook".into(),
    };
    let Action::Avancar { to, .. } = ops::plan(&l, &spec(&save_b)).unwrap().0.action else {
        panic!()
    };
    ops::restore(&mut l, &spec(&save_b), to).unwrap();
    assert_eq!(fs::read(save_b.join("Honour.lsv")).unwrap(), big);

    // A nuvem também é um repositório verificável pela própria cópia cifrada.
    let rc = Repository::open(Arc::new(enc3)).unwrap();
    assert!(verify::verify(&rc, true).unwrap().ok());
}

#[test]
fn arquivo_trocado_na_nuvem_nao_decifra() {
    let t = tempfile::tempdir().unwrap();
    let (a, nuvem, save) = (
        t.path().join("a"),
        t.path().join("nuvem"),
        t.path().join("s"),
    );
    fs::create_dir_all(&save).unwrap();
    fs::write(save.join("x.sav"), b"um").unwrap();
    Repository::init(Arc::new(LocalFolder::new(&a))).unwrap();
    let cfg = read_config(&LocalFolder::new(&a)).unwrap();
    let raw: Arc<dyn Remote> = Arc::new(LocalFolder::new(&nuvem));
    let (enc, _) = Encrypted::init(raw.clone(), &cfg, "s").unwrap();
    snap(&a, "pc", &save);
    fs::write(save.join("x.sav"), b"dois").unwrap();
    snap(&a, "pc", &save);
    replicate(&LocalFolder::new(&a), &enc, &MachineId::new("pc")).unwrap();
    let snaps = raw.list(Kind::Snapshots).unwrap();
    let other = raw.get(Kind::Snapshots, &snaps[1]).unwrap().unwrap();
    raw.put(Kind::Snapshots, &snaps[0], &other).unwrap();
    let e = enc.get(Kind::Snapshots, &snaps[0]).unwrap_err();
    assert!(e.to_string().contains("alterado"), "{e}");
}
