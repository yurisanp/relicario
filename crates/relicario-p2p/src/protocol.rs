//! Mensagens: um pedido por stream bidirecional. Cada lado escreve `[u32 tamanho][JSON]` e, se
//! houver, o corpo até o fim do stream.

use crate::{Error, Result};
use relicario_transport::Kind;
use serde::{Deserialize, Serialize};

pub const MAX_HEADER: usize = 1 << 20;
pub const MAX_BODY: usize = 256 << 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum Req {
    Hello {
        machine: String,
        name: String,
    },
    List {
        kind: String,
    },
    Get {
        kind: String,
        name: String,
    },
    /// Corpo: o arquivo.
    Put {
        kind: String,
        name: String,
    },
    /// Pareamento, passo 1: mensagem SPAKE2 de quem entra.
    Pair {
        machine: String,
        name: String,
        msg: Vec<u8>,
    },
    /// Pareamento, passo 2: confirmação de quem entra.
    PairConfirm {
        mac: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Resp {
    Hello {
        machine: String,
        name: String,
        trusted: bool,
    },
    Names {
        names: Vec<String>,
    },
    /// Corpo: o arquivo.
    Data,
    NotFound,
    Ok,
    Pair {
        machine: String,
        name: String,
        msg: Vec<u8>,
        mac: Vec<u8>,
    },
    Error {
        message: String,
    },
}

pub fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Config => "config",
        other => other.dir(),
    }
}

pub fn kind_from(s: &str) -> Result<Kind> {
    Kind::ALL
        .into_iter()
        .find(|k| kind_name(*k) == s)
        .ok_or_else(|| Error::Protocol(format!("tipo desconhecido {s:?}")))
}

fn conn_err(e: impl std::fmt::Display) -> Error {
    Error::Connection(e.to_string())
}

pub async fn write<T: Serialize>(
    send: &mut quinn::SendStream,
    header: &T,
    body: &[u8],
) -> Result<()> {
    let h = serde_json::to_vec(header).map_err(|e| Error::Protocol(e.to_string()))?;
    send.write_all(&(h.len() as u32).to_be_bytes())
        .await
        .map_err(conn_err)?;
    send.write_all(&h).await.map_err(conn_err)?;
    if !body.is_empty() {
        send.write_all(body).await.map_err(conn_err)?;
    }
    send.finish().map_err(conn_err)?;
    Ok(())
}

pub async fn read<T: for<'de> Deserialize<'de>>(
    recv: &mut quinn::RecvStream,
) -> Result<(T, Vec<u8>)> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await.map_err(conn_err)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_HEADER {
        return Err(Error::Protocol("cabeçalho grande demais".into()));
    }
    let mut h = vec![0u8; len];
    recv.read_exact(&mut h).await.map_err(conn_err)?;
    let header = serde_json::from_slice(&h).map_err(|e| Error::Protocol(e.to_string()))?;
    let body = recv.read_to_end(MAX_BODY).await.map_err(conn_err)?;
    Ok((header, body))
}
