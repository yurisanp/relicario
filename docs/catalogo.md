# Catálogo e detecção de jogos

Implementação: crate [`relicario-catalog`](../crates/relicario-catalog).

## Fontes

1. **Manifesto do Ludusavi** (gerado do PCGamingWiki, licença MIT). Baixado de `mtkennerly/ludusavi-manifest` com `ETag`, conferido no máximo uma vez por semana e guardado em cache como JSON (`<cache>/manifest/manifest.json`). Ler o cache leva cerca de 0,2 s para 53 mil jogos.
2. **Lojas e lançadores**:
   - Steam: `libraryfolders.vdf`, `appmanifest_*.acf`, `compatdata/<appid>/pfx` (Proton) e `userdata/<id>`. Linux (nativo, Flatpak, Snap), Windows (registro `SteamPath`) e macOS.
   - Heroic: GOG (`gog_store/installed.json`) e Epic via Legendary (`legendaryConfig/legendary/installed.json`), com o prefixo Wine de `GamesConfig/<app>.json`.
   - Lutris: `games/*.yml` (executável e prefixo).
   - Wine solto: `~/.wine` e `WINEPREFIX`.
   - Windows: GOG Galaxy (registro) e Epic Games Launcher (`Manifests/*.item`).
3. **Emuladores**: RetroArch, PCSX2, Dolphin, Ryujinx, DuckStation e PPSSPP, com caminhos nativos e Flatpak.
4. **Regras locais** (`[[rules]]` no `config.toml`): ignorar arquivos, ignorar o jogo, acrescentar pastas.

## Como um jogo vira raízes de save

- Cada caminho do manifesto que vale neste ambiente (`when: os/store`) é resolvido. Um jogo no Proton é resolvido como Windows dentro do prefixo e também como Linux nativo.
- Entradas marcadas só como `config` ficam de fora por padrão (costumam ser opções de vídeo da máquina).
- Sem loja detectada, só valem caminhos que não dependem da instalação (`<base>`, `<root>`, `<game>`), e a restrição de loja é ignorada: a existência da pasta é a prova.
- Nunca vira raiz a pasta do usuário inteira nem a raiz do disco.
- **Nome lógico** da raiz: a última parte fixa do caminho do manifesto, em minúsculas (`hookshot`, `playerprofiles`, `remote`). É o que casa o mesmo save entre máquinas, inclusive Linux nativo × Windows nos jogos Unity (`~/.config/unity3d/Empresa/Jogo` e `AppData/LocalLow/Empresa/Jogo` viram `jogo`). Em caso de empate, mais partes do caminho entram no nome.
- **Caminhos espelhados**: duas raízes do mesmo jogo com exatamente os mesmos arquivos, tamanhos e hashes viram uma só; a outra é restaurada junto. A principal é a que não está no `userdata/.../remote` da Steam.

## Limitações conhecidas

- Chaves de registro do Windows (`registry` no manifesto) ainda não são salvas.
- Steam Cloud vem do campo `cloud.steam` do manifesto; a leitura do `appinfo.vdf` binário fica para quando o manifesto não bastar.
- O nome lógico entre sistemas diferentes é uma heurística. Quando falhar, uma regra local com `roots` fixa o nome.

## Arte dos jogos

Ordem: SteamGridDB (com a chave do usuário e o crédito do autor) → cache local da Steam e do Lutris → CDN pública da Steam → capa gerada em SVG com o nome e uma cor estável. Tudo fica em `<cache>/art/<jogo>/` e nunca é sincronizado. `relicario art <jogo> --set arquivo.png` troca a imagem.
