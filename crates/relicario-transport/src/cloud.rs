//! Destinos de nuvem pelo `opendal`: S3 e compatíveis, WebDAV/Nextcloud e Google Drive.
//!
//! O `opendal` é assíncrono; aqui ele roda num runtime próprio e é visto como um [`Remote`]
//! síncrono comum. Onde o serviço permite renomear, a escrita vai para um temporário e depois é
//! renomeada (atômica); no S3 e no Drive o envio já é atômico.

use crate::{Error, Kind, Remote, Result, validate_name};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

/// Como chegar num destino de nuvem. Segredos (`secret_key`, `password`, `refresh_token`) são
/// guardados no chaveiro do sistema pelo daemon; aqui chegam já preenchidos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CloudSpec {
    S3 {
        /// Vazio para a AWS; URL para R2, B2, MinIO, Wasabi…
        #[serde(default)]
        endpoint: String,
        bucket: String,
        #[serde(default = "auto")]
        region: String,
        /// Pasta dentro do bucket.
        #[serde(default)]
        root: String,
        access_key: String,
        #[serde(default, skip_serializing)]
        secret_key: String,
    },
    Webdav {
        endpoint: String,
        #[serde(default)]
        root: String,
        #[serde(default)]
        user: String,
        #[serde(default, skip_serializing)]
        password: String,
    },
    Gdrive {
        /// Pasta no Drive, por exemplo `/Relicario`.
        #[serde(default = "gdrive_root")]
        root: String,
        client_id: String,
        #[serde(default, skip_serializing)]
        client_secret: String,
        #[serde(default, skip_serializing)]
        refresh_token: String,
    },
    /// Uma pasta vista como nuvem (pasta já sincronizada por outro programa, e testes).
    Pasta { path: String },
    /// Na memória, para testes.
    Memory,
}

fn auto() -> String {
    "auto".into()
}

fn gdrive_root() -> String {
    "/Relicario".into()
}

impl CloudSpec {
    /// Nome curto do tipo, para mensagens e para a chave no chaveiro.
    pub fn kind(&self) -> &'static str {
        match self {
            CloudSpec::S3 { .. } => "s3",
            CloudSpec::Webdav { .. } => "webdav",
            CloudSpec::Gdrive { .. } => "gdrive",
            CloudSpec::Pasta { .. } => "pasta",
            CloudSpec::Memory => "memory",
        }
    }

    /// O segredo deste destino (para guardar no chaveiro).
    pub fn secret(&self) -> Option<&str> {
        match self {
            CloudSpec::S3 { secret_key, .. } => Some(secret_key),
            CloudSpec::Webdav { password, .. } => Some(password),
            CloudSpec::Gdrive { refresh_token, .. } => Some(refresh_token),
            CloudSpec::Pasta { .. } | CloudSpec::Memory => None,
        }
    }

    /// Preenche o segredo lido do chaveiro.
    pub fn set_secret(&mut self, s: String) {
        match self {
            CloudSpec::S3 { secret_key, .. } => *secret_key = s,
            CloudSpec::Webdav { password, .. } => *password = s,
            CloudSpec::Gdrive { refresh_token, .. } => *refresh_token = s,
            CloudSpec::Pasta { .. } | CloudSpec::Memory => {}
        }
    }

    /// Descrição sem segredos.
    pub fn describe(&self) -> String {
        match self {
            CloudSpec::S3 {
                endpoint,
                bucket,
                root,
                ..
            } => {
                let host = if endpoint.is_empty() {
                    "AWS"
                } else {
                    endpoint.as_str()
                };
                format!("S3 {bucket}/{root} ({host})")
            }
            CloudSpec::Webdav { endpoint, root, .. } => format!("WebDAV {endpoint}{root}"),
            CloudSpec::Gdrive { root, .. } => format!("Google Drive {root}"),
            CloudSpec::Pasta { path } => format!("pasta {path}"),
            CloudSpec::Memory => "memória".into(),
        }
    }
}

fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("relicario-nuvem")
            .enable_all()
            .build()
            .expect("runtime da nuvem")
    })
}

/// Um destino de nuvem.
pub struct Cloud {
    op: opendal::Operator,
    desc: String,
    rename: bool,
}

impl std::fmt::Debug for Cloud {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cloud")
            .field("desc", &self.desc)
            .finish_non_exhaustive()
    }
}

fn cfg_err(e: opendal::Error) -> Error {
    Error::Other(format!("configuração do destino: {e}"))
}

fn map_err(path: &str, e: opendal::Error) -> Error {
    match e.kind() {
        opendal::ErrorKind::PermissionDenied => Error::Auth(format!("{path}: {e}")),
        _ => {
            let msg = e.to_string();
            if msg.contains("401") || msg.contains("invalid_grant") || msg.contains("Unauthorized")
            {
                Error::Auth(format!("{path}: {e}"))
            } else {
                Error::Io {
                    path: path.into(),
                    source: std::io::Error::other(msg),
                }
            }
        }
    }
}

impl Cloud {
    pub fn open(spec: &CloudSpec) -> Result<Self> {
        use opendal::services;
        let op = match spec {
            CloudSpec::S3 {
                endpoint,
                bucket,
                region,
                root,
                access_key,
                secret_key,
            } => {
                let mut b = services::S3::default()
                    .bucket(bucket)
                    .region(region)
                    .root(&format!("/{}", root.trim_matches('/')))
                    .access_key_id(access_key)
                    .secret_access_key(secret_key)
                    .disable_config_load()
                    .disable_ec2_metadata();
                if !endpoint.is_empty() {
                    b = b.endpoint(endpoint);
                }
                opendal::Operator::new(b).map_err(cfg_err)?.finish()
            }
            CloudSpec::Webdav {
                endpoint,
                root,
                user,
                password,
            } => {
                let mut b = services::Webdav::default()
                    .endpoint(endpoint)
                    .root(&format!("/{}", root.trim_matches('/')));
                if !user.is_empty() {
                    b = b.username(user).password(password);
                }
                opendal::Operator::new(b).map_err(cfg_err)?.finish()
            }
            CloudSpec::Gdrive {
                root,
                client_id,
                client_secret,
                refresh_token,
            } => {
                let b = services::Gdrive::default()
                    .root(root)
                    .client_id(client_id)
                    .client_secret(client_secret)
                    .refresh_token(refresh_token);
                opendal::Operator::new(b).map_err(cfg_err)?.finish()
            }
            CloudSpec::Pasta { path } => {
                std::fs::create_dir_all(path).map_err(|e| Error::Io {
                    path: path.clone(),
                    source: e,
                })?;
                opendal::Operator::new(services::Fs::default().root(path))
                    .map_err(cfg_err)?
                    .finish()
            }
            CloudSpec::Memory => opendal::Operator::new(services::Memory::default())
                .map_err(cfg_err)?
                .finish(),
        };
        let op = op
            .layer(
                opendal::layers::RetryLayer::new()
                    .with_max_times(4)
                    .with_jitter(),
            )
            .layer(
                opendal::layers::TimeoutLayer::new()
                    .with_timeout(Duration::from_secs(120))
                    .with_io_timeout(Duration::from_secs(60)),
            );
        let rename = op.info().full_capability().rename
            && matches!(spec, CloudSpec::Webdav { .. } | CloudSpec::Pasta { .. });
        Ok(Cloud {
            op,
            desc: spec.describe(),
            rename,
        })
    }

    fn path(kind: Kind, name: &str) -> Result<String> {
        validate_name(name)?;
        Ok(match (kind.dir(), kind.sharded() && name.len() >= 2) {
            ("", _) => name.to_string(),
            (d, true) => format!("{d}/{}/{name}", &name[..2]),
            (d, false) => format!("{d}/{name}"),
        })
    }

