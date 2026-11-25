//! Instala o lançador `relicario run` nas lojas, para o Relicário sincronizar antes do jogo
//! abrir e depois que ele fechar.
//!
//! - Steam: opção de inicialização em `userdata/<id>/config/localconfig.vdf` (só com a Steam
//!   fechada, e sempre com backup).
//! - Heroic: `wrapperOptions` em `GamesConfig/<app>.json`.
//! - Lutris: `system.prefix_command` no `.yml` do jogo.

use crate::error::{Error, Result, io};
use crate::vdf::{self, Vdf};
use std::path::{Path, PathBuf};

/// Marca que identifica o que o Relicário escreveu.
pub const MARK: &str = "relicario";

/// Opção de inicialização da Steam para um executável.
pub fn steam_launch_option(exe: &Path) -> String {
    format!("\"{}\" run -- %command%", exe.display())
}

/// Junta a opção do Relicário com o que o usuário já tinha (`gamemoderun %command%` vira
/// `"relicario" run -- gamemoderun %command%`).
pub fn merge_launch_options(existing: &str, exe: &Path) -> String {
    let existing = existing.trim();
    if existing.contains(MARK) {
        return existing.to_string();
    }
    let ours = format!("\"{}\" run --", exe.display());
    if existing.is_empty() {
        format!("{ours} %command%")
    } else if existing.contains("%command%") {
        format!("{ours} {existing}")
    } else {
        // Só argumentos do jogo: eles vêm depois do %command%.
        format!("{ours} %command% {existing}")
    }
}

/// Tira a parte do Relicário.
pub fn strip_launch_options(existing: &str) -> String {
    let s = existing.trim();
    if !s.contains(MARK) {
        return s.to_string();
    }
    match s.find(" run -- ") {
        Some(i) => {
            let rest = s[i + " run -- ".len()..].trim();
            if rest == "%command%" {
                String::new()
            } else {
                rest.to_string()
            }
        }
        None => s.to_string(),
    }
}

/// Arquivos `localconfig.vdf` de cada usuário da Steam.
pub fn steam_localconfigs(steam_root: &Path) -> Vec<PathBuf> {
    crate::stores::steam::user_ids(steam_root)
        .into_iter()
        .map(|u| {
            steam_root
                .join("userdata")
                .join(u)
                .join("config/localconfig.vdf")
        })
        .filter(|p| p.is_file())
        .collect()
}

fn apps_mut(v: &mut Vdf) -> &mut Vdf {
    let root_key = v
        .entries()
        .next()
        .map(|(k, _)| k.clone())
        .unwrap_or_else(|| "UserLocalConfigStore".into());
    v.child_mut(&root_key)
        .child_mut("Software")
        .child_mut("Valve")
        .child_mut("Steam")
        .child_mut("apps")
}

/// Aplica (ou remove, com `install = false`) a opção nos appids dados. Grava um backup
/// `localconfig.vdf.relicario-<horário>` antes. Devolve quantos jogos mudaram.
pub fn steam_apply(
    localconfig: &Path,
    appids: &[String],
    exe: &Path,
    install: bool,
) -> Result<usize> {
    let text = std::fs::read_to_string(localconfig).map_err(io(localconfig))?;
    let mut v = vdf::parse(&text)?;
    let mut changed = 0;
    {
        let apps = apps_mut(&mut v);
        for id in appids {
            let app = apps.child_mut(id);
            let old = app.get_str("LaunchOptions").unwrap_or("").to_string();
            let new = if install {
                merge_launch_options(&old, exe)
            } else {
                strip_launch_options(&old)
            };
            if new != old {
                if new.is_empty() {
                    app.remove("LaunchOptions");
                } else {
                    app.set("LaunchOptions", Vdf::Str(new));
                }
                changed += 1;
            }
        }
    }
    if changed > 0 {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup = localconfig.with_file_name(format!("localconfig.vdf.relicario-{stamp}"));
        std::fs::copy(localconfig, &backup).map_err(io(&backup))?;
        let tmp = localconfig.with_extension("vdf.tmp");
        std::fs::write(&tmp, v.to_text()).map_err(io(&tmp))?;
        std::fs::rename(&tmp, localconfig).map_err(io(localconfig))?;
    }
    Ok(changed)
}

