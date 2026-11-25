//! Leitor do formato VDF de texto da Valve (`libraryfolders.vdf`, `appmanifest_*.acf`,
//! `localconfig.vdf`).

use crate::error::{Error, Result};
use std::collections::BTreeMap;

/// Um valor VDF: texto ou bloco aninhado. Chaves repetidas ficam com o último valor; a
/// comparação de chaves ignora maiúsculas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vdf {
    Str(String),
    Map(BTreeMap<String, Vdf>),
}

impl Vdf {
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Map(m) => m.get(&key.to_ascii_lowercase()),
            Vdf::Str(_) => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Map(_) => None,
        }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Vdf::str)
    }

    /// Caminho de chaves, por exemplo `["UserLocalConfigStore", "Software", "Valve"]`.
    pub fn path(&self, keys: &[&str]) -> Option<&Vdf> {
        keys.iter().try_fold(self, |v, k| v.get(k))
    }

    pub fn entries(&self) -> impl Iterator<Item = (&String, &Vdf)> {
        match self {
            Vdf::Map(m) => Some(m.iter()),
            Vdf::Str(_) => None,
        }
        .into_iter()
        .flatten()
    }
}

/// Lê um documento VDF. A raiz é um mapa com a(s) chave(s) do topo.
pub fn parse(text: &str) -> Result<Vdf> {
    let mut p = Parser {
        chars: text.chars().collect(),
        pos: 0,
    };
    let m = p.map(true)?;
    Ok(Vdf::Map(m))
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn skip_ws(&mut self) {
        loop {
            while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
                self.pos += 1;
            }
            if self.pos + 1 < self.chars.len()
                && self.chars[self.pos] == '/'
                && self.chars[self.pos + 1] == '/'
            {
                while self.pos < self.chars.len() && self.chars[self.pos] != '\n' {
                    self.pos += 1;
                }
                continue;
            }
            // Condicionais como [$WIN32] são ignoradas.
            if self.pos < self.chars.len() && self.chars[self.pos] == '[' {
                while self.pos < self.chars.len() && self.chars[self.pos] != ']' {
                    self.pos += 1;
                }
                self.pos += 1;
                continue;
            }
            break;
        }
    }

    fn token(&mut self) -> Result<Option<String>> {
        self.skip_ws();
        if self.pos >= self.chars.len() {
            return Ok(None);
        }
        let c = self.chars[self.pos];
        if c == '{' || c == '}' {
            return Ok(None);
        }
        let mut s = String::new();
        if c == '"' {
            self.pos += 1;
            while self.pos < self.chars.len() {
                let c = self.chars[self.pos];
                self.pos += 1;
                match c {
                    '"' => return Ok(Some(s)),
                    '\\' if self.pos < self.chars.len() => {
                        let n = self.chars[self.pos];
                        self.pos += 1;
                        s.push(match n {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                    }
                    _ => s.push(c),
                }
            }
            Err(Error::Vdf("texto entre aspas sem fim".into()))
        } else {
            while self.pos < self.chars.len() {
                let c = self.chars[self.pos];
                if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                    break;
                }
                s.push(c);
                self.pos += 1;
            }
            Ok(Some(s))
        }
    }

    fn map(&mut self, top: bool) -> Result<BTreeMap<String, Vdf>> {
        let mut m = BTreeMap::new();
        loop {
            self.skip_ws();
            if self.pos >= self.chars.len() {
                if top {
                    return Ok(m);
                }
                return Err(Error::Vdf("bloco sem fechar".into()));
            }
            if self.chars[self.pos] == '}' {
                if top {
                    return Err(Error::Vdf("} sobrando".into()));
                }
                self.pos += 1;
                return Ok(m);
            }
            let Some(key) = self.token()? else {
                return Err(Error::Vdf(format!(
                    "esperava chave na posição {}",
                    self.pos
                )));
            };
            self.skip_ws();
            if self.pos < self.chars.len() && self.chars[self.pos] == '{' {
                self.pos += 1;
                let child = self.map(false)?;
                m.insert(key.to_ascii_lowercase(), Vdf::Map(child));
            } else {
                let v = self
                    .token()?
                    .ok_or_else(|| Error::Vdf(format!("chave {key:?} sem valor")))?;
                m.insert(key.to_ascii_lowercase(), Vdf::Str(v));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libraryfolders() {
        let t = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"/home/yuri/.local/share/Steam"
		"apps"
		{
			"1031570"		"123"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}"#;
        let v = parse(t).unwrap();
        let lf = v.get("libraryfolders").unwrap();
        assert_eq!(
            lf.path(&["0", "path"]).unwrap().str(),
            Some("/home/yuri/.local/share/Steam")
        );
        assert_eq!(
            lf.path(&["1", "path"]).unwrap().str(),
            Some("D:\\SteamLibrary")
        );
        assert_eq!(
            lf.path(&["0", "apps", "1031570"]).unwrap().str(),
            Some("123")
        );
        assert_eq!(lf.entries().count(), 2);
    }

    #[test]
    fn erros() {
        assert!(parse("\"a\" { \"b\" \"c\" ").is_err());
        assert!(parse("\"a").is_err());
    }
}
