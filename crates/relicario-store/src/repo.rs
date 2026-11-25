use crate::config::{FORMAT_VERSION, RepoConfig};
use crate::error::{Error, Result, json};
use crate::lock::{LockGuard, LockMode};
use crate::pack::{BlobLoc, IndexFile, IndexPack, PACK_MAGIC, PackBuilder};
use relicario_model::{BlobId, Hash, HeadsFile, MachineId, Snapshot, SnapshotId, Tree};
use relicario_transport::{Kind, Remote};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const CONFIG: &str = "config";

/// Um repositório aberto.
#[derive(Debug)]
pub struct Repository {
    remote: Arc<dyn Remote>,
    config: RepoConfig,
    index: HashMap<BlobId, BlobLoc>,
    /// Arquivos de índice já lidos.
    index_files: Vec<String>,
    pending: PackBuilder,
    cache: Mutex<Option<(Hash, Arc<Vec<u8>>)>>,
}

impl Repository {
    /// Cria um repositório novo num destino vazio.
    pub fn init(remote: Arc<dyn Remote>) -> Result<Self> {
        if remote.get(Kind::Config, CONFIG)?.is_some() {
            return Err(Error::AlreadyExists);
        }
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).map_err(|e| Error::Corrupt(format!("aleatório: {e}")))?;
        let id: String = id.iter().map(|b| format!("{b:02x}")).collect();
        let config = RepoConfig::new(id, crate::time::now());
        let bytes = serde_json::to_vec_pretty(&config).map_err(json(CONFIG))?;
        remote.put(Kind::Config, CONFIG, &bytes)?;
        Self::open(remote)
    }

    /// Abre um repositório existente e lê o índice.
    pub fn open(remote: Arc<dyn Remote>) -> Result<Self> {
        let bytes = remote
            .get(Kind::Config, CONFIG)?
            .ok_or(Error::NotARepository)?;
        let config: RepoConfig = serde_json::from_slice(&bytes).map_err(json(CONFIG))?;
        if config.format_version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion(config.format_version));
        }
        if config.encryption != "none" {
            return Err(Error::Corrupt(format!(
                "criptografia {:?} não suportada por esta versão",
                config.encryption
            )));
        }
        let mut repo = Repository {
            remote,
            config,
            index: HashMap::new(),
            index_files: Vec::new(),
            pending: PackBuilder::default(),
            cache: Mutex::new(None),
        };
        repo.reload_index()?;
        Ok(repo)
    }

    pub fn config(&self) -> &RepoConfig {
        &self.config
    }

    pub fn remote(&self) -> &Arc<dyn Remote> {
        &self.remote
    }

    /// Lê índices novos escritos por outras máquinas.
    pub fn reload_index(&mut self) -> Result<()> {
        let names = self.remote.list(Kind::Index)?;
        for n in &names {
            if self.index_files.contains(n) {
                continue;
            }
            let Some(bytes) = self.remote.get(Kind::Index, n)? else {
                continue;
            };
            if Hash::of(&bytes).to_hex() != *n {
                return Err(Error::Corrupt(format!("índice {n} não confere com o hash")));
            }
            let idx: IndexFile =
                serde_json::from_slice(&bytes).map_err(json(format!("index/{n}")))?;
            for p in idx.packs {
                for b in p.blobs {
                    self.index.entry(b.id).or_insert(BlobLoc {
                        pack: p.id,
                        offset: b.offset,
                        length: b.length,
                        raw_length: b.raw_length,
                    });
                }
            }
            self.index_files.push(n.clone());
        }
        // Índices apagados por um prune de outra máquina.
        self.index_files.retain(|n| names.contains(n));
        Ok(())
    }

    /// Esquece o índice em memória e lê tudo de novo.
    pub fn rebuild_index_cache(&mut self) -> Result<()> {
        self.index.clear();
        self.index_files.clear();
        self.reload_index()
    }

    pub fn has_blob(&self, id: &BlobId) -> bool {
        self.index.contains_key(id) || self.pending.contains(id).is_some()
    }

    pub fn blob_location(&self, id: &BlobId) -> Option<BlobLoc> {
        self.index.get(id).copied()
    }

    pub(crate) fn index_entries(&self) -> &HashMap<BlobId, BlobLoc> {
        &self.index
    }

    pub(crate) fn index_file_names(&self) -> &[String] {
        &self.index_files
    }

    /// Guarda um blob (com deduplicação) e devolve o id.
    pub fn put_blob(&mut self, data: &[u8]) -> Result<BlobId> {
        let id = Hash::of(data);
        if self.has_blob(&id) {
            return Ok(id);
        }
        let stored = zstd::bulk::compress(data, self.config.compression_level)
            .map_err(|e| Error::Corrupt(format!("zstd: {e}")))?;
        self.pending.push(id, &stored, data.len() as u64);
        if self.pending.len() >= self.config.pack_size {
            self.flush()?;
        }
        Ok(id)
    }

    /// Escreve o pack em montagem e o índice dele. Packs vêm antes do índice, então uma queda no
    /// meio deixa no máximo um pack órfão, que o `prune` remove.
    pub fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let pb = std::mem::take(&mut self.pending);
        let pack_id = Hash::of(&pb.data);
        self.remote.put(Kind::Packs, &pack_id.to_hex(), &pb.data)?;
        let idx = IndexFile {
            version: 1,
            packs: vec![IndexPack {
                id: pack_id,
                blobs: pb.blobs.clone(),
            }],
        };
        self.write_index(&idx)?;
        for b in pb.blobs {
            self.index.insert(
                b.id,
                BlobLoc {
                    pack: pack_id,
                    offset: b.offset,
                    length: b.length,
                    raw_length: b.raw_length,
                },
            );
        }
        Ok(())
    }

    pub(crate) fn write_index(&mut self, idx: &IndexFile) -> Result<String> {
        let bytes = serde_json::to_vec(idx).map_err(json("index"))?;
        let name = Hash::of(&bytes).to_hex();
        self.remote.put(Kind::Index, &name, &bytes)?;
        if !self.index_files.contains(&name) {
            self.index_files.push(name.clone());
        }
        Ok(name)
    }

    pub(crate) fn load_pack(&self, id: &Hash) -> Result<Arc<Vec<u8>>> {
        if let Some((cid, data)) = self.cache.lock().expect("cache").as_ref()
            && cid == id
        {
            return Ok(data.clone());
        }
        let data = self
            .remote
            .get(Kind::Packs, &id.to_hex())?
            .ok_or_else(|| Error::Corrupt(format!("pack {} ausente", id.short())))?;
        if !data.starts_with(PACK_MAGIC) {
            return Err(Error::Corrupt(format!("pack {} sem cabeçalho", id.short())));
        }
        let data = Arc::new(data);
        *self.cache.lock().expect("cache") = Some((*id, data.clone()));
        Ok(data)
    }

    /// Lê um blob e confere o hash.
    pub fn get_blob(&self, id: &BlobId) -> Result<Vec<u8>> {
        let (stored, raw_len) = if let Some(b) = self.pending.contains(id) {
            let s = &self.pending.data[b.offset as usize..(b.offset + b.length) as usize];
            (s.to_vec(), b.raw_length)
        } else {
            let loc = self.index.get(id).ok_or(Error::MissingBlob(*id))?;
            let pack = self.load_pack(&loc.pack)?;
            let end = (loc.offset + loc.length) as usize;
            if end > pack.len() {
                return Err(Error::Corrupt(format!(
                    "pack {} truncado",
                    loc.pack.short()
                )));
            }
            (pack[loc.offset as usize..end].to_vec(), loc.raw_length)
        };
        let data = zstd::bulk::decompress(&stored, raw_len as usize)
            .map_err(|e| Error::Corrupt(format!("bloco {}: {e}", id.short())))?;
        if Hash::of(&data) != *id {
            return Err(Error::Corrupt(format!(
                "bloco {} não confere com o hash",
                id.short()
            )));
        }
        Ok(data)
    }

    pub fn put_tree(&mut self, tree: &Tree) -> Result<BlobId> {
        let bytes = serde_json::to_vec(tree).map_err(json("tree"))?;
        self.put_blob(&bytes)
    }

    pub fn get_tree(&self, id: &BlobId) -> Result<Tree> {
        let bytes = self.get_blob(id)?;
        serde_json::from_slice(&bytes).map_err(json(format!("tree {}", id.short())))
    }

    /// Grava os metadados de um snapshot. Faz `flush` antes, para que um snapshot nunca aponte
    /// para blocos que ainda não estão no destino.
    pub fn put_snapshot(&mut self, s: &Snapshot) -> Result<SnapshotId> {
        self.flush()?;
        let id = s.id();
        let bytes = serde_json::to_vec_pretty(s).map_err(json("snapshot"))?;
        self.remote.put(Kind::Snapshots, &id.to_hex(), &bytes)?;
        Ok(id)
    }

    pub fn get_snapshot(&self, id: &SnapshotId) -> Result<Snapshot> {
        let bytes = self
            .remote
            .get(Kind::Snapshots, &id.to_hex())?
            .ok_or(Error::MissingSnapshot(*id))?;
        let s: Snapshot =
            serde_json::from_slice(&bytes).map_err(json(format!("snapshot {}", id.short())))?;
        if s.id() != *id {
            return Err(Error::Corrupt(format!(
                "snapshot {} não confere com o hash",
                id.short()
            )));
        }
        Ok(s)
    }

    /// Todos os snapshots. Arquivos ilegíveis viram erro na lista, sem derrubar o resto.
    pub fn snapshots(&self) -> Result<Vec<(SnapshotId, Result<Snapshot>)>> {
        let mut out = Vec::new();
        for n in self.remote.list(Kind::Snapshots)? {
            let Ok(id) = n.parse::<Hash>() else { continue };
            out.push((id, self.get_snapshot(&id)));
        }
        Ok(out)
    }

    /// Snapshots legíveis.
    pub fn valid_snapshots(&self) -> Result<Vec<(SnapshotId, Snapshot)>> {
        Ok(self
            .snapshots()?
            .into_iter()
            .filter_map(|(id, s)| s.ok().map(|s| (id, s)))
            .collect())
    }

    pub fn heads(&self) -> Result<Vec<HeadsFile>> {
        let mut out = Vec::new();
        for n in self.remote.list(Kind::Heads)? {
            if let Some(h) = self.heads_of(&MachineId::new(n))? {
                out.push(h);
            }
        }
        Ok(out)
    }

    pub fn heads_of(&self, machine: &MachineId) -> Result<Option<HeadsFile>> {
        let Some(bytes) = self.remote.get(Kind::Heads, machine.as_str())? else {
            return Ok(None);
        };
        let h = serde_json::from_slice(&bytes).map_err(json(format!("heads/{machine}")))?;
        Ok(Some(h))
    }

    /// Grava o arquivo de heads desta máquina.
    pub fn put_heads(&mut self, machine: &MachineId, h: &HeadsFile) -> Result<()> {
        self.flush()?;
        let bytes = serde_json::to_vec_pretty(h).map_err(json("heads"))?;
        self.remote.put(Kind::Heads, machine.as_str(), &bytes)?;
        Ok(())
    }

    /// Trava o repositório para esta máquina.
    pub fn lock(&self, machine: &MachineId, mode: LockMode) -> Result<LockGuard> {
        LockGuard::acquire(self.remote.clone(), machine, mode)
    }

    pub(crate) fn delete(&mut self, kind: Kind, name: &str) -> Result<()> {
        self.remote.delete(kind, name)?;
        Ok(())
    }
}
