# Relicário: roadmap para um app robusto

Escrito em 24/11/2026, a partir do plano `relicario-plano.html`. O plano original resolve o caso PC ⇄ notebook no CachyOS. Este roadmap mantém tudo o que ele decidiu e leva o app até uma versão 1.0 **agnóstica a plataforma** (Linux, Windows, macOS e Steam Deck), robusta o bastante para outras pessoas usarem.

---

## 1. Visão em uma frase

O Relicário guarda cada versão dos saves dos seus jogos, sabe sozinho qual é a mais atual entre várias máquinas e nunca sobrescreve progresso sem guardar uma cópia antes. É o Steam Cloud para os jogos que não têm Steam Cloud, com histórico de verdade.

## 2. Princípios (inegociáveis)

1. **Nunca perder progresso.** Toda substituição guarda o estado anterior no histórico. Nenhum caminho de código apaga um save sem cópia.
2. **Ancestralidade, não relógio.** A decisão vem da árvore de snapshots. O horário só serve para exibir e desempatar.
3. **Zero configuração por jogo** nos casos comuns (Steam, Proton, Heroic, Lutris, GOG, Epic, emuladores). Casos especiais entram com um clique.
4. **Invisível no dia a dia.** A interface aparece só quando há decisão a tomar.
5. **Rede primeiro, nuvem depois.** Máquina a máquina quando as duas estão ligadas; nuvem como cofre e caixa de correio.
6. **Núcleo portável, bordas por plataforma.** Tudo que decide fica num núcleo sem dependência de SO; só detecção, vigia de processos, segredos, serviço e bandeja variam.
7. **Formato aberto e versionado.** O repositório de saves é documentado e lido por uma CLI independente, para que o usuário nunca fique preso ao app.

## 3. Decisões em aberto: recomendação deste roadmap

O plano deixou seis decisões em aberto. Abaixo está a recomendação usada no restante do documento. Qualquer uma pode ser revista sem quebrar as fases.

| Decisão | Recomendação | Por quê |
|---|---|---|
| Repositório | GitHub, público, licença MIT ou Apache-2.0 | O manifesto do Ludusavi é MIT e o ecossistema Rust/Tauri vive no GitHub; CI gratuito com runners de Linux, Windows e macOS |
| Interface | **Tauri 2** (Rust + front web) | É o único caminho que cobre Linux, Windows e macOS com o mesmo código e reaproveita os tipos do daemon. Kirigami ficaria bonito no KDE, mas prenderia o app ao Linux |
| Windows | Entra na 1.0 (fase 2 em diante) | O projeto se define como agnóstico a plataforma, e Windows é onde a maioria dos jogos sem nuvem roda |
| Criptografia | Ligada por padrão para destinos de nuvem, com **chave de recuperação** impressa/exportada na configuração | Protege o Drive; a chave de recuperação resolve "perdi a senha" |
| Jogos com Steam Cloud | Padrão "só histórico local"; envio à nuvem do Relicário opcional por jogo | Evita dobrar espaço e briga com o Steam Cloud |
| Backups antigos do Ludusavi | Importar na fase 5 e só depois oferecer apagar | Ninguém perde o que já existe; o importador vira recurso para novos usuários vindos do Ludusavi |

## 4. Arquitetura alvo

```
┌──────────────────────────── cada máquina ────────────────────────────┐
│ relicario-daemon (serviço do usuário)                                 │
│  ├─ catalog    manifesto Ludusavi + detectores por loja + regras      │
│  ├─ watch      vigia de processos (por SO)                            │
│  ├─ engine     snapshots, árvore, decisão (núcleo puro, sem I/O)      │
│  ├─ store      repositório de blocos (FastCDC + BLAKE3 + zstd + AEAD)│
│  ├─ sync       troca de "heads" e blocos entre máquinas e nuvem       │
│  └─ transport  p2p (QUIC) │ Google Drive │ WebDAV │ S3 │ SFTP │ pasta │
│  IPC local: socket Unix (Linux/macOS) / named pipe (Windows), JSON-RPC│
│ relicario (CLI)            relicario-ui (Tauri: janela + bandeja)     │
└───────────────────────────────────────────────────────────────────────┘
```

