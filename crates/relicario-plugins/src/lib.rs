//! Leitores de save: tiram de um snapshot o que ajuda a reconhecer o ponto do jogo (local,
//! personagem, nível, tempo de jogo, miniatura). Só leem: nunca mudam o save.
//!
//! A interface é estável e pequena para virar WASM de terceiros na 1.x: o leitor recebe a lista
//! de caminhos do snapshot e uma função para ler o conteúdo de um caminho.

mod bg3;
mod generic;
mod hookshot;
pub mod lspk;

use serde::{Deserialize, Serialize};

/// Campos com nome fixo, para a interface mostrar igual em todos os jogos.
pub mod field {
    pub const TITLE: &str = "titulo";
    pub const PLACE: &str = "local";
    pub const CHARACTER: &str = "personagem";
    pub const LEVEL: &str = "nivel";
    pub const PLAYTIME: &str = "tempo_de_jogo";
    pub const SLOT: &str = "slot";
    pub const DIFFICULTY: &str = "dificuldade";
    pub const VERSION: &str = "versao_do_jogo";
}

/// Miniatura achada no save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thumbnail {
    /// `image/png`, `image/jpeg` ou `image/webp`.
    pub mime: String,
    pub data: Vec<u8>,
}

/// O que um leitor achou.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveMeta {
    /// Nome do leitor que achou ("BG3", "Hookshot", "genérico").
    pub reader: String,
    /// Arquivo de onde veio a informação principal.
    pub source: String,
    /// Pares (campo, valor), na ordem em que devem aparecer.
    pub fields: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<Thumbnail>,
}

impl SaveMeta {
    pub fn get(&self, k: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(f, _)| f == k)
            .map(|(_, v)| v.as_str())
    }

    fn set(&mut self, k: &str, v: impl Into<String>) {
        let v = v.into();
        if v.trim().is_empty() {
            return;
        }
        match self.fields.iter_mut().find(|(f, _)| f == k) {
            Some(e) => e.1 = v,
            None => self.fields.push((k.to_string(), v)),
        }
    }

    fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.thumbnail.is_none()
    }
}

/// Um arquivo do snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInfo {
    /// Caminho relativo, com `/`.
    pub path: String,
    pub size: u64,
    /// Última modificação (ms desde 1970).
    pub mtime: i64,
}

impl FileInfo {
    pub fn new(path: impl Into<String>, size: u64, mtime: i64) -> Self {
        FileInfo {
            path: path.into(),
            size,
            mtime,
        }
    }
}

/// Lê um arquivo do snapshot pelo caminho relativo (com `/`). `None` se não existir.
pub type Fetch<'a> = dyn FnMut(&str) -> Option<Vec<u8>> + 'a;

/// Um leitor de save.
pub trait Reader: Send + Sync {
    fn name(&self) -> &'static str;
    /// Serve para este jogo? `game` é o id do Relicário (`steam-1086940`, `hookshot`...).
    fn matches(&self, game: &str, title: &str) -> bool;
    /// Lê o que der. `files` são os arquivos do snapshot.
    fn read(&self, files: &[FileInfo], fetch: &mut Fetch<'_>) -> Option<SaveMeta>;
}

/// Leitores embutidos, do mais específico ao genérico.
pub fn readers() -> Vec<Box<dyn Reader>> {
    vec![
        Box::new(bg3::Bg3),
        Box::new(hookshot::Hookshot),
        Box::new(generic::Generic),
    ]
}

/// Maior arquivo que um leitor vai pedir (saves de BG3 passam de 20 MB).
pub const MAX_READ: u64 = 96 * 1024 * 1024;

/// Usa o primeiro leitor que serve e acha algo.
pub fn read(
    game: &str,
    title: &str,
    files: &[FileInfo],
    fetch: &mut Fetch<'_>,
) -> Option<SaveMeta> {
    for r in readers() {
        if !r.matches(game, title) {
            continue;
        }
        if let Some(m) = r.read(files, fetch)
            && !m.is_empty()
        {
            return Some(m);
        }
    }
    None
}

/// Tempo de jogo em segundos para "12 h 05 min".
pub fn format_playtime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("{h} h {m:02} min")
    } else {
        format!("{m} min")
    }
}

/// Acha uma imagem embutida (PNG, JPEG ou WebP) em qualquer lugar do arquivo.
pub fn find_image(data: &[u8]) -> Option<Thumbnail> {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    const PNG_END: &[u8] = b"IEND\xaeB`\x82";
    if let Some(start) = find(data, PNG) {
        let end = find(&data[start..], PNG_END)
            .map(|e| start + e + PNG_END.len())
            .unwrap_or(data.len());
        return Some(Thumbnail {
            mime: "image/png".into(),
            data: data[start..end].to_vec(),
        });
    }
    if let Some(start) = find(data, b"RIFF")
        && data.len() >= start + 12
        && &data[start + 8..start + 12] == b"WEBP"
    {
        let len = u32::from_le_bytes(data[start + 4..start + 8].try_into().ok()?) as usize + 8;
        let end = (start + len).min(data.len());
        return Some(Thumbnail {
            mime: "image/webp".into(),
            data: data[start..end].to_vec(),
        });
    }
    if let Some(start) = find(data, b"\xff\xd8\xff") {
        let end = find(&data[start..], b"\xff\xd9")
            .map(|e| start + e + 2)
            .unwrap_or(data.len());
        if end - start > 128 {
            return Some(Thumbnail {
                mime: "image/jpeg".into(),
                data: data[start..end].to_vec(),
            });
        }
    }
    None
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Nome do arquivo sem pastas.
fn file_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// Pasta do arquivo (sem a barra final).
fn parent(p: &str) -> &str {
    p.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imagem_embutida() {
        let mut d = b"lixo antes".to_vec();
        d.extend_from_slice(b"\x89PNG\r\n\x1a\nCORPO");
        d.extend_from_slice(b"IEND\xaeB`\x82");
        d.extend_from_slice(b"depois");
        let t = find_image(&d).unwrap();
        assert_eq!(t.mime, "image/png");
        assert!(t.data.starts_with(b"\x89PNG"));
        assert!(t.data.ends_with(b"IEND\xaeB`\x82"));
    }

    #[test]
    fn tempo() {
        assert_eq!(format_playtime(43_500), "12 h 05 min");
        assert_eq!(format_playtime(59 * 60), "59 min");
    }
}
