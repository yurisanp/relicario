//! Caminhos espelhados: o mesmo save em dois lugares (por exemplo, o BG3 no prefixo do Proton e
//! em `userdata/<id>/1086940/remote` do Steam Cloud).
//!
//! Para não haver falso positivo, duas raízes só são espelhos quando têm exatamente os mesmos
//! arquivos relativos, com os mesmos tamanhos e o mesmo conteúdo (BLAKE3), e pelo menos um
//! arquivo.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Mapa caminho relativo → (tamanho, hash) de uma raiz.
pub type Fingerprint = BTreeMap<String, (u64, [u8; 32])>;

/// Calcula a impressão digital de uma lista de arquivos (relativo, absoluto).
pub fn fingerprint(files: &[(String, PathBuf)]) -> Option<Fingerprint> {
    let mut m = BTreeMap::new();
    for (rel, abs) in files {
        let data = std::fs::read(abs).ok()?;
        m.insert(
            rel.clone(),
            (data.len() as u64, *blake3::hash(&data).as_bytes()),
        );
    }
    Some(m)
}

/// Tamanhos, sem ler o conteúdo: um filtro barato antes de [`fingerprint`].
pub fn sizes(files: &[(String, PathBuf)]) -> BTreeMap<String, u64> {
    files
        .iter()
        .map(|(r, p)| {
            (
                r.clone(),
                std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX),
            )
        })
        .collect()
}

/// Dadas as raízes de um jogo (com os arquivos de cada uma), devolve pares (principal,
/// espelho) por índice. A principal é a que não está dentro de `userdata` da Steam, ou a
/// primeira.
pub fn detect(roots: &[(PathBuf, Vec<(String, PathBuf)>)]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut used = vec![false; roots.len()];
    for i in 0..roots.len() {
        if used[i] || roots[i].1.is_empty() {
            continue;
        }
        let si = sizes(&roots[i].1);
        let mut fi: Option<Option<Fingerprint>> = None;
        for j in i + 1..roots.len() {
            if used[j] || roots[j].1.len() != roots[i].1.len() || sizes(&roots[j].1) != si {
                continue;
            }
            let a = fi.get_or_insert_with(|| fingerprint(&roots[i].1));
            let Some(a) = a else { break };
            if fingerprint(&roots[j].1).as_ref() == Some(a) {
                let (p, m) = if is_steam_cloud(&roots[i].0) && !is_steam_cloud(&roots[j].0) {
                    (j, i)
                } else {
                    (i, j)
                };
                out.push((p, m));
                used[j] = true;
                if p == j {
                    used[i] = true;
                    break;
                }
            }
        }
    }
    out
}

fn is_steam_cloud(p: &Path) -> bool {
    let s = p.to_string_lossy().replace('\\', "/");
    s.contains("/userdata/") && s.contains("/remote")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn files(dir: &Path) -> Vec<(String, PathBuf)> {
        let mut v: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (e.file_name().into_string().unwrap(), e.path())
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn so_espelho_exato() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("pfx/Larian/Savegames");
        let b = t.path().join("userdata/1/1086940/remote");
        let c = t.path().join("outra");
        let d = t.path().join("mesmo-tamanho");
        for (dir, x) in [
            (&a, b"save1"),
            (&b, b"save1"),
            (&c, b"save2"),
            (&d, b"saveX"),
        ] {
            fs::create_dir_all(dir).unwrap();
            fs::write(dir.join("slot.lsv"), x).unwrap();
        }
        fs::write(c.join("extra"), b"1").unwrap();
        let roots = vec![
            (b.clone(), files(&b)),
            (a.clone(), files(&a)),
            (c.clone(), files(&c)),
            (d.clone(), files(&d)),
        ];
        // b (Steam Cloud) espelha a; c tem arquivo a mais; d tem mesmo tamanho mas conteúdo diferente.
        assert_eq!(detect(&roots), vec![(1, 0)]);
        assert!(detect(&[(a.clone(), vec![]), (b, vec![])]).is_empty());
    }
}
