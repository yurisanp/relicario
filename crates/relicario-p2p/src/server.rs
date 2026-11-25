//! Lado que escuta: serve o repositório local aos pares confiáveis e atende pareamentos.

use crate::identity::Identity;
use crate::pairing::{self, Session};
use crate::protocol::{self, Req, Resp};
use crate::{Error, Peer, Result, rt, tls};
use relicario_model::HeadsFile;
use relicario_store::replicate::{IMMUTABLE, verify_file};
use relicario_transport::{Kind, Remote};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

type Callback<T> = Arc<dyn Fn(T) + Send + Sync>;

/// Como o servidor se comporta.
#[derive(Clone)]
pub struct ServerOptions {
    pub machine: String,
    pub name: String,
    /// Repositório local servido.
    pub repo: Arc<dyn Remote>,
    /// Pares confiáveis (a lista é lida a cada conexão).
    pub peers: Arc<RwLock<Vec<Peer>>>,
    /// Chamado quando um par grava um head novo aqui.
    pub on_heads: Option<Callback<String>>,
    /// Chamado quando um pareamento termina bem.
    pub on_paired: Option<Callback<Peer>>,
}

impl std::fmt::Debug for ServerOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerOptions")
            .field("machine", &self.machine)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

struct Shared {
    opts: ServerOptions,
    fingerprint: String,
    pairing: Mutex<Option<Session>>,
    heads_lock: Mutex<()>,
}

