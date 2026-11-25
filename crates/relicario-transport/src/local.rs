use crate::{Error, Kind, Remote, Result, validate_name};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const TMP_PREFIX: &str = ".tmp-";

/// Repositório numa pasta do sistema de arquivos.
#[derive(Debug, Clone)]
pub struct LocalFolder {
    root: PathBuf,
}

impl LocalFolder {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn dir(&self, kind: Kind, name: &str) -> PathBuf {
        let mut p = self.root.clone();
        if !kind.dir().is_empty() {
            p.push(kind.dir());
        }
        if kind.sharded() {
            p.push(&name[..name.len().min(2)]);
        }
        p
    }

    fn path(&self, kind: Kind, name: &str) -> PathBuf {
        self.dir(kind, name).join(name)
    }
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.display().to_string(),
        source,
    }
}

fn random_suffix() -> String {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("gerador aleatório do sistema");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

impl Remote for LocalFolder {
    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> Result<()> {
        validate_name(name)?;
        let dir = self.dir(kind, name);
        fs::create_dir_all(&dir).map_err(io_err(&dir))?;
        let tmp = dir.join(format!("{TMP_PREFIX}{name}-{}", random_suffix()));
        let fin = dir.join(name);
        let res = (|| {
            let mut f = File::create(&tmp)?;
            f.write_all(data)?;
            f.sync_all()?;
            drop(f);
            fs::rename(&tmp, &fin)?;
            sync_dir(&dir)
        })();
        if res.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        res.map_err(io_err(&fin))
    }

    fn get(&self, kind: Kind, name: &str) -> Result<Option<Vec<u8>>> {
        validate_name(name)?;
        let p = self.path(kind, name);
        match fs::read(&p) {
            Ok(d) => Ok(Some(d)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(&p)(e)),
        }
    }

    fn list(&self, kind: Kind) -> Result<Vec<String>> {
        let mut base = self.root.clone();
        if !kind.dir().is_empty() {
            base.push(kind.dir());
        }
        let mut out = Vec::new();
        let dirs: Vec<PathBuf> = if kind.sharded() {
            match fs::read_dir(&base) {
                Ok(rd) => rd
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .map(|e| e.path())
                    .collect(),
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
                Err(e) => return Err(io_err(&base)(e)),
            }
        } else {
            vec![base]
        };
        for d in dirs {
            let rd = match fs::read_dir(&d) {
                Ok(rd) => rd,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io_err(&d)(e)),
            };
            for e in rd {
                let e = e.map_err(io_err(&d))?;
                if !e.file_type().map_err(io_err(&d))?.is_file() {
                    continue;
                }
                let Some(n) = e.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                if validate_name(&n).is_ok() {
                    out.push(n);
                }
            }
        }
        out.sort();
        Ok(out)
    }

    fn delete(&self, kind: Kind, name: &str) -> Result<()> {
        validate_name(name)?;
        let p = self.path(kind, name);
        match fs::remove_file(&p) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_err(&p)(e)),
        }
    }

    fn describe(&self) -> String {
        format!("pasta {}", self.root.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conformidade() {
        let dir = tempfile::tempdir().unwrap();
        crate::conformance::run(&LocalFolder::new(dir.path()));
    }

    #[test]
    fn ignora_temporarios_de_escrita_interrompida() {
        let dir = tempfile::tempdir().unwrap();
        let r = LocalFolder::new(dir.path());
        r.put(Kind::Snapshots, "abc", b"1").unwrap();
        fs::write(dir.path().join("snapshots").join(".tmp-xyz-123"), b"metade").unwrap();
        assert_eq!(r.list(Kind::Snapshots).unwrap(), vec!["abc".to_string()]);
    }
}
