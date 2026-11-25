//! Registro local do daemon: uma linha JSON por evento em `<dados>/logs/relicario.log`, com
//! rotação por tamanho. Nada sai da máquina; o `relicario doctor --report` lê daqui e tira o
//! que identifica a pessoa antes de gravar o relatório.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Tamanho máximo de um arquivo antes de girar.
const MAX: u64 = 1024 * 1024;
/// Arquivos antigos guardados (`relicario.1.log` ... `relicario.3.log`).
const KEEP: usize = 3;

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Line {
    /// Milissegundos Unix.
    pub at: i64,
    /// `info`, `aviso` ou `erro`.
    pub level: String,
    pub msg: String,
}

pub fn dir() -> PathBuf {
    relicario_platform::data_dir().join("logs")
}

pub fn path() -> PathBuf {
    dir().join("relicario.log")
}

fn rotated(dir: &Path, n: usize) -> PathBuf {
    dir.join(format!("relicario.{n}.log"))
}

pub fn info(msg: &str) {
    write("info", msg);
}

pub fn warn(msg: &str) {
    write("aviso", msg);
}

pub fn error(msg: &str) {
    write("erro", msg);
}

/// Grava uma linha (e repete no stderr, para quem roda `relicario daemon run`). Falha ao gravar
/// nunca derruba o daemon.
pub fn write(level: &str, msg: &str) {
    eprintln!(
        "[{}] {msg}",
        relicario_store::time::format_utc(relicario_store::time::now())
    );
    write_in(&dir(), level, msg);
}

fn write_in(dir: &Path, level: &str, msg: &str) {
    let at = relicario_store::time::now();
    let line = Line {
        at,
        level: level.into(),
        msg: msg.into(),
    };
    let Ok(mut json) = serde_json::to_string(&line) else {
        return;
    };
    json.push('\n');
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = fs::create_dir_all(dir);
    let p = dir.join("relicario.log");
    if fs::metadata(&p).is_ok_and(|m| m.len() > MAX) {
        for n in (1..KEEP).rev() {
            let _ = fs::rename(rotated(dir, n), rotated(dir, n + 1));
        }
        let _ = fs::rename(&p, rotated(dir, 1));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&p) {
        let _ = f.write_all(json.as_bytes());
    }
}

/// As últimas `n` linhas, da mais antiga para a mais nova, olhando também os arquivos girados.
pub fn recent(n: usize) -> Vec<Line> {
    recent_in(&dir(), n)
}

fn recent_in(dir: &Path, n: usize) -> Vec<Line> {
    let mut files = vec![dir.join("relicario.log")];
    files.extend((1..=KEEP).map(|i| rotated(dir, i)));
    let mut out: Vec<Line> = Vec::new();
    for f in files {
        if out.len() >= n {
            break;
        }
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        let mut lines: Vec<Line> = text
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        lines.append(&mut out);
        out = lines;
    }
    let skip = out.len().saturating_sub(n);
    out.split_off(skip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grava_gira_e_le_as_ultimas() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        let big = "x".repeat(4000);
        for i in 0..1200 {
            write_in(d, "info", &format!("{i} {big}"));
        }
        assert!(rotated(d, 1).exists());
        assert!(!rotated(d, KEEP + 1).exists());
        let r = recent_in(d, 5);
        assert_eq!(r.len(), 5);
        assert!(r[4].msg.starts_with("1199 "));
        assert!(r[0].msg.starts_with("1195 "));
        let many = recent_in(d, 400);
        assert_eq!(many.len(), 400);
        assert!(many[0].msg.starts_with("800 "));
    }
}
