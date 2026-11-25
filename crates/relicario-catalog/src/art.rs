//! Arte dos jogos: capa, banner, ícone e logo.
//!
//! Ordem das fontes (roadmap, fase 2):
//! 1. SteamGridDB, com a chave de API do usuário, guardando o crédito do autor;
//! 2. cache local das lojas (Steam `appcache/librarycache`, Lutris);
//! 3. CDN pública da Steam pelo appid;
//! 4. capa gerada com o nome e uma cor do jogo.
//!
//! As imagens ficam só no cache local desta máquina e nunca são sincronizadas.

use crate::error::{Error, Result, io};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Tipo de arte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtKind {
    /// Capa vertical (600x900).
    Capa,
    /// Banner horizontal largo (hero).
    Banner,
    Icone,
    Logo,
}

impl ArtKind {
    pub const ALL: [ArtKind; 4] = [
        ArtKind::Capa,
        ArtKind::Banner,
        ArtKind::Icone,
        ArtKind::Logo,
    ];

    fn file(self) -> &'static str {
        match self {
            ArtKind::Capa => "capa",
            ArtKind::Banner => "banner",
            ArtKind::Icone => "icone",
            ArtKind::Logo => "logo",
        }
    }

    fn sgdb_path(self) -> &'static str {
        match self {
            ArtKind::Capa => "grids",
            ArtKind::Banner => "heroes",
            ArtKind::Icone => "icons",
            ArtKind::Logo => "logos",
        }
    }

    fn steam_cdn(self) -> &'static str {
        match self {
            ArtKind::Capa => "library_600x900_2x.jpg",
            ArtKind::Banner => "library_hero.jpg",
            ArtKind::Icone => "capsule_231x87.jpg",
            ArtKind::Logo => "logo.png",
        }
    }

    fn steam_local(self) -> &'static [&'static str] {
        match self {
            ArtKind::Capa => &["library_600x900.jpg", "library_600x900_2x.jpg"],
            ArtKind::Banner => &["library_hero.jpg"],
            ArtKind::Icone => &["icon.jpg", "logo.png"],
            ArtKind::Logo => &["logo.png"],
        }
    }
}

/// De onde veio uma imagem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtInfo {
    pub kind: ArtKind,
    /// `steamgriddb`, `steam-cache`, `lutris-cache`, `steam-cdn`, `gerada` ou `usuario`.
    pub source: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Crédito do autor (SteamGridDB), para mostrar na interface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

/// Uma opção de arte do SteamGridDB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtChoice {
    pub url: String,
    /// Versão pequena para a grade do seletor.
    pub thumb: String,
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

/// O que se sabe do jogo para procurar arte.
#[derive(Debug, Clone, Default)]
pub struct ArtQuery {
    pub id: String,
    pub title: String,
    pub steam_appid: Option<u64>,
    pub gog_id: Option<u64>,
    pub lutris_slug: Option<String>,
}

/// Resolve e guarda arte em cache.
#[derive(Debug, Clone)]
pub struct ArtCache {
    pub dir: PathBuf,
    pub steam_roots: Vec<PathBuf>,
    pub lutris_dirs: Vec<PathBuf>,
    pub steamgriddb_key: Option<String>,
    /// Permitir rede (SteamGridDB e CDN). Desligado em testes.
    pub online: bool,
}

