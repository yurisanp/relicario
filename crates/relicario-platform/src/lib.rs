//! Camada de plataforma: caminhos, serviço do usuário, segredos, notificações e lançadores.
//!
//! Fase 0: só identifica o sistema. O resto chega nas fases 2 e 3 do roadmap.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sistema_atual_e_conhecido_nos_runners_de_ci() {
        assert_ne!(Os::atual(), Os::Outro);
    }
}
