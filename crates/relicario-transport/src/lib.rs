//! Destinos onde o repositório do Relicário fica guardado.
//!
//! Todo destino implementa [`Remote`]: um armazenamento "burro" de arquivos, sem servidor. As
//! garantias exigidas estão documentadas no trait; a suíte em [`conformance`] confere cada
//! implementação.
//!
//! Disponível: [`LocalFolder`] (pasta local, HD externo, pasta sincronizada) e [`Cloud`] (S3 e
//! compatíveis, WebDAV/Nextcloud e Google Drive, pelo `opendal`). A sincronização direta entre
//! máquinas fica no crate `relicario-p2p`.

use std::fmt;

pub mod cloud;
pub mod limit;
mod local;
pub use cloud::{Cloud, CloudSpec};
pub use limit::Limited;
pub use local::LocalFolder;

pub mod conformance;

/// Tipos de arquivo do repositório, cada um numa pasta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Config,
    Keys,
    Packs,
    Index,
    Snapshots,
    Heads,
    Locks,
}

impl Kind {
    pub const ALL: [Kind; 7] = [
        Kind::Config,
        Kind::Keys,
        Kind::Packs,
        Kind::Index,
        Kind::Snapshots,
        Kind::Heads,
        Kind::Locks,
    ];

    /// Nome da pasta no destino.
    pub const fn dir(self) -> &'static str {
        match self {
            Kind::Config => "",
            Kind::Keys => "keys",
            Kind::Packs => "packs",
            Kind::Index => "index",
            Kind::Snapshots => "snapshots",
            Kind::Heads => "heads",
            Kind::Locks => "locks",
        }
    }

    /// Packs são divididos em subpastas pelos 2 primeiros caracteres do nome.
    pub const fn sharded(self) -> bool {
        matches!(self, Kind::Packs)
    }
}

/// Erros de um destino.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("nome inválido no repositório: {0:?}")]
    InvalidName(String),
    #[error("erro de E/S em {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// Login expirado, revogado ou sem permissão: precisa do usuário.
    #[error("acesso negado pelo destino: {0}")]
    Auth(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Um destino de armazenamento.
///
/// Garantias exigidas de toda implementação:
/// - `put` é atômico: quem lê vê o arquivo inteiro antigo ou o inteiro novo, nunca metade.
/// - Depois que `put` retorna `Ok`, o arquivo sobrevive a queda de energia (quando o destino
///   permite garantir isso).
/// - `list` não devolve arquivos temporários de escritas interrompidas.
/// - `get` de um arquivo que não existe devolve `Ok(None)`.
/// - `delete` de um arquivo que não existe é `Ok(())`.
pub trait Remote: Send + Sync + fmt::Debug {
    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> Result<()>;
    fn get(&self, kind: Kind, name: &str) -> Result<Option<Vec<u8>>>;
    fn list(&self, kind: Kind) -> Result<Vec<String>>;
    fn delete(&self, kind: Kind, name: &str) -> Result<()>;
    /// Descrição curta para o usuário (por exemplo, o caminho da pasta).
    fn describe(&self) -> String;
}

/// Nomes aceitos: letras, números, `-`, `_` e `.`, sem começar com `.`.
pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidName(name.to_string()))
    }
}