impl ArtCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        ArtCache {
            dir: dir.into(),
            steam_roots: vec![],
            lutris_dirs: vec![],
            steamgriddb_key: None,
            online: true,
        }
    }

    pub fn default_dir() -> PathBuf {
        relicario_platform::cache_dir().join("art")
    }

    fn game_dir(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    fn info_path(&self, id: &str, kind: ArtKind) -> PathBuf {
        self.game_dir(id).join(format!("{}.json", kind.file()))
    }

    /// Arte já em cache.
    pub fn cached(&self, id: &str, kind: ArtKind) -> Option<ArtInfo> {
        let info: ArtInfo =
            serde_json::from_slice(&std::fs::read(self.info_path(id, kind)).ok()?).ok()?;
        info.path.is_file().then_some(info)
    }

    /// Arte do jogo: cache, ou a primeira fonte que responder.
    pub fn get(&self, q: &ArtQuery, kind: ArtKind) -> Result<ArtInfo> {
        if let Some(i) = self.cached(&q.id, kind) {
            return Ok(i);
        }
        let found = self
            .find_local(q, kind)
            .or_else(|| {
                if self.online {
                    self.find_sgdb(q, kind).ok().flatten()
                } else {
                    None
                }
            })
            .or_else(|| {
                if self.online {
                    self.find_steam_cdn(q, kind).ok().flatten()
                } else {
                    None
                }
            });
        let info = match found {
            Some(i) => i,
            None => self.generated(q, kind)?,
        };
        self.save_info(&q.id, &info)?;
        Ok(info)
    }

    /// Troca a arte por um arquivo escolhido pelo usuário.
    pub fn set_override(&self, id: &str, kind: ArtKind, file: &Path) -> Result<ArtInfo> {
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("png");
        let dest = self.game_dir(id).join(format!("{}.{ext}", kind.file()));
        std::fs::create_dir_all(self.game_dir(id)).map_err(io(&self.dir))?;
        std::fs::copy(file, &dest).map_err(io(file))?;
        let info = ArtInfo {
            kind,
            source: "usuario".into(),
            path: dest,
            url: None,
            author: None,
        };
        self.save_info(id, &info)?;
        Ok(info)
    }

    /// Esquece a arte em cache (para buscar de novo).
    pub fn clear(&self, id: &str, kind: ArtKind) {
        let _ = std::fs::remove_file(self.info_path(id, kind));
    }

    fn save_info(&self, id: &str, info: &ArtInfo) -> Result<()> {
        std::fs::create_dir_all(self.game_dir(id)).map_err(io(&self.dir))?;
        let p = self.info_path(id, kind_of(info));
        std::fs::write(&p, serde_json::to_vec_pretty(info).expect("json")).map_err(io(&p))
    }

    fn find_local(&self, q: &ArtQuery, kind: ArtKind) -> Option<ArtInfo> {
        if let Some(appid) = q.steam_appid {
            for root in &self.steam_roots {
                let cache = root.join("appcache/librarycache");
                for name in kind.steam_local() {
                    for p in [
                        cache.join(format!("{appid}_{name}")),
                        cache.join(appid.to_string()).join(name),
                    ] {
                        if p.is_file() {
                            return Some(ArtInfo {
                                kind,
                                source: "steam-cache".into(),
                                path: p,
                                url: None,
                                author: None,
                            });
                        }
                    }
                }
            }
        }
        if let Some(slug) = &q.lutris_slug {
            let sub = match kind {
                ArtKind::Capa => "coverart",
                ArtKind::Banner => "banners",
                _ => return None,
            };
            for d in &self.lutris_dirs {
                for ext in ["jpg", "png"] {
                    let p = d.join(sub).join(format!("{slug}.{ext}"));
                    if p.is_file() {
                        return Some(ArtInfo {
                            kind,
                            source: "lutris-cache".into(),
                            path: p,
                            url: None,
                            author: None,
                        });
                    }
                }
            }
        }
        None
    }

    /// Opções do SteamGridDB para o seletor de arte (até `limit`), com o crédito de cada autor.
    #[cfg(feature = "net")]
    pub fn choices(&self, q: &ArtQuery, kind: ArtKind, limit: usize) -> Result<Vec<ArtChoice>> {
        let Some(key) = &self.steamgriddb_key else {
            return Ok(vec![]);
        };
        if !self.online {
            return Ok(vec![]);
        }
        let agent = http_agent();
        let base = "https://www.steamgriddb.com/api/v2";
        let dims = if kind == ArtKind::Capa {
            "?dimensions=600x900"
        } else {
            ""
        };
        let url = if let Some(appid) = q.steam_appid {
            format!("{base}/{}/steam/{appid}{dims}", kind.sgdb_path())
        } else if let Some(gog) = q.gog_id {
            format!("{base}/{}/gog/{gog}{dims}", kind.sgdb_path())
        } else {
            let search = format!("{base}/search/autocomplete/{}", urlencode(&q.title));
            let v: serde_json::Value = get_json(&agent, &search, key)?;
            let Some(id) = v
                .get("data")
                .and_then(|d| d.get(0))
                .and_then(|g| g.get("id"))
                .and_then(|i| i.as_u64())
            else {
                return Ok(vec![]);
            };
            format!("{base}/{}/game/{id}{dims}", kind.sgdb_path())
        };
        let v = get_json(&agent, &url, key)?;
        let list = v
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(list
            .iter()
            .filter_map(|it| {
                let url = it.get("url")?.as_str()?.to_string();
                Some(ArtChoice {
                    thumb: it
                        .get("thumb")
                        .and_then(|t| t.as_str())
                        .unwrap_or(&url)
                        .to_string(),
                    url,
                    author: it
                        .get("author")
                        .and_then(|a| a.get("name"))
                        .and_then(|n| n.as_str())
                        .map(str::to_string),
                    style: it.get("style").and_then(|s| s.as_str()).map(str::to_string),
                })
            })
            .take(limit)
            .collect())
    }

    #[cfg(not(feature = "net"))]
    pub fn choices(&self, _q: &ArtQuery, _kind: ArtKind, _limit: usize) -> Result<Vec<ArtChoice>> {
        Ok(vec![])
    }

    /// Troca a arte por uma opção do SteamGridDB.
    #[cfg(feature = "net")]
    pub fn set_choice(&self, id: &str, kind: ArtKind, c: &ArtChoice) -> Result<ArtInfo> {
        let path = self.download(&http_agent(), id, kind, &c.url)?;
        let info = ArtInfo {
            kind,
            source: "steamgriddb".into(),
            path,
            url: Some(c.url.clone()),
            author: c.author.clone(),
        };
        self.save_info(id, &info)?;
        Ok(info)
    }

    #[cfg(not(feature = "net"))]
    pub fn set_choice(&self, _id: &str, _kind: ArtKind, _c: &ArtChoice) -> Result<ArtInfo> {
        Err(Error::Net("sem rede nesta versão".into()))
    }

    #[cfg(feature = "net")]
    fn find_sgdb(&self, q: &ArtQuery, kind: ArtKind) -> Result<Option<ArtInfo>> {
        let Some(first) = self.choices(q, kind, 1)?.into_iter().next() else {
            return Ok(None);
        };
        let path = self.download(&http_agent(), &q.id, kind, &first.url)?;
        Ok(Some(ArtInfo {
            kind,
            source: "steamgriddb".into(),
            path,
            url: Some(first.url),
            author: first.author,
        }))
    }

    #[cfg(not(feature = "net"))]
    fn find_sgdb(&self, _q: &ArtQuery, _kind: ArtKind) -> Result<Option<ArtInfo>> {
        Ok(None)
    }

    #[cfg(feature = "net")]
    fn find_steam_cdn(&self, q: &ArtQuery, kind: ArtKind) -> Result<Option<ArtInfo>> {
        let Some(appid) = q.steam_appid else {
            return Ok(None);
        };
        let url = steam_cdn_url(appid, kind);
        let agent = http_agent();
        match self.download(&agent, &q.id, kind, &url) {
            Ok(path) => Ok(Some(ArtInfo {
                kind,
                source: "steam-cdn".into(),
                path,
                url: Some(url),
                author: None,
            })),
            Err(_) => Ok(None),
        }
    }

    #[cfg(not(feature = "net"))]
    fn find_steam_cdn(&self, _q: &ArtQuery, _kind: ArtKind) -> Result<Option<ArtInfo>> {
        Ok(None)
    }

    #[cfg(feature = "net")]
    fn download(&self, agent: &ureq::Agent, id: &str, kind: ArtKind, url: &str) -> Result<PathBuf> {
        let mut resp = agent
            .get(url)
            .call()
            .map_err(|e| Error::Net(e.to_string()))?;
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(20 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| Error::Net(e.to_string()))?;
        let ext = if url.ends_with(".png") {
            "png"
        } else if url.ends_with(".webp") {
            "webp"
        } else {
            "jpg"
        };
        std::fs::create_dir_all(self.game_dir(id)).map_err(io(&self.dir))?;
        let p = self.game_dir(id).join(format!("{}.{ext}", kind.file()));
        std::fs::write(&p, bytes).map_err(io(&p))?;
        Ok(p)
    }

    /// Capa gerada em SVG com o nome e uma cor tirada do id.
    fn generated(&self, q: &ArtQuery, kind: ArtKind) -> Result<ArtInfo> {
        let svg = generated_svg(&q.title, &q.id, kind);
        std::fs::create_dir_all(self.game_dir(&q.id)).map_err(io(&self.dir))?;
        let p = self.game_dir(&q.id).join(format!("{}.svg", kind.file()));
        std::fs::write(&p, svg).map_err(io(&p))?;
        Ok(ArtInfo {
            kind,
            source: "gerada".into(),
            path: p,
            url: None,
            author: None,
        })
    }
}

