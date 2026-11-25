use relicario_model::{GameId, MachineId, SnapshotReason};
use relicario_store::ops::{self, GameSpec, Local, SnapshotOutcome};
use relicario_store::{Filter, Repository, Source, files, prune, verify};
use relicario_transport::{Kind, LocalFolder, Remote};
use std::fs;
use std::path::Path;
use std::sync::Arc;

fn repo(dir: &Path) -> Repository {
    Repository::init(Arc::new(LocalFolder::new(dir))).unwrap()
}

fn hookshot(save: &Path) -> GameSpec {
    GameSpec {
        id: GameId::new("hookshot"),
        sources: vec![Source {
            root: "save".into(),
            path: save.to_path_buf(),
            filter: Filter::new(
                &[],
                &["save-backups/**".into(), "hookshot_options.ini".into()],
            )
            .unwrap(),
            mirrors: vec![],
        }],
    }
}

fn write(dir: &Path, rel: &str, data: &[u8]) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

fn ruido(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

#[test]
fn restauracao_identica_byte_a_byte() {
    let tmp = tempfile::tempdir().unwrap();
    let save = tmp.path().join("save");
    write(&save, "slot1/caption.sav", &ruido(3_000_000, 1));
    write(&save, "slot1/progress.sav", b"{\"chapter\":3}");
    write(&save, "vazio.sav", b"");
    write(&save, "save-backups/old.sav", b"ignorado");
    write(&save, "hookshot_options.ini", b"volume=3");
    let mut r = repo(&tmp.path().join("repo"));
    let game = hookshot(&save);
    let mut l = Local {
        repo: &mut r,
        machine: MachineId::new("pc"),
        machine_name: "PC".into(),
    };
    let SnapshotOutcome::Created(s1) =
        ops::take_snapshot(&mut l, &game, SnapshotReason::Manual, false).unwrap()
    else {
        panic!()
    };
    let original = files::scan(&game.sources).unwrap();
    assert_eq!(original.files.len(), 3, "filtro de exclusão aplicado");

    // Joga mais: arquivo muda, outro some, outro aparece.
    write(&save, "slot1/caption.sav", &ruido(3_000_000, 2));
    fs::remove_file(save.join("slot1/progress.sav")).unwrap();
    write(&save, "slot2/novo.sav", b"novo");
    let out = ops::restore(&mut l, &game, s1).unwrap();
    assert!(
        out.safety.is_some(),
        "estado atual guardado antes de restaurar"
    );
    let depois = files::scan(&game.sources).unwrap();
    assert_eq!(depois.content_hash(), original.content_hash());
    assert_eq!(
        fs::read(save.join("slot1/caption.sav")).unwrap(),
        ruido(3_000_000, 1)
    );
    assert!(!save.join("slot2/novo.sav").exists());
    // Arquivos ignorados pelo filtro ficam intactos.
    assert_eq!(
        fs::read(save.join("hookshot_options.ini")).unwrap(),
        b"volume=3"
    );
    assert!(save.join("save-backups/old.sav").exists());
    // E o estado de antes da restauração pode voltar.
    let safety = out.safety.unwrap();
    ops::restore(&mut l, &game, safety).unwrap();
    assert_eq!(fs::read(save.join("slot2/novo.sav")).unwrap(), b"novo");

    let v = verify::verify(l.repo, true).unwrap();
    assert!(v.ok(), "{:?}", v.errors);
}

#[test]
fn deduplica_e_nao_regrava_sem_mudanca() {
    let tmp = tempfile::tempdir().unwrap();
    let save = tmp.path().join("save");
    let big = ruido(2_000_000, 7);
    write(&save, "a.sav", &big);
    let mut r = repo(&tmp.path().join("repo"));
    let game = hookshot(&save);
    let mut l = Local {
        repo: &mut r,
        machine: MachineId::new("pc"),
        machine_name: "PC".into(),
    };
    ops::take_snapshot(&mut l, &game, SnapshotReason::Manual, false).unwrap();
    assert!(matches!(
        ops::take_snapshot(&mut l, &game, SnapshotReason::Manual, false).unwrap(),
        SnapshotOutcome::Unchanged(Some(_))
    ));
    let packs_antes = l.repo.remote().list(Kind::Packs).unwrap().len();
    // Muda só o fim do arquivo: quase todos os blocos se repetem.
    let mut big2 = big.clone();
    big2.extend_from_slice(b"fim do capitulo");
    write(&save, "a.sav", &big2);
    ops::take_snapshot(&mut l, &game, SnapshotReason::Manual, false).unwrap();
    let total: u64 = l
        .repo
        .remote()
        .list(Kind::Packs)
        .unwrap()
        .iter()
        .map(|n| l.repo.remote().get(Kind::Packs, n).unwrap().unwrap().len() as u64)
        .sum();
    assert!(packs_antes >= 1);
    assert!(
        total < 2 * big.len() as u64 * 6 / 10,
        "envio incremental pequeno: {total}"
    );
}

#[test]
fn guarda_nao_promove_save_corrompido() {
    let tmp = tempfile::tempdir().unwrap();
    let save = tmp.path().join("save");
    write(&save, "a.sav", &ruido(100_000, 3));
    let mut r = repo(&tmp.path().join("repo"));
    let game = hookshot(&save);
    let mut l = Local {
        repo: &mut r,
        machine: MachineId::new("pc"),
        machine_name: "PC".into(),
    };
    let SnapshotOutcome::Created(s1) =
        ops::take_snapshot(&mut l, &game, SnapshotReason::Manual, false).unwrap()
    else {
        panic!()
    };
    write(&save, "a.sav", &vec![0u8; 100_000]);
    let out = ops::take_snapshot(&mut l, &game, SnapshotReason::Manual, false).unwrap();
    assert!(matches!(out, SnapshotOutcome::Suspect(_, _)));
    assert_eq!(l.head_of(&game.id).unwrap(), Some(s1), "head não andou");
}

#[test]
fn le_de_volta_depois_de_escrita_interrompida() {
    let tmp = tempfile::tempdir().unwrap();
    let save = tmp.path().join("save");
    let repo_dir = tmp.path().join("repo");
    write(&save, "a.sav", &ruido(500_000, 4));
    {
        let mut r = repo(&repo_dir);
        let game = hookshot(&save);
        let mut l = Local {
            repo: &mut r,
            machine: MachineId::new("pc"),
            machine_name: "PC".into(),
        };
        ops::take_snapshot(&mut l, &game, SnapshotReason::Manual, false).unwrap();
    }
    // Simula o processo morto no meio: pack escrito sem índice, temporários pela metade e uma
    // trava que ficou para trás.
    let remote = LocalFolder::new(&repo_dir);
    remote
        .put(Kind::Packs, "ff00deadbeef", b"RLCPACK1metade")
        .unwrap();
    fs::write(repo_dir.join("snapshots").join(".tmp-xyz-1"), b"{meio").unwrap();
    fs::create_dir_all(repo_dir.join("index")).unwrap();
    fs::write(repo_dir.join("index").join(".tmp-abc-2"), b"[").unwrap();

    let mut r = Repository::open(Arc::new(LocalFolder::new(&repo_dir))).unwrap();
    let v = verify::verify(&r, true).unwrap();
    assert!(v.ok(), "{:?}", v.errors);
    assert_eq!(v.warnings.len(), 1, "pack órfão avisado");
    let rep = prune::prune(&mut r, &MachineId::new("pc")).unwrap();
    assert_eq!(rep.packs_removed, 1);
    let v = verify::verify(&r, true).unwrap();
    assert!(
        v.ok() && v.warnings.is_empty(),
        "{:?} {:?}",
        v.errors,
        v.warnings
    );
}

#[test]
fn prune_libera_esquecidos_e_mantem_o_resto() {
    let tmp = tempfile::tempdir().unwrap();
    let save = tmp.path().join("save");
    let mut r = repo(&tmp.path().join("repo"));
    let game = hookshot(&save);
    let mut l = Local {
        repo: &mut r,
        machine: MachineId::new("pc"),
        machine_name: "PC".into(),
    };
    let mut ids = vec![];
    for i in 0..4 {
        write(&save, "a.sav", &ruido(300_000, 10 + i));
        match ops::take_snapshot(&mut l, &game, SnapshotReason::Manual, false).unwrap() {
            SnapshotOutcome::Created(id) => ids.push(id),
            o => panic!("{o:?}"),
        }
    }
    ops::pin(&mut l, ids[0], "antes do chefe".into()).unwrap();
    // Esquece o segundo e o terceiro à mão (a política real é testada no motor).
    let mut h = l.heads().unwrap();
    h.forgotten.insert(ids[1]);
    h.forgotten.insert(ids[2]);
    l.repo.put_heads(&MachineId::new("pc"), &h).unwrap();
    let rep = prune::prune(l.repo, &MachineId::new("pc")).unwrap();
    assert!(rep.bytes_freed > 400_000, "{rep:?}");
    let v = verify::verify(l.repo, true).unwrap();
    assert!(v.ok(), "{:?}", v.errors);
    // O fixado e o head continuam restauráveis.
    ops::restore(&mut l, &game, ids[0]).unwrap();
    assert_eq!(fs::read(save.join("a.sav")).unwrap(), ruido(300_000, 10));
    ops::restore(&mut l, &game, ids[3]).unwrap();
    // A árvore de metadados continua inteira.
    let g = ops::graph(l.repo, &game.id).unwrap();
    assert_eq!(g.len(), 4);
}

#[test]
fn duas_maquinas_no_mesmo_repositorio() {
    let tmp = tempfile::tempdir().unwrap();
    let repo_dir = tmp.path().join("repo");
    let pc = tmp.path().join("pc");
    let note = tmp.path().join("note");
    repo(&repo_dir);
    let open = || Repository::open(Arc::new(LocalFolder::new(&repo_dir))).unwrap();
    write(&pc, "a.sav", b"capitulo 1");
    let (mut rp, mut rn) = (open(), open());
    let mut lp = Local {
        repo: &mut rp,
        machine: MachineId::new("pc"),
        machine_name: "PC".into(),
    };
    ops::take_snapshot(&mut lp, &hookshot(&pc), SnapshotReason::GameExit, false).unwrap();

    let mut ln = Local {
        repo: &mut rn,
        machine: MachineId::new("note"),
        machine_name: "Notebook".into(),
    };
    ln.repo.reload_index().unwrap();
    let (p, _) = ops::plan(&ln, &hookshot(&note)).unwrap();
    let relicario_engine::Action::Avancar { to, .. } = p.action else {
        panic!("{p:?}")
    };
    ops::restore(&mut ln, &hookshot(&note), to).unwrap();
    assert_eq!(fs::read(note.join("a.sav")).unwrap(), b"capitulo 1");

    // Os dois jogam offline: divergência, nada é restaurado.
    write(&pc, "a.sav", b"capitulo 2 no pc");
    write(&note, "a.sav", b"capitulo 2 no note");
    ops::take_snapshot(&mut lp, &hookshot(&pc), SnapshotReason::GameExit, false).unwrap();
    ln.repo.reload_index().unwrap();
    let (p, _) = ops::plan(&ln, &hookshot(&note)).unwrap();
    assert!(p.dirty);
    assert!(
        matches!(p.action, relicario_engine::Action::Conflito { .. }),
        "{p:?}"
    );
}

#[test]
fn snapshot_do_hookshot_em_menos_de_1s() {
    let tmp = tempfile::tempdir().unwrap();
    let save = tmp.path().join("save");
    // Tamanho típico do Hookshot: alguns arquivos pequenos.
    write(&save, "caption.sav", &ruido(40_000, 1));
    write(&save, "progress.sav", &ruido(8_000, 2));
    write(&save, "slot1/world.sav", &ruido(200_000, 3));
    let mut r = repo(&tmp.path().join("repo"));
    let game = hookshot(&save);
    let mut l = Local {
        repo: &mut r,
        machine: MachineId::new("pc"),
        machine_name: "PC".into(),
    };
    let t = std::time::Instant::now();
    ops::take_snapshot(&mut l, &game, SnapshotReason::GameExit, false).unwrap();
    write(&save, "caption.sav", &ruido(40_000, 9));
    ops::take_snapshot(&mut l, &game, SnapshotReason::GameExit, false).unwrap();
    assert!(
        t.elapsed() < std::time::Duration::from_secs(1),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn restauracao_escreve_nos_espelhos() {
    let tmp = tempfile::tempdir().unwrap();
    let save = tmp.path().join("save");
    let espelho = tmp.path().join("userdata/remote");
    write(&save, "slot.lsv", b"v1");
    let mut game = hookshot(&save);
    game.sources[0].mirrors.push(espelho.clone());
    let mut r = repo(&tmp.path().join("repo"));
    let mut l = Local {
        repo: &mut r,
        machine: MachineId::new("pc"),
        machine_name: "PC".into(),
    };
    let SnapshotOutcome::Created(s1) =
        ops::take_snapshot(&mut l, &game, SnapshotReason::Manual, false).unwrap()
    else {
        panic!()
    };
    write(&save, "slot.lsv", b"v2");
    ops::take_snapshot(&mut l, &game, SnapshotReason::Manual, false).unwrap();
    ops::restore(&mut l, &game, s1).unwrap();
    assert_eq!(fs::read(save.join("slot.lsv")).unwrap(), b"v1");
    assert_eq!(fs::read(espelho.join("slot.lsv")).unwrap(), b"v1");
}
