//! Cópia cifrada do repositório (nuvem).
//!
//! [`Encrypted`] embrulha um destino e mostra por cima dele um repositório comum, sem
//! criptografia: o resto do Relicário (replicação, verificação, restauração) não muda. Por baixo,
//! cada arquivo é cifrado com XChaCha20-Poly1305 usando a chave mestra, com o caminho do arquivo
//! como dado associado (um arquivo trocado de lugar não decifra). A chave mestra fica em `keys/`,
//! cifrada duas vezes: por Argon2id(senha) e pela chave de recuperação.
//!
//! O que fica visível para quem tem acesso ao destino: o `config`, os nomes dos arquivos (hashes
//! e ids de máquina) e os tamanhos.

use crate::config::RepoConfig;
use crate::{Error, Result};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use relicario_transport::{Kind, Remote};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const ALGORITHM: &str = "xchacha20poly1305";
const MAGIC: &[u8; 8] = b"RLCENC1\0";
const CONFIG: &str = "config";
const KEY_PASSWORD: &str = "password";
const KEY_RECOVERY: &str = "recovery";

/// Parâmetros do Argon2id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        // RELICARIO_KDF_FAST deixa os testes rápidos; nunca em uso real.
        if std::env::var_os("RELICARIO_KDF_FAST").is_some() {
            return KdfParams {
                m_kib: 1024,
                t: 1,
                p: 1,
            };
        }
        KdfParams {
            m_kib: 64 * 1024,
            t: 3,
            p: 1,
        }
    }
}

/// Chave mestra embrulhada, gravada em `keys/<nome>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WrappedKey {
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kdf: Option<KdfParams>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    salt: Vec<u8>,
    nonce: Vec<u8>,
    key: Vec<u8>,
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("aleatório");
    b
}

fn derive(password: &str, salt: &[u8], p: KdfParams) -> Result<[u8; 32]> {
    let params = argon2::Params::new(p.m_kib, p.t, p.p, Some(32))
        .map_err(|e| Error::Corrupt(format!("argon2: {e}")))?;
    let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut out = [0u8; 32];
    a.hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| Error::Corrupt(format!("argon2: {e}")))?;
    Ok(out)
}

fn seal(key: &[u8; 32], aad: &[u8], plain: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let nonce: [u8; 24] = random();
    let c = XChaCha20Poly1305::new(key.into());
    let ct = c
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad })
        .expect("cifrar");
    (nonce.to_vec(), ct)
}

fn open(key: &[u8; 32], aad: &[u8], nonce: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    if nonce.len() != 24 {
        return None;
    }
    XChaCha20Poly1305::new(key.into())
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad })
        .ok()
}

/// Chave de recuperação em texto: 32 bytes em base32, grupos de 4.
pub fn format_recovery(k: &[u8; 32]) -> String {
    const A: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bits = 0u32;
    let mut n = 0;
    let mut s = String::new();
    for &b in k {
        bits = (bits << 8) | b as u32;
        n += 8;
        while n >= 5 {
            n -= 5;
            s.push(A[((bits >> n) & 31) as usize] as char);
        }
    }
    if n > 0 {
        s.push(A[((bits << (5 - n)) & 31) as usize] as char);
    }
    s.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).expect("ascii"))
        .collect::<Vec<_>>()
        .join("-")
}

pub fn parse_recovery(s: &str) -> Option<[u8; 32]> {
    const A: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bits = 0u64;
    let mut n = 0;
    let mut out = Vec::new();
    for c in s.chars().filter(|c| c.is_ascii_alphanumeric()) {
        let c = match c.to_ascii_uppercase() {
            'O' => '0',
            'I' => '1',
            x => x,
        };
        let v = A.iter().position(|&a| a as char == c)? as u64;
        bits = (bits << 5) | v;
        n += 5;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
            bits &= (1 << n) - 1;
        }
    }
    out.try_into().ok()
}

fn aad(kind: Kind, name: &str) -> Vec<u8> {
    format!("relicario-v1\0{}/{name}", kind.dir()).into_bytes()
}

/// Repositório cifrado visto como um comum.
pub struct Encrypted {
    inner: Arc<dyn Remote>,
    key: [u8; 32],
}