fn kind_of(i: &ArtInfo) -> ArtKind {
    i.kind
}

/// URL da arte na CDN pública da Steam.
pub fn steam_cdn_url(appid: u64, kind: ArtKind) -> String {
    format!(
        "https://cdn.cloudflare.steamstatic.com/steam/apps/{appid}/{}",
        kind.steam_cdn()
    )
}

/// Cor (HSL) estável a partir do id.
pub fn color_for(id: &str) -> (u16, u8, u8) {
    let h = blake3::hash(id.as_bytes());
    let b = h.as_bytes();
    (
        (u16::from(b[0]) << 8 | u16::from(b[1])) % 360,
        45 + b[2] % 20,
        28 + b[3] % 12,
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn generated_svg(title: &str, id: &str, kind: ArtKind) -> String {
    let (h, s, l) = color_for(id);
    let (w, hgt) = match kind {
        ArtKind::Capa => (600, 900),
        ArtKind::Banner => (1920, 620),
        ArtKind::Icone => (256, 256),
        ArtKind::Logo => (800, 310),
    };
    let text = if kind == ArtKind::Icone {
        title
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
    } else {
        title.to_string()
    };
    let size = match kind {
        ArtKind::Icone => 140,
        ArtKind::Capa => 56,
        _ => 72,
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{hgt}" viewBox="0 0 {w} {hgt}"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="hsl({h},{s}%,{l2}%)"/><stop offset="1" stop-color="hsl({h},{s}%,{l}%)"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><text x="50%" y="50%" fill="#fff" font-family="sans-serif" font-weight="700" font-size="{size}" text-anchor="middle" dominant-baseline="middle">{t}</text></svg>"##,
        l2 = l + 14,
        t = xml_escape(&text)
    )
}

#[cfg(feature = "net")]
fn get_json(agent: &ureq::Agent, url: &str, key: &str) -> Result<serde_json::Value> {
    let mut r = agent
        .get(url)
        .header("Authorization", &format!("Bearer {key}"))
        .call()
        .map_err(|e| Error::Net(e.to_string()))?;
    let text = r
        .body_mut()
        .read_to_string()
        .map_err(|e| Error::Net(e.to_string()))?;
    serde_json::from_str(&text).map_err(|e| Error::Net(e.to_string()))
}

#[cfg(feature = "net")]
fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Cliente HTTP com tempo limite e identificação genérica.
#[cfg(feature = "net")]
pub fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .user_agent(concat!("Relicario/", env!("CARGO_PKG_VERSION")))
        .http_status_as_error(false)
        .build()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_local_da_steam_e_capa_gerada() {
        let t = tempfile::tempdir().unwrap();
        let steam = t.path().join("Steam");
        std::fs::create_dir_all(steam.join("appcache/librarycache/1086940")).unwrap();
        std::fs::write(
            steam.join("appcache/librarycache/1086940/library_600x900.jpg"),
            b"jpg",
        )
        .unwrap();
        std::fs::write(
            steam.join("appcache/librarycache/1031570_library_hero.jpg"),
            b"jpg",
        )
        .unwrap();
        let mut c = ArtCache::new(t.path().join("art"));
        c.steam_roots = vec![steam];
        c.online = false;
        let bg3 = ArtQuery {
            id: "baldurs-gate-3".into(),
            title: "Baldur's Gate 3".into(),
            steam_appid: Some(1086940),
            ..Default::default()
        };
        let i = c.get(&bg3, ArtKind::Capa).unwrap();
        assert_eq!(i.source, "steam-cache");
        let hs = ArtQuery {
            id: "hookshot".into(),
            title: "Hookshot".into(),
            steam_appid: Some(1031570),
            ..Default::default()
        };
        assert_eq!(c.get(&hs, ArtKind::Banner).unwrap().source, "steam-cache");
        let g = c.get(&hs, ArtKind::Capa).unwrap();
        assert_eq!(g.source, "gerada");
        let svg = std::fs::read_to_string(&g.path).unwrap();
        assert!(svg.contains("Hookshot") && svg.starts_with("<svg"));
        // Fica em cache.
        assert_eq!(c.cached("hookshot", ArtKind::Capa).unwrap(), g);
        // O usuário troca.
        let mine = t.path().join("minha.png");
        std::fs::write(&mine, b"png").unwrap();
        let o = c.set_override("hookshot", ArtKind::Capa, &mine).unwrap();
        assert_eq!(c.get(&hs, ArtKind::Capa).unwrap(), o);
        assert_eq!(
            steam_cdn_url(10, ArtKind::Logo),
            "https://cdn.cloudflare.steamstatic.com/steam/apps/10/logo.png"
        );
        assert!(generated_svg("A & <B>", "x", ArtKind::Logo).contains("A &amp; &lt;B&gt;"));
    }
}