### 4.1 Crates (workspace Rust)

| Crate | Responsabilidade | Depende de SO? |
|---|---|---|
| `relicario-model` | Tipos: `Game`, `Machine`, `Snapshot`, `Head`, eventos, erros | Não |
| `relicario-engine` | Motor de decisão (igual / avanço / envio / divergiu), retenção, proteção contra save corrompido | Não |
| `relicario-store` | Repositório de blocos endereçado por conteúdo, packs, índice, GC, verificação | Não |
| `relicario-catalog` | Leitura do manifesto, resolução de placeholders, detectores de loja, caminhos espelhados | Parcial (detectores) |
| `relicario-watch` | Vigia de processos e de arquivos, "espera terminar de gravar" | Sim |
| `relicario-transport` | Trait `Remote` e implementações (p2p, Drive, WebDAV, S3, SFTP, pasta local) | Não |
| `relicario-platform` | Caminhos, serviço, segredos, notificações, lançadores | Sim |
| `relicario-daemon` | Orquestração, filas, travas por jogo, SQLite, API JSON-RPC | Não |
| `relicario-cli` | `snapshot`, `plan`, `restore`, `log`, `run`, `doctor`, `export` | Não |
| `relicario-ui` | App Tauri | Não |
| `relicario-plugins` | Leitores de save (Hookshot, BG3…) via interface estável; WASM a partir da 1.x | Não |

### 4.2 Camada de plataforma

| Ponto | Linux / Steam Deck | Windows | macOS |
|---|---|---|---|
| Serviço | systemd `--user` | Tarefa agendada no logon (sem admin) | LaunchAgent |
| Vigia de processos | `/proc` + netlink proc connector (fallback: polling 2 s) | WMI `Win32_ProcessStartTrace` / ETW (fallback: polling) | `NSWorkspace` + `kqueue` |
| Vigia de arquivos | inotify | `ReadDirectoryChangesW` | FSEvents |
| Segredos | Secret Service (KWallet/GNOME Keyring) | Credential Manager | Keychain |
| Notificações | D-Bus (KDE/GNOME) | Toast | UserNotifications |
| Caminhos do manifesto | XDG + prefixos Wine/Proton | `%APPDATA%`, Documentos, registro | `~/Library/...` |
| Pacotes | AppImage, Flatpak, AUR, .deb/.rpm | MSI + winget | .dmg assinado |

A crate `keyring` cobre os três chaveiros. `notify-rust`/`tauri-plugin-notification` cobrem notificações. A bandeja vem do Tauri.

### 4.3 Modelo de dados

```
machine   { id: ed25519 pubkey, nome, so, criado_em }
game      { id: slug estável, nome, fontes: [steam:1086940, gog:…], regras }
snapshot  { id: blake3(conteudo+meta), jogo, maquina, pai: [ids], arvore: hash,
            criado_em, mtime_max, tamanho, metadados (plugin), fixado, etiqueta }
head      { jogo, maquina, snapshot, atualizado_em }   # um por jogo por máquina
```

- `pai` é uma lista para registrar **merges** (quando o usuário resolve um conflito "mantendo as duas como slots", o novo snapshot tem dois pais).
- Os IDs são hashes, então o mesmo conteúdo vindo de máquinas diferentes vira o mesmo blob e o resultado "igual" sai de graça.
- Estado local em SQLite (WAL), com migrações versionadas. O SQLite é **cache reconstruível**: a fonte da verdade é o repositório.

### 4.4 Formato do repositório (local e remoto)

Inspirado no restic, para funcionar em qualquer destino "burro" (pasta, Drive, S3) sem servidor:

