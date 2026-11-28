# Taskbeat 2.0: plano de implementação

Este documento diz **como** e **em que ordem** fazer a 2.0. O **quê** e o
**porquê** estão em [plano-proximas-versoes.md](plano-proximas-versoes.md), e os
códigos usados aqui vêm de lá:

| Código | O que é |
|---|---|
| B | bugs |
| V | vazamentos |
| D | desempenho |
| A | APIs depreciadas ou desaconselhadas |
| F | fundações técnicas |
| 1 a 12 | os recursos pedidos |

O plano tem cerca de 60 PRs em 9 marcos. Cada PR trata de um tema só, dá para
revisar numa sentada e passa sozinho no CI.

---

## 0. Antes de começar

### 0.1 Decisões adotadas

São as recomendações do plano. Para mudar uma, edite aqui antes de chegar ao
marco correspondente.

| # | Decisão | Adotado | Afeta |
|---|---|---|---|
| D-1 | Quando o widget conta como "nada tocando" (item 9) | Sem sessão de mídia, **ou** pausado há mais de `idle_after_min` minutos (padrão 5; 0 = só sem sessão) | PR-52 |
| D-2 | Playlists e busca (itens 1 e 2) | **Abrir no player**, sem Web API nem login; resultados no app só no Apple Music do Mac | M6 |
| D-3 | Fundo animado (item 5) | **Opção**, desligada por padrão (protege o CPU de quem atualiza) | PR-62 |
| D-4 | Tutorial no Linux (item 11) | **Dicas na ilha** | PR-85 |
| D-5 | Estratégia de panic | **Manter `panic = "abort"`** e remover a falsa recuperação | PR-29 |
| D-6 | Formatação | **Adotar o rustfmt** com `max_width = 160`, num commit único | PR-34 |
| D-7 | Correções urgentes | **Lançar a 1.4.3 na `main`** antes da 2.0 | M1 |

### 0.2 Branches e versões

```
main ─────●─────────────●(1.4.3)─────────────────────────────●(2.0.0)
           \            ↑ hotfix/1.4.3                       ↑
            └ release/2.0 ──●beta.1──●beta.2──●beta.3──●rc.1─┘
                 ↑ PRs: feat/…, fix/…, perf/…, refactor/…, ci/…
```

- A `release/2.0` nasce da `main` no M0. Todo PR da 2.0 aponta para ela.
- A `hotfix/1.4.3` nasce da `main`. Depois de lançada, é mergeada na `main` **e**
  na `release/2.0`.
- Toda sexta (ou ao fechar um PR grande), a `main` é mergeada de volta na
  `release/2.0`, para os conflitos não se acumularem.
- **Versões:**
  - o `Cargo.toml` da `release/2.0` fica em `2.0.0-beta.N`;
  - a extensão sobe para `1.2.0` no M6 (comandos novos `focus` e `navigate`).
- **Betas:** saem como *prerelease* no GitHub, e o atualizador só as entrega a
  quem marcou "receber as versões de teste" (`update/mod.rs:110`).

### 0.3 Convenções de PR
- **Título** no padrão do repositório: `tipo(área): descrição em português`.
  Exemplos: `fix(letras): …`, `perf(mac): …`, `feat(card): …`.
  - Tipos: `feat`, `fix`, `perf`, `refactor`, `ci`, `docs`, `chore`, `test`.
  - Áreas: `windows`, `mac`, `linux`, `card`, `widget`, `ilha`, `letras`,
    `config`, `render`, `atualizacao`, `extensao`, `ci`.
- **Corpo:** o código do plano que o PR fecha (por exemplo, "Fecha B1, B2") e,
  em PRs `perf`, a medição de antes e depois.
- **Checklist de todo PR** (vai no template, `.github/pull_request_template.md`,
  criado no PR-01):
  - [ ] `cargo test` (debug e release) passa no(s) sistema(s) afetado(s)
  - [ ] Textos novos nas 4 línguas
  - [ ] Capturas atualizadas, quando muda o visual
  - [ ] Medição antes e depois, quando é `perf` ou quando o efeito roda por quadro
  - [ ] Config: campo novo com default e teste de migração
  - [ ] Nada de `unwrap`/`expect`/índice sobre dados externos
- **Tamanho:**

  | Tamanho | Linhas alteradas, fora o que é só mover |
  |---|---|
  | P | menos de 300 |
  | M | 300 a 1000 |
  | G | mais de 1000; quebrar se der |

---

## Módulos e interfaces novos

Estes são os contratos que vários PRs compartilham. Definir cedo evita que cada
PR invente o seu.

```rust
// src/win/guard.rs (PR-20). RAII para os recursos do Win32.
pub struct Dc(HDC);              // Drop: DeleteDC
pub struct WindowDc(HWND, HDC);  // Drop: ReleaseDC
pub struct Gdi<T: Into<HGDIOBJ> + Copy>(T); // Drop: DeleteObject
pub struct Icon(HICON);          // Drop: DestroyIcon
pub struct ComInit;              // new(): CoInitializeEx; Drop: CoUninitialize
pub struct Local<T>(*mut T);     // Drop: LocalFree
```

```rust
// src/stats.rs (PR-00). Ligado por TASKBEAT_FRAMESTATS=1; sem custo se desligado.
pub struct FrameStats { /* histograma de µs por superfície */ }
impl FrameStats {
    pub fn frame(&mut self, surface: &'static str, draw: Duration, present: Duration);
    pub fn wake(&mut self, source: &'static str); // despertares por segundo
}
```

```rust
// src/anim.rs (PR-41). Mola criticamente amortecida.
pub struct Spring { pub x: f32, pub v: f32, pub target: f32, omega: f32 }
impl Spring {
    pub fn new(x: f32, response_s: f32) -> Self;  // response ≈ tempo até ~95%
    pub fn set(&mut self, target: f32);           // retarget mantém a velocidade
    pub fn step(&mut self, dt: f32) -> bool;      // true = ainda se movendo
}
```

