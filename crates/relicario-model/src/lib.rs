//! Tipos centrais do Relicário: jogos, máquinas, snapshots e heads.
//!
//! Este crate não depende do sistema operacional nem faz I/O.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

mod hash;
pub use hash::{Hash, HashParseError};

macro_rules! text_id {
    ($(#[$m:meta])* $nome:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $nome(String);

        impl $nome {
            pub fn new(valor: impl Into<String>) -> Self {
                Self(valor.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $nome {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

text_id!(
    /// Identificador estável de um jogo (slug), igual em todas as máquinas.
    GameId
);
text_id!(
    /// Identificador de uma máquina. Na fase 4 passa a ser a chave pública ed25519.
    MachineId
);

/// Identificador de um snapshot: BLAKE3 dos metadados canônicos.
pub type SnapshotId = Hash;
/// Identificador de um blob (bloco de dados ou árvore): BLAKE3 do conteúdo em claro.
pub type BlobId = Hash;

/// Milissegundos desde a época Unix, em UTC.
pub type Millis = i64;

/// Um arquivo dentro de uma raiz de save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Nome lógico da raiz (por exemplo `save`), igual em todas as máquinas.
    pub root: String,
    /// Caminho relativo à raiz, sempre com `/`.
    pub path: String,
    pub size: u64,
    /// Última modificação, em milissegundos Unix.
    pub mtime: Millis,
    /// BLAKE3 do conteúdo inteiro.
    pub hash: Hash,
    /// Formato reconhecido pelo começo do arquivo, usado pelo guarda contra save corrompido.
    #[serde(default)]
    pub kind: FileKind,
    /// Blocos na ordem, cada um endereçado pelo BLAKE3 do seu conteúdo.
    pub chunks: Vec<BlobId>,
}

/// Formato de um arquivo, reconhecido pelos primeiros bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    #[default]
    Desconhecido,
    Vazio,
    Zeros,
    Texto,
    Binario,
    Gzip,
    Zip,
    Zstd,
    Zlib,
    Sqlite,
    Png,
    Lz4,
}

impl FileKind {
    /// Classifica um arquivo pelo começo (até 512 bytes) e por saber se é todo zero.
    pub fn detect(prefix: &[u8], all_zero: bool) -> Self {
        if prefix.is_empty() {
            return FileKind::Vazio;
        }
        if all_zero {
            return FileKind::Zeros;
        }
        let starts = |m: &[u8]| prefix.starts_with(m);
        if starts(&[0x1f, 0x8b]) {
            FileKind::Gzip
        } else if starts(b"PK\x03\x04") {
            FileKind::Zip
        } else if starts(&[0x28, 0xb5, 0x2f, 0xfd]) {
            FileKind::Zstd
        } else if starts(b"SQLite format 3") {
            FileKind::Sqlite
        } else if starts(&[0x89, b'P', b'N', b'G']) {
            FileKind::Png
        } else if starts(&[0x04, 0x22, 0x4d, 0x18]) {
            FileKind::Lz4
        } else if prefix.len() >= 2
            && prefix[0] == 0x78
            && matches!(prefix[1], 0x01 | 0x5e | 0x9c | 0xda)
        {
            FileKind::Zlib
        } else if std::str::from_utf8(prefix)
            .is_ok_and(|t| t.chars().all(|c| !c.is_control() || c.is_whitespace()))
            || utf8_cut(prefix)
        {
            FileKind::Texto
        } else {
            FileKind::Binario
        }
    }
}

/// Texto UTF-8 válido que só quebrou porque o prefixo cortou um caractere no meio.
fn utf8_cut(prefix: &[u8]) -> bool {
    match std::str::from_utf8(prefix) {
        Ok(_) => false,
        Err(e) => {
            e.error_len().is_none()
                && std::str::from_utf8(&prefix[..e.valid_up_to()])
                    .is_ok_and(|t| t.chars().all(|c| !c.is_control() || c.is_whitespace()))
        }
    }
}

/// Lista de arquivos de um snapshot, guardada como blob no repositório.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree {
    /// Ordenados por `(root, path)`.
    pub files: Vec<FileEntry>,
}

impl Tree {
    pub fn new(mut files: Vec<FileEntry>) -> Self {
        files.sort_by(|a, b| (&a.root, &a.path).cmp(&(&b.root, &b.path)));
        Self { files }
    }

    /// Hash do conteúdo, ignorando horários: dois saves iguais em máquinas diferentes têm o
    /// mesmo valor.
    pub fn content_hash(&self) -> Hash {
        let mut h = blake3::Hasher::new();
        h.update(b"relicario-content-v1\0");
        for f in &self.files {
            h.update(f.root.as_bytes());
            h.update(b"\0");
            h.update(f.path.as_bytes());
            h.update(b"\0");
            h.update(&f.size.to_le_bytes());
            h.update(f.hash.as_bytes());
        }
        Hash::from_bytes(*h.finalize().as_bytes())
    }

    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn get(&self, root: &str, path: &str) -> Option<&FileEntry> {
        self.files
            .binary_search_by(|f| (f.root.as_str(), f.path.as_str()).cmp(&(root, path)))
            .ok()
            .map(|i| &self.files[i])
    }
}

/// Metadados de um snapshot. Imutável depois de gravado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub game: GameId,
    pub machine: MachineId,
    /// Pais na árvore. Vazio no primeiro snapshot; dois ou mais num merge.
    pub parents: Vec<SnapshotId>,
    /// Blob com a [`Tree`].
    pub tree: BlobId,
    /// [`Tree::content_hash`] da árvore.
    pub content: Hash,
    pub created_at: Millis,
    pub mtime_max: Millis,
    pub size: u64,
    pub files: u64,
    /// Por que o snapshot foi feito.
    pub reason: SnapshotReason,
    /// Avisos do guarda contra save corrompido.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suspicions: Vec<Suspicion>,
    /// Metadados extraídos por plugins de leitura de save (fase 6).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meta: BTreeMap<String, String>,
}

impl Snapshot {
    /// BLAKE3 da forma canônica (JSON com campos em ordem fixa).
    pub fn id(&self) -> SnapshotId {
        let bytes = serde_json_canonical(self);
        let mut h = blake3::Hasher::new();
        h.update(b"relicario-snapshot-v1\0");
        h.update(&bytes);
        Hash::from_bytes(*h.finalize().as_bytes())
    }
}

fn serde_json_canonical(s: &Snapshot) -> Vec<u8> {
    // Os campos são serializados na ordem da struct e os mapas são BTreeMap, então a saída é
    // determinística. Evitamos depender de serde_json aqui para manter o crate leve.
    let mut out = String::new();
    out.push_str(s.game.as_str());
    out.push('\0');
    out.push_str(s.machine.as_str());
    out.push('\0');
    for p in &s.parents {
        out.push_str(&p.to_hex());
        out.push(',');
    }
    out.push('\0');
    out.push_str(&s.tree.to_hex());
    out.push('\0');
    out.push_str(&s.content.to_hex());
    out.push('\0');
    out.push_str(&format!(
        "{}\0{}\0{}\0{}\0{:?}\0",
        s.created_at, s.mtime_max, s.size, s.files, s.reason
    ));
    for x in &s.suspicions {
        out.push_str(&format!("{x:?},"));
    }
    out.push('\0');
    for (k, v) in &s.meta {
        out.push_str(k);
        out.push('=');
        out.push_str(v);
        out.push('\0');
    }
    out.into_bytes()
}

/// Por que um snapshot foi feito.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotReason {
    /// Pedido pelo usuário ou pela CLI.
    Manual,
    /// Ao fechar o jogo.
    GameExit,
    /// Conferência periódica.
    Periodic,
    /// Cópia de segurança do estado local antes de uma restauração.
    BeforeRestore,
    /// Resolução de conflito com mais de um pai.
    Merge,
    /// Importado de outra ferramenta (Ludusavi).
    Import,
    /// Volta a uma versão antiga, pedida pelo usuário: mesmo conteúdo, mas descende do head
    /// atual, para que as outras máquinas sigam a volta em vez de ver um conflito.
    Restored,
}

/// Motivo para desconfiar de um save novo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Suspicion {
    /// O tamanho total encolheu mais que o limite (em porcentagem).
    Shrunk { from: u64, to: u64 },
    /// O save ficou vazio ou todos os arquivos zeraram.
    Emptied,
    /// Um arquivo virou só zeros.
    Zeroed { path: String },
    /// Um arquivo trocou de tipo (o começo do arquivo mudou de formato).
    TypeChanged { path: String },
}

/// Estado de uma máquina publicado no repositório. É o único arquivo mutável, e só a própria
/// máquina escreve nele.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadsFile {
    pub machine: Option<MachineId>,
    #[serde(default)]
    pub machine_name: String,
    /// Último snapshot de cada jogo nesta máquina.
    #[serde(default)]
    pub heads: BTreeMap<GameId, Head>,
    /// Snapshots fixados por esta máquina, com nome opcional.
    #[serde(default)]
    pub pins: BTreeMap<SnapshotId, String>,
    /// Snapshots desta máquina cujos dados foram liberados pela retenção. Os metadados ficam,
    /// para a árvore continuar inteira.
    #[serde(default)]
    pub forgotten: BTreeSet<SnapshotId>,
    /// Tempo de jogo acumulado nesta máquina, em milissegundos.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub playtime: BTreeMap<GameId, u64>,
    /// Jogos abertos agora nesta máquina, com o horário em que abriram.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub running: BTreeMap<GameId, Millis>,
    /// Executáveis que o usuário associou a cada jogo nesta máquina. As outras máquinas usam a
    /// mesma lista de nomes (o caminho de cada uma é ajustado localmente).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub associations: BTreeMap<GameId, Vec<String>>,
    /// Endereços (`host:porta`) onde esta máquina aceita sincronização direta.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<String>,
    pub updated_at: Millis,
}

