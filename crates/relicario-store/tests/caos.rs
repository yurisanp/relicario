//! Bateria de caos: o que acontece quando as coisas dão errado no meio. A regra é sempre a
//! mesma: nenhum progresso de jogo se perde e o estado volta a ser coerente sozinho.

use relicario_engine::Action;
use relicario_engine::retention::Policy;
use relicario_model::{GameId, MachineId, SnapshotReason};
use relicario_store::ops::{self, GameSpec, Local, SnapshotOutcome};
use relicario_store::replicate::replicate;
use relicario_store::{Filter, Repository, Source, prune, verify};
use relicario_transport::{Kind, LocalFolder, Remote};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const DIA: i64 = 86_400_000;

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

struct Machine {
    id: MachineId,
    name: &'static str,
    dir: PathBuf,
    save: PathBuf,
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
        Repository::init(Arc::new(m.remote())).unwrap();
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
    fn write(&self, name: &str, content: &[u8]) {
        fs::write(self.save.join(name), content).unwrap();
    }
    fn read(&self, name: &str) -> Vec<u8> {
        fs::read(self.save.join(name)).unwrap()
    }
    fn snap(&self) -> SnapshotOutcome {
        self.with(|l| {
            ops::take_snapshot(l, &spec(&self.save), SnapshotReason::GameExit, false).unwrap()
        })
    }
    fn play(&self, content: &[u8]) -> ops::SnapshotOutcome {
        self.write("caption.sav", content);
        self.snap()
    }
    fn head(&self) -> Option<relicario_model::SnapshotId> {
        self.with(|l| l.head_of(&GameId::new("hookshot")).unwrap())
    }
    fn plan(&self) -> Action {
        self.with(|l| ops::plan(l, &spec(&self.save)).unwrap().0.action)
    }
    fn count(&self) -> usize {
        self.with(|l| {
            ops::game_snapshots(l.repo, &GameId::new("hookshot"))
                .unwrap()
                .len()
        })
    }
    /// Traz a versão mais nova de outra máquina, como o `sync` faz.
    fn follow(&self) {
        match self.plan() {
            Action::Avancar { to, .. } => {
                self.with(|l| ops::restore(l, &spec(&self.save), to).unwrap());
            }
            Action::Adotar { to } => {
                self.with(|l| ops::adopt(l, &GameId::new("hookshot"), to).unwrap());
            }
            Action::Nada | Action::Enviar => {}
            a => panic!("{a:?}"),
        }
    }
}

fn both_ways(a: &Machine, b: &Machine) {
    replicate(&a.remote(), &b.remote(), &a.id).unwrap();
    replicate(&b.remote(), &a.remote(), &b.id).unwrap();
}

#[test]
fn restauracao_interrompida_termina_na_proxima_vez_sem_virar_snapshot() {
    let t = tempfile::tempdir().unwrap();
    let a = Machine::new(t.path(), "pc", "PC");
    a.write("a.sav", b"capitulo 1 a");
    a.write("b.sav", b"capitulo 1 b");
    a.snap();
    let first = a.head().unwrap();
    a.write("a.sav", b"capitulo 2 a");
    a.write("b.sav", b"capitulo 2 b");
    a.snap();
    let second = a.head().unwrap();

    // `b.sav` não pode ser escrito (uma pasta no lugar, como um arquivo travado pelo jogo):
    // a restauração cai depois de escrever `a.sav`.
    fs::remove_file(a.save.join("b.sav")).unwrap();
    fs::create_dir_all(a.save.join("b.sav/travado")).unwrap();
    fs::write(a.save.join("b.sav/travado/x"), b"x").unwrap();
    let err = a.with(|l| ops::rollback(l, &spec(&a.save), first));
    assert!(err.is_err());
    let before = a.count();

    // O obstáculo some (o jogo fechou). A próxima operação termina a restauração em vez de
    // guardar a mistura como versão nova.
    fs::remove_dir_all(a.save.join("b.sav")).unwrap();
    let out = a.snap();
    assert!(matches!(out, SnapshotOutcome::Unchanged(_)), "{out:?}");
    assert_eq!(a.read("a.sav"), b"capitulo 1 a");
    assert_eq!(a.read("b.sav"), b"capitulo 1 b");
    assert_eq!(
        a.count(),
        before,
        "nenhuma versão nova com o disco misturado"
    );
    assert!(a.with(|l| l.heads().unwrap().restoring.is_empty()));
    // O capítulo 2 continua guardado.
    assert_ne!(a.head(), Some(second));
    assert!(a.with(|l| l.repo.get_snapshot(&second).is_ok()));
    let r = Repository::open(Arc::new(a.remote())).unwrap();
    assert!(verify::verify(&r, true).unwrap().ok());
}

/// Destino que cai depois de `limit` escritas, como um processo morto no meio do envio.
#[derive(Debug)]
struct Morre {
    inner: LocalFolder,
    puts: AtomicUsize,
    limit: usize,
}

