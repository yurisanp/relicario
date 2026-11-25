//! Hookshot: cada slot é uma pasta `save/slotN/` com `caption.sav` (o "cartão" do slot: título,
//! local e miniatura) e `progress.sav` (o progresso).
//!
//! O formato exato ainda não está documentado (veja `docs/pendencias-do-yuri.md`). Este leitor
//! aceita o que for mais provável: JSON, texto `chave=valor` ou binário com textos e uma imagem
//! embutidos. Na dúvida, mostra só os slots e a miniatura.

use crate::generic::{from_json, from_text};
use crate::{Fetch, FileInfo, Reader, SaveMeta, field, file_name, find_image, parent};

#[derive(Debug)]
pub struct Hookshot;

/// Textos legíveis de um binário (UTF-8, pelo menos 4 caracteres, com alguma letra).
fn strings(d: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for run in d.split(|b| *b < 0x20 && *b != b'\t') {
        let Ok(s) = std::str::from_utf8(run) else {
            continue;
        };
        let s = s.trim();
        if s.chars().count() >= 4
            && s.chars().filter(|c| c.is_alphabetic()).count() * 2 >= s.chars().count()
            && !s.contains('\u{fffd}')
        {
            out.push(s.to_string());
        }
        if out.len() >= 16 {
            break;
        }
    }
    out
}

fn slot_number(dir: &str) -> Option<u32> {
    let n = file_name(dir).to_lowercase();
    n.strip_prefix("slot")?
        .trim_start_matches(['_', '-'])
        .parse()
        .ok()
}

impl Reader for Hookshot {
    fn name(&self) -> &'static str {
        "Hookshot"
    }

    fn matches(&self, game: &str, title: &str) -> bool {
        game.to_lowercase().contains("hookshot") || title.to_lowercase().contains("hookshot")
    }

    fn read(&self, files: &[FileInfo], fetch: &mut Fetch<'_>) -> Option<SaveMeta> {
        // (número, caminho do caption, modificação mais recente do slot)
        let mut slots: Vec<(u32, &str, i64)> = files
            .iter()
            .filter(|f| file_name(&f.path) == "caption.sav")
            .map(|f| {
                let dir = parent(&f.path);
                let newest = files
                    .iter()
                    .filter(|o| parent(&o.path) == dir)
                    .map(|o| o.mtime)
                    .max()
                    .unwrap_or(f.mtime);
                (slot_number(dir).unwrap_or(0), f.path.as_str(), newest)
            })
            .collect();
        if slots.is_empty() {
            return None;
        }
        slots.sort();
        let mut meta = SaveMeta {
            reader: self.name().into(),
            ..Default::default()
        };
        let list: Vec<String> = slots.iter().map(|(n, _, _)| n.to_string()).collect();
        meta.set("slots", list.join(", "));
        // O slot mostrado é o jogado por último (no empate, o de número mais alto).
        slots.sort_by(|a, b| b.2.cmp(&a.2).then(b.0.cmp(&a.0)));
        for (n, cap, _) in &slots {
            let Some(d) = fetch(cap) else { continue };
            let mut m = SaveMeta::default();
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&d) {
                from_json(&v, &mut m, 0);
            } else if let Ok(t) = std::str::from_utf8(&d) {
                from_text(t, &mut m);
            } else {
                let s = strings(&d);
                if let Some(t) = s.first() {
                    m.set(field::TITLE, t.clone());
                }
                if let Some(t) = s.get(1) {
                    m.set(field::PLACE, t.clone());
                }
            }
            let thumb = find_image(&d);
            if m.fields.is_empty() && thumb.is_none() {
                continue;
            }
            meta.set(field::SLOT, n.to_string());
            meta.fields.extend(m.fields);
            meta.thumbnail = thumb;
            meta.source = cap.to_string();
            break;
        }
        Some(meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn slot_jogado_por_ultimo() {
        let mut cap4 = vec![0u8, 0, 0, 4];
        cap4.extend_from_slice(b"Templo da Floresta\x00\x00\x07Cap\xc3\xadtulo dois\x00");
        cap4.extend_from_slice(b"\x89PNG\r\n\x1a\nimg");
        cap4.extend_from_slice(b"IEND\xaeB`\x82");
        let files: BTreeMap<String, Vec<u8>> = [
            (
                "save/slot1/caption.sav",
                b"{\"location\":\"Vila do Porto\"}".to_vec(),
            ),
            ("save/slot1/progress.sav", vec![1, 2, 3]),
            ("save/slot4/caption.sav", cap4),
            ("save/slot4/progress.sav", vec![9; 10]),
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
                    if p.contains("slot4") { 20 } else { 10 },
                )
            })
            .collect();
        let m = Hookshot
            .read(&infos, &mut |p: &str| files.get(p).cloned())
            .unwrap();
        assert_eq!(m.get("slots"), Some("1, 4"));
        assert_eq!(m.get(field::SLOT), Some("4"));
        assert_eq!(m.get(field::TITLE), Some("Templo da Floresta"));
        assert_eq!(m.get(field::PLACE), Some("Capítulo dois"));
        assert!(m.thumbnail.is_some());
        // Jogando o slot 1 depois, ele passa a ser o mostrado.
        let infos: Vec<FileInfo> = files
            .iter()
            .map(|(p, d)| {
                FileInfo::new(
                    p.clone(),
                    d.len() as u64,
                    if p.contains("slot1") { 30 } else { 10 },
                )
            })
            .collect();
        let m = Hookshot
            .read(&infos, &mut |p: &str| files.get(p).cloned())
            .unwrap();
        assert_eq!(m.get(field::SLOT), Some("1"));
        assert_eq!(m.get(field::PLACE), Some("Vila do Porto"));
        assert!(Hookshot.matches("hookshot", ""));
        assert!(!Hookshot.matches("steam-1086940", "Baldur's Gate 3"));
    }
}
