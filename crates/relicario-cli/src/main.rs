//! `relicario`: linha de comando.

mod servico;

use relicario_daemon::{config, games};

use clap::{Parser, Subcommand};
use config::{Config, GameConfig, RootConfig};
use relicario_engine::Action;
use relicario_model::{Hash, SnapshotId, SnapshotReason};
use relicario_store::ops::{self, Local, SnapshotOutcome};
use relicario_store::time::format_utc;
use relicario_store::{Repository, prune, verify};
use relicario_transport::LocalFolder;
use std::process::ExitCode;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "relicario",
    version,
    about = "Mantém os saves dos seus jogos iguais em todas as máquinas",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Cria um repositório numa pasta (ou conecta a um existente) e configura esta máquina
    Init {
        /// Pasta do repositório: local, HD externo ou pasta sincronizada
        #[arg(long)]
        repo: String,
        /// Nome desta máquina
        #[arg(long)]
        name: Option<String>,
    },
    /// Cadastra e lista jogos
    #[command(subcommand)]
    Game(GameCmd),
    /// Tira snapshot dos jogos (todos, se nenhum for dado)
    Snapshot {
        games: Vec<String>,
        /// Promove a head mesmo se o save parecer corrompido
        #[arg(long)]
        force: bool,
    },
    /// Mostra o que seria feito com cada jogo, sem mudar nada
    Plan { games: Vec<String> },
    /// Aplica o plano: guarda o que mudou e traz o que está mais novo em outra máquina
    Sync { games: Vec<String> },
    /// Restaura um snapshot (por padrão, o mais novo segundo o plano)
    Restore {
        game: String,
        /// Id ou prefixo do snapshot
        snapshot: Option<String>,
    },
    /// Histórico de um jogo
    Log { game: String },
    /// Fixa um snapshot para nunca ser apagado pela retenção
    Pin {
        snapshot: String,
        /// Nome para lembrar (por exemplo, "antes do chefe")
        #[arg(long, default_value = "")]
        name: String,
    },
    /// Desfaz o pin
    Unpin { snapshot: String },
    /// Confere o repositório
    Verify {
        /// Lê e confere o hash de todos os blocos
        #[arg(long)]
        read_data: bool,
    },
    /// Aplica a retenção e libera espaço
    Prune,
    /// Procura jogos instalados e pastas de save nesta máquina
    Scan {
        /// Saída em JSON
        #[arg(long)]
        json: bool,
        /// Não tentar atualizar o manifesto pela rede
        #[arg(long)]
        offline: bool,
    },
    /// Manifesto de jogos (Ludusavi/PCGamingWiki)
    #[command(subcommand)]
    Manifest(ManifestCmd),
    /// Arte de um jogo (capa, banner, ícone, logo)
    Art {
        game: String,
        #[arg(long, default_value = "capa")]
        kind: String,
        /// Trocar pela imagem deste arquivo
        #[arg(long)]
        set: Option<String>,
        /// Esquecer a arte em cache e buscar de novo
        #[arg(long)]
        refresh: bool,
    },
    /// Mostra informações do ambiente
    Doctor,
    /// Abre um jogo sincronizando antes e depois (use nas opções de inicialização da loja)
    Run {
        /// Id do jogo (sem isso, descobre pela Steam ou pelo executável)
        #[arg(long)]
        game: Option<String>,
        /// Comando do jogo
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Controla o daemon
    #[command(subcommand)]
    Daemon(servico::DaemonCmd),
    /// Instala o daemon como serviço do usuário
    #[command(subcommand)]
    Service(servico::ServiceCmd),
    /// Integra com Steam, Heroic e Lutris
    #[command(subcommand)]
    Launcher(servico::LauncherCmd),
    /// Diz que um executável é de um jogo (vale para todas as máquinas)
    Associate {
        game: String,
        #[arg(long)]
        exe: String,
    },
    /// Executáveis desconhecidos vistos rodando
    #[command(subcommand)]
    Apps(servico::AppsCmd),
}

