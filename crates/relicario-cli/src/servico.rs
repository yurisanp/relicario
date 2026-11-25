//! Comandos que conversam com o daemon e com os lançadores: `run`, `daemon`, `service`,
//! `launcher`, `associate` e `apps`.

use clap::Subcommand;
use relicario_daemon::config::Config;
use relicario_daemon::games::{self, Game};
use relicario_daemon::ipc::{self, Request, Response};
use relicario_daemon::lifecycle;
use relicario_daemon::state::State;
use relicario_model::GameId;
use relicario_store::ops::{self, Local};
use std::path::PathBuf;
use std::process::ExitCode;

type R<T> = Result<T, String>;

#[derive(Subcommand)]
pub enum DaemonCmd {
    /// Roda o daemon neste terminal
    Run,
    /// Mostra se o daemon está rodando e quais jogos estão abertos
    Status,
    /// Para o daemon
    Stop,
}

#[derive(Subcommand)]
pub enum ServiceCmd {
    /// Faz o daemon iniciar sozinho quando você entra na sessão
    Install,
    /// Desfaz o `service install`
    Uninstall,
}

#[derive(Subcommand)]
pub enum LauncherCmd {
    /// Faz os lançadores abrirem os jogos pelo `relicario run`
    Install(LauncherSel),
    /// Desfaz o `launcher install`
    Uninstall(LauncherSel),
}

#[derive(clap::Args)]
pub struct LauncherSel {
    /// Só a Steam (precisa estar fechada)
    #[arg(long)]
    steam: bool,
    /// Só o Heroic
    #[arg(long)]
    heroic: bool,
    /// Só o Lutris
    #[arg(long)]
    lutris: bool,
}

#[derive(Subcommand)]
pub enum AppsCmd {
    /// Lista os executáveis desconhecidos vistos em pastas de jogos
    List,
    /// Não perguntar mais sobre um executável
    Ignore { name: String },
}

fn print_response(r: Response) -> R<()> {
    match r {
        Response::Ok { messages } => {
            messages.iter().for_each(|m| println!("{m}"));
            Ok(())
        }
        Response::Error { message } => Err(message),
        Response::Status {
            pid,
            version,
            machine,
            running,
            games,
        } => {
            println!("daemon {version} rodando (pid {pid}) em {machine}; {games} jogos conhecidos");
            if running.is_empty() {
                println!("nenhum jogo aberto");
            } else {
                println!("abertos: {}", running.join(", "));
            }
            Ok(())
        }
    }
}

pub fn cmd_daemon(c: DaemonCmd) -> R<ExitCode> {
    match c {
        DaemonCmd::Run => relicario_daemon::server::run()?,
        DaemonCmd::Status => match ipc::request(&Request::Status) {
            Ok(r) => print_response(r)?,
            Err(_) => {
                println!(
                    "o daemon não está rodando (inicie com `relicario daemon run` ou `relicario service install`)"
                );
                return Ok(ExitCode::from(3));
            }
        },
        DaemonCmd::Stop => match ipc::request(&Request::Stop) {
            Ok(r) => print_response(r)?,
            Err(_) => println!("o daemon não estava rodando"),
        },
    }
    Ok(ExitCode::SUCCESS)
}

pub fn cmd_service(c: ServiceCmd) -> R<ExitCode> {
    let msg = match c {
        ServiceCmd::Install => {
            let exe = relicario_platform::service::current_exe().map_err(|e| e.to_string())?;
            relicario_platform::service::install(&exe)?
        }
        ServiceCmd::Uninstall => relicario_platform::service::uninstall()?,
    };
    println!("{msg}");
    Ok(ExitCode::SUCCESS)
}

/// Descobre qual jogo o `run` vai abrir: `--game`, variáveis da Steam ou executável no comando.
pub fn identify(cfg: &Config, explicit: Option<&str>, command: &[String]) -> R<Option<Game>> {
    let all = games::all(cfg)?;
    if let Some(id) = explicit {
        return all
            .into_iter()
            .find(|g| g.id() == id)
            .map(Some)
            .ok_or_else(|| format!("jogo {id:?} não encontrado"));
    }
    for var in ["SteamAppId", "SteamGameId"] {
        if let Ok(appid) = std::env::var(var)
            && !appid.is_empty()
            && appid != "0"
            && let Some(g) = all.iter().find(|g| {
                g.detected.as_ref().is_some_and(|d| {
                    d.store.as_deref() == Some("steam")
                        && d.store_id.as_deref() == Some(appid.as_str())
                })
            })
        {
            return Ok(Some(g.clone()));
        }
    }
    let assoc = cfg
        .open_repo()
        .ok()
        .and_then(|r| ops::all_associations(&r).ok())
        .unwrap_or_default();
    let m = games::matcher(&all, &assoc);
    for arg in command {
        let p = PathBuf::from(arg);
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let proc = relicario_watch::procs::ProcInfo {
            pid: 0,
            name,
            exe: if p.is_absolute() {
                Some(p.clone())
            } else {
                None
            },
            cmdline: vec![arg.clone()],
        };
        if let Some(id) = m.match_proc(&proc)
            && let Some(g) = all.iter().find(|g| g.id() == id)
        {
            return Ok(Some(g.clone()));
        }
    }
    Ok(None)
}

