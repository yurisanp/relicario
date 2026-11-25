//! Packs e índice.

use relicario_model::{BlobId, Hash};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Começo de todo pack.
pub const PACK_MAGIC: &[u8; 8] = b"RLCPACK1";

/// Onde um blob está.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobLoc {
    pub pack: Hash,
    pub offset: u64,
    pub length: u64,
    pub raw_length: u64,
}

/// Arquivo `index/<hash>`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexFile {
    pub version: u32,
    pub packs: Vec<IndexPack>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexPack {
    pub id: Hash,
    pub blobs: Vec<IndexBlob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexBlob {
    pub id: BlobId,
    pub offset: u64,
    pub length: u64,
    pub raw_length: u64,
}

/// Pack em montagem na memória.
#[derive(Debug, Default)]
pub struct PackBuilder {
    pub data: Vec<u8>,
    pub blobs: Vec<IndexBlob>,
    pos: HashMap<BlobId, usize>,
}

impl PackBuilder {
    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty()
    }

    pub fn len(&self) -> u64 {
        self.data.len() as u64
    }

    pub fn push(&mut self, id: BlobId, stored: &[u8], raw_length: u64) {
        if self.data.is_empty() {
            self.data.extend_from_slice(PACK_MAGIC);
        }
        let offset = self.data.len() as u64;
        self.data.extend_from_slice(stored);
        self.pos.insert(id, self.blobs.len());
        self.blobs.push(IndexBlob {
            id,
            offset,
            length: stored.len() as u64,
            raw_length,
        });
    }

    pub fn contains(&self, id: &BlobId) -> Option<&IndexBlob> {
        self.pos.get(id).map(|i| &self.blobs[*i])
    }
}
