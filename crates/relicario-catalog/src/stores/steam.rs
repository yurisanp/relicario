//! Steam: bibliotecas (`libraryfolders.vdf`), jogos (`appmanifest_*.acf`), prefixos Proton
//! (`compatdata`) e usuários (`userdata`).

use super::{Install, canon};
use crate::vdf;
use std::path::{Path, PathBuf};

/// Resultado do detector.
#[derive(Debug, Clone, Default)]
pub struct Steam {
    pub roots: Vec<PathBuf>,
    pub installs: Vec<Install>,
}

/// Pastas onde a Steam costuma ficar.
pub fn candidate_roots(home: &Path) -> Vec<PathBuf> {
    #[allow(unused_mut)]
    let mut v = vec![
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".steam/root"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        home.join("snap/steam/common/.local/share/Steam"),
        home.join("Library/Application Support/Steam"),
    ];
    #[cfg(windows)]
    {
        if let Some(p) = windows_steam_path() {
            v.insert(0, p);
        }
        v.push(PathBuf::from("C:/Program Files (x86)/Steam"));
        v.push(PathBuf::from("C:/Program Files/Steam"));
    }
    v
}

#[cfg(windows)]
fn windows_steam_path() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let k = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Valve\\Steam")
        .ok()?;
    let p: String = k.get_value("SteamPath").ok()?;
    Some(PathBuf::from(p))
}

fn is_steam_root(p: &Path) -> bool {
    p.join("steamapps").is_dir() || p.join("config/libraryfolders.vdf").is_file()
}

/// Bibliotecas de uma instalação da Steam (inclui a própria pasta).
pub fn libraries(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    for f in [
        root.join("steamapps/libraryfolders.vdf"),
        root.join("config/libraryfolders.vdf"),
    ] {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let Ok(v) = vdf::parse(&text) else { continue };
        let Some(lf) = v.get("libraryfolders") else {
            continue;
        };
        for (_, entry) in lf.entries() {
            // Formato novo: bloco com "path"; antigo: "1" "D:\\Steam".
            let p = entry.get_str("path").or_else(|| entry.str());
            if let Some(p) = p
                && !p.is_empty()
                && (p.contains('/') || p.contains('\\'))
            {
                out.push(PathBuf::from(p.replace("\\\\", "\\")));
            }
        }
    }
    let mut seen = Vec::new();
    out.retain(|p| {
        let c = canon(p);
        if seen.contains(&c) || !p.join("steamapps").is_dir() {
            false
        } else {
            seen.push(c);
            true
        }
    });
    out
}

/// Ids de usuário em `userdata/`.
pub fn user_ids(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root.join("userdata"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.chars().all(|c| c.is_ascii_digit()) && n != "0")
        .collect();
    v.sort();
    v
}

/// Lê um `appmanifest_<id>.acf`.
pub fn parse_app_manifest(text: &str) -> Option<(String, String, String)> {
    let v = vdf::parse(text).ok()?;
    let s = v.get("AppState")?;
    Some((
        s.get_str("appid")?.to_string(),
        s.get_str("name")?.to_string(),
        s.get_str("installdir")?.to_string(),
    ))
}

/// Encontra Steam, bibliotecas e jogos instalados.
pub fn discover(home: &Path) -> Steam {
    discover_from(candidate_roots(home))
}

