//! Tipos centrais do Relicário: jogos, máquinas, snapshots e heads.
//!
//! Este crate não depende do sistema operacional nem faz I/O.

use std::fmt;

macro_rules! id {
    ($(#[$m:meta])* $nome:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $nome(String);

        impl $nome {
            pub fn new(valor: impl Into<String>) -> Self {
                Self(valor.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $nome {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id!(
    /// Identificador estável de um jogo no catálogo.
    GameId
);
id!(
    /// Identificador de uma máquina que participa da sincronização.
    MachineId
);
id!(
    /// Identificador de um snapshot (hash do conteúdo, a partir da fase 1).
    SnapshotId
);

/// O último snapshot conhecido de um jogo numa máquina.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub game: GameId,
    pub machine: MachineId,
    pub snapshot: SnapshotId,
}

/// Relação entre o head local e o head de outra máquina.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// Os dois apontam para o mesmo snapshot.
    Igual,
    /// O remoto descende do local: dá para avançar sem perder nada.
    Avanco,
    /// O local descende do remoto: há algo novo para enviar.
    Envio,
    /// Os dois mudaram desde o ancestral comum: precisa de decisão.
    Divergiu,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_mostram_o_valor() {
        assert_eq!(GameId::new("hookshot").to_string(), "hookshot");
        assert_eq!(MachineId::new("pc").as_str(), "pc");
    }
}
