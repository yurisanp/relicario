//! Duas máquinas, cada uma com a própria cópia do repositório, replicando entre si.

use relicario_engine::Action;
use relicario_model::{GameId, MachineId, SnapshotReason};
use relicario_store::ops::{self, GameSpec, Local};
use relicario_store::replicate::replicate;
use relicario_store::{Filter, Repository, Source, verify};
use relicario_transport::{Kind, LocalFolder, Remote};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn spec(save: &Path) -> GameSpec {
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

/// Destino que "perde a rede" depois de `limit` escritas.
#[derive(Debug)]
struct Flaky {
    inner: LocalFolder,
    puts: AtomicUsize,
    limit: usize,
}

impl Remote for Flaky {
    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> relicario_transport::Result<()> {
        if self.puts.fetch_add(1, Ordering::SeqCst) >= self.limit {
            return Err(relicario_transport::Error::Other("conexão caiu".into()));
        }
        self.inner.put(kind, name, data)
    }
    fn get(&self, kind: Kind, name: &str) -> relicario_transport::Result<Option<Vec<u8>>> {
        self.inner.get(kind, name)
    }
    fn list(&self, kind: Kind) -> relicario_transport::Result<Vec<String>> {
        self.inner.list(kind)
    }
    fn delete(&self, kind: Kind, name: &str) -> relicario_transport::Result<()> {
        self.inner.delete(kind, name)
    }
    fn describe(&self) -> String {
        "instável".into()
    }
}

struct Machine {
    id: MachineId,
    name: &'static str,
    dir: std::path::PathBuf,
    save: std::path::PathBuf,
}

impl Machine {
    fn new(root: &Path, id: &str, name: &'static str) -> Self {
        let m = Machine {
            id: MachineId::new(id),
            name,
            dir: root.join(format!("repo-{id}")),
            save: root.join(format!("save-{id}")),
        };
        fs::create_dir_all(&m.save).unwrap();
        m
    }
    fn remote(&self) -> LocalFolder {
        LocalFolder::new(&self.dir)
    }
    fn with<T>(&self, f: impl FnOnce(&mut Local<'_>) -> T) -> T {
        let mut r = Repository::open(Arc::new(self.remote())).unwrap();
        let mut l = Local {
            repo: &mut r,
            machine: self.id.clone(),
            machine_name: self.name.into(),
        };
        f(&mut l)
    }
    fn play(&self, content: &[u8]) {
        fs::write(self.save.join("caption.sav"), content).unwrap();
        self.with(|l| {
            ops::take_snapshot(l, &spec(&self.save), SnapshotReason::GameExit, false).unwrap()
        });
    }
    fn plan(&self) -> Action {
        self.with(|l| ops::plan(l, &spec(&self.save)).unwrap().0.action)
    }
}

#[test]
fn jogar_em_a_abrir_em_b_divergir_e_retomar() {
    let t = tempfile::tempdir().unwrap();
    let a = Machine::new(t.path(), "pc", "PC");
    let b = Machine::new(t.path(), "note", "Notebook");
    Repository::init(Arc::new(a.remote())).unwrap();
    // B começa com um repositório próprio, vazio: os parâmetros iguais deixam juntar.
    Repository::init(Arc::new(b.remote())).unwrap();

    // Joga em A, replica, B vê avanço.
    a.play(b"capitulo 1");
    let r = replicate(&a.remote(), &b.remote(), &a.id).unwrap();
    assert!(
        r.pushed > 0 && r.heads_out == vec!["pc".to_string()],
        "{r:?}"
    );
    let Action::Avancar { to, .. } = b.plan() else {
        panic!("{:?}", b.plan())
    };
    b.with(|l| ops::restore(l, &spec(&b.save), to).unwrap());
    assert_eq!(fs::read(b.save.join("caption.sav")).unwrap(), b"capitulo 1");
    let r = replicate(&b.remote(), &a.remote(), &b.id).unwrap();
    assert_eq!(r.heads_out, vec!["note".to_string()]);
    assert!(
        !replicate(&a.remote(), &b.remote(), &a.id)
            .unwrap()
            .changed(),
        "nada novo dos dois lados"
    );

    // As duas jogam offline e reconectam: divergência, nada se perde.
    a.play(b"capitulo 2 no pc");
    b.play(b"capitulo 2 no notebook");
    replicate(&a.remote(), &b.remote(), &a.id).unwrap();
    assert!(
        matches!(a.plan(), Action::Conflito { .. }),
        "{:?}",
        a.plan()
    );
    assert!(
        matches!(b.plan(), Action::Conflito { .. }),
        "{:?}",
        b.plan()
    );

    // Envio grande com a rede caindo no meio: nada corrompido, a próxima vez completa.
    for i in 0..6u64 {
        fs::write(
            a.save.join(format!("mundo{i}.dat")),
            ruido(3_000_000, i + 7),
        )
        .unwrap();
    }
    a.with(|l| ops::take_snapshot(l, &spec(&a.save), SnapshotReason::GameExit, true).unwrap());
    let flaky = Flaky {
        inner: b.remote(),
        puts: AtomicUsize::new(0),
        limit: 1,
    };
    assert!(
        replicate(&a.remote(), &flaky, &a.id).is_err(),
        "a rede caiu"
    );
    // B continua consistente: o head do PC ainda é o antigo, e o que chegou está íntegro.
    b.with(|l| {
        let v = verify::verify(l.repo, true).unwrap();
        assert!(v.ok(), "{v:?}");
    });
    let r = replicate(&a.remote(), &b.remote(), &a.id).unwrap();
    assert!(r.pushed > 0 && r.rejected.is_empty(), "{r:?}");
    b.with(|l| {
        let v = verify::verify(l.repo, true).unwrap();
        assert!(v.ok(), "{v:?}");
    });
}

#[test]
fn arquivo_adulterado_e_recusado() {
    let t = tempfile::tempdir().unwrap();
    let a = Machine::new(t.path(), "pc", "PC");
    let b = Machine::new(t.path(), "note", "Notebook");
    Repository::init(Arc::new(a.remote())).unwrap();
    Repository::init(Arc::new(b.remote())).unwrap();
    a.play(b"capitulo 1");
    let pack = a.remote().list(Kind::Packs).unwrap().remove(0);
    a.remote().put(Kind::Packs, &pack, b"lixo").unwrap();
    let r = replicate(&a.remote(), &b.remote(), &a.id).unwrap();
    assert_eq!(r.rejected, vec![format!("packs/{pack}")]);
    assert!(b.remote().get(Kind::Packs, &pack).unwrap().is_none());
}

#[test]
fn resolver_conflito_mantendo_a_versao_da_outra_maquina() {
    let t = tempfile::tempdir().unwrap();
    let a = Machine::new(t.path(), "pc", "PC");
    let b = Machine::new(t.path(), "note", "Notebook");
    Repository::init(Arc::new(a.remote())).unwrap();
    Repository::init(Arc::new(b.remote())).unwrap();
    a.play(b"comum");
    replicate(&a.remote(), &b.remote(), &a.id).unwrap();
    let Action::Avancar { to, .. } = b.plan() else {
        panic!()
    };
    b.with(|l| ops::restore(l, &spec(&b.save), to).unwrap());
    a.play(b"no pc");
    b.play(b"no notebook");
    replicate(&a.remote(), &b.remote(), &a.id).unwrap();
    let Action::Conflito { tips } = a.plan() else {
        panic!("{:?}", a.plan())
    };
    let theirs = tips.iter().find(|(m, _)| *m == b.id).unwrap().1;
    // A ponta local aparece com um id especial.
    let ours = tips.iter().find(|(m, _)| *m != b.id).unwrap().1;
    let (merge, out) = a.with(|l| ops::resolve(l, &spec(&a.save), theirs, &[ours]).unwrap());
    // O save do PC já estava guardado: nada de cópia de segurança.
    assert_eq!(out.safety, None);
    assert_eq!(
        fs::read(a.save.join("caption.sav")).unwrap(),
        b"no notebook"
    );
    let s = a.with(|l| l.repo.get_snapshot(&merge).unwrap());
    assert_eq!(s.parents, vec![theirs, ours]);
    assert_eq!(s.reason, SnapshotReason::Merge);
    assert_eq!(a.plan(), Action::Nada);
    // Do lado do notebook, o merge tem o mesmo conteúdo e descende do que ele tem: sem conflito.
    replicate(&a.remote(), &b.remote(), &a.id).unwrap();
    match b.plan() {
        Action::Nada => {}
        Action::Adotar { to } | Action::Avancar { to, .. } => assert_eq!(to, merge),
        other => panic!("{other:?}"),
    }
}

#[test]
fn voltar_a_uma_versao_antiga_vale_para_as_outras_maquinas() {
    let t = tempfile::tempdir().unwrap();
    let a = Machine::new(t.path(), "pc", "PC");
    let b = Machine::new(t.path(), "note", "Notebook");
    Repository::init(Arc::new(a.remote())).unwrap();
    Repository::init(Arc::new(b.remote())).unwrap();
    a.play(b"capitulo 1");
    a.play(b"capitulo 2 (save estragado)");
    let first = a.with(|l| {
        ops::game_snapshots(l.repo, &GameId::new("hookshot"))
            .unwrap()
            .first()
            .unwrap()
            .0
    });
    replicate(&a.remote(), &b.remote(), &a.id).unwrap();
    let Action::Avancar { to, .. } = b.plan() else {
        panic!("{:?}", b.plan())
    };
    b.with(|l| ops::restore(l, &spec(&b.save), to).unwrap());
    replicate(&b.remote(), &a.remote(), &b.id).unwrap();

    // A volta ao capítulo 1. Vira uma versão nova, filha da atual.
    a.with(|l| ops::rollback(l, &spec(&a.save), first).unwrap());
    assert_eq!(fs::read(a.save.join("caption.sav")).unwrap(), b"capitulo 1");
    // Para A, é só uma versão nova a enviar.
    assert_eq!(a.plan(), Action::Enviar);
    let head = a.with(|l| l.head_of(&GameId::new("hookshot")).unwrap().unwrap());
    assert_ne!(head, first);
    let s = a.with(|l| l.repo.get_snapshot(&head).unwrap());
    assert_eq!(s.reason, SnapshotReason::Restored);

    // B recebe a volta como novidade, sem conflito.
    replicate(&a.remote(), &b.remote(), &a.id).unwrap();
    let Action::Avancar { to, .. } = b.plan() else {
        panic!("{:?}", b.plan())
    };
    assert_eq!(to, head);
    b.with(|l| ops::restore(l, &spec(&b.save), to).unwrap());
    assert_eq!(fs::read(b.save.join("caption.sav")).unwrap(), b"capitulo 1");

    // Voltar para a frente (uma versão que já descende do head) só restaura.
    let n = a.with(|l| {
        ops::game_snapshots(l.repo, &GameId::new("hookshot"))
            .unwrap()
            .len()
    });
    a.with(|l| ops::rollback(l, &spec(&a.save), head).unwrap());
    let n2 = a.with(|l| {
        ops::game_snapshots(l.repo, &GameId::new("hookshot"))
            .unwrap()
            .len()
    });
    assert_eq!(n, n2);
}