/// Como [`discover`], com a lista de pastas candidatas dada.
pub fn discover_from(candidates: Vec<PathBuf>) -> Steam {
    let mut roots: Vec<PathBuf> = Vec::new();
    for c in candidates {
        if is_steam_root(&c) && !roots.iter().any(|r| canon(r) == canon(&c)) {
            roots.push(c);
        }
    }
    let mut installs = Vec::new();
    let mut seen_apps = std::collections::HashSet::new();
    for root in &roots {
        let users = user_ids(root);
        let libs = libraries(root);
        for lib in &libs {
            let apps = lib.join("steamapps");
            let Ok(rd) = std::fs::read_dir(&apps) else {
                continue;
            };
            let mut files: Vec<PathBuf> = rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("appmanifest_") && n.ends_with(".acf"))
                })
                .collect();
            files.sort();
            for f in files {
                let Ok(text) = std::fs::read_to_string(&f) else {
                    continue;
                };
                let Some((appid, name, dir)) = parse_app_manifest(&text) else {
                    continue;
                };
                if !seen_apps.insert(appid.clone()) || is_tool(&name) {
                    continue;
                }
                let base = apps.join("common").join(&dir);
                let prefix = libs
                    .iter()
                    .map(|l| l.join("steamapps/compatdata").join(&appid).join("pfx"))
                    .find(|p| p.is_dir());
                installs.push(Install {
                    launcher: "steam",
                    store: Some("steam"),
                    id: Some(appid),
                    title: name,
                    root: Some(root.clone()),
                    dir_name: Some(dir),
                    base: Some(base),
                    prefix,
                    store_user_ids: users.clone(),
                });
            }
        }
    }
    Steam { roots, installs }
}

/// Ferramentas que a Steam instala como "jogos" e que não têm save.
fn is_tool(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("proton")
        || n.starts_with("steam linux runtime")
        || n.starts_with("steamworks common")
        || n == "steamvr"
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;

    /// Monta uma Steam falsa com um jogo nativo e um no Proton.
    pub fn fake_steam(home: &Path) -> PathBuf {
        let root = home.join(".local/share/Steam");
        let lib2 = home.join("Games/SteamLibrary");
        fs::create_dir_all(root.join("steamapps/common/Hookshot")).unwrap();
        fs::create_dir_all(lib2.join("steamapps/common/Baldurs Gate 3")).unwrap();
        fs::create_dir_all(root.join("userdata/12345/1086940/remote")).unwrap();
        fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n \"0\" {{ \"path\" \"{}\" }}\n \"1\" {{ \"path\" \"{}\" }}\n}}\n",
                root.display(),
                lib2.display()
            ),
        )
        .unwrap();
        fs::write(
            root.join("steamapps/appmanifest_1031570.acf"),
            "\"AppState\" { \"appid\" \"1031570\" \"name\" \"Hookshot\" \"installdir\" \"Hookshot\" }",
        )
        .unwrap();
        fs::write(
            lib2.join("steamapps/appmanifest_1086940.acf"),
            "\"AppState\" { \"appid\" \"1086940\" \"name\" \"Baldur's Gate 3\" \"installdir\" \"Baldurs Gate 3\" }",
        )
        .unwrap();
        fs::write(
            root.join("steamapps/appmanifest_1493710.acf"),
            "\"AppState\" { \"appid\" \"1493710\" \"name\" \"Proton Experimental\" \"installdir\" \"Proton - Experimental\" }",
        )
        .unwrap();
        fs::create_dir_all(
            lib2.join("steamapps/compatdata/1086940/pfx/drive_c/users/steamuser/AppData/Local"),
        )
        .unwrap();
        root
    }

    #[test]
    fn encontra_bibliotecas_jogos_e_proton() {
        let t = tempfile::tempdir().unwrap();
        let root = fake_steam(t.path());
        let s = discover_from(vec![root.clone(), t.path().join("nao-existe")]);
        assert_eq!(s.roots.len(), 1);
        assert_eq!(libraries(&root).len(), 2);
        assert_eq!(s.installs.len(), 2, "Proton não conta como jogo");
        let bg3 = s
            .installs
            .iter()
            .find(|i| i.id.as_deref() == Some("1086940"))
            .unwrap();
        assert!(bg3.prefix.is_some());
        assert_eq!(bg3.store_user_ids, vec!["12345".to_string()]);
        let hs = s.installs.iter().find(|i| i.title == "Hookshot").unwrap();
        assert!(hs.prefix.is_none());
        assert!(
            hs.base
                .as_ref()
                .unwrap()
                .ends_with("steamapps/common/Hookshot")
        );
    }
}
