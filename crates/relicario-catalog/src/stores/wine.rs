//! Prefixos Wine soltos (`~/.wine` e `WINEPREFIX`).

use std::path::{Path, PathBuf};

pub fn discover(home: &Path) -> Vec<PathBuf> {
    if cfg!(windows) {
        return vec![];
    }
    let mut v = vec![home.join(".wine")];
    if let Some(p) = std::env::var_os("WINEPREFIX") {
        v.push(PathBuf::from(p));
    }
    v.into_iter()
        .filter(|p| p.join("drive_c").is_dir())
        .collect()
}
