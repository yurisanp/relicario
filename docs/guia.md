# Guia do usuário

O Relicário guarda cada versão dos saves dos seus jogos e leva o save certo para a máquina em que você for jogar. É para jogos que não têm nuvem da loja (ou que você joga fora dela: emuladores, GOG, Epic, Proton sem Steam Cloud).

## Instalar

### Linux (AppImage)

1. Baixe `Relicario-<versão>-x86_64.AppImage`.
2. Dê permissão de execução: `chmod +x Relicario-*.AppImage` (ou pelas propriedades do arquivo).
3. Abra. Na primeira vez aparece o assistente.

O AppImage traz junto a linha de comando. Para usar `relicario` no terminal e nas opções de inicialização dos jogos, extraia uma vez:

```sh
./Relicario-*.AppImage --appimage-extract 'usr/bin/relicario*'
mkdir -p ~/.local/bin && cp squashfs-root/usr/bin/relicario* ~/.local/bin/
```

Também existe um `.tar.gz` só com a linha de comando, para servidores ou para quem não quer a janela.

Windows e macOS entram junto com a publicação (instaladores assinados); até lá, dá para compilar com `cargo build --release`.

## Primeira vez

O assistente pergunta:

1. **Nome desta máquina** (Desktop, Notebook, Steam Deck) e onde guardar o histórico. O padrão serve.
2. **Se você já usa o Relicário em outra máquina.** Na mesma rede, gere um código em Dispositivos na outra máquina e digite aqui. Por uma nuvem, use a mesma nuvem e a mesma senha da outra máquina.
3. **Jogos encontrados.** O Relicário procura nas lojas instaladas (Steam, Heroic, Lutris, GOG Galaxy, Epic), em emuladores (RetroArch, PCSX2, Dolphin e outros) e no catálogo do Ludusavi. "Trazer os saves" baixa o que as outras máquinas já guardaram.

Depois disso ele fica na bandeja. Fechar a janela não para nada.

## Como funciona no dia a dia

- Quando um jogo abre, o Relicário percebe. Quando fecha, guarda o save como uma versão nova e manda para as outras máquinas e nuvens.
- Quando você vai jogar em outra máquina, ela já tem a versão mais nova e coloca no lugar antes de você abrir (ou no próximo minuto, se estava desligada).
- Para jogos que ele não percebe sozinho, use nas opções de inicialização da Steam, do Lutris ou do Heroic:

  ```
  relicario run -- %command%
  ```

  Assim ele sincroniza antes de abrir e guarda logo depois de fechar.

## Conflito

Se você jogou o mesmo jogo em duas máquinas sem que uma visse a outra (as duas offline), o Relicário não escolhe sozinho. O jogo aparece em **Conflitos** com as duas versões lado a lado: quando e onde cada uma foi jogada e, quando o Relicário sabe ler o save, local, nível e tempo de jogo. Você escolhe com qual seguir; a outra continua no histórico (ou fica fixada com um nome, se pedir).

## Voltar a uma versão antiga

Na página do jogo, a linha do tempo mostra todas as versões, uma coluna por máquina. Escolha uma e **Restaurar**. O save atual é guardado antes, e a volta vira uma versão nova, que as outras máquinas também recebem. Para não perder uma versão importante para a limpeza automática, use **Fixar**.

## Máquinas e nuvens

- **Pareamento direto** (Dispositivos → Parear outra máquina): as máquinas conversam pela rede local, cifrado, sem servidor no meio.
- **Nuvem**: uma pasta que outro programa já sincroniza (Syncthing, Dropbox, OneDrive, disco externo), WebDAV (Nextcloud), S3 (Backblaze B2, Cloudflare R2, MinIO) ou Google Drive. Por padrão tudo é cifrado antes de sair da máquina. **Guarde a chave de recuperação** que aparece ao criar: sem a senha e sem ela, ninguém (nem você) abre a cópia da nuvem.

## Steam Deck

1. No modo Desktop, instale o AppImage e extraia a linha de comando (veja acima).
2. Instale o serviço, para ele rodar também no modo Jogo: `relicario service install`.
3. Abra o app uma vez para o assistente.

No modo Jogo, o Relicário não abre janela: o serviço guarda e traz os saves sozinho. As notificações de área de trabalho não aparecem no modo Jogo; o que aconteceu fica em Atividade, no modo Desktop. Para jogos fora da Steam (emuladores, Heroic), use `relicario run -- %command%` nas opções de inicialização.

## Quando algo dá errado

- `relicario doctor` mostra o que o Relicário vê nesta máquina.
- `relicario doctor --report` grava um relatório sem dados pessoais para anexar a um pedido de ajuda. Veja [robustez](robustez.md).
- **A máquina morreu ou o Relicário não abre:** `relicario-restore` tira os saves de qualquer cópia do repositório, sem configuração e sem mudar nada nela:

  ```sh
  relicario-restore /caminho/do/repositorio jogos
  relicario-restore /caminho/do/repositorio versoes <jogo>
  relicario-restore /caminho/do/repositorio extrair <jogo ou versão> ~/saves-recuperados
  ```

  Funciona com a cópia cifrada da nuvem também (pede a senha, ou use `--recuperacao <chave>`). Para S3, WebDAV ou Drive, baixe a pasta `Relicario` inteira primeiro.

## Para ir além

- [Formato do repositório](format.md)
- [Catálogo de jogos e regras locais (`[[rules]]`)](catalogo.md)
- [Como escrever um leitor de save](plugins.md)
- [Daemon e API](daemon.md)