#[derive(Subcommand)]
enum ManifestCmd {
    /// Baixa o manifesto se mudou
    Update {
        /// Baixar mesmo se o cache for recente
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum GameCmd {
    /// Cadastra um jogo à mão
    Add {
        /// Identificador estável (o mesmo em todas as máquinas), por exemplo `hookshot`
        id: String,
        /// Pasta de save nesta máquina
        #[arg(long)]
        path: String,
        /// Nome lógico da raiz
        #[arg(long, default_value = "save")]
        root: String,
        /// Nome para mostrar
        #[arg(long)]
        name: Option<String>,
        /// Padrões a incluir (padrão: tudo)
        #[arg(long)]
        include: Vec<String>,
        /// Padrões a ignorar
        #[arg(long)]
        exclude: Vec<String>,
    },
    /// Lista os jogos cadastrados
    List,
    /// Remove o cadastro (não apaga nada do repositório nem do disco)
    Remove { id: String },
    /// Ignora um jogo detectado automaticamente
    Ignore { id: String },
    /// Volta a considerar um jogo ignorado
    Unignore { id: String },
    /// Ignora arquivos de um jogo (padrão relativo à pasta de save, por exemplo `backup/**`)
    Exclude { id: String, pattern: String },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.cmd) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("erro: {e}");
            ExitCode::FAILURE
        }
    }
}

type R<T> = Result<T, String>;

fn s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn open_repo(cfg: &Config) -> R<Repository> {
    Repository::open(Arc::new(LocalFolder::new(relicario_platform::expand_home(
        &cfg.repo.to_string_lossy(),
    ))))
    .map_err(s)
}

fn selected(cfg: &Config, names: &[String]) -> R<Vec<GameConfig>> {
    if names.is_empty() {
        Ok(games::all(cfg)?.into_iter().map(|g| g.config).collect())
    } else {
        names
            .iter()
            .map(|n| games::find(cfg, n).map(|g| g.config))
            .collect()
    }
}

fn resolve(repo: &Repository, prefix: &str) -> R<SnapshotId> {
    if let Ok(h) = prefix.parse::<Hash>() {
        return Ok(h);
    }
    if prefix.len() < 4 {
        return Err("use pelo menos 4 caracteres do id do snapshot".into());
    }
    let found: Vec<SnapshotId> = repo
        .snapshots()
        .map_err(s)?
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| id.to_hex().starts_with(prefix))
        .collect();
    match found.as_slice() {
        [one] => Ok(*one),
        [] => Err(format!("nenhum snapshot começa com {prefix}")),
        _ => Err(format!("{prefix} é ambíguo ({} snapshots)", found.len())),
    }
}