```rust
// src/config.rs (PR-40)
pub enum ArtClick   { OpenPlayer, OpenCard, Nothing }       // default OpenPlayer
pub enum IdleWidget { Full, Compact, Hidden }               // default Compact
pub enum Motion     { Full, Reduced, Off }                  // default Full (respeita o sistema)
pub enum CoverViz   { Bars, Vinyl, None }                   // default Bars
pub enum SourceMode { All, Only, Except }
pub struct SourceFilter { pub mode: SourceMode, pub ids: Vec<String> }
pub struct SeenSource   { pub id: String, pub name: String, pub last_seen: u64 }
pub struct Pinned       { pub name: String, pub uri: String, pub player: PlayerKind }
```

```rust
// src/media_filter.rs (PR-53). Comum aos três sistemas, puro e testável.
pub fn allowed(f: &SourceFilter, id: &str, site: Option<&str>) -> bool;
```

```rust
// src/library/ (PR-71..73)
pub enum PlayerKind { Spotify, YouTubeMusic, AppleMusic, Mpris(String), Custom }
pub fn parse_link(url: &str) -> Option<Pinned>;                // links → Pinned
pub fn search_target(kind: &PlayerKind, q: &str) -> Target;    // URI/URL já escapados
pub enum Target { Uri(String), Extension { url: String }, AppleScript(String) }
pub trait Provider: Send {
    fn kind(&self) -> PlayerKind;
    fn open(&self, t: &Target) -> Result<(), LibError>;
    fn playlists(&self) -> Option<Vec<PlaylistRef>> { None }  // só onde o sistema dá
    fn search(&self, _q: &str) -> Option<Vec<TrackRef>> { None }
}
```

```rust
// render.rs (PR-42)
pub fn draw_image_rotated(cv: &mut Canvas, img: &Image, c: (f32, f32), r: f32, angle: f32, alpha: f32);
pub struct Mip { levels: Vec<Image> }      // caixa 2×2 em cascata
impl Mip { pub fn level_for(&self, dst_px: f32) -> &Image; }
// lyrics_bg.rs
pub struct BgCache { /* grade, buffers reaproveitados, crossfade */ }
pub fn fill_rect(cv: &mut Canvas, r: RectF, radius: f32, art: &Image, t: f64, mix: f32, c: &mut BgCache);
```

---

## M0 — Preparação

**Objetivo:** medir a 1.4.2 e ligar o CI antes de mudar qualquer coisa.

**PR-00 `feat(diagnostico): medição de quadros e consumo`** — M
- **Código:**
  - `src/stats.rs` (interface acima);
  - chamadas no laço do `overlay.rs` (desenho e `UpdateLayeredWindow`), em
    `mac/notch.rs` (`present`) e em `linux/island.rs`;
  - `wake()` nas threads que acordam sozinhas: áudio, mídia, mixer, vsync e
    timers do Mac.
- **Saída:** `stats.log` na pasta da config a cada 60 s, com p50/p95/p99 por
  superfície e despertares por segundo.
- **Script** `tools/measure.ps1` (e um `.sh` para o Mac e o Linux):
  - roda 5 min em cada cenário (parado, tocando, card aberto, mixer, letra em
    tela cheia);
  - registra CPU, working set, handles GDI e USER (`GetGuiResources`) e o número
    de threads.
- **Entrega:** `docs/medicoes/1.4.2.md`, a base de comparação de todo PR `perf`.
- **Teste:** com a variável desligada, nenhum custo (um `if` sobre um
  `OnceLock<bool>`).

**PR-01 `ci: toolchain fixo, permissões, testes em debug e template de PR`** — P
- **Toolchain:** `rust-toolchain.toml` com `channel = "1.100.0"` e os componentes
  `clippy` e `rustfmt`.
- **Workflows** (`windows.yml`, `linux.yml`, `macos.yml`):
  - `permissions: contents: read`, abrindo só o necessário no job de release;
  - `windows.yml` também em `push: main`;
  - um passo `cargo test`, em debug, antes do `--release`;
  - `cargo clippy --all-targets`, **sem** `-D warnings` por enquanto (só
    reporta);
  - `cargo deny check` com `deny.toml` (licenças MIT/Apache/BSD/ISC/Zlib/
    Unicode, advisories e fontes só do crates.io).
- **Linux:**
  - `build-linux.sh` com `cargo build --release --locked`;
  - o `appimagetool` com versão fixa e `sha256sum -c`.
- **Template:** `.github/pull_request_template.md` com o checklist da 0.3.

**PR-02 `test: job de soak (5 h 30 no CI, 8 h local)`** — P
- Um workflow `soak.yml`, disparado à mão (`workflow_dispatch`), nos três
  sistemas.
- Ele toca uma faixa em loop com o player de teste do CI (`macos/ci-nowplaying.swift`,
  `linux/ci-mpris.py`; no Windows, o mesmo via SMTC de teste), com o
  `measure` a cada 5 min.
- **Falha se** a regressão linear da memória ou dos handles tiver inclinação
  acima de 1 MB/h ou 10 handles/h.
- Roda pela primeira vez na 1.4.2, para registrar a base.

**Fim do M0:** o CI roda o clippy só para reportar; existem as medições e o soak
da 1.4.2.

---

## M1 — Hotfix 1.4.3 (branch `hotfix/1.4.3`, a partir da `main`)

**Objetivo:** tirar os panics e as perdas de dados de quem está na 1.4.x.

**PR-10 `fix(letras): artistas e títulos com letras especiais não fecham o app`** — P · B1, B2
- `lyrics/lrclib.rs`: em `first_artist` e `clean_title`, trocar
  `to_lowercase()` por `to_ascii_lowercase()`.
- **Testes:** `"BİRİCİK, ŞEBNEM"`, `"İİİİ, x"`, `"Straße feat. X"`,
  `"KELVIN & Y"` (com o K de Kelvin), e uma propriedade: para todo `&str`, as
  duas funções não dão panic. Um gerador simples com os caracteres que mudam de
  tamanho no `to_lowercase`.

**PR-11 `fix: subtração de Instant sem risco de panic`** — P · B3 (severidade baixa: ver a etapa 1)
- `volume.rs:30`, `overlay.rs:849,856,857,1234,1322,1986,1996` e `settings.rs:2366`: os campos
  `Instant` que guardam "há muito tempo" viram `Option<Instant>`, com `None`
  querendo dizer "nunca".
- Helper `fn elapsed_or_max(t: Option<Instant>) -> Duration`.
- **Teste:** unitário do helper, e o lint `clippy::unchecked_time_subtraction`
  como `warn` no lugar do `grep`.

