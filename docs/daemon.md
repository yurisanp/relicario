# Daemon e integração com lançadores

O `relicario-daemon` (ou `relicario daemon run`, o mesmo código) roda em segundo plano em cada
máquina. Ele não precisa de administrador.

## O que ele faz

| Quando | O quê |
|---|---|
| Ao iniciar | Confere todos os jogos fechados: guarda o que mudou e traz o que outra máquina fez enquanto esta estava desligada (sempre com cópia de segurança). |
| A cada 2 s | Lê a lista de processos (`/proc` no Linux, `sysinfo` no Windows e macOS) e reconhece jogos pelo executável do manifesto, pela pasta de instalação da loja, pelos executáveis cadastrados e pelas associações feitas com `relicario associate`. |
| Jogo abriu | Guarda o estado de antes (o jogo já está aberto, então nunca restaura), marca o jogo como aberto nesta máquina e avisa se ele também está aberto em outra. |
| Jogo fechou | Espera o jogo terminar de gravar (arquivos sem mudar por 5 s e, no Windows, sem trava exclusiva), guarda o snapshot, soma o tempo de jogo e tira a marca de aberto. |
| A cada hora | Conferência periódica, igual à do início, nos jogos fechados. |
| A cada 6 h | Reescaneia lojas e manifesto (jogos instalados ou removidos). |
| Executável desconhecido numa pasta de jogos | Pergunta uma vez por notificação; a resposta é `relicario associate` ou `relicario apps ignore`. |

Save que parece corrompido (encolheu mais de 80%, ficou vazio, virou zeros ou mudou de formato)
é guardado mas não vira head e gera uma notificação. Conflito entre máquinas nunca é resolvido
sozinho: as duas versões ficam guardadas.

`relicario restore` se recusa a rodar com o jogo aberto nesta máquina.

## `relicario run`

Detecção por processo chega tarde para restaurar: quando o daemon vê o jogo, ele já leu o save.
Para trazer o save de outra máquina **antes** do jogo abrir, a loja abre o jogo pelo
`relicario run`:

1. descobre o jogo (`--game`, `SteamAppId` da Steam ou o executável no comando);
2. sincroniza (pelo daemon, se estiver rodando, ou direto);
3. abre o jogo e espera ele fechar;
4. guarda o snapshot e devolve o código de saída do jogo.

Se qualquer coisa falhar (sem configuração, repositório inacessível, jogo desconhecido), o
`run` avisa e abre o jogo do mesmo jeito.

`relicario launcher install` escreve isso nas lojas:

- **Steam**: `LaunchOptions` em `userdata/<id>/config/localconfig.vdf` (a Steam precisa estar
  fechada; um backup `localconfig.vdf.relicario-<horário>` fica ao lado). Opções existentes
  são mantidas: `gamemoderun %command%` vira `"relicario" run -- gamemoderun %command%`.
- **Heroic**: `wrapperOptions` em `GamesConfig/<app>.json`.
- **Lutris**: `system.prefix_command` no `.yml` do jogo.

`relicario launcher uninstall` desfaz só o que o Relicário escreveu.

## Serviço

| Sistema | `relicario service install` |
|---|---|
| Linux | `~/.config/systemd/user/relicario.service`, `systemctl --user enable --now` |
| Windows | Tarefa agendada "Relicario" ao entrar na sessão |
| macOS | `~/Library/LaunchAgents/io.github.yurisanp.relicario.plist` |

Dentro de um AppImage, o serviço aponta para o próprio AppImage (`$APPIMAGE`).

## API local

Uma linha JSON por pedido e uma por resposta, num socket local (socket abstrato no Linux, pipe
nomeado no Windows, arquivo em `/tmp` no macOS) cujo nome depende da pasta de dados do usuário.

```json
{"op":"status"}
{"op":"launching","game":"hookshot"}
{"op":"exited","game":"hookshot","started_at":1795572883000}
{"op":"sync","game":null}
{"op":"rescan"}
{"op":"stop"}
```

A interface usa o mesmo socket com `{"op":"api","call":{...}}`, que responde
`{"kind":"api","value":...}`. Os pedidos estão em `crates/relicario-daemon/src/api.rs`
(`Call`): `overview`, `game`, `snapshot`, `activity`, `devices`, `settings`, `restore`, `pin`,
`unpin`, `resolve`, `set_mode`, `set_ignored`, `save_settings`, `art`, `art_choices`, `art_set`,
`art_reset`, `export_zip`, `pair_join`, `peer_remove`, `cloud_add`, `cloud_remove`, `cloud_sync`,
`cloud_check` e `prune`. Para testar à mão:

```sh
relicario api '{"call":"overview"}'
relicario api '{"call":"game","id":"hookshot"}'
```

Sem daemon rodando, o `relicario api` atende no próprio processo. A interface faz o mesmo,
mas sobe o daemon inteiro dentro dela (laço, sincronização direta e socket) quando não há
outro rodando.

## Conflitos

`resolve` fica com uma das pontas: grava um snapshot de merge (pais: a escolhida e as outras),
restaura o conteúdo dela (o que estava no disco vira cópia de segurança, se ainda não estiver
guardado) e faz do merge o head desta máquina. As outras máquinas veem o merge descendendo do
que elas têm e só avançam, sem conflito. Com `keep_both`, as outras versões ficam fixadas com
o nome da máquina ("versão de Notebook"), para a retenção não apagá-las.

## Atividade

Cada máquina guarda as últimas 1000 entradas em `atividade.jsonl` na pasta de dados: snapshot
guardado, restaurado, conflito, save suspeito, jogo aberto e fechado, nuvem, pares e
pareamento. Resultados repetidos (o mesmo conflito a cada conferência) entram uma vez só.

## Variáveis para testes

`RELICARIO_POLL_MS`, `RELICARIO_PERIODIC_MS`, `RELICARIO_RESCAN_MS`, `RELICARIO_SETTLE_MS`,
`RELICARIO_SOCKET` e `RELICARIO_NO_NOTIFY`.
