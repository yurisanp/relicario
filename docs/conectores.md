# Relicário: levantamento de conectores de armazenamento

Escrito em 24/11/2026 como complemento do [roadmap](relicario-roadmap.md) (seções 4.4, 4.5 e fase 5). "Conector" aqui é um destino onde o repositório de saves vive: a trait `Remote` da crate `relicario-transport`.

Números de cota e preço mudam com frequência. Os valores abaixo são os conhecidos em novembro de 2026 e devem ser conferidos de novo na fase 0, junto com a criação dos clients OAuth.

---

## 1. Resumo e recomendação

1. **O formato do repositório já resolve a parte difícil.** Como tudo é imutável e endereçado por hash, e cada máquina só escreve o próprio `heads/<machine>`, nenhum destino precisa de escrita condicional para o sync do dia a dia. Escrita condicional só faz diferença em dois pontos raros: a trava do prune/GC e a detecção de "duas instalações com o mesmo ID de máquina". Isso abre a porta para destinos "burros" como Google Drive, pasta sincronizada e WebDAV genérico.
2. **A "pasta local" da fase 1 já é um conector de nuvem de graça.** Apontada para a pasta do Dropbox, OneDrive, Google Drive para desktop, iCloud Drive ou Syncthing, ela funciona sem nenhum OAuth, pelo mesmo motivo do item 1. Vale documentar isso como caminho oficial já no M1/M2.
3. **Ordem sugerida dos conectores nativos (fase 5):** S3-compatível primeiro (mais simples, melhor semântica, testável no CI com MinIO, cobre R2/B2/AWS/Wasabi/MinIO), depois Google Drive (é o destino pessoal do Yuri, mas é o mais trabalhoso), depois WebDAV/Nextcloud e SFTP. OneDrive e Dropbox nativos ficam para a 1.x. rclone entra como opção avançada.
4. **GitHub não deve ser destino de saves.** O roadmap escolhe GitHub para o *código* do projeto, não para guardar backups. Como armazenamento ele é ruim (detalhes na seção 3.9).
5. **Base de implementação:** `opendal` cobre S3, WebDAV, SFTP, Google Drive, OneDrive e Dropbox com uma API só. Usar como base, mas com uma camada fina própria em cima para OAuth/PKCE, escrita condicional e uma suíte de conformidade, porque a qualidade dos serviços menos usados do opendal varia.

## 2. Tabela comparativa

| Destino | Autenticação no desktop | Grátis | Custo pago (aprox.) | Escrita condicional | Rename atômico | Crate Rust | Esforço |
|---|---|---|---|---|---|---|---|
| Pasta local / pasta sincronizada | Nenhuma | Depende do cliente | – | `O_EXCL` local; a sincronização não garante | Sim (local) | `std` | Baixo (já na fase 1) |
| S3 (AWS) | Access key + secret | Não relevante | ~US$ 0,023/GB/mês + saída | Sim: `If-None-Match` e `If-Match` no PUT | Não (não precisa) | `aws-sdk-s3`, `object_store`, `opendal` | Baixo |
| Cloudflare R2 | Access key (token de API) | 10 GB | ~US$ 0,015/GB/mês, saída grátis | Sim (`If-Match`/`If-None-Match`) | Não | idem S3 | Baixo |
| Backblaze B2 | Application key | 10 GB | ~US$ 6/TB/mês | Não confiável pela API S3 (Proxmox desativa `If-None-Match` para B2) | Não | idem S3 | Baixo |
| Google Drive | OAuth 2 + PKCE, loopback | 15 GB (dividido com Gmail/Fotos) | Google One | Não (API v3 sem `If-Match` útil) | Não, mas tem IDs estáveis | `opendal` (gdrive) ou `reqwest` + `oauth2` | Alto |
| OneDrive | OAuth 2 + PKCE (Microsoft identity, client público) | 5 GB | Microsoft 365 | Sim: `If-Match` com eTag no upload | Mover por ID | `opendal` (onedrive), `onedrive-api` | Médio |
| Dropbox | OAuth 2 + PKCE, refresh token offline | 2 GB | Plus/Essentials | Sim: `WriteMode::Update(rev)` | Sim (`move_v2`) | `dropbox-sdk` (oficial), `opendal` | Médio |
| WebDAV / Nextcloud | Usuário + senha de app (Nextcloud: Login Flow v2) | Self-host; Koofr e outros têm plano grátis | Varia | Nextcloud (sabre/dav): `If-Match`/`If-None-Match` funcionam; WebDAV genérico varia | `MOVE` (geralmente atômico) | `opendal` (webdav), `reqwest_dav` | Médio |
| SFTP | Chave SSH / senha | Self-host (NAS, VPS, outra máquina) | Varia | Não; trava via criação exclusiva | Sim, com `posix-rename@openssh.com` | `russh` + `russh-sftp` (puro Rust); `opendal` sftp usa `openssh` (só Unix) | Médio |
| rclone (camada genérica) | A do rclone (o usuário configura) | – | – | Depende do backend | Depende | Processo `rclone rcd` / `serve` via HTTP | Baixo para nós, alto para o usuário |
| GitHub | OAuth device flow / PAT | Repositório: limite ~1 GB recomendado, 100 MB por arquivo | LFS pago | Sim (SHA no `PUT contents`) | – | `octocrab` | Não recomendado |

