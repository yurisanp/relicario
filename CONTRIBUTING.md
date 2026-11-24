# Contribuindo com o Relicário

Obrigado pelo interesse. O projeto está no começo, então a melhor ajuda agora é abrir uma issue antes de um PR grande.

## Antes de abrir um PR

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check   # opcional localmente: cargo install cargo-deny
```

O CI roda tudo isso em Linux, Windows e macOS.

## Regras do código

- O princípio número 1 é **nunca perder progresso**: nenhum caminho de código apaga ou substitui um save sem guardar uma cópia antes.
- `relicario-model` e `relicario-engine` não fazem I/O nem dependem do sistema operacional. Código específico de SO fica em `relicario-platform` e `relicario-watch`.
- `unsafe` é proibido no workspace. Se um dia for necessário, vira um ADR.
- Decisões de arquitetura ficam em `docs/adr/`, numeradas.

## Licença das contribuições

Salvo indicação explícita em contrário, qualquer contribuição enviada para inclusão no projeto é licenciada como MIT OR Apache-2.0, sem termos adicionais.
