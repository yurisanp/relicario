//! Baldur's Gate 3: cada save é uma pasta com `<nome>.lsv` (pacote LSPK com `SaveInfo.json`) e
//! `<nome>.WebP` (a miniatura). Mostra o save mais recente do snapshot.

use crate::generic::{from_json, newest_first};
use crate::lspk::Package;
use crate::{Fetch, FileInfo, Reader, SaveMeta, Thumbnail, field, file_name, parent};
use serde_json::Value;

#[derive(Debug)]
pub struct Bg3;

/// Steam 1086940, GOG 1456460669.
const IDS: &[&str] = &["1086940", "1456460669"];

/// Nome amigável das fases mais conhecidas.
fn level_name(code: &str) -> Option<&'static str> {
    let prefix = code.split('_').next().unwrap_or(code);
    Some(match prefix {
        "TUT" => "Prólogo (nautiloide)",
        "WLD" => "Ato 1",
        "CRE" => "Ato 1 (creche)",
        "SCL" => "Ato 2 (terras sombrias)",
        "BGO" => "Ato 3 (arredores de Baldur's Gate)",
        "CTY" => "Ato 3 (cidade baixa)",
        "IRN" => "Ato 3 (forja de ferro)",
        "END" => "Final",
        _ => return None,
    })
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(a) => {
            let parts: Vec<String> = a.iter().filter_map(text).collect();
            (!parts.is_empty()).then(|| parts.join(", "))
        }
        _ => None,
    }
}

fn get<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let o = v.as_object()?;
    keys.iter().find_map(|k| {
        o.iter()
            .find(|(ok, _)| ok.eq_ignore_ascii_case(k))
            .map(|(_, v)| v)
    })
}

/// Campos do `SaveInfo.json`.
fn from_save_info(v: &Value, meta: &mut SaveMeta) {
    if let Some(n) = get(v, &["Save Name", "SaveName"]).and_then(text) {
        meta.set(field::TITLE, n);
    }
    if let Some(code) = get(v, &["Current Level", "CurrentLevel"]).and_then(text) {
        let name = level_name(&code)
            .map(|n| format!("{n} · {code}"))
            .unwrap_or(code);
        meta.set(field::PLACE, name);
    }
    let chars = get(v, &["Active Party", "ActiveParty"])
        .and_then(|p| get(p, &["Characters"]))
        .and_then(Value::as_array);
    if let Some(chars) = chars {
        if let Some(c) = chars.first() {
            let origin = get(c, &["Origin", "Name"])
                .and_then(text)
                .unwrap_or_default();
            let race = get(c, &["Race"]).and_then(text).unwrap_or_default();
            let class = get(c, &["Classes"])
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|c| get(c, &["Main"]))
                .and_then(text)
                .unwrap_or_default();
            let parts: Vec<String> = [origin, race, class]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect();
            meta.set(field::CHARACTER, parts.join(" · "));
        }
        let max = chars
            .iter()
            .filter_map(|c| get(c, &["Level"]).and_then(Value::as_u64))
            .max();
        if let Some(l) = max {
            meta.set(field::LEVEL, l.to_string());
        }
        meta.set("grupo", chars.len().to_string());
    }
    if let Some(d) = get(v, &["Difficulty"]).and_then(text) {
        meta.set(field::DIFFICULTY, d);
    }
    if let Some(g) = get(v, &["Game Version", "GameVersion"]).and_then(text) {
        meta.set(field::VERSION, g);
    }
    // O que mais tiver nome conhecido.
    from_json(v, meta, 1);
}

