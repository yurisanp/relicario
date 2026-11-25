//! Suíte de conformidade que todo [`Remote`] precisa passar.

use crate::{Kind, Remote};

/// Roda a suíte. Entra em pânico na primeira falha. O destino precisa começar vazio.
pub fn run(r: &dyn Remote) {
    for k in Kind::ALL {
        assert!(
            r.list(k).unwrap().is_empty(),
            "destino não está vazio em {k:?}"
        );
    }
    // Escrever e ler de volta logo em seguida.
    r.put(Kind::Snapshots, "aa11", b"um").unwrap();
    assert_eq!(
        r.get(Kind::Snapshots, "aa11").unwrap().as_deref(),
        Some(&b"um"[..])
    );
    assert_eq!(r.list(Kind::Snapshots).unwrap(), vec!["aa11".to_string()]);
    // Sobrescrever.
    r.put(Kind::Heads, "pc", b"v1").unwrap();
    r.put(Kind::Heads, "pc", b"v2").unwrap();
    assert_eq!(
        r.get(Kind::Heads, "pc").unwrap().as_deref(),
        Some(&b"v2"[..])
    );
    // Packs divididos em subpastas continuam listáveis.
    r.put(Kind::Packs, "ab12", b"p1").unwrap();
    r.put(Kind::Packs, "cd34", b"p2").unwrap();
    assert_eq!(
        r.list(Kind::Packs).unwrap(),
        vec!["ab12".to_string(), "cd34".to_string()]
    );
    // Config fica na raiz.
    r.put(Kind::Config, "config", b"{}").unwrap();
    assert_eq!(r.list(Kind::Config).unwrap(), vec!["config".to_string()]);
    // Ausente.
    assert!(r.get(Kind::Index, "nada").unwrap().is_none());
    r.delete(Kind::Index, "nada").unwrap();
    // Apagar.
    r.delete(Kind::Packs, "ab12").unwrap();
    assert_eq!(r.list(Kind::Packs).unwrap(), vec!["cd34".to_string()]);
    // Nomes perigosos são recusados.
    assert!(r.put(Kind::Snapshots, "../fora", b"x").is_err());
    assert!(r.put(Kind::Snapshots, ".oculto", b"x").is_err());
    // Arquivo vazio e arquivo grande.
    r.put(Kind::Index, "vazio", b"").unwrap();
    assert_eq!(
        r.get(Kind::Index, "vazio").unwrap().as_deref(),
        Some(&b""[..])
    );
    let grande = vec![7u8; 5 * 1024 * 1024];
    r.put(Kind::Packs, "ee99", &grande).unwrap();
    assert_eq!(r.get(Kind::Packs, "ee99").unwrap().unwrap(), grande);
}
