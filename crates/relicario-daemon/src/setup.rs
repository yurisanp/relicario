//! Primeira execução: cria o repositório local e a configuração desta máquina.

use crate::config::{self, Config};
use relicario_store::Repository;
use relicario_transport::LocalFolder;
use std::path::PathBuf;
use std::sync::Arc;

/// Onde fica o repositório local quando o usuário não escolhe.
pub fn default_repo() -> PathBuf {
    relicario_platform::data_dir().join("repositorio")
}

/// Já está configurado?
pub fn configured() -> bool {
    config::path().is_file()
}

/// Cria (ou conecta) o repositório e grava a configuração. Uma configuração existente mantém o
/// id da máquina, os jogos e os pares; só o nome e o repositório mudam.
pub fn init(name: &str, repo: Option<PathBuf>) -> Result<Config, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("dê um nome para esta máquina".into());
    }
    let path = repo.unwrap_or_else(default_repo);
    let path = std::path::absolute(&path).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let remote = Arc::new(LocalFolder::new(&path));
    match Repository::open(remote.clone()) {
        Ok(_) => {}
        Err(relicario_store::Error::NotARepository) => {
            Repository::init(remote).map_err(|e| e.to_string())?;
        }
        Err(e) => return Err(e.to_string()),
    }
    let mut cfg = Config::load().unwrap_or_else(|_| Config::new(name, path.clone()));
    cfg.machine_name = name.to_string();
    cfg.repo = path;
    cfg.save()?;
    Ok(cfg)
}
