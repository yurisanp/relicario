# Interface

O app (`relicario-app`, crate [`relicario-ui`](../crates/relicario-ui)) é uma janela Tauri 2 com as telas do [protótipo](design/prototipo.html) e um ícone na bandeja. A interface em si é HTML, CSS e JavaScript sem etapa de build, em [`crates/relicario-ui/ui`](../crates/relicario-ui/ui).

## Como fala com o daemon

- Se o serviço do Relicário já está rodando (`relicario service install`), o app fala com ele pelo socket local.
- Se não está, o app sobe um daemon dentro do próprio processo. Fechar a janela só esconde: o app fica na bandeja vigiando os jogos. "Sair", no menu da bandeja, para tudo.
- Toda tela usa o comando `api` do Tauri, que repassa um [`Call`](../crates/relicario-daemon/src/api.rs) ao daemon (`Request::Api`). A CLI tem o mesmo caminho em `relicario api '<json>'`, usado nos testes.
- Sincronizar, procurar jogos e parear usam o comando `daemon`, que manda um `Request` direto (`{"op":"sync","game":null}`, `{"op":"pair_start","code":"123456"}`...).

## Telas

| Rota | O que mostra |
|---|---|
| `#biblioteca` | Jogos em grade ou lista, filtros, números do repositório e aviso de conflito |
| `#jogo/<id>` | Arte, linha do tempo em árvore (uma coluna por máquina), detalhe da versão com o que o leitor de save achou, arquivos e regras do jogo |
| `#conflito/<id>` | As duas versões lado a lado, a versão de onde vieram e as três saídas: seguir com uma, seguir e guardar a outra fixada, decidir depois |
| `#conflitos` | Jogos esperando decisão |
| `#atividade` | Registro do que o daemon fez, por dia |
| `#dispositivos` | Esta máquina, pareamento por código, máquinas pareadas e nuvens |
| `#ajustes` | Nome, idioma, tema, detecção, conferência periódica, porta, limite de envio, retenção, chave do SteamGridDB, jogos ignorados |

Na primeira execução (ou sem configuração), aparece o assistente: nome da máquina e pasta do repositório, ligar a outra máquina (código de pareamento, nuvem ou nenhuma), jogos encontrados e pronto.

## Restaurar uma versão antiga

Restaurar pela interface ou pela CLI (`relicario restore`) grava o conteúdo escolhido como uma versão nova, filha da atual (motivo "Volta a uma versão antiga"). Assim as outras máquinas recebem a volta como novidade. Antes, o save que estava no disco é guardado, como em toda restauração.

## Idiomas

Português do Brasil e inglês, em [`ui/i18n.js`](../crates/relicario-ui/ui/i18n.js). O idioma segue o sistema, ou o escolhido em Ajustes. O CI roda [`ui/check.mjs`](../crates/relicario-ui/ui/check.mjs), que confere a sintaxe dos scripts e se toda chave usada existe nos dois idiomas.

## Compilar

No Linux, a interface precisa de `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libxdo-dev` e `libssl-dev`. No Windows, do WebView2 (já vem no Windows 11). No macOS, nada além do Xcode.

```sh
cargo run -p relicario-ui            # abre a janela
cargo run -p relicario-ui -- --hidden  # começa só na bandeja
```
