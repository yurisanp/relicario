# ADR 0002: sincronização por ancestralidade de snapshots

- **Estado:** proposto (revisar na fase 0)
- **Data:** 2026-11-24

## Contexto

Comparar horários de modificação entre máquinas falha com relógios errados, cópias e restaurações. O Relicário precisa decidir sozinho, com segurança, qual save é o mais atual.

## Decisão

Cada snapshot aponta para o pai. Cada máquina publica um *head* por jogo. Ao abrir um jogo, o daemon lê os heads das outras máquinas (par direto com limite de 3 s, nuvem em seguida) e classifica a relação em **igual**, **avanço**, **envio** ou **divergiu**. O horário só serve para exibir e desempatar. Divergências nunca são resolvidas apagando um lado: os dois ficam no histórico.

## Consequências

- O motor de decisão é puro e testável com históricos aleatórios (`relicario-engine`).
- A troca de heads precisa ser barata e, onde houver, usar escrita condicional.
- A interface precisa de uma tela para escolher o lado em divergências.
