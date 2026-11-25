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
