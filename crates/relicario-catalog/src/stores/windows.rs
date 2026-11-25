//! Lojas que só existem no Windows: GOG Galaxy (registro) e Epic Games Launcher (manifestos).

use super::Install;
use std::path::{Path, PathBuf};

pub fn discover() -> Vec<Install> {
    let mut v = gog_galaxy();
    v.extend(epic(Path::new(
        "C:/ProgramData/Epic/EpicGamesLauncher/Data/Manifests",
    )));
    v
}

fn gog_galaxy() -> Vec<Install> {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;
    let mut out = Vec::new();
    let Ok(games) =
        RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey("SOFTWARE\\WOW6432Node\\GOG.com\\Games")
    else {
        return out;
    };
    for id in games.enum_keys().filter_map(|k| k.ok()) {
        let Ok(k) = games.open_subkey(&id) else {
            continue;
        };
        let path: Option<String> = k.get_value("path").ok();
        let name: Option<String> = k.get_value("gameName").ok();
        let path = path.map(PathBuf::from);
        out.push(Install {
            launcher: "gog-galaxy",
            store: Some("gog"),
            id: Some(id.clone()),
            title: name.unwrap_or(id),
            root: path
                .as_ref()
                .and_then(|p| p.parent().map(Path::to_path_buf)),
            dir_name: path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned()),
            base: path,
            prefix: None,
            store_user_ids: vec![],
        });
    }
    out
}

pub fn epic(dir: &Path) -> Vec<Install> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("item") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&p) else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let loc = v
            .get("InstallLocation")
            .and_then(|x| x.as_str())
            .map(PathBuf::from);
        out.push(Install {
            launcher: "epic",
            store: Some("epic"),
            id: v
                .get("AppName")
                .and_then(|x| x.as_str())
                .map(str::to_string),
            title: v
                .get("DisplayName")
                .and_then(|x| x.as_str())
                .unwrap_or("?")
                .to_string(),
            root: loc.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)),
            dir_name: loc
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned()),
            base: loc,
            prefix: None,
            store_user_ids: vec![],
        });
    }
    out
}
