# Etapa 1: hotfix 1.4.3 (M1) e preparação da 2.0 (M0)

**Estado:** aceito em 27/11/2026; implementado nos branches `hotfix/1.4.3` e `release/2.0` (ver `docs/historico/`).

**Fontes:**
- [plano-implementacao-2.0.md](../plano-implementacao-2.0.md), seções M0 e M1;
- [plano-proximas-versoes.md](../plano-proximas-versoes.md), tabelas B, V e D;
- a consulta à documentação de 27/11/2026, resumida em "Doc consultada" no fim.
  Ela mudou o plano em seis pontos, marcados com **[doc]**.

## Decisões desta etapa (rodada de perguntas)

| Pergunta | Resposta |
|---|---|
| Escopo | M1 (hotfix 1.4.3) e depois M0 |
| Sessões | **Uma** sessão de implementação, em sequência: primeiro a `hotfix/1.4.3`, depois a `release/2.0` |
| Lentes da revisão | unsafe/FFI e recursos por sistema; panics de dados externos e compatibilidade da config; consumo por quadro e parado; regra de negócio que os testes não pegam |
| Mac | **Pode dar push só do branch**, sem PR, para o CI do Mac rodar. Sem merge, sem tag, sem release |

## Ordem de trabalho da sessão de implementação

1. **Branch `hotfix/1.4.3`** a partir da `main` (`a93cea3`). PRs 10 a 16, um
   commit por camada (ver "Commits").
2. Suíte inteira verde (ver "Pronto"), `graphify update .`, revisão por
   workflow, correções e suíte de novo.
3. Push do `hotfix/1.4.3` para o CI do Mac. Os resultados entram no relatório.
4. **Branch `release/2.0`** a partir do `hotfix/1.4.3`, porque a 1.4.3 ainda
   não está na `main` e assim o M0 já parte dela. PRs 00 a 02.
5. Suíte, `graphify`, revisão, correções, push só do branch.
6. Relatório para a sessão pai.

**Fica fora desta etapa:**
- o `chore(release): 1.4.3` (bump de versão, notas e tag): só depois do seu
  aceite do resultado;
- qualquer merge na `main`.

## M1: o que cada PR faz (com as mudanças da doc)

**PR-10 `fix(letras): artistas e títulos com letras especiais não fecham o app`** (B1, B2)
- `src/lyrics/lrclib.rs`: trocar `to_lowercase` por `to_ascii_lowercase`
  **só** em `first_artist` (:135) e em `clean_title` (:112).
- **[doc]** **Não** trocar em `is_noise` (:74) nem em `channel_like` (:147). Ali o
  índice não é reaproveitado, e a lista `NOISE` tem "vídeo" e "áudio", que
  deixariam de casar com "VÍDEO".
- **Testes:** `"BİRİCİK, ŞEBNEM"`, `"İİİİ, x"`, o K de Kelvin (U+212A) com
  `" & "`, `'ẞ'` (U+1E9E) com `" feat. "`, e uma propriedade de que não há panic
  para qualquer texto montado com esses caracteres. O `"Straße"` não serve,
  porque `ß` não muda de tamanho.