## 3. Detalhe por destino

### 3.1 Pasta local e pasta sincronizada

- Já está na fase 1 para testes e HD externo.
- Apontar para uma pasta do Dropbox/OneDrive/Google Drive para desktop/iCloud/Syncthing dá nuvem sem OAuth, sem verificação do Google e sem custo de desenvolvimento.
- Seguro por causa do formato: arquivos imutáveis não geram "cópia em conflito", e `heads/<machine>` só é escrito por uma máquina.
- Cuidados: escrever em `*.tmp` e renomear (o cliente de sync pode subir arquivo pela metade); ignorar arquivos `.tmp` e cópias de conflito na leitura; aceitar que o head de outra máquina chega com atraso (o cliente sincroniza quando quer). O app não sabe se o upload terminou, então a tela de saúde deve dizer "entregue ao cliente de sincronização", não "na nuvem".
- No macOS, iCloud Drive só existe por esse caminho (`~/Library/Mobile Documents/...`): não há API pública para app de terceiros fora do ecossistema Apple.

### 3.2 S3 e compatíveis (AWS, R2, B2, Wasabi, MinIO, Garage)

- Melhor semântica de todas: chaves = caminhos, listagem por prefixo consistente, e na AWS e no R2 há `If-None-Match: *` (não sobrescreve) e `If-Match: <etag>` (compare-and-swap) no PUT.
- Autenticação por chave de acesso, sem OAuth. Isso é mais difícil para usuário leigo (criar bucket e chave), então a UI precisa de um assistente com links por provedor. Chave guardada no chaveiro do SO.
- **R2** é o melhor custo-benefício para quem quer nuvem paga: 10 GB grátis e saída grátis (restaurar não custa nada).
- **B2** é o mais barato por GB, mas não dá para contar com escrita condicional. Tratar como destino sem CAS.
- No CI, MinIO em contêiner permite testar o conector de verdade nos três SOs (Linux no runner; Windows/macOS contra o mesmo serviço).
- Crates: `aws-sdk-s3` é o oficial e pesado; `object_store` (do Arrow) é enxuto e já suporta PUT condicional; `opendal` também. Sugestão: `opendal` para manter uma dependência só, conferindo que o PUT condicional passa os cabeçalhos.

### 3.3 Google Drive

O destino pessoal do plano original, e o mais trabalhoso.

