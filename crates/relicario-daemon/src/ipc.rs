//! Protocolo entre CLI/interface e daemon: uma linha JSON de pedido, uma linha JSON de resposta,
//! por um socket local (soquete abstrato no Linux, pipe nomeado no Windows, arquivo em /tmp no macOS).

use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, Name, Stream, ToNsName, prelude::*,
};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum Request {
    Status,
    Stop,
    /// `relicario run` vai abrir o jogo: sincroniza antes e deixa o jogo por conta do `run`.
    Launching {
        game: String,
    },
    /// O jogo aberto pelo `relicario run` fechou.
    Exited {
        game: String,
        started_at: i64,
    },
    /// Sincroniza agora (um jogo ou todos que não estão abertos).
    Sync {
        game: Option<String>,
    },
    Rescan,
    /// Relê a configuração (depois de parear, por exemplo).
    Reload,
    /// Abre o pareamento com este código por 5 minutos.
    PairStart {
        code: String,
    },
    PairStatus,
    PairCancel,
    /// Replica agora com as máquinas pareadas.
    PeersSync,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Response {
    Ok {
        messages: Vec<String>,
    },
    Status {
        pid: u32,
        version: String,
        machine: String,
        running: Vec<String>,
        games: usize,
    },
    Error {
        message: String,
    },
}

/// Nome do socket: depende da pasta de dados, para que cada usuário (e cada teste) tenha o seu.
pub fn socket_name() -> String {
    if let Ok(n) = std::env::var("RELICARIO_SOCKET") {
        return n;
    }
    let dir = relicario_platform::data_dir();
    let h = blake3::hash(dir.to_string_lossy().as_bytes()).to_hex();
    format!("relicario-{}.sock", &h[..16])
}

fn name(s: &str) -> std::io::Result<Name<'static>> {
    s.to_string().to_ns_name::<GenericNamespaced>()
}

pub fn listen() -> std::io::Result<interprocess::local_socket::Listener> {
    ListenerOptions::new()
        .name(name(&socket_name())?)
        .create_sync()
}

/// Lê um pedido de uma conexão e responde com `handle`.
pub fn serve_one(conn: Stream, handle: impl FnOnce(Request) -> Response) -> std::io::Result<()> {
    let mut r = BufReader::new(conn);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let resp = match serde_json::from_str::<Request>(line.trim()) {
        Ok(req) => handle(req),
        Err(e) => Response::Error {
            message: format!("pedido inválido: {e}"),
        },
    };
    let mut out = serde_json::to_string(&resp).expect("json");
    out.push('\n');
    r.get_mut().write_all(out.as_bytes())?;
    r.get_mut().flush()
}

/// Manda um pedido ao daemon. `Err` se ele não está rodando.
pub fn request(req: &Request) -> std::io::Result<Response> {
    let conn = Stream::connect(name(&socket_name())?)?;
    let mut r = BufReader::new(conn);
    let mut out = serde_json::to_string(req).expect("json");
    out.push('\n');
    r.get_mut().write_all(out.as_bytes())?;
    r.get_mut().flush()?;
    let mut line = String::new();
    r.read_line(&mut line)?;
    serde_json::from_str(line.trim())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// O daemon está rodando?
pub fn is_up() -> bool {
    matches!(request(&Request::Status), Ok(Response::Status { .. }))
}