/// Servidor rodando em segundo plano. Para quando é descartado.
pub struct Server {
    endpoints: Vec<quinn::Endpoint>,
    shared: Arc<Shared>,
    pub addr: SocketAddr,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl Server {
    /// Escuta em `bind` (porta 0 escolhe uma livre).
    pub fn start(id: &Identity, bind: SocketAddr, opts: ServerOptions) -> Result<Server> {
        Self::start_many(id, &[bind], opts)
    }

    /// Escuta em todas as interfaces, IPv4 e IPv6, na porta dada (0 escolhe uma livre).
    pub fn start_any(id: &Identity, port: u16, opts: ServerOptions) -> Result<Server> {
        Self::start_many(
            id,
            &[
                SocketAddr::from(([0u16; 8], port)),
                SocketAddr::from(([0u8; 4], port)),
            ],
            opts,
        )
    }

    /// Escuta em cada endereço que conseguir. `[::]` já atende IPv4 no Linux e no macOS, e aí o
    /// `0.0.0.0` na mesma porta falha sem problema; no Windows os dois sobem. Os seguintes usam a
    /// porta do primeiro.
    pub fn start_many(id: &Identity, binds: &[SocketAddr], opts: ServerOptions) -> Result<Server> {
        let shared = Arc::new(Shared {
            opts,
            fingerprint: id.fingerprint.clone(),
            pairing: Mutex::new(None),
            heads_lock: Mutex::new(()),
        });
        let mut endpoints: Vec<quinn::Endpoint> = Vec::new();
        let mut first_err = None;
        for b in binds {
            let mut b = *b;
            if let Some(first) = endpoints.first() {
                b.set_port(first.local_addr()?.port());
            }
            let ep = {
                let _g = rt().enter();
                quinn::Endpoint::server(tls::server_config(id)?, b)
            };
            match ep {
                Ok(ep) => {
                    let (e, sh) = (ep.clone(), shared.clone());
                    rt().spawn(async move {
                        while let Some(incoming) = e.accept().await {
                            let sh = sh.clone();
                            tokio::spawn(async move {
                                if let Ok(conn) = incoming.await {
                                    serve_conn(sh, conn).await;
                                }
                            });
                        }
                    });
                    endpoints.push(ep);
                }
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        let Some(first) = endpoints.first() else {
            return Err(first_err
                .map(Error::Io)
                .unwrap_or_else(|| Error::Connection("nenhum endereço".into())));
        };
        let addr = first.local_addr()?;
        Ok(Server {
            endpoints,
            shared,
            addr,
        })
    }

    /// Abre o pareamento com um código por `valid`. Uma tentativa errada encerra.
    pub fn start_pairing(&self, code: &str, valid: Duration) {
        *self.shared.pairing.lock().expect("lock") =
            Some(Session::new(code, Instant::now() + valid));
    }

    pub fn cancel_pairing(&self) {
        *self.shared.pairing.lock().expect("lock") = None;
    }

    /// Ainda esperando alguém entrar com o código?
    pub fn pairing_open(&self) -> bool {
        self.shared
            .pairing
            .lock()
            .expect("lock")
            .as_ref()
            .is_some_and(|s| s.open())
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        for e in &self.endpoints {
            e.close(0u32.into(), b"desligando");
        }
    }
}

async fn serve_conn(sh: Arc<Shared>, conn: quinn::Connection) {
    let Some(fp) = tls::peer_fingerprint(&conn) else {
        conn.close(1u32.into(), b"sem certificado");
        return;
    };
    // Quem entrou pelo pareamento nesta conexão passa a ser confiável sem reconectar.
    let paired = Arc::new(Mutex::new(None::<Peer>));
    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        let (sh, fp, paired) = (sh.clone(), fp.clone(), paired.clone());
        tokio::spawn(async move {
            let resp = match protocol::read::<Req>(&mut recv).await {
                Ok((req, body)) => {
                    let sh2 = sh.clone();
                    tokio::task::spawn_blocking(move || handle(&sh2, &fp, &paired, req, body))
                        .await
                        .unwrap_or_else(|e| {
                            (
                                Resp::Error {
                                    message: e.to_string(),
                                },
                                vec![],
                            )
                        })
                }
                Err(e) => (
                    Resp::Error {
                        message: e.to_string(),
                    },
                    vec![],
                ),
            };
            let _ = protocol::write(&mut send, &resp.0, &resp.1).await;
        });
    }
}

fn trusted(sh: &Shared, fp: &str, paired: &Mutex<Option<Peer>>) -> Option<Peer> {
    if let Some(p) = paired.lock().expect("lock").clone()
        && p.fingerprint == fp
    {
        return Some(p);
    }
    sh.opts
        .peers
        .read()
        .expect("lock")
        .iter()
        .find(|p| p.fingerprint == fp)
        .cloned()
}

fn err(m: impl Into<String>) -> (Resp, Vec<u8>) {
    (Resp::Error { message: m.into() }, vec![])
}

fn handle(
    sh: &Shared,
    fp: &str,
    paired: &Mutex<Option<Peer>>,
    req: Req,
    body: Vec<u8>,
) -> (Resp, Vec<u8>) {
    let who = trusted(sh, fp, paired);
    match req {
        Req::Hello { .. } => (
            Resp::Hello {
                machine: sh.opts.machine.clone(),
                name: sh.opts.name.clone(),
                trusted: who.is_some(),
            },
            vec![],
        ),
        Req::Pair { machine, name, msg } => {
            let mut s = sh.pairing.lock().expect("lock");
            let Some(sess) = s.as_mut().filter(|s| s.open()) else {
                return err("esta máquina não está esperando pareamento");
            };
            match sess.respond(&msg, &sh.fingerprint, fp, machine, name) {
                Ok((reply, mac)) => (
                    Resp::Pair {
                        machine: sh.opts.machine.clone(),
                        name: sh.opts.name.clone(),
                        msg: reply,
                        mac,
                    },
                    vec![],
                ),
                Err(e) => {
                    *s = None;
                    err(e.to_string())
                }
            }
        }
        Req::PairConfirm { mac } => {
            let mut s = sh.pairing.lock().expect("lock");
            let Some(sess) = s.take() else {
                return err("nenhum pareamento em andamento");
            };
            match sess.confirm(&mac, &sh.fingerprint, fp) {
                Ok(mut peer) => {
                    peer.fingerprint = fp.to_string();
                    *paired.lock().expect("lock") = Some(peer.clone());
                    if let Some(cb) = &sh.opts.on_paired {
                        cb(peer);
                    }
                    (Resp::Ok, vec![])
                }
                Err(e) => err(e.to_string()),
            }
        }
        other => {
            let Some(peer) = who else {
                return err("máquina não pareada");
            };
            match serve_repo(sh, &peer, other, body) {
                Ok(r) => r,
                Err(e) => err(e.to_string()),
            }
        }
    }
}

fn serve_repo(sh: &Shared, peer: &Peer, req: Req, body: Vec<u8>) -> Result<(Resp, Vec<u8>)> {
    let repo = &sh.opts.repo;
    let terr = |e: relicario_transport::Error| Error::Store(e.into());
    Ok(match req {
        Req::List { kind } => {
            let k = protocol::kind_from(&kind)?;
            if k == Kind::Locks {
                return Ok((Resp::Names { names: vec![] }, vec![]));
            }
            (
                Resp::Names {
                    names: repo.list(k).map_err(terr)?,
                },
                vec![],
            )
        }
        Req::Get { kind, name } => {
            let k = protocol::kind_from(&kind)?;
            match repo.get(k, &name).map_err(terr)? {
                Some(d) if k != Kind::Locks => (Resp::Data, d),
                _ => (Resp::NotFound, vec![]),
            }
        }
        Req::Put { kind, name } => {
            let k = protocol::kind_from(&kind)?;
            if IMMUTABLE.contains(&k) {
                if !verify_file(k, &name, &body) {
                    return Err(Error::Protocol(format!(
                        "{kind}/{name} não confere com o hash"
                    )));
                }
                repo.put(k, &name, &body).map_err(terr)?;
            } else if k == Kind::Heads {
                if name == sh.opts.machine {
                    return Err(Error::Protocol(
                        "só esta máquina escreve o próprio head".into(),
                    ));
                }
                if !verify_file(k, &name, &body) {
                    return Err(Error::Protocol(format!("heads/{name} inválido")));
                }
                let _g = sh.heads_lock.lock().expect("lock");
                let new: HeadsFile =
                    serde_json::from_slice(&body).map_err(|e| Error::Protocol(e.to_string()))?;
                let old: Option<HeadsFile> = repo
                    .get(k, &name)
                    .map_err(terr)?
                    .and_then(|b| serde_json::from_slice(&b).ok());
                if old.is_none_or(|o| new.updated_at > o.updated_at) {
                    repo.put(k, &name, &body).map_err(terr)?;
                    if let Some(cb) = &sh.opts.on_heads {
                        cb(name.clone());
                    }
                }
            } else {
                return Err(Error::Protocol(format!(
                    "{kind} não pode ser gravado por {}",
                    peer.name
                )));
            }
            (Resp::Ok, vec![])
        }
        _ => return Err(Error::Protocol("pedido inesperado".into())),
    })
}

/// Usado por [`pairing::join`].
pub(crate) use pairing::mac as pairing_mac;
