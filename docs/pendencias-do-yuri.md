# Pendências que dependem do Yuri

Itens que só o dono do projeto pode fazer (contas, cadastros, assinaturas). Combinado em 24/11/2026: a publicação fica para o final e, até lá, a distribuição é só um AppImage gerado no CI.

| Quando | O quê | Por quê |
|---|---|---|
| Antes da v1 (nuvem) | Criar o client OAuth do Google Cloud (tipo "app para computador", escopo `drive.file`) e iniciar a verificação do app. Depois, colocar `RELICARIO_GDRIVE_CLIENT_ID` e `RELICARIO_GDRIVE_CLIENT_SECRET` nos segredos do GitHub Actions, para o AppImage sair com o client embutido | Sem verificação, o login expira a cada 7 dias; a verificação pode levar semanas. Até lá funciona com as mesmas variáveis de ambiente na hora de rodar (`relicario cloud add drive --gdrive`) |
| Fase 5 (opcional) | Registrar app no Microsoft Entra (OneDrive) e no Dropbox | Só se esses destinos entrarem na 1.0 (veja `docs/conectores.md`) |
| Fase 6 | Mandar um `caption.sav` e um `progress.sav` do Hookshot (ou dizer o formato) | O leitor do Hookshot hoje adivinha: JSON, texto `chave=valor` ou textos e imagem embutidos num binário. Com o formato certo ele mostra local, capítulo e tempo de jogo de verdade |
| Fase 6 | Conferir o leitor do BG3 com um save real | Foi escrito pelo formato LSPK 18 conhecido e testado com pacotes montados nos testes |
| Fase 6 (opcional) | Chave de API do SteamGridDB para testes | A chave é por usuário; o app pede nas configurações |
| Fase 7, no fim | Conta Apple Developer, certificado de assinatura do Windows, domínio, submissão ao Flathub/AUR/winget | Publicação; fica por último |
| Fase 7 | Teste de 2 semanas nas duas máquinas reais e teste de usabilidade com 3 pessoas | Critérios de pronto da 1.0 que precisam de gente de verdade |
