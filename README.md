# Relicário

> Seus saves iguais em todas as máquinas, para os jogos que não têm nuvem própria.

O Relicário guarda cada versão dos saves dos seus jogos, descobre sozinho qual é a mais atual entre várias máquinas e **nunca sobrescreve progresso sem guardar uma cópia antes**. É o Steam Cloud para os jogos que não têm Steam Cloud, com histórico de verdade.

**Estado:** fase 1 concluída (marco M1, "funciona na CLI"). Ainda é para testes, não para seus saves de verdade; acompanhe o [roadmap](docs/roadmap.md).

## Como vai funcionar

- Um serviço pequeno (`relicario-daemon`) roda em cada máquina, percebe quando um jogo abre e fecha e tira um snapshot do save.
- Os snapshots formam uma árvore. A decisão entre "igual", "avançar", "enviar" ou "divergiu" vem da ancestralidade, nunca do relógio.
- Os dados vão para um repositório de blocos com deduplicação e criptografia, guardado onde você quiser: outra máquina na rede, Google Drive, WebDAV, S3, SFTP ou uma pasta.
- Jogos da Steam, Proton, Heroic, Lutris, GOG, Epic e emuladores são reconhecidos sem configuração, usando o manifesto do [Ludusavi](https://github.com/mtkennerly/ludusavi-manifest).

Plataformas da 1.0: Linux (incluindo Steam Deck), Windows e macOS.

## Estrutura

| Crate | Responsabilidade |
|---|---|
| [`relicario-model`](crates/relicario-model) | Tipos centrais: jogos, máquinas, snapshots, heads |
| [`relicario-engine`](crates/relicario-engine) | Motor de decisão, retenção e proteção contra save corrompido (sem I/O) |
| [`relicario-store`](crates/relicario-store) | Repositório de blocos (FastCDC + BLAKE3 + zstd + AEAD) |
| [`relicario-catalog`](crates/relicario-catalog) | Manifesto, detectores de loja e caminhos |
| [`relicario-watch`](crates/relicario-watch) | Vigia de processos e arquivos |
| [`relicario-transport`](crates/relicario-transport) | Destinos remotos: p2p, Drive, WebDAV, S3, SFTP, pasta |
| [`relicario-platform`](crates/relicario-platform) | Caminhos, serviço, segredos e notificações por sistema |
| [`relicario-daemon`](crates/relicario-daemon) | Serviço que orquestra tudo e expõe a API local |
| [`relicario-cli`](crates/relicario-cli) | Linha de comando `relicario` |
| [`relicario-ui`](crates/relicario-ui) | Interface em Tauri 2 (a partir da fase 6) |
| [`relicario-plugins`](crates/relicario-plugins) | Leitores de save específicos de jogos |

Documentação em [`docs/`](docs): [roadmap](docs/roadmap.md), [conectores de armazenamento](docs/conectores.md), [decisões de arquitetura](docs/adr), [formato do repositório](docs/format.md) e o [protótipo da interface](docs/design/prototipo.html).

## Compilar

Requer Rust estável (1.88 ou mais novo).

```sh
cargo build --workspace
cargo test --workspace
cargo run -p relicario-cli -- doctor
```

## Uso pela linha de comando

```sh
# Em cada máquina, apontando para a mesma pasta (HD externo, pasta sincronizada…)
relicario init --repo /mnt/externo/relicario --name PC

# Cadastrar um jogo à mão (a detecção automática chega na fase 2)
relicario game add hookshot --name Hookshot --path ~/.local/share/Hookshot \
  --exclude 'save-backups/**' --exclude hookshot_options.ini

relicario snapshot          # guarda o estado atual
relicario plan              # mostra o que mudou em cada máquina
relicario sync              # guarda o que mudou aqui e traz o que está mais novo lá
relicario log hookshot      # histórico
relicario restore hookshot 3f2a9c   # volta para um ponto (o estado atual é guardado antes)
relicario pin 3f2a9c --name "antes do chefe"
relicario verify --read-data
relicario prune             # aplica a retenção e libera espaço
```

## Contribuir

Veja [CONTRIBUTING.md](CONTRIBUTING.md).

## Licença

Distribuído sob a licença MIT ou Apache-2.0, à sua escolha. Veja [LICENSE-MIT](LICENSE-MIT) e [LICENSE-APACHE](LICENSE-APACHE).

O manifesto de jogos vem do projeto [ludusavi-manifest](https://github.com/mtkennerly/ludusavi-manifest) (MIT), gerado a partir do [PCGamingWiki](https://www.pcgamingwiki.com).
