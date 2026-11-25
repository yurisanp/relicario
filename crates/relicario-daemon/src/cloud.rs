//! Cópias na nuvem: abrir (com segredos do chaveiro), criar, replicar e conferir.

use crate::config::{CloudConfig, Config};
use relicario_store::crypto::{self, Encrypted};
use relicario_store::replicate;
use relicario_transport::{Cloud, CloudSpec, Kind, Limited, LocalFolder, Remote};
use std::sync::Arc;

/// Nome do segredo do destino (chave do S3, senha do WebDAV, token do Drive).
pub fn secret_name(c: &str) -> String {
    format!("nuvem:{c}")
}

/// Nome da senha do cofre.
pub fn vault_name(c: &str) -> String {
    format!("cofre:{c}")
}

fn raw(c: &CloudConfig) -> Result<Arc<dyn Remote>, String> {
    let mut spec = c.spec.clone();
    if let Some(s) = relicario_platform::secrets::get(&secret_name(&c.name)) {
        spec.set_secret(s);
    }
    let cloud = Cloud::open(&spec).map_err(|e| e.to_string())?;
    Ok(match c.limit_kbps {
        Some(k) if k > 0 => Arc::new(Limited::new(cloud, k as u64 * 1024)),
        _ => Arc::new(cloud),
    })
}

/// Abre a cópia como um repositório comum (decifrada).
pub fn open(c: &CloudConfig) -> Result<Arc<dyn Remote>, String> {
    let r = raw(c)?;
    if !c.encrypted {
        return Ok(r);
    }
    let pw = relicario_platform::secrets::get(&vault_name(&c.name)).ok_or_else(|| {
        format!(
            "sem a senha do cofre {:?}; rode `relicario cloud unlock {}`",
            c.name, c.name
        )
    })?;
    Ok(Arc::new(
        Encrypted::open_with_password(r, &pw).map_err(|e| e.to_string())?,
    ))
}

/// O que `add` fez.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Added {
    /// Cofre novo: mostre a chave de recuperação uma vez.
    New { recovery: Option<String> },
    /// Já havia um repositório lá (outra máquina criou): só conectou.
    Joined,
}

/// Conecta (ou cria) uma cópia na nuvem e grava na configuração. `secret` é o segredo do
/// destino; `password` a senha do cofre.
pub fn add(
    cfg: &mut Config,
    mut c: CloudConfig,
    secret: Option<String>,
    password: Option<String>,
) -> Result<Added, String> {
    if cfg.clouds.iter().any(|x| x.name == c.name) {
        return Err(format!("já existe uma nuvem chamada {:?}", c.name));
    }
    if let Some(s) = &secret {
        c.spec.set_secret(s.clone());
    }
    let cloud = Cloud::open(&c.spec).map_err(|e| e.to_string())?;
    cloud
        .check()
        .map_err(|e| format!("não consegui gravar em {}: {e}", c.spec.describe()))?;
    let r: Arc<dyn Remote> = Arc::new(cloud);
    let local_cfg =
        replicate::read_config(&LocalFolder::new(cfg.repo_path())).map_err(|e| e.to_string())?;
    let existing = r
        .get(Kind::Config, "config")
        .map_err(|e| e.to_string())?
        .is_some();
    let added = if crypto::is_encrypted(r.as_ref()).map_err(|e| e.to_string())? {
        let pw = password
            .as_deref()
            .ok_or("este cofre já existe e é cifrado: informe a senha")?;
        Encrypted::open_with_password(r, pw).map_err(|e| e.to_string())?;
        c.encrypted = true;
        Added::Joined
    } else if existing {
        if c.encrypted {
            return Err("já há um repositório sem criptografia nesse destino; use --sem-criptografia para usá-lo".into());
        }
        let theirs = replicate::read_config(r.as_ref()).map_err(|e| e.to_string())?;
        if !replicate::compatible(&local_cfg, &theirs) {
            return Err("o repositório no destino não é compatível com o desta máquina".into());
        }
        Added::Joined
    } else if c.encrypted {
        let pw = password.as_deref().ok_or("informe a senha do cofre")?;
        let (_, rec) = Encrypted::init(r, &local_cfg, pw).map_err(|e| e.to_string())?;
        Added::New {
            recovery: Some(rec),
        }
    } else {
        r.put(
            Kind::Config,
            "config",
            &serde_json::to_vec_pretty(&local_cfg).expect("json"),
        )
        .map_err(|e| e.to_string())?;
        Added::New { recovery: None }
    };
    if let Some(s) = c.spec.secret().filter(|s| !s.is_empty()) {
        relicario_platform::secrets::set(&secret_name(&c.name), s)?;
    }
    if let Some(pw) = &password {
        relicario_platform::secrets::set(&vault_name(&c.name), pw)?;
    }
    c.spec.set_secret(String::new());
    cfg.clouds.push(c);
    cfg.save()?;
    Ok(added)
}

/// Resultado com uma nuvem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudSync {
    pub name: String,
    pub ok: bool,
    /// Login expirado ou revogado: precisa do usuário.
    pub auth: bool,
    pub heads_in: bool,
    pub message: String,
}

