//! Segredos (senha do cofre, chaves de nuvem) no chaveiro do sistema: Keychain no macOS,
//! Gerenciador de Credenciais no Windows e Secret Service (GNOME Keyring, KWallet) no Linux.
//!
//! Sem chaveiro disponível (modo jogo do Steam Deck, servidor sem sessão), cai para um arquivo
//! `segredos.json` na pasta de configuração, legível só pelo usuário, e avisa.

use std::collections::BTreeMap;
use std::path::PathBuf;

const SERVICE: &str = "relicario";

fn file() -> PathBuf {
    crate::config_dir().join("segredos.json")
}

fn read_file() -> BTreeMap<String, String> {
    std::fs::read(file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_file(m: &BTreeMap<String, String>) -> Result<(), String> {
    let p = file();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let data = serde_json::to_string_pretty(m).expect("json");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&p)
            .map_err(|e| e.to_string())?;
        f.write_all(data.as_bytes()).map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&p, data).map_err(|e| e.to_string())
    }
}

fn use_keyring() -> bool {
    std::env::var_os("RELICARIO_NO_KEYRING").is_none()
}

/// Onde os segredos estão guardados (para o `doctor`).
pub fn backend() -> &'static str {
    if use_keyring()
        && keyring::Entry::new(SERVICE, "teste").is_ok_and(|e| {
            !matches!(
                e.get_password(),
                Err(keyring::Error::PlatformFailure(_) | keyring::Error::NoStorageAccess(_))
            )
        })
    {
        "chaveiro do sistema"
    } else {
        "arquivo protegido na pasta de configuração"
    }
}

/// Guarda um segredo. Devolve onde ficou.
pub fn set(name: &str, value: &str) -> Result<&'static str, String> {
    if use_keyring()
        && let Ok(e) = keyring::Entry::new(SERVICE, name)
        && e.set_password(value).is_ok()
    {
        // Tira uma cópia antiga do arquivo, se houver.
        let mut m = read_file();
        if m.remove(name).is_some() {
            let _ = write_file(&m);
        }
        return Ok("chaveiro do sistema");
    }
    let mut m = read_file();
    m.insert(name.to_string(), value.to_string());
    write_file(&m)?;
    Ok("arquivo protegido na pasta de configuração")
}

pub fn get(name: &str) -> Option<String> {
    if use_keyring()
        && let Ok(e) = keyring::Entry::new(SERVICE, name)
        && let Ok(v) = e.get_password()
    {
        return Some(v);
    }
    read_file().get(name).cloned()
}

pub fn delete(name: &str) {
    if use_keyring()
        && let Ok(e) = keyring::Entry::new(SERVICE, name)
    {
        let _ = e.delete_credential();
    }
    let mut m = read_file();
    if m.remove(name).is_some() {
        let _ = write_file(&m);
    }
}
