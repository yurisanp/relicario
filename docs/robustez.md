# Robustez

A regra do Relicário é não perder progresso de jogo, mesmo quando algo dá errado no meio do caminho. Esta página diz o que acontece em cada caso e onde isso é testado.

## Bateria de caos

Os testes ficam em [`crates/relicario-store/tests/caos.rs`](../crates/relicario-store/tests/caos.rs) e rodam no CI nos três sistemas.

| Situação | O que acontece |
|---|---|
| Restauração interrompida (processo morto, disco cheio, arquivo travado pelo jogo) | Antes de escrever no disco, o save atual vira snapshot de segurança e a máquina anota "restaurando X" no seu arquivo de heads. Na próxima operação naquele jogo (sincronizar, guardar), o Relicário termina a restauração antes de qualquer outra coisa. O disco pela metade nunca vira uma versão nova. |
| Envio para uma réplica cortado em qualquer ponto (rede caiu, disco cheio, daemon morto) | O destino é escrito na ordem packs, índice, snapshots e por último o head. Cortado em qualquer escrita, o destino continua coerente: nenhum head aponta para dado que não chegou. O envio seguinte completa. |
| Relógios com duas horas de diferença | Quem é mais novo vem da árvore de versões, não do relógio. A retenção nunca apaga o head de nenhuma máquina nem versões fixadas, com o relógio adiantado ou atrasado. |
| Máquina desligada por 30 dias | As outras continuam guardando e aplicando a retenção. O head que elas conhecem da máquina desligada continua protegido. Quando ela volta, segue a versão mais nova; se também jogou desligada, aparece um conflito com as duas versões guardadas, e nada é sobrescrito sozinho. |
| Nuvem com login revogado | A sincronização com aquela nuvem falha e o daemon avisa uma vez (não a cada tentativa). O repositório local e as outras réplicas seguem normalmente. |

Outras proteções que já existiam: o guarda de save suspeito (encolheu muito, ficou vazio, virou zeros) e a conferência do disco depois de toda restauração.

## Registro local

O daemon grava uma linha JSON por evento em `<pasta de dados>/logs/relicario.log` (no Linux, `~/.local/share/relicario/logs/`). Ao passar de 1 MiB, o arquivo gira e os três anteriores ficam guardados. Nada é enviado: o Relicário não tem telemetria.

## Relatório para pedir ajuda

```sh
relicario doctor --report
```

Grava `relicario-relatorio-<data>.txt` na pasta atual (ou em `--saida <arquivo>`) com o diagnóstico, os últimos 300 eventos do daemon e a atividade recente. Antes de gravar, tira a pasta pessoal, o nome de usuário, os nomes das máquinas, caminhos, endereços e contas de nuvem, e-mails e IPs. Confira o arquivo antes de mandar.
