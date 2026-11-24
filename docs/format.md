# Formato do repositório (v1)

> Rascunho. O formato é especificado e congelado na fase 1 do roadmap. Esta página registra as decisões já tomadas.

## Decisões já tomadas

- Blocos de tamanho variável com **FastCDC**, endereçados por **BLAKE3**.
- Compressão **zstd** por bloco antes da criptografia.
- Criptografia autenticada (AEAD) ligada por padrão para destinos de nuvem, com chave de recuperação exportável (ver [ADR 0004](adr/0004-criptografia.md)).
- Blocos agrupados em *packs* para não criar milhares de arquivos pequenos no destino.
- Snapshots apontam para o snapshot pai; os *heads* de cada máquina ficam separados dos dados, para que a troca de heads seja barata.
- Todo arquivo do repositório é imutável depois de escrito, exceto os heads, que usam escrita condicional quando o destino oferece.

## A definir na fase 1

- Layout de diretórios e nomes de arquivos.
- Formato do índice e dos manifestos de snapshot.
- Cabeçalho de versão e regra de migração.
- Algoritmo AEAD e derivação de chave.
