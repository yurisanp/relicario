//! Heroic Games Launcher: GOG (`gog_store/installed.json`) e Epic via Legendary
//! (`legendaryConfig/legendary/installed.json`), com o prefixo Wine de cada jogo.

use super::Install;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn config_dirs(home: &Path) -> Vec<PathBuf> {
    let mut v = vec![
        home.join(".config/heroic"),
        home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"),
        home.join("Library/Application Support/heroic"),
    ];
    if let Some(a) = std::env::var_os("APPDATA") {
        v.push(PathBuf::from(a).join("heroic"));
    }
    v.into_iter().filter(|p| p.is_dir()).collect()
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

/// Prefixo Wine configurado para um jogo.
fn wine_prefix(cfg: &Path, app: &str) -> Option<PathBuf> {
    let v = read_json(&cfg.join("GamesConfig").join(format!("{app}.json")))?;
    let p = v.get(app)?.get("winePrefix")?.as_str()?;
    let p = PathBuf::from(p);
    // Prefixos do Proton têm a pasta pfx dentro.
    if p.join("pfx").is_dir() {
        Some(p.join("pfx"))
    } else {
        p.is_dir().then_some(p)
    }
}

fn gog_titles(cfg: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for f in ["store_cache/gog_library.json", "gog_store/library.json"] {
        let Some(v) = read_json(&cfg.join(f)) else {
            continue;
        };
        let games = v
            .get("games")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for g in games {
            if let (Some(a), Some(t)) = (
                g.get("app_name").and_then(Value::as_str),
                g.get("title").and_then(Value::as_str),
            ) {
                out.entry(a.to_string()).or_insert_with(|| t.to_string());
            }
        }
    }
    out
}

pub fn discover(home: &Path) -> Vec<Install> {
    let mut out = Vec::new();
    for cfg in config_dirs(home) {
        let titles = gog_titles(&cfg);
        if let Some(v) = read_json(&cfg.join("gog_store/installed.json")) {
            for g in v
                .get("installed")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(app) = g.get("appName").and_then(Value::as_str) else {
                    continue;
                };
                let path = g
                    .get("install_path")
                    .and_then(Value::as_str)
                    .map(PathBuf::from);
                let windows = g.get("platform").and_then(Value::as_str) == Some("windows");
                let title = titles.get(app).cloned().or_else(|| {
                    path.as_ref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                });
                out.push(Install {
                    launcher: "heroic",
                    store: Some("gog"),
                    id: Some(app.to_string()),
                    title: title.unwrap_or_else(|| app.to_string()),
                    root: path
                        .as_ref()
                        .and_then(|p| p.parent().map(Path::to_path_buf)),
                    dir_name: path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned()),
                    prefix: if windows && !cfg!(windows) {
                        wine_prefix(&cfg, app)
                    } else {
                        None
                    },
                    base: path,
                    store_user_ids: vec![],
                });
            }
        }
        if let Some(v) = read_json(&cfg.join("legendaryConfig/legendary/installed.json"))
            && let Some(map) = v.as_object()
        {
            for (app, g) in map {
                let path = g
                    .get("install_path")
                    .and_then(Value::as_str)
                    .map(PathBuf::from);
                let windows = g
                    .get("platform")
                    .and_then(Value::as_str)
                    .is_none_or(|p| p.eq_ignore_ascii_case("windows"));
                out.push(Install {
                    launcher: "heroic",
                    store: Some("epic"),
                    id: Some(app.clone()),
                    title: g
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or(app)
                        .to_string(),
                    root: path
                        .as_ref()
                        .and_then(|p| p.parent().map(Path::to_path_buf)),
                    dir_name: path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned()),
                    prefix: if windows && !cfg!(windows) {
                        wine_prefix(&cfg, app)
                    } else {
                        None
                    },
                    base: path,
                    store_user_ids: vec![],
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn gog_e_epic() {
        let t = tempfile::tempdir().unwrap();
        let cfg = t.path().join(".config/heroic");
        let pfx = t.path().join("Games/Heroic/Prefixes/Witcher");
        fs::create_dir_all(pfx.join("drive_c/users/yuri")).unwrap();
        fs::create_dir_all(cfg.join("gog_store")).unwrap();
        fs::create_dir_all(cfg.join("store_cache")).unwrap();
        fs::create_dir_all(cfg.join("GamesConfig")).unwrap();
        fs::create_dir_all(cfg.join("legendaryConfig/legendary")).unwrap();
        fs::write(
            cfg.join("gog_store/installed.json"),
            r#"{"installed":[{"appName":"1207664663","install_path":"/games/The Witcher 3","platform":"windows"}]}"#,
        )
        .unwrap();
        fs::write(
            cfg.join("store_cache/gog_library.json"),
            r#"{"games":[{"app_name":"1207664663","title":"The Witcher 3: Wild Hunt"}]}"#,
        )
        .unwrap();
        fs::write(
            cfg.join("GamesConfig/1207664663.json"),
            format!(r#"{{"1207664663":{{"winePrefix":"{}"}}}}"#, pfx.display()),
        )
        .unwrap();
        fs::write(
            cfg.join("legendaryConfig/legendary/installed.json"),
            r#"{"Fortnite":{"title":"Jogo Epic","install_path":"/games/Epic","platform":"Windows"}}"#,
        )
        .unwrap();
        let v = discover(t.path());
        assert_eq!(v.len(), 2);
        let w = v.iter().find(|i| i.store == Some("gog")).unwrap();
        assert_eq!(w.title, "The Witcher 3: Wild Hunt");
        if !cfg!(windows) {
            assert_eq!(w.prefix.as_deref(), Some(pfx.as_path()));
        }
        assert_eq!(
            v.iter().find(|i| i.store == Some("epic")).unwrap().title,
            "Jogo Epic"
        );
    }
}
