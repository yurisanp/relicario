//! `relicario cloud`: cópias do repositório na nuvem.

use clap::{Args, Subcommand};
use relicario_daemon::cloud::{self, Added};
use relicario_daemon::config::{CloudConfig, Config};
use relicario_transport::CloudSpec;
use std::process::ExitCode;

type R<T> = Result<T, String>;

#[derive(Subcommand)]
pub enum CloudCmd {
    /// Conecta uma nuvem (cria o cofre se ela estiver vazia)
    Add(Box<AddArgs>),
    /// Lista as nuvens
    List,
    /// Replica agora
    Sync { name: Option<String> },
    /// Baixa tudo, confere os hashes e restaura numa pasta temporária
    Check { name: Option<String> },
    /// Guarda a senha do cofre nesta máquina (ou recupera com a chave de recuperação)
    Unlock {
        name: String,
        /// Usar a chave de recuperação (pede também uma senha nova)
        #[arg(long)]
        recovery: bool,
    },
    /// Entra de novo na conta (token expirado ou revogado)
    Login { name: String },
    /// Desconecta (não apaga nada da nuvem)
    Remove { name: String },
}

#[derive(Args)]
pub struct AddArgs {
    /// Nome curto, por exemplo `drive` ou `r2`
    name: String,
    /// Google Drive (abre o navegador para entrar na conta)
    #[arg(long, group = "tipo")]
    gdrive: bool,
    /// S3 ou compatível: nome do bucket
    #[arg(long, group = "tipo")]
    s3: Option<String>,
    /// WebDAV/Nextcloud: URL
    #[arg(long, group = "tipo")]
    webdav: Option<String>,
    /// Uma pasta qualquer (já sincronizada por outro programa)
    #[arg(long, group = "tipo")]
    pasta: Option<String>,
    /// S3: endpoint (R2, B2, MinIO…); vazio para AWS
    #[arg(long, default_value = "")]
    endpoint: String,
    #[arg(long, default_value = "auto")]
    region: String,
    /// Pasta dentro do destino
    #[arg(long, default_value = "relicario")]
    root: String,
    /// S3: chave de acesso. A chave secreta vem de RELICARIO_SECRET ou é pedida.
    #[arg(long, default_value = "")]
    access_key: String,
    /// WebDAV: usuário. A senha vem de RELICARIO_SECRET ou é pedida.
    #[arg(long, default_value = "")]
    user: String,
    /// Sem criptografia (só para destinos que você controla)
    #[arg(long)]
    sem_criptografia: bool,
    /// Limite de banda, em KiB/s
    #[arg(long)]
    limite: Option<u32>,
}

fn prompt(env: &str, question: &str) -> R<String> {
    if let Ok(v) = std::env::var(env) {
        return Ok(v);
    }
    rpassword::prompt_password(question)
        .map_err(|e| format!("não consegui ler do terminal ({e}); use {env}"))
}

fn new_password() -> R<String> {
    if let Ok(v) = std::env::var("RELICARIO_PASSWORD") {
        return Ok(v);
    }
    let a = prompt("RELICARIO_PASSWORD", "Senha do cofre: ")?;
    let b = prompt("RELICARIO_PASSWORD", "Repita a senha: ")?;
    if a != b {
        return Err("as senhas não conferem".into());
    }
    if a.chars().count() < 8 {
        return Err("use uma senha com pelo menos 8 caracteres".into());
    }
    Ok(a)
}

fn gdrive_login() -> R<(String, String, String)> {
    let (id, secret) = relicario_daemon::oauth::client().ok_or(
        "esta versão ainda não tem o client do Google Drive; defina RELICARIO_GDRIVE_CLIENT_ID e RELICARIO_GDRIVE_CLIENT_SECRET",
    )?;
    let p = relicario_daemon::oauth::start(&id)?;
    println!(
        "Abrindo o navegador para entrar no Google. Se não abrir, use este endereço:\n\n  {}\n",
        p.url
    );
    relicario_daemon::oauth::open_browser(&p.url);
    let code = relicario_daemon::oauth::wait_code(&p, std::time::Duration::from_secs(300))?;
    let token = relicario_daemon::oauth::exchange(&p, &id, &secret, &code)?;
    Ok((id, secret, token))
}

fn find<'a>(cfg: &'a Config, name: &str) -> R<&'a CloudConfig> {
    cfg.clouds
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| format!("nenhuma nuvem chamada {name:?} (veja `relicario cloud list`)"))
}