**PR-12 `fix(config): config com erro não apaga mais as preferências`** — M · B4, B5, B6
- **Gravação atômica:** `config.rs::save()` grava `config.toml.tmp` e chama
  `std::fs::rename`. No Windows o `rename` substitui o destino (`MoveFileExW`
  com `REPLACE_EXISTING`). O mesmo para `update.toml` e o cache de letras.
- **Carga tolerante:** em `load()`, se `toml::from_str` falhar:
  1. copiar o arquivo para `config.toml.bad`;
  2. ler como `toml::Table`;
  3. manter as chaves que, uma a uma, ainda deserializam;
  4. registrar no log quais caíram.

  É isso que permite voltar da 2.0 para a 1.4.3 sem perder tudo.
- **Ordem:** `sanitize()` antes de `save()`.
- **Números:** `is_finite()` em todo `f32`/`f64` da config (`sensitivity_db` e
  outros).
- **Testes:**
  - TOML com uma variante desconhecida (`card_style = "Animated"`): as outras
    chaves continuam;
  - `sensitivity_db = nan`;
  - gravação interrompida, simulada com um arquivo `.tmp` deixado para trás.

**PR-13 `fix(letras,atualizacao): relógio e números inválidos`** — P · B7, B14, B15, B16
- `lyrics/mod.rs:133`: o offset precisa ser `is_finite`, com `clamp(±600 000 ms)`.
- `update/mod.rs:566` e `lyrics/cache.rs:114`: `last > now` conta como vencido.
- `lyrics_overlay.rs:1933`: `bg_clock` vira `f64`.
- `update/sig.rs:16`: o `assert!` vira um `Result` propagado.

**PR-14 `fix(linux): capas enormes ou caminhos especiais`** — P · B8, B9
- `linux/gfx.rs`:
  - um `read_capped(path)`, comum a `file://` e a caminho puro, com
    `is_file()` e `take(MAX_BYTES)`;
  - limite de pixels antes de decodificar (os decodificadores dão as dimensões
    no cabeçalho).
- `linux/media.rs:573`: uma thread de capas com fila de 1 (a URL nova substitui
  a pendente).

**PR-15 `fix(linux): ilha sobrevive a monitor removido e a compositores lentos`** — M · B10, B11
- `wayland.rs`:
  - tratar `wl_output` em `global`/`global_remove`;
  - no `closed`, recriar a layer-surface no output que sobrou, em vez de sair;
  - `configured = false` só quando o tamanho lógico muda, com timeout de 500 ms.
- **Teste:** à mão no Sway (`swaymsg output … disable`) e no KDE.

**PR-16 `fix(mac,atualizacao): aviso de versão não congela a ilha`** — M · B12, B13
- `mac/update.rs:15` e `:38`:
  - um `NSPanel` não modal no lugar do `runModal`;
  - os timers de `notch.rs:267`, `lyrics.rs:234` e `osd.rs:113` registrados em
    `NSRunLoopCommonModes`.
- `main.rs:163`: espera de 60 s pelo processo antigo, **sem** evento com nome (o
  mutex abandonado já libera a espera, até num abort).

**Lançamento 1.4.3:**
- `chore(release): 1.4.3` com a versão no `Cargo.toml` e no `app.rc`;
- notas nas 4 línguas;
- merge da `hotfix/1.4.3` na `main` e na `release/2.0`.

---

## M2 — Qualidade (na `release/2.0`)

**Objetivo:** gastar menos parado, sem vazamentos, com o clippy limpo e o código
dividido. **Nenhum recurso novo.** Os PRs 20 a 28 são independentes entre si e
podem correr em paralelo (Windows, Mac e Linux não se tocam).

### Recursos e vazamentos

**PR-20 `refactor(windows): guardas RAII para GDI, COM e LocalFree`** — M · V3, V4, V5
- Novo `src/win/guard.rs` (interface acima), aplicado em:
  - `webplayer.rs:530` (o `?` que vazava);
  - `webext.rs:506` (`LocalFree`), com um evento reaproveitado por direção em
    `Pipe::io`;
  - todas as threads com `CoInitializeEx`: `lyrics_overlay.rs:550,904`,
    `overlay.rs:240`, `media.rs:71`, `taskbar.rs:238,275` e `mixer.rs:44`.
- Trocar os pares manuais `CreateCompatibleDC`/`DeleteDC` nos módulos que
  tocar, sem caçar todos de uma vez.
- **Teste:** no soak, os handles GDI ficam estáveis.

**PR-21 `fix(windows): assinaturas do SMTC e redesenho a cada 2 s`** — M · V1, V2, D6
- `media.rs:506`: se alguma das três assinaturas falha, remover as que deram
  certo.
- `media.rs:481`: guardar a `Session` assinada e comparar pela identidade COM,
  não só pelo AUMID.
- `media.rs:129`: `media_version` só sobe quando `changed`. A posição ganha um
  `position_version` próprio, e só a barra de progresso (o *partial present* que
  já existe) o consulta.
- **Medição:** redesenhos por minuto com música pausada; o alvo é 0.

**PR-22 `perf(windows): menos despertares parado`** — M · D5, D7, V6, V7, A3
- `audio.rs:300`: loopback com `AUDCLNT_STREAMFLAGS_EVENTCALLBACK` e
  `WaitForSingleObject(evento, 200 ms)`, no lugar do `sleep(10ms)`.
- `taskbar.rs:74,374`, `overlay.rs:1535`: o hwnd da barra em cache, e o
  `keep_on_top` só em `WM_WINDOWPOSCHANGED`, na ativação ou num timer de 2 s.
- `mixer.rs:332-361`: podar `done` e `asked` contra as sessões vivas a cada
  refresh.
- `vsync.rs:159`: se `WaitForVBlank` falhar, descartar o `IDXGIOutput` e
  recriar.
- `launchers.rs:216` e `mixer.rs:342`: MTA (`COINIT_MULTITHREADED`). Conferir
  antes que as APIs usadas aceitam MTA; o shell às vezes exige STA, e nesse caso
  usar `CoWaitForMultipleHandles`.
- **Medição:** despertares por segundo parado, antes e depois.

