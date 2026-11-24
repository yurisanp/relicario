# ADR 0004: criptografia ligada por padrão na nuvem, com chave de recuperação

- **Estado:** proposto (revisar na fase 0)
- **Data:** 2026-11-24

## Contexto

Saves vão para contas de nuvem de terceiros. Ao mesmo tempo, "esqueci a senha" não pode significar perder anos de progresso.

## Decisão

Criptografia autenticada (AEAD) ligada por padrão para destinos de nuvem. Na configuração, o Relicário gera uma **chave de recuperação** para imprimir ou exportar. A senha fica no chaveiro do sistema (crate `keyring`). Destinos locais podem desligar a criptografia.

## Consequências

- O repositório no Drive fica ilegível sem a chave (critério da fase 5).
- Precisamos de um fluxo de recuperação claro na interface e no `relicario doctor`.
- Algoritmo e derivação de chave são escolhidos na fase 1 e registrados em `docs/format.md`.
