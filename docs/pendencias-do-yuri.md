# Pendências que dependem do Yuri

Itens que só o dono do projeto pode fazer (contas, cadastros, assinaturas). Combinado em 24/11/2026: a publicação fica para o final e, até lá, a distribuição é só um AppImage gerado no CI.

| Quando | O quê | Por quê |
|---|---|---|
| Antes da fase 5 (nuvem) | Criar o client OAuth do Google Cloud (escopo `drive.file`) e iniciar a verificação do app | Sem verificação, o login expira a cada 7 dias; a verificação pode levar semanas. Até lá o Relicário aceita "usar meu próprio client" |
| Fase 5 (opcional) | Registrar app no Microsoft Entra (OneDrive) e no Dropbox | Só se esses destinos entrarem na 1.0 (veja `docs/conectores.md`) |
| Fase 6 (opcional) | Chave de API do SteamGridDB para testes | A chave é por usuário; o app pede nas configurações |
| Fase 7, no fim | Conta Apple Developer, certificado de assinatura do Windows, domínio, submissão ao Flathub/AUR/winget | Publicação; fica por último |
| Fase 7 | Teste de 2 semanas nas duas máquinas reais e teste de usabilidade com 3 pessoas | Critérios de pronto da 1.0 que precisam de gente de verdade |
