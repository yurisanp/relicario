//! Detectores de lojas e lançadores.

pub mod heroic;
pub mod lutris;
pub mod steam;
#[cfg(windows)]
pub mod windows;
pub mod wine;

use std::path::PathBuf;

/// Um jogo instalado, encontrado por algum detector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    /// Lançador que encontrou o jogo (`steam`, `heroic`, `lutris`, `gog-galaxy`, `epic`).
    pub launcher: &'static str,
    /// Loja no vocabulário do manifesto (`steam`, `gog`, `epic`), quando se aplica.
    pub store: Option<&'static str>,
    /// Id na loja: appid da Steam, id do GOG, appName da Epic, slug do Lutris.
    pub id: Option<String>,
    pub title: String,
    /// `<root>`: pasta da biblioteca.
    pub root: Option<PathBuf>,
    /// `<game>`: nome da pasta de instalação.
    pub dir_name: Option<String>,
    /// `<base>`: pasta de instalação.
    pub base: Option<PathBuf>,
    /// Prefixo Wine/Proton, quando o jogo de Windows roda no Linux.
    pub prefix: Option<PathBuf>,
    /// Ids de usuário da loja (`userdata/<id>` da Steam).
    pub store_user_ids: Vec<String>,
}

/// Tudo que os detectores encontraram.
#[derive(Debug, Clone, Default)]
pub struct Discovery {
    pub installs: Vec<Install>,
    /// Prefixos Wine que não pertencem a um jogo só (por exemplo `~/.wine`), onde vale a pena
    /// procurar qualquer jogo do manifesto.
    pub shared_prefixes: Vec<PathBuf>,
    /// Pastas raiz da Steam (para arte em cache e `localconfig.vdf`).
    pub steam_roots: Vec<PathBuf>,
}

/// Roda todos os detectores disponíveis neste sistema.
pub fn discover(home: &std::path::Path) -> Discovery {
    let mut d = Discovery::default();
    let steam = steam::discover(home);
    d.steam_roots = steam.roots;
    d.installs.extend(steam.installs);
    d.installs.extend(heroic::discover(home));
    let (lutris, prefixes) = lutris::discover(home);
    d.installs.extend(lutris);
    d.shared_prefixes.extend(prefixes);
    d.shared_prefixes.extend(wine::discover(home));
    #[cfg(windows)]
    d.installs.extend(windows::discover());
    d.shared_prefixes.sort();
    d.shared_prefixes.dedup();
    d
}

/// Canoniza sem falhar (para remover duplicatas de links simbólicos).
pub(crate) fn canon(p: &std::path::Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}
