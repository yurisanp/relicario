//! Guarda contra save corrompido. O snapshot é sempre guardado; o guarda só decide se ele
//! pode virar head e ser propagado sem confirmação.

use relicario_model::{FileKind, Suspicion, Tree};

/// Limites do guarda.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuardPolicy {
    /// Encolher mais que esta porcentagem do tamanho total é suspeito.
    pub shrink_percent: u64,
    /// Saves menores que isto não disparam o aviso de encolhimento.
    pub min_size: u64,
}

impl Default for GuardPolicy {
    fn default() -> Self {
        GuardPolicy {
            shrink_percent: 80,
            min_size: 1024,
        }
    }
}

/// Compara a árvore nova com a anterior.
pub fn check(prev: Option<&Tree>, new: &Tree, policy: GuardPolicy) -> Vec<Suspicion> {
    let Some(prev) = prev else { return Vec::new() };
    let mut out = Vec::new();
    let (from, to) = (prev.total_size(), new.total_size());
    if from > 0 && (new.files.is_empty() || to == 0) {
        out.push(Suspicion::Emptied);
        return out;
    }
    if from >= policy.min_size && to * 100 < from * (100 - policy.shrink_percent) {
        out.push(Suspicion::Shrunk { from, to });
    }
    for f in &new.files {
        let Some(old) = prev.get(&f.root, &f.path) else {
            continue;
        };
        if f.kind == FileKind::Zeros && old.kind != FileKind::Zeros && old.size > 0 {
            out.push(Suspicion::Zeroed {
                path: format!("{}/{}", f.root, f.path),
            });
        } else if changed_type(old.kind, f.kind) {
            out.push(Suspicion::TypeChanged {
                path: format!("{}/{}", f.root, f.path),
            });
        }
    }
    out
}

fn changed_type(a: FileKind, b: FileKind) -> bool {
    use FileKind::*;
    let neutral = |k| matches!(k, Desconhecido | Vazio | Zeros);
    if neutral(a) || neutral(b) || a == b {
        return false;
    }
    // Texto e binário genérico se alternam em saves legítimos (por exemplo, um campo com
    // caractere de controle); só formatos reconhecidos contam.
    !(matches!(a, Texto | Binario) && matches!(b, Texto | Binario))
}

#[cfg(test)]
mod tests {
    use super::*;
    use relicario_model::{FileEntry, Hash};

    fn f(path: &str, size: u64, kind: FileKind) -> FileEntry {
        FileEntry {
            root: "save".into(),
            path: path.into(),
            size,
            mtime: 0,
            hash: Hash::of(path.as_bytes()),
            kind,
            chunks: vec![],
        }
    }

    #[test]
    fn detecta_encolhimento_zeros_e_tipo() {
        let prev = Tree::new(vec![
            f("a.sav", 10_000, FileKind::Gzip),
            f("b.sav", 10_000, FileKind::Binario),
        ]);
        let new = Tree::new(vec![
            f("a.sav", 100, FileKind::Texto),
            f("b.sav", 100, FileKind::Zeros),
        ]);
        let s = check(Some(&prev), &new, GuardPolicy::default());
        assert!(s.contains(&Suspicion::Shrunk {
            from: 20_000,
            to: 200
        }));
        assert!(s.contains(&Suspicion::Zeroed {
            path: "save/b.sav".into()
        }));
        assert!(s.contains(&Suspicion::TypeChanged {
            path: "save/a.sav".into()
        }));
        assert_eq!(
            check(Some(&prev), &Tree::default(), GuardPolicy::default()),
            vec![Suspicion::Emptied]
        );
        assert!(check(Some(&prev), &prev, GuardPolicy::default()).is_empty());
        assert!(check(None, &new, GuardPolicy::default()).is_empty());
    }
}