impl Remote for Morre {
    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> relicario_transport::Result<()> {
        if self.puts.fetch_add(1, Ordering::SeqCst) >= self.limit {
            return Err(relicario_transport::Error::Other(
                "No space left on device".into(),
            ));
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
        "morre".into()
    }
}

#[test]
fn envio_cortado_em_qualquer_ponto_nunca_deixa_o_destino_apontando_para_o_que_falta() {
    let t = tempfile::tempdir().unwrap();
    let a = Machine::new(t.path(), "pc", "PC");
    for i in 0..4u8 {
        a.write("caption.sav", &vec![i; 300_000]);
        a.write(&format!("extra{i}.sav"), &vec![i ^ 0x55; 50_000]);
        a.snap();
    }
    // Corta o envio depois de 0, 1, 2... escritas. Depois de cada corte, quem lê o destino vê
    // um repositório coerente (sem head para o que não chegou), e um envio inteiro completa.
    for limit in 0..12 {
        let dest = t.path().join(format!("dest-{limit}"));
        Repository::init(Arc::new(LocalFolder::new(&dest))).unwrap();
        let morre = Morre {
            inner: LocalFolder::new(&dest),
            puts: AtomicUsize::new(0),
            limit,
        };
        let _ = replicate(&a.remote(), &morre, &a.id);
        let r = Repository::open(Arc::new(LocalFolder::new(&dest))).unwrap();
        let report = verify::verify(&r, true).unwrap();
        assert!(report.ok(), "corte em {limit}: {report:?}");
        drop(r);
        replicate(&a.remote(), &LocalFolder::new(&dest), &a.id).unwrap();
        let r = Repository::open(Arc::new(LocalFolder::new(&dest))).unwrap();
        assert!(verify::verify(&r, true).unwrap().ok());
        assert_eq!(
            r.heads_of(&a.id).unwrap().unwrap().heads[&GameId::new("hookshot")].snapshot,
            a.head().unwrap()
        );
    }
}

#[test]
fn relogios_com_duas_horas_de_diferenca_nao_mudam_quem_e_mais_novo() {
    let t = tempfile::tempdir().unwrap();
    let a = Machine::new(t.path(), "pc", "PC");
    let b = Machine::new(t.path(), "note", "Notebook");
    a.play(b"capitulo 1");
    both_ways(&a, &b);
    b.follow();
    b.play(b"capitulo 2");
    both_ways(&a, &b);
    // Quem é mais novo vem da árvore, não do relógio.
    assert!(matches!(a.plan(), Action::Avancar { .. }), "{:?}", a.plan());
    let newest = b.head().unwrap();
    // Retenção rodando com o relógio 2 h adiantado ou atrasado não apaga o que está em uso.
    let now = relicario_store::time::now();
    for skew in [-2 * 3_600_000, 2 * 3_600_000] {
        for m in [&a, &b] {
            let forgot =
                m.with(|l| ops::apply_retention(l, Policy::default(), now + skew).unwrap());
            assert!(!forgot.contains(&newest));
        }
    }
    a.follow();
    assert_eq!(a.read("caption.sav"), b"capitulo 2");
}

#[test]
fn maquina_offline_por_trinta_dias_volta_sem_perder_nada() {
    let t = tempfile::tempdir().unwrap();
    let a = Machine::new(t.path(), "pc", "PC");
    let b = Machine::new(t.path(), "note", "Notebook");
    a.play(b"inicio");
    both_ways(&a, &b);
    b.follow();
    let old = b.head().unwrap();

    // A fica desligada. B joga muito, aplica a retenção como se 30 dias tivessem passado e
    // libera espaço.
    for i in 0..20 {
        b.play(format!("dia {i}").as_bytes());
    }
    let now = relicario_store::time::now();
    let forgot = b.with(|l| ops::apply_retention(l, Policy::default(), now + 30 * DIA).unwrap());
    assert!(!forgot.is_empty());
    b.with(|l| prune::prune(l.repo, &l.machine).unwrap());
    let r = Repository::open(Arc::new(b.remote())).unwrap();
    assert!(verify::verify(&r, true).unwrap().ok());
    drop(r);
    // O que A tinha como atual (o head que B conhece dela) continua guardado em B.
    assert!(!forgot.contains(&old));

    // A volta, sem ter jogado: só segue.
    both_ways(&a, &b);
    assert!(matches!(a.plan(), Action::Avancar { .. }), "{:?}", a.plan());
    a.follow();
    assert_eq!(a.read("caption.sav"), b"dia 19");

    // De novo, agora A joga offline enquanto B também joga: conflito, as duas versões guardadas.
    a.play(b"offline em A");
    b.play(b"enquanto isso em B");
    both_ways(&a, &b);
    let Action::Conflito { tips } = a.plan() else {
        panic!("{:?}", a.plan())
    };
    assert_eq!(tips.len(), 2);
    for (_, s) in &tips {
        assert!(a.with(|l| l.repo.get_snapshot(s).is_ok()));
    }
    assert_eq!(
        a.read("caption.sav"),
        b"offline em A",
        "nada sobrescrito sozinho"
    );
}