/// Heroic: liga ou desliga o wrapper do Relicário em `GamesConfig/<app>.json`.
pub fn heroic_apply(
    heroic_cfg: &Path,
    app: &str,
    game_id: &str,
    exe: &Path,
    install: bool,
) -> Result<bool> {
    let p = heroic_cfg.join("GamesConfig").join(format!("{app}.json"));
    let mut v: serde_json::Value = match std::fs::read(&p) {
        Ok(b) => {
            serde_json::from_slice(&b).map_err(|e| Error::Other(format!("{}: {e}", p.display())))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(io(&p)(e)),
    };
    let obj = v
        .as_object_mut()
        .ok_or_else(|| Error::Other(format!("{} não é um objeto", p.display())))?;
    let game = obj
        .entry(app.to_string())
        .or_insert_with(|| serde_json::json!({}));
    let g = game
        .as_object_mut()
        .ok_or_else(|| Error::Other("configuração do jogo inválida".into()))?;
    let wrappers = g
        .entry("wrapperOptions")
        .or_insert_with(|| serde_json::json!([]));
    let list = wrappers
        .as_array_mut()
        .ok_or_else(|| Error::Other("wrapperOptions inválido".into()))?;
    let before = list.len();
    list.retain(|w| {
        !w.get("exe")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains(MARK))
    });
    if install {
        list.insert(0, serde_json::json!({ "exe": exe.to_string_lossy(), "args": format!("run --game {game_id} --") }));
    }
    let changed = install || list.len() != before;
    if changed {
        std::fs::create_dir_all(p.parent().expect("pasta")).map_err(io(&p))?;
        std::fs::write(&p, serde_json::to_vec_pretty(&v).expect("json")).map_err(io(&p))?;
    }
    Ok(changed)
}

