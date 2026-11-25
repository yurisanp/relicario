//! Manifesto do Ludusavi (<https://github.com/mtkennerly/ludusavi-manifest>, MIT), gerado a
//! partir do PCGamingWiki. Formato conferido com `data/schema.yaml` em novembro de 2026.

use crate::error::{Error, Result, io};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Endereço oficial do manifesto.
pub const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/mtkennerly/ludusavi-manifest/master/data/manifest.yaml";

/// Intervalo entre conferências de atualização.
pub const UPDATE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);

/// Restrição de um caminho ou executável.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bit: Option<u8>,
}

/// Um caminho de save ou chave de registro.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRule {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<Constraint>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchRule {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<Constraint>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreId {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraIds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flatpak: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gog_extra: Vec<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lutris: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steam_extra: Vec<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cloud {
    #[serde(default)]
    pub epic: bool,
    #[serde(default)]
    pub gog: bool,
    #[serde(default)]
    pub origin: bool,
    #[serde(default)]
    pub steam: bool,
    #[serde(default)]
    pub uplay: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Empty {}

/// Uma entrada do manifesto.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameEntry {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub files: BTreeMap<String, FileRule>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub install_dir: BTreeMap<String, Empty>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub launch: BTreeMap<String, Vec<LaunchRule>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub registry: BTreeMap<String, FileRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam: Option<StoreId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gog: Option<StoreId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<ExtraIds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud: Option<Cloud>,
}

impl GameEntry {
    pub fn steam_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.steam
            .as_ref()
            .and_then(|s| s.id)
            .into_iter()
            .chain(self.id.iter().flat_map(|i| i.steam_extra.iter().copied()))
    }

    pub fn gog_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.gog
            .as_ref()
            .and_then(|s| s.id)
            .into_iter()
            .chain(self.id.iter().flat_map(|i| i.gog_extra.iter().copied()))
    }

    pub fn steam_cloud(&self) -> bool {
        self.cloud.as_ref().is_some_and(|c| c.steam)
    }
}

/// O manifesto inteiro, com índices de busca.
#[derive(Debug, Clone, Default)]
pub struct Manifest {
    pub games: BTreeMap<String, GameEntry>,
    by_steam: HashMap<u64, String>,
    by_gog: HashMap<u64, String>,
    by_lutris: HashMap<String, String>,
    by_name: HashMap<String, String>,
}

impl Manifest {
    pub fn from_games(games: BTreeMap<String, GameEntry>) -> Self {
        let mut m = Manifest {
            games,
            ..Default::default()
        };
        for (title, e) in &m.games {
            if e.alias.is_some() {
                continue;
            }
            for id in e.steam_ids() {
                m.by_steam.entry(id).or_insert_with(|| title.clone());
            }
            for id in e.gog_ids() {
                m.by_gog.entry(id).or_insert_with(|| title.clone());
            }
            if let Some(l) = e.id.as_ref().and_then(|i| i.lutris.clone()) {
                m.by_lutris.entry(l).or_insert_with(|| title.clone());
            }
            m.by_name
                .entry(normalize(title))
                .or_insert_with(|| title.clone());
        }
        for (title, e) in &m.games {
            if let Some(a) = &e.alias
                && m.games.contains_key(a)
            {
                m.by_name
                    .entry(normalize(title))
                    .or_insert_with(|| a.clone());
            }
        }
        m
    }

    pub fn from_yaml(text: &str) -> Result<Self> {
        let games: BTreeMap<String, GameEntry> =
            serde_yaml_ng::from_str(text).map_err(|e| Error::Manifest(e.to_string()))?;
        Ok(Self::from_games(games))
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let games: BTreeMap<String, GameEntry> =
            serde_json::from_slice(bytes).map_err(|e| Error::Manifest(e.to_string()))?;
        Ok(Self::from_games(games))
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(&self.games).expect("manifesto serializa")
    }

    pub fn len(&self) -> usize {
        self.games.len()
    }

    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    pub fn get(&self, title: &str) -> Option<(&str, &GameEntry)> {
        self.games
            .get_key_value(title)
            .map(|(k, v)| (k.as_str(), v))
    }

    pub fn by_steam_id(&self, id: u64) -> Option<(&str, &GameEntry)> {
        self.by_steam.get(&id).and_then(|t| self.get(t))
    }

    pub fn by_gog_id(&self, id: u64) -> Option<(&str, &GameEntry)> {
        self.by_gog.get(&id).and_then(|t| self.get(t))
    }

    pub fn by_lutris(&self, slug: &str) -> Option<(&str, &GameEntry)> {
        self.by_lutris.get(slug).and_then(|t| self.get(t))
    }

    /// Busca pelo nome, ignorando maiúsculas, pontuação e símbolos de marca.
    pub fn by_name(&self, name: &str) -> Option<(&str, &GameEntry)> {
        self.by_name.get(&normalize(name)).and_then(|t| self.get(t))
    }
}

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Arquivos do cache do manifesto.
#[derive(Debug, Clone)]
pub struct Cache {
    pub dir: PathBuf,
}

