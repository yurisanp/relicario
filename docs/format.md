# Formato do repositório (v1)

Este documento descreve o formato que a versão 1 do Relicário lê e escreve. Ele foi pensado para funcionar em qualquer destino "burro" (uma pasta, um HD externo, Google Drive, S3), sem servidor, com várias máquinas escrevendo ao mesmo tempo.

Implementação de referência: crate [`relicario-store`](../crates/relicario-store).

## Princípios

1. **Tudo é imutável, exceto `heads/<máquina>`**, e esse arquivo só é escrito pela própria máquina. Duas máquinas enviando ao mesmo tempo nunca se sobrescrevem.
2. **Endereçado por conteúdo.** Blocos, árvores, packs, índices e snapshots são identificados pelo BLAKE3 do conteúdo. O mesmo arquivo em duas máquinas vira o mesmo bloco.
3. **Ordem de escrita segura.** Pack → índice → snapshot → heads. Uma queda no meio deixa no máximo arquivos órfãos, nunca um snapshot que aponta para dados ausentes.
4. **Escrita atômica.** Todo arquivo é escrito num temporário (`.tmp-*`) e renomeado. Leitores ignoram temporários.

## Layout

```
repo/
  config                 # parâmetros do repositório (JSON)
  keys/<id>              # chaves (fase 5; vazio enquanto a criptografia é "none")
  packs/<ab>/<hash>      # blocos agrupados, imutáveis; subpasta = 2 primeiros caracteres
  index/<hash>           # onde cada bloco está, imutável
  snapshots/<hash>       # metadados de snapshot (JSON), imutável
  heads/<máquina>        # estado de cada máquina (JSON), mutável só pela dona
  locks/<máquina>-<modo> # travas (JSON), com validade
```

Nomes de arquivo aceitos: letras, números, `-`, `_` e `.`, sem começar com `.`. Hashes são BLAKE3 de 32 bytes em hexadecimal minúsculo (64 caracteres).

## `config`

```json
{
  "format_version": 1,
  "id": "9f1c…",                 // 16 bytes aleatórios em hex
  "created_at": 1795996800000,   // milissegundos Unix (UTC)
  "chunker": { "min": 16384, "avg": 65536, "max": 262144 },
  "compression_level": 3,
  "encryption": "none",
  "pack_size": 8388608
}
```

Uma build que encontra `format_version` desconhecido recusa abrir o repositório. Mudanças de formato aumentam a versão e trazem uma migração explícita.

## Blocos

- Cada arquivo de save é cortado com **FastCDC (variante 2020)** usando os parâmetros de `chunker`.
- O id do bloco é o **BLAKE3 do conteúdo em claro**.
- O bloco é guardado comprimido com **zstd** no nível `compression_level`.
- Na leitura, o bloco é descomprimido e o hash é conferido. Bloco que não confere é erro.

## Packs

Um pack é a concatenação de blocos comprimidos, precedida do cabeçalho mágico de 8 bytes `RLCPACK1`. O nome do pack é o BLAKE3 do arquivo inteiro. Packs são fechados ao chegar perto de `pack_size` e no fim de cada snapshot.

## Índice

`index/<hash>`, onde o hash é o BLAKE3 do próprio arquivo:

```json
{
  "version": 1,
  "packs": [
    { "id": "<pack>", "blobs": [ { "id": "<bloco>", "offset": 8, "length": 1234, "raw_length": 65536 } ] }
  ]
}
```

`offset` e `length` se referem ao bloco comprimido dentro do pack; `raw_length` é o tamanho em claro. O índice em memória é a união de todos os arquivos de índice. Um bloco pode aparecer em mais de um pack (duas máquinas enviando o mesmo dado); qualquer cópia serve.

## Árvore

A lista de arquivos de um snapshot é um blob como qualquer outro (JSON, comprimido, num pack):

```json
{
  "files": [
    {
      "root": "save",                 // nome lógico da raiz, igual em todas as máquinas
      "path": "slot1/caption.sav",    // relativo à raiz, sempre com "/"
      "size": 1234,
      "mtime": 1795996800000,
      "hash": "<blake3 do arquivo inteiro>",
      "kind": "binario",              // formato detectado, usado pelo guarda
      "chunks": ["<bloco>", "<bloco>"]
    }
  ]
}
```

