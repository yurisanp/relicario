//! Atividade recente desta máquina (para a tela "Atividade" e a bandeja). Fica só nesta máquina,
//! em `atividade.jsonl`, com as últimas [`KEEP`] entradas.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// Quantas entradas guardar.
pub const KEEP: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// ms desde 1970.
    pub at: i64,
    /// `guardado`, `restaurado`, `conflito`, `suspeito`, `adiado`, `aberto`, `fechado`, `nuvem`,
    /// `par`, `pareamento`, `resolvido`, `fixado`, `erro`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
    pub text: String,
}

pub fn path() -> PathBuf {
    relicario_platform::data_dir().join("atividade.jsonl")
}

static LOCK: Mutex<()> = Mutex::new(());

/// Último resultado de cada jogo nesta execução (para a biblioteca mostrar o estado sem ler o
/// disco de novo).
static LAST: Mutex<BTreeMap<String, (i64, crate::lifecycle::SyncOutcome)>> =
    Mutex::new(BTreeMap::new());

pub fn set_last(game: &str, out: &crate::lifecycle::SyncOutcome) {
    LAST.lock().expect("lock").insert(
        game.to_string(),
        (relicario_store::time::now(), out.clone()),
    );
}

pub fn last() -> BTreeMap<String, (i64, crate::lifecycle::SyncOutcome)> {
    LAST.lock().expect("lock").clone()
}

pub fn clear_last(game: &str) {
    LAST.lock().expect("lock").remove(game);
}

/// Registra uma entrada. Erros de disco são ignorados: a atividade é só informativa.
pub fn record(kind: &str, game: Option<&str>, text: impl Into<String>) {
    let e = Entry {
        at: relicario_store::time::now(),
        kind: kind.to_string(),
        game: game.map(str::to_string),
        text: text.into(),
    };
    let _g = LOCK.lock().expect("lock");
    let p = path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    else {
        return;
    };
    let mut line = serde_json::to_string(&e).expect("json");
    line.push('\n');
    let _ = f.write_all(line.as_bytes());
    // De vez em quando, corta o arquivo para as últimas entradas.
    if f.metadata().map(|m| m.len()).unwrap_or(0) > (KEEP as u64) * 400 {
        drop(f);
        let all = read_all();
        let keep = &all[all.len().saturating_sub(KEEP)..];
        let text: String = keep
            .iter()
            .map(|e| serde_json::to_string(e).expect("json") + "\n")
            .collect();
        let _ = crate::config::write_atomic(&p, text.as_bytes());
    }
}

fn read_all() -> Vec<Entry> {
    std::fs::read_to_string(path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// As mais novas primeiro.
pub fn recent(limit: usize, game: Option<&str>) -> Vec<Entry> {
    let _g = LOCK.lock().expect("lock");
    let mut v: Vec<Entry> = read_all()
        .into_iter()
        .filter(|e| game.is_none_or(|g| e.game.as_deref() == Some(g)))
        .collect();
    v.reverse();
    v.truncate(limit);
    v
}