/// `relicario run [--game X] -- comando`: sincroniza, abre o jogo, espera fechar e guarda. Nunca
/// impede o jogo de abrir: se algo der errado, avisa e segue.
pub fn cmd_run(game: Option<String>, command: Vec<String>) -> R<ExitCode> {
    let Some((prog, args)) = command.split_first() else {
        return Err("diga o comando do jogo depois de `--`".into());
    };
    let cfg = Config::load();
    let target = match &cfg {
        Ok(cfg) => match identify(cfg, game.as_deref(), &command) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("relicario: {e}; abrindo o jogo sem sincronizar");
                None
            }
        },
        Err(e) => {
            eprintln!("relicario: {e}; abrindo o jogo sem sincronizar");
            None
        }
    };
    let daemon = ipc::is_up();
    let started_at = relicario_store::time::now();
    if let (Ok(cfg), Some(g)) = (&cfg, &target) {
        let r = if daemon {
            ipc::request(&Request::Launching {
                game: g.id().to_string(),
            })
            .map_err(|e| e.to_string())
            .and_then(|r| match r {
                Response::Ok { messages } => Ok(messages),
                Response::Error { message } => Err(message),
                Response::Status { .. } => Ok(vec![]),
            })
        } else {
            lifecycle::before_launch(cfg, g)
        };
        match r {
            Ok(m) => m.iter().for_each(|m| eprintln!("relicario: {m}")),
            Err(e) => eprintln!(
                "relicario: não consegui sincronizar antes de abrir ({e}); abrindo assim mesmo"
            ),
        }
    } else if target.is_none() && cfg.is_ok() {
        eprintln!("relicario: não reconheci o jogo; use `relicario run --game <id> -- ...`");
    }
    let status = std::process::Command::new(prog).args(args).status();
    if let (Ok(cfg), Some(g)) = (&cfg, &target) {
        let r = if daemon && ipc::is_up() {
            ipc::request(&Request::Exited {
                game: g.id().to_string(),
                started_at,
            })
            .map_err(|e| e.to_string())
            .and_then(|r| match r {
                Response::Ok { messages } => Ok(messages),
                Response::Error { message } => Err(message),
                Response::Status { .. } => Ok(vec![]),
            })
        } else {
            lifecycle::after_exit(cfg, g, Some(started_at))
        };
        match r {
            Ok(m) => m.iter().for_each(|m| eprintln!("relicario: {m}")),
            Err(e) => {
                eprintln!("relicario: não consegui guardar o save ({e})");
                lifecycle::notify(g.title(), &format!("Não consegui guardar o save: {e}"));
            }
        }
    }
    match status {
        Ok(s) => Ok(s
            .code()
            .map(|c| ExitCode::from(c.clamp(0, 255) as u8))
            .unwrap_or(ExitCode::FAILURE)),
        Err(e) => Err(format!("não consegui abrir {prog}: {e}")),
    }
}

/// O jogo está aberto nesta máquina (pelo daemon ou pelos processos)?
pub fn running_here(cfg: &Config, game: &Game) -> bool {
    if let Ok(Response::Status { running, .. }) = ipc::request(&Request::Status)
        && running.iter().any(|r| r == game.id())
    {
        return true;
    }
    let assoc = cfg
        .open_repo()
        .ok()
        .and_then(|r| ops::all_associations(&r).ok())
        .unwrap_or_default();
    let all = games::all(cfg).unwrap_or_default();
    games::is_running(game, &games::matcher(&all, &assoc))
}

pub fn cmd_associate(game: String, exe: String) -> R<ExitCode> {
    let cfg = Config::load()?;
    let g = games::find(&cfg, &game)?;
    let name = PathBuf::from(&exe)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(exe);
    let mut repo = cfg.open_repo()?;
    let mut l = Local {
        repo: &mut repo,
        machine: cfg.machine(),
        machine_name: cfg.machine_name.clone(),
    };
    ops::associate(&mut l, &GameId::new(g.id()), &name).map_err(|e| e.to_string())?;
    let mut st = State::load();
    st.unknown.remove(&name.to_lowercase());
    st.save()?;
    if ipc::is_up() {
        let _ = ipc::request(&Request::Rescan);
    }
    println!(
        "{name} agora abre {} (vale para todas as máquinas)",
        g.title()
    );
    Ok(ExitCode::SUCCESS)
}

