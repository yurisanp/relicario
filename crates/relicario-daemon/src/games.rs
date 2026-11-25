//! Lista única de jogos: detectados pelo catálogo mais os cadastrados à mão.

use crate::config::{Config, GameConfig, RootConfig};
use relicario_catalog::manifest::{self, Cache, Manifest};
use relicario_catalog::{DetectedGame, ScanOptions};
use std::path::PathBuf;

/// Jogo pronto para usar.
#[derive(Debug, Clone)]
pub struct Game {
    pub config: GameConfig,
    pub detected: Option<DetectedGame>,
}

impl Game {
    pub fn id(&self) -> &str {
        &self.config.id
    }

    pub fn title(&self) -> &str {
        self.config.display_name()
    }
}

pub fn scan_cache_path() -> PathBuf {
    relicario_platform::data_dir().join("scan.json")
}

/// Lê o cache do último scan.
pub fn load_scan() -> Option<Vec<DetectedGame>> {
    serde_json::from_slice(&std::fs::read(scan_cache_path()).ok()?).ok()
}

/// Carrega o manifesto do cache, atualizando se `update` e se tiver mais de uma semana.
pub fn manifest(update: bool, quiet: bool) -> Option<Manifest> {
    let cache = Cache::default_dir();
    if update {
        match manifest::update(&cache, false) {
            Ok(manifest::UpdateOutcome::Downloaded) if !quiet => {
                eprintln!("manifesto de jogos atualizado")
            }
            Ok(_) => {}
            Err(e) if !quiet => eprintln!(
                "aviso: não consegui atualizar o manifesto ({e}); usando o que estiver em cache"
            ),
            Err(_) => {}
        }
    }
    match cache.load() {
        Ok(m) => m,
        Err(e) => {
            if !quiet {
                eprintln!("aviso: manifesto em cache ilegível: {e}");
            }
            None
        }
    }
}

/// Roda o scan e grava o cache.
pub fn rescan(cfg: &Config, update: bool, quiet: bool) -> Result<Vec<DetectedGame>, String> {
    let m = manifest(update, quiet);
    if m.is_none() && !quiet {
        eprintln!(
            "aviso: sem manifesto de jogos; só lojas com regras, emuladores e jogos cadastrados à mão"
        );
    }
    let opts = ScanOptions {
        rules: relicario_catalog::rules::merged(&cfg.rules),
        ..ScanOptions::default()
    };
    let found = relicario_catalog::scan(m.as_ref(), &opts);
    let p = scan_cache_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(&p, serde_json::to_vec_pretty(&found).expect("json"))
        .map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(found)
}

fn to_config(d: &DetectedGame) -> GameConfig {
    GameConfig {
        id: d.id.clone(),
        name: d.title.clone(),
        roots: d
            .roots
            .iter()
            .map(|r| RootConfig {
                name: r.name.clone(),
                path: r.path.to_string_lossy().into_owned(),
                include: r.include.clone(),
                exclude: r.exclude.clone(),
                mirrors: r
                    .mirrors
                    .iter()
                    .map(|m| m.to_string_lossy().into_owned())
                    .collect(),
            })
            .collect(),
        executables: d.executables.clone(),
    }
}

/// Todos os jogos, na ordem do título.
pub fn all(cfg: &Config) -> Result<Vec<Game>, String> {
    let mut out: Vec<Game> = Vec::new();
    if cfg.auto_detect {
        let detected = match load_scan() {
            Some(d) => d,
            None => rescan(cfg, false, true)?,
        };
        for d in detected {
            if cfg.rules.iter().any(|r| r.game == d.id && r.ignore) {
                continue;
            }
            out.push(Game {
                config: to_config(&d),
                detected: Some(d),
            });
        }
    }
    for g in &cfg.games {
        out.retain(|x| x.config.id != g.id);
        out.push(Game {
            config: g.clone(),
            detected: None,
        });
    }
    out.sort_by_key(|g| g.title().to_lowercase());
    Ok(out)
}

pub fn find(cfg: &Config, id: &str) -> Result<Game, String> {
    all(cfg)?.into_iter().find(|g| g.id() == id).ok_or_else(|| {
        format!("jogo {id:?} não encontrado; veja `relicario game list` ou rode `relicario scan`")
    })
}

/// Como reconhecer cada jogo entre os processos: executáveis do manifesto, pasta de instalação,
/// executáveis cadastrados e associações feitas pelo usuário em qualquer máquina.
pub fn matcher(
    all: &[Game],
    associations: &std::collections::BTreeMap<relicario_model::GameId, Vec<String>>,
) -> relicario_watch::Matcher {
    let mut v = Vec::new();
    for g in all {
        let mut m = relicario_watch::GameMatch {
            id: g.id().to_string(),
            ..Default::default()
        };
        for e in &g.config.executables {
            if e.contains('/') || e.contains('\\') {
                m.exe_paths.push(relicario_platform::expand_home(e));
            }
            m.exe_names.push(e.clone());
        }
        if let Some(d) = &g.detected
            && let Some(dir) = &d.install_dir
            && d.launcher.is_some()
        {
            m.install_dirs.push(dir.clone());
        }
        if let Some(a) = associations.get(&relicario_model::GameId::new(g.id())) {
            m.exe_names.extend(a.iter().cloned());
        }
        if !m.exe_names.is_empty() || !m.exe_paths.is_empty() || !m.install_dirs.is_empty() {
            v.push(m);
        }
    }
    relicario_watch::Matcher::new(v)
}

/// Pastas de biblioteca onde um executável desconhecido provavelmente é um jogo.
pub fn library_dirs(home: &std::path::Path) -> Vec<PathBuf> {
    let d = relicario_catalog::stores::discover(home);
    let mut v: Vec<PathBuf> = d
        .steam_roots
        .iter()
        .flat_map(|r| relicario_catalog::stores::steam::libraries(r))
        .map(|l| l.join("steamapps/common"))
        .collect();
    v.push(home.join("Games"));
    v.push(home.join("games"));
    v
}

/// O jogo está aberto agora nesta máquina?
pub fn is_running(game: &Game, matcher: &relicario_watch::Matcher) -> bool {
    relicario_watch::procs::list()
        .iter()
        .any(|p| matcher.match_proc(p) == Some(game.id()))
}
