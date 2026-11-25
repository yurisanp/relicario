//! Identificador estável de um jogo a partir do título.

/// `"Baldur's Gate 3"` → `"baldurs-gate-3"`. Acentos comuns viram a letra base.
pub fn slug(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut dash = false;
    for c in title.chars() {
        let c = fold(c);
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if c == '\'' || c == '’' || c == '™' || c == '®' {
            continue;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() { "jogo".into() } else { out }
}

fn fold(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' | 'Á' | 'À' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'a',
        'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
        'ç' | 'Ç' => 'c',
        'ñ' | 'Ñ' => 'n',
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn exemplos() {
        assert_eq!(slug("Baldur's Gate 3"), "baldurs-gate-3");
        assert_eq!(slug("Hookshot"), "hookshot");
        assert_eq!(slug("  Pokémon: Edição ™ "), "pokemon-edicao");
        assert_eq!(slug("!!!"), "jogo");
    }
}