```
repo/
  config            # versão do formato, parâmetros de chunking, salt
  keys/<id>         # chave mestra cifrada pela senha e pela chave de recuperação
  packs/<hash>      # blocos agrupados (~4–16 MB), imutáveis
  index/<hash>      # quais blocos estão em quais packs, imutável
  snapshots/<hash>  # metadados de snapshot, imutável
  heads/<machine>   # único arquivo mutável, e só a própria máquina escreve nele
  locks/<machine>   # só para prune/GC, com expiração
```

Como **cada máquina só escreve no próprio `heads/`** e todo o resto é imutável, não há corrida na nuvem: duas máquinas enviando ao mesmo tempo nunca se sobrescrevem. Esse é o defeito do Ludusavi ("a sincronização sobrescreve a pasta inteira") resolvido no formato.

### 4.5 Protocolo de sincronização

1. Cada máquina publica seu `head` por jogo (no par e na nuvem).
2. Ao abrir um jogo, o daemon lê os heads das outras máquinas (par direto com limite de 3 s; nuvem em seguida, também com limite) e calcula a relação na árvore.
3. **avanço** → baixa só os blocos que faltam, faz snapshot do estado local (cópia de segurança) e restaura.
4. **envio** → nada a fazer antes; ao fechar, envia blocos novos e atualiza o head.
5. **divergiu** → não restaura; guarda as duas pontas e abre a tela de conflito.
6. Pai desconhecido (snapshot ainda não chegou) → tratar como divergência até a história completar, nunca como avanço.

### 4.6 Transporte par-a-par sem depender do Tailscale

O plano usa Tailscale, que continua sendo o caminho recomendado. Para ser agnóstico:

- QUIC (`quinn`) com TLS mútuo pelas chaves ed25519 das máquinas.
- Pareamento por código de 6 dígitos ou QR, uma vez por par.
- Descoberta: mDNS na rede local, MagicDNS do Tailscale quando existir, e um "cartão de visita" (endereços conhecidos) gravado na nuvem como caixa de correio.
- Sem servidor próprio do projeto, nem relay. Se o par não responde, a nuvem resolve.

## 5. O que reaproveitar do Ludusavi

| Item | Como usar |
|---|---|
| **Manifesto** (`ludusavi-manifest`, gerado do PCGamingWiki, MIT) | Fonte principal de caminhos de save. Baixar semanalmente com ETag, cache local, atribuição no "Sobre" |
| Placeholders (`<base>`, `<home>`, `<storeUserId>`, `<winAppData>`, `<winDocuments>`, `<xdgData>`, `<xdgConfig>` etc.) | Implementar o mesmo resolvedor, incluindo a tradução de caminhos Windows para dentro de prefixos Wine/Proton |
| Campos `when` (os/store), `tags` (save/config), `registry`, `installDir`, `steam.id`, `gog.id`, `cloud` | `tags: config` vira "não sincronizar por padrão" (resolve o `hookshot_options.ini`); `cloud` indica quem já tem nuvem da loja, antes mesmo de ler o `appinfo.vdf` |
| Formato de backup (`mapping.yaml`) | Importador na fase 5 |
| Jogos personalizados do usuário (`config.yaml`) | Importar as regras como jogos locais |
| Modelos de interface | Lista de jogos com busca, filtros e caixas de seleção; modos Prévia / Backup / Restaurar; editor de jogo personalizado; tela de "outros" ajustes. A thread de design pode usar esses padrões como ponto de partida e melhorar onde o Ludusavi é fraco (linha do tempo, conflito, estado de várias máquinas) |

Na fase 0 confirmar a especificação atual do manifesto (campos e placeholders) no repositório oficial, porque ela evolui.

## 6. Fases e marcos

Estimativas em semanas de trabalho focado de uma pessoa. Cada fase termina num marco com **critérios de pronto** verificáveis. Nenhuma fase começa sem a anterior passar nos critérios.