**PR-11 `fix: subtração de Instant sem risco de panic`** (B3, agora de severidade baixa)
- **[doc]** Desde o rustc 1.97, o `Instant` do Windows começa em `u64::MAX/4` ns
  (rust-lang/rust#156177). Com o 1.100 local, `now() - 60s` logo após o boot
  **não** dá panic, e no Unix o `tv_sec` é `i64` com sinal. A correção continua
  valendo por três motivos:
  - os libs-api dizem que isso não é garantia da API;
  - o PR-29 vai declarar `rust-version = 1.88`, e com 1.88 a 1.96 o panic é real;
  - `Duration::from_secs_f32(inf)` também dá panic.
- Mudar o título do PR. Não tentar reproduzir pelo uptime.
- **Sites:**
  - `src/volume.rs:30`;
  - `src/overlay.rs:849,856,857,1234,1322,1986,1996`;
  - **[doc]** `src/settings.rs:2366` (`Instant::now() - from_secs_f32(t.max(0.0))`),
    que não estava no plano.
- Os campos "há muito tempo" viram `Option<Instant>`. O helper
  `elapsed_or_max(Option<Instant>) -> Duration` usa `checked_duration_since` ou
  `saturating_duration_since`, **não** o operador `-` (a doc avisa que o
  saturar pode voltar a dar panic numa versão futura).
- **[doc]** No lugar do grep proposto no plano, o lint
  `clippy::unchecked_time_subtraction` como `warn` (o nome antigo,
  `unchecked_duration_subtraction`, foi renomeado). Os sites do Mac e do Linux
  que não são bug (`mac/media.rs:206`, `mac/osd.rs:211`) ganham um
  `#[expect(...)]` com o motivo, ou são trocados pelo helper para padronizar.
- **Teste:** unitário do helper, com `None`, `Some(now)` e um valor no futuro.

**PR-12 `fix(config): config com erro não apaga mais as preferências`** (B4, B5, B6)

**Algoritmo de carga** (`src/config.rs::load`), conferido no fonte do toml 1.2.2
e do serde 1.0.229:
1. Tentar `toml::from_str::<Config>(&text)`. Se der certo, acabou.
2. Senão:
   - copiar **o arquivo que foi lido de fato** (o atual ou o `legacy_path`)
     para `config.toml.bad`. **Não sobrescrever** um `.bad` que já existe: usar
     `config.toml.bad.1`, `.2`… até 5;
   - ler com `text.parse::<toml::Table>()`;
   - **[doc]** se isso também falhar (erro de sintaxe), usar
     `Config::default()` e registrar no log.
3. Para cada `(k, v)` da `Table`, tentar `toml::Value::Table(valid + (k, v))`
   com `.try_into::<Config>()`. Se der certo, mantém; senão, registra a chave
   que caiu. Cada tentativa precisa de `clone()`, porque o `try_into` consome o
   valor. A ordem da `Table` é alfabética, o que só afeta a ordem do log.
4. **[doc]** Depois da carga tolerante, **não chamar `save()` de imediato**, para
   não gravar por cima a variante desconhecida. Só grava na próxima mudança feita
   pela pessoa.
5. **[doc] Downgrade 2.0 → 1.4.3:**
   - se o arquivo tem `version > CONFIG_VERSION`, **manter a `version` lida**:
     nunca rebaixar nem gravar um número menor;
   - assim a 2.0 não "remigra" um arquivo que a 1.4.3 regravou, e a 1.4.3 não
     apaga o número que a 2.0 precisa ver;
   - as chaves desconhecidas a 1.4.3 descarta, já que o `Config` não usa
     `deny_unknown_fields`;
   - os detalhes de como a 2.0 lida com isso ficam para o PR-40.

**Gravação** (`save()`):
- grava `config.toml.tmp` no **mesmo diretório** e chama `std::fs::rename`;
- **[doc]** no Windows, o `rename` do std usa `MoveFileExW(REPLACE_EXISTING)` e,
  se receber `ACCESS_DENIED`, tenta de novo com `FILE_RENAME_FLAG_POSIX_SEMANTICS`.
  Com um `SHARING_VIOLATION` ele devolve `Err` sem tentar de novo, e aí o certo é
  registrar no log e manter o `.tmp`;
- o nome do `.tmp` leva o PID, para não colidir entre threads e processos;
- o mesmo vale para `update.toml` e o cache de letras.

**Ordem e números:**
- `sanitize()` antes de qualquer `save()` (B5);
- `is_finite()` em todos os floats da config;
- **[doc]** também `lyrics.free_x` e `lyrics.free_y` (`src/lyrics/config.rs:97-98`).

**Testes:**
- TOML com uma variante desconhecida (`card_style = "animated"`): as outras
  chaves sobrevivem e não há `save()`;
- TOML com erro de sintaxe: padrões e `.bad`;
- `sensitivity_db = nan`;
- `version = 6` carregado pela 1.4.3: o número é mantido;
- um `.bad` que já existe não é sobrescrito;
- um `.tmp` de outra execução deixado para trás é ignorado.

**PR-13 `fix(letras,atualizacao): relógio e números inválidos`** (B7, B14, B15, B16)
- `lyrics/mod.rs:133`: offset `is_finite`, com `clamp(±600 000 ms)` aplicado
  **depois** da conversão para milissegundos.
- `update/mod.rs:566` e `lyrics/cache.rs:114`: `last > now` conta como vencido.
- `lyrics_overlay.rs:491/1933`: `bg_clock: f64`. **[doc]** A perda de precisão
  começa por volta de 36 h, não de 73 h.
- `update/sig.rs:16`: o `assert!` vira `Result`, propagado até quem chama, que
  trata como "assinatura inválida".

**PR-14 `fix(linux): capas enormes ou caminhos especiais`** (B8, B9)
- `src/linux/gfx.rs`:
  - um `read_capped(path)`, comum a `file://` e a caminho puro;
  - **[doc]** abrir com `OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK)`
    e conferir `f.metadata()?.is_file()` **depois** de abrir (sem `metadata`
    antes, por causa do TOCTOU); depois, `take(MAX_BYTES)`.
- **[doc] Limite de área** `w·h ≤ 4096²`, **mantendo** o `MAX_SIDE = 8192` por
  lado, para uma capa larga não ser recusada à toa:
  - PNG: logo depois de `decode_headers` (`gfx.rs:42-43`);
  - JPEG: mover o `dimensions()` para antes do `decode()`, porque hoje ele está
    depois (`gfx.rs:51-52`);
  - WebP: trocar a checagem por lado pela do produto (`gfx.rs:57-61`). O
    `set_memory_limit` não limita os pixels.
- `src/linux/media.rs:573`: uma thread de capas com fila de 1 (a URL nova
  substitui a pendente).

**PR-15 `fix(linux): ilha sobrevive a monitor removido e a compositores lentos`** (B10, B11)

**Recriar a superfície** (**[doc]**, segundo o wlr-layer-shell e o
wayland-client 0.31.15):
- **Superfície nova:** um `wl_surface` **novo**, porque o antigo já teve buffer
  e `get_layer_surface` daria `already_constructed`. Junto, um `wp_viewport` e
  um `wp_fractional_scale_v1` novos. Hoje o `_fractional` nunca é destruído.
- **Ordem de destruição:** layer_surface, viewport, fractional, wl_surface.
- **Depois de recriar:** `set_*` iniciais, região de entrada vazia, um commit
  sem buffer e a espera do `configure` de forma **assíncrona**. O `roundtrip`
  bloqueante de 3 s do `open()` não pode entrar no laço principal.
- **Estado:** zerar `frame_pending` e trocar o pool (`pool_gen+1`,
  `busy=[false;2]`).
- **Quando recriar:** só se houver algum `wl_output`. Sem nenhum, esperar o
  `Event::Global` de um output novo, para não entrar num laço de criar e fechar.

**Outputs:**
- tratar `Global`/`GlobalRemove` de `wl_output` no `Dispatch<WlRegistry,
  GlobalListContents>` (`wayland.rs:347`);
- guardar o `name` do registry por output (`Dispatch<WlOutput, u32>`);
- no remove: `release()` (versão ≥3), tirar a entrada de `outputs` e de
  `entered`, e chamar `rescale()`.

**B11:**
- zerar `configured` só quando o tamanho **lógico** mudar;
- **[doc]** isso não basta, porque a spec não promete `configure`. O que resolve
  é um `Instant` próprio para `configured`, com timeout de 500 ms;
- esse timeout **não vale** para uma superfície recém-criada: anexar buffer
  antes do primeiro `configure` é erro de protocolo.

**Teste:** à mão, no Sway (`swaymsg output <nome> disable`, depois `enable`) e no
KDE (desligar o monitor externo). No WSL não dá para testar Wayland real; fica
como "não verificado" se não houver máquina.

**PR-16 `fix(mac,atualizacao): aviso de versão não congela a ilha`** (B12, B13)
- **[doc] A referência certa é `src/mac/update.rs:15` e `:38`.** O arquivo tem
  54 linhas; as linhas 356 e 379 do plano estavam erradas.
- **O que congela de fato:**
  - o laço modal ignora mouse e teclado das outras janelas;
  - a fila principal é serial, então `media_changed`, `launchers_changed` e
  `config_changed` ficam esperando. Pôr os timers em common modes não basta.
- **A correção é um `NSPanel` próprio, não modal:**
  - `show()` retorna logo, e a escolha chega por target/action, com seletores
    novos no `define_class!` do `Target` (`mac/mod.rs:165`);
  - estilos `Titled | Closable | UtilityWindow`, nível `NSFloatingWindowLevel`;
  - `setHidesOnDeactivate(false)`: sem isso, o aviso some quando a pessoa clica
    em outro app, e sem Dock não há como trazer de volta;
  - fechar (Esc ou o x) conta como "Depois": `windowWillClose:` limpa o estado;
  - o painel e a `Offer` ficam num `thread_local` até a escolha, com uma guarda
    contra uma segunda oferta enquanto o painel está aberto;
  - `compat::activate(mtm)` antes do `makeKeyAndOrderFront`;
  - **não** usar `NSAlert::window()` sem `runModal`, porque isso não é
    documentado.
- **Timers:**
  - `src/mac/notch.rs:267`, `lyrics.rs:234` e `osd.rs:113` passam a usar
    `timerWithTimeInterval…` mais `NSRunLoop::mainRunLoop().addTimer_forMode(…,
    NSRunLoopCommonModes)`, o padrão que já existe em `mac/compat.rs:69-71`;
  - ganho extra: o tick continua com o menu da barra aberto. O `try_borrow_mut`
    do `notch::with` já protege contra reentrância. Conferir o mesmo em `lyrics`
    e `osd`;
  - a tolerância e o invalidate ficam para o PR-24. Avisar no commit que os
    dois vão mexer nas mesmas linhas.
- **[doc] B13 (`src/main.rs:156-166`):**
  - só o timeout de 15 s vira 60 s, **sem** o evento com nome;
  - o processo antigo já segura o mutex até morrer, e o novo já aceita
    `WAIT_ABANDONED`, o que vale também num abort;
  - a 1.4.2, que é o processo antigo na primeira atualização, não teria o
    evento de qualquer forma;
  - registrar no log quando o timeout estourar.
- Nenhuma feature nova no `Cargo.toml`: os defaults do objc2-app-kit e do
  objc2-foundation 0.3.2 já trazem `NSPanel`, `NSButton`, `NSRunLoop` e
  `NSTimer`.

## M0: o que cada PR faz (com as mudanças da doc)

**PR-00 `feat(diagnostico): medição de quadros e consumo`**

`src/stats.rs`:
- ligado por `TASKBEAT_FRAMESTATS=1`;
- quando desligado, custa uma leitura atômica por quadro;
- grava `stats.log` a cada 60 s.

`tools/measure.ps1`:
- **[doc]** registra **três** contagens separadas:
  - handles do kernel: `GetProcessHandleCount`, ou `Process.HandleCount`
    com `.Refresh()` antes de cada amostra;
  - GDI e USER: `GetGuiResources` com `GR_GDIOBJECTS`/`GR_USEROBJECTS` e os
    `_PEAK`, via `Add-Type` (P/Invoke);
- `GetGuiResources` devolve 0 tanto em erro quanto em zero objetos, então
  conferir o `GetLastError`;
- abre o processo com `PROCESS_QUERY_LIMITED_INFORMATION`.

Medição dentro do app:
- as funções ficam em `windows::Win32::System::Threading`, que já está ligada;
- para o working set, o `GetProcessMemoryInfo` pede a feature nova
  `Win32_System_ProcessStatus`. Uma alternativa é tirar o working set do script e
  não do app.

`tools/measure.sh` (Mac e Linux):
- RSS e número de threads pelo `ps`, e descritores abertos pelo `lsof`/`/proc/<pid>/fd`;
- não há equivalente de GDI.

**Entrega:** `docs/medicoes/1.4.2.md`, com a 1.4.2 medida no Windows (local) e no
Linux (WSL, se a ilha subir em X11 lá; senão, "não medido").

**PR-01 `ci: toolchain fixo, permissões, testes em debug e cargo-deny`**

`rust-toolchain.toml`:
- `[toolchain]` com `channel = "1.100.0"`, `profile = "minimal"` e
  `components = ["clippy", "rustfmt"]`;
- **[doc]** tirar `rustup update stable` e `rustup default stable` dos três
  workflows;
- pôr um passo `rustup toolchain install` seguido de `rustc -V`, antes do cache;
- não definir `RUSTUP_TOOLCHAIN`, que passa por cima do arquivo.

Workflows:
- `permissions: contents: read` no topo de `windows.yml` e `linux.yml`. O
  `macos.yml` já faz isso;
- `windows.yml` também em `push: main`;
- `cargo test --locked` em debug antes do `--release`. Conferir se o
  `timeout-minutes: 40` aguenta e subir se precisar;
- `cargo clippy --all-targets`, só reportando (sem `-D warnings`).

`cargo-deny`:
- **[doc]** só no `linux.yml`, num job próprio em ubuntu: a action é Docker e
  quebra no Windows e no Mac;
- uma matriz `checks: [advisories, "bans licenses sources"]`, com
  `continue-on-error` só no `advisories`, para um advisory novo não quebrar o PR;
- `deny.toml`:
  - `[licenses] allow` com os SPDX exatos do lock: `MIT`, `Apache-2.0`, `Zlib`,
    `Unlicense`, `Apache-2.0 WITH LLVM-exception` e `Unicode-3.0`. Sem
    `unlicensed`, `copyleft` nem `version`, que hoje dão erro;
  - `[sources] unknown-registry = "deny"` e `unknown-git = "deny"`, porque o
    padrão só avisa;
  - `[graph]` sem `targets`, ou com os três triples.

`build-linux.sh`:
- `cargo build --release --locked`;
- **[doc]** o appimagetool **1.9.1** em URL fixa, com o sha256 gravado no script
  e conferido baixando uma vez antes de gravar, porque o projeto não publica
  checksum;
- **[doc]** o **runtime type2** também fixo (`20251108/runtime-x86_64`, sha256
  `2fca8b44…ec260d`) via `--runtime-file`. Sem isso o appimagetool baixa o
  runtime "continuous" a cada build.

Template `.github/pull_request_template.md`, com o checklist da seção 0.3 do
plano.

**PR-02 `test: job de soak`**
- **[doc] 8 h não cabem num job hospedado** do GitHub, que tem limite de 6 h, e
  quebrar em vários jobs zera o processo a cada um. Nesta etapa:
  - **`soak.yml` hospedado de 5 h 30** (`timeout-minutes: 350`), com
    `strategy.fail-fast: false` e o log sempre enviado (`if: always()`);
  - **`tools/soak-local.ps1`/`.sh`** para as 8 h contínuas numa máquina sua.
  - Os critérios da 2.0 passam a dizer "soak de 5 h 30 no CI e 8 h local".
- **[doc]** O `workflow_dispatch` só aparece com o arquivo **na `main`**. O
  `soak.yml` recebe o input `alvo` (o ref a compilar e testar, padrão
  `v1.4.2`) e faz checkout desse ref. Por isso o PR-02 vai para a **main**
  junto com a 1.4.3, não para a `release/2.0`. A sessão deixa o commit pronto no
  `hotfix/1.4.3`.
- **[doc]** O teste roda o **binário de `cargo build --release`**, porque os
  testes ignoram o `panic = "abort"`.
- **Fonte de música no CI:** o Mac e o Linux já têm (`macos/ci-nowplaying.swift`,
  `linux/ci-mpris.py`). **[doc]** O Windows não tem "SMTC de teste": criar
  `examples/ci_player.rs`, que toca um WAV em loop pelo
  `Windows.Media.Playback.MediaPlayer` e assim aparece no SMTC (feature
  `Media_Playback`). Se o runner sem áudio não deixar, o soak do Windows mede só
  o app parado, e isso vai para o relatório.

## Pronto (definição desta etapa)

- [ ] **Suíte inteira verde, uma por vez na máquina, nesta ordem:**
  1. `cargo test --locked` e `cargo test --locked --release` no Windows;
  2. `cargo clippy --all-targets --locked` no Windows e com
     `--target aarch64-apple-darwin`, sem aviso novo em relação à `main`;
  3. no WSL Ubuntu, `cargo test --locked` (debug e release);
  4. o CI do Mac no branch, depois do push, com o resultado registrado.
- [ ] `graphify update .` rodado ao fim de cada branch. O `graphify-out/` vai
      para o `.gitignore` (é gerado) e não é commitado.
- [ ] Docs:
  - `docs/historico/<área>.md` para cada área tocada (`letras`, `config`,
    `linux`, `mac`, `atualizacao`, `ci`, `diagnostico`);
  - um `CLAUDE.md` na raiz com as regras do projeto que esta etapa firmou
    (carga tolerante da config, `Option<Instant>`, nada de `to_lowercase` para
    fatiar, o soak);
  - as correções nos dois planos (as linhas do `mac/update.rs`, o B3, o soak).
- [ ] **Commits** por camada, no padrão do repositório, cada um terminando com
      `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`:
  - `fix(...)`: código, um por PR do plano;
  - `test(...)`: testes, se não couberem junto;
  - `ci(...)`: workflows e scripts;
  - `docs(...)`: histórico, `CLAUDE.md` e planos, com `[skip ci]`.
- [ ] **Revisão:** um `Workflow` com 4 revisores (as lentes acima) e até 3
      verificadores por achado, que tentam refutar. Entra no relatório só o que
      sobreviveu. Os revisores não rodam testes.
- [ ] **Relatório final** com:
  - o que mudou;
  - a config (formato e compatibilidade);
  - os achados corrigidos e os descartados;
  - o que não foi verificado (Wayland real, o soak do Windows no CI);
  - o nome e o hash dos dois branches.

## Fora do escopo
- Bump de versão, notas da 1.4.3, tag e release.
- Merge em `main`, PR aberto, deploy, publicação da extensão.
- Qualquer item do M2 em diante, inclusive `[lints]` e `rust-version`, que
  ficam para o PR-29. O lint `unchecked_time_subtraction` do PR-11 entra só
  como atributo local.

## Doc consultada

Consulta de 27/11/2026, feita por 6 pesquisadores e 1 crítico, na documentação
oficial e no fonte local das versões do `Cargo.lock`. Abaixo, só o que o crítico
confirmou.

| PR | Fonte | O que ela diz | Efeito |
|---|---|---|---|
| 10 | [str::find](https://doc.rust-lang.org/std/primitive.str.html#method.find), [char::to_lowercase](https://doc.rust-lang.org/std/primitive.char.html#method.to_lowercase) | o índice é em bytes; 'İ' vira "i̇" | o defeito existe; a troca fica restrita a 2 funções |
| 10 | [str::to_ascii_lowercase](https://doc.rust-lang.org/std/primitive.str.html#method.to_ascii_lowercase) | "non-ASCII letters are unchanged" | preserva os offsets |
| 11 | `library/std/src/sys/time/windows.rs` (rustc 1.100.0, L41-44); rust-lang/rust#156142 e #156177 | o Instant soma `u64::MAX/4` desde a 1.97 | o B3 vira defensivo, de severidade baixa |
| 11 | [Instant, Monotonicity](https://doc.rust-lang.org/std/time/struct.Instant.html) | "future Rust versions may reintroduce panics" | usar `checked_duration_since` no helper |
| 11 | `sys/pal/unix/time.rs` L27, L159 | `tv_sec: i64` | Mac e Linux não são crash |
| 11 | clippy 1.100 (tabela de lints renomeados) | `unchecked_duration_subtraction` → `unchecked_time_subtraction` | o lint entra no lugar do grep |
| 12 | `std/src/sys/fs/windows.rs` L1306-1340 (1.100.0); [fs::rename](https://doc.rust-lang.org/std/fs/fn.rename.html) | `MoveFileExW(REPLACE_EXISTING)`, com reserva POSIX em `ACCESS_DENIED` | tmp no mesmo diretório; `SHARING_VIOLATION` vira erro tratado |
| 12 | `serde_derive-1.0.229/src/de/struct_.rs:248-251, 289`; [container attrs](https://serde.rs/container-attrs.html) | `default` só cobre campo ausente; chave desconhecida é ignorada | algoritmo chave a chave; a `version` é mantida no downgrade |
| 12 | `toml-1.2.2/src/table.rs:8-13, 35-57`; `value.rs:67-72` | `Table: FromStr` e `try_into` consomem por valor; ordem alfabética | `clone()` por tentativa; ramo de sintaxe inválida |
| 12, 40 | [field attrs](https://serde.rs/field-attrs.html), [variant attrs](https://serde.rs/variant-attrs.html) | `alias` é do mesmo tipo; `other` só para enums com tag | sem `alias` bool→enum no PR-40; sem `other` |
| 13 | `src/update/sig.rs:16`; `lyrics_overlay.rs:491`; [f32](https://doc.rust-lang.org/std/primitive.f32.html) | `assert!` confirmado; ulp do f32 de 15,6 ms em 2^17 s | o B15 começa por volta de 36 h |
| 14 | `zune-png-0.4.10/src/decoder.rs:404`, `zune-jpeg-0.4.21/src/decoder.rs:734`, `image-webp-0.2.4/src/decoder.rs:336-345, 551-556` | há dimensões antes de decodificar; o `set_memory_limit` do webp não cobre pixels | limite de área manual; o `dimensions()` do JPEG vem antes |
| 14 | [OpenOptionsExt](https://doc.rust-lang.org/std/os/unix/fs/trait.OpenOptionsExt.html) | `custom_flags` | `O_NONBLOCK` mais `is_file` depois de abrir |
| 15 | `wayland-protocols-wlr-0.3.12/.../wlr-layer-shell-unstable-v1.xml:45-53, 122-135, 300-350`; [wayland.app](https://wayland.app/protocols/wlr-layer-shell-unstable-v1) | `closed` permite recriar com um surface novo; set_size não promete configure | recriação completa; timeout de 500 ms só depois do primeiro configure |
| 15 | `wayland-client-0.31.15/src/globals.rs:356+`, `wayland.xml:1487, 1788, 3078` | Dispatch de `GlobalListContents`; ordem de destroy; `release` | tratar o output removido; `entered` e `rescale` |
| 16 | [NSApplication.runModal(for:)](https://developer.apple.com/documentation/appkit/nsapplication/runmodal(for:)), [main queue](https://developer.apple.com/documentation/dispatch/dispatch_get_main_queue), [beginSheet](https://developer.apple.com/documentation/appkit/nswindow/beginsheet(_:completionhandler:)), [hidesOnDeactivate](https://developer.apple.com/documentation/appkit/nswindow/hidesondeactivate) | o laço modal ignora outros eventos; a fila é serial; a folha exige janela-mãe | `NSPanel` não modal com `hidesOnDeactivate(false)` |
| 16 | `objc2-app-kit-0.3.2/src/generated/NSPanel.rs`, `NSWindow.rs:33-54, 271, 1073`, `NSButton.rs:164`; `objc2-foundation-0.3.2/.../NSTimer.rs`, `NSRunLoop.rs` | os nomes das APIs e as features padrão | sem feature nova |
| 16 | [Mutex objects](https://learn.microsoft.com/en-us/windows/win32/sync/mutex-objects), [CreateEventW](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createeventw); `src/main.rs:156-166, 213-216` | o mutex abandonado sinaliza; eventos e mutexes dividem o namespace | o B13 fica só com o timeout de 60 s |
| 00 | [GetGuiResources](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getguiresources), [GetProcessHandleCount](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesshandlecount), [Process.HandleCount](https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.process.handlecount) | 0 é ambíguo; os handles do kernel são separados; o valor fica em cache | três contagens; `Refresh()`; `GetLastError` |
| 01 | [rustup overrides](https://rust-lang.github.io/rustup/overrides.html), [profiles](https://rust-lang.github.io/rustup/concepts/profiles.html) | formato do `[toolchain]`; o `RUSTUP_TOOLCHAIN` vence o arquivo | o `rust-toolchain.toml` descrito acima |
| 01 | [workflow syntax](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax) | o `permissions` do job substitui o do topo | bloco no topo |
| 01 | [cargo-deny-action action.yml](https://github.com/EmbarkStudios/cargo-deny-action), [licenses](https://embarkstudios.github.io/cargo-deny/checks/licenses/cfg.html), [sources](https://embarkstudios.github.io/cargo-deny/checks/sources/cfg.html), [advisories](https://embarkstudios.github.io/cargo-deny/checks/advisories/cfg.html) | a action é Docker; chaves removidas; fontes desconhecidas só avisam | só no Linux; `deny.toml` descrito acima |
| 01 | [appimagetool](https://github.com/AppImage/appimagetool), [releases](https://github.com/AppImage/appimagetool/releases) | baixa o runtime mais recente; sem checksum publicado | versão 1.9.1 e runtime fixos, com hash |
| 01, 02 | [cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html) | "Tests… ignore the `panic` setting" | o soak usa o binário de release |
| 02 | [Actions limits](https://docs.github.com/en/actions/reference/limits), [triggers](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows), [hosted runners](https://docs.github.com/en/actions/concepts/runners/github-hosted-runners) | 6 h por job; `workflow_dispatch` só no branch padrão; cada job é uma VM nova | 5 h 30 no CI, 8 h local; o `soak.yml` vai para a main |

**O que ninguém conferiu (a sessão de implementação confere ou deixa no
relatório como não verificado):**
- se o binário publicado da 1.4.2 foi compilado com rustc ≥ 1.97;
- se o runner do Windows hospedado toca áudio para o SMTC;
- se o `timeout-minutes: 40` aguenta os testes em debug mais release;
- o teste à mão da recriação no Sway e no KDE;
- a reentrância do tick do `lyrics` e do `osd` com os timers em common modes.
