# Sincronização direta (par-a-par)

Duas ou mais máquinas sincronizam sem nuvem e sem servidor do projeto. Cada máquina tem o
próprio repositório (normalmente na pasta de dados) e o daemon replica com as máquinas pareadas.

## Parear

Na máquina A:

```sh
relicario pair
# Na outra máquina, rode:
#     relicario pair 123 456
```

Na máquina B (na mesma rede local ela acha A sozinha por mDNS; fora dela, use `--addr`):

```sh
relicario pair 123 456
relicario pair 123456 --addr pc.tail1234.ts.net   # Tailscale, ou IP:porta
```

O código vale 5 minutos e **uma tentativa**: errar encerra a sessão. Depois de pareadas, as
máquinas se reconhecem pela impressão digital do certificado, guardada no `config.toml` de cada
uma (`[[peers]]`). `relicario peers list|remove|sync|discover` cuida do resto.

## Como funciona

| Peça | Escolha |
|---|---|
| Transporte | QUIC (`quinn`), UDP 47321 (`p2p_port` no `config.toml`; `0` desliga) |
| Segurança | TLS 1.3 mútuo com certificado autoassinado por máquina (`config/p2p/identidade.*`). O handshake prova a posse da chave; a confiança é a impressão digital BLAKE3 do certificado. |
| Pareamento | SPAKE2 com o código de 6 dígitos. Cada lado manda um MAC sobre as duas impressões digitais com a chave combinada: quem não sabe o código não consegue se passar por nenhum dos lados, nem ficar no meio. |
| Descoberta | mDNS (`_relicario._udp.local.`), endereços gravados no pareamento (aceita nomes do Tailscale) e o cartão de visita (`addresses` no `heads/` de cada máquina, que chega também pela nuvem). |
| Dados | A [replicação do formato](format.md#replicação): cada arquivo é conferido pelo hash antes de ser gravado, em ordem segura, e só o que falta trafega. |

Um par só lê e grava arquivos do repositório: não apaga nada, não grava `config` nem `locks/`, e
nunca grava o `heads/` da máquina que está servindo. Heads de outras máquinas só entram se forem
mais novos.

## Quando sincroniza

- Ao iniciar o daemon e a cada minuto.
- Antes de abrir um jogo pelo `relicario run` (limite de 3 s: o jogo não espera a rede).
- Depois de o jogo fechar, logo após o snapshot.
- Quando um par grava um head novo aqui, o daemon confere os jogos na hora: se outra máquina
  avançou e o jogo está fechado, o save chega sozinho (com cópia de segurança).
- `relicario sync` e `relicario plan` replicam antes de decidir.

Se a conexão cair no meio, o outro lado fica com arquivos a mais (íntegros), nunca com um head
apontando para o que não chegou. A próxima replicação continua de onde parou.

## Limites atuais

- Sem travessia de NAT nem relay: fora da rede local, use Tailscale (ou outra VPN) ou a nuvem
  da fase 5.
- Replicação lista os arquivos dos dois lados a cada rodada; para repositórios de saves
  (milhares de arquivos) isso é rápido. Um índice incremental entra se precisar.