### Fase 0: Fundação (1 semana)

- Repositório, workspace Rust, CI no GitHub Actions com matriz Linux / Windows / macOS (fmt, clippy, testes, `cargo deny`).
- ADRs curtos: formato do repositório, protocolo de sync, Tauri, criptografia.
- Verificar Ludusavi, Syncthing, restic e sync do Heroic nas duas máquinas (do plano).
- Estudar `caption.sav` e `progress.sav` do Hookshot e o `.lsv` do BG3.
- Criar o client OAuth próprio no Google Cloud (escopo `drive.file`) e iniciar a verificação do app no Google, que pode demorar semanas.

**Pronto quando:** CI verde nos três SOs; ADRs revisados; client OAuth criado; notas de formato do Hookshot salvas.

### Fase 1: Núcleo e CLI (3 semanas) → **M1 "Funciona na CLI"**

- `relicario-store`: FastCDC, BLAKE3, zstd, packs, índice, verificação, GC com trava.
- `relicario-engine`: snapshots com pai, as 4 relações, retenção (24 h / 30 dias / 1 ano), fixar, proteção contra save corrompido (encolheu > 80 %, zerou, mudou de tipo).
- Formato do repositório v1 documentado em `docs/format.md`.
- CLI: `init`, `snapshot`, `plan`, `restore`, `log`, `pin`, `verify`, com o Hookshot cadastrado à mão.
- Destino "pasta local" (serve para testes e para HD externo).

**Pronto quando:** testes de propriedade com 10 000 históricos aleatórios em 3 máquinas sem perder snapshot e sem restauração sem cópia; restauração idêntica byte a byte; snapshot do Hookshot < 1 s; repositório lido de volta após matar o processo no meio da escrita.

### Fase 2: Catálogo e detecção multiplataforma (3 semanas)

- Resolvedor de manifesto e placeholders; cache e atualização semanal.
- Detectores: Steam (`libraryfolders.vdf`, `appmanifest_*.acf`, `compatdata`), Steam Cloud (`appinfo.vdf` + campo `cloud`), Heroic, Lutris, Wine solto, GOG Galaxy e Epic no Windows, Steam no macOS.
- Emuladores: RetroArch, PCSX2, Dolphin, Ryujinx, DuckStation, PPSSPP.
- Caminhos espelhados (mesmo conteúdo em dois lugares, ex.: BG3 Proton + `userdata/…/remote`).
- Regras locais (Hookshot: incluir `save/`, ignorar `save-backups/` e `hookshot_options.ini`).
- `relicario scan` e `relicario doctor`.
- **Arte dos jogos** (capa, banner, ícone, logo), em ordem de fonte:
  1. **SteamGridDB** (fonte principal): API gratuita com chave de API por usuário, busca por ID da Steam/GOG/Epic ou por nome; guardar e **mostrar o crédito do autor** de cada imagem e respeitar os limites de uso da API.
  2. Cache local das lojas: `appcache/librarycache` da Steam, imagens do Heroic e do Lutris (sem rede, instantâneo).
  3. CDN pública da Steam (capa e header pelo `appid`) para jogos da Steam sem cache local.
  4. Sem imagem: capa gerada com o nome e a cor do jogo.
  - As imagens ficam em cache local (`~/.cache/relicario/art`, equivalente por SO) e **não são sincronizadas** entre máquinas nem enviadas à nuvem. O usuário pode trocar a capa de um jogo pela interface.

**Pronto quando:** `scan` encontra os jogos da matriz real nas duas máquinas Linux e numa VM Windows; prévia de todos os jogos < 5 s; zero falso positivo de espelho nos testes.

### Fase 3: Daemon e ciclo do jogo (3 semanas) → **M2 "Invisível no Linux"**