- **OAuth no desktop:** client do tipo "Desktop app", fluxo com navegador do sistema, PKCE e redirecionamento para `http://127.0.0.1:<porta>` (loopback). O fluxo "out-of-band" foi desativado pelo Google. O `client_secret` de app desktop não é segredo de verdade e pode ir no binário.
- **Escopo:** `drive.file` (vê só o que o app criou, visível ao usuário no Drive) ou `drive.appdata` (pasta oculta do app, o usuário não vê nem apaga por engano, mas também não consegue copiar à mão e conta na cota). Os dois são escopos não restritos, então não exigem auditoria de segurança paga, só a verificação de marca (tela de consentimento, domínio, política de privacidade). Recomendação: `drive.file` com pasta visível "Relicário", porque o usuário enxergar o repositório combina com o princípio "formato aberto". Confirmar a classificação dos escopos na fase 0.
- **Armadilha 1: app em modo "Testing" tem refresh token que expira em 7 dias.** Até publicar o app, cada usuário teria que logar de novo toda semana. Isso reforça começar a verificação na fase 0, como o roadmap já prevê, e manter a opção "usar meu próprio client" para quem tem pressa.
- **Armadilha 2: o Drive não tem caminhos, tem IDs, e aceita nomes duplicados na mesma pasta.** Duas criações de `heads/pc` geram dois arquivos com o mesmo nome. O conector precisa de um cache local nome → ID, sempre atualizar por ID (`files.update`) e, ao listar, se houver duplicatas, ficar com a mais recente e apagar as outras (só para heads; packs duplicados são idênticos por hash e qualquer um serve).
- **Sem escrita condicional:** não há `If-Match` funcional na API v3. Para o head não importa (um escritor). Para trava de prune, usar o esquema de lock do restic (arquivo de lock + esperar + reler) e só fazer prune raramente.
- **Cotas:** limite por projeto e por usuário na casa de milhares de requisições por minuto, e limite de upload diário de 750 GB por usuário. Para saves isso é folgado, mas packs de 4 a 16 MB ajudam a manter o número de requisições baixo. Tratar `403 rateLimitExceeded`/`429` com backoff exponencial.
- **Upload:** usar upload resumível para packs; `multipart` para heads e índices pequenos.
- **Crates:** o serviço `gdrive` do opendal existe e resolve caminho → ID; `google-drive3` (gerado) é completo mas pesado. Sugestão: começar com o opendal e trocar por um cliente próprio fino (`reqwest` + `oauth2`) se o cache de IDs e o tratamento de duplicatas não ficarem do jeito acima.

### 3.4 OneDrive (Microsoft Graph)

- Client público no Microsoft Entra, PKCE e loopback, sem segredo. Escopo `Files.ReadWrite.AppFolder` dá uma pasta própria do app (`/drive/special/approot`), o equivalente ao `drive.file`. Não exige verificação pesada como a do Google.
- Upload aceita `If-Match` com eTag, então há compare-and-swap. Sessão de upload para arquivos grandes.
- 5 GB grátis; muito comum em quem joga no Windows. Throttling do Graph com `Retry-After`, tratar como no Drive.
- Bom candidato para a 1.x, ou para a fase 5 se sobrar tempo, porque o público Windows é grande.

### 3.5 Dropbox

- PKCE sem segredo, `token_access_type=offline` para refresh token. Permissão "App folder" limita o app a `/Apps/Relicário`.
- Upload com `mode: update(<rev>)` é compare-and-swap de verdade.
- Só 2 GB grátis. App começa em modo de desenvolvimento (limite de usuários) e precisa pedir "production" ao Dropbox antes de distribuir.
- `dropbox-sdk` é a crate oficial. Também 1.x; quem usa Dropbox já tem a pasta sincronizada da seção 3.1 enquanto isso.

### 3.6 WebDAV / Nextcloud

