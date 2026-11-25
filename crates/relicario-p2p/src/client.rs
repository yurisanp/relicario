//! Lado que conecta: um par visto como um destino ([`Remote`]) comum.

use crate::identity::Identity;
use crate::protocol::{self, Req, Resp};
use crate::{Error, Result, rt, tls};
use relicario_transport::{Kind, Remote};
use std::net::SocketAddr;
use std::time::Duration;

/// Conexão aberta com um par.
#[derive(Debug)]
pub struct PeerClient {
    endpoint: quinn::Endpoint,
    conn: quinn::Connection,
    pub machine: String,
    pub name: String,
    pub fingerprint: String,
    /// O outro lado já confia nesta máquina?
    pub trusted: bool,
    addr: SocketAddr,
}

/// Conecta ao primeiro endereço que responder. Com `expected`, recusa se a impressão digital do
/// outro lado for diferente.
pub fn connect(
    id: &Identity,
    me: &str,
    addrs: &[SocketAddr],
    expected: Option<&str>,
    timeout: Duration,
) -> Result<PeerClient> {
    let cfg = tls::client_config(id)?;
    let me = me.to_string();
    rt().block_on(async move {
        let mut last = None;
        for &addr in addrs {
            let bind: SocketAddr = if addr.is_ipv6() {
                "[::]:0".parse().expect("addr")
            } else {
                "0.0.0.0:0".parse().expect("addr")
            };
            let mut ep = quinn::Endpoint::client(bind)?;
            ep.set_default_client_config(cfg.clone());
            let attempt = async {
                let conn = ep
                    .connect(addr, "relicario")
                    .map_err(|e| Error::Connection(e.to_string()))?
                    .await
                    .map_err(|e| Error::Connection(e.to_string()))?;
                let fp = tls::peer_fingerprint(&conn).ok_or(Error::WrongPeer)?;
                if expected.is_some_and(|e| e != fp) {
                    conn.close(0u32.into(), b"impressao digital");
                    return Err(Error::WrongPeer);
                }
                let resp = call(
                    &conn,
                    &Req::Hello {
                        machine: me.clone(),
                        name: String::new(),
                    },
                    &[],
                )
                .await?;
                match resp {
                    (
                        Resp::Hello {
                            machine,
                            name,
                            trusted,
                        },
                        _,
                    ) => Ok((conn, fp, machine, name, trusted)),
                    (Resp::Error { message }, _) => Err(Error::Refused(message)),
                    (r, _) => Err(Error::Protocol(format!("resposta inesperada {r:?}"))),
                }
            };
            match tokio::time::timeout(timeout, attempt).await {
                Ok(Ok((conn, fingerprint, machine, name, trusted))) => {
                    return Ok(PeerClient {
                        endpoint: ep,
                        conn,
                        machine,
                        name,
                        fingerprint,
                        trusted,
                        addr,
                    });
                }
                Ok(Err(e @ Error::WrongPeer)) => return Err(e),
                Ok(Err(e)) => last = Some(e),
                Err(_) => last = Some(Error::Unreachable(addr.to_string())),
            }
        }
        Err(last.unwrap_or_else(|| Error::Unreachable("nenhum endereço".into())))
    })
}

pub(crate) async fn call(
    conn: &quinn::Connection,
    req: &Req,
    body: &[u8],
) -> Result<(Resp, Vec<u8>)> {
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| Error::Connection(e.to_string()))?;
    protocol::write(&mut send, req, body).await?;
    protocol::read(&mut recv).await
}

impl PeerClient {
    pub(crate) fn request(&self, req: &Req, body: &[u8]) -> Result<(Resp, Vec<u8>)> {
        let r = rt().block_on(async {
            tokio::time::timeout(Duration::from_secs(120), call(&self.conn, req, body)).await
        });
        match r {
            Ok(Ok((Resp::Error { message }, _))) => Err(Error::Refused(message)),
            Ok(other) => other,
            Err(_) => Err(Error::Unreachable(format!(
                "{} (tempo esgotado)",
                self.addr
            ))),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn close(&self) {
        self.conn.close(0u32.into(), b"tchau");
        let ep = self.endpoint.clone();
        rt().block_on(async move {
            let _ = tokio::time::timeout(Duration::from_millis(300), ep.wait_idle()).await;
        });
    }
}

fn terr(e: Error) -> relicario_transport::Error {
    relicario_transport::Error::Other(e.to_string())
}

impl Remote for PeerClient {
    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> relicario_transport::Result<()> {
        match self
            .request(
                &Req::Put {
                    kind: protocol::kind_name(kind).into(),
                    name: name.into(),
                },
                data,
            )
            .map_err(terr)?
        {
            (Resp::Ok, _) => Ok(()),
            (r, _) => Err(terr(Error::Protocol(format!("resposta inesperada {r:?}")))),
        }
    }

    fn get(&self, kind: Kind, name: &str) -> relicario_transport::Result<Option<Vec<u8>>> {
        match self
            .request(
                &Req::Get {
                    kind: protocol::kind_name(kind).into(),
                    name: name.into(),
                },
                &[],
            )
            .map_err(terr)?
        {
            (Resp::Data, body) => Ok(Some(body)),
            (Resp::NotFound, _) => Ok(None),
            (r, _) => Err(terr(Error::Protocol(format!("resposta inesperada {r:?}")))),
        }
    }

    fn list(&self, kind: Kind) -> relicario_transport::Result<Vec<String>> {
        match self
            .request(
                &Req::List {
                    kind: protocol::kind_name(kind).into(),
                },
                &[],
            )
            .map_err(terr)?
        {
            (Resp::Names { names }, _) => Ok(names),
            (r, _) => Err(terr(Error::Protocol(format!("resposta inesperada {r:?}")))),
        }
    }

    fn delete(&self, _: Kind, _: &str) -> relicario_transport::Result<()> {
        Err(relicario_transport::Error::Other(
            "um par não apaga arquivos do outro".into(),
        ))
    }

    fn describe(&self) -> String {
        format!(
            "{} ({})",
            if self.name.is_empty() {
                &self.machine
            } else {
                &self.name
            },
            self.addr
        )
    }
}