- Daemon com API JSON-RPC, filas e **trava por jogo** (envio, prévia e restauração de jogos diferentes em paralelo).
- Vigia de processos Linux e Windows; "espera terminar de gravar" (5 s sem mudança + arquivo não travado).
- Lançador `relicario run -- %command%` e instalação automática: `localconfig.vdf` da Steam (com a Steam fechada e backup), `.desktop`, Steam ROM Manager, Heroic, Lutris.
- Conferência periódica a cada hora; tempo de jogo por máquina.
- **Associar apps desconhecidos a um jogo.** Quando o vigia vê um processo que não está no catálogo e que grava em pastas de save conhecidas (ou que o usuário escolhe na lista "abertos agora"), o app oferece "Este app é um jogo?" com três saídas: associar a um jogo existente, criar um jogo novo a partir dele, ou ignorar sempre. A associação guarda o executável (caminho, nome e hash opcional) e vale para lançadores, builds próprios, versões portáteis, jogos piratas/DRM-free soltos, mods e emuladores com nome trocado. Um jogo pode ter vários executáveis, e a regra vai junto com o jogo para as outras máquinas (o caminho é ajustado por máquina).
- Notificações nativas; aviso "jogo aberto em outra máquina".

**Pronto quando:** teste com jogo falso (lê ao abrir, grava aos poucos, grava ao abrir o menu) passa nos 3 caminhos de abertura do Hookshot; daemon em repouso < 30 MB de RAM e < 0,5 % de CPU; nenhuma restauração com o jogo aberto.

### Fase 4: Sincronização par-a-par (2 semanas)

- Transporte QUIC, pareamento, descoberta (mDNS, Tailscale, cartão na nuvem).
- Troca de heads e de blocos faltantes; retomada de envio interrompido.
- Presença ("Hookshot aberto no PC").

**Pronto quando:** teste com dois contêineres: jogar em A, abrir em B e ver avanço; jogar nas duas offline e reconectar e ver divergência; derrubar a rede no meio do envio e ver retomada sem bloco corrompido.

### Fase 5: Nuvem, segurança e importação (3 semanas) → **M3 "Cofre externo"**

- Destinos: Google Drive (client próprio, login pelo navegador com PKCE e loopback), WebDAV/Nextcloud, S3, SFTP. Usar `opendal` onde couber.
- Criptografia: XChaCha20-Poly1305, chave mestra cifrada por Argon2id(senha) e pela chave de recuperação; senha no chaveiro do SO.
- Limite de banda, envio só com a máquina ociosa, prioridade de I/O baixa.
- Modo "só histórico" para jogos com Steam Cloud.
- Importação dos backups e jogos personalizados do Ludusavi.
- Conferência semanal: baixa, confere hashes e restaura numa pasta temporária.
- Alerta claro quando o token expira ou é revogado.

**Pronto quando:** envio incremental do BG3 após autosave < 10 % do tamanho do save; repositório no Drive ilegível sem a chave; token revogado gera alerta em até 1 h; importação dos 2,7 GB do Ludusavi sem perder nenhum arquivo (conferido por hash).

### Fase 6: Interface (4 semanas) → **M4 "Beta fechado"**

Trabalho em paralelo com a thread de design (veja a seção 10).

- Bandeja com estado por máquina e por jogo.
- Biblioteca de jogos, detalhe do jogo com linha do tempo, restauração com um clique, fixar com nome.
- Tela de conflito lado a lado (tempo de jogo, local, slot, miniatura) com "manter esta", "manter as duas como slots", "decidir depois".
- Assistente de primeira execução e de máquina nova ("formatar o notebook vira 2 minutos").
- Configurações: destinos, criptografia, banda, retenção, jogos ignorados.
- Plugins de leitura de save: Hookshot e BG3.
- Seletor de arte por jogo (escolher outra capa do SteamGridDB, com o crédito do autor visível) e campo para a chave de API nas configurações.
- Acessibilidade (teclado, leitores de tela, contraste AA) e i18n (pt-BR e en desde o início).

