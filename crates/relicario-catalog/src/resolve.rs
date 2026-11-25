//! Resolução dos placeholders do manifesto para caminhos desta máquina.
//!
//! Placeholders suportados (documentação do Ludusavi): `<root>`, `<game>`, `<base>`, `<home>`,
//! `<storeUserId>`, `<osUserName>`, `<winAppData>`, `<winLocalAppData>`,
//! `<winLocalAppDataLow>`, `<winDocuments>`, `<winPublic>`, `<winProgramData>`, `<winDir>`,
//! `<xdgData>`, `<xdgConfig>`. Chaves de registro (`<regHkcu>`, `<regHklm>`) ainda não são
//! salvas.

use crate::manifest::{Constraint, FileRule};
use std::path::{Path, PathBuf};

/// Sistema que o jogo "enxerga". Um jogo no Proton enxerga Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameOs {
    Linux,
    Windows,
    Mac,
}

impl GameOs {
    pub fn host() -> Self {
        match relicario_platform::Os::atual() {
            relicario_platform::Os::Windows => GameOs::Windows,
            relicario_platform::Os::MacOs => GameOs::Mac,
            _ => GameOs::Linux,
        }
    }

    fn manifest_name(self) -> &'static str {
        match self {
            GameOs::Linux => "linux",
            GameOs::Windows => "windows",
            GameOs::Mac => "mac",
        }
    }
}

/// Pastas do usuário no Windows, reais ou dentro de um prefixo Wine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinDirs {
    pub home: PathBuf,
    pub app_data: PathBuf,
    pub local_app_data: PathBuf,
    pub local_app_data_low: PathBuf,
    pub documents: PathBuf,
    pub public: PathBuf,
    pub program_data: PathBuf,
    pub windir: PathBuf,
    pub user_name: String,
}

impl WinDirs {
    /// Layout de um prefixo Wine (`<prefixo>/drive_c/users/<usuário>`).
    pub fn wine_prefix(prefix: &Path, user: &str) -> Self {
        let c = prefix.join("drive_c");
        let home = c.join("users").join(user);
        WinDirs {
            app_data: home.join("AppData/Roaming"),
            local_app_data: home.join("AppData/Local"),
            local_app_data_low: home.join("AppData/LocalLow"),
            documents: home.join("Documents"),
            public: c.join("users/Public"),
            program_data: c.join("ProgramData"),
            windir: c.join("windows"),
            user_name: user.to_string(),
            home,
        }
    }

    /// Prefixo Wine com o usuário que existir (`steamuser` no Proton, o usuário do sistema no
    /// Wine comum).
    pub fn detect_wine_prefix(prefix: &Path) -> Self {
        let users = prefix.join("drive_c/users");
        let mut chosen = None;
        if let Ok(rd) = std::fs::read_dir(&users) {
            let mut names: Vec<String> = rd
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n != "Public" && n != "Default")
                .collect();
            names.sort();
            if names.iter().any(|n| n == "steamuser") {
                chosen = Some("steamuser".to_string());
            } else {
                chosen = names.into_iter().next();
            }
        }
        Self::wine_prefix(prefix, chosen.as_deref().unwrap_or("steamuser"))
    }

    /// Pastas reais do Windows a partir das variáveis de ambiente.
    pub fn native() -> Option<Self> {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let home = env("USERPROFILE")?;
        let app_data = env("APPDATA").unwrap_or_else(|| home.join("AppData/Roaming"));
        let local = env("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData/Local"));
        let low = local
            .parent()
            .map(|p| p.join("LocalLow"))
            .unwrap_or_else(|| home.join("AppData/LocalLow"));
        Some(WinDirs {
            documents: home.join("Documents"),
            public: env("PUBLIC").unwrap_or_else(|| PathBuf::from("C:/Users/Public")),
            program_data: env("PROGRAMDATA").unwrap_or_else(|| PathBuf::from("C:/ProgramData")),
            windir: env("WINDIR").unwrap_or_else(|| PathBuf::from("C:/Windows")),
            user_name: std::env::var("USERNAME").unwrap_or_default(),
            app_data,
            local_app_data: local,
            local_app_data_low: low,
            home,
        })
    }
}

/// Onde e como um jogo roda nesta máquina.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    pub os: GameOs,
    /// Loja, no vocabulário do manifesto (`steam`, `gog`, `epic`…), se conhecida.
    pub store: Option<String>,
    pub home: PathBuf,
    pub win: Option<WinDirs>,
    pub xdg_data: Option<PathBuf>,
    pub xdg_config: Option<PathBuf>,
    /// `<root>`: pasta de biblioteca da loja.
    pub root: Option<PathBuf>,
    /// `<game>`: nome da pasta de instalação.
    pub game: Option<String>,
    /// `<base>`: pasta de instalação.
    pub base: Option<PathBuf>,
    /// `<storeUserId>`; `None` vira curinga.
    pub store_user_id: Option<String>,
    pub os_user: String,
}

