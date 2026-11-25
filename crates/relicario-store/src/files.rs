//! Ler pastas de save para snapshots e restaurar snapshots no disco.

use crate::error::{Error, Result, io};
use crate::repo::Repository;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use relicario_model::{FileEntry, FileKind, Hash, Tree};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Quais arquivos de uma raiz entram no save. Caminhos relativos, com `/`.
#[derive(Debug, Clone)]
pub struct Filter {
    include: GlobSet,
    exclude: GlobSet,
    patterns: (Vec<String>, Vec<String>),
}

impl Filter {
    /// `include` vazio significa "tudo". `*` não atravessa `/`; use `**` para isso.
    pub fn new(include: &[String], exclude: &[String]) -> Result<Self> {
        let build = |pats: &[String]| -> Result<GlobSet> {
            let mut b = GlobSetBuilder::new();
            for p in pats {
                b.add(
                    GlobBuilder::new(p)
                        .literal_separator(true)
                        .build()
                        .map_err(|e| Error::BadPattern(format!("{p}: {e}")))?,
                );
            }
            b.build().map_err(|e| Error::BadPattern(e.to_string()))
        };
        let inc: Vec<String> = if include.is_empty() {
            vec!["**".into()]
        } else {
            include.to_vec()
        };
        Ok(Filter {
            include: build(&inc)?,
            exclude: build(exclude)?,
            patterns: (inc, exclude.to_vec()),
        })
    }

    pub fn all() -> Self {
        Self::new(&[], &[]).expect("filtro padrão")
    }

    pub fn matches(&self, rel: &str) -> bool {
        self.include.is_match(rel) && !self.exclude.is_match(rel)
    }

    pub fn patterns(&self) -> (&[String], &[String]) {
        (&self.patterns.0, &self.patterns.1)
    }
}

/// Uma raiz de save nesta máquina.
#[derive(Debug, Clone)]
pub struct Source {
    /// Nome lógico, igual em todas as máquinas.
    pub root: String,
    pub path: PathBuf,
    pub filter: Filter,
}

/// Arquivos de uma raiz que passam no filtro, em ordem. Links simbólicos são ignorados.
pub fn list_files(src: &Source) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    if !src.path.is_dir() {
        return Ok(out);
    }
    walk(&src.path, "", &mut |rel, path| {
        if src.filter.matches(rel) {
            out.push((rel.to_string(), path.to_path_buf()));
        }
    })?;
    out.sort();
    Ok(out)
}

fn walk(dir: &Path, prefix: &str, f: &mut dyn FnMut(&str, &Path)) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(io(dir))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        if name.starts_with(".relicario-tmp-") {
            continue;
        }
        let ft = e.file_type().map_err(io(&e.path()))?;
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if ft.is_dir() {
            walk(&e.path(), &rel, f)?;
        } else if ft.is_file() {
            f(&rel, &e.path());
        }
    }
    Ok(())
}

/// Recebe cada bloco lido e devolve o id dele no repositório.
type ChunkSink<'a> = &'a mut dyn FnMut(&[u8]) -> Result<Hash>;

struct FileInfo {
    size: u64,
    mtime: i64,
    hash: Hash,
    kind: FileKind,
    chunks: Vec<Hash>,
}

fn read_file(
    path: &Path,
    mut on_chunk: Option<ChunkSink<'_>>,
    repo_params: (u32, u32, u32),
) -> Result<FileInfo> {
    let meta = fs::metadata(path).map_err(io(path))?;
    let mtime = meta.modified().map(crate::time::to_millis).unwrap_or(0);
    let mut file = File::open(path).map_err(io(path))?;
    let mut hasher = blake3::Hasher::new();
    let mut prefix = Vec::with_capacity(512);
    let mut all_zero = true;
    let mut size = 0u64;
    let mut chunks = Vec::new();
    let mut account = |data: &[u8], hasher: &mut blake3::Hasher| {
        hasher.update(data);
        if prefix.len() < 512 {
            let n = (512 - prefix.len()).min(data.len());
            prefix.extend_from_slice(&data[..n]);
        }
        if all_zero && data.iter().any(|b| *b != 0) {
            all_zero = false;
        }
        size += data.len() as u64;
    };
    match on_chunk.as_mut() {
        Some(cb) => {
            let (min, avg, max) = repo_params;
            let cdc = fastcdc::v2020::StreamCDC::new(&mut file, min, avg, max);
            for c in cdc {
                let c = c.map_err(|e| Error::Io {
                    path: path.display().to_string(),
                    source: std::io::Error::other(e.to_string()),
                })?;
                account(&c.data, &mut hasher);
                chunks.push(cb(&c.data)?);
            }
        }
        None => {
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                let n = file.read(&mut buf).map_err(io(path))?;
                if n == 0 {
                    break;
                }
                account(&buf[..n], &mut hasher);
            }
        }
    }
    let kind = FileKind::detect(&prefix, all_zero && size > 0);
    Ok(FileInfo {
        size,
        mtime,
        hash: Hash::from_bytes(*hasher.finalize().as_bytes()),
        kind,
        chunks,
    })
}