**PR-23 `perf: chaves de cache sem alocar e hash sem format!`** — M · D8, D9, D10
- `text.rs:367,391`, `mac/text.rs:205` e `linux/text.rs:277`: a chave do cache
  vira `(u64 /* hash FxHash-like do texto */, font, w, h, align)`, e o texto
  real fica no valor para conferir colisões.
- `notch.rs:731` e `linux/island.rs:532`: `#[derive(Hash)]` em `Interact`.
- `notch.rs:451`: um `Vec` de barras guardado na struct.
- **Teste:** um teste de alocação (um `GlobalAlloc` contador, só em teste) que
  confirma zero alocações num quadro do widget parado.

**PR-24 `perf(mac): fim do osascript a cada 3 s e timers o dia todo`** — M · D1, D4, D19, D20, D21, D22
- `mac/media.rs:228`: a reserva por AppleScript só roda se:
  - o adaptador nunca publicou nada nesta sessão; **ou**
  - `NSRunningApplication::runningApplicationsWithBundleIdentifier` acha o
    Spotify ou o Music.

  Com backoff de 3 s a 30 s enquanto não acha nada.
- `mac/media.rs:659`: `osascript` com `spawn` e `wait` com timeout de 3 s, mais
  `kill`.
- **Timers:** `setTolerance(0.1–0.25)`. O OSD é invalidado com
  `caps_lock_osd = false`, e a letra quando a janela não está visível.
- `mac/media.rs:436`: comparar a capa base64 por hash e tamanho, sem clonar.
- `mac/audio.rs:313`: `AudioObjectAddPropertyListenerBlock` no dispositivo
  padrão.
- `mac/volume.rs:321`: juntar as gravações de volume (no máximo uma a cada 30
  ms).
- `mac/settings.rs:645`: `refresh()` pula quando `!isVisible`.
- **Medição:** processos criados por hora parado; o alvo é 0.

**PR-25 `perf(mac): quadro sem cópias e sem acumular memória`** — M · D2, D3, V8, V9
- `mac/gfx.rs:19`:
  - `CGDataProviderCreateWithData` com callback de liberação, entregando o
    `Vec` (uma cópia a menos e nenhum `CFData`);
  - o `CGColorSpace` sRGB num `thread_local`;
  - a soundness: conferir `px.len() >= w*h` (é a 1.5.2 do plano).
- `mac/lyrics.rs:341,781`: o fundo animado em tela cheia a 12 fps, compondo o
  texto sobre o último fundo.
- `autoreleasepool` em `mac/launchers.rs:87` (cada `scan`), `mac/media.rs:618`
  (`app_info`) e `mac/gfx.rs:92` (`decode_with_nsimage`).
- `mac/text.rs:167`: orçamento do cache de 16 MB, medido em bytes.

**PR-26 `perf(linux): X11 com SHM, menos commits e laços que não giram`** — G · D11-D14, V10-V12, A9, A10
- `x11.rs:213`: buffer guardado na struct; feature `shm` do x11rb, usando
  `shm_put_image` quando o servidor tem MIT-SHM e o `put_image` de hoje como
  reserva.
- `wayland.rs:323`, `island.rs:634`: a região de entrada dentro de `present`.
- `media.rs:379`: `POLLHUP`/`POLLERR` reconecta, com backoff.
- `audio.rs`:
  - `pa_stream_unref` quando o contexto falha;
  - podar `monitors`;
  - cache pid→ancestral;
  - timer pausado sem barras visíveis.
- `gfx.rs:176`, `system.rs:146`: `wait()` depois do `kill`, e o `xdg-open`
  esperado numa thread.
- `mod.rs:81`: `sigaction`, com o pipe de wake criado antes.
- `system.rs:63`: o portal numa thread de fundo, assinando `SettingChanged`.
- **Dá para quebrar em dois:** X11/Wayland (D11, D12) e o resto.

**PR-27 `perf(render): blur O(1), pesos sem alocar, FFT sem sqrt`** — M · D15-D18
- `render.rs:208` `blurred`: soma corrente deslizante em inteiro (`u32` por
  canal).
- `render.rs:126` `resampled`: pesos num array plano e buffers de rascunho
  reaproveitados. Antes, confirmar com o `stats` se o destino anima.
- `lyrics_bg.rs:132`: os três buffers dentro do `Cache`, e a distância comparada
  ao quadrado.
- `dsp.rs:105`: `norm_sqr` com `5·log10` no lugar de `norm` com `20·log10`.
- **Testes:**
  - os benches existentes (`fundo_custo`), com o antes e o depois no PR;
  - comparar a saída pixel a pixel com a implementação antiga, com tolerância
    de ±1 por canal.

**PR-28 `refactor: APIs desaconselhadas e código duplicado`** — M · A2, A5, A6, A7, A8, V13
- `appicon.rs:302`: `SHDefExtractIconW`, ou o `IShellItemImageFactory` que já
  existe.
- `text.rs:252`: o locale do `CreateTextFormat` vem da i18n.
- `lyrics/http_win.rs`: removido; tudo passa pelo `http::get`, com limite de
  tamanho.
- `mac/launchers.rs:250`: `NSWorkspace openApplicationAtURL:configuration:completionHandler:`.
- `mac/compat.rs:95`: conferir a checagem de versão do `activate()`.
- `lyrics/cache.rs`: `prune()` também ao gravar; `get` confere a linha `#key`.

### Rust idiomático e seguro

**PR-29 `refactor: soundness, lints e estratégia de panic`** — M · 1.5.2-1.5.6
- **Soundness:**
  - `tray.rs:26`: `static mut ICON_*` vira `AtomicPtr<c_void>`;
  - `linux/audio.rs:120`: ponteiro cru até travar o mainloop;
  - `tray.rs:90`: `ptr::dangling()`.
- **Panic (D-5):** remover o `catch_unwind` e o `MAX_PANICS` de
  `mac/notch.rs:155` e `mac/lyrics.rs:69`. Fica o gancho de panic que grava o
  `crash.log`.
- **`Cargo.toml`:** `rust-version = "1.88"` e a seção `[lints]` (como na 1.5.4
  do plano).
- **Código morto:** em `main.rs:15`, sai o `allow(dead_code)` do crate inteiro;
  os itens que são só do Windows ganham `#[cfg(windows)]`.
