//! Repositório de blocos endereçado por conteúdo.
//!
//! Formato descrito em `docs/format.md`. Resumo:
//!
//! - Arquivos são cortados em blocos de tamanho variável (FastCDC), cada bloco é identificado
//!   pelo BLAKE3 do conteúdo em claro, comprimido com zstd e guardado em *packs* imutáveis.
//! - Arquivos de *index* dizem em que pack e posição cada bloco está.
//! - A lista de arquivos de um snapshot (a [`Tree`]) também é um blob.
//! - Metadados de snapshot ficam em `snapshots/<id>`; o estado de cada máquina em
//!   `heads/<máquina>`, o único arquivo mutável, escrito só pela própria máquina.
//!
//! [`Tree`]: relicario_model::Tree

mod config;
pub mod crypto;
mod error;
pub mod files;
mod lock;
pub mod ops;
mod pack;
pub mod prune;
pub mod replicate;
mod repo;
pub mod time;
pub mod verify;

pub use config::RepoConfig;
pub use error::{Error, Result};
pub use files::{Filter, RestoreReport, Source};
pub use lock::{LockGuard, LockMode};
pub use repo::Repository;
