# Changelog

O formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/) e as versões seguem [SemVer](https://semver.org/lang/pt-BR/). O formato do repositório (`docs/format.md`) é a versão 1 e será lido por todas as versões 1.x.

## [1.0.1] - 2026-11-25

### Corrigido
- O AppImage não abria em distribuições com glib recente (CachyOS, Arch): faltava no pacote a biblioteca da bandeja, e o app usava a do sistema, incompatível com a glib embutida. O CI agora confere que ela está no pacote.

## [1.0.0] - 2026-11-25

Primeira versão para uso real. Distribuída como AppImage (Linux x86_64) e `.tar.gz` com a linha de comando; Windows e macOS compilam a partir do código e ganham instaladores na publicação.

### Núcleo e linha de comando
- Repositório de blocos endereçado por conteúdo, com deduplicação, compressão, verificação e limpeza com retenção (`relicario snapshot`, `restore`, `log`, `verify`, `prune`).
- Cada versão guarda máquina, motivo e horário; restaurar uma versão antiga cria uma versão nova, que as outras máquinas recebem.
- Restauração protegida por diário: se o processo morrer no meio, a próxima execução termina o serviço.

### Detecção de jogos
- Lojas: Steam (inclusive Proton), Heroic, Lutris, GOG Galaxy e Epic; emuladores comuns; catálogo do Ludusavi; regras locais (`[[rules]]`).
- Linux, Windows, macOS e Steam Deck.

### Daemon
- Percebe quando um jogo abre e fecha, guarda o save ao fechar e traz a versão mais nova antes de abrir.
- `relicario run -- %command%` para as opções de inicialização dos lançadores.
- Serviço de usuário (`relicario service install`), bandeja e notificações.
- Registro local em JSONL com rotação; nenhuma telemetria.

### Sincronização
- Par a par na rede local (QUIC, pareamento por código, descoberta por mDNS), cifrado.
- Nuvem: pasta sincronizada, WebDAV, S3 e Google Drive, cifrada por padrão (senha + chave de recuperação); segredos no chaveiro do sistema.
- Conflitos nunca são resolvidos sozinhos: a interface mostra as duas versões lado a lado.

### Interface
- App Tauri com biblioteca, página do jogo com linha do tempo por máquina, conflitos, atividade, dispositivos, ajustes e assistente da primeira vez.
- Leitores de save (BG3, Hookshot, genéricos) mostram local, nível e tempo de jogo quando possível; arte da Steam e do SteamGridDB.
- Português e inglês.
- No modo Jogo do Steam Deck, roda sem janela.

### Robustez e suporte
- Bateria de caos: daemon morto no meio do envio e da restauração, escrita cortada em cada passo da replicação, relógio ±2 h, máquina offline por 30 dias.
- `relicario doctor --report` gera um relatório anonimizado.
- `relicario-restore`: binário mínimo, só leitura, que tira os saves de qualquer cópia do repositório (inclusive a cifrada) sem configuração.

### Ainda depende do dono do projeto
Veja `docs/pendencias-do-yuri.md`: client do Google Drive verificado, teste de 2 semanas nas máquinas reais, teste de usabilidade e publicação (assinatura, Flathub, AUR, winget, .dmg).

[1.0.1]: https://github.com/yurisanp/relicario/releases/tag/v1.0.1
[1.0.0]: https://github.com/yurisanp/relicario/releases/tag/v1.0.0
