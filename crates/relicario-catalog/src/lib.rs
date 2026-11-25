//! Catálogo de jogos: onde ficam os saves de cada jogo nesta máquina.
//!
//! - [`manifest`]: o manifesto do Ludusavi (gerado do PCGamingWiki), com cache e atualização.
//! - [`resolve`]: placeholders do manifesto (`<base>`, `<winAppData>`…) para caminhos reais,
//!   inclusive dentro de prefixos Wine/Proton.
//! - [`stores`]: detectores de lojas e lançadores (Steam, Heroic, Lutris, Wine, GOG, Epic).
//! - [`emulators`]: emuladores com pastas de save conhecidas.
//! - [`mirror`]: caminhos espelhados (o mesmo save em dois lugares).
//! - [`rules`]: regras locais por jogo.
//! - [`scan`]: junta tudo e devolve os jogos encontrados.
//! - [`art`]: capa, banner, ícone e logo de cada jogo.

#![cfg_attr(not(feature = "net"), allow(dead_code, unused_imports))]

pub mod art;
pub mod emulators;
mod error;
pub mod launchers;
pub mod manifest;
pub mod mirror;
pub mod resolve;
pub mod rules;
pub mod scan;
pub mod slug;
pub mod stores;
pub mod vdf;

pub use error::{Error, Result};
pub use scan::{DetectedGame, DetectedRoot, ScanOptions, scan};