impl std::fmt::Debug for Encrypted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encrypted")
            .field("inner", &self.inner.describe())
            .finish_non_exhaustive()
    }
}

fn read_raw_config(r: &dyn Remote) -> Result<Option<RepoConfig>> {
    match r.get(Kind::Config, CONFIG)? {
        None => Ok(None),
        Some(b) => serde_json::from_slice(&b)
            .map(Some)
            .map_err(|e| Error::Corrupt(format!("config: {e}"))),
    }
}

/// O destino já tem um repositório cifrado?
pub fn is_encrypted(r: &dyn Remote) -> Result<bool> {
    Ok(read_raw_config(r)?.is_some_and(|c| c.encryption == ALGORITHM))
}

impl Encrypted {
    /// Cria a cópia cifrada num destino vazio, com o mesmo id e parâmetros do repositório local.
    /// Devolve a chave de recuperação, que deve ser mostrada uma vez ao usuário.
    pub fn init(
        inner: Arc<dyn Remote>,
        local: &RepoConfig,
        password: &str,
    ) -> Result<(Self, String)> {
        if read_raw_config(inner.as_ref())?.is_some() {
            return Err(Error::AlreadyExists);
        }
        let key: [u8; 32] = random();
        let recovery: [u8; 32] = random();
        let kdf = KdfParams::default();
        let salt: [u8; 16] = random();
        let pk = derive(password, &salt, kdf)?;
        let (nonce, wrapped) = seal(&pk, b"relicario-chave-senha", &key);
        let w = WrappedKey {
            kind: "argon2id".into(),
            kdf: Some(kdf),
            salt: salt.to_vec(),
            nonce,
            key: wrapped,
        };
        inner.put(
            Kind::Keys,
            KEY_PASSWORD,
            &serde_json::to_vec_pretty(&w).expect("json"),
        )?;
        let (nonce, wrapped) = seal(&recovery, b"relicario-chave-recuperacao", &key);
        let w = WrappedKey {
            kind: "recovery".into(),
            kdf: None,
            salt: vec![],
            nonce,
            key: wrapped,
        };
        inner.put(
            Kind::Keys,
            KEY_RECOVERY,
            &serde_json::to_vec_pretty(&w).expect("json"),
        )?;
        let mut cfg = local.clone();
        cfg.encryption = ALGORITHM.into();
        // Config por último: um destino sem config ainda não é repositório.
        inner.put(
            Kind::Config,
            CONFIG,
            &serde_json::to_vec_pretty(&cfg).expect("json"),
        )?;
        Ok((Encrypted { inner, key }, format_recovery(&recovery)))
    }

    fn wrapped(inner: &dyn Remote, name: &str) -> Result<WrappedKey> {
        let b = inner
            .get(Kind::Keys, name)?
            .ok_or_else(|| Error::Corrupt(format!("falta keys/{name}")))?;
        serde_json::from_slice(&b).map_err(|e| Error::Corrupt(format!("keys/{name}: {e}")))
    }

    fn check(inner: &dyn Remote) -> Result<()> {
        match read_raw_config(inner)? {
            None => Err(Error::NotARepository),
            Some(c) if c.encryption != ALGORITHM => Err(Error::Corrupt(format!(
                "o repositório não é cifrado com {ALGORITHM} ({})",
                c.encryption
            ))),
            Some(_) => Ok(()),
        }
    }

    /// Abre com a senha.
    pub fn open_with_password(inner: Arc<dyn Remote>, password: &str) -> Result<Self> {
        Self::check(inner.as_ref())?;
        let w = Self::wrapped(inner.as_ref(), KEY_PASSWORD)?;
        let pk = derive(password, &w.salt, w.kdf.unwrap_or_default())?;
        let key =
            open(&pk, b"relicario-chave-senha", &w.nonce, &w.key).ok_or(Error::WrongPassword)?;
        Ok(Encrypted {
            inner,
            key: key.try_into().map_err(|_| Error::Corrupt("chave".into()))?,
        })
    }