- **Erros:** `src/update/error.rs` com um `enum UpdateError { Network, Http(u16),
  Signature, Io(io::ErrorKind), Version }`, com `Display` em PT via i18n.
- **Lints pequenos:** `menu.rs:445`, `demo_gif.rs:661`, `iter().any()` →
  `contains()`, `Rc<Vec<u32>>` → `Rc<[u32]>`, `build.rs:15` (ordenar as versões
  como números).

**PR-30 a PR-33 `docs(unsafe): comentários SAFETY e escopo mínimo`** — M cada

| PR | Arquivos |
|---|---|
| PR-30 | `overlay.rs`, `lyrics_overlay.rs`, `taskbar.rs`, e o `main.rs` (o `unsafe {}` gigante quebrado por chamada) |
| PR-31 | `mixer.rs`, `audio.rs`, `volume.rs`, `vsync.rs`, `media.rs`, `webext.rs`, `webplayer.rs` |
| PR-32 | resto do Windows: `setup.rs`, `launchers.rs`, `settings.rs`, `text.rs`, `tray.rs`, `menu.rs`, `appicon.rs`, `update/` |
| PR-33 | Mac e Linux |

- Cada `// SAFETY:` diz **por que** a chamada é segura (dono do handle, tempo de
  vida do ponteiro, thread), não o que ela faz.
- Onde o mesmo padrão aparece 3 vezes ou mais, uma função segura no lugar.
- **Fim do PR-33:** o CI passa a usar `cargo clippy -- -D warnings`.

**PR-34 `style: rustfmt em todo o projeto`** — G mecânico · D-6
- Só depois que os PRs 20 a 33 entrarem: sem PRs abertos, para não gerar
  conflito.
- `rustfmt.toml`:
  ```toml
  max_width = 160
  use_small_heuristics = "Max"
  ```
  Dá para incluir também `fn_call_width` e `chain_width` ajustados.
- Um commit só, `style: cargo fmt`, com o hash em `.git-blame-ignore-revs`.
- CI: `cargo fmt --check`.

**PR-35 a PR-38 `refactor: dividir os arquivos gigantes`** — G mecânico cada
- Só movem código: nenhuma linha de lógica muda, e o diff é revisado com
  `git diff --color-moved`.

  | PR | De | Para |
  |---|---|---|
  | PR-35 | `settings.rs` | `settings/{mod, window, layout, pages, preview, input, search}.rs` |
  | PR-36 | `ui.rs` | `ui/{mod, layers, widget, card, mixer, hit}.rs` |
  | PR-37 | `overlay.rs` | `overlay/{mod, window, frame_loop, input, acrylic}.rs` |
  | PR-38 | `lyrics_overlay.rs` | `lyrics_overlay/{mod, window, draw}.rs` |

- Ordem: o 36 e o 37 antes do M3, que mexe em `ui` e `overlay`; o 35 antes do
  M7.

**PR-39 `refactor(render): DrawCtx no lugar de 8-11 argumentos`** — M
- `struct DrawCtx<'a> { cv: &'a mut Canvas, tr: &'a mut TextRenderer, s: f32,
  look: &'a Look, cfg: &'a Config }`.
- Remove os 23 `#[allow(clippy::too_many_arguments)]`.

**Fim do M2:**
- clippy limpo com `-D warnings` e fmt no CI;
- `docs/medicoes/2.0-m2.md` mostra CPU parado ≤ 1.4.2, zero processos criados
  por hora no Mac e menos despertares no Windows;
- soak de 5 h 30 no CI e de 8 h local verde nos três sistemas.

---

## M3 — Fundações

**PR-40 `feat(config): config v6`** — M · F1
- Os campos e enums da interface acima, `CONFIG_VERSION = 6`, e a migração:
  - `always_show_widget: true` vira `idle_widget = Full`;
  - `false` vira `Hidden`;
  - **quem atualiza mantém o comportamento de hoje**, e só a instalação nova ganha
    `Compact`;
  - `onboarding_done = true` para quem já tinha config.
- Manter `always_show_widget` lido (`#[serde(alias)]` ou um campo de leitura
  única), para não perder a escolha.
- **Testes:**
  - os snapshots de config v1, v2, v3, v4 e v5 (fixtures em `tests/config/`)
    carregam e migram;
  - uma v6 carregada pela lógica tolerante do PR-12 não perde chaves conhecidas.

**PR-41 `feat(anim): mola criticamente amortecida`** — M · F5
- O `Spring` da interface, com integração exata (forma fechada, que não explode
  com `dt` grande depois de o PC dormir).
- `anim::Toggle` passa a usar o `Spring` por dentro, mantendo a API pública, para
  migrar sem tocar em quem chama.
- **Testes:**
  - converge;
  - sem overshoot;
  - retarget no meio mantém a velocidade contínua;
  - `dt = 5 s` não gera NaN.

**PR-42 `feat(render): imagem girada, mipmap e fundo animado recortado`** — M · F2
- `draw_image_rotated`, extraído de `lyrics_bg::layer()`.
- `Mip`, gerado sob demanda e em cache junto da imagem.
- `lyrics_bg::fill_rect` com `BgCache` por superfície. O `fill` atual vira
  `fill_rect` com a tela inteira, e o `thread_local` sai.
- **Testes:** capturas em ângulos fixos (0°, 45° e 90°), mip comparado à
  referência, e o fundo da letra em tela cheia igual pixel a pixel ao de antes.

**PR-43 `feat(card): páginas no card`** — G · F3 (Windows)
- **Estrutura:** `enum CardPage` em `ui/card.rs` e o estado em `overlay`.
- **Transição:** deslizando com o `Spring`; a página que sai e a que entra são
  desenhadas em camadas próprias (`LayerTag::CardPage(p)`).
- **Interação:**
  - hit-test por página;
  - `Esc` volta a `Player`;
  - fechar o card reseta a página;
  - o mixer continua sendo só da página `Player`.
- **Nesta etapa:** as páginas `Library` e `Search` ficam vazias, atrás de uma
  flag de debug, até o M6.

**PR-44 `feat(ilha): páginas na ilha aberta`** — M · F3 (Mac/Linux)
- O mesmo em `notch.rs`, com as alturas por página: a ilha cresce para baixo na
  biblioteca.

