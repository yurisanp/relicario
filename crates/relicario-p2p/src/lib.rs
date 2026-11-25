//! Sincronização direta entre máquinas do Relicário, sem servidor nem relay.
//!
//! - QUIC (`quinn`) com TLS 1.3 mútuo. Cada máquina tem um certificado próprio; o que vale é a
//!   impressão digital dele, gravada nos dois lados no pareamento.
//! - Pareamento por código de 6 dígitos com SPAKE2: quem não sabe o código não consegue se
//!   passar por nenhum dos lados, e cada código vale para uma tentativa só.
//! - Descoberta por mDNS na rede local; endereços conhecidos (inclusive nomes do Tailscale) e o
//!   "cartão de visita" gravado no `heads/` de cada máquina servem fora dela.
//! - O que trafega são os arquivos do repositório, pela mesma replicação usada com a nuvem
//!   ([`relicario_store::replicate`]): cada arquivo é conferido pelo hash antes de ser gravado.

mod client;
pub mod discovery;
mod identity;
mod pairing;
mod protocol;
mod server;
mod tls;

pub use client::{PeerClient, connect};
pub use identity::{Identity, fingerprint};
pub use pairing::{join, new_code};
pub use server::{Server, ServerOptions};

use serde::{Deserialize, Serialize};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::OnceLock;
use std::time::Duration;

/// Porta UDP padrão.
pub const DEFAULT_PORT: u16 = 47321;
/// Protocolo anunciado no ALPN.
pub const ALPN: &[u8] = b"relicario/1";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("TLS: {0}")]
    Tls(String),
    #[error("conexão: {0}")]
    Connection(String),
    #[error("protocolo: {0}")]
    Protocol(String),
    #[error("a outra máquina recusou: {0}")]
    Refused(String),
    #[error("não consegui falar com {0}")]
    Unreachable(String),
    #[error("a máquina do outro lado não é quem devia ser (impressão digital diferente)")]
    WrongPeer,
    #[error("código de pareamento errado")]
    BadCode,
    #[error(transparent)]
    Store(#[from] relicario_store::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Uma máquina pareada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub machine: String,
    pub name: String,
    pub fingerprint: String,
    /// `host:porta` conhecidos (IP, nome na rede local ou MagicDNS do Tailscale).
    #[serde(default)]
    pub addresses: Vec<String>,
}

/// Runtime compartilhado do QUIC. O resto do Relicário é síncrono.
pub(crate) fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("relicario-p2p")
            .enable_all()
            .build()
            .expect("runtime do p2p")
    })
}

/// Resolve `host:porta` (ou só `host`, com a porta padrão). Nomes que não resolvem são
/// ignorados.
pub fn resolve(addrs: &[String]) -> Vec<SocketAddr> {
    let mut out = Vec::new();
    for a in addrs {
        let found: Vec<SocketAddr> = if let Ok(s) = a.parse::<SocketAddr>() {
            vec![s]
        } else if let Ok(ip) = a.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
            vec![SocketAddr::new(ip, DEFAULT_PORT)]
        } else if a
            .rsplit_once(':')
            .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
        {
            a.as_str()
                .to_socket_addrs()
                .map(|i| i.collect())
                .unwrap_or_default()
        } else {
            (a.as_str(), DEFAULT_PORT)
                .to_socket_addrs()
                .map(|i| i.collect())
                .unwrap_or_default()
        };
        for s in found {
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// Endereços pelos quais esta máquina provavelmente é alcançável: IP de saída de cada família e
/// o nome da máquina.
pub fn local_addresses(port: u16, hostname: &str) -> Vec<String> {
    let mut v = Vec::new();
    for probe in ["192.0.2.1:9", "[2001:db8::1]:9"] {
        let bind = if probe.starts_with('[') {
            "[::]:0"
        } else {
            "0.0.0.0:0"
        };
        if let Ok(s) = std::net::UdpSocket::bind(bind)
            && s.connect(probe).is_ok()
            && let Ok(a) = s.local_addr()
            && !a.ip().is_unspecified()
            && !a.ip().is_loopback()
        {
            v.push(SocketAddr::new(a.ip(), port).to_string());
        }
    }
    if !hostname.is_empty() {
        v.push(format!("{hostname}:{port}"));
    }
    v
}

/// Replica o repositório local com um par: conecta (até `timeout`), confere a impressão digital e
/// copia o que falta nos dois sentidos.
pub fn sync_with(
    identity: &Identity,
    local: &dyn relicario_transport::Remote,
    me: &relicario_model::MachineId,
    peer: &Peer,
    extra_addrs: &[SocketAddr],
    timeout: Duration,
) -> Result<relicario_store::replicate::Report> {
    let mut addrs = extra_addrs.to_vec();
    for a in resolve(&peer.addresses) {
        if !addrs.contains(&a) {
            addrs.push(a);
        }
    }
    if addrs.is_empty() {
        return Err(Error::Unreachable(format!(
            "{} (nenhum endereço conhecido)",
            peer.name
        )));
    }
    let client = connect(
        identity,
        me.as_str(),
        &addrs,
        Some(&peer.fingerprint),
        timeout,
    )?;
    let rep = relicario_store::replicate::replicate(local, &client, me)?;
    client.close();
    Ok(rep)
}