pub fn sync_one(cfg: &Config, c: &CloudConfig) -> CloudSync {
    let r = open(c).and_then(|remote| {
        replicate::replicate(
            &LocalFolder::new(cfg.repo_path()),
            remote.as_ref(),
            &cfg.machine(),
        )
        .map_err(|e| e.to_string())
    });
    match r {
        Ok(rep) => CloudSync {
            name: c.name.clone(),
            ok: true,
            auth: false,
            heads_in: !rep.heads_in.is_empty(),
            message: if rep.changed() {
                format!(
                    "{}: {} recebidos, {} enviados ({} KiB)",
                    c.name,
                    rep.pulled,
                    rep.pushed,
                    rep.bytes_out / 1024
                )
            } else {
                format!("{}: em dia", c.name)
            },
        },
        Err(e) => {
            let auth = e.contains("acesso negado") || e.contains("senha");
            CloudSync {
                name: c.name.clone(),
                ok: false,
                auth,
                heads_in: false,
                message: format!("{}: {e}", c.name),
            }
        }
    }
}

pub fn sync_all(cfg: &Config) -> Vec<CloudSync> {
    cfg.clouds.iter().map(|c| sync_one(cfg, c)).collect()
}

/// Conferência: lê todos os blocos da nuvem, confere os hashes e restaura o último snapshot de
/// cada jogo numa pasta temporária, comparando com o que o snapshot diz.
pub fn check(c: &CloudConfig) -> Result<Vec<String>, String> {
    let remote = open(c)?;
    let repo = relicario_store::Repository::open(remote).map_err(|e| e.to_string())?;
    let v = relicario_store::verify::verify(&repo, true).map_err(|e| e.to_string())?;
    let mut msgs = vec![format!(
        "{}: {} snapshots, {} blocos conferidos",
        c.name, v.snapshots, v.blobs_checked
    )];
    msgs.extend(v.errors.iter().map(|e| format!("ERRO: {e}")));
    if !v.ok() {
        return Err(msgs.join("\n"));
    }
    let tmp = std::env::temp_dir().join(format!("relicario-conferencia-{}", std::process::id()));
    let mut seen = std::collections::BTreeSet::new();
    for h in repo.heads().map_err(|e| e.to_string())? {
        for (game, head) in h.heads {
            if !seen.insert(head.snapshot) {
                continue;
            }
            let s = repo
                .get_snapshot(&head.snapshot)
                .map_err(|e| e.to_string())?;
            let tree = repo.get_tree(&s.tree).map_err(|e| e.to_string())?;
            let dir = tmp.join(head.snapshot.short());
            let roots: std::collections::BTreeSet<&str> =
                tree.files.iter().map(|f| f.root.as_str()).collect();
            let sources: Vec<relicario_store::Source> = roots
                .iter()
                .map(|r| {
                    relicario_store::Source::new(
                        *r,
                        dir.join(r),
                        relicario_store::Filter::new(&[], &[]).expect("filtro"),
                    )
                })
                .collect();
            relicario_store::files::restore(&repo, &tree, &sources).map_err(|e| e.to_string())?;
            let back = relicario_store::files::scan(&sources).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_dir_all(&dir);
            if back.content_hash() != tree.content_hash() {
                return Err(format!(
                    "{}: {game} restaurado diferente do snapshot {}",
                    c.name,
                    head.snapshot.short()
                ));
            }
            msgs.push(format!(
                "{game}: {} restaurado e conferido",
                head.snapshot.short()
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(msgs)
}

/// Desbloqueia com a senha ou a chave de recuperação e guarda a senha no chaveiro.
pub fn unlock(
    c: &CloudConfig,
    password: Option<&str>,
    recovery: Option<&str>,
    new_password: Option<&str>,
) -> Result<(), String> {
    let r = raw(c)?;
    let enc = match (password, recovery) {
        (Some(p), _) => Encrypted::open_with_password(r, p),
        (None, Some(k)) => Encrypted::open_with_recovery(r, k),
        (None, None) => return Err("informe a senha ou a chave de recuperação".into()),
    }
    .map_err(|e| e.to_string())?;
    let pw = match new_password.or(password) {
        Some(p) => p,
        None => return Err("com a chave de recuperação, informe também a senha nova".into()),
    };
    if new_password.is_some() {
        enc.set_password(pw).map_err(|e| e.to_string())?;
    }
    relicario_platform::secrets::set(&vault_name(&c.name), pw)?;
    Ok(())
}

/// Descreve uma nuvem sem segredos.
pub fn describe(c: &CloudConfig) -> String {
    let mut s = c.spec.describe();
    if c.encrypted {
        s.push_str(", cifrada");
    } else {
        s.push_str(", SEM criptografia");
    }
    if let Some(k) = c.limit_kbps {
        s.push_str(&format!(", até {k} KiB/s"));
    }
    s
}

/// Para testes e para o `doctor`: um `CloudSpec` está completo?
pub fn spec_ok(s: &CloudSpec) -> bool {
    match s {
        CloudSpec::S3 {
            bucket, access_key, ..
        } => !bucket.is_empty() && !access_key.is_empty(),
        CloudSpec::Webdav { endpoint, .. } => !endpoint.is_empty(),
        CloudSpec::Gdrive { client_id, .. } => !client_id.is_empty(),
        CloudSpec::Pasta { path } => !path.is_empty(),
        CloudSpec::Memory => true,
    }
}