    /// Confere se dá para ler e escrever (para `relicario cloud add` e o `doctor`).
    pub fn check(&self) -> Result<()> {
        let p = format!(".relicario-teste-{}", std::process::id());
        rt().block_on(async {
            self.op
                .write(&p, b"ok".to_vec())
                .await
                .map_err(|e| map_err(&p, e))?;
            let _ = self.op.delete(&p).await;
            Ok(())
        })
    }
}

impl Remote for Cloud {
    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> Result<()> {
        let path = Self::path(kind, name)?;
        let data = data.to_vec();
        rt().block_on(async {
            if self.rename {
                let mut r = [0u8; 8];
                getrandom::fill(&mut r).map_err(|e| Error::Other(e.to_string()))?;
                let tmp = format!("{}.tmp-{:016x}", path, u64::from_le_bytes(r));
                let tmp = match path.rsplit_once('/') {
                    Some((d, n)) => format!("{d}/.tmp-{n}-{:016x}", u64::from_le_bytes(r)),
                    None => tmp,
                };
                self.op
                    .write(&tmp, data)
                    .await
                    .map_err(|e| map_err(&tmp, e))?;
                self.op
                    .rename(&tmp, &path)
                    .await
                    .map_err(|e| map_err(&path, e))?;
            } else {
                self.op
                    .write(&path, data)
                    .await
                    .map_err(|e| map_err(&path, e))?;
            }
            Ok(())
        })
    }

    fn get(&self, kind: Kind, name: &str) -> Result<Option<Vec<u8>>> {
        let path = Self::path(kind, name)?;
        rt().block_on(async {
            match self.op.read(&path).await {
                Ok(b) => Ok(Some(b.to_vec())),
                Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(map_err(&path, e)),
            }
        })
    }

    fn list(&self, kind: Kind) -> Result<Vec<String>> {
        let dir = match kind.dir() {
            "" => "/".to_string(),
            d => format!("{d}/"),
        };
        rt().block_on(async {
            let entries = match self.op.list_with(&dir).recursive(kind.sharded()).await {
                Ok(v) => v,
                Err(e) if e.kind() == opendal::ErrorKind::NotFound => return Ok(vec![]),
                Err(e) => return Err(map_err(&dir, e)),
            };
            let mut out: Vec<String> = entries
                .into_iter()
                .filter(|e| e.metadata().mode().is_file())
                .filter_map(|e| e.path().rsplit('/').next().map(str::to_string))
                .filter(|n| !n.is_empty() && !n.starts_with('.') && validate_name(n).is_ok())
                .filter(|n| kind != Kind::Config || n == "config")
                .collect();
            out.sort();
            out.dedup();
            Ok(out)
        })
    }

    fn delete(&self, kind: Kind, name: &str) -> Result<()> {
        let path = Self::path(kind, name)?;
        rt().block_on(async {
            match self.op.delete(&path).await {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(map_err(&path, e)),
            }
        })
    }

    fn describe(&self) -> String {
        self.desc.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conformidade_na_memoria() {
        let c = Cloud::open(&CloudSpec::Memory).unwrap();
        crate::conformance::run(&c);
    }

    #[test]
    fn conformidade_numa_pasta() {
        let t = tempfile::tempdir().unwrap();
        let c = Cloud::open(&CloudSpec::Pasta {
            path: t.path().to_string_lossy().into_owned(),
        })
        .unwrap();
        crate::conformance::run(&c);
    }

    #[test]
    fn segredos_nao_vao_para_a_configuracao() {
        let s = CloudSpec::S3 {
            endpoint: "https://x.r2.cloudflarestorage.com".into(),
            bucket: "saves".into(),
            region: "auto".into(),
            root: "relicario".into(),
            access_key: "AK".into(),
            secret_key: "SEGREDO".into(),
        };
        let j = serde_json::to_string(&s).unwrap();
        assert!(!j.contains("SEGREDO"), "{j}");
    }
}
