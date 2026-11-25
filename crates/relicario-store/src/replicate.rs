//! Replicação entre duas cópias do mesmo repositório (outra máquina, nuvem).
//!
//! Funciona porque o formato só tem arquivos imutáveis com nome igual ao hash do conteúdo,
//! mais um `heads/<máquina>` que só a própria máquina escreve. Replicar é copiar o que falta
//! nos dois sentidos e, nos heads, ficar com o mais novo de cada máquina. Não há como uma cópia
//! sobrescrever o trabalho da outra.
//!
//! A ordem (packs → índice → snapshots → heads) é a mesma da escrita local: se a conexão cair
//! no meio, o outro lado fica com arquivos a mais, nunca com um head apontando para o que não
//! chegou. A próxima replicação continua de onde parou, porque só copia o que falta.

use crate::config::RepoConfig;
use crate::{Error, Result};
use relicario_model::{Hash, HeadsFile, MachineId, Snapshot};
use relicario_transport::{Kind, Remote};
use std::collections::BTreeSet;

const CONFIG: &str = "config";

/// Tipos copiados por inteiro, na ordem em que precisam chegar.
pub const IMMUTABLE: [Kind; 4] = [Kind::Keys, Kind::Packs, Kind::Index, Kind::Snapshots];

/// O que uma replicação fez.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub pulled: usize,
    pub pushed: usize,
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// Máquinas cujo head mudou aqui.
    pub heads_in: Vec<String>,
    /// Máquinas cujo head mudou lá.
    pub heads_out: Vec<String>,
    /// Arquivos recusados por não baterem com o hash do nome.
    pub rejected: Vec<String>,
}

impl Report {
    pub fn changed(&self) -> bool {
        self.pulled + self.pushed > 0 || !self.heads_in.is_empty() || !self.heads_out.is_empty()
    }
}

/// Confere se o conteúdo bate com o nome. `Keys` não tem nome por hash.
pub fn verify_file(kind: Kind, name: &str, data: &[u8]) -> bool {
    match kind {
        Kind::Packs | Kind::Index => Hash::of(data).to_hex() == name,
        Kind::Snapshots => {
            serde_json::from_slice::<Snapshot>(data).is_ok_and(|s| s.id().to_hex() == name)
        }
        Kind::Heads => serde_json::from_slice::<HeadsFile>(data)
            .is_ok_and(|h| h.machine.is_some_and(|m| m.as_str() == name)),
        Kind::Keys => !data.is_empty(),
        Kind::Config | Kind::Locks => false,
    }
}

pub fn read_config(r: &dyn Remote) -> Result<RepoConfig> {
    let bytes = r.get(Kind::Config, CONFIG)?.ok_or(Error::NotARepository)?;
    serde_json::from_slice(&bytes).map_err(|e| Error::Corrupt(format!("config: {e}")))
}

/// Os dois repositórios podem ser juntados? Mesmo id, ou mesmos parâmetros sem criptografia
/// (blocos sem chave têm o mesmo nome nos dois, então juntar é seguro).
pub fn compatible(a: &RepoConfig, b: &RepoConfig) -> bool {
    a.id == b.id
        || (a.format_version == b.format_version
            && a.chunker == b.chunker
            && a.encryption == "none"
            && b.encryption == "none")
}

fn heads_of(r: &dyn Remote, name: &str) -> Result<Option<HeadsFile>> {
    Ok(r.get(Kind::Heads, name)?
        .and_then(|b| serde_json::from_slice(&b).ok()))
}

/// Copia o que falta de `from` para `to` num tipo. Devolve (arquivos, bytes, recusados).
fn copy_missing(
    from: &dyn Remote,
    to: &dyn Remote,
    kind: Kind,
    names: &BTreeSet<String>,
    rejected: &mut Vec<String>,
) -> Result<(usize, u64)> {
    let (mut n, mut bytes) = (0, 0);
    for name in names {
        let Some(data) = from.get(kind, name)? else {
            continue;
        };
        if !verify_file(kind, name, &data) {
            rejected.push(format!("{}/{name}", kind.dir()));
            continue;
        }
        to.put(kind, name, &data)?;
        n += 1;
        bytes += data.len() as u64;
    }
    Ok((n, bytes))
}

/// Replica nos dois sentidos. `me` é a máquina dona de `local`: o head dela nunca é trocado pelo
/// que veio de fora.
pub fn replicate(local: &dyn Remote, peer: &dyn Remote, me: &MachineId) -> Result<Report> {
    let (lc, pc) = (read_config(local)?, read_config(peer)?);
    if !compatible(&lc, &pc) {
        return Err(Error::Corrupt(format!(
            "os repositórios são diferentes ({} e {}) e não podem ser juntados",
            lc.id, pc.id
        )));
    }
    let mut rep = Report::default();
    for kind in IMMUTABLE {
        let l: BTreeSet<String> = local.list(kind)?.into_iter().collect();
        let p: BTreeSet<String> = peer.list(kind)?.into_iter().collect();
        let pull: BTreeSet<String> = p.difference(&l).cloned().collect();
        let push: BTreeSet<String> = l.difference(&p).cloned().collect();
        let (n, b) = copy_missing(peer, local, kind, &pull, &mut rep.rejected)?;
        rep.pulled += n;
        rep.bytes_in += b;
        let (n, b) = copy_missing(local, peer, kind, &push, &mut rep.rejected)?;
        rep.pushed += n;
        rep.bytes_out += b;
    }
    let mut names: BTreeSet<String> = local.list(Kind::Heads)?.into_iter().collect();
    names.extend(peer.list(Kind::Heads)?);
    for name in names {
        let (lh, ph) = (heads_of(local, &name)?, heads_of(peer, &name)?);
        let newer = |a: &Option<HeadsFile>, b: &Option<HeadsFile>| match (a, b) {
            (Some(a), Some(b)) => a.updated_at > b.updated_at,
            (Some(_), None) => true,
            _ => false,
        };
        if name != me.as_str() && newer(&ph, &lh) {
            let data = peer.get(Kind::Heads, &name)?.unwrap_or_default();
            if verify_file(Kind::Heads, &name, &data) {
                local.put(Kind::Heads, &name, &data)?;
                rep.heads_in.push(name.clone());
            } else {
                rep.rejected.push(format!("heads/{name}"));
            }
        } else if newer(&lh, &ph) {
            let data = local.get(Kind::Heads, &name)?.unwrap_or_default();
            peer.put(Kind::Heads, &name, &data)?;
            rep.heads_out.push(name);
        }
    }
    Ok(rep)
}