**PR-45 `feat: campo de texto reutilizável com foco de teclado`** — M · F4
- `settings_controls::TextField`, extraído de `settings.rs:4566`, com cursor,
  seleção, Ctrl+A/C/V/Backspace, IME e composição (`WM_IME_*`).
- **Foco:**
  - Windows: ao focar, `SetWindowLongPtr(GWL_EXSTYLE, ex & !WS_EX_NOACTIVATE)` e
    `SetForegroundWindow`; ao sair, restaura;
  - Mac: uma subclasse da janela com `canBecomeKeyWindow` controlado por flag;
  - Wayland: `set_keyboard_interactivity(OnDemand)`;
  - X11: `SetInputFocus`.
- **Teste:** à mão, digitar com IME (japonês) nos três sistemas, porque é a
  parte que costuma quebrar.

**Fim do M3:** sai a **`2.0.0-beta.1`**: a 2.0 mais estável e econômica, ainda
sem recurso novo visível.

---

## M4 — Comportamento (itens 3, 9 e 10)

**PR-50 `feat(windows): clicar na capa traz o player para a frente`** — P · item 3
- `Hit::Art`, com a ação conforme `cfg.art_click`, chamando o `focus_player` que
  já existe.
- **Extensão:** `{t:"cmd", cmd:"focus"}`; no `sw.js`, `chrome.tabs.update(tabId,
  {active:true})` mais `chrome.windows.update(winId, {focused:true})`. Usado
  quando a fonte é o YT Music do navegador.
- A extensão sobe para 1.2.0.
- **Configuração:** a opção em Card › Interação.

**PR-51 `feat(mac,linux): clicar na capa abre o player`** — P · item 3
- Mac: no clique da ilha (`mac/notch.rs:927`), o `open` do NSWorkspace com o
  bundle id.
- Linux: `MediaCmd::Raise` e ler `CanRaise` em `apply` (`linux/media.rs:288`).
  Sem Raise, cair no `gtk-launch <desktop entry>`.

**PR-52 `feat(widget): widget compacto quando nada toca`** — M · item 9, D-1
- Um `WLayout::compact(s)` em `ui/widget.rs`: uma pílula de 36·s com o ícone ou
  a última capa a 40%.
- Em `overlay`, um estado `IdleState { Active, Grace(Instant), Idle }`:
  - `Grace` dura 10 s, para não piscar entre faixas;
  - pausado conta como ocioso depois de `idle_after_min`.
- A largura anima com o `Spring`, e o `taskbar::compute` recebe o comprimento
  interpolado.
- **Interação:** o hover mostra "Nada tocando"; o clique abre o menu de atalhos
  (`launchers`).
- Mac e Linux respeitam `Hidden` e `Compact` com a mesma máquina de estados (no
  módulo comum).
- **Testes:**
  - unitários da máquina de estados (tempos simulados);
  - captura do compacto em 3 escalas.

**PR-53 `feat: filtro dos apps ouvidos (motor)`** — M · item 10
- `src/media_filter.rs::allowed`, que é puro, com os testes de cada modo e de
  sub-fonte de site.
- **Ganchos:**
  - Windows: o `State` recebe um `Arc<RwLock<SourceFilter>>` atualizado no
    `update_config`, e `pick_session` filtra antes dos candidatos;
  - Mac: `adapter_publish` mais `publish_empty()`;
  - Linux: em `pick()`.
- **`seen_sources`:** atualizado no escritor de cada sistema, com no máximo 50
  entradas, descartando as mais antigas por `last_seen`.
- **Navegador:** o site vem do `WebState` da extensão
  (`chrome:music.youtube.com`).

**PR-54 `feat(configuracoes): escolher os apps ouvidos`** — M · item 10
- **Windows:** uma seção nova em "Apps de música":
  - o modo (segmentado: Todos, Só estes, Todos menos estes);
  - a lista (`seen_sources` mais a tabela `KNOWN`), com ícone (`appicon`),
    nome e interruptor.
- **Mac:** o mesmo com AppKit (`NSTableView` com checkbox).
- **Linux:** uma seção comentada no `config.toml` de exemplo.

---

## M5 — Visual e movimento (itens 4, 5, 7 e 12)

**PR-60 `perf(animacao): transições na taxa do monitor e mola em tudo`** — G · item 4
- `vsync.rs`: um modo `burst(dur)` que libera todo vblank (sem o teto de 75 fps)
  durante a transição, chamado ao abrir e fechar o card, trocar de página e
  expandir a ilha.
- **Mola:**
  - trocar `mixer_anim` (`overlay.rs:2057`) e as aberturas por `Spring`;
  - abrir o card escala pelo `Mip` (PR-42), com aritmética inteira;
  - no Mac, a escala da ilha vai para o `CALayer.transform` (GPU), e só o
    conteúdo final é rasterizado.
- **X11:** intervalo pela taxa do RandR.
- **Reduzir movimento:** ler `SPI_GETCLIENTAREAANIMATION` e
  `accessibilityDisplayShouldReduceMotion`, e o GNOME `enable-animations` pelo
  portal. Com ele ligado, `Motion::Reduced`.
- **Medição:** p95 do quadro durante abrir e fechar a 60, 120 e 144 Hz. O alvo é
  zero quadros perdidos a 120 Hz numa máquina de referência.

**PR-61 `feat(windows): acrílico pelo DWM`** — G · A1
- Windows 11 22H2 ou mais novo:
  - o card e o menu viram janelas com `DWMWA_SYSTEMBACKDROP_TYPE =
    DWMSBT_TRANSIENTWINDOW` e cantos com `DWMWA_WINDOW_CORNER_PREFERENCE`;
  - o conteúdo vai por cima, com DirectComposition, ou por uma janela filha
    layered.
- Windows 10: continua o `BitBlt` de hoje.
- **Risco:** é a maior mudança de janela da 2.0. Começar por um protótipo isolado
  (`examples/backdrop.rs`) antes de mexer no `overlay`.

**PR-62 `feat(widget): fundo "Capa em movimento" e efeitos`** — M · item 5, D-3
- `WidgetTint::Animated`, desenhado com `fill_rect` num cache próprio do widget.
  A camada do widget passa a guardar só o primeiro plano, composto sobre o fundo
  do quadro.