**Pronto quando:** as 5 tarefas centrais (configurar, ver histórico, restaurar ponto antigo, resolver conflito, configurar máquina nova) são feitas sem ajuda por 3 pessoas num teste de usabilidade; a UI não trava durante envio de 2 GB.

### Fase 7: Robustez e distribuição (3 semanas) → **M5 "1.0"**

- Testes de caos: matar o daemon no meio de envio e restauração, disco cheio, token revogado, save travado, relógio ±2 h, máquina offline 30 dias.
- Teste de longa duração: 2 semanas de uso real nas duas máquinas sem intervenção.
- Pacotes: AppImage, Flatpak, AUR, MSI/winget, .dmg; assinatura de código; atualização automática do Tauri com assinatura.
- Steam Deck: modo Jogo, sem janela, notificações.
- Logs estruturados locais, `relicario doctor --report` para gerar relatório anonimizado. Sem telemetria por padrão.
- Documentação: guia do usuário, formato do repositório, como escrever regra e plugin.

**Pronto quando:** zero perda de progresso em toda a bateria de caos e no teste de 2 semanas; instaladores funcionam em máquina limpa dos 3 SOs; `relicario-restore` (binário mínimo, só leitura) recupera um repositório sem o daemon.

### Depois da 1.0

- Plugins WASM de terceiros para ler saves; regras compartilhadas pela comunidade.
- Android (emuladores como RetroArch/Dolphin) via cliente leve.
- Restauração parcial (um slot específico).
- Compartilhar um save com amigo (exportação `.zip` assinada).
- Integração com Decky Loader no Steam Deck.

### Cronograma resumido

| Fase | Semanas | Acumulado | Marco |
|---|---|---|---|
| 0 Fundação | 1 | 1 | |
| 1 Núcleo e CLI | 3 | 4 | M1 Funciona na CLI |
| 2 Catálogo | 3 | 7 | |
| 3 Daemon e ciclo | 3 | 10 | M2 Invisível no Linux |
| 4 Par-a-par | 2 | 12 | |
| 5 Nuvem e segurança | 3 | 15 | M3 Cofre externo |
| 6 Interface | 4 | 19 | M4 Beta fechado |
| 7 Robustez | 3 | 22 | M5 1.0 |

Cerca de 22 semanas focadas (5 a 6 meses). O uso pessoal PC ⇄ notebook já fica bom no M2/M3 (semana 10 a 15). As fases 2 e 6 podem começar parcialmente antes, porque só dependem do modelo e da API.

## 7. Requisitos não funcionais

| Área | Meta |
|---|---|
| Atraso ao abrir jogo | ≤ 3 s no pior caso (limite de consulta), < 300 ms quando nada mudou |
| Daemon em repouso | < 30 MB RAM, < 0,5 % CPU, nenhuma escrita de disco |
| Snapshot | Hookshot < 1 s; BG3 (25 MB) < 3 s |
| Envio incremental | < 10 % do tamanho do save após um autosave |
| Prévia de todos os jogos | < 5 s |
| Durabilidade | Escrita atômica (arquivo temporário + `fsync` + rename); verificação semanal |
| Compatibilidade | Formato v1 lido por todas as versões 1.x; migrações só para frente com backup |

## 8. Segurança e privacidade

- Chaves de máquina ed25519 geradas no primeiro uso, guardadas no chaveiro do SO.
- OAuth com PKCE e escopo mínimo (`drive.file`: o app vê só o que criou).
- Nuvem só recebe blocos cifrados e nomes por hash (sem nomes de jogos visíveis).
- A chave do SteamGridDB fica no chaveiro do SO; as buscas de arte enviam só o ID ou o nome do jogo, nunca dados do usuário.
- Sem telemetria; relatórios de erro só por ação do usuário e anonimizados.
- Dependências auditadas com `cargo deny`/`cargo audit` no CI.
- O lançador nunca roda comandos vindos da rede; o par só troca dados, nunca instruções de execução.

