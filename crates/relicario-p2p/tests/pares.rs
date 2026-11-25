//! Duas máquinas na mesma rede (localhost): pareamento, avanço, divergência e recusas.

use relicario_engine::Action;
use relicario_model::{GameId, MachineId, SnapshotReason};
use relicario_p2p::{Identity, Peer, Server, ServerOptions, join, new_code, sync_with};
use relicario_store::ops::{self, GameSpec, Local};
use relicario_store::{Filter, Repository, Source};
use relicario_transport::LocalFolder;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

struct M {
    id: MachineId,
    name: &'static str,
    dir: PathBuf,
    save: PathBuf,
    ident: Identity,
    peers: Arc<RwLock<Vec<Peer>>>,
}

impl M {
    fn new(root: &Path, id: &str, name: &'static str) -> M {
        let m = M {
            id: MachineId::new(id),
            name,
            dir: root.join(format!("repo-{id}")),
            save: root.join(format!("save-{id}")),
            ident: Identity::load_or_create(&root.join(format!("id-{id}"))).unwrap(),
            peers: Arc::new(RwLock::new(vec![])),
        };
        fs::create_dir_all(&m.save).unwrap();
        Repository::init(Arc::new(LocalFolder::new(&m.dir))).unwrap();
        m
    }
    fn spec(&self) -> GameSpec {
        GameSpec {
            id: GameId::new("hookshot"),
            sources: vec![Source {
                root: "save".into(),
                path: self.save.clone(),
                filter: Filter::new(&[], &[]).unwrap(),
                mirrors: vec![],
            }],
        }
    }
    fn with<T>(&self, f: impl FnOnce(&mut Local<'_>) -> T) -> T {
        let mut r = Repository::open(Arc::new(LocalFolder::new(&self.dir))).unwrap();
        let mut l = Local {
            repo: &mut r,
            machine: self.id.clone(),
            machine_name: self.name.into(),
        };
        f(&mut l)
    }
    fn play(&self, s: &[u8]) {
        fs::write(self.save.join("caption.sav"), s).unwrap();
        let spec = self.spec();
        self.with(|l| ops::take_snapshot(l, &spec, SnapshotReason::GameExit, false).unwrap());
    }
    fn plan(&self) -> Action {
        let spec = self.spec();
        self.with(|l| ops::plan(l, &spec).unwrap().0.action)
    }
    fn server(&self, paired: Arc<Mutex<Vec<Peer>>>) -> Server {
        let peers = self.peers.clone();
        let opts = ServerOptions {
            machine: self.id.to_string(),
            name: self.name.into(),
            repo: Arc::new(LocalFolder::new(&self.dir)),
            peers: self.peers.clone(),
            on_heads: None,
            on_paired: Some(Arc::new(move |p: Peer| {
                peers.write().unwrap().push(p.clone());
                paired.lock().unwrap().push(p);
            })),
        };
        Server::start(&self.ident, "127.0.0.1:0".parse().unwrap(), opts).unwrap()
    }
}

const T: Duration = Duration::from_secs(5);

#[test]
fn parear_avancar_divergir() {
    let t = tempfile::tempdir().unwrap();
    let a = M::new(t.path(), "pc", "PC");
    let b = M::new(t.path(), "note", "Notebook");
    let paired = Arc::new(Mutex::new(vec![]));
    let srv = a.server(paired.clone());
    let addr: Vec<SocketAddr> = vec![srv.addr];

    // Sem parear, nada feito.
    let fake = Peer {
        machine: "pc".into(),
        name: "PC".into(),
        fingerprint: a.ident.fingerprint.clone(),
        addresses: vec![srv.addr.to_string()],
    };
    let e = sync_with(&b.ident, &LocalFolder::new(&b.dir), &b.id, &fake, &[], T).unwrap_err();
    assert!(e.to_string().contains("não pareada"), "{e}");

    // Código errado encerra a sessão; o certo, numa sessão nova, funciona.
    let code = new_code();
    assert_eq!(code.len(), 6);
    srv.start_pairing(&code, Duration::from_secs(60));
    let wrong = if code == "000000" { "111111" } else { "000000" };
    assert!(join(&b.ident, "note", "Notebook", &addr, wrong, T).is_err());
    assert!(
        !srv.pairing_open() || join(&b.ident, "note", "Notebook", &addr, &code, T).is_err(),
        "uma tentativa só"
    );
    srv.start_pairing(&code, Duration::from_secs(60));
    let pa = join(&b.ident, "note", "Notebook", &addr, &code, T).unwrap();
    assert_eq!(pa.machine, "pc");
    assert_eq!(pa.fingerprint, a.ident.fingerprint);
    assert_eq!(paired.lock().unwrap()[0].fingerprint, b.ident.fingerprint);
    b.peers.write().unwrap().push(pa.clone());

    // Joga em A, B sincroniza e vê avanço.
    a.play(b"capitulo 1");
    let r = sync_with(&b.ident, &LocalFolder::new(&b.dir), &b.id, &pa, &[], T).unwrap();
    assert!(
        r.pulled > 0 && r.heads_in == vec!["pc".to_string()],
        "{r:?}"
    );
    let Action::Avancar { to, .. } = b.plan() else {
        panic!("{:?}", b.plan())
    };
    let spec = b.spec();
    b.with(|l| ops::restore(l, &spec, to).unwrap());
    assert_eq!(fs::read(b.save.join("caption.sav")).unwrap(), b"capitulo 1");

    // As duas jogam offline e reconectam: divergência nos dois lados.
    a.play(b"capitulo 2 no pc");
    b.play(b"capitulo 2 no notebook");
    let r = sync_with(&b.ident, &LocalFolder::new(&b.dir), &b.id, &pa, &[], T).unwrap();
    assert!(
        r.pushed > 0 && r.heads_out.contains(&"note".to_string()),
        "{r:?}"
    );
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
}

#[test]
fn impressao_digital_errada_e_recusada() {
    let t = tempfile::tempdir().unwrap();
    let a = M::new(t.path(), "pc", "PC");
    let b = M::new(t.path(), "note", "Notebook");
    let srv = a.server(Arc::new(Mutex::new(vec![])));
    let impostor = Peer {
        machine: "pc".into(),
        name: "PC".into(),
        fingerprint: "0".repeat(64),
        addresses: vec![srv.addr.to_string()],
    };
    let e = sync_with(
        &b.ident,
        &LocalFolder::new(&b.dir),
        &b.id,
        &impostor,
        &[],
        T,
    )
    .unwrap_err();
    assert!(matches!(e, relicario_p2p::Error::WrongPeer), "{e}");
}

#[test]
fn enderecos() {
    let r = relicario_p2p::resolve(&[
        "127.0.0.1".into(),
        "127.0.0.1:9000".into(),
        "::1".into(),
        "[::1]:5".into(),
        "nao-existe.invalid".into(),
    ]);
    assert_eq!(
        r,
        vec![
            "127.0.0.1:47321".parse::<SocketAddr>().unwrap(),
            "127.0.0.1:9000".parse().unwrap(),
            "[::1]:47321".parse().unwrap(),
            "[::1]:5".parse().unwrap()
        ]
    );
}
