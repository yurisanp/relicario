//! `relicario pair` e `relicario peers`: sincronização direta entre máquinas.

use clap::Subcommand;
use relicario_daemon::config::Config;
use relicario_daemon::ipc::{self, Request, Response};
use relicario_daemon::p2p;
use std::process::ExitCode;
use std::time::{Duration, Instant};

type R<T> = Result<T, String>;

#[derive(Subcommand)]
pub enum PeersCmd {
    /// Lista as máquinas pareadas
    List,
    /// Esquece uma máquina (pelo nome ou id)
    Remove { machine: String },
    /// Replica agora com todas as máquinas pareadas
    Sync,
    /// Procura máquinas do Relicário na rede local
    Discover,
}

fn ask(req: Request) -> R<Vec<String>> {
    match ipc::request(&req).map_err(|e| e.to_string())? {
        Response::Ok { messages } => Ok(messages),
        Response::Error { message } => Err(message),
        _ => Ok(vec![]),
    }
}

/// Sem código: esta máquina mostra um código e espera. Com código: entra no pareamento aberto
/// em outra máquina (achada na rede local ou por `--addr`).
pub fn cmd_pair(code: Option<String>, addr: Option<String>, wait_secs: u64) -> R<ExitCode> {
    let cfg = Config::load()?;
    let id = p2p::identity()?;
    // O outro lado acha esta máquina pelos endereços publicados no repositório.
    p2p::publish_card(&cfg);
    match code {
        Some(code) => {
            let targets = p2p::pairing_targets(addr.as_deref(), Duration::from_secs(3));
            if targets.is_empty() {
                return Err(
                    "não achei nenhuma máquina esperando pareamento na rede; use --addr host:porta"
                        .into(),
                );
            }
            let peer = relicario_p2p::join(
                &id,
                &cfg.machine_id,
                &cfg.machine_name,
                &targets,
                &code,
                Duration::from_secs(5),
            )
            .map_err(|e| e.to_string())?;
            let name = peer.name.clone();
            let cfg = p2p::save_peer(peer)?;
            println!("pareado com {name}");
            if ipc::is_up() {
                let _ = ask(Request::Reload);
                ask(Request::PeersSync)?
                    .iter()
                    .for_each(|m| println!("{m}"));
            } else {
                for r in p2p::sync_peers(&cfg, &[], Duration::from_secs(5)) {
                    println!("{}", r.message);
                }
            }
        }
        None => {
            let code = relicario_p2p::new_code();
            let wait = Duration::from_secs(wait_secs);
            if ipc::is_up() {
                ask(Request::PairStart { code: code.clone() })?;
                show_code(&code, &cfg);
                let end = Instant::now() + wait;
                loop {
                    std::thread::sleep(Duration::from_millis(300));
                    let st = ask(Request::PairStatus)?;
                    let st = st.first().cloned().unwrap_or_default();
                    if st.starts_with("pareado") {
                        println!("{st}");
                        break;
                    }
                    if st == "encerrado" {
                        return Err(
                            "o pareamento falhou (código errado?); rode `relicario pair` de novo"
                                .into(),
                        );
                    }
                    if Instant::now() > end {
                        let _ = ask(Request::PairCancel);
                        return Err("ninguém entrou com o código a tempo".into());
                    }
                }
            } else {
                host_without_daemon(&cfg, &id, &code, wait)?;
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn show_code(code: &str, cfg: &Config) {
    println!(
        "Na outra máquina, rode:\n\n    relicario pair {} {}\n",
        &code[..3],
        &code[3..]
    );
    let port = cfg.port().unwrap_or(relicario_p2p::DEFAULT_PORT);
    let addrs = relicario_p2p::local_addresses(port, &relicario_platform::hostname());
    if !addrs.is_empty() {
        println!(
            "Se ela não achar esta máquina sozinha, use --addr com um destes: {}",
            addrs.join(", ")
        );
    }
    println!("Aguardando…");
}

fn host_without_daemon(
    cfg: &Config,
    id: &relicario_p2p::Identity,
    code: &str,
    wait: Duration,
) -> R<()> {
    use std::sync::{Arc, Mutex, RwLock};
    let port = cfg.port().unwrap_or(relicario_p2p::DEFAULT_PORT);
    let got = Arc::new(Mutex::new(None));
    let g = got.clone();
    let opts = relicario_p2p::ServerOptions {
        machine: cfg.machine_id.clone(),
        name: cfg.machine_name.clone(),
        repo: Arc::new(relicario_transport::LocalFolder::new(cfg.repo_path())),
        peers: Arc::new(RwLock::new(cfg.peers.clone())),
        on_heads: None,
        on_paired: Some(Arc::new(move |p| *g.lock().expect("lock") = Some(p))),
    };
    let srv = relicario_p2p::Server::start_any(id, port, opts)
        .map_err(|e| format!("porta UDP {port}: {e}"))?;
    let _ad = relicario_p2p::discovery::advertise(
        &cfg.machine_id,
        &cfg.machine_name,
        &id.fingerprint,
        srv.port(),
        true,
    )
    .ok();
    srv.start_pairing(code, wait);
    show_code(code, cfg);
    loop {
        std::thread::sleep(Duration::from_millis(200));
        if let Some(p) = got.lock().expect("lock").take() {
            let name = p.name.clone();
            p2p::save_peer(p)?;
            println!("pareado com {name}");
            // Fica um pouco no ar para a outra máquina terminar a primeira replicação.
            std::thread::sleep(Duration::from_secs(3));
            return Ok(());
        }
        if !srv.pairing_open() {
            return Err("o pareamento falhou ou expirou; rode `relicario pair` de novo".into());
        }
    }
}

pub fn cmd_peers(c: PeersCmd) -> R<ExitCode> {
    match c {
        PeersCmd::List => {
            let cfg = Config::load()?;
            if cfg.peers.is_empty() {
                println!("nenhuma máquina pareada (use `relicario pair`)");
            }
            for p in &cfg.peers {
                println!(
                    "{}  {}  {}  {}",
                    p.name,
                    p.machine,
                    &p.fingerprint[..16.min(p.fingerprint.len())],
                    p.addresses.join(", ")
                );
            }
        }
        PeersCmd::Remove { machine } => {
            let mut cfg = Config::load()?;
            let before = cfg.peers.len();
            cfg.peers
                .retain(|p| p.machine != machine && !p.name.eq_ignore_ascii_case(&machine));
            if cfg.peers.len() == before {
                return Err(format!("nenhuma máquina pareada chamada {machine:?}"));
            }
            cfg.save()?;
            if ipc::is_up() {
                let _ = ask(Request::Reload);
            }
            println!("{machine} esquecida");
        }
        PeersCmd::Sync => {
            if ipc::is_up() {
                ask(Request::PeersSync)?
                    .iter()
                    .for_each(|m| println!("{m}"));
            } else {
                let cfg = Config::load()?;
                let r = p2p::sync_peers(&cfg, &[], Duration::from_secs(5));
                if r.is_empty() {
                    println!("nenhuma máquina pareada");
                }
                let mut fail = false;
                for x in r {
                    fail |= !x.ok;
                    println!("{}", x.message);
                }
                if fail {
                    return Ok(ExitCode::from(2));
                }
            }
        }
        PeersCmd::Discover => {
            let found = relicario_p2p::discovery::browse(Duration::from_secs(3));
            if found.is_empty() {
                println!("nenhuma máquina do Relicário encontrada na rede local");
            }
            for f in found {
                let a: Vec<String> = f.addrs.iter().map(|a| a.to_string()).collect();
                println!(
                    "{}  {}{}  {}",
                    f.name,
                    f.machine,
                    if f.pairing {
                        "  (esperando pareamento)"
                    } else {
                        ""
                    },
                    a.join(", ")
                );
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