impl Env {
    /// Ambiente nativo desta máquina, sem loja.
    pub fn native() -> Self {
        let home = relicario_platform::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let os = GameOs::host();
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default();
        let xdg = |k: &str, d: &str| {
            (os == GameOs::Linux).then(|| {
                std::env::var_os(k)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(d))
            })
        };
        Env {
            os,
            store: None,
            win: if os == GameOs::Windows {
                WinDirs::native()
            } else {
                None
            },
            xdg_data: xdg("XDG_DATA_HOME", ".local/share"),
            xdg_config: xdg("XDG_CONFIG_HOME", ".config"),
            root: None,
            game: None,
            base: None,
            store_user_id: None,
            os_user: user,
            home,
        }
    }

    /// Ambiente de um prefixo Wine/Proton.
    pub fn wine(prefix: &Path) -> Self {
        let w = WinDirs::detect_wine_prefix(prefix);
        Env {
            os: GameOs::Windows,
            store: None,
            home: w.home.clone(),
            os_user: w.user_name.clone(),
            win: Some(w),
            xdg_data: None,
            xdg_config: None,
            root: None,
            game: None,
            base: None,
            store_user_id: None,
        }
    }

    pub fn with_store(mut self, store: &str) -> Self {
        self.store = Some(store.to_string());
        self
    }

    pub fn with_install(
        mut self,
        root: Option<PathBuf>,
        game: Option<String>,
        base: Option<PathBuf>,
    ) -> Self {
        self.root = root;
        self.game = game;
        self.base = base;
        self
    }

    /// A regra vale neste ambiente?
    pub fn applies(&self, rule: &FileRule) -> bool {
        rule.when.is_empty() || rule.when.iter().any(|c| self.matches(c))
    }

    pub fn matches(&self, c: &Constraint) -> bool {
        let os_ok = c.os.as_deref().is_none_or(|o| o == self.os.manifest_name());
        let store_ok = match (&c.store, &self.store) {
            (None, _) => true,
            (Some(want), Some(have)) => want == have,
            (Some(_), None) => false,
        };
        os_ok && store_ok
    }

    /// Troca os placeholders. `None` quando falta algum valor (por exemplo, `<base>` de um jogo
    /// que não está instalado).
    pub fn resolve(&self, pattern: &str) -> Option<String> {
        let mut out = String::with_capacity(pattern.len() + 32);
        let mut rest = pattern;
        while let Some(start) = rest.find('<') {
            out.push_str(&rest[..start]);
            let end = rest[start..].find('>')? + start;
            let name = &rest[start + 1..end];
            let value: String = match name {
                "root" => p(self.root.as_ref()?),
                "game" => self.game.clone()?,
                "base" => p(self.base.as_ref()?),
                "home" => p(&self.home),
                "storeUserId" => self.store_user_id.clone().unwrap_or_else(|| "*".into()),
                "osUserName" => {
                    if self.os_user.is_empty() {
                        return None;
                    }
                    self.os_user.clone()
                }
                "winAppData" => p(&self.win.as_ref()?.app_data),
                "winLocalAppData" => p(&self.win.as_ref()?.local_app_data),
                "winLocalAppDataLow" => p(&self.win.as_ref()?.local_app_data_low),
                "winDocuments" => p(&self.win.as_ref()?.documents),
                "winPublic" => p(&self.win.as_ref()?.public),
                "winProgramData" => p(&self.win.as_ref()?.program_data),
                "winDir" => p(&self.win.as_ref()?.windir),
                "xdgData" => p(self.xdg_data.as_ref()?),
                "xdgConfig" => p(self.xdg_config.as_ref()?),
                _ => return None,
            };
            out.push_str(&value);
            rest = &rest[end + 1..];
        }
        out.push_str(rest);
        Some(out.replace('\\', "/"))
    }
}