- Os efeitos (pulso nos graves, *shimmer* na troca e gradiente fluindo) viram
  `bg_fx: Vec<BgFx>` na config.
- **Ritmo:** ~12 fps (`set_rate` "decorativo"), pausado sem música, com app em
  tela cheia, com `Motion::Reduced` e na bateria (`GetSystemPowerStatus`,
  `IOPSCopyPowerSourcesInfo`, UPower). A bateria tem uma opção própria.
- **Medição:** o custo de CPU do widget animado, anotado no README.

**PR-63 `feat(card,ilha): fundo animado no card e na ilha`** — M · item 5
- `CardStyle::Animated` e `NotchBg::Animated`.
- **Card:** medir. Se passar de 2 ms por quadro a 2×, usar a grade a 1/16.

**PR-64 `feat: disco de vinil girando`** — M · item 7
- `src/vinyl.rs`:
  - sulcos com anéis SDF, numa textura gerada uma vez por tamanho;
  - o selo com `draw_image_rotated`;
  - o furo;
  - o reflexo fixo, um gradiente cônico estático por cima.
- **Física:** `Spring` aplicado à velocidade angular: 0,555 volta/s tocando e 0
  pausado.
- **Onde:** no widget e na ilha fechada, com `CoverViz::Vinyl`, que esconde as
  barras. No card fica para uma etapa futura.
- **Testes:** capturas em ângulo fixo e o custo por quadro.

**PR-65 `feat: capa em mais resolução`** — M · item 12
- **Atenção à história:** o commit `ae79a7e` mostrou que guardar a capa a 768 px
  saiu **pior** que 384 com a redução do CoreGraphics no card de 144 px, com mais
  moiré e 4× a memória. Portanto:
  - o `ART_SIZE` passa a ser calculado (tamanho físico da maior capa visível,
    arredondado à potência de 2, com teto de 1024);
  - a redução é sempre de alta qualidade (CoreGraphics no Mac, o `Mip` nos
    outros);
  - **aceitar só se o `cover_shots` medir contraste ≥ e moiré ≤** que hoje, no
    card a 1×, 2× e 3×.
- **Miniatura pequena:** quando o SMTC dá menos que o necessário (o YT Music dá
  120 px), usar o `cover::fetch` em alta resolução também no card, com o cache em
  disco que já existe.
- **Troca de DPI:**
  - Windows: o card trata `WM_DPICHANGED` ao abrir noutro monitor;
  - Mac: `NSWindowDidChangeBackingPropertiesNotification`;
  - X11: `RRScreenChangeNotify`.
- **Texto:** *snapping* da linha de base nas escalas fracionárias nos três
  sistemas.

**PR-66 `ci: matriz de capturas em 6 escalas`** — M · item 12
- O `shots.rs` e os `ci-shots.sh` passam a rodar em 100, 125, 150, 175, 200 e
  250%.
- Comparar com as referências guardadas (`docs/screenshots/ref/`), com tolerância.
  Falha mostra o diff como artefato do CI.

**Fim do M5:** sai a **`2.0.0-beta.2`**.

---

## M6 — Biblioteca no pop-up (itens 8, 2 e 1)

**PR-70 `feat(card,ilha): páginas de biblioteca e busca`** — M · item 8
- Ícones de biblioteca e lupa ao lado do botão do mixer. Ao clicar, mudam a
  página (PR-43/44).
- Um mini player no topo das páginas: capa de 32·s, título e play/pausa.
- **Atalho global:** `Win+Alt+F` (`RegisterHotKey`, como o `Win+Alt+L`); no Mac,
  `⌥⌘F` via o monitor global que já existe.
- Estados vazios com orientação ("Cole o link de uma playlist para fixar aqui").

**PR-71 `feat: busca rápida`** — M · item 2
- `src/library/`:
  - `search_target` com escape correto (`percent-encode` próprio, ~20 linhas,
    sem dependência);
  - os provedores do Spotify, do YT Music, do Apple Music no Mac (AppleScript,
    com o timeout do PR-24) e o personalizado.
- **Extensão:** comando `{cmd:"navigate", url}`. O `content.js` valida que a URL
  começa com `https://music.youtube.com/`; a extensão nunca navega para fora.
- Página `Search`: o campo (PR-45), Enter abre no player, e um histórico de 5
  itens com "Limpar".
- **Mac:** os resultados do Apple Music em lista, e clicar toca
  (`play track id`).
- **Testes:** `search_target` com acentos, espaços, `&`, `#` e emoji.

**PR-72 `feat: playlists fixadas`** — M · item 1
- `library::parse_link` para `open.spotify.com/playlist|album/…`,
  `spotify:playlist:…`, `music.youtube.com/playlist?list=…`,
  `music.apple.com/…/playlist/…`.
- **Página `Library`:**
  - uma grade 2×N de cartões com nome e ícone do player;
  - "+" cola da área de transferência;
  - menu de contexto para renomear e remover;
  - arrastar para reordenar.
- **"Fixar a que está tocando":** o YT Music lê o `list=` da aba pela extensão;
  o Spotify, pelo contexto, quando disponível.
- **Testes:** `parse_link` com URLs reais, URLs com rastreio (`?si=…`) e
  inválidas.

**PR-73 `feat(mac,linux): listas automáticas de playlists`** — M · item 1
- **Mac:**
  - `get name of every user playlist` com cache de 5 min;
  - `play playlist "X"`, com o nome escapado no AppleScript.
- **Linux:** `org.mpris.MediaPlayer2.Playlists` (`GetPlaylists`,
  `ActivatePlaylist`) quando o player tiver.
- **YT Music pelo DOM:** atrás da opção "Ler as playlists da página (experimental)",
  com um teste no CI que carrega um HTML salvo e falha se os seletores pararem de
  achar.

**Fim do M6:** sai a **`2.0.0-beta.3`**.

---

## M7 — Configurações e primeira execução (itens 6 e 11)

**PR-80 `test(configuracoes): protótipo do layout novo no settings_mock`** — M · item 6
- Gerar no `settings_mock.rs` os PNGs do layout novo em 900, 1180, 1440 e 1920
  px de largura, a 1× e a 2×.
- Revisar os PNGs antes do PR-81. Este é o momento barato de mudar de ideia.