fn run(cmd: Cmd) -> R<ExitCode> {
    match cmd {
        Cmd::Init { repo, name } => cmd_init(repo, name),
        Cmd::Game(g) => cmd_game(g),
        Cmd::Doctor => cmd_doctor(),
        Cmd::Scan { json, offline } => cmd_scan(json, offline),
        Cmd::Manifest(ManifestCmd::Update { force }) => {
            let cache = relicario_catalog::manifest::Cache::default_dir();
            let out = relicario_catalog::manifest::update(&cache, force).map_err(s)?;
            println!(
                "{}",
                match out {
                    relicario_catalog::manifest::UpdateOutcome::Fresh =>
                        "manifesto em dia (conferido há menos de uma semana)",
                    relicario_catalog::manifest::UpdateOutcome::NotModified => "manifesto em dia",
                    relicario_catalog::manifest::UpdateOutcome::Downloaded =>
                        "manifesto atualizado",
                }
            );
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Art {
            game,
            kind,
            set,
            refresh,
        } => cmd_art(game, kind, set, refresh),
        Cmd::Run { game, command } => servico::cmd_run(game, command),
        Cmd::Daemon(c) => servico::cmd_daemon(c),
        Cmd::Service(c) => servico::cmd_service(c),
        Cmd::Launcher(c) => servico::cmd_launcher(c),
        Cmd::Associate { game, exe } => servico::cmd_associate(game, exe),
        Cmd::Apps(c) => servico::cmd_apps(c),
        other => {
            let cfg = Config::load()?;
            let mut repo = open_repo(&cfg)?;
            let mut l = Local {
                repo: &mut repo,
                machine: cfg.machine(),
                machine_name: cfg.machine_name.clone(),
            };
            match other {
                Cmd::Snapshot { games, force } => cmd_snapshot(&cfg, &mut l, &games, force),
                Cmd::Plan { games } => cmd_plan(&cfg, &l, &games),
                Cmd::Sync { games } => cmd_sync(&cfg, &mut l, &games),
                Cmd::Restore { game, snapshot } => cmd_restore(&cfg, &mut l, &game, snapshot),
                Cmd::Log { game } => cmd_log(&cfg, &l, &game),
                Cmd::Pin { snapshot, name } => {
                    let id = resolve(l.repo, &snapshot)?;
                    ops::pin(&mut l, id, name).map_err(s)?;
                    println!("fixado: {}", id.short());
                    Ok(ExitCode::SUCCESS)
                }
                Cmd::Unpin { snapshot } => {
                    let id = resolve(l.repo, &snapshot)?;
                    if ops::unpin(&mut l, id).map_err(s)? {
                        println!("pin removido: {}", id.short());
                    } else {
                        println!("{} não estava fixado", id.short());
                    }
                    Ok(ExitCode::SUCCESS)
                }
                Cmd::Verify { read_data } => cmd_verify(&l, read_data),
                Cmd::Prune => cmd_prune(&mut l),
                Cmd::Init { .. }
                | Cmd::Game(_)
                | Cmd::Doctor
                | Cmd::Scan { .. }
                | Cmd::Manifest(_)
                | Cmd::Art { .. }
                | Cmd::Run { .. }
                | Cmd::Daemon(_)
                | Cmd::Service(_)
                | Cmd::Launcher(_)
                | Cmd::Associate { .. }
                | Cmd::Apps(_) => {
                    unreachable!()
                }
            }
        }
    }
}

fn cmd_init(repo: String, name: Option<String>) -> R<ExitCode> {
    let path = relicario_platform::expand_home(&repo);
    let path = std::path::absolute(&path).map_err(s)?;
    let remote = Arc::new(LocalFolder::new(&path));
    match Repository::open(remote.clone()) {
        Ok(_) => println!("conectado ao repositório existente em {}", path.display()),
        Err(relicario_store::Error::NotARepository) => {
            Repository::init(remote).map_err(s)?;
            println!("repositório criado em {}", path.display());
        }
        Err(e) => return Err(e.to_string()),
    }
    let mut cfg = match Config::load() {
        Ok(c) => c,
        Err(_) => {
            let n = name.clone().unwrap_or_else(relicario_platform::hostname);
            Config {
                machine_id: config::new_machine_id(&n),
                machine_name: n,
                repo: path.clone(),
                games: vec![],
                auto_detect: true,
                rules: vec![],
                steamgriddb_key: None,
                periodic_secs: None,
            }
        }
    };
    cfg.repo = path;
    if let Some(n) = name {
        cfg.machine_name = n;
    }
    cfg.save()?;
    println!("esta máquina: {} ({})", cfg.machine_name, cfg.machine_id);
    println!("configuração em {}", config::path().display());
    Ok(ExitCode::SUCCESS)
}

fn cmd_game(g: GameCmd) -> R<ExitCode> {
    let mut cfg = Config::load()?;
    match g {
        GameCmd::Add {
            id,
            path,
            root,
            name,
            include,
            exclude,
        } => {
            if id.is_empty()
                || !id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            {
                return Err("id do jogo: use letras, números, -, _ ou .".into());
            }
            relicario_store::Filter::new(&include, &exclude).map_err(s)?;
            let entry = cfg.games.iter_mut().find(|x| x.id == id);
            let rc = RootConfig {
                name: root.clone(),
                path,
                include,
                exclude,
                mirrors: vec![],
            };
            match entry {
                Some(gc) => {
                    gc.roots.retain(|r| r.name != root);
                    gc.roots.push(rc);
                    if let Some(n) = name {
                        gc.name = n;
                    }
                }
                None => cfg.games.push(GameConfig {
                    id: id.clone(),
                    name: name.unwrap_or_default(),
                    roots: vec![rc],
                    executables: vec![],
                }),
            }
            cfg.save()?;
            println!("jogo {id} cadastrado");
        }
        GameCmd::List => {
            let all = games::all(&cfg)?;
            if all.is_empty() {
                println!(
                    "nenhum jogo encontrado; rode `relicario scan` ou use `relicario game add`"
                );
            }
            for game in &all {
                let g = &game.config;
                let origem = match &game.detected {
                    Some(d) => d.launcher.clone().unwrap_or_else(|| d.source.clone()),
                    None => "manual".into(),
                };
                println!("{} ({}) [{origem}]", g.display_name(), g.id);
                for r in &g.roots {
                    println!("  {}: {}", r.name, r.path);
                    for m in &r.mirrors {
                        println!("    espelho: {m}");
                    }
                    if !r.include.is_empty() {
                        println!("    inclui: {}", r.include.join(", "));
                    }
                    if !r.exclude.is_empty() {
                        println!("    ignora: {}", r.exclude.join(", "));
                    }
                }
            }
        }
        GameCmd::Remove { id } => {
            let before = cfg.games.len();
            cfg.games.retain(|g| g.id != id);
            if before == cfg.games.len() {
                return Err(format!("jogo {id:?} não cadastrado"));
            }
            cfg.save()?;
            println!("jogo {id} removido do cadastro (o histórico continua no repositório)");
        }
        GameCmd::Ignore { id } => {
            if !cfg.rules.iter().any(|r| r.game == id && r.ignore) {
                cfg.rules.push(relicario_catalog::rules::Rule {
                    game: id.clone(),
                    ignore: true,
                    ..Default::default()
                });
            }
            cfg.save()?;
            games::rescan(&cfg, false, true)?;
            println!("{id} ignorado");
        }
        GameCmd::Unignore { id } => {
            cfg.rules.retain(|r| !(r.game == id && r.ignore));
            cfg.save()?;
            games::rescan(&cfg, false, true)?;
            println!("{id} volta a ser considerado");
        }
        GameCmd::Exclude { id, pattern } => {
            relicario_store::Filter::new(&[], std::slice::from_ref(&pattern)).map_err(s)?;
            match cfg.rules.iter_mut().find(|r| r.game == id && !r.ignore) {
                Some(r) => r.exclude.push(pattern.clone()),
                None => cfg.rules.push(relicario_catalog::rules::Rule {
                    game: id.clone(),
                    exclude: vec![pattern.clone()],
                    ..Default::default()
                }),
            }
            if let Some(g) = cfg.games.iter_mut().find(|g| g.id == id) {
                for r in &mut g.roots {
                    r.exclude.push(pattern.clone());
                }
            }
            cfg.save()?;
            games::rescan(&cfg, false, true)?;
            println!("{id}: ignorando {pattern}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_snapshot(cfg: &Config, l: &mut Local<'_>, games: &[String], force: bool) -> R<ExitCode> {
    let mut code = ExitCode::SUCCESS;
    for g in selected(cfg, games)? {
        let started = std::time::Instant::now();
        match ops::take_snapshot(l, &g.spec()?, SnapshotReason::Manual, force).map_err(s)? {
            SnapshotOutcome::Unchanged(_) => println!("{}: sem mudanças", g.display_name()),
            SnapshotOutcome::Created(id) => println!(
                "{}: snapshot {} ({} ms)",
                g.display_name(),
                id.short(),
                started.elapsed().as_millis()
            ),
            SnapshotOutcome::Suspect(id, why) => {
                println!(
                    "{}: snapshot {} guardado, mas NÃO promovido: parece corrompido",
                    g.display_name(),
                    id.short()
                );
                for w in why {
                    println!("  - {}", describe_suspicion(&w));
                }
                println!("  se o save estiver certo, rode de novo com --force");
                code = ExitCode::from(3);
            }
        }
    }
    Ok(code)
}

fn describe_suspicion(s: &relicario_model::Suspicion) -> String {
    use relicario_model::Suspicion::*;
    match s {
        Shrunk { from, to } => format!("encolheu de {} para {}", human(*from), human(*to)),
        Emptied => "o save ficou vazio".into(),
        Zeroed { path } => format!("{path} virou só zeros"),
        TypeChanged { path } => format!("{path} mudou de formato"),
    }
}

fn human(b: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < 3 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

fn machine_label(l: &Local<'_>, m: &relicario_model::MachineId) -> String {
    if m.as_str() == "<local>" {
        return "esta máquina".into();
    }
    l.repo
        .heads_of(m)
        .ok()
        .flatten()
        .map(|h| h.machine_name)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| m.to_string())
}

fn describe_plan(l: &Local<'_>, p: &relicario_engine::Plan) -> String {
    let mut out = String::new();
    if p.dirty {
        out.push_str("mudou no disco; ");
    }
    out.push_str(&match &p.action {
        Action::Nada => "em dia".into(),
        Action::Enviar => "esta máquina está à frente".into(),
        Action::Adotar { to } => format!("já está igual a {}", to.short()),
        Action::Avancar { to, from } => format!(
            "{} está à frente: restaurar {}",
            machine_label(l, from),
            to.short()
        ),
        Action::Conflito { tips } => {
            let t: Vec<String> = tips
                .iter()
                .map(|(m, id)| format!("{} ({})", machine_label(l, m), id.short()))
                .collect();
            format!(
                "CONFLITO entre {}; nada será restaurado sozinho",
                t.join(", ")
            )
        }
    });
    out
}

fn cmd_plan(cfg: &Config, l: &Local<'_>, games: &[String]) -> R<ExitCode> {
    for g in selected(cfg, games)? {
        let (p, _) = ops::plan(l, &g.spec()?).map_err(s)?;
        println!("{}: {}", g.display_name(), describe_plan(l, &p));
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_sync(cfg: &Config, l: &mut Local<'_>, games: &[String]) -> R<ExitCode> {
    let mut code = ExitCode::SUCCESS;
    for g in selected(cfg, games)? {
        let spec = g.spec()?;
        let name = g.display_name();
        let (p, _) = ops::plan(l, &spec).map_err(s)?;
        if p.dirty && !matches!(p.action, Action::Avancar { .. } | Action::Adotar { .. }) {
            match ops::take_snapshot(l, &spec, SnapshotReason::Manual, false).map_err(s)? {
                SnapshotOutcome::Created(id) => println!("{name}: snapshot {}", id.short()),
                SnapshotOutcome::Suspect(id, _) => {
                    println!(
                        "{name}: snapshot {} parece corrompido; não sincronizado (veja `relicario snapshot`)",
                        id.short()
                    );
                    code = ExitCode::from(3);
                    continue;
                }
                SnapshotOutcome::Unchanged(_) => {}
            }
        }
        match p.action {
            Action::Nada | Action::Enviar => println!("{name}: em dia"),
            Action::Adotar { to } => {
                ops::adopt(l, &spec.id, to).map_err(s)?;
                println!("{name}: em dia ({})", to.short());
            }
            Action::Avancar { to, from } => {
                let out = ops::restore(l, &spec, to).map_err(s)?;
                if let Some(sf) = out.safety {
                    println!("{name}: cópia de segurança {}", sf.short());
                }
                println!(
                    "{name}: restaurado {} de {}",
                    to.short(),
                    machine_label(l, &from)
                );
            }
            Action::Conflito { .. } => {
                println!("{name}: {}", describe_plan(l, &p));
                println!(
                    "  escolha com `relicario restore {} <snapshot>` depois de ver `relicario log {}`",
                    g.id, g.id
                );
                code = ExitCode::from(4);
            }
        }
    }
    Ok(code)
}

fn cmd_restore(
    cfg: &Config,
    l: &mut Local<'_>,
    game: &str,
    snapshot: Option<String>,
) -> R<ExitCode> {
    let found = games::find(cfg, game)?;
    if servico::running_here(cfg, &found) {
        return Err(format!(
            "{} está aberto nesta máquina; feche o jogo antes de restaurar",
            found.title()
        ));
    }
    let g = found.config;
    let spec = g.spec()?;
    let target = match snapshot {
        Some(p) => resolve(l.repo, &p)?,
        None => {
            let (p, _) = ops::plan(l, &spec).map_err(s)?;
            match p.action {
                Action::Avancar { to, .. } | Action::Adotar { to } => to,
                Action::Conflito { .. } => {
                    return Err(format!(
                        "{}; diga qual snapshot restaurar",
                        describe_plan(l, &p)
                    ));
                }
                _ => l
                    .head_of(&spec.id)
                    .map_err(s)?
                    .ok_or("nenhum snapshot deste jogo")?,
            }
        }
    };
    let out = ops::restore(l, &spec, target).map_err(s)?;
    if let Some(sf) = out.safety {
        println!("cópia de segurança do estado anterior: {}", sf.short());
    }
    println!(
        "restaurado {}: {} escritos, {} iguais, {} removidos",
        target.short(),
        out.report.written.len(),
        out.report.unchanged.len(),
        out.report.removed.len()
    );
    Ok(ExitCode::SUCCESS)
}

fn cmd_log(cfg: &Config, l: &Local<'_>, game: &str) -> R<ExitCode> {
    let g = games::find(cfg, game)?.config;
    let snaps = ops::game_snapshots(l.repo, &relicario_model::GameId::new(&g.id)).map_err(s)?;
    let heads = l.repo.heads().map_err(s)?;
    let mut names = std::collections::HashMap::new();
    let mut pins = std::collections::HashMap::new();
    let mut tags: std::collections::HashMap<SnapshotId, Vec<String>> = Default::default();
    let mut forgotten = std::collections::HashSet::new();
    for h in &heads {
        let mname = if h.machine_name.is_empty() {
            h.machine
                .as_ref()
                .map(|m| m.to_string())
                .unwrap_or_default()
        } else {
            h.machine_name.clone()
        };
        if let Some(m) = &h.machine {
            names.insert(m.clone(), mname.clone());
        }
        if let Some(x) = h.heads.get(&relicario_model::GameId::new(&g.id)) {
            tags.entry(x.snapshot)
                .or_default()
                .push(format!("head de {mname}"));
        }
        pins.extend(h.pins.clone());
        forgotten.extend(h.forgotten.iter().copied());
    }
    if snaps.is_empty() {
        println!("nenhum snapshot de {}", g.display_name());
    }
    for (id, sn) in snaps.iter().rev() {
        let mut extra: Vec<String> = tags.remove(id).unwrap_or_default();
        if let Some(p) = pins.get(id) {
            extra.push(if p.is_empty() {
                "fixado".into()
            } else {
                format!("fixado: {p}")
            });
        }
        if forgotten.contains(id) {
            extra.push("dados liberados".into());
        }
        if !sn.suspicions.is_empty() {
            extra.push("suspeito".into());
        }
        if sn.parents.len() > 1 {
            extra.push("merge".into());
        }
        println!(
            "{}  {}  {:<12} {:>9}  {:?}{}",
            id.short(),
            format_utc(sn.created_at),
            names
                .get(&sn.machine)
                .cloned()
                .unwrap_or_else(|| sn.machine.to_string()),
            human(sn.size),
            sn.reason,
            if extra.is_empty() {
                String::new()
            } else {
                format!("  [{}]", extra.join(", "))
            }
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_verify(l: &Local<'_>, read_data: bool) -> R<ExitCode> {
    let r = verify::verify(l.repo, read_data).map_err(s)?;
    println!(
        "{} snapshots, {} packs, {} blocos lidos",
        r.snapshots, r.packs, r.blobs_checked
    );
    for w in &r.warnings {
        println!("aviso: {w}");
    }
    for e in &r.errors {
        println!("ERRO: {e}");
    }
    if r.ok() {
        println!("repositório íntegro");
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(5))
    }
}

fn cmd_prune(l: &mut Local<'_>) -> R<ExitCode> {
    let now = relicario_store::time::now();
    let forgot = ops::apply_retention(l, Default::default(), now).map_err(s)?;
    let m = l.machine.clone();
    let r = prune::prune(l.repo, &m).map_err(s)?;
    println!(
        "{} snapshots liberados pela retenção; {} packs removidos, {} reescritos, {} liberados",
        forgot.len(),
        r.packs_removed,
        r.packs_rewritten,
        human(r.bytes_freed)
    );
    Ok(ExitCode::SUCCESS)
}

fn cmd_doctor() -> R<ExitCode> {
    println!("relicario {}", env!("CARGO_PKG_VERSION"));
    println!("sistema: {}", relicario_platform::Os::atual().nome());
    println!("configuração: {}", config::path().display());
    match Config::load() {
        Ok(cfg) => {
            println!("máquina: {} ({})", cfg.machine_name, cfg.machine_id);
            println!("repositório: {}", cfg.repo.display());
            match open_repo(&cfg) {
                Ok(r) => println!(
                    "repositório abre: sim (formato v{})",
                    r.config().format_version
                ),
                Err(e) => println!("repositório abre: NÃO ({e})"),
            }
            println!("jogos cadastrados à mão: {}", cfg.games.len());
            println!(
                "detecção automática: {}",
                if cfg.auto_detect {
                    "ligada"
                } else {
                    "desligada"
                }
            );
            match games::load_scan() {
                Some(v) => println!("jogos detectados no último scan: {}", v.len()),
                None => println!("jogos detectados: ainda não rodou `relicario scan`"),
            }
        }
        Err(e) => println!("{e}"),
    }
    let cache = relicario_catalog::manifest::Cache::default_dir();
    match cache.age() {
        Some(a) => println!(
            "manifesto de jogos: em cache há {} dia(s)",
            a.as_secs() / 86_400
        ),
        None => println!("manifesto de jogos: não baixado (`relicario manifest update`)"),
    }
    let home = relicario_platform::home_dir().unwrap_or_default();
    let d = relicario_catalog::stores::discover(&home);
    for r in &d.steam_roots {
        println!("Steam: {}", r.display());
    }
    let mut by: std::collections::BTreeMap<&str, usize> = Default::default();
    for i in &d.installs {
        *by.entry(i.launcher).or_default() += 1;
    }
    for (l, n) in by {
        println!("instalados via {l}: {n}");
    }
    for p in &d.shared_prefixes {
        println!("prefixo Wine: {}", p.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_scan(json: bool, offline: bool) -> R<ExitCode> {
    let cfg = Config::load()?;
    let started = std::time::Instant::now();
    let found = games::rescan(&cfg, !offline, json)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&found).map_err(s)?);
        return Ok(ExitCode::SUCCESS);
    }
    for g in &found {
        let mut flags = Vec::new();
        if let Some(l) = &g.launcher {
            flags.push(l.clone());
        }
        if g.prefix.is_some() {
            flags.push("Proton/Wine".into());
        }
        if g.steam_cloud {
            flags.push("Steam Cloud".into());
        }
        if g.roots.iter().any(|r| !r.mirrors.is_empty()) {
            flags.push("espelhado".into());
        }
        println!(
            "{} ({})  {}  {}",
            g.title,
            g.id,
            human(g.size()),
            flags.join(", ")
        );
        for r in &g.roots {
            println!("  {}: {} ({} arquivos)", r.name, r.path.display(), r.files);
        }
    }
    println!(
        "{} jogos em {:.1} s",
        found.len(),
        started.elapsed().as_secs_f32()
    );
    Ok(ExitCode::SUCCESS)
}

fn cmd_art(game: String, kind: String, set: Option<String>, refresh: bool) -> R<ExitCode> {
    use relicario_catalog::art::{ArtCache, ArtKind, ArtQuery};
    let cfg = Config::load()?;
    let g = games::find(&cfg, &game)?;
    let kind = match kind.as_str() {
        "capa" => ArtKind::Capa,
        "banner" => ArtKind::Banner,
        "icone" | "ícone" => ArtKind::Icone,
        "logo" => ArtKind::Logo,
        k => {
            return Err(format!(
                "tipo de arte desconhecido: {k} (use capa, banner, icone ou logo)"
            ));
        }
    };
    let home = relicario_platform::home_dir().unwrap_or_default();
    let mut cache = ArtCache::new(ArtCache::default_dir());
    cache.steam_roots = relicario_catalog::stores::discover(&home).steam_roots;
    cache.lutris_dirs = vec![home.join(".local/share/lutris")];
    cache.steamgriddb_key = cfg.steamgriddb_key.clone();
    let d = g.detected.as_ref();
    let q = ArtQuery {
        id: g.id().to_string(),
        title: g.title().to_string(),
        steam_appid: d
            .filter(|d| d.store.as_deref() == Some("steam"))
            .and_then(|d| d.store_id.as_ref())
            .and_then(|i| i.parse().ok()),
        gog_id: d
            .filter(|d| d.store.as_deref() == Some("gog"))
            .and_then(|d| d.store_id.as_ref())
            .and_then(|i| i.parse().ok()),
        lutris_slug: d
            .filter(|d| d.launcher.as_deref() == Some("lutris"))
            .and_then(|d| d.store_id.clone()),
    };
    if refresh {
        cache.clear(&q.id, kind);
    }
    let info = match set {
        Some(f) => cache
            .set_override(&q.id, kind, &relicario_platform::expand_home(&f))
            .map_err(s)?,
        None => cache.get(&q, kind).map_err(s)?,
    };
    println!("{}", info.path.display());
    println!("fonte: {}", info.source);
    if let Some(a) = info.author {
        println!("autor: {a} (via SteamGridDB)");
    }
    Ok(ExitCode::SUCCESS)
}
