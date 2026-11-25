//! O laço do daemon: vigia processos, sincroniza na abertura e no fechamento, faz a conferência
//! periódica, reescaneia de tempos em tempos e atende pedidos pelo socket.

use crate::config::Config;
use crate::games::{self, Game};
use crate::ipc::{self, Request, Response};
use crate::lifecycle::{self, SyncOutcome};
use crate::state::State;
use interprocess::local_socket::traits::ListenerExt as _;
use relicario_model::{GameId, SnapshotReason};
use relicario_store::ops::{self, Local};
use relicario_watch::{Event, Watcher};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn env_ms(name: &str, default: u64) -> Duration {
    Duration::from_millis(
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default),
    )
}

/// Intervalos do laço (ajustáveis por variável de ambiente para testes).
#[derive(Debug, Clone)]
pub struct Timing {
    pub poll: Duration,
    pub periodic: Duration,
    pub rescan: Duration,
}

impl Timing {
    pub fn from_env(cfg: &Config) -> Self {
        Timing {
            poll: env_ms("RELICARIO_POLL_MS", 2_000),
            periodic: cfg
                .periodic_secs
                .map(|s| Duration::from_secs(s.max(60)))
                .unwrap_or_else(|| env_ms("RELICARIO_PERIODIC_MS", 3_600_000)),
            rescan: env_ms("RELICARIO_RESCAN_MS", 6 * 3_600_000),
        }
    }
}

#[derive(Debug)]
struct Inner {
    cfg: Config,
    games: Vec<Game>,
    watcher: Watcher,
    /// Quando cada jogo aberto começou (para o tempo de jogo).
    started: BTreeMap<String, i64>,
    /// Jogos abertos pelo `relicario run`: o `run` cuida do antes e do depois.
    external: BTreeSet<String>,
    state: State,
    libraries: Vec<std::path::PathBuf>,
}

/// O daemon. `work` serializa as operações no repositório desta máquina.
#[derive(Debug)]
pub struct Daemon {
    inner: Mutex<Inner>,
    work: Mutex<()>,
}

fn log(msg: &str) {
    eprintln!("[{}] {msg}", relicario_store::time::now() / 1000);
}

impl Daemon {
    pub fn new(cfg: Config) -> Result<Arc<Self>, String> {
        let all = games::all(&cfg)?;
        let assoc = cfg
            .open_repo()
            .ok()
            .and_then(|r| ops::all_associations(&r).ok())
            .unwrap_or_default();
        let matcher = games::matcher(&all, &assoc);
        let home = relicario_platform::home_dir().unwrap_or_default();
        Ok(Arc::new(Daemon {
            inner: Mutex::new(Inner {
                cfg,
                games: all,
                watcher: Watcher::new(matcher),
                started: BTreeMap::new(),
                external: BTreeSet::new(),
                state: State::load(),
                libraries: games::library_dirs(&home),
            }),
            work: Mutex::new(()),
        }))
    }

    fn snapshot(&self) -> (Config, Vec<Game>) {
        let i = self.inner.lock().expect("lock");
        (i.cfg.clone(), i.games.clone())
    }

    fn game(&self, id: &str) -> Result<(Config, Game), String> {
        let (cfg, all) = self.snapshot();
        let g = all
            .into_iter()
            .find(|g| g.id() == id)
            .ok_or_else(|| format!("jogo {id:?} não encontrado"))?;
        Ok((cfg, g))
    }

    /// Recarrega a configuração e a lista de jogos (depois de `game add`, `associate`, scan...).
    pub fn reload(&self, rescan: bool) -> Result<(), String> {
        let cfg = Config::load()?;
        if rescan && cfg.auto_detect {
            games::rescan(&cfg, true, true)?;
        }
        let all = games::all(&cfg)?;
        let assoc = cfg
            .open_repo()
            .ok()
            .and_then(|r| ops::all_associations(&r).ok())
            .unwrap_or_default();
        let m = games::matcher(&all, &assoc);
        let mut i = self.inner.lock().expect("lock");
        i.watcher.set_matcher(m);
        i.cfg = cfg;
        i.games = all;
        Ok(())
    }

