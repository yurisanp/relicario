//! `config.toml` desta máquina.

use relicario_model::{GameId, MachineId};
use relicario_store::ops::GameSpec;
use relicario_store::{Filter, Source};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub machine_id: String,
    pub machine_name: String,
    /// Pasta do repositório (destino "pasta local").
    pub repo: PathBuf,
    /// Jogos cadastrados à mão. Ganham dos detectados com o mesmo id.
    #[serde(default)]
    pub games: Vec<GameConfig>,
    /// Detectar jogos automaticamente (lojas, manifesto, emuladores).
    #[serde(default = "yes")]
    pub auto_detect: bool,
    /// Regras locais por jogo (ignorar arquivos, ignorar o jogo, pastas extras).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<relicario_catalog::rules::Rule>,
    /// Chave de API do SteamGridDB, para a arte dos jogos (vai para o chaveiro do sistema na
    /// fase 5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steamgriddb_key: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameConfig {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub roots: Vec<RootConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootConfig {
    /// Nome lógico, igual em todas as máquinas.
    pub name: String,
    /// Caminho nesta máquina (aceita `~`).
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// Pastas espelhadas (mesmo conteúdo), restauradas junto.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirrors: Vec<String>,
}

pub fn path() -> PathBuf {
    relicario_platform::config_dir().join("config.toml")
}

impl Config {
    pub fn load() -> Result<Self, String> {
        let p = path();
        let text = std::fs::read_to_string(&p).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "ainda não configurado ({} não existe); rode `relicario init --repo <pasta>`",
                    p.display()
                )
            } else {
                format!("não consegui ler {}: {e}", p.display())
            }
        })?;
        toml::from_str(&text).map_err(|e| format!("{} inválido: {e}", p.display()))
    }

    pub fn save(&self) -> Result<(), String> {
        let p = path();
        write_atomic(
            &p,
            toml::to_string_pretty(self)
                .map_err(|e| e.to_string())?
                .as_bytes(),
        )
    }

    pub fn machine(&self) -> MachineId {
        MachineId::new(&self.machine_id)
    }
}

impl GameConfig {
    pub fn spec(&self) -> Result<GameSpec, String> {
        let mut sources = Vec::new();
        for r in &self.roots {
            sources.push(Source {
                root: r.name.clone(),
                path: relicario_platform::expand_home(&r.path),
                filter: Filter::new(&r.include, &r.exclude).map_err(|e| e.to_string())?,
                mirrors: r
                    .mirrors
                    .iter()
                    .map(|m| relicario_platform::expand_home(m))
                    .collect(),
            });
        }
        Ok(GameSpec {
            id: GameId::new(&self.id),
            sources,
        })
    }

    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }
}

fn write_atomic(p: &Path, data: &[u8]) -> Result<(), String> {
    let dir = p.parent().expect("config tem pasta");
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = dir.join(".config.toml.tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, p).map_err(|e| format!("{}: {e}", p.display()))
}

/// Gera um id de máquina a partir do nome.
pub fn new_machine_id(name: &str) -> String {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("aleatório");
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(24)
        .collect();
    let slug = if slug.is_empty() {
        "maquina".to_string()
    } else {
        slug
    };
    format!(
        "{slug}-{}",
        b.iter().map(|x| format!("{x:02x}")).collect::<String>()
    )
}