impl Cache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Cache { dir: dir.into() }
    }

    /// Pasta padrão: `<cache>/manifest`.
    pub fn default_dir() -> Self {
        Cache::new(relicario_platform::cache_dir().join("manifest"))
    }

    fn json(&self) -> PathBuf {
        self.dir.join("manifest.json")
    }

    fn etag(&self) -> PathBuf {
        self.dir.join("manifest.etag")
    }

    /// Lê o manifesto do cache, se existir.
    pub fn load(&self) -> Result<Option<Manifest>> {
        match std::fs::read(self.json()) {
            Ok(b) => Ok(Some(Manifest::from_json(&b)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io(&self.json())(e)),
        }
    }

    /// Grava a partir do YAML baixado.
    pub fn store_yaml(&self, yaml: &str, etag: Option<&str>) -> Result<Manifest> {
        let m = Manifest::from_yaml(yaml)?;
        std::fs::create_dir_all(&self.dir).map_err(io(&self.dir))?;
        write_atomic(&self.json(), &m.to_json())?;
        match etag {
            Some(e) => write_atomic(&self.etag(), e.as_bytes())?,
            None => {
                let _ = std::fs::remove_file(self.etag());
            }
        }
        Ok(m)
    }

    /// Idade do cache, se existir.
    pub fn age(&self) -> Option<std::time::Duration> {
        let m = std::fs::metadata(self.json()).ok()?.modified().ok()?;
        std::time::SystemTime::now().duration_since(m).ok()
    }

    pub fn stored_etag(&self) -> Option<String> {
        std::fs::read_to_string(self.etag())
            .ok()
            .map(|s| s.trim().to_string())
    }

    fn touch(&self) -> Result<()> {
        let p = self.json();
        let f = std::fs::File::options()
            .write(true)
            .open(&p)
            .map_err(io(&p))?;
        f.set_modified(std::time::SystemTime::now()).map_err(io(&p))
    }
}

fn write_atomic(p: &Path, data: &[u8]) -> Result<()> {
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(io(&tmp))?;
    std::fs::rename(&tmp, p).map_err(io(p))
}

/// Resultado de [`update`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// O cache tem menos de uma semana; nada foi baixado.
    Fresh,
    /// O servidor respondeu que não mudou (ETag).
    NotModified,
    Downloaded,
}

/// Confere e baixa o manifesto se o cache tiver mais de uma semana (ou com `force`).
#[cfg(feature = "net")]
pub fn update(cache: &Cache, force: bool) -> Result<UpdateOutcome> {
    if !force && cache.age().is_some_and(|a| a < UPDATE_INTERVAL) {
        return Ok(UpdateOutcome::Fresh);
    }
    let agent = crate::art::http_agent();
    let mut req = agent.get(MANIFEST_URL);
    let have_cache = cache.json().exists();
    if have_cache && let Some(e) = cache.stored_etag() {
        req = req.header("If-None-Match", &e);
    }
    let mut resp = req.call().map_err(|e| Error::Net(e.to_string()))?;
    if resp.status() == 304 {
        cache.touch()?;
        return Ok(UpdateOutcome::NotModified);
    }
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = resp
        .body_mut()
        .with_config()
        .limit(256 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| Error::Net(e.to_string()))?;
    cache.store_yaml(&body, etag.as_deref())?;
    Ok(UpdateOutcome::Downloaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
Hookshot:
  cloud:
    steam: true
  files:
    "<home>/.config/unity3d/NanningsGames/Hookshot/*.sav":
      when:
        - os: linux
          store: steam
    "<home>/AppData/LocalLow/NanningsGames/Hookshot/*.sav":
      when:
        - store: steam
  installDir:
    Hookshot: {}
  launch:
    "<base>/Hookshot.x86_64":
      - when:
          - os: linux
            store: steam
  steam:
    id: 1031570
Hookshot Alias:
  alias: Hookshot
Outro Jogo:
  files:
    "<winDocuments>/My Games/Outro":
      tags:
        - save
  gog:
    id: 42
  id:
    lutris: outro-jogo
    steamExtra:
      - 7
"#;

    #[test]
    fn le_yaml_e_indices() {
        let m = Manifest::from_yaml(YAML).unwrap();
        assert_eq!(m.len(), 3);
        let (t, e) = m.by_steam_id(1031570).unwrap();
        assert_eq!(t, "Hookshot");
        assert!(e.steam_cloud());
        assert_eq!(e.files.len(), 2);
        assert_eq!(
            e.files["<home>/.config/unity3d/NanningsGames/Hookshot/*.sav"].when[0]
                .os
                .as_deref(),
            Some("linux")
        );
        assert_eq!(m.by_gog_id(42).unwrap().0, "Outro Jogo");
        assert_eq!(m.by_steam_id(7).unwrap().0, "Outro Jogo");
        assert_eq!(m.by_lutris("outro-jogo").unwrap().0, "Outro Jogo");
        assert_eq!(m.by_name("hookshot alias").unwrap().0, "Hookshot");
        assert_eq!(m.by_name("OUTRO-JOGO").unwrap().0, "Outro Jogo");
    }

    #[test]
    fn cache_ida_e_volta() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path());
        assert!(c.load().unwrap().is_none());
        c.store_yaml(YAML, Some("\"abc\"")).unwrap();
        let m = c.load().unwrap().unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(c.stored_etag().as_deref(), Some("\"abc\""));
        assert!(c.age().unwrap() < UPDATE_INTERVAL);
    }
}

#[cfg(test)]
mod real {
    /// Confere o manifesto de verdade quando `RELICARIO_MANIFEST_YAML` aponta para ele.
    #[test]
    fn manifesto_real() {
        let Some(p) = std::env::var_os("RELICARIO_MANIFEST_YAML") else {
            return;
        };
        let text = std::fs::read_to_string(p).unwrap();
        let t = std::time::Instant::now();
        let m = super::Manifest::from_yaml(&text).unwrap();
        let yaml_time = t.elapsed();
        let json = m.to_json();
        let t = std::time::Instant::now();
        let m2 = super::Manifest::from_json(&json).unwrap();
        eprintln!(
            "{} jogos; yaml {:?}; json {:?} ({} MB)",
            m.len(),
            yaml_time,
            t.elapsed(),
            json.len() / 1_000_000
        );
        assert_eq!(m.len(), m2.len());
        assert!(m.len() > 10_000);
        assert!(m.by_steam_id(1086940).is_some());
    }
}