- Cobre Nextcloud, ownCloud, Koofr, InfiniCLOUD, NAS (Synology, QNAP) e servidores próprios.
- Nextcloud: usar Login Flow v2 (abre o navegador e devolve uma senha de app), que é a melhor experiência sem OAuth. Para WebDAV genérico, URL + usuário + senha de app.
- Nextcloud (sabre/dav) respeita `If-Match`/`If-None-Match` no PUT; outros servidores podem ignorar. Detectar na configuração: tentar um PUT com `If-None-Match: *` sobre um arquivo que existe e ver se dá 412. Se não der, marcar o destino como "sem CAS".
- `MOVE` permite escrever em temporário e renomear. `PROPFIND` com `Depth: 1` para listar (evitar `infinity`, que muitos servidores bloqueiam).

### 3.7 SFTP

- Para quem tem NAS, VPS ou quer usar outra máquina da casa. Atende bem o Steam Deck com um PC servindo de cofre.
- Escrita em temporário + `posix-rename@openssh.com` dá atomicidade. Trava por criação exclusiva (`O_EXCL`).
- Crates: `russh` + `russh-sftp` são puro Rust e funcionam no Windows; o serviço sftp do opendal usa o binário `openssh` e não roda no Windows. Para ser agnóstico, usar `russh`.

### 3.8 rclone como camada genérica

- Dá acesso a mais de 70 provedores (pCloud, MEGA, Box, Proton Drive, Jottacloud etc.) sem escrevermos nada.
- Integração mais limpa: o usuário aponta para um remote do rclone já configurado, e o Relicário sobe `rclone serve webdav` (ou `rcd`) local e fala WebDAV com ele. Reaproveita o conector WebDAV.
- Contras: dependência externa (~50 MB, não embutir), configuração na mão, semântica de CAS desconhecida (tratar sempre como "sem CAS"). Bom como opção "avançado" na 1.x.

### 3.9 GitHub (por que não)

- Os termos do GitHub não aceitam repositório como armazenamento de backup genérico, e o limite prático é ~1 GB por repositório e 100 MB por arquivo; o LFS tem cota baixa e paga.
- Git não combina com packs binários cifrados (sem delta útil, histórico cresce para sempre, prune exige reescrever histórico).
- A API de conteúdo tem compare-and-swap (SHA no `PUT`), mas isso não compensa o resto.
- Conclusão: GitHub fica como casa do código e do CI, como o roadmap decidiu, e não como destino.

### 3.10 Outros avaliados e descartados para a 1.0

| Serviço | Motivo |
|---|---|
| iCloud Drive (API) | Sem API pública para app de terceiros multiplataforma; no macOS, usar como pasta sincronizada |
| Proton Drive | Sem API pública estável; via rclone |
| MEGA | SDK em C++; crate `mega` não oficial; via rclone |
| pCloud, Box, Jottacloud | Público pequeno para o caso; via rclone ou WebDAV quando houver |

## 4. O que a trait `Remote` precisa ter

Desenhar para o menor denominador comum (Google Drive) e expor o resto como capacidade:

```rust
trait Remote {
    async fn put_immutable(&self, path: &str, data: Bytes) -> Result<()>; // idempotente; If-None-Match quando houver
    async fn get(&self, path: &str) -> Result<Bytes>;
    async fn get_range(&self, path: &str, range: Range<u64>) -> Result<Bytes>;
    async fn list(&self, prefix: &str) -> Result<Vec<Entry>>;             // nome, tamanho, versão (etag/rev)
    async fn put_head(&self, machine: &MachineId, data: Bytes, expected: Option<Version>) -> Result<Version>;
    async fn delete(&self, path: &str) -> Result<()>;                     // só GC/prune
    fn capabilities(&self) -> Capabilities; // cas, atomic_rename, consistent_list, max_object_size
}
```

