# ADR 0003: interface em Tauri 2

- **Estado:** proposto (revisar na fase 0)
- **Data:** 2026-11-24

## Contexto

O projeto quer ser agnóstico a plataforma (Linux, Windows, macOS, Steam Deck) e o núcleo é Rust.

## Decisão

Interface em **Tauri 2** (Rust + front web), com janela e ícone na bandeja, falando com o daemon pela API JSON-RPC local. Kirigami foi descartado por prender o app ao Linux.

## Consequências

- Mesmo código de interface nos três sistemas; reaproveita os tipos de `relicario-model`.
- Depende de WebView do sistema (WebKitGTK no Linux, WebView2 no Windows).
- O app Tauri entra no workspace na fase 6. Até lá, `relicario-ui` só reserva o nome, para o CI não precisar das dependências de sistema do WebView.
