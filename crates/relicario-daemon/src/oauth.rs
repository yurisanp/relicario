//! Login no Google Drive pelo navegador: OAuth 2.0 para apps instalados, com PKCE e retorno
//! para um endereço local (`http://127.0.0.1:<porta>`). Sem servidor do projeto: o token de
//! atualização fica só no chaveiro desta máquina.
//!
//! O client OAuth (tipo "app para computador") é do projeto; enquanto ele não existe, use
//! `RELICARIO_GDRIVE_CLIENT_ID` e `RELICARIO_GDRIVE_CLIENT_SECRET` (veja
//! `docs/pendencias-do-yuri.md`). Para apps instalados o "secret" do Google não é segredo.

use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Só os arquivos que o próprio Relicário criou.
pub const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

/// Client do projeto, embutido na compilação quando existir.
pub fn client() -> Option<(String, String)> {
    let id = std::env::var("RELICARIO_GDRIVE_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("RELICARIO_GDRIVE_CLIENT_ID").map(str::to_string))?;
    let secret = std::env::var("RELICARIO_GDRIVE_CLIENT_SECRET")
        .ok()
        .or_else(|| option_env!("RELICARIO_GDRIVE_CLIENT_SECRET").map(str::to_string))
        .unwrap_or_default();
    Some((id, secret))
}

pub fn b64url(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            s.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    s
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn dec(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn random_token() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("aleatório");
    b64url(&b)
}

/// Pedido de login pronto para abrir no navegador.
#[derive(Debug)]
pub struct Pending {
    listener: TcpListener,
    pub url: String,
    verifier: String,
    state: String,
    redirect: String,
}

pub fn start(client_id: &str) -> Result<Pending, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}");
    let verifier = random_token();
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let state = random_token();
    let url = format!(
        "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={challenge}&code_challenge_method=S256&state={state}&access_type=offline&prompt=consent",
        enc(client_id),
        enc(&redirect),
        enc(SCOPE)
    );
    Ok(Pending {
        listener,
        url,
        verifier,
        state,
        redirect,
    })
}

/// Abre o navegador padrão (sem garantia: a URL também é mostrada).
pub fn open_browser(url: &str) {
    let r = match relicario_platform::Os::atual() {
        relicario_platform::Os::Windows => std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn(),
        relicario_platform::Os::MacOs => std::process::Command::new("open").arg(url).spawn(),
        _ => std::process::Command::new("xdg-open").arg(url).spawn(),
    };
    let _ = r;
}

/// Espera o navegador voltar com o código.
pub fn wait_code(p: &Pending, timeout: Duration) -> Result<String, String> {
    p.listener
        .set_nonblocking(true)
        .map_err(|e| e.to_string())?;
    let end = Instant::now() + timeout;
    loop {
        match p.listener.accept() {
            Ok((mut s, _)) => {
                let _ = s.set_nonblocking(false);
                let mut line = String::new();
                BufReader::new(&s)
                    .read_line(&mut line)
                    .map_err(|e| e.to_string())?;
                let query = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|u| u.split_once('?'))
                    .map(|(_, q)| q.to_string())
                    .unwrap_or_default();
                let params: Vec<(String, String)> = query
                    .split('&')
                    .filter_map(|kv| kv.split_once('='))
                    .map(|(k, v)| (k.to_string(), dec(v)))
                    .collect();
                let get = |k: &str| params.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
                let (ok, msg) = match (get("code"), get("state"), get("error")) {
                    (_, _, Some(e)) => (
                        Err(format!("o Google recusou: {e}")),
                        "Login cancelado. Pode fechar esta aba.",
                    ),
                    (Some(c), Some(st), _) if st == p.state => (
                        Ok(c),
                        "Pronto! O Relicário já pode usar o seu Google Drive. Pode fechar esta aba.",
                    ),
                    (Some(_), _, _) => (
                        Err("resposta do login não confere (state)".into()),
                        "Algo deu errado. Tente de novo.",
                    ),
                    _ => continue,
                };
                let body = format!(
                    "<!doctype html><meta charset=utf-8><title>Relicário</title><p style=\"font:16px system-ui;margin:3em\">{msg}</p>"
                );
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                return ok;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > end {
                    return Err("o login não terminou a tempo".into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Troca o código pelo token de atualização.
pub fn exchange(
    p: &Pending,
    client_id: &str,
    client_secret: &str,
    code: &str,
) -> Result<String, String> {
    let form = [
        ("code", code),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("code_verifier", &p.verifier),
        ("redirect_uri", &p.redirect),
        ("grant_type", "authorization_code"),
    ];
    let mut resp = relicario_catalog::art::http_agent()
        .post(TOKEN_URL)
        .send_form(form)
        .map_err(|e| format!("token do Google: {e}"))?;
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("token do Google: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("token do Google: {e}"))?;
    v.get("refresh_token")
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("o Google não devolveu um token de atualização: {v}"))
}

/// Entra no Google Drive pelo navegador: devolve (client id, client secret, refresh token).
/// `show` recebe o endereço, para quem chamou mostrar caso o navegador não abra.
pub fn gdrive_login(show: impl FnOnce(&str)) -> Result<(String, String, String), String> {
    let (id, secret) = client().ok_or(
        "esta versão ainda não tem o client do Google Drive; defina RELICARIO_GDRIVE_CLIENT_ID e RELICARIO_GDRIVE_CLIENT_SECRET",
    )?;
    let p = start(&id)?;
    show(&p.url);
    open_browser(&p.url);
    let code = wait_code(&p, Duration::from_secs(300))?;
    let token = exchange(&p, &id, &secret, &code)?;
    Ok((id, secret, token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_e_pkce() {
        assert_eq!(b64url(b""), "");
        assert_eq!(b64url(b"f"), "Zg");
        assert_eq!(b64url(b"fo"), "Zm8");
        assert_eq!(b64url(b"foo"), "Zm9v");
        assert_eq!(b64url(&[0xfb, 0xff]), "-_8");
        // RFC 7636, apêndice B.
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            b64url(&Sha256::digest(v.as_bytes())),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn retorno_do_navegador() {
        let p = start("cliente").unwrap();
        assert!(p.url.contains("code_challenge_method=S256") && p.url.contains("drive.file"));
        let addr = p.listener.local_addr().unwrap();
        let state = p.state.clone();
        let t = std::thread::spawn(move || {
            let mut s = std::net::TcpStream::connect(addr).unwrap();
            write!(
                s,
                "GET /?state={state}&code=4%2F0Abc HTTP/1.1\r\nHost: x\r\n\r\n"
            )
            .unwrap();
            let mut r = String::new();
            let _ = std::io::Read::read_to_string(&mut s, &mut r);
            r
        });
        assert_eq!(wait_code(&p, Duration::from_secs(5)).unwrap(), "4/0Abc");
        assert!(t.join().unwrap().contains("Pronto"));
    }
}
