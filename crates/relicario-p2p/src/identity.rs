//! Certificado próprio de cada máquina.

use crate::{Error, Result};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::path::Path;

/// Impressão digital de um certificado: BLAKE3 do DER, em hex.
pub fn fingerprint(cert_der: &[u8]) -> String {
    blake3::hash(cert_der).to_hex().to_string()
}

/// Certificado e chave privada desta máquina.
pub struct Identity {
    pub(crate) cert: CertificateDer<'static>,
    key: Vec<u8>,
    pub fingerprint: String,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

impl Identity {
    pub fn generate() -> Result<Self> {
        let ck = rcgen::generate_simple_self_signed(vec!["relicario".to_string()])
            .map_err(|e| Error::Tls(e.to_string()))?;
        let cert = ck.cert.der().clone();
        Ok(Identity {
            fingerprint: fingerprint(&cert),
            cert,
            key: ck.key_pair.serialize_der(),
        })
    }

    /// Lê de `dir/identidade.der` e `dir/identidade.key`, criando na primeira vez.
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        let (c, k) = (dir.join("identidade.der"), dir.join("identidade.key"));
        if let (Ok(cert), Ok(key)) = (std::fs::read(&c), std::fs::read(&k)) {
            let cert = CertificateDer::from(cert);
            return Ok(Identity {
                fingerprint: fingerprint(&cert),
                cert,
                key,
            });
        }
        let id = Self::generate()?;
        std::fs::create_dir_all(dir)?;
        write_private(&k, &id.key)?;
        std::fs::write(&c, id.cert.as_ref())?;
        Ok(id)
    }

    pub(crate) fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key.clone()))
    }
}

#[cfg(unix)]
fn write_private(p: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(p)?;
    f.write_all(data)
}

#[cfg(not(unix))]
fn write_private(p: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(p, data)
}
