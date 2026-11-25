//! `relicario doctor --report`: um arquivo de texto para anexar a um pedido de ajuda. Junta o
//! diagnóstico, os últimos eventos do daemon e a atividade, e tira o que identifica a pessoa:
//! pasta pessoal, nome de usuário, nome das máquinas, todo texto da configuração (caminhos,
//! endereços de nuvem, usuários), e-mails e endereços IP. Nada é enviado a lugar nenhum.

use relicario_daemon::config::Config;
use relicario_store::time::format_utc;

pub fn build(doctor: &[String]) -> String {
    let a = Anon::new();
    let mut out = String::new();
    out.push_str("# Relatório do Relicário\n\n");
    out.push_str(&format!(
        "gerado em {} UTC\n\n## Diagnóstico\n\n",
        format_utc(relicario_store::time::now())
    ));
    for l in doctor {
        out.push_str(&a.clean(l));
        out.push('\n');
    }
    out.push_str("\n## Últimos eventos do daemon\n\n");
    let logs = relicario_daemon::logs::recent(300);
    if logs.is_empty() {
        out.push_str("(nenhum)\n");
    }
    for l in logs {
        out.push_str(&format!(
            "{} {:5} {}\n",
            format_utc(l.at),
            l.level,
            a.clean(&l.msg)
        ));
    }
    out.push_str("\n## Atividade recente\n\n");
    let act = relicario_daemon::activity::recent(100, None);
    if act.is_empty() {
        out.push_str("(nenhuma)\n");
    }
    for e in act.iter().rev() {
        out.push_str(&format!(
            "{} {} {}\n",
            format_utc(e.at),
            e.kind,
            a.clean(&e.text)
        ));
    }
    out
}

/// Troca o que é pessoal por marcadores.
pub struct Anon {
    /// (texto, marcador), do mais longo para o mais curto. Nomes (sem `/`, `\\`, `@` ou `:`)
    /// só são trocados como palavra inteira, para "PC" não estragar "PCGamingWiki".
    subs: Vec<(String, String)>,
}

impl Anon {
    pub fn new() -> Self {
        let mut subs: Vec<(String, String)> = Vec::new();
        let mut add = |s: &str, tag: &str| {
            let s = s.trim();
            if s.chars().count() >= 2 && !subs.iter().any(|(x, _)| x == s) {
                subs.push((s.to_string(), tag.to_string()));
            }
        };
        if let Ok(cfg) = Config::load() {
            add(&cfg.machine_name, "<esta máquina>");
            // Nomes das outras máquinas: as pareadas e as que aparecem no repositório.
            let mut others: Vec<String> = cfg.peers.iter().map(|p| p.name.clone()).collect();
            if let Ok(repo) = cfg.open_repo()
                && let Ok(heads) = repo.heads()
            {
                others.extend(heads.into_iter().map(|h| h.machine_name));
            }
            others.retain(|n| *n != cfg.machine_name);
            others.sort();
            others.dedup();
            for (i, name) in others.iter().enumerate() {
                add(name, &format!("<máquina {}>", i + 1));
            }
            if let Ok(v) = serde_json::to_value(&cfg) {
                let mut texts = Vec::new();
                strings(&v, false, &mut texts);
                for t in texts {
                    add(&t, "<oculto>");
                }
            }
        }
        if let Some(home) = relicario_platform::home_dir() {
            let h = home.to_string_lossy().to_string();
            add(&h, "~");
            add(&h.replace('\\', "/"), "~");
            if let Some(user) = home.file_name() {
                add(&user.to_string_lossy(), "<usuário>");
            }
        }
        for var in ["USER", "USERNAME", "LOGNAME"] {
            if let Ok(u) = std::env::var(var) {
                add(&u, "<usuário>");
            }
        }
        add(&relicario_platform::hostname(), "<máquina>");
        subs.sort_by_key(|(s, _)| std::cmp::Reverse(s.len()));
        Anon { subs }
    }

    #[cfg(test)]
    fn with(subs: &[(&str, &str)]) -> Self {
        Anon {
            subs: subs
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        }
    }

    pub fn clean(&self, s: &str) -> String {
        let mut s = s.to_string();
        for (from, to) in &self.subs {
            if s.contains(from.as_str()) {
                s = replace(&s, from, to);
            }
        }
        mask_emails(&mask_ipv4(&s))
    }
}