## 9. Riscos e mitigação

| Risco | Prob. | Impacto | Mitigação |
|---|---|---|---|
| Bug no motor de decisão perde progresso | Média | Crítico | Núcleo puro, testes de propriedade, cópia antes de toda restauração, modo `plan` |
| Verificação do Google para o client OAuth demora ou é negada | Alta | Alto | Começar na fase 0; escopo `drive.file` (menos exigências); WebDAV/S3/pasta como alternativa; usuário pode usar o próprio client |
| Jogo regrava save ao abrir o menu e gera falsa divergência | Alta | Médio | Ignorar mudanças só de mtime; "igual" por hash; regras `ignore` por jogo; janela de silêncio após restaurar |
| Editar `localconfig.vdf` quebra opções da Steam | Média | Médio | Só com a Steam fechada, backup e verificação; fallback pelo vigia de processos |
| Vigia de processos no Windows exige permissões | Média | Médio | WMI sem admin; fallback por polling; lançador como caminho principal |
| Anticheat/antivírus estranha o lançador | Baixa | Médio | Lançador apenas encadeia o processo; assinatura de código; documentação |
| Manifesto muda de formato | Baixa | Médio | Parser tolerante, versão fixada no cache, testes com snapshot do manifesto |
| Escopo cresce (Android, plugins) antes da 1.0 | Alta | Médio | Tudo isso está fora da 1.0 por decisão deste roadmap |
| SteamGridDB fora do ar, muda a API ou limita uso | Média | Baixo | Arte é só cosmética: cache local, fontes alternativas (cache das lojas, CDN da Steam) e capa gerada |
| Projeto de uma pessoa só | Alta | Médio | Marcos que já entregam valor pessoal (M2, M3); formato aberto para ninguém ficar preso |

## 10. Insumos para a thread de design

Telas e estados que a interface precisa cobrir, em ordem de prioridade:

1. **Bandeja**: estado geral (tudo sincronizado / enviando / conflito / erro), jogo aberto em outra máquina, ação rápida "sincronizar agora".
2. **Biblioteca**: lista/grade de jogos com capa, origem (Steam, Heroic, emulador, manual), estado de sync por máquina, filtros e busca. Referência: lista de jogos do Ludusavi, mais visual.
3. **Detalhe do jogo / linha do tempo**: snapshots com máquina, data, tempo de jogo, local, miniatura; fixar com nome; restaurar; ver arquivos; árvore quando houve divergência.
4. **Conflito**: duas versões lado a lado, com metadados e miniatura; "manter esta", "manter as duas como slots", "decidir depois". É a tela mais importante: precisa ser calma e impossível de errar.
5. **Primeira execução / máquina nova**: login no destino, senha e chave de recuperação, escolher jogos detectados, restaurar tudo.
6. **Máquinas**: dispositivos pareados, presença, último contato, parear novo (código/QR).
7. **Destinos e saúde**: Drive/WebDAV/S3, espaço usado, último teste de restauração, alertas de token.
8. **Jogo personalizado**: caminhos, executáveis associados (vários por jogo, por máquina), ignorados (referência: editor de jogo personalizado do Ludusavi). Inclui o aviso "Este app é um jogo?" para processos desconhecidos e a lista "abertos agora" para associar manualmente.
9. **Configurações**: retenção, banda, quando enviar, Steam Cloud, idioma, tema.

Estados obrigatórios em todas: vazio, carregando, offline, erro com ação de correção. Plataformas: janela desktop (Linux/Windows/macOS), modo Jogo do Steam Deck (controle, tela 1280×800).

## 11. Métricas de sucesso da 1.0

- Zero relato de progresso perdido.
- 95 % dos jogos instalados detectados sem configuração manual.
- Configurar uma máquina nova em < 5 min.
- Conflitos reais < 1 por mês no uso normal com o aviso de presença ligado.
