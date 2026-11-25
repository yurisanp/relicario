//! Pareamento por código de 6 dígitos com SPAKE2.
//!
//! 1. Quem entra manda a mensagem SPAKE2 dele.
//! 2. Quem hospeda responde com a dele e um MAC sobre as duas impressões digitais.
//! 3. Quem entra confere o MAC (prova de que o outro sabe o código e é o certificado da conexão)
//!    e manda o próprio MAC; quem hospeda confere e grava o par.
//!
//! Errar o código encerra a sessão: não dá para tentar vários códigos.

use crate::client::{self, PeerClient};
use crate::identity::Identity;
use crate::protocol::{Req, Resp};
use crate::{Error, Peer, Result};
use spake2::{Ed25519Group, Identity as SpakeId, Password, Spake2};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

const CONTEXT: &[u8] = b"relicario-pareamento-v1";

/// Código aleatório de 6 dígitos.
pub fn new_code() -> String {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("aleatório");
    format!("{:06}", u32::from_le_bytes(b) % 1_000_000)
}

pub(crate) fn mac(key: &[u8], role: &str, mine: &str, theirs: &str) -> Vec<u8> {
    let k = blake3::derive_key("relicario pareamento mac v1", key);
    let mut h = blake3::Hasher::new_keyed(&k);
    h.update(role.as_bytes());
    h.update(b"\0");
    h.update(mine.as_bytes());
    h.update(b"\0");
    h.update(theirs.as_bytes());
    h.finalize().as_bytes().to_vec()
}

fn start(code: &str) -> (Spake2<Ed25519Group>, Vec<u8>) {
    Spake2::<Ed25519Group>::start_symmetric(
        &Password::new(code.trim().as_bytes()),
        &SpakeId::new(CONTEXT),
    )
}

/// Sessão aberta do lado de quem hospeda.
pub(crate) struct Session {
    code: String,
    until: Instant,
    pending: Option<(Vec<u8>, Peer)>,
}

impl Session {
    pub fn new(code: &str, until: Instant) -> Self {
        Session {
            code: code.to_string(),
            until,
            pending: None,
        }
    }

    pub fn open(&self) -> bool {
        Instant::now() < self.until
    }

    /// Passo 2. Devolve (mensagem, MAC).
    pub fn respond(
        &mut self,
        msg: &[u8],
        my_fp: &str,
        their_fp: &str,
        machine: String,
        name: String,
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        if self.pending.is_some() {
            return Err(Error::Refused("pareamento já em andamento".into()));
        }
        let (s, reply) = start(&self.code);
        let key = s.finish(msg).map_err(|_| Error::BadCode)?;
        let m = mac(&key, "anfitriao", my_fp, their_fp);
        self.pending = Some((
            key,
            Peer {
                machine,
                name,
                fingerprint: their_fp.to_string(),
                addresses: vec![],
            },
        ));
        Ok((reply, m))
    }

    /// Passo 3, do lado de quem hospeda.
    pub fn confirm(self, their_mac: &[u8], my_fp: &str, their_fp: &str) -> Result<Peer> {
        let Some((key, peer)) = self.pending else {
            return Err(Error::Protocol("confirmação sem início".into()));
        };
        if peer.fingerprint != their_fp || mac(&key, "convidado", their_fp, my_fp) != their_mac {
            return Err(Error::BadCode);
        }
        Ok(peer)
    }
}

/// Entra no pareamento aberto em `addrs` com o código. Devolve o par (com o endereço que
/// funcionou) para gravar na configuração.
pub fn join(
    id: &Identity,
    me: &str,
    my_name: &str,
    addrs: &[SocketAddr],
    code: &str,
    timeout: Duration,
) -> Result<Peer> {
    let c: PeerClient = client::connect(id, me, addrs, None, timeout)?;
    let (s, msg) = start(code);
    let (resp, _) = c.request(
        &Req::Pair {
            machine: me.into(),
            name: my_name.into(),
            msg,
        },
        &[],
    )?;
    let Resp::Pair {
        machine,
        name,
        msg,
        mac: their,
    } = resp
    else {
        return Err(Error::Protocol(format!("resposta inesperada {resp:?}")));
    };
    let key = s.finish(&msg).map_err(|_| Error::BadCode)?;
    if crate::server::pairing_mac(&key, "anfitriao", &c.fingerprint, &id.fingerprint) != their {
        // Avisa o outro lado para encerrar a sessão: o código não serve mais.
        let _ = c.request(&Req::PairConfirm { mac: vec![] }, &[]);
        c.close();
        return Err(Error::BadCode);
    }
    let mine = mac(&key, "convidado", &id.fingerprint, &c.fingerprint);
    c.request(&Req::PairConfirm { mac: mine }, &[])?;
    let peer = Peer {
        machine,
        name,
        fingerprint: c.fingerprint.clone(),
        addresses: vec![c.addr().to_string()],
    };
    c.close();
    Ok(peer)
}
