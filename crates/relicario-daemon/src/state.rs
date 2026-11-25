//! Estado local do daemon (não vai para o repositório): o que já foi perguntado ao usuário.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Executável desconhecido visto rodando numa pasta de jogos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownApp {
    pub name: String,
    pub path: Option<PathBuf>,
    pub first_seen: i64,
    #[serde(default)]
    pub ignored: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    /// Por nome do executável em minúsculas.
    #[serde(default)]
    pub unknown: BTreeMap<String, UnknownApp>,
}

pub fn path() -> PathBuf {
    relicario_platform::data_dir().join("daemon-state.json")
}

impl State {
    pub fn load() -> Self {
        std::fs::read(path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).expect("json"))
            .and_then(|_| std::fs::rename(&tmp, &p))
            .map_err(|e| format!("{}: {e}", p.display()))
    }

    /// Registra um executável desconhecido. Devolve `true` se é a primeira vez (perguntar).
    pub fn saw_unknown(&mut self, name: &str, path: Option<PathBuf>, now: i64) -> bool {
        let key = name.to_lowercase();
        if self.unknown.contains_key(&key) {
            return false;
        }
        self.unknown.insert(
            key,
            UnknownApp {
                name: name.to_string(),
                path,
                first_seen: now,
                ignored: false,
            },
        );
        true
    }
}