fn selected<'a>(cfg: &'a Config, name: &Option<String>) -> R<Vec<&'a CloudConfig>> {
    match name {
        Some(n) => Ok(vec![find(cfg, n)?]),
        None if cfg.clouds.is_empty() => {
            Err("nenhuma nuvem conectada (use `relicario cloud add`)".into())
        }
        None => Ok(cfg.clouds.iter().collect()),
    }
}

pub fn cmd_cloud(c: CloudCmd) -> R<ExitCode> {
    let mut cfg = Config::load()?;
    match c {
        CloudCmd::Add(a) => {
            let (spec, secret) = if a.gdrive {
                let (client_id, client_secret, token) = gdrive_login()?;
                let root = format!("/{}", a.root.trim_matches('/'));
                (
                    CloudSpec::Gdrive {
                        root,
                        client_id,
                        client_secret,
                        refresh_token: String::new(),
                    },
                    Some(token),
                )
            } else if let Some(bucket) = a.s3 {
                if a.access_key.is_empty() {
                    return Err("informe --access-key".into());
                }
                let secret = prompt("RELICARIO_SECRET", "Chave secreta do S3: ")?;
                (
                    CloudSpec::S3 {
                        endpoint: a.endpoint,
                        bucket,
                        region: a.region,
                        root: a.root,
                        access_key: a.access_key,
                        secret_key: String::new(),
                    },
                    Some(secret),
                )
            } else if let Some(endpoint) = a.webdav {
                let secret = if a.user.is_empty() {
                    None
                } else {
                    Some(prompt("RELICARIO_SECRET", "Senha do WebDAV: ")?)
                };
                (
                    CloudSpec::Webdav {
                        endpoint,
                        root: a.root,
                        user: a.user,
                        password: String::new(),
                    },
                    secret,
                )
            } else if let Some(path) = a.pasta {
                let p = std::path::absolute(relicario_platform::expand_home(&path))
                    .map_err(|e| e.to_string())?;
                (
                    CloudSpec::Pasta {
                        path: p.to_string_lossy().into_owned(),
                    },
                    None,
                )
            } else {
                return Err("escolha o destino: --gdrive, --s3 <bucket>, --webdav <url> ou --pasta <caminho>".into());
            };
            let encrypted = !a.sem_criptografia;
            let probe = cloud_has_vault(&spec, secret.as_deref());
            let password = if !encrypted {
                None
            } else if probe {
                Some(prompt(
                    "RELICARIO_PASSWORD",
                    "Este cofre já existe. Senha do cofre: ",
                )?)
            } else {
                Some(new_password()?)
            };
            let name = a.name.clone();
            let c = CloudConfig {
                name: a.name,
                spec,
                encrypted,
                limit_kbps: a.limite,
            };
            match cloud::add(&mut cfg, c, secret, password)? {
                Added::New { recovery: Some(r) } => {
                    println!("cofre criado em {name}.\n");
                    println!(
                        "CHAVE DE RECUPERAÇÃO (anote ou imprima; é o único jeito de abrir o cofre sem a senha):\n\n    {r}\n"
                    );
                }
                Added::New { recovery: None } => {
                    println!("nuvem {name} conectada, sem criptografia")
                }
                Added::Joined => println!("conectado ao repositório que já existia em {name}"),
            }
            let cfg = Config::load()?;
            let r = cloud::sync_one(&cfg, find(&cfg, &name)?);
            println!("{}", r.message);
            notify_daemon();
        }
        CloudCmd::List => {
            if cfg.clouds.is_empty() {
                println!("nenhuma nuvem conectada");
            }
            for c in &cfg.clouds {
                println!("{}  {}", c.name, cloud::describe(c));
            }
        }
        CloudCmd::Sync { name } => {
            let mut code = ExitCode::SUCCESS;
            for c in selected(&cfg, &name)? {
                let r = cloud::sync_one(&cfg, c);
                if !r.ok {
                    code = ExitCode::from(2);
                }
                println!("{}", r.message);
                if r.auth {
                    println!(
                        "  rode `relicario cloud login {}` ou `relicario cloud unlock {}`",
                        c.name, c.name
                    );
                }
            }
            return Ok(code);
        }
        CloudCmd::Check { name } => {
            for c in selected(&cfg, &name)? {
                cloud::check(c)?.iter().for_each(|m| println!("{m}"));
            }
        }
        CloudCmd::Unlock { name, recovery } => {
            let c = find(&cfg, &name)?.clone();
            if recovery {
                let k = prompt("RELICARIO_RECOVERY", "Chave de recuperação: ")?;
                let pw = new_password()?;
                cloud::unlock(&c, None, Some(&k), Some(&pw))?;
                println!("senha nova gravada; a chave de recuperação continua valendo");
            } else {
                let pw = prompt("RELICARIO_PASSWORD", "Senha do cofre: ")?;
                cloud::unlock(&c, Some(&pw), None, None)?;
                println!("senha guardada nesta máquina");
            }
        }
        CloudCmd::Login { name } => {
            let c = find(&cfg, &name)?.clone();
            let secret = match &c.spec {
                CloudSpec::Gdrive { .. } => gdrive_login()?.2,
                CloudSpec::S3 { .. } => prompt("RELICARIO_SECRET", "Chave secreta do S3: ")?,
                CloudSpec::Webdav { .. } => prompt("RELICARIO_SECRET", "Senha do WebDAV: ")?,
                _ => return Err("este destino não tem login".into()),
            };
            relicario_platform::secrets::set(&cloud::secret_name(&name), &secret)?;
            println!("{}", cloud::sync_one(&cfg, &c).message);
        }
        CloudCmd::Remove { name } => {
            find(&cfg, &name)?;
            cfg.clouds.retain(|c| c.name != name);
            cfg.save()?;
            relicario_platform::secrets::delete(&cloud::secret_name(&name));
            relicario_platform::secrets::delete(&cloud::vault_name(&name));
            notify_daemon();
            println!("{name} desconectada (os arquivos na nuvem continuam lá)");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cloud_has_vault(spec: &CloudSpec, secret: Option<&str>) -> bool {
    let mut s = spec.clone();
    if let Some(x) = secret {
        s.set_secret(x.to_string());
    }
    relicario_transport::Cloud::open(&s)
        .ok()
        .and_then(|c| relicario_store::crypto::is_encrypted(&c).ok())
        .unwrap_or(false)
}

fn notify_daemon() {
    if relicario_daemon::ipc::is_up() {
        let _ = relicario_daemon::ipc::request(&relicario_daemon::ipc::Request::Reload);
    }
}

/// `relicario import ludusavi`.
pub fn cmd_import_ludusavi(
    backups: Option<String>,
    config: Option<String>,
    game: Option<String>,
    no_custom: bool,
) -> R<ExitCode> {
    use relicario_daemon::ludusavi;
    let mut cfg = Config::load()?;
    let lcfg = config
        .map(|c| relicario_platform::expand_home(&c))
        .or_else(ludusavi::default_config_path);
    if !no_custom && let Some(lc) = &lcfg {
        let (custom, skipped) = ludusavi::custom_games(lc)?;
        let mut added = 0;
        for g in custom {
            if !cfg.games.iter().any(|x| x.id == g.id) {
                cfg.games.push(g);
                added += 1;
            }
        }
        if added > 0 {
            cfg.save()?;
            println!("{added} jogos personalizados do Ludusavi cadastrados");
        }
        for s in skipped {
            println!("  pulado (caminho com marcador que não sei resolver): {s}");
        }
    }
    let root = backups
        .map(|b| relicario_platform::expand_home(&b))
        .or_else(|| lcfg.as_deref().and_then(ludusavi::backup_path))
        .ok_or("não achei a pasta de backups do Ludusavi; use --backups <pasta>")?;
    let all = relicario_daemon::games::all(&cfg)?;
    let mut problems = 0;
    let (mut snaps, mut files, mut bytes) = (0, 0, 0u64);
    for r in ludusavi::import_backups(&cfg, &all, &root, game.as_deref())? {
        match r {
            Ok(g) => {
                println!(
                    "{} → {}: {} snapshots, {} arquivos{}",
                    g.name,
                    g.id,
                    g.snapshots,
                    g.files,
                    if g.unmapped > 0 {
                        format!(
                            " ({} fora das pastas conhecidas, guardados em `ludusavi/`)",
                            g.unmapped
                        )
                    } else {
                        String::new()
                    }
                );
                for p in &g.problems {
                    println!("  PROBLEMA: {p}");
                }
                problems += g.problems.len();
                snaps += g.snapshots;
                files += g.files;
                bytes += g.bytes;
            }
            Err(e) => {
                problems += 1;
                println!("ERRO: {e}");
            }
        }
    }
    println!(
        "importados {snaps} snapshots, {files} arquivos ({} MiB); conferindo o repositório…",
        bytes / (1024 * 1024)
    );
    let repo = cfg.open_repo()?;
    let v = relicario_store::verify::verify(&repo, true).map_err(|e| e.to_string())?;
    if !v.ok() {
        return Err(format!(
            "o repositório tem problemas depois da importação: {:?}",
            v.errors
        ));
    }
    println!("tudo conferido. Os backups viram histórico; nada foi restaurado.");
    Ok(if problems > 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    })
}
