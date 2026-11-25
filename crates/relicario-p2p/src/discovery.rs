//! Descoberta na rede local por mDNS (`_relicario._udp.local.`).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

const SERVICE: &str = "_relicario._udp.local.";

/// Máquina anunciada na rede.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub machine: String,
    pub name: String,
    /// Começo da impressão digital (para mostrar e filtrar; a conferência é na conexão).
    pub fingerprint: String,
    pub pairing: bool,
    pub addrs: Vec<SocketAddr>,
}

/// Anúncio ativo. Para quando é descartado.
pub struct Advert {
    daemon: mdns_sd::ServiceDaemon,
    fullname: String,
}

impl std::fmt::Debug for Advert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Advert")
            .field("fullname", &self.fullname)
            .finish()
    }
}

impl Drop for Advert {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

fn instance(machine: &str) -> String {
    machine
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(60)
        .collect()
}

/// Anuncia esta máquina.
pub fn advertise(
    machine: &str,
    name: &str,
    fingerprint: &str,
    port: u16,
    pairing: bool,
) -> Result<Advert, String> {
    let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?;
    let inst = instance(machine);
    let host = format!("{inst}.local.");
    let props = [
        ("m", machine.to_string()),
        ("n", name.to_string()),
        ("fp", fingerprint.chars().take(16).collect::<String>()),
        ("pair", if pairing { "1".into() } else { "0".into() }),
    ];
    let info = mdns_sd::ServiceInfo::new(SERVICE, &inst, &host, "", port, &props[..])
        .map_err(|e| e.to_string())?
        .enable_addr_auto();
    let fullname = info.get_fullname().to_string();
    daemon.register(info).map_err(|e| e.to_string())?;
    Ok(Advert { daemon, fullname })
}

/// Procura máquinas por até `wait`.
pub fn browse(wait: Duration) -> Vec<Found> {
    let Ok(daemon) = mdns_sd::ServiceDaemon::new() else {
        return vec![];
    };
    let Ok(rx) = daemon.browse(SERVICE) else {
        return vec![];
    };
    let mut out: HashMap<String, Found> = HashMap::new();
    let end = Instant::now() + wait;
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) => {
                let get = |k: &str| info.get_property_val_str(k).unwrap_or("").to_string();
                let f = Found {
                    machine: get("m"),
                    name: get("n"),
                    fingerprint: get("fp"),
                    pairing: get("pair") == "1",
                    addrs: info
                        .get_addresses()
                        .iter()
                        .map(|ip| SocketAddr::new(*ip, info.get_port()))
                        .collect(),
                };
                if !f.machine.is_empty() {
                    out.insert(f.machine.clone(), f);
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.shutdown();
    let mut v: Vec<Found> = out.into_values().collect();
    v.sort_by(|a, b| a.machine.cmp(&b.machine));
    v
}
