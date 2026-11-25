//! Camada de plataforma: caminhos, serviço do usuário, segredos, notificações e lançadores.
//!
//! - identificação do sistema, pastas padrão e nome da máquina;
//! - [`notify`]: notificações nativas;
//! - [`service`]: instalar o daemon como serviço do usuário (systemd `--user`, tarefa agendada
//!   no Windows, LaunchAgent no macOS).
//!
//! Segredos (chaveiro do sistema) chegam na fase 5.

pub mod deck;
pub mod notify;
pub mod secrets;
pub mod service;

use std::path::PathBuf;

/// Sistema operacional em que o Relicário está rodando.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Windows,
    MacOs,
    Outro,
}

impl Os {
    /// Sistema atual, decidido em tempo de compilação.
    pub const fn atual() -> Self {
        if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Outro
        }
    }

    pub const fn nome(self) -> &'static str {
        match self {
            Os::Linux => "Linux",
            Os::Windows => "Windows",
            Os::MacOs => "macOS",
            Os::Outro => "outro",
        }
    }
}

fn env_path(k: &str) -> Option<PathBuf> {
    std::env::var_os(k)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Pasta do usuário.
pub fn home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        env_path("USERPROFILE").or_else(|| env_path("HOME"))
    } else {
        env_path("HOME")
    }
}

/// Pasta de configuração do Relicário. `RELICARIO_CONFIG_DIR` substitui o padrão (útil para
/// testes e para rodar várias "máquinas" no mesmo computador).
pub fn config_dir() -> PathBuf {
    if let Some(p) = env_path("RELICARIO_CONFIG_DIR") {
        return p;
    }
    base_dir(Kind::Config)
}

/// Pasta de dados locais (estado reconstruível, logs).
pub fn data_dir() -> PathBuf {
    if let Some(p) = env_path("RELICARIO_DATA_DIR") {
        return p;
    }
    base_dir(Kind::Data)
}

/// Pasta de cache (arte dos jogos, manifesto).
pub fn cache_dir() -> PathBuf {
    if let Some(p) = env_path("RELICARIO_CACHE_DIR") {
        return p;
    }
    base_dir(Kind::Cache)
}

#[derive(Clone, Copy)]
enum Kind {
    Config,
    Data,
    Cache,
}

fn base_dir(kind: Kind) -> PathBuf {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    match Os::atual() {
        Os::Windows => {
            let base = match kind {
                Kind::Config => env_path("APPDATA"),
                Kind::Data | Kind::Cache => env_path("LOCALAPPDATA"),
            }
            .unwrap_or_else(|| home.join("AppData").join("Roaming"));
            let p = base.join("Relicario");
            if matches!(kind, Kind::Cache) {
                p.join("cache")
            } else {
                p
            }
        }
        Os::MacOs => match kind {
            Kind::Config | Kind::Data => home.join("Library/Application Support/Relicario"),
            Kind::Cache => home.join("Library/Caches/Relicario"),
        },
        _ => {
            let (var, fallback) = match kind {
                Kind::Config => ("XDG_CONFIG_HOME", ".config"),
                Kind::Data => ("XDG_DATA_HOME", ".local/share"),
                Kind::Cache => ("XDG_CACHE_HOME", ".cache"),
            };
            env_path(var)
                .unwrap_or_else(|| home.join(fallback))
                .join("relicario")
        }
    }
}

/// Nome desta máquina, para mostrar ao usuário.
pub fn hostname() -> String {
    for k in ["HOSTNAME", "COMPUTERNAME"] {
        if let Ok(v) = std::env::var(k)
            && !v.trim().is_empty()
        {
            return v.trim().to_string();
        }
    }
    for f in ["/etc/hostname", "/proc/sys/kernel/hostname"] {
        if let Ok(v) = std::fs::read_to_string(f)
            && !v.trim().is_empty()
        {
            return v.trim().to_string();
        }
    }
    "maquina".into()
}

/// Expande `~` no começo de um caminho.
pub fn expand_home(p: &str) -> PathBuf {
    if p == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from(p));
    }
    if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\"))
        && let Some(h) = home_dir()
    {
        return h.join(rest);
    }
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sistema_atual_e_conhecido_nos_runners_de_ci() {
        assert_ne!(Os::atual(), Os::Outro);
    }

    #[test]
    fn pastas_padrao_existem_como_caminho() {
        assert!(
            config_dir().ends_with("relicario")
                || config_dir().ends_with("Relicario")
                || std::env::var_os("RELICARIO_CONFIG_DIR").is_some()
        );
        assert!(!hostname().is_empty());
        if let Some(h) = home_dir() {
            assert_eq!(expand_home("~/x"), h.join("x"));
        }
        assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
    }
}
