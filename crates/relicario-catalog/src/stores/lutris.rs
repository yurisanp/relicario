//! Lutris: configurações em `games/<slug>-<id>.yml`, com executável e prefixo Wine.

use super::Install;
use std::path::{Path, PathBuf};

pub fn config_dirs(home: &Path) -> Vec<PathBuf> {
    [
        home.join(".config/lutris/games"),
        home.join(".local/share/lutris/games"),
        home.join(".var/app/net.lutris.Lutris/config/lutris/games"),
        home.join(".var/app/net.lutris.Lutris/data/lutris/games"),
    ]
    .into_iter()
    .filter(|p| p.is_dir())
    .collect()
}

#[derive(Debug, Default, serde::Deserialize)]
struct LutrisFile {
    #[serde(default)]
    game: LutrisGame,
}

#[derive(Debug, Default, serde::Deserialize)]
struct LutrisGame {
    exe: Option<String>,
    prefix: Option<String>,
    working_dir: Option<String>,
}

/// `celeste-1700000000.yml` → `celeste`.
pub fn slug_of(file: &str) -> String {
    let stem = file.trim_end_matches(".yml");
    match stem.rsplit_once('-') {
        Some((s, ts)) if ts.chars().all(|c| c.is_ascii_digit()) => s.to_string(),
        _ => stem.to_string(),
    }
}

fn title_of(slug: &str) -> String {
    slug.split('-')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Jogos do Lutris e os prefixos Wine deles.
pub fn discover(home: &Path) -> (Vec<Install>, Vec<PathBuf>) {
    let mut out = Vec::new();
    let mut prefixes = Vec::new();
    for dir in config_dirs(home) {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        files.sort();
        for f in files {
            let Some(name) = f.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
                continue;
            };
            if !name.ends_with(".yml") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&f) else {
                continue;
            };
            let parsed: LutrisFile = serde_yaml_ng::from_str(&text).unwrap_or_default();
            let slug = slug_of(&name);
            let prefix = parsed
                .game
                .prefix
                .map(|p| relicario_platform::expand_home(&p))
                .filter(|p| p.is_dir());
            let base = parsed.game.working_dir.map(PathBuf::from).or_else(|| {
                parsed
                    .game
                    .exe
                    .as_ref()
                    .and_then(|e| Path::new(e).parent().map(Path::to_path_buf))
            });
            if let Some(p) = &prefix {
                prefixes.push(p.clone());
            }
            out.push(Install {
                launcher: "lutris",
                store: None,
                id: Some(slug.clone()),
                title: title_of(&slug),
                root: base
                    .as_ref()
                    .and_then(|b| b.parent().map(Path::to_path_buf)),
                dir_name: base
                    .as_ref()
                    .and_then(|b| b.file_name())
                    .map(|n| n.to_string_lossy().into_owned()),
                base,
                prefix,
                store_user_ids: vec![],
            });
        }
    }
    (out, prefixes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_yml() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join(".config/lutris/games");
        let pfx = t.path().join("Games/celeste/pfx");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::create_dir_all(&pfx).unwrap();
        std::fs::write(
            d.join("celeste-1700000000.yml"),
            format!("game:\n  exe: /games/celeste/Celeste.exe\n  prefix: {}\nwine:\n  version: lutris\n", pfx.display()),
        )
        .unwrap();
        let (v, p) = discover(t.path());
        assert_eq!(v[0].id.as_deref(), Some("celeste"));
        assert_eq!(v[0].title, "Celeste");
        assert_eq!(v[0].base.as_deref(), Some(Path::new("/games/celeste")));
        assert_eq!(p, vec![pfx]);
        assert_eq!(slug_of("a-b-c.yml"), "a-b-c");
    }
}
