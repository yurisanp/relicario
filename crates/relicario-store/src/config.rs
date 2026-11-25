use serde::{Deserialize, Serialize};

/// Versão do formato que esta build lê e escreve.
pub const FORMAT_VERSION: u32 = 1;

/// Arquivo `config` na raiz do repositório.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoConfig {
    pub format_version: u32,
    /// Identificador aleatório do repositório (hex de 16 bytes).
    pub id: String,
    pub created_at: i64,
    pub chunker: ChunkerParams,
    /// Nível do zstd.
    pub compression_level: i32,
    /// `none` na fase 1. A fase 5 adiciona `xchacha20poly1305`.
    pub encryption: String,
    /// Tamanho alvo de um pack, em bytes.
    pub pack_size: u64,
}

/// Parâmetros do FastCDC (v2020).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkerParams {
    pub min: u32,
    pub avg: u32,
    pub max: u32,
}

impl RepoConfig {
    pub fn new(id: String, created_at: i64) -> Self {
        RepoConfig {
            format_version: FORMAT_VERSION,
            id,
            created_at,
            chunker: ChunkerParams {
                min: 16 * 1024,
                avg: 64 * 1024,
                max: 256 * 1024,
            },
            compression_level: 3,
            encryption: "none".into(),
            pack_size: 8 * 1024 * 1024,
        }
    }
}