Os arquivos ficam ordenados por `(root, path)`. Caminhos com `..`, `.`, partes vazias, `\` ou `:` são recusados na restauração.

O **hash de conteúdo** de uma árvore é o BLAKE3 de `relicario-content-v1\0` seguido, para cada arquivo em ordem, de `root\0path\0` + tamanho (u64 little-endian) + hash do arquivo. Ele ignora horários: dois saves iguais em máquinas diferentes têm o mesmo hash de conteúdo, e o motor trata isso como "igual".

## Snapshot

`snapshots/<id>`:

```json
{
  "game": "hookshot",
  "machine": "pc-1a2b3c4d",
  "parents": ["<snapshot>"],     // vazio no primeiro; dois ou mais num merge
  "tree": "<blob da árvore>",
  "content": "<hash de conteúdo>",
  "created_at": 1795996800000,
  "mtime_max": 1795996790000,
  "size": 1234,
  "files": 3,
  "reason": "game_exit",         // manual | game_exit | periodic | before_restore | merge | import
  "suspicions": [],              // avisos do guarda contra save corrompido
  "meta": {}                     // metadados de plugins de leitura de save
}
```

O id é o BLAKE3 de `relicario-snapshot-v1\0` seguido de uma serialização canônica dos campos (veja `Snapshot::id` em `relicario-model`). Na leitura o id é recalculado e conferido.

## Heads

`heads/<máquina>`:

```json
{
  "machine": "pc-1a2b3c4d",
  "machine_name": "PC",
  "heads": { "hookshot": { "snapshot": "<id>", "updated_at": 1795996800000 } },
  "pins": { "<id>": "antes do chefe" },
  "forgotten": ["<id>"],
  "updated_at": 1795996800000
}
```

- `heads`: o último snapshot de cada jogo nesta máquina.
- `pins`: snapshots fixados, com nome. Fixados nunca são liberados.
- `forgotten`: snapshots **desta máquina** cujos dados a retenção liberou. Os metadados continuam em `snapshots/`, para a árvore de ancestralidade continuar inteira.
- `playtime`, `running`, `associations` (opcionais): tempo de jogo em ms, jogos abertos agora (com o horário de abertura) e executáveis associados à mão.
- `addresses` (opcional): `host:porta` onde a máquina aceita sincronização direta. É o "cartão de visita": chega às outras máquinas por qualquer cópia do repositório.
- `updated_at` sempre cresce (nunca volta, mesmo com o relógio voltando). Entre duas cópias, vale o maior.

## Replicação

Duas cópias do mesmo repositório (outra máquina, nuvem) são juntadas copiando o que falta nos dois sentidos, na ordem `keys` → `packs` → `index` → `snapshots` → `heads`. Cada arquivo é conferido antes de ser gravado: packs e índices pelo BLAKE3 do conteúdo, snapshots pelo id recalculado, heads pelo campo `machine`. Para cada `heads/<máquina>`, fica o de `updated_at` maior, e o head da própria máquina nunca é trocado pelo que veio de fora. `config` e `locks/` não viajam.

Duas cópias com `id` diferente podem ser juntadas quando ambas não têm criptografia e usam os mesmos parâmetros de chunking (os nomes dos blocos são iguais nos dois).

## Travas

`locks/<máquina>-shared` ou `locks/<máquina>-exclusive`, com `expires_at` 30 minutos à frente. Snapshots e restaurações pegam trava compartilhada; `prune` pega a exclusiva e falha se houver qualquer outra trava válida. Travas vencidas (processo que morreu) são ignoradas.

Em destinos sem escrita condicional, duas travas exclusivas simultâneas são detectadas relendo a pasta depois de escrever; a janela de corrida que sobra é aceitável porque o `prune` é raro e só roda numa máquina por vez.

## Retenção e prune

A política padrão (crate `relicario-engine`, módulo `retention`):

- tudo das últimas 24 horas;
- o mais recente de cada dia nos últimos 30 dias;
- o mais recente de cada período de 30 dias no último ano;
- heads de qualquer máquina, fixados e o mais recente de cada jogo nunca saem.

`relicario prune` marca os snapshots fora da política em `forgotten` e então coleta o lixo: blocos que nenhum snapshot ativo usa são removidos; packs parcialmente usados são reescritos. Ordem: novos packs → índice consolidado → apagar índices antigos → apagar packs antigos. Packs sem índice (escrita interrompida) também são removidos.

## Regras de segurança implementadas sobre o formato

- **Nunca restaurar sem cópia:** antes de restaurar, se o conteúdo atual do disco não está em nenhum snapshot ativo do jogo, um snapshot `before_restore` é gravado.
- **Restauração conferida:** cada arquivo é escrito num temporário, conferido pelo hash e só então renomeado; no fim, o disco é relido e comparado com a árvore.
- **Guarda contra save corrompido:** um snapshot que encolheu mais de 80 %, ficou vazio, tem arquivo que virou só zeros ou mudou de formato é guardado, mas não vira head sem `--force`.

## A definir na fase 5

- `encryption: "xchacha20poly1305"`: cada blob, índice, snapshot e heads cifrado individualmente; chave mestra em `keys/` cifrada por Argon2id(senha) e pela chave de recuperação. Veja o [ADR 0004](adr/0004-criptografia.md).
