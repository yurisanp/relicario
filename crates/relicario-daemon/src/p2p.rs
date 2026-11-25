//! Sincronização direta com as máquinas pareadas.

use crate::config::Config;
use relicario_model::MachineId;
use relicario_p2p::{Identity, Peer};
use relicario_transport::LocalFolder;
use std::net::SocketAddr;
use std::time::Duration;

/// Certificado desta máquina (criado na primeira vez).
pub fn identity() -> Result<Identity, String> {
    Identity::load_or_create(&relicario_platform::config_dir().join("p2p"))
        .map_err(|e| e.to_string())
}

/// Endereços que cada máquina publicou no próprio `heads/` (o "cartão de visita").
fn card(cfg: &Config, machine: &str) -> Vec<String> {
    cfg.open_repo()
        .ok()
        .and_then(|r| r.heads_of(&MachineId::new(machine)).ok().flatten())
        .map(|h| h.addresses)
        .unwrap_or_default()
}

/// Resultado com um par.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerSync {
    pub peer: String,
    pub ok: bool,
    /// Algum head de outra máquina mudou aqui?
    pub heads_in: bool,
    pub message: String,
}

/// Replica com cada par, com `timeout` para conectar. `discovered` vem do mDNS.
pub fn sync_peers(
    cfg: &Config,
    discovered: &[(String, Vec<SocketAddr>)],
    timeout: Duration,
) -> Vec<PeerSync> {
    if cfg.peers.is_empty() {
        return vec![];
    }
    let id = match identity() {
        Ok(i) => i,
        Err(e) => {
            return vec![PeerSync {
                peer: "-".into(),
                ok: false,
                heads_in: false,
                message: format!("certificado: {e}"),
            }];
        }
    };
    let local = LocalFolder::new(cfg.repo_path());
    let me = cfg.machine();
    let mut out = Vec::new();
    for p in &cfg.peers {
        let mut peer = p.clone();
        for a in card(cfg, &p.machine) {
            if !peer.addresses.contains(&a) {
                peer.addresses.push(a);
            }
        }
        let extra: Vec<SocketAddr> = discovered
            .iter()
            .filter(|(m, _)| *m == p.machine)
            .flat_map(|(_, a)| a.clone())
            .collect();
        let r = relicario_p2p::sync_with(&id, &local, &me, &peer, &extra, timeout);
        out.push(match r {
            Ok(rep) => PeerSync {
                peer: p.name.clone(),
                ok: true,
                heads_in: !rep.heads_in.is_empty(),
                message: if rep.changed() {
                    format!(
                        "{}: {} recebidos, {} enviados",
                        p.name,
                        rep.pulled + rep.heads_in.len(),
                        rep.pushed + rep.heads_out.len()
                    )
                } else {
                    format!("{}: em dia", p.name)
                },
            },
            Err(e) => PeerSync {
                peer: p.name.clone(),
                ok: false,
                heads_in: false,
                message: format!("{}: {e}", p.name),
            },
        });
    }
    out
}

/// Grava (ou atualiza) um par na configuração.
pub fn save_peer(peer: Peer) -> Result<Config, String> {
    let mut cfg = Config::load()?;
    cfg.peers
        .retain(|p| p.machine != peer.machine && p.fingerprint != peer.fingerprint);
    cfg.peers.push(peer);
    cfg.save()?;
    Ok(cfg)
}

/// Publica os endereços desta máquina no repositório.
pub fn publish_card(cfg: &Config) {
    let Some(port) = cfg.port() else { return };
    let addrs = relicario_p2p::local_addresses(port, &relicario_platform::hostname());
    if let Ok(mut repo) = cfg.open_repo() {
        let mut l = relicario_store::ops::Local {
            repo: &mut repo,
            machine: cfg.machine(),
            machine_name: cfg.machine_name.clone(),
        };
        let _ = relicario_store::ops::set_addresses(&mut l, addrs);
    }
}

/// Endereços para entrar num pareamento: os dados ou as máquinas anunciando pareamento na rede.
pub fn pairing_targets(addr: Option<&str>, wait: Duration) -> Vec<SocketAddr> {
    if let Some(a) = addr {
        return relicario_p2p::resolve(&[a.to_string()]);
    }
    relicario_p2p::discovery::browse(wait)
        .into_iter()
        .filter(|f| f.pairing)
        .flat_map(|f| f.addrs)
        .collect()
}
pub use relicario_p2p::new_code;