/// Lutris: define ou remove `system.prefix_command` no `.yml` do jogo.
pub fn lutris_apply(yml: &Path, game_id: &str, exe: &Path, install: bool) -> Result<bool> {
    let text = std::fs::read_to_string(yml).map_err(io(yml))?;
    let mut v: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&text).map_err(|e| Error::Other(e.to_string()))?;
    let map = v
        .as_mapping_mut()
        .ok_or_else(|| Error::Other("yml do Lutris inválido".into()))?;
    let system = map
        .entry(serde_yaml_ng::Value::from("system"))
        .or_insert_with(|| serde_yaml_ng::Value::Mapping(Default::default()));
    let sys = system
        .as_mapping_mut()
        .ok_or_else(|| Error::Other("system inválido".into()))?;
    let key = serde_yaml_ng::Value::from("prefix_command");
    let current = sys
        .get(&key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let changed = if install {
        if current.contains(MARK) {
            false
        } else {
            let ours = format!("\"{}\" run --game {game_id} --", exe.display());
            let value = if current.is_empty() {
                ours
            } else {
                format!("{ours} {current}")
            };
            sys.insert(key, serde_yaml_ng::Value::from(value));
            true
        }
    } else if current.contains(MARK) {
        let rest = current
            .split(" -- ")
            .nth(1)
            .unwrap_or("")
            .trim()
            .to_string();
        if rest.is_empty() {
            sys.remove(&key);
        } else {
            sys.insert(key, serde_yaml_ng::Value::from(rest));
        }
        true
    } else {
        false
    };
    if changed {
        let out = serde_yaml_ng::to_string(&v).map_err(|e| Error::Other(e.to_string()))?;
        std::fs::write(yml, out).map_err(io(yml))?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcoes_de_inicializacao() {
        let exe = Path::new("/opt/relicario");
        assert_eq!(
            merge_launch_options("", exe),
            "\"/opt/relicario\" run -- %command%"
        );
        assert_eq!(
            merge_launch_options("gamemoderun %command%", exe),
            "\"/opt/relicario\" run -- gamemoderun %command%"
        );
        assert_eq!(
            merge_launch_options("-novid", exe),
            "\"/opt/relicario\" run -- %command% -novid"
        );
        let m = merge_launch_options("gamemoderun %command%", exe);
        assert_eq!(merge_launch_options(&m, exe), m, "não duplica");
        assert_eq!(strip_launch_options(&m), "gamemoderun %command%");
        assert_eq!(strip_launch_options(&merge_launch_options("", exe)), "");
    }

    #[test]
    fn steam_com_backup() {
        let t = tempfile::tempdir().unwrap();
        let cfg = t.path().join("userdata/123/config");
        std::fs::create_dir_all(&cfg).unwrap();
        let lc = cfg.join("localconfig.vdf");
        std::fs::write(
            &lc,
            "\"UserLocalConfigStore\"\n{\n\t\"friends\"\n\t{\n\t\t\"x\"\t\t\"1\"\n\t}\n}\n",
        )
        .unwrap();
        let exe = Path::new("/opt/relicario");
        assert_eq!(steam_localconfigs(t.path()), vec![lc.clone()]);
        assert_eq!(steam_apply(&lc, &["1031570".into()], exe, true).unwrap(), 1);
        let v = vdf::parse(&std::fs::read_to_string(&lc).unwrap()).unwrap();
        let o = v
            .path(&[
                "UserLocalConfigStore",
                "Software",
                "Valve",
                "Steam",
                "apps",
                "1031570",
                "LaunchOptions",
            ])
            .unwrap();
        assert_eq!(o.str(), Some("\"/opt/relicario\" run -- %command%"));
        assert_eq!(
            v.path(&["UserLocalConfigStore", "friends", "x"])
                .unwrap()
                .str(),
            Some("1")
        );
        assert!(std::fs::read_dir(&cfg).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("localconfig.vdf.relicario-")
        }));
        assert_eq!(steam_apply(&lc, &["1031570".into()], exe, true).unwrap(), 0);
        assert_eq!(
            steam_apply(&lc, &["1031570".into()], exe, false).unwrap(),
            1
        );
        let v = vdf::parse(&std::fs::read_to_string(&lc).unwrap()).unwrap();
        assert!(
            v.path(&[
                "UserLocalConfigStore",
                "Software",
                "Valve",
                "Steam",
                "apps",
                "1031570",
                "LaunchOptions"
            ])
            .is_none()
        );
    }

    #[test]
    fn heroic_e_lutris() {
        let t = tempfile::tempdir().unwrap();
        let exe = Path::new("/opt/relicario");
        assert!(heroic_apply(t.path(), "123", "witcher-3", exe, true).unwrap());
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(t.path().join("GamesConfig/123.json")).unwrap())
                .unwrap();
        assert_eq!(
            v["123"]["wrapperOptions"][0]["args"],
            "run --game witcher-3 --"
        );
        assert!(heroic_apply(t.path(), "123", "witcher-3", exe, false).unwrap());
        let yml = t.path().join("celeste.yml");
        std::fs::write(
            &yml,
            "game:\n  exe: /g/Celeste.exe\nsystem:\n  prefix_command: gamemoderun\n",
        )
        .unwrap();
        assert!(lutris_apply(&yml, "celeste", exe, true).unwrap());
        let text = std::fs::read_to_string(&yml).unwrap();
        assert!(text.contains("run --game celeste -- gamemoderun"), "{text}");
        assert!(!lutris_apply(&yml, "celeste", exe, true).unwrap());
        assert!(lutris_apply(&yml, "celeste", exe, false).unwrap());
        assert!(
            std::fs::read_to_string(&yml)
                .unwrap()
                .contains("prefix_command: gamemoderun")
        );
    }
}
