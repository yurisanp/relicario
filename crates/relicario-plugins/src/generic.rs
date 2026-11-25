//! Leitor genérico: miniatura em arquivo de imagem (ou embutida em `caption`, `thumb`...) e
//! campos conhecidos em JSON ou em texto `chave=valor`.

use crate::{Fetch, FileInfo, Reader, SaveMeta, field, file_name, find_image};
use serde_json::Value;

#[derive(Debug)]
pub struct Generic;

const TEXT_MAX: usize = 2 * 1024 * 1024;
const IMAGE_MAX: usize = 8 * 1024 * 1024;

/// Chave do save (em minúsculas, sem `_`, `-` e espaço) para o campo do Relicário.
fn key_for(k: &str) -> Option<&'static str> {
    let k: String = k
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect();
    Some(match k.as_str() {
        "level" | "lvl" | "playerlevel" | "characterlevel" | "nivel" => field::LEVEL,
        "location" | "area" | "scene" | "map" | "mapname" | "place" | "zone" | "currentlevel"
        | "region" | "local" | "room" | "stage" => field::PLACE,
        "playtime" | "timeplayed" | "totalplaytime" | "gametime" | "playedtime" | "tempo" => {
            field::PLAYTIME
        }
        "slot" | "saveslot" | "slotindex" => field::SLOT,
        "playername" | "charactername" | "character" | "hero" | "personagem" => field::CHARACTER,
        "difficulty" | "dificuldade" => field::DIFFICULTY,
        "savename" | "title" | "caption" | "titulo" => field::TITLE,
        _ => return None,
    })
}

/// Tempo de jogo: número em segundos (ou milissegundos, se for grande demais) ou texto.
fn playtime(v: &str) -> String {
    match v.parse::<f64>() {
        Ok(n) if n >= 0.0 => {
            let secs = if n > 10_000_000.0 { n / 1000.0 } else { n };
            crate::format_playtime(secs as u64)
        }
        _ => v.to_string(),
    }
}