impl Reader for Bg3 {
    fn name(&self) -> &'static str {
        "BG3"
    }

    fn matches(&self, game: &str, title: &str) -> bool {
        let t = title.to_lowercase();
        IDS.iter().any(|id| game.ends_with(id))
            || t.contains("baldur's gate 3")
            || t.contains("baldurs gate 3")
            || game.contains("baldurs-gate-3")
    }

    fn read(&self, files: &[FileInfo], fetch: &mut Fetch<'_>) -> Option<SaveMeta> {
        let lsv = newest_first(files)
            .into_iter()
            .find(|f| f.path.to_lowercase().ends_with(".lsv") && f.size <= crate::MAX_READ)?;
        let mut meta = SaveMeta {
            reader: self.name().into(),
            source: lsv.path.clone(),
            ..Default::default()
        };
        let stem = file_name(&lsv.path)
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or_default()
            .to_string();
        meta.set("save", stem.clone());
        if let Some(data) = fetch(&lsv.path)
            && let Some(pkg) = Package::open(&data)
            && let Some(e) = pkg.find("SaveInfo.json")
            && let Some(json) = pkg.read(e)
            && let Ok(v) = serde_json::from_slice::<Value>(&json)
        {
            from_save_info(&v, &mut meta);
        }
        // Miniatura: a imagem com o mesmo nome, na mesma pasta.
        let dir = parent(&lsv.path);
        let thumb = files.iter().find(|f| {
            parent(&f.path) == dir && {
                let n = file_name(&f.path).to_lowercase();
                n.starts_with(&stem.to_lowercase())
                    && (n.ends_with(".webp") || n.ends_with(".png") || n.ends_with(".jpg"))
            }
        });
        if let Some(t) = thumb
            && t.size <= 8 * 1024 * 1024
            && let Some(d) = fetch(&t.path)
        {
            let mime = match file_name(&t.path).to_lowercase().rsplit('.').next() {
                Some("png") => "image/png",
                Some("jpg") => "image/jpeg",
                _ => "image/webp",
            };
            meta.thumbnail = Some(Thumbnail {
                mime: mime.into(),
                data: d,
            });
        }
        Some(meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lspk::build;
    use std::collections::BTreeMap;

    #[test]
    fn le_save_info_do_mais_recente() {
        let info = br#"{
            "Active Party": {"Characters": [
                {"Origin": "Tav", "Race": "Elf", "Level": 5, "Classes": [{"Main": "Wizard"}]},
                {"Origin": "Astarion", "Race": "Elf", "Level": 6}
            ]},
            "Current Level": "SCL_Main_A",
            "Difficulty": ["Tactician"],
            "Game Version": "4.1.1.6"
        }"#;
        let old = build(&[("SaveInfo.json", br#"{"Current Level":"WLD_Main_A"}"#, 2)]);
        let new = build(&[("meta.lsf", b"x", 0), ("SaveInfo.json", info, 2)]);
        let files: BTreeMap<String, Vec<u8>> = [
            ("Story/A__HonourMode/A.lsv", old),
            ("Story/B__Quick/B.lsv", new),
            (
                "Story/B__Quick/B.WebP",
                b"RIFF\x04\x00\x00\x00WEBP".to_vec(),
            ),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b))
        .collect();
        let infos: Vec<FileInfo> = files
            .iter()
            .map(|(p, d)| {
                FileInfo::new(
                    p.clone(),
                    d.len() as u64,
                    if p.contains("B") { 2 } else { 1 },
                )
            })
            .collect();
        let m = Bg3
            .read(&infos, &mut |p: &str| files.get(p).cloned())
            .unwrap();
        assert_eq!(m.source, "Story/B__Quick/B.lsv");
        assert_eq!(
            m.get(field::PLACE),
            Some("Ato 2 (terras sombrias) · SCL_Main_A")
        );
        assert_eq!(m.get(field::CHARACTER), Some("Tav · Elf · Wizard"));
        assert_eq!(m.get(field::LEVEL), Some("6"));
        assert_eq!(m.get(field::DIFFICULTY), Some("Tactician"));
        assert_eq!(m.get("grupo"), Some("2"));
        assert_eq!(m.thumbnail.unwrap().mime, "image/webp");
        assert!(Bg3.matches("steam-1086940", ""));
        assert!(!Bg3.matches("hookshot", "Hookshot"));
    }
}
