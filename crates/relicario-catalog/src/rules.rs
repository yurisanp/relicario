//! Regras locais por jogo: ignorar arquivos, acrescentar pastas, ignorar o jogo inteiro.

use serde::{Deserialize, Serialize};

/// Uma regra. Vale para o jogo com o id dado (slug).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub game: String,
    /// Padrões a ignorar em todas as raízes do jogo.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// Ignorar o jogo inteiro.
    #[serde(default)]
    pub ignore: bool,
    /// Pastas extras.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<ExtraRoot>,
    /// Como sincronizar. Sem valor: só histórico quando o jogo já tem Steam Cloud, completo nos
    /// outros.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<SyncMode>,
}

/// Modo de sincronização de um jogo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// Guarda e traz saves entre máquinas.
    Completo,
    /// Só guarda o histórico; quem leva o save entre máquinas é a nuvem da loja.
    Historico,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraRoot {
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

/// Regras embutidas, para casos conhecidos.
pub fn builtin() -> Vec<Rule> {
    vec![Rule {
        // Do plano original: o Hookshot guarda backups próprios e opções de vídeo junto do save.
        game: "hookshot".into(),
        exclude: vec![
            "save-backups/**".into(),
            "**/save-backups/**".into(),
            "hookshot_options.ini".into(),
            "**/hookshot_options.ini".into(),
        ],
        ..Rule::default()
    }]
}

/// Junta regras do usuário (que vêm por último e ganham) com as embutidas.
pub fn merged(user: &[Rule]) -> Vec<Rule> {
    let mut v = builtin();
    v.extend(user.iter().cloned());
    v
}

/// Regras que valem para um jogo, na ordem.
pub fn for_game<'a>(rules: &'a [Rule], id: &'a str) -> impl Iterator<Item = &'a Rule> + 'a {
    rules.iter().filter(move |r| r.game == id)
}