/// O último snapshot conhecido de um jogo numa máquina.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Head {
    pub snapshot: SnapshotId,
    pub updated_at: Millis,
}

/// Relação entre o head local e o head de outra máquina.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// Os dois têm o mesmo conteúdo.
    Igual,
    /// O remoto descende do local: dá para avançar sem perder nada.
    Avanco,
    /// O local descende do remoto: há algo novo para enviar.
    Envio,
    /// Os dois mudaram desde o ancestral comum, ou a história está incompleta.
    Divergiu,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, data: &[u8], mtime: Millis) -> FileEntry {
        FileEntry {
            root: "save".into(),
            path: path.into(),
            size: data.len() as u64,
            mtime,
            hash: Hash::of(data),
            kind: FileKind::detect(data, false),
            chunks: vec![Hash::of(data)],
        }
    }

    #[test]
    fn conteudo_ignora_horario_e_ordem() {
        let a = Tree::new(vec![entry("a", b"1", 10), entry("b", b"2", 10)]);
        let b = Tree::new(vec![entry("b", b"2", 99), entry("a", b"1", 50)]);
        assert_eq!(a.content_hash(), b.content_hash());
        let c = Tree::new(vec![entry("a", b"1", 10), entry("b", b"3", 10)]);
        assert_ne!(a.content_hash(), c.content_hash());
        assert_eq!(a.get("save", "b").unwrap().size, 1);
    }

    #[test]
    fn detecta_formatos() {
        assert_eq!(FileKind::detect(b"", false), FileKind::Vazio);
        assert_eq!(FileKind::detect(&[0, 0, 0], true), FileKind::Zeros);
        assert_eq!(
            FileKind::detect(b"[save]\nslot=1\n", false),
            FileKind::Texto
        );
        assert_eq!(FileKind::detect(&[0x1f, 0x8b, 8, 0], false), FileKind::Gzip);
        assert_eq!(
            FileKind::detect(&[0x00, 0xff, 0x13, 0x02], false),
            FileKind::Binario
        );
        assert_eq!(FileKind::detect("olá".as_bytes(), false), FileKind::Texto);
        assert_eq!(
            FileKind::detect(&"olá".as_bytes()[..3], false),
            FileKind::Texto
        );
    }

    #[test]
    fn id_do_snapshot_e_estavel() {
        let t = Tree::new(vec![entry("a", b"1", 10)]);
        let s = Snapshot {
            game: GameId::new("hookshot"),
            machine: MachineId::new("pc"),
            parents: vec![],
            tree: Hash::of(b"tree"),
            content: t.content_hash(),
            created_at: 1,
            mtime_max: 10,
            size: 1,
            files: 1,
            reason: SnapshotReason::Manual,
            suspicions: vec![],
            meta: BTreeMap::new(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(s.id(), back.id());
        let mut outro = s.clone();
        outro.created_at = 2;
        assert_ne!(s.id(), outro.id());
    }
}
