//! Leitor do formato VDF de texto da Valve (`libraryfolders.vdf`, `appmanifest_*.acf`,
//! `localconfig.vdf`).

use crate::error::{Error, Result};

/// Um valor VDF: texto ou bloco aninhado. A ordem e a grafia das chaves são preservadas, para
/// regravar arquivos da Steam sem estragá-los; a busca de chaves ignora maiúsculas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vdf {
    Str(String),
    Map(Vec<(String, Vdf)>),
}

impl Vdf {
    pub fn new_map() -> Self {
        Vdf::Map(Vec::new())
    }

    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            // A última ocorrência ganha, como na Steam.
            Vdf::Map(m) => m
                .iter()
                .rev()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Vdf> {
        match self {
            Vdf::Map(m) => m
                .iter_mut()
                .rev()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    /// Bloco filho, criado se não existir.
    pub fn child_mut(&mut self, key: &str) -> &mut Vdf {
        if self.get(key).is_none_or(|v| v.str().is_some()) {
            self.set(key, Vdf::new_map());
        }
        self.get_mut(key).expect("acabou de criar")
    }

    /// Define uma chave (substitui a existente, mantendo a posição).
    pub fn set(&mut self, key: &str, value: Vdf) {
        if let Vdf::Map(m) = self {
            if let Some(slot) = m
                .iter_mut()
                .rev()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
            {
                slot.1 = value;
            } else {
                m.push((key.to_string(), value));
            }
        }
    }

    pub fn remove(&mut self, key: &str) {
        if let Vdf::Map(m) = self {
            m.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
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
            Vdf::Map(m) => Some(m.iter().map(|(k, v)| (k, v))),
            Vdf::Str(_) => None,
        }
        .into_iter()
        .flatten()
    }

    /// Texto no formato da Steam (tabs, chaves entre aspas).
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        if let Vdf::Map(m) = self {
            for (k, v) in m {
                write_entry(&mut out, k, v, 0);
            }
        }
        out
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn write_entry(out: &mut String, k: &str, v: &Vdf, depth: usize) {
    let ind = "\t".repeat(depth);
    match v {
        Vdf::Str(s) => {
            out.push_str(&format!("{ind}{}\t\t{}\n", quote(k), quote(s)));
        }
        Vdf::Map(m) => {
            out.push_str(&format!("{ind}{}\n{ind}{{\n", quote(k)));
            for (ck, cv) in m {
                write_entry(out, ck, cv, depth + 1);
            }
            out.push_str(&format!("{ind}}}\n"));
        }
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

    fn map(&mut self, top: bool) -> Result<Vec<(String, Vdf)>> {
        let mut m = Vec::new();
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
                m.push((key, Vdf::Map(child)));
            } else {
                let v = self
                    .token()?
                    .ok_or_else(|| Error::Vdf(format!("chave {key:?} sem valor")))?;
                m.push((key, Vdf::Str(v)));
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
    fn edita_e_regrava_preservando_o_resto() {
        let t = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"10\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n\t\"Zeta\"\t\t\"c:\\\\x \\\"y\\\"\"\n}\n";
        let mut v = parse(t).unwrap();
        assert_eq!(v.to_text(), t, "ida e volta sem mudança");
        let apps = v
            .get_mut("userlocalconfigstore")
            .unwrap()
            .child_mut("Software")
            .child_mut("Valve")
            .child_mut("Steam")
            .child_mut("apps");
        apps.child_mut("10").set(
            "LaunchOptions",
            Vdf::Str("relicario run -- %command%".into()),
        );
        apps.child_mut("20")
            .set("LaunchOptions", Vdf::Str("x".into()));
        let again = parse(&v.to_text()).unwrap();
        let a = again
            .path(&["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"])
            .unwrap();
        assert_eq!(
            a.path(&["10", "LaunchOptions"]).unwrap().str(),
            Some("relicario run -- %command%")
        );
        assert_eq!(a.path(&["10", "LastPlayed"]).unwrap().str(), Some("1"));
        assert_eq!(a.path(&["20", "launchoptions"]).unwrap().str(), Some("x"));
        assert_eq!(
            again.path(&["UserLocalConfigStore", "Zeta"]).unwrap().str(),
            Some("c:\\x \"y\"")
        );
    }

    #[test]
    fn erros() {
        assert!(parse("\"a\" { \"b\" \"c\" ").is_err());
        assert!(parse("\"a").is_err());
    }
}