/// Textos da configuração que identificam a pessoa: caminhos, endereços, e-mails e os campos
/// de conta (usuário, chaves, bucket, nomes de máquina). Ids de jogo e nomes de opção ficam.
fn strings(v: &serde_json::Value, sensitive: bool, out: &mut Vec<String>) {
    match v {
        serde_json::Value::String(s) => {
            if sensitive || s.contains(['/', '\\', '@', ':']) {
                out.push(s.clone());
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| strings(x, sensitive, out)),
        serde_json::Value::Object(o) => {
            for (k, x) in o {
                let k = k.as_str();
                let s = sensitive
                    || matches!(
                        k,
                        "user"
                            | "access_key"
                            | "secret_key"
                            | "password"
                            | "bucket"
                            | "endpoint"
                            | "client_id"
                            | "client_secret"
                            | "refresh_token"
                            | "steamgriddb_key"
                            | "machine_name"
                            | "addrs"
                            | "addresses"
                            | "fingerprint"
                    );
                strings(x, s, out);
            }
        }
        _ => {}
    }
}

/// Troca `from` por `to`; se `from` é um nome, só onde ele é palavra inteira.
fn replace(s: &str, from: &str, to: &str) -> String {
    if from.contains(['/', '\\', '@', ':']) {
        return s.replace(from, to);
    }
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(from) {
        let before = rest[..i].chars().next_back();
        let after = rest[i + from.len()..].chars().next();
        out.push_str(&rest[..i]);
        if word(before) || word(after) {
            out.push_str(from);
        } else {
            out.push_str(to);
        }
        rest = &rest[i + from.len()..];
    }
    out.push_str(rest);
    out
}

fn mask_ipv4(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit()
            && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'.'))
        {
            // Tenta ler quatro grupos de 1 a 3 dígitos separados por ponto.
            let mut j = i;
            let mut groups = 0;
            loop {
                let start = j;
                while j < b.len() && b[j].is_ascii_digit() && j - start < 4 {
                    j += 1;
                }
                let len = j - start;
                if len == 0 || len > 3 {
                    break;
                }
                groups += 1;
                if groups == 4 || j >= b.len() || b[j] != b'.' {
                    break;
                }
                j += 1;
            }
            let ends_clean = j >= b.len()
                || !(b[j].is_ascii_digit()
                    || b[j] == b'.' && j + 1 < b.len() && b[j + 1].is_ascii_digit());
            if groups == 4 && ends_clean {
                out.push_str("<ip>");
                i = j;
                continue;
            }
        }
        let ch = s[i..].chars().next().expect("char");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn mask_emails(s: &str) -> String {
    s.split_inclusive(|c: char| {
        c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '<' | '>' | '"' | '\'')
    })
    .map(|w| {
        let (word, tail) = match w.char_indices().last() {
            Some((i, c)) if !(c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | '@' | '+')) => {
                w.split_at(i)
            }
            _ => (w, ""),
        };
        match word.split_once('@') {
            Some((user, dom)) if !user.is_empty() && dom.contains('.') && !dom.starts_with('.') => {
                format!("<email>{tail}")
            }
            _ => w.to_string(),
        }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tira_o_que_e_pessoal() {
        let a = Anon::with(&[
            ("/home/maria/Jogos", "<oculto>"),
            ("/home/maria", "~"),
            ("maria", "<usuário>"),
            ("Notebook da Maria", "<máquina 1>"),
            ("PC", "<esta máquina>"),
        ]);
        let t = a.clean(
            "pareado com Notebook da Maria em 192.168.0.12:47321; save em /home/maria/.local/x \
             e /home/maria/Jogos/y; conta maria.silva@exemplo.com.br (versão 1.2.3.45678)",
        );
        assert!(!t.contains("maria"), "{t}");
        assert!(!t.contains("192.168"), "{t}");
        assert!(t.contains("<máquina 1> em <ip>:47321"), "{t}");
        assert!(t.contains("~/.local/x"), "{t}");
        assert!(t.contains("<oculto>/y"), "{t}");
        assert!(t.contains("<email>"), "{t}");
        assert!(t.contains("1.2.3.45678"), "versão não é IP: {t}");
        let t = a.clean("PC baixou o manifesto do PCGamingWiki; pasta mariana");
        assert_eq!(
            t,
            "<esta máquina> baixou o manifesto do PCGamingWiki; pasta mariana"
        );
    }
}