/// Lê as raízes e calcula a árvore sem gravar nada (para comparar com o head).
pub fn scan(sources: &[Source]) -> Result<Tree> {
    let mut files = Vec::new();
    for src in sources {
        for (rel, path) in list_files(src)? {
            let i = read_file(&path, None, (0, 0, 0))?;
            files.push(FileEntry {
                root: src.root.clone(),
                path: rel,
                size: i.size,
                mtime: i.mtime,
                hash: i.hash,
                kind: i.kind,
                chunks: vec![],
            });
        }
    }
    Ok(Tree::new(files))
}

/// Lê as raízes, guarda os blocos no repositório e devolve a árvore completa.
pub fn store_files(repo: &mut Repository, sources: &[Source]) -> Result<Tree> {
    let c = repo.config().chunker;
    let params = (c.min, c.avg, c.max);
    let mut files = Vec::new();
    for src in sources {
        for (rel, path) in list_files(src)? {
            let mut cb = |data: &[u8]| repo.put_blob(data);
            let i = read_file(&path, Some(&mut cb), params)?;
            files.push(FileEntry {
                root: src.root.clone(),
                path: rel,
                size: i.size,
                mtime: i.mtime,
                hash: i.hash,
                kind: i.kind,
                chunks: i.chunks,
            });
        }
    }
    Ok(Tree::new(files))
}

/// O que uma restauração fez.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub written: Vec<String>,
    pub unchanged: Vec<String>,
    pub removed: Vec<String>,
}

/// Restaura `tree` nas raízes. Arquivos são escritos num temporário, conferidos pelo hash e só
/// então trocados de lugar. Arquivos que passam no filtro e não estão na árvore são removidos.
///
/// Quem chama é responsável por ter tirado o snapshot de segurança antes.
pub fn restore(repo: &Repository, tree: &Tree, sources: &[Source]) -> Result<RestoreReport> {
    let mut report = RestoreReport::default();
    for src in sources {
        let wanted: Vec<&FileEntry> = tree.files.iter().filter(|f| f.root == src.root).collect();
        let keep: BTreeSet<&str> = wanted.iter().map(|f| f.path.as_str()).collect();
        for f in &wanted {
            let dest = safe_join(&src.path, &f.path)?;
            let label = format!("{}/{}", f.root, f.path);
            if dest.is_file()
                && let Ok(i) = read_file(&dest, None, (0, 0, 0))
                && i.size == f.size
                && i.hash == f.hash
            {
                set_mtime(&dest, f.mtime)?;
                report.unchanged.push(label);
                continue;
            }
            write_file(repo, f, &dest)?;
            report.written.push(label);
        }
        for (rel, path) in list_files(src)? {
            if !keep.contains(rel.as_str()) {
                fs::remove_file(&path).map_err(io(&path))?;
                report.removed.push(format!("{}/{rel}", src.root));
            }
        }
    }
    Ok(report)
}

/// Junta um caminho relativo vindo do repositório sem permitir sair da raiz.
fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let mut p = root.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.contains('\\')
            || part.contains(':')
        {
            return Err(Error::Corrupt(format!(
                "caminho inseguro no snapshot: {rel:?}"
            )));
        }
        p.push(part);
    }
    Ok(p)
}

fn write_file(repo: &Repository, f: &FileEntry, dest: &Path) -> Result<()> {
    let dir = dest.parent().expect("arquivo tem pasta");
    fs::create_dir_all(dir).map_err(io(dir))?;
    let name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("arquivo");
    let tmp = dir.join(format!(".relicario-tmp-{name}"));
    let res = (|| -> Result<()> {
        let mut out = File::create(&tmp).map_err(io(&tmp))?;
        let mut h = blake3::Hasher::new();
        for c in &f.chunks {
            let data = repo.get_blob(c)?;
            h.update(&data);
            out.write_all(&data).map_err(io(&tmp))?;
        }
        out.sync_all().map_err(io(&tmp))?;
        drop(out);
        if Hash::from_bytes(*h.finalize().as_bytes()) != f.hash {
            return Err(Error::Corrupt(format!(
                "{} restaurado não confere com o hash",
                f.path
            )));
        }
        set_mtime(&tmp, f.mtime)?;
        replace(&tmp, dest)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

fn replace(tmp: &Path, dest: &Path) -> Result<()> {
    // No Windows, rename sobre arquivo existente funciona com MoveFileEx(REPLACE_EXISTING), que
    // é o que std::fs::rename usa.
    fs::rename(tmp, dest).map_err(io(dest))
}

fn set_mtime(path: &Path, mtime: i64) -> Result<()> {
    let f = File::options().write(true).open(path).map_err(io(path))?;
    f.set_modified(crate::time::from_millis(mtime))
        .map_err(io(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtro() {
        let f = Filter::new(
            &["save/**".into(), "*.ini".into()],
            &["save-backups/**".into(), "hookshot_options.ini".into()],
        )
        .unwrap();
        assert!(f.matches("save/slot1/caption.sav"));
        assert!(f.matches("game.ini"));
        assert!(!f.matches("hookshot_options.ini"));
        assert!(!f.matches("sub/game.ini"));
        assert!(!f.matches("save-backups/x"));
        assert!(Filter::all().matches("a/b/c"));
    }

    #[test]
    fn caminho_inseguro() {
        let r = Path::new("/tmp/x");
        assert!(safe_join(r, "../etc/passwd").is_err());
        assert!(safe_join(r, "a//b").is_err());
        assert!(safe_join(r, "C:/x").is_err());
        assert_eq!(safe_join(r, "a/b").unwrap(), r.join("a").join("b"));
    }
}
