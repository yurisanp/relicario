use relicario_model::Hash;

/// Erros do repositório.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Remote(#[from] relicario_transport::Error),
    #[error("erro de E/S em {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("o destino não tem um repositório do Relicário (falta o arquivo config)")]
    NotARepository,
    #[error("já existe um repositório neste destino")]
    AlreadyExists,
    #[error("formato do repositório {0} não é suportado por esta versão")]
    UnsupportedVersion(u32),
    #[error("senha ou chave de recuperação errada")]
    WrongPassword,
    #[error("dado corrompido: {0}")]
    Corrupt(String),
    #[error("bloco {0} não encontrado no repositório")]
    MissingBlob(Hash),
    #[error("snapshot {0} não encontrado")]
    MissingSnapshot(Hash),
    #[error("repositório travado por {0}")]
    Locked(String),
    #[error("padrão de arquivo inválido: {0}")]
    BadPattern(String),
    #[error("JSON inválido em {what}: {source}")]
    Json {
        what: String,
        #[source]
        source: serde_json::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn io(path: &std::path::Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.display().to_string(),
        source,
    }
}

pub(crate) fn json(what: impl Into<String>) -> impl FnOnce(serde_json::Error) -> Error {
    let what = what.into();
    move |source| Error::Json { what, source }
}