pub fn cmd_apps(c: AppsCmd) -> R<ExitCode> {
    let mut st = State::load();
    match c {
        AppsCmd::List => {
            let pending: Vec<_> = st.unknown.values().filter(|a| !a.ignored).collect();
            if pending.is_empty() {
                println!("nenhum executável desconhecido");
            }
            for a in pending {
                let p = a
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                println!("{}  {p}", a.name);
            }
        }
        AppsCmd::Ignore { name } => {
            let key = name.to_lowercase();
            let now = relicario_store::time::now();
            st.unknown
                .entry(key)
                .or_insert_with(|| relicario_daemon::state::UnknownApp {
                    name: name.clone(),
                    path: None,
                    first_seen: now,
                    ignored: true,
                })
                .ignored = true;
            st.save()?;
            println!("não vou mais perguntar sobre {name}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn steam_running() -> bool {
    relicario_watch::procs::list().iter().any(|p| {
        let n = p.name.to_lowercase();
        n == "steam" || n == "steam.exe" || n == "steamwebhelper" || n == "steam_osx"
    })
}

pub fn cmd_launcher(c: LauncherCmd) -> R<ExitCode> {
    let (sel, install) = match c {
        LauncherCmd::Install(s) => (s, true),
        LauncherCmd::Uninstall(s) => (s, false),
    };
    let all_sel = !sel.steam && !sel.heroic && !sel.lutris;
    let cfg = Config::load()?;
    let all = games::all(&cfg)?;
    let exe = relicario_platform::service::current_exe().map_err(|e| e.to_string())?;
    let home = relicario_platform::home_dir().ok_or("não achei a pasta do usuário")?;
    let by = |l: &str| -> Vec<&Game> {
        all.iter()
            .filter(|g| {
                g.detected
                    .as_ref()
                    .is_some_and(|d| d.launcher.as_deref() == Some(l) && d.store_id.is_some())
            })
            .collect()
    };
    let verb = if install { "instalado" } else { "removido" };
    if sel.steam || all_sel {
        let games = by("steam");
        let ids: Vec<String> = games
            .iter()
            .filter_map(|g| g.detected.as_ref()?.store_id.clone())
            .collect();
        let roots = relicario_catalog::stores::discover(&home).steam_roots;
        if roots.is_empty() || ids.is_empty() {
            if sel.steam {
                println!("Steam: nenhum jogo encontrado");
            }
        } else if steam_running() {
            println!(
                "Steam: feche a Steam e rode de novo (ela sobrescreve as opções de inicialização)"
            );
        } else {
            let mut n = 0;
            for r in &roots {
                for lc in relicario_catalog::launchers::steam_localconfigs(r) {
                    n += relicario_catalog::launchers::steam_apply(&lc, &ids, &exe, install)
                        .map_err(|e| e.to_string())?;
                }
            }
            println!("Steam: {verb} em {n} jogos (backup do localconfig.vdf ao lado)");
        }
    }
    if sel.heroic || all_sel {
        let dirs = relicario_catalog::stores::heroic::config_dirs(&home);
        let mut n = 0;
        for g in by("heroic") {
            let app = g
                .detected
                .as_ref()
                .and_then(|d| d.store_id.clone())
                .unwrap_or_default();
            for d in &dirs {
                if relicario_catalog::launchers::heroic_apply(d, &app, g.id(), &exe, install)
                    .map_err(|e| e.to_string())?
                {
                    n += 1;
                }
            }
        }
        if n > 0 || sel.heroic {
            println!("Heroic: {verb} em {n} jogos");
        }
    }
    if sel.lutris || all_sel {
        let mut n = 0;
        for g in by("lutris") {
            let slug = g
                .detected
                .as_ref()
                .and_then(|d| d.store_id.clone())
                .unwrap_or_default();
            for d in relicario_catalog::stores::lutris::config_dirs(&home) {
                let Ok(rd) = std::fs::read_dir(&d) else {
                    continue;
                };
                for f in rd.filter_map(|e| e.ok()).map(|e| e.path()) {
                    let name = f
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if name.ends_with(".yml")
                        && relicario_catalog::stores::lutris::slug_of(&name) == slug
                        && relicario_catalog::launchers::lutris_apply(&f, g.id(), &exe, install)
                            .map_err(|e| e.to_string())?
                    {
                        n += 1;
                    }
                }
            }
        }
        if n > 0 || sel.lutris {
            println!("Lutris: {verb} em {n} jogos");
        }
    }
    Ok(ExitCode::SUCCESS)
}