fn p(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

fn is_glob(s: &str) -> bool {
    s.contains(['*', '?', '[', '{'])
}

/// Divide um caminho resolvido em pasta fixa e padrão relativo. Se não houver curinga, o
/// padrão é `None` e o caminho pode ser um arquivo ou uma pasta.
pub fn split_static(resolved: &str) -> (PathBuf, Option<String>) {
    let parts: Vec<&str> = resolved.split('/').collect();
    let first_glob = parts.iter().position(|c| is_glob(c));
    match first_glob {
        None => (PathBuf::from(resolved), None),
        Some(i) => {
            let fixed = parts[..i].join("/");
            let fixed = if fixed.is_empty() {
                "/".to_string()
            } else {
                fixed
            };
            (PathBuf::from(fixed), Some(parts[i..].join("/")))
        }
    }
}

/// Nome lógico de uma raiz a partir do padrão do manifesto (antes de resolver), para ser o
/// mesmo em todas as máquinas: a última parte fixa do caminho, em minúsculas. `depth` maior
/// usa mais partes, para desempatar.
pub fn root_name(pattern: &str, depth: usize) -> String {
    let literal: Vec<String> = pattern
        .split('/')
        .filter(|c| !c.is_empty() && !c.contains('<') && !is_glob(c))
        .map(crate::slug::slug)
        .collect();
    if literal.is_empty() {
        return "save".into();
    }
    let n = depth.min(literal.len()).max(1);
    literal[literal.len() - n..].join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux_env() -> Env {
        Env {
            os: GameOs::Linux,
            store: Some("steam".into()),
            home: "/home/yuri".into(),
            win: None,
            xdg_data: Some("/home/yuri/.local/share".into()),
            xdg_config: Some("/home/yuri/.config".into()),
            root: Some("/home/yuri/.local/share/Steam".into()),
            game: Some("Hookshot".into()),
            base: Some("/home/yuri/.local/share/Steam/steamapps/common/Hookshot".into()),
            store_user_id: None,
            os_user: "yuri".into(),
        }
    }

    #[test]
    fn resolve_placeholders() {
        let e = linux_env();
        assert_eq!(
            e.resolve("<home>/.config/unity3d/NanningsGames/Hookshot/*.sav")
                .unwrap(),
            "/home/yuri/.config/unity3d/NanningsGames/Hookshot/*.sav"
        );
        assert_eq!(
            e.resolve("<root>/userdata/<storeUserId>/123/remote")
                .unwrap(),
            "/home/yuri/.local/share/Steam/userdata/*/123/remote"
        );
        assert!(e.resolve("<winAppData>/x").is_none());
        assert!(e.resolve("<regHkcu>/x").is_none());
        let mut w = Env::wine(Path::new("/p/pfx"));
        w.base = Some("/g".into());
        assert_eq!(
            w.resolve("<winLocalAppDataLow>/A/B").unwrap(),
            "/p/pfx/drive_c/users/steamuser/AppData/LocalLow/A/B"
        );
        assert_eq!(
            w.resolve("<winDocuments>/My Games").unwrap(),
            "/p/pfx/drive_c/users/steamuser/Documents/My Games"
        );
        assert_eq!(w.resolve("<base>/saves").unwrap(), "/g/saves");
    }

    #[test]
    fn restricoes() {
        let e = linux_env();
        let r = |os: Option<&str>, store: Option<&str>| FileRule {
            tags: vec![],
            when: vec![Constraint {
                os: os.map(Into::into),
                store: store.map(Into::into),
                bit: None,
            }],
        };
        assert!(e.applies(&FileRule::default()));
        assert!(e.applies(&r(Some("linux"), Some("steam"))));
        assert!(!e.applies(&r(Some("windows"), None)));
        assert!(!e.applies(&r(None, Some("gog"))));
        let mut sem_loja = e.clone();
        sem_loja.store = None;
        assert!(!sem_loja.applies(&r(None, Some("steam"))));
    }

    #[test]
    fn divide_e_nomeia() {
        assert_eq!(
            split_static("/a/b/*.sav"),
            (PathBuf::from("/a/b"), Some("*.sav".into()))
        );
        assert_eq!(
            split_static("/a/*/c/d"),
            (PathBuf::from("/a"), Some("*/c/d".into()))
        );
        assert_eq!(split_static("/a/b"), (PathBuf::from("/a/b"), None));
        assert_eq!(
            root_name("<home>/.config/unity3d/NanningsGames/Hookshot/*.sav", 1),
            "hookshot"
        );
        assert_eq!(
            root_name("<winLocalAppDataLow>/NanningsGames/Hookshot/*.sav", 1),
            "hookshot"
        );
        assert_eq!(
            root_name("<winLocalAppDataLow>/NanningsGames/Hookshot/*.sav", 2),
            "nanningsgames-hookshot"
        );
        assert_eq!(
            root_name("<root>/userdata/<storeUserId>/1086940/remote", 1),
            "remote"
        );
        assert_eq!(root_name("<base>", 1), "save");
    }
}
