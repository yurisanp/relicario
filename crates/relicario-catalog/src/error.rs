/// Erros do catálogo.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("erro de E/S em {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("manifesto inválido: {0}")]
    Manifest(String),
    #[error("arquivo VDF inválido: {0}")]
    Vdf(String),
    #[error("rede: {0}")]
    Net(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn io(path: &std::path::Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.display().to_string(),
        source,
    }
}
