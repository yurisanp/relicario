//! Leitura de pacotes LSPK da Larian (versão 18, a do Baldur's Gate 3). Saves `.lsv` são
//! pacotes LSPK com `SaveInfo.json`, `meta.lsf` e o mundo.
//!
//! Layout (little endian):
//!
//! ```text
//! "LSPK" | versão u32 | posição da lista u64 | tamanho da lista u32 | flags u8 | prioridade u8
//!        | md5 [16] | partes u16
//! lista: quantidade u32 | tamanho comprimido u32 | LZ4 (bloco) com quantidade × 272 bytes:
//!        nome [256] | posição baixa u32 | posição alta u16 | parte u8 | flags u8
//!        | tamanho no disco u32 | tamanho descomprimido u32
//! ```
//!
//! Compressão de cada arquivo em `flags & 0x0f`: 0 nenhuma, 1 zlib, 2 LZ4, 3 zstd.

use std::io::Read;

const MAGIC: &[u8; 4] = b"LSPK";
const ENTRY: usize = 272;
/// Limite para não alocar demais com um arquivo estragado.
const MAX_FILES: usize = 200_000;
const MAX_FILE: usize = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub offset: u64,
    pub part: u8,
    pub flags: u8,
    pub size_on_disk: u32,
    pub uncompressed: u32,
}

#[derive(Debug)]
pub struct Package<'a> {
    data: &'a [u8],
    pub version: u32,
    pub entries: Vec<Entry>,
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn u64_at(d: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(o..o + 8)?.try_into().ok()?))
}

fn decompress(method: u8, data: &[u8], size: usize) -> Option<Vec<u8>> {
    if size > MAX_FILE {
        return None;
    }
    match method {
        0 => Some(data.to_vec()),
        1 => {
            let mut out = Vec::with_capacity(size);
            flate2::read::ZlibDecoder::new(data)
                .take(MAX_FILE as u64)
                .read_to_end(&mut out)
                .ok()?;
            Some(out)
        }
        2 => lz4_flex::block::decompress(data, size).ok().or_else(|| {
            let mut out = Vec::with_capacity(size);
            lz4_flex::frame::FrameDecoder::new(data)
                .take(MAX_FILE as u64)
                .read_to_end(&mut out)
                .ok()?;
            Some(out)
        }),
        3 => {
            let mut out = Vec::with_capacity(size);
            zstd::stream::read::Decoder::new(data)
                .ok()?
                .take(MAX_FILE as u64)
                .read_to_end(&mut out)
                .ok()?;
            Some(out)
        }
        _ => None,
    }
}

impl<'a> Package<'a> {
    /// Abre o pacote. `None` se não for LSPK 18 ou estiver estragado.
    pub fn open(data: &'a [u8]) -> Option<Self> {
        if data.get(..4)? != MAGIC {
            return None;
        }
        let version = u32_at(data, 4)?;
        if version != 18 {
            return None;
        }
        let list = usize::try_from(u64_at(data, 8)?).ok()?;
        let count = u32_at(data, list)? as usize;
        let csize = u32_at(data, list + 4)? as usize;
        if count == 0 || count > MAX_FILES {
            return None;
        }
        let comp = data.get(list + 8..list + 8 + csize)?;
        let raw = lz4_flex::block::decompress(comp, count * ENTRY).ok()?;
        let mut entries = Vec::with_capacity(count);
        for e in raw.chunks_exact(ENTRY) {
            let end = e[..256].iter().position(|b| *b == 0).unwrap_or(256);
            let name = String::from_utf8_lossy(&e[..end]).replace('\\', "/");
            let low = u32_at(e, 256)? as u64;
            let high = u16_at(e, 260)? as u64;
            entries.push(Entry {
                name,
                offset: low | (high << 32),
                part: e[262],
                flags: e[263],
                size_on_disk: u32_at(e, 264)?,
                uncompressed: u32_at(e, 268)?,
            });
        }
        Some(Package {
            data,
            version,
            entries,
        })
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name) || e.name.ends_with(&format!("/{name}")))
    }

    /// Conteúdo de um arquivo do pacote (só da parte 0, que é onde ficam os saves).
    pub fn read(&self, e: &Entry) -> Option<Vec<u8>> {
        if e.part != 0 {
            return None;
        }
        let start = usize::try_from(e.offset).ok()?;
        let raw = self.data.get(start..start + e.size_on_disk as usize)?;
        let method = e.flags & 0x0f;
        if method == 0 || e.uncompressed == 0 {
            return Some(raw.to_vec());
        }
        decompress(method, raw, e.uncompressed as usize)
    }
}

/// Monta um pacote LSPK 18 (para testes e ferramentas). Cada arquivo vai com o método dado.
pub fn build(files: &[(&str, &[u8], u8)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&18u32.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes()); // posição da lista, preenchida no fim
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&1u16.to_le_bytes());
    let mut table = Vec::new();
    for (name, data, method) in files {
        let stored = match method {
            1 => {
                let mut e =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                std::io::Write::write_all(&mut e, data).expect("zlib");
                e.finish().expect("zlib")
            }
            2 => lz4_flex::block::compress(data),
            3 => zstd::encode_all(*data, 3).expect("zstd"),
            _ => data.to_vec(),
        };
        let off = out.len() as u64;
        out.extend_from_slice(&stored);
        let mut e = [0u8; ENTRY];
        e[..name.len()].copy_from_slice(name.as_bytes());
        e[256..260].copy_from_slice(&(off as u32).to_le_bytes());
        e[260..262].copy_from_slice(&((off >> 32) as u16).to_le_bytes());
        e[262] = 0;
        e[263] = *method;
        e[264..268].copy_from_slice(&(stored.len() as u32).to_le_bytes());
        let unc = if *method == 0 { 0 } else { data.len() as u32 };
        e[268..272].copy_from_slice(&unc.to_le_bytes());
        table.extend_from_slice(&e);
    }
    let list = out.len() as u64;
    let comp = lz4_flex::block::compress(&table);
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
    out.extend_from_slice(&comp);
    out[8..16].copy_from_slice(&list.to_le_bytes());
    out[16..20].copy_from_slice(&((comp.len() + 8) as u32).to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ida_e_volta_com_cada_compressao() {
        let a = b"{\"x\": 1}".repeat(50);
        let pkg = build(&[
            ("SaveInfo.json", &a, 2),
            ("meta.lsf", b"meta", 0),
            ("Globals.lsf", &a, 1),
            ("Mods/x.lsf", &a, 3),
        ]);
        let p = Package::open(&pkg).unwrap();
        assert_eq!(p.entries.len(), 4);
        for n in ["SaveInfo.json", "Globals.lsf", "x.lsf"] {
            assert_eq!(p.read(p.find(n).unwrap()).unwrap(), a, "{n}");
        }
        assert_eq!(p.read(p.find("meta.lsf").unwrap()).unwrap(), b"meta");
    }

    #[test]
    fn rejeita_lixo() {
        assert!(Package::open(b"LSPK").is_none());
        assert!(Package::open(b"nada disso").is_none());
        let mut pkg = build(&[("a", b"b", 0)]);
        pkg.truncate(pkg.len() - 3);
        assert!(Package::open(&pkg).is_none());
    }
}