fn put(meta: &mut SaveMeta, f: &'static str, v: String) {
    if meta.get(f).is_some() {
        return;
    }
    let v = if f == field::PLAYTIME {
        playtime(&v)
    } else {
        v
    };
    if v.chars().count() <= 120 {
        meta.set(f, v);
    }
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Procura até dois níveis abaixo da raiz.
pub(crate) fn from_json(v: &Value, meta: &mut SaveMeta, depth: u8) {
    let Value::Object(m) = v else { return };
    for (k, v) in m {
        if let (Some(f), Some(s)) = (key_for(k), scalar(v)) {
            put(meta, f, s);
        }
    }
    if depth < 2 {
        for v in m.values() {
            if v.is_object() {
                from_json(v, meta, depth + 1);
            }
        }
    }
}

pub(crate) fn from_text(t: &str, meta: &mut SaveMeta) {
    for line in t.lines().take(2000) {
        let Some((k, v)) = line.split_once('=').or_else(|| line.split_once(':')) else {
            continue;
        };
        if let Some(f) = key_for(k.trim()) {
            let v = v.trim().trim_matches('"').trim();
            if !v.is_empty() {
                put(meta, f, v.to_string());
            }
        }
    }
}

fn is_image_name(p: &str) -> bool {
    let n = file_name(p).to_lowercase();
    [".png", ".jpg", ".jpeg", ".webp"]
        .iter()
        .any(|e| n.ends_with(e))
}

fn is_thumb_hint(p: &str) -> bool {
    let n = file_name(p).to_lowercase();
    [
        "thumb",
        "screenshot",
        "caption",
        "preview",
        "screen",
        "capture",
    ]
    .iter()
    .any(|h| n.contains(h))
}

/// Do mais novo ao mais velho.
pub(crate) fn newest_first(files: &[FileInfo]) -> Vec<&FileInfo> {
    let mut v: Vec<&FileInfo> = files.iter().collect();
    v.sort_by(|a, b| b.mtime.cmp(&a.mtime).then(a.path.cmp(&b.path)));
    v
}

/// Primeira miniatura: arquivo de imagem, depois imagem embutida num arquivo com nome sugestivo.
pub(crate) fn thumbnail(
    files: &[FileInfo],
    fetch: &mut Fetch<'_>,
) -> Option<(String, crate::Thumbnail)> {
    let files = newest_first(files);
    let mut cands: Vec<&str> = files
        .iter()
        .map(|f| f.path.as_str())
        .filter(|p| is_image_name(p))
        .collect();
    cands.extend(
        files
            .iter()
            .map(|f| f.path.as_str())
            .filter(|p| !is_image_name(p) && is_thumb_hint(p)),
    );
    for p in cands.into_iter().take(8) {
        let Some(d) = fetch(p) else { continue };
        if d.len() > IMAGE_MAX {
            continue;
        }
        if let Some(t) = find_image(&d) {
            return Some((p.to_string(), t));
        }
    }
    None
}

impl Reader for Generic {
    fn name(&self) -> &'static str {
        "genérico"
    }

    fn matches(&self, _: &str, _: &str) -> bool {
        true
    }

    fn read(&self, files: &[FileInfo], fetch: &mut Fetch<'_>) -> Option<SaveMeta> {
        let mut meta = SaveMeta {
            reader: self.name().into(),
            ..Default::default()
        };
        let texty = |p: &str| {
            let n = file_name(p).to_lowercase();
            [".json", ".ini", ".cfg", ".txt", ".sav", ".save", ".dat"]
                .iter()
                .any(|e| n.ends_with(e))
        };
        for f in newest_first(files)
            .into_iter()
            .filter(|f| texty(&f.path) && f.size as usize <= TEXT_MAX)
            .take(16)
        {
            let p = &f.path;
            let Some(d) = fetch(p) else { continue };
            if d.len() > TEXT_MAX {
                continue;
            }
            let before = meta.fields.len();
            if let Ok(v) = serde_json::from_slice::<Value>(&d) {
                from_json(&v, &mut meta, 0);
            } else if let Ok(t) = std::str::from_utf8(&d) {
                from_text(t, &mut meta);
            }
            if meta.fields.len() > before && meta.source.is_empty() {
                meta.source = p.clone();
            }
        }
        if let Some((p, t)) = thumbnail(files, fetch) {
            if meta.source.is_empty() {
                meta.source = p;
            }
            meta.thumbnail = Some(t);
        }
        Some(meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn run(files: &[(&str, &[u8])]) -> SaveMeta {
        let m: BTreeMap<String, Vec<u8>> = files
            .iter()
            .map(|(p, d)| (p.to_string(), d.to_vec()))
            .collect();
        let infos: Vec<FileInfo> = m
            .iter()
            .map(|(p, d)| FileInfo::new(p.clone(), d.len() as u64, 0))
            .collect();
        Generic
            .read(&infos, &mut |p: &str| m.get(p).cloned())
            .unwrap()
    }

    #[test]
    fn json_e_texto() {
        let m = run(&[(
            "save/slot1.json",
            br#"{"player": {"Level": 12, "location": "Cavernas de Sal"}, "PlayTime": 3700}"#,
        )]);
        assert_eq!(m.get(field::LEVEL), Some("12"));
        assert_eq!(m.get(field::PLACE), Some("Cavernas de Sal"));
        assert_eq!(m.get(field::PLAYTIME), Some("1 h 01 min"));
        assert_eq!(m.source, "save/slot1.json");

        let m = run(&[("opts.ini", b"[jogo]\nslot = 2\nscene=\"Vila\"\n")]);
        assert_eq!(m.get(field::SLOT), Some("2"));
        assert_eq!(m.get(field::PLACE), Some("Vila"));
    }

    #[test]
    fn miniatura_por_nome() {
        let mut png = b"\x89PNG\r\n\x1a\nxx".to_vec();
        png.extend_from_slice(b"IEND\xaeB`\x82");
        let m = run(&[("a/thumb.bin", &png), ("a/x.dat", b"\x00\x01")]);
        assert_eq!(m.thumbnail.unwrap().mime, "image/png");
    }
}