**PR-81 `feat(configuracoes): grade alinhada e layout responsivo`** — G · item 6
- **`settings/layout.rs`:**
  - a coluna de controle por seção é a maior largura de controle da seção, com
    os controles alinhados à esquerda nela;
  - os pontos de quebra: abaixo de 1000 (palco em cima), 1000 a 1400 (55/45) e
    acima de 1400 (duas colunas de cartões, com o palco *sticky*).
- Tirar as larguras fixas de escolha e slider (`settings.rs:2068,2071`), que
  passam a vir da grade.
- **Teste:** captura das 8 páginas em 3 larguras.

**PR-82 `feat(configuracoes): prévia em contexto e mostrar a mudança`** — G · item 6
- A prévia mostra uma barra de tarefas falsa com papel de parede (ou o notch),
  com o widget e o card juntos na escala real.
- **Mostrar a mudança:**
  - uma tabela `fn region(key) -> Option<PreviewRegion>` ao lado de
    `requirement`;
  - hover na linha desenha o contorno animado da região;
  - mudar um valor faz um crossfade de 250 ms do antes para o depois;
  - um botão "Comparar", que se segura para ver a versão salva.

**PR-83 `feat(mac,configuracoes): janela redimensionável e prévia ao lado`** — M · item 6
- `mac/settings.rs`:
  - sai a largura fixa de 620 px (`mac/settings.rs:507`);
  - um `NSSplitView` com a prévia ao lado acima de 900 px;
  - o contorno da região com uma `CAShapeLayer` sobre o `NSImageView`.

**PR-84 `feat(windows): tutorial da primeira execução`** — G · item 11
- **`src/onboarding.rs`:**
  - uma janela própria (a família do `setup.rs`, com o Mica);
  - 7 passos;
  - Pular, Voltar e Próximo, mais a barra de progresso;
  - cada passo grava na config na hora, então pular no meio mantém o que já foi
    escolhido.
- Reusa o `draw_preview` (PR-35 dividiu), o `settings_controls` e o `TextField`.
- **Gatilho:** `first_run && !onboarding_done`. O `setup.rs:932` passa
  `--welcome` no lugar de `--settings` na instalação nova.
- **"Rever o tutorial"** em Geral.
- **Testes:**
  - captura de cada passo nas 4 línguas;
  - teste de que "Pular" no passo 1 marca `onboarding_done` e não altera a
    config.

**PR-85 `feat(mac,linux): tutorial`** — M · item 11, D-4
- **Mac:** uma janela AppKit com as mesmas etapas e as prévias da `mac/settings`.
- **Linux:** 4 dicas sequenciais na ilha, no formato "A seguir": o que é,
  clique, atalhos e o `config.toml`. Avança com clique e tem um "x" para pular.

**PR-86 `feat: novidades da 2.0`** — P · item 11
- Para quem atualiza: um aviso no `update/popup.rs` (`View::WhatsNew`) com 4
  destaques e o botão "Ver o que mudou", que abre o tutorial a partir do passo 3.

**Fim do M7:** sai a **`2.0.0-rc.1`**.

---

## M8 — Lançamento

**PR-90 `docs: README, capturas e avisos da 2.0`** — M
- README com recursos novos, tabela de consumo refeita (das medições), capturas
  e GIFs novos pelo CI.
- `THIRD-PARTY-NOTICES.md`, regenerado por `tools/third_party_notices.py`, para
  o caso de uma dependência ter mudado (por exemplo, a feature `shm` do x11rb).
- `extension/store/listing.md`: a versão 1.2.0, para publicar na Chrome Web
  Store.

**Checklist de lançamento** (de "Critérios de pronto" no plano):
- [ ] Soak de 5 h 30 no CI e de 8 h local verde nos três sistemas, com a `2.0.0-rc.N`
- [ ] Medições da rc ≤ 1.4.2 parado, com o custo de cada efeito documentado
- [ ] Migração testada de v1 a v5; downgrade para a 1.4.3 sem perder a config
- [ ] Capturas nas 6 escalas e as 4 línguas completas
- [ ] Uma semana de rc sem bug novo de severidade alta
- [ ] `chore(release): 2.0.0`, tag `v2.0.0`, merge na `main` e notas nas 4
      línguas

---

## Visão do caminho crítico

```
M0 ─> M1 (1.4.3) ─> M2 ─────────────> M3 ─┬─> M4 ───────────┐
       PR-20..28 em paralelo ─┘     PR-40..45 ├─> M5 (60..66) ┼─> M7 ─> M8
       PR-29..34 em série               └─> M6 (70..73) ┘
       PR-35..39 (35 pode esperar o M7)
```

- **Caminho crítico:** o M2, com a série 29 → 33 → 34 e a divisão dos arquivos,
  depois o M3, depois o M7.
- **O que corre em paralelo:**
  - no M2, os PRs por sistema (20-22, 24-25, 26) e o 27-28;
  - depois do M3, o M4, o M5 e o M6, desde que o M5 e o M6 não mexam no mesmo
    arquivo ao mesmo tempo. O PR-60 (`overlay`) e o PR-70 (`ui/card`) tocam
    vizinhos, então coordenar.

## Riscos

| Risco | Onde | Mitigação |
|---|---|---|
| Foco de teclado em janela `NOACTIVATE` não funciona igual em todo Windows | PR-45 | Protótipo antes; reserva: uma caixa de busca numa janela pequena normal |
| Acrílico pelo DWM exige reestruturar a janela do card | PR-61 | Protótipo isolado; manter o `BitBlt` como reserva |
| Mais resolução de capa pode piorar a nitidez (já aconteceu) | PR-65 | Só aceitar com o `cover_shots` medindo melhor |
| Leitura do DOM do YT Music quebra quando o site muda | PR-73 | Opção experimental, teste com HTML salvo, falha silenciosa |
| Fundo animado e vinil aumentam o CPU | PR-62..64 | Desligados por padrão, ritmo decorativo, pausa na bateria, custo no README |
| rustfmt e divisão de arquivos geram conflitos | PR-34..38 | Fazer sem PRs abertos; `.git-blame-ignore-revs`; diff com `--color-moved` |
| Downgrade da 2.0 para a 1.4.x perde a config | PR-12, PR-40 | Carga tolerante já na 1.4.3, antes da 2.0 existir |