    pub fn handle(&self, req: Request) -> Response {
        let r: Result<Vec<String>, String> = match req {
            Request::Status => {
                let i = self.inner.lock().expect("lock");
                let mut running: BTreeSet<String> = i.watcher.running().cloned().collect();
                running.extend(i.external.iter().cloned());
                return Response::Status {
                    pid: std::process::id(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    machine: i.cfg.machine_name.clone(),
                    running: running.into_iter().collect(),
                    games: i.games.len(),
                };
            }
            Request::Stop => {
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(100));
                    log("parando a pedido");
                    std::process::exit(0);
                });
                Ok(vec!["parando".into()])
            }
            Request::Launching { game } => self.game(&game).and_then(|(cfg, g)| {
                self.inner
                    .lock()
                    .expect("lock")
                    .external
                    .insert(game.clone());
                let _w = self.work.lock().expect("lock");
                lifecycle::before_launch(&cfg, &g)
            }),
            Request::Exited { game, started_at } => self.game(&game).and_then(|(cfg, g)| {
                let r = {
                    let _w = self.work.lock().expect("lock");
                    lifecycle::after_exit(&cfg, &g, Some(started_at))
                };
                self.inner.lock().expect("lock").external.remove(&game);
                r
            }),
            Request::Sync { game } => self.sync(game.as_deref()),
            Request::Rescan => self
                .reload(true)
                .map(|_| vec!["lista de jogos atualizada".into()]),
        };
        match r {
            Ok(messages) => Response::Ok { messages },
            Err(message) => Response::Error { message },
        }
    }

    /// Sincroniza um jogo ou todos os que não estão abertos. Com o jogo aberto, nunca restaura.
    pub fn sync(&self, only: Option<&str>) -> Result<Vec<String>, String> {
        let (cfg, all) = self.snapshot();
        let running: BTreeSet<String> = {
            let i = self.inner.lock().expect("lock");
            i.watcher
                .running()
                .cloned()
                .chain(i.external.iter().cloned())
                .collect()
        };
        let mut msgs = Vec::new();
        for g in all.iter().filter(|g| only.is_none_or(|o| o == g.id())) {
            let open = running.contains(g.id());
            let _w = self.work.lock().expect("lock");
            match lifecycle::sync_game(&cfg, g, SnapshotReason::Periodic, !open) {
                Ok(SyncOutcome::EmDia) if only.is_none() => {}
                Ok(o) => {
                    if let SyncOutcome::Restaurado { .. } = o {
                        lifecycle::notify(g.title(), &o.describe(g.title()));
                    }
                    msgs.push(o.describe(g.title()));
                }
                Err(e) => msgs.push(format!("{}: erro: {e}", g.title())),
            }
        }
        if only.is_some() && msgs.is_empty() {
            return Err(format!(
                "jogo {:?} não encontrado",
                only.unwrap_or_default()
            ));
        }
        Ok(msgs)
    }

    fn on_event(&self, ev: Event) {
        let now = relicario_store::time::now();
        match ev {
            Event::Started { game, pid } => {
                {
                    let mut i = self.inner.lock().expect("lock");
                    if i.external.contains(&game) {
                        return;
                    }
                    i.started.insert(game.clone(), now);
                }
                log(&format!("{game} abriu (pid {pid})"));
                let Ok((cfg, g)) = self.game(&game) else {
                    return;
                };
                let _w = self.work.lock().expect("lock");
                // O jogo já está aberto: guarda o que havia antes, mas não restaura nada.
                match lifecycle::sync_game(&cfg, &g, SnapshotReason::Periodic, false) {
                    Ok(o @ SyncOutcome::AdiadoJogoAberto { .. }) => {
                        lifecycle::notify(g.title(), &o.describe(g.title()));
                        log(&o.describe(g.title()));
                    }
                    Ok(o) => log(&o.describe(g.title())),
                    Err(e) => log(&format!("{}: erro: {e}", g.title())),
                }
                if let Ok(mut repo) = cfg.open_repo() {
                    let mut l = Local {
                        repo: &mut repo,
                        machine: cfg.machine(),
                        machine_name: cfg.machine_name.clone(),
                    };
                    let id = GameId::new(g.id());
                    if let Ok(others) = ops::open_elsewhere(&l, &id, now) {
                        for (m, _) in others {
                            lifecycle::notify(
                                "Jogo aberto em outra máquina",
                                &format!("{} está aberto em {m}", g.title()),
                            );
                        }
                    }
                    let _ = ops::set_running(&mut l, &id, Some(now));
                }
            }
            Event::Stopped { game } => {
                let started = {
                    let mut i = self.inner.lock().expect("lock");
                    if i.external.contains(&game) {
                        return;
                    }
                    i.started.remove(&game)
                };
                log(&format!("{game} fechou"));
                let Ok((cfg, g)) = self.game(&game) else {
                    return;
                };
                let _w = self.work.lock().expect("lock");
                match lifecycle::after_exit(&cfg, &g, started) {
                    Ok(m) => m.iter().for_each(|m| log(m)),
                    Err(e) => log(&format!("{}: erro: {e}", g.title())),
                }
            }
        }
    }

    /// Executáveis desconhecidos em pastas de jogos: pergunta uma vez.
    fn check_unknown(&self, procs: &[relicario_watch::procs::ProcInfo]) {
        let mut i = self.inner.lock().expect("lock");
        let libs = i.libraries.clone();
        let cands: Vec<_> = i
            .watcher
            .matcher
            .unknown_candidates(procs, &libs)
            .into_iter()
            .cloned()
            .collect();
        let now = relicario_store::time::now();
        let mut changed = false;
        for p in cands {
            if i.state.saw_unknown(&p.name, p.exe.clone(), now) {
                changed = true;
                lifecycle::notify(
                    "Jogo desconhecido",
                    &format!(
                        "{} parece um jogo. Associe com `relicario associate <jogo> --exe {}` ou ignore com `relicario apps ignore {}`.",
                        p.name, p.name, p.name
                    ),
                );
            }
        }
        if changed {
            let _ = i.state.save();
        }
    }

    /// Uma volta do laço: processos, eventos e pendências.
    pub fn tick(&self) {
        let procs = relicario_watch::procs::list();
        let events = self.inner.lock().expect("lock").watcher.update(&procs);
        for ev in events {
            self.on_event(ev);
        }
        self.check_unknown(&procs);
    }
}

