# Nuvem

Uma cópia do repositório em Google Drive, S3 (AWS, Cloudflare R2, Backblaze B2, MinIO, Wasabi),
WebDAV/Nextcloud ou qualquer pasta. Cifrada por padrão; o Relicário só lê e grava os próprios
arquivos.

## Conectar

```sh
relicario cloud add drive --gdrive                          # abre o navegador
relicario cloud add r2 --s3 saves --endpoint https://<conta>.r2.cloudflarestorage.com --access-key <id>
relicario cloud add nextcloud --webdav https://nuvem.exemplo/remote.php/dav/files/yuri --user yuri
relicario cloud add pendrive --pasta /media/pendrive/relicario
```

Na primeira máquina o cofre é criado: o Relicário pede uma senha e mostra uma **chave de
recuperação**, uma vez só. Nas outras, o mesmo comando conecta ao cofre que já existe e pede a
senha. Senhas e chaves ficam no chaveiro do sistema (sem chaveiro, num arquivo legível só pelo
usuário; o `doctor` diz qual).

Opções: `--sem-criptografia` (só para destinos seus), `--limite <KiB/s>`, `--root <pasta>`.

Sem terminal interativo, a senha vem de `RELICARIO_PASSWORD`, o segredo do destino de
`RELICARIO_SECRET` e a chave de recuperação de `RELICARIO_RECOVERY`.

## Dia a dia

O daemon envia o que mudou depois de cada jogo fechar e confere de hora em hora, só sem jogo
aberto. Outra máquina que enviou um save novo é trazida como numa sincronização direta. Uma vez
por semana a cópia inteira é baixada, conferida pelos hashes e o último snapshot de cada jogo é
restaurado numa pasta temporária e comparado.

Login expirado ou revogado vira notificação (no máximo uma por dia por nuvem, a primeira dentro de
uma hora): `relicario cloud login <nome>`.

```sh
relicario cloud list
relicario cloud sync [nome]
relicario cloud check [nome]
relicario cloud unlock <nome>             # guarda a senha nesta máquina
relicario cloud unlock <nome> --recovery  # esqueceu a senha: chave de recuperação + senha nova
relicario cloud remove <nome>             # desconecta, não apaga nada lá
```

## Jogos que já têm Steam Cloud

Por padrão, jogos com Steam Cloud ficam em **só histórico**: o Relicário guarda as versões, mas
quem leva o save entre máquinas é a Steam. Para mudar:

```sh
relicario game mode <jogo> completo   # ou historico, ou auto
```

## Importar do Ludusavi

```sh
relicario import ludusavi                       # usa a configuração do Ludusavi desta máquina
relicario import ludusavi --backups ~/ludusavi-backup --game "Baldur"
```

Cada backup (completo ou diferencial, em pasta ou zip) vira um snapshot de histórico com o horário
original, conferido arquivo por arquivo pelo SHA-1 do `mapping.yaml`. Nada é restaurado e nada
vira o save atual; o backup mais recente de cada jogo fica fixado. Os jogos personalizados do
Ludusavi viram jogos cadastrados (caminhos com marcadores que só o Ludusavi resolve são listados).

## Google Drive

Escopo `drive.file`: o Relicário enxerga só os arquivos que ele mesmo criou, numa pasta
`Relicario`. O login usa o fluxo de apps instalados (PKCE e retorno para `127.0.0.1`); o token de
atualização fica só no chaveiro desta máquina. Enquanto o client OAuth do projeto não existe,
defina `RELICARIO_GDRIVE_CLIENT_ID` e `RELICARIO_GDRIVE_CLIENT_SECRET` (veja
`docs/pendencias-do-yuri.md`).
