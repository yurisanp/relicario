# ADR 0001: repositório de blocos próprio, endereçado por conteúdo

- **Estado:** proposto (revisar na fase 0)
- **Data:** 2026-11-24

## Contexto

Saves vão de poucos KB (Hookshot) a centenas de MB com autosaves frequentes (BG3). Precisamos de histórico longo, envio incremental, criptografia e destinos "burros" (Drive, WebDAV, pasta) que só guardam arquivos.

## Decisão

Repositório próprio no estilo restic: FastCDC + BLAKE3 + zstd + AEAD, blocos agrupados em packs imutáveis, índice e heads separados. Especificação em [`docs/format.md`](../format.md), congelada na fase 1.

## Consequências

- Deduplicação entre versões e entre jogos; envio incremental pequeno.
- Qualquer destino que guarde arquivos serve.
- Precisamos manter o formato com versão e garantir leitura após falha no meio da escrita (critério da fase 1).