/// Roda o daemon até receber `Stop`.
pub fn run() -> Result<(), String> {
    if ipc::is_up() {
        return Err("o daemon já está rodando".into());
    }
    let cfg = Config::load()?;
    let timing = Timing::from_env(&cfg);
    let d = Daemon::new(cfg)?;
    let listener = ipc::listen()
        .map_err(|e| format!("não consegui abrir o socket {}: {e}", ipc::socket_name()))?;
    log(&format!(
        "relicario-daemon {} ouvindo em {}",
        env!("CARGO_PKG_VERSION"),
        ipc::socket_name()
    ));
    {
        let d = d.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                match conn {
                    Ok(c) => {
                        let d = d.clone();
                        std::thread::spawn(move || {
                            let _ = ipc::serve_one(c, |r| d.handle(r));
                        });
                    }
                    Err(e) => log(&format!("conexão recusada: {e}")),
                }
            }
        });
    }
    let mut last_periodic = Instant::now();
    let mut last_rescan = Instant::now();
    // Conferência inicial: traz o que outra máquina fez enquanto esta estava desligada.
    d.tick();
    if let Ok(m) = d.sync(None) {
        m.iter().for_each(|m| log(m));
    }
    loop {
        std::thread::sleep(timing.poll);
        d.tick();
        if last_rescan.elapsed() >= timing.rescan {
            last_rescan = Instant::now();
            if let Err(e) = d.reload(true) {
                log(&format!("reescaneamento falhou: {e}"));
            }
        }
        if last_periodic.elapsed() >= timing.periodic {
            last_periodic = Instant::now();
            let _ = d.reload(false);
            match d.sync(None) {
                Ok(m) => m.iter().for_each(|m| log(m)),
                Err(e) => log(&format!("conferência periódica falhou: {e}")),
            }
        }
    }
}