    /// Abre com a chave de recuperação.
    pub fn open_with_recovery(inner: Arc<dyn Remote>, recovery: &str) -> Result<Self> {
        Self::check(inner.as_ref())?;
        let r = parse_recovery(recovery).ok_or(Error::WrongPassword)?;
        let w = Self::wrapped(inner.as_ref(), KEY_RECOVERY)?;
        let key = open(&r, b"relicario-chave-recuperacao", &w.nonce, &w.key)
            .ok_or(Error::WrongPassword)?;
        Ok(Encrypted {
            inner,
            key: key.try_into().map_err(|_| Error::Corrupt("chave".into()))?,
        })
    }

    /// Troca a senha (a chave mestra e a de recuperação continuam as mesmas).
    pub fn set_password(&self, password: &str) -> Result<()> {
        let kdf = KdfParams::default();
        let salt: [u8; 16] = random();
        let pk = derive(password, &salt, kdf)?;
        let (nonce, wrapped) = seal(&pk, b"relicario-chave-senha", &self.key);
        let w = WrappedKey {
            kind: "argon2id".into(),
            kdf: Some(kdf),
            salt: salt.to_vec(),
            nonce,
            key: wrapped,
        };
        self.inner.put(
            Kind::Keys,
            KEY_PASSWORD,
            &serde_json::to_vec_pretty(&w).expect("json"),
        )?;
        Ok(())
    }

    /// O destino por baixo (para mostrar ao usuário).
    pub fn inner(&self) -> &Arc<dyn Remote> {
        &self.inner
    }
}

fn other(m: impl Into<String>) -> relicario_transport::Error {
    relicario_transport::Error::Other(m.into())
}

impl Remote for Encrypted {
    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> relicario_transport::Result<()> {
        match kind {
            Kind::Config | Kind::Keys => {
                Err(other("config e chaves não são gravados pela cópia cifrada"))
            }
            _ => {
                let (nonce, ct) = seal(&self.key, &aad(kind, name), data);
                let mut out = Vec::with_capacity(MAGIC.len() + nonce.len() + ct.len());
                out.extend_from_slice(MAGIC);
                out.extend_from_slice(&nonce);
                out.extend_from_slice(&ct);
                self.inner.put(kind, name, &out)
            }
        }
    }

    fn get(&self, kind: Kind, name: &str) -> relicario_transport::Result<Option<Vec<u8>>> {
        match kind {
            Kind::Keys => Ok(None),
            Kind::Config => {
                // Mostra o config como o de um repositório comum: os dados já chegam decifrados.
                let Some(b) = self.inner.get(kind, name)? else {
                    return Ok(None);
                };
                let mut c: RepoConfig =
                    serde_json::from_slice(&b).map_err(|e| other(format!("config: {e}")))?;
                c.encryption = "none".into();
                Ok(Some(serde_json::to_vec_pretty(&c).expect("json")))
            }
            _ => {
                let Some(b) = self.inner.get(kind, name)? else {
                    return Ok(None);
                };
                if b.len() < MAGIC.len() + 24 || &b[..8] != MAGIC {
                    return Err(other(format!("{}/{name} não está cifrado", kind.dir())));
                }
                open(&self.key, &aad(kind, name), &b[8..32], &b[32..])
                    .map(Some)
                    .ok_or_else(|| {
                        other(format!(
                            "{}/{name} foi alterado ou não pertence a este repositório",
                            kind.dir()
                        ))
                    })
            }
        }
    }

    fn list(&self, kind: Kind) -> relicario_transport::Result<Vec<String>> {
        match kind {
            Kind::Keys => Ok(vec![]),
            _ => self.inner.list(kind),
        }
    }

    fn delete(&self, kind: Kind, name: &str) -> relicario_transport::Result<()> {
        match kind {
            Kind::Config | Kind::Keys => {
                Err(other("config e chaves não são apagados pela cópia cifrada"))
            }
            _ => self.inner.delete(kind, name),
        }
    }

    fn describe(&self) -> String {
        format!("{} (cifrado)", self.inner.describe())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recuperacao_ida_e_volta() {
        for _ in 0..50 {
            let k: [u8; 32] = random();
            let s = format_recovery(&k);
            assert_eq!(parse_recovery(&s), Some(k), "{s}");
            assert_eq!(parse_recovery(&s.to_lowercase().replace('-', " ")), Some(k));
        }
        assert_eq!(parse_recovery("ABCD"), None);
    }
}