- **Head sem CAS:** o arquivo do head leva um contador monotônico e um nonce da instalação. Se a máquina ler o próprio head remoto com contador maior do que o último que ela escreveu, há duas instalações com o mesmo ID (máquina clonada ou restaurada de imagem): parar e avisar. Isso funciona em qualquer destino; com CAS o aviso só fica mais rápido.
- **Prune sem CAS:** lock no estilo restic (escreve `locks/<machine>`, espera, relê e confere se é o único), feito raramente e só quando os heads de todas as máquinas conhecidas foram lidos.
- **Suíte de conformidade:** um conjunto de testes que roda contra cada conector (pasta local, MinIO, WebDAV em contêiner, SFTP em contêiner; Drive/OneDrive/Dropbox com conta de teste num job manual). Inclui: listar logo após escrever, arquivo pela metade, duplicata de nome, token expirado, 429 com `Retry-After`.

## 5. Encaixe nas fases do roadmap

| Fase | Conectores | Observação |
|---|---|---|
| 0 Fundação | Nenhum código; criar clients OAuth do Google (iniciar verificação), Microsoft Entra e Dropbox; confirmar cotas e escopos | O app do Google em "Testing" expira tokens em 7 dias, por isso começar cedo |
| 1 Núcleo e CLI | Pasta local, com escrita temp + rename e tolerância a cópias de conflito | Já serve para pasta do Dropbox/OneDrive/Drive desktop/iCloud/Syncthing |
| 3 Daemon | Documentar "pasta sincronizada" como caminho oficial | Uso pessoal PC ⇄ notebook já roda com nuvem no M2 |
| 4 Par-a-par | QUIC (já no roadmap) | Não muda |
| 5 Nuvem (M3) | Semana 1: trait `Remote` final + suíte de conformidade + S3 (MinIO, R2, B2, AWS). Semana 2: Google Drive. Semana 3: WebDAV/Nextcloud e SFTP | Se apertar, SFTP vai para a fase 7 |
| 7 Robustez | Rodar a suíte de caos também contra Drive e S3 reais; alertas de token | |
| 1.x | OneDrive, Dropbox nativos, rclone avançado | OneDrive primeiro, pelo público Windows |

Impacto no cronograma: nenhum se SFTP escorregar para a fase 7. Com os seis nativos dentro da fase 5, ela passaria de 3 para cerca de 5 semanas.

## 6. Pontos para o Yuri decidir

1. **Escopo do Drive:** `drive.file` com pasta visível (recomendado) ou `drive.appdata` oculta.
2. **OneDrive na 1.0 ou na 1.x:** recomendado 1.x, a não ser que o público Windows seja prioridade no beta.
3. **Ordem dentro da fase 5:** recomendado S3 antes do Drive, pela suíte de conformidade. Se o importante é o uso pessoal no Drive o quanto antes, inverter (a pasta do Google Drive para desktop cobre o intervalo no Windows/macOS, mas no Linux não existe cliente oficial).

## Fontes

- [Amazon S3: conditional writes](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html) e [anúncio de If-Match (nov/2024)](https://aws.amazon.com/about-aws/whats-new/2024/11/amazon-s3-functionality-conditional-writes)
- [Cloudflare R2: conditional operations](https://developers.cloudflare.com/r2/api/s3/conditional-operations/) e [preços](https://developers.cloudflare.com/r2/pricing)
- [Backblaze B2: API compatível com S3](https://www.backblaze.com/bb/docs/cloud-storage-s3-compatible-api) e [Proxmox desativando If-None-Match para B2](https://lists.proxmox.com/pipermail/pbs-devel/2025-July/014234.html)
- [Google Drive API v3: files.update](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/update)
- [Dropbox: modos de escrita e revisões (fórum oficial)](https://www.dropboxforum.com/t5/API-support/Using-file-revs-in-API-2/m-p/191299)
- [Apache OpenDAL: serviços](https://opendal.apache.org/docs/rust/src/opendal/services/mod.rs.html)
- Demais pontos (OAuth loopback do Google, expiração de 7 dias em "Testing", Graph `approot`, Nextcloud Login Flow v2, limites do GitHub) vêm da documentação dos provedores conhecida até 2026 e devem ser reconferidos na fase 0.
