# Leitores de save

Um leitor tira de um snapshot o que ajuda a reconhecer o ponto do jogo: local, personagem,
nível, slot, tempo de jogo e uma miniatura. Ele só lê, nunca muda o save, e roda na hora de
mostrar o snapshot (nada disso vai para o repositório).

A interface é pequena para virar WASM de terceiros depois da 1.0 (`crates/relicario-plugins`):

```rust
pub trait Reader: Send + Sync {
    fn name(&self) -> &'static str;
    fn matches(&self, game: &str, title: &str) -> bool;
    fn read(&self, files: &[FileInfo], fetch: &mut Fetch<'_>) -> Option<SaveMeta>;
}
```

`files` lista os arquivos do snapshot (`raiz/caminho`, tamanho e modificação) e `fetch` lê o
conteúdo de um deles, conferido pelo hash. `SaveMeta` traz pares `(campo, valor)` com nomes
fixos (`titulo`, `local`, `personagem`, `nivel`, `tempo_de_jogo`, `slot`, `dificuldade`,
`versao_do_jogo`) mais os que o leitor quiser, e a miniatura.

## Embutidos

| Leitor | Jogo | O que lê |
|---|---|---|
| BG3 | Baldur's Gate 3 (Steam 1086940, GOG 1456460669) | O `.lsv` mais recente: pacote LSPK 18 com o `SaveInfo.json` (fase, grupo, nível, dificuldade, versão) e a miniatura `.WebP` ao lado |
| Hookshot | Hookshot | O `caption.sav` do slot jogado por último: JSON, texto `chave=valor` ou textos e imagem embutidos. O formato exato ainda não foi confirmado |
| genérico | Todos | JSON e `chave=valor` com nomes conhecidos (`level`, `location`, `playtime`...) e a primeira imagem (`.png`, `.jpg`, `.webp` ou embutida em `thumb`, `caption`, `screenshot`) |

O primeiro que serve para o jogo e acha algo ganha.

## LSPK

Pacotes da Larian (`.lsv`, `.pak`), versão 18:

```text
"LSPK" | versão u32 | posição da lista u64 | tamanho da lista u32 | flags u8 | prioridade u8
       | md5 [16] | partes u16
lista: quantidade u32 | tamanho comprimido u32 | LZ4 (bloco) com quantidade × 272 bytes:
       nome [256] | posição baixa u32 | posição alta u16 | parte u8 | flags u8
       | tamanho no disco u32 | tamanho descomprimido u32
```

Compressão de cada arquivo em `flags & 0x0f`: 0 nenhuma, 1 zlib, 2 LZ4, 3 zstd.
