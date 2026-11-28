# Plano de implementação — Taskbeat 2.0 (big-update)

Base: v1.4.2. As referências `arquivo:linha` são do código atual e servem de
ponto de partida; confira antes de editar, porque algumas vão mudar.

A quebra em PRs, com os passos, os testes e a ordem, está em
[plano-implementacao-2.0.md](plano-implementacao-2.0.md).

Tudo sai numa única versão grande, a **2.0**, com três frentes:
1. qualidade, bugs e otimização;
2. fundações técnicas;
3. os 12 recursos novos.

## Estratégia de entrega

- **Branch de integração `release/2.0`.** Cada etapa entra nela por PRs pequenos
  (um tema por PR), e a `main` só recebe o merge final. Correções de bug graves da
  Etapa 1 também vão para a `main` como **1.4.3**, para não deixar quem está na
  1.4.x esperando pela 2.0.
- **Betas pelo canal de teste que já existe.** O atualizador já tem a opção
  "receber as versões de teste" (Configurações › Geral › Atualizações). As betas
  saem como `2.0.0-beta.N` a cada etapa fechada, e quem está na versão estável não
  recebe.
- **Ordem interna:** qualidade primeiro, depois fundações, depois recursos. A
  Etapa 1 deixa a base estável e medida; sem isso, os recursos visuais (que
  gastam CPU) seriam medidos sobre uma base que já desperdiça.
- **Config v6 numa migração só,** cobrindo todos os campos novos da 2.0.

## O que o código já impõe (vale para tudo)

- **Um rasterizador só, na CPU.** Tudo é desenhado pelo `render::Canvas` (SDF num
  buffer BGRA, `render.rs`). Cada sistema só apresenta o buffer:
  - Windows: `UpdateLayeredWindow` (`overlay.rs:358`).
  - Mac: `CGImage` num `CALayer` (`mac/notch.rs:787`).
  - Linux: `wl_shm` ou X11.

  A vantagem é que efeito novo se escreve **uma vez**. O custo é que todo efeito
  animado gasta CPU por quadro. A meta do README ("parado ≈ 0% de CPU") vira
  critério de aceite: toda animação contínua pausa quando a música para, quando há
  app em tela cheia e, no notebook, quando está na bateria.
- **Modelo de camadas em cache.** O fundo do widget e do card fica pronto em cache
  (`LayerTag::Widget`/`Card`/`CardFinal`, `ui.rs:890-968`). O que se mexe sempre
  sai desse cache e vira trabalho por quadro, só na área que muda.
- **`panic = "abort"`** (`Cargo.toml:81`). Qualquer panic fecha o app, e o
  `catch_unwind` não pega nada. Por isso panics vindos de dados externos são
  bugs de severidade alta.
- **i18n.** O texto em PT é a chave da tabela `T` (`i18n.rs:127`). Cada texto
  novo precisa ter PT, EN, ES e DE, e os testes de `i18n::has` cobram isso.

---

## Etapa 1 — Qualidade, bugs e otimização

**Levantamento feito em 27/11/2026:**
- `cargo clippy` para Windows e macOS, com os lints `undocumented_unsafe_blocks`,
  `mem_forget` e `rust_2024_compatibility`;
- `cargo fmt --check`;
- `cargo test --release`: 107 passaram, 0 falharam, 47 ignorados;
- quatro auditorias por leitura de código (Windows, Mac, Linux e o código comum).

Os itens marcados ✔ foram reconferidos diretamente no código. O resto vem da
auditoria e deve ser confirmado ao abrir o PR.

### 1.1 Bugs que fecham o app ou perdem dados (entram também na 1.4.3)

| # | Onde | Problema | Correção |
|---|---|---|---|
| B1 ✔ | `lyrics/lrclib.rs:135-142` (`first_artist`) | O índice achado em `a.to_lowercase()` é usado para fatiar `a`. O `to_lowercase` muda o tamanho em bytes ('İ' 2→3, 'K' Kelvin 3→1), então um artista como "BİRİCİK, ŞEBNEM" corta no meio de um caractere e dá **panic** na thread de letras, o que fecha o app. | `to_ascii_lowercase()`, que preserva os offsets (os separadores são ASCII). Teste com 'İ', 'ẞ' e 'K'. |
| B2 ✔ | `lyrics/lrclib.rs:112-115` (`clean_title`) | O mesmo defeito: `low.find(w)` seguido de `s.truncate(i)`. | Idem. |
| B3 ✔ (baixa) | `volume.rs:30`; `overlay.rs:849,856,857,1234,1322,1986,1996`; `settings.rs:2366` | `Instant::now() - Duration` dá **panic** se o resultado ficar antes do zero do relógio. **Severidade baixa:** desde o rustc 1.97 o `Instant` do Windows começa em `u64::MAX/4` ns (rust-lang/rust#156177), então com o 1.100 não há panic logo depois do boot; continua valendo porque isso não é garantia da API, o `rust-version = 1.88` do PR-29 aceita compiladores em que o panic é real, e `Duration::from_secs_f32(inf)` também dá panic. | `Option<Instant>` e um helper com `checked_duration_since`/`checked_sub` (`src/clock.rs`), mais o lint `clippy::unchecked_time_subtraction`. |
| B4 ✔ | `config.rs:866-871, 901-909` | Se o TOML não abre (erro de digitação, ou um enum de uma versão mais nova, que é o caso de quem volta da 2.0 para a 1.4), o app carrega os padrões calado, e a próxima gravação **apaga as preferências**. Além disso, o `save()` usa `fs::write`, que trunca o arquivo antes de gravar. | Renomear o arquivo ruim para `config.toml.bad` e registrar no log; gravar num `.tmp` e trocar com `rename`, que é atômico. O mesmo vale para `update.toml` e o cache de letras. |
| B5 ✔ | `config.rs:894-897` | O `save()` roda antes do `sanitize()`, então a migração grava valores fora da faixa. | Inverter a ordem. |
| B6 | `config.rs:919`, `dsp.rs:141` | `sensitivity_db = nan` é TOML válido e passa pelo `clamp` (`NaN.clamp` devolve NaN), e as barras morrem para sempre. | Exigir `is_finite()` em todos os `f32`/`f64` da config, como `lyrics/config.rs` já faz. |
| B7 | `lyrics/mod.rs:133` | `[offset:NaN]` ou `[offset:inf]` num LRC joga todas as linhas para 0 ou para +inf. | Exigir `is_finite` e limitar o offset (±10 min). |
| B8 | `linux/gfx.rs:146-147` ✔ | Uma capa com caminho puro (`/…`) é lida sem limite: um `artUrl=/dev/zero` leva a falta de memória, e um FIFO prende a thread. | O mesmo `take(MAX_BYTES)` do `file://`, mais `metadata().is_file()`. |
| B9 | `linux/gfx.rs:12,39-65` | Uma capa de 8192² passa por cerca de 470 MB de pico, com uma thread de decodificação por URL e sem limite de quantas. | Limitar w·h ≤ 4096², ou reduzir já na decodificação; uma thread só de capas, com fila que descarta URLs velhas. |
| B10 | `linux/island.rs:828`, `wayland.rs:427` | O evento `closed` da layer-surface (monitor removido, laptop desencaixado) **fecha o app inteiro**. | Recriar a superfície, e tratar `global`/`global_remove` do `wl_output`. |
| B11 | `linux/wayland.rs:289,295` | `set_size` sempre zera `configured`. Um compositor que só manda `configure` quando o tamanho muda nunca responde, e a ilha **congela**. | Só zerar quando o tamanho lógico mudar, ou dar um timeout. |
| B12 | `mac/update.rs:15,38` | `runModal` dentro de um `exec_async` na fila principal: enquanto o aviso de atualização está aberto, a ilha congela, porque a fila fica presa e os timers do modo padrão param. | Mostrar o aviso sem modal (um `NSPanel` próprio) e pôr os timers em `NSRunLoopCommonModes`. |
| B13 | `main.rs:163` | Depois de atualizar, o processo novo espera só 15 s pelo antigo. Se o antigo demorar mais, o novo sai e o app fica fechado. | Esperar mais (60 s), ou o antigo avisa ao fechar. |
| B14 | `update/mod.rs:566`, `lyrics/cache.rs:114` | Se `last_check` ficou no futuro (relógio adiantado uma vez), o app não procura atualização até chegar aquela data. | Tratar `last_check > now` como vencido. |
| B15 | `lyrics_overlay.rs:1933` | `bg_clock` é `f32` e só soma: depois de cerca de 73 h em tela cheia, a animação do fundo engasga. | `f64`, ou dar a volta num período. |
| B16 | `update/sig.rs:16` | `assert!` sobre o `BCryptHash` aborta o app. | Devolver `Result`. |

### 1.2 Vazamentos e recursos

| # | Onde | Problema | Correção |
|---|---|---|---|
| V1 | `media.rs:506` (Windows) | Se só uma das três assinaturas WinRT falhar, os tokens que deram certo nunca são removidos, e o app reassina a cada poll. Os handlers se acumulam. | Remover os tokens `Ok` quando alguma falhar. |
| V2 | `media.rs:481` | A assinatura é indexada pelo AUMID. Um player que reinicia ganha uma `Session` nova, que não é assinada, e só resta o poll de 2 s. | Comparar também a identidade do objeto. |
| V3 | `webplayer.rs:530` | `CreateDIBSection(...).ok()?` sai sem `DeleteDC` nem `ReleaseDC`. | **Guardas RAII** para HDC, HBITMAP e HICON, num módulo `win::gdi` usado em todo o projeto. |
| V4 | `webext.rs:506` | O `PSECURITY_DESCRIPTOR` nunca recebe `LocalFree`, e cada read/write do pipe cria e fecha um evento. | `LocalFree`; um evento reaproveitado por direção. |
| V5 | `lyrics_overlay.rs:550,904` e outras threads | `CoInitializeEx` sem `CoUninitialize` em threads que são recriadas. | Guarda RAII `ComInit`. |
| V6 | `mixer.rs:332-361,554` | `Names.done` e `asked` (chaves `pid:{pid}`) crescem o dia todo. | Podar contra as sessões vivas. |
| V7 | `vsync.rs:159` | O `IDXGIOutput` fica em cache para sempre. Depois de trocar de GPU ou de dock, cai em `DwmFlush` permanentemente. | Recriar o output quando `WaitForVBlank` falhar. |
| V8 | `mac/launchers.rs:87`, `mac/media.rs:618`, `mac/gfx.rs:92` | Chamadas do AppKit e do Foundation em `std::thread` sem `autoreleasepool`. As threads não terminam, então o pool implícito nunca esvazia. | `objc2::rc::autoreleasepool(|_| …)` em cada iteração. |
| V9 | `mac/text.rs:167,210` | O cache de texto é limitado por número de entradas, não por bytes, e a letra em tela cheia chega a dezenas de MB. | Orçamento em bytes. |
| V10 | `linux/audio.rs:209-215` | Quando o contexto do Pulse falha, `p.stream` é zerado sem `pa_stream_unref`: perde um stream a cada reinício do PipeWire. | Limpar os callbacks e dar `unref` antes de zerar. |
| V11 | `linux/gfx.rs:176`, `linux/system.rs:146` | O `curl` é morto com `kill()` sem `wait()`, e o `xdg-open` nunca é esperado: deixam processos zumbis. | `wait()` depois do `kill`; para o `xdg-open`, esperar numa thread. |
| V12 | `linux/audio.rs:305` | `monitors` nunca remove saídas que sumiram (cada reconexão Bluetooth cria um índice). | Podar no `remove`. |
| V13 | `lyrics/fetch.rs:44`, `lyrics/cache.rs:132` | O `prune()` só roda na partida, então o cache passa de `MAX_FILES` numa sessão longa. O `get` também ignora a linha `#key`, e uma colisão do FNV devolveria a letra de outra música. | Podar ao gravar e conferir a chave. |

### 1.3 Desempenho e consumo (o app fica aberto o dia todo)

**Cada correção precisa de medição antes e depois** (ver 1.6).

| # | Onde | Problema | Correção |
|---|---|---|---|
| D1 ✔ | `mac/media.rs:228-235` | Com nada tocando, o AppleScript reserva roda o dia inteiro: 1-2 `osascript` a cada 3 s, uns **57 mil processos por dia** parado. | Só usar a reserva se o adaptador nunca mandou nada, ou se o Spotify ou o Music estão abertos (`NSRunningApplication`, que não cria processo); com backoff de 30 s ou mais. |
| D2 | `mac/lyrics.rs:341,781,808` | Letra em tela cheia com fundo animado: 60 fps em software da tela inteira, mais cerca de 24 MB copiados por quadro para o `CFData`. | Fundo a 12-15 fps (como no Windows, `BG_FRAME`), ou em resolução menor escalada pelo CA. |
| D3 | `mac/notch.rs:787`, `mac/gfx.rs:19` | Cada quadro faz um `Vec` novo, uma cópia para o `CFData` e cria `CGImage` e `CGColorSpace`. | `CGDataProviderCreateWithData` com callback de liberação, e o sRGB em cache; o ideal é um IOSurface reaproveitado. |
| D4 | `mac/notch.rs:267`, `osd.rs:113`, `lyrics.rs:234` | Timers rodam o dia todo: a ilha a 2 Hz sem tolerância, o aviso de teclas a 8 Hz mesmo desligado, a letra a 10 Hz escondida. | `setTolerance`; invalidar o timer quando a opção ou a janela está desligada. |
| D5 | `audio.rs:300` (Windows) | O loopback do sistema faz polling com `sleep(10ms)`: 100 despertares por segundo, mesmo em silêncio. | `AUDCLNT_STREAMFLAGS_EVENTCALLBACK`, como `process_capture` já faz. |
| D6 | `media.rs:129` | `media_version` sobe a cada poll de 2 s mesmo sem mudança, então o widget e o card são **redesenhados inteiros a cada 2 s**. | Subir só quando mudou; ter uma versão separada para a posição. |
| D7 | `overlay.rs:1535`, `taskbar.rs:74,374` | `keep_on_top` faz `FindWindowW` e percorre até 256 janelas a cada tick. | Guardar o hwnd e checar só em `WM_WINDOWPOSCHANGED` ou na ativação. |
| D8 | `text.rs:367,391`, `mac/text.rs:205`, `linux/text.rs:277` | Toda consulta ao cache de texto aloca uma `String`, mesmo quando acerta. | Chave por hash `u64`, ou `Rc<str>` com lookup via `Borrow`. |
| D9 | `notch.rs:731`, `linux/island.rs:532` | `format!("{:?}", self.it)` a cada quadro, só para fazer um hash. | `#[derive(Hash)]`. |
| D10 | `notch.rs:451` | `vec!` a cada quadro para as barras. | Buffer de rascunho guardado na struct. |
| D11 | `linux/x11.rs:213-225` | Um `Vec` novo por quadro e `put_image` pelo socket, sem MIT-SHM. Esse também é o caminho do GNOME. | Buffer reaproveitado; feature `shm` do x11rb. |
| D12 | `linux/wayland.rs:323`, `island.rs:634` | `set_input` faz um commit próprio: 2 commits por quadro durante as animações. | Definir a região dentro de `present`. |
| D13 | `linux/media.rs:379-404` | Ignora `POLLHUP`: se o D-Bus cair, a thread gira a 100%. | Checar `revents` e reconectar. |
| D14 | `linux/audio.rs:309-345` | Timer de 400 ms a vida toda, lendo até 8 arquivos de `/proc` por fluxo. | Cache pid→ancestral; pausar quando as barras não estão visíveis. |
| D15 | `lyrics_bg.rs:132-201` | 3 buffers alocados por quadro e `sqrt` por pixel. | Reaproveitar os buffers no `Cache`; comparar a distância ao quadrado. |
| D16 | `render.rs:208-249` (`blurred`) | Box blur O(r) por pixel em `[f32;4]`, usado no acrílico. | Soma corrente deslizante, O(1) por pixel. |
| D17 | `render.rs:126-172` (`resampled`) | O cache tem só 4 entradas por bits exatos de f32, e cada erro aloca um `Vec` por linha. | Pesos num array plano e buffers reaproveitados. Confirmar se anima. |
| D18 | `dsp.rs:105` | `norm()` faz `sqrt` em 2049 bins por quadro. | `norm_sqr` com `10·log10`. |
| D19 | `mac/media.rs:436` | A string base64 da capa (100-300 KB) é clonada a cada evento. | Comparar por hash. |
| D20 | `mac/audio.rs:313`, `mac/volume.rs:321` | Lê o UID do dispositivo a cada 0,5 s, e grava o volume a cada evento de arrasto. | `AudioObjectAddPropertyListenerBlock`; juntar os eventos de arrasto. |
| D21 | `mac/settings.rs:645`, `statusbar.rs:143` | As configurações continuam renderizando prévias depois de fechadas. | Pular quando `!isVisible`. |
| D22 | `mac/media.rs:659` | `osascript` sem timeout na thread de mídia: um player travado ou um pedido de permissão pendente bloqueia os comandos. | Timeout de 2-3 s com `kill`. |

### 1.4 APIs depreciadas ou desaconselhadas

| # | Onde | Hoje | Recomendado |
|---|---|---|---|
| A1 | `overlay.rs:446`, `menu.rs:554` | O acrílico é feito com `GetDC(NULL)` + `BitBlt` da tela: força o DWM a ler a GPU, sai preto em conteúdo com DRM e não acompanha o que está atrás. | Janela não-layered com `DWMWA_SYSTEMBACKDROP_TYPE` (acrílico do sistema, Win11 22H2+), mantendo o `BitBlt` como reserva no Win10. É a maior mudança desta etapa: fazer junto com o item 4. |
| A2 | `appicon.rs:302` | `PrivateExtractIconsW`, que a documentação diz não ser para uso geral. | `IShellItemImageFactory` (já usado em `shell_item_icon`) ou `SHDefExtractIconW`. |
| A3 | `launchers.rs:216`, `mixer.rs:342` | Threads STA que não bombeiam mensagens. | MTA, ou `CoWaitForMultipleHandles`. |
| A4 | `overlay.rs:1029` | `GWLP_HWNDPARENT` apontando para a barra, que é de outro processo (o Explorer). | Documentar o porquê e conferir o retorno, ou só `HWND_TOPMOST`. |
| A5 | `text.rs:252` | `CreateTextFormat(..., "pt-br")` fixo. | O locale da i18n, que afeta o fallback de glifos CJK. |
| A6 | `http.rs:103`, `lyrics/http_win.rs` | Código WinHTTP duplicado, e o segundo não confere `Content-Length`. | Um `http::get` só, com limite. |
| A7 | `mac/launchers.rs:250` | `open -b` cria um processo. | `NSWorkspace openApplicationAtURL:configuration:completionHandler:`. |
| A8 | `mac/compat.rs:95` | `#[allow(deprecated)] activateIgnoringOtherApps`. | `activate()` no macOS 14+, mantendo o antigo atrás de checagem de versão (já está quase assim; só conferir). |
| A9 | `linux/mod.rs:81`, `system.rs:21` | `signal()` com handler que chama `OnceLock::get_or_init`, o que não é seguro num sinal. | `sigaction` e criar o pipe antes de instalar o handler. |
| A10 | `linux/system.rs:63` | Cores do portal lidas uma vez, com D-Bus bloqueante de 600 ms na thread do UI. | Assinar `SettingChanged` numa thread de fundo. |

### 1.5 Boas práticas de Rust e organização do código

1. **`unsafe` documentado.** Há 314 blocos `unsafe` sem comentário `// SAFETY:`
   no Windows e 88 no Mac (os que mais têm: `overlay.rs` 37, `lyrics_overlay.rs`
   31, `mixer.rs` 21, `taskbar.rs` 20).
   - Ligar `clippy::undocumented_unsafe_blocks` e documentar, arquivo por
     arquivo.
   - **Encolher o escopo:** o `main` inteiro está num único `unsafe {}`
     (`main.rs:135-227`), e o certo é que cada bloco cubra só a chamada FFI.
   - Envolver os padrões repetidos em funções seguras (RAII para GDI e COM,
     `HString`), o que reduz o número de blocos.
2. **Soundness.**
   - `mac/gfx.rs:19`: uma `pub fn` segura monta uma fatia de `w*h*4` bytes sem
     conferir o tamanho. Pôr um `assert`/`if` de tamanho, ou marcar como
     `unsafe fn`.
   - `linux/audio.rs:120`: um `&mut` criado antes de travar o mainloop é
     aliasing de `&mut`, UB formal. Usar o ponteiro cru até travar.
   - `tray.rs:26`: `static mut ICON_*`, que o Rust 2024 desencoraja. Trocar por
     `AtomicPtr` ou `OnceLock`.
3. **Estratégia de panic.** O `catch_unwind` de `mac/notch.rs:155` e
   `mac/lyrics.rs:69` é código morto com `panic = "abort"`. Uma de duas:
   - **(a)** manter o abort, remover a falsa recuperação e endurecer o código
     (recomendado, porque o binário fica menor e o comportamento, previsível);
   - **(b)** passar a `panic = "unwind"` só no Mac. Os métodos do objc2 já são
     `C-unwind`.
4. **Lints no `Cargo.toml`.** Adicionar `rust-version = "1.88"` (o código usa
   let-chains) e uma seção `[lints]`:
   ```toml
   [lints.rust]
   unsafe_op_in_unsafe_fn = "warn"
   [lints.clippy]
   undocumented_unsafe_blocks = "warn"
   collapsible_if = "allow"   # o estilo do projeto prefere ifs separados
   ```
   O clippy padrão já está quase limpo. Além dos blocos `unsafe` sem comentário,
   sobram:
   - 22 `if` que dá para juntar;
   - 9 `Rc<Vec>`/`Arc<Vec>` que poderiam ser `Rc<[T]>`/`Arc<[T]>`;
   - `iter().any()` no lugar de `contains()`;
   - `unwrap` depois de `is_some` em `menu.rs:445`;
   - um ponteiro pendente manual em `tray.rs:90` (trocar por
     `ptr::dangling()`);
   - um `if` com ramos idênticos em `demo_gif.rs:661`.
5. **Código morto escondido.** `main.rs:15` tem um
   `#![cfg_attr(any(macos,linux), allow(dead_code))]` que vale para o crate
   inteiro. Trocar por `#[cfg]` nos itens que são só do Windows.
6. **Erros com tipo.** O update e o install usam `Result<_, String>`. Criar um
   `enum UpdateError` (sem dependência nova, com `Display` à mão) permite reagir à
   causa: sem rede, assinatura inválida ou disco cheio.
7. **Arquivos gigantes.** Cada divisão é um PR só de mover código, sem mudar
   comportamento, o que facilita a revisão:

   | Arquivo | Linhas | Dividir em |
   |---|---|---|
   | `settings.rs` | 5,3 mil | `settings/{layout, pages, preview, input, window}.rs` |
   | `ui.rs` | 3,3 mil | `ui/{widget, card, mixer, layers}.rs` |
   | `overlay.rs` | 2,4 mil | `overlay/{window, loop, input}.rs` |
   | `lyrics_overlay.rs` | 2,1 mil | separar janela e desenho |

   A Etapa 2 (páginas no card) já mexe em `ui.rs` e `overlay.rs`, então dividir
   antes evita conflito.
8. **Funções com 8-11 argumentos** (23 casos marcados com
   `#[allow(clippy::too_many_arguments)]`). Agrupar em structs de contexto
   (`DrawCtx { cv, tr, s, look, cfg }`), o que também simplifica as assinaturas
   das features novas.
9. **Formatação.** `cargo fmt --check` aponta 2.383 trechos: o código segue um
   estilo próprio, com linhas longas. A decisão fica com você (ver "Decisões em
   aberto"). A recomendação é um `rustfmt.toml` (`max_width = 160`,
   `use_small_heuristics = "Max"`) que fique perto do estilo atual, aplicado num
   **commit único** listado em `.git-blame-ignore-revs`, para o `git blame`
   continuar útil.
10. **Pequenos:** `build.rs:15` ordena as versões do SDK como texto (comparar
    como números); no `.desktop`, escapar `%` como `%%` (`linux/system.rs:108`).

### 1.6 CI, ferramentas e medição

- **Lint nos três workflows:** `cargo clippy --all-targets -- -D warnings` (depois
  de zerar os avisos da 1.5) e `cargo fmt --check` (depois da decisão da 1.5.9).
- **Testes em debug:** rodar `cargo test` também sem `--release`, para que
  `debug_assert`, as checagens de overflow e o código `cfg(debug_assertions)`
  rodem no CI.
- **Toolchain fixo:** um `rust-toolchain.toml` no lugar do `rustup update stable`.
  A atualização do Rust vira um PR que dá para revisar.
- **Dependências:** `cargo deny` (licenças, que já importam para o
  `THIRD-PARTY-NOTICES`, e advisories) e `cargo audit`.
- **Segurança dos workflows:**
  - bloco `permissions:` mínimo em `windows.yml` e `linux.yml`;
  - `windows.yml` rodar também em push para a `main`;
  - `build-linux.sh` com `cargo build --locked`;
  - o `appimagetool` com versão e checksum fixos, em vez do "continuous".
- **Teste de longa duração:** um job manual de *soak* de 5 h 30 no CI (o limite
  de um job hospedado é 6 h) e 8 h contínuas numa máquina local
  (`tools/soak-local.*`), com o app tocando uma playlist de teste, que registra a memória (working set, handles GDI, número
  de threads) a cada 5 min e falha se crescer de forma contínua. É o que pega
  vazamentos como V1, V5 e V6.
- **Medição de consumo:** um `TASKBEAT_FRAMESTATS=1` que registra o tempo por
  quadro (p50, p95, p99) e os despertares por segundo. A tabela do README vira
  um teste: cada PR da Etapa 1 anota o antes e o depois.

---

## Etapa 2 — Fundações técnicas

### F1 Config v6 e campos novos
Todos com default e com migração de `version < 6` em `load()`:

| Campo | Tipo | Item |
|---|---|---|
| `art_click` | `ArtClick { OpenPlayer, OpenCard, Nothing }` | 3 |
| `idle_widget` | `IdleWidget { Full, Compact, Hidden }` | 9 |
| `sources` | `SourceFilter { mode: All \| Only \| Except, ids: Vec<String> }` | 10 |
| `seen_sources` | `Vec<SeenSource { id, name, last_seen }>` | 10 |
| `motion` | `Motion { Full, Reduced, Off }` | 4, 5, 7 |
| `cover_viz` | `CoverViz { Bars, Vinyl, None }` | 7 |
| `pinned` | `Vec<Pinned { name, uri, player }>` | 1 |
| `onboarding_done` | `bool` | 11 |

- `idle_widget` substitui o `always_show_widget`: `true` vira `Full` e `false`
  vira `Hidden`. O Mac e o Linux leem o mesmo campo (`linux/island.rs:314`,
  `mac/notch.rs:550`).
- O teste de "config antiga carrega" (`config.rs:953`) ganha um caso v5 → v6.
- **Compatibilidade para trás:** depende do B4. Quem voltar da 2.0 para a 1.4
  recebe um TOML com enums que a 1.4 não conhece. A 1.4.3 já tem que guardar o
  arquivo como `.bad` em vez de apagar.

### F2 Primitivas novas em `render.rs`
- `draw_image_rotated(img, center, radius, angle)`: bilinear com UV girado e
  cobertura circular. A matemática já existe em `lyrics_bg::layer()`. Serve ao
  item 7.
- `lyrics_bg::fill_rect(cv, rect, radius, art, t, mix, &mut BgCache)`: recortado
  num retângulo arredondado, com cache **por superfície**, no lugar do
  `thread_local!` único, e com os buffers reaproveitados do D15. Serve ao item 5.
- Redução por mipmap (caixa 2×2 em cascata) para imagens que encolhem mais de 2×.
  Serve ao item 12.

### F3 Páginas no card e na ilha
- `enum CardPage { Player, Library, Search }` em `CLayout`/`CardFrame`, com
  transição deslizando (≈220 ms, mola).
- Hit-test por página (`CLayout::hit`, `ui.rs:2076`).
- O mesmo enum na ilha comum (`notch.rs`), que serve ao Mac e ao Linux.
- Ao fechar o card, volta para `Player`, como o mixer faz (`overlay.rs:2044`).

### F4 Campo de texto reutilizável
- Extrair a lógica que já existe nas configurações (`search_char`,
  `settings.rs:4566`; desenho em `settings_controls.rs:248`) para um `TextField`
  comum.
- **Risco:** o card é `WS_EX_NOACTIVATE` (`overlay.rs:787`) e não recebe
  teclado. Ao focar o campo, tirar o estilo e chamar `SetForegroundWindow`; ao
  sair, devolver.
- **Mac:** `canBecomeKeyWindow` só nesse momento.
- **Wayland:** `keyboard_interactivity = on_demand`.

### F5 Motor de movimento
- Uma mola criticamente amortecida com posição e velocidade, no lugar do
  `anim::Toggle` linear (`anim.rs:227`).
- Uma animação interrompida no meio muda de alvo sem solavanco.
- É a base dos itens 4, 7, 8 e 9.

---

## Etapa 3 — Recursos de comportamento

### Item 3: clicar na capa abre o app que está tocando
- **Windows:** `Hit::Art` (`ui.rs:46`) chama `launchers::focus_player(aumid,
  title)` (`launchers.rs:487`), que já serve ao selo do app (`overlay.rs:1918`).
- **Mac:** um braço no clique da ilha (`mac/notch.rs:927`) que chama
  `mac::launchers::open(&m.aumid)`, já com o NSWorkspace do A7.
- **Linux:** um novo `MediaCmd::Raise`, que lê `CanRaise` e chama `Raise` em
  `org.mpris.MediaPlayer2` (`linux/media.rs`). O clique fica em
  `linux/island.rs:725`.
- **YT Music no navegador:** um comando novo na extensão, `{cmd:"focus"}`, que
  chama `chrome.tabs.update({active:true})` e `chrome.windows.update({focused:true})`.
- **Configuração:** "Clique na capa: abrir o player / abrir o card / nada".

### Item 9: widget compacto quando nada toca (Windows)
- Hoje o widget fica largo com "Nada tocando" (`ui.rs:1583`), ou some.
- **Novo `Compact`:**
  - uma pílula do tamanho de um ícone da barra, com o ícone do Taskbeat ou a
    última capa esmaecida;
  - o hover mostra "Nada tocando";
  - o clique abre os atalhos dos apps.
- **Onde mexer:** variante em `WLayout::build_for` (`ui.rs:1383`) e na decisão de
  `overlay.rs:1327-1398`. O `taskbar::compute` já recebe a largura.
- **Transição:** a largura anima com a mola da F5, e só encolhe depois de 10 s
  sem nada, para não piscar entre faixas.

### Item 10: escolher quais apps são "ouvidos"
- **Ganchos:**
  - Windows: `.filter()` em `pick_session` (`media.rs:353`); o `State` passa a
    receber o filtro.
  - Mac: em `adapter_publish` (`mac/media.rs:352`), chamar `publish_empty()`.
    **Limitação:** o MediaRemote só informa um app, então o filtro esconde, mas
    não troca de player.
  - Linux: em `add()`/`pick()` (`linux/media.rs:430`, `:521`).
- **Lista para escolher:** `seen_sources` mais a tabela `KNOWN` dos launchers
  (`launchers.rs:57`). Modos "Todos", "Só estes" e "Todos menos estes".
- **Navegador:** o AUMID é do navegador inteiro. Para separar o YouTube vídeo do
  YouTube Music, uma sub-fonte com o site que a extensão informa.

---

## Etapa 4 — Visual e movimento

### Item 4: animações mais fluidas
O ponto de partida é a medição da 1.6. Depois, nesta ordem:
1. **Taxa do monitor só nas transições curtas** (abrir e fechar o card, trocar de
   página). Hoje o teto é de cerca de 75 fps (`vsync.rs:23`), e telas de 120/144
   Hz pegam um vblank a cada dois. Os ritmos decorativo (30 fps) e parado
   (250 ms) do `set_rate` (`overlay.rs:956`) ficam como estão.
2. **A mola da F5** no card, no mixer, nas páginas e no hover da ilha.
3. **Escala do card:** com a mip da F2 e aritmética inteira, no lugar da bilinear
   em float (`anim.rs:309`). No Mac, a escala fica com o `CALayer.transform`, que
   é GPU.
4. **Mac:** o buffer reaproveitado do D3; abaixo do macOS 14, CVDisplayLink no
   lugar do NSTimer de 60 Hz.
5. **X11:** derivar o intervalo de quadros da taxa do monitor, pelo RandR.
6. **Acessibilidade:** respeitar "reduzir movimento" do sistema. Com ele ligado,
   `motion = Reduced`.

### Item 5: fundo animado na barra, igual à tela cheia
- **Opções:** `WidgetTint::Animated`, `CardStyle::Animated` e `NotchBg::Animated`,
  com o `fill_rect` da F2.
- **Desenho:** o fundo sai do cache, e o resto continua em cache por cima. O
  widget (≈300×48 px) tem custo desprezível; no card, medir.
- **Efeitos extras** (cada um uma opção):
  - pulso no ritmo dos graves (`dsp.rs`);
  - *shimmer* na troca de faixa;
  - gradiente fluindo, sem usar a capa.
- **Economia:** ~12 fps, só tocando. Para quando pausado, em tela cheia, com
  `motion = Reduced` e (configurável) na bateria.

### Item 7: disco de vinil no lugar das barras
- **Opção:** `cover_viz = Vinyl`.
- **Desenho:**
  - sulcos por SDF;
  - a capa como selo central, girando com o `draw_image_rotated`;
  - furo no meio;
  - reflexo especular **fixo**: a luz não gira com o disco, e é isso que vende o
    efeito.
- **Física:** 33⅓ rpm; desacelera em cerca de 0,8 s ao pausar, pela mola da F5.
- **Onde vale:**
  - widget e ilha fechada: custo baixo;
  - card: é a parte cara, fica opcional e numa segunda etapa.
- **Cache:** a capa sai de `LayerTag::Card` (`ui.rs:2328`), e "tocando com vinil"
  passa a contar como decorativo no `set_rate`.

### Item 12: mais resolução em todos os dispositivos
- **Tamanho da capa:** o `ART_SIZE = 384` (`media.rs:39`, `mac/media.rs:41`,
  `linux/media.rs:38`) passa a ser calculado pelo maior tamanho físico pedido,
  com teto de 1024.
- **Miniatura pequena:** quando o SMTC dá menos que o necessário (o YT Music dá
  120 px, `cover.rs:1`), usar o `cover::fetch` em alta resolução também no card.
- **Redução:** pela mip da F2.
- **Troca de monitor:**
  - Windows: conferir o `WM_DPICHANGED` do card;
  - Mac: tratar `NSWindowDidChangeBackingPropertiesNotification`;
  - X11: reagir a `RRScreenChangeNotify` (hoje a escala é lida uma vez,
    `x11.rs:61`).
- **Texto:** conferir o *snapping* da linha de base em 125, 150 e 175%, nos três
  sistemas.
- **Regressão visual:** matriz de capturas em 100, 125, 150, 175, 200 e 250% no
  CI dos três sistemas.

---

## Etapa 5 — Biblioteca no pop-up

> **Limite técnico.** Nenhuma API do sistema expõe playlists nem busca: o SMTC
> não tem nada, o MediaRemote também não, e o MPRIS `Playlists` é raro. Ter
> resultados dentro do app exigiria a Web API de cada serviço, com OAuth, chave
> de app e o envio de dados a terceiros. Por isso a proposta é **"abrir no
> player"**, com resultados no app só onde o sistema dá sem login.

### Item 8: botão no pop-up para playlists e busca
- **Ícones:** biblioteca e lupa, ao lado do botão do mixer no card e na ilha
  aberta.
- **Páginas:** trocam pela F3; um mini player fica no topo de cada página.
- **Atalho opcional:** `Win+Alt+F` abre o card já na busca.

### Item 2: busca rápida
O campo de texto é o da F4. **Enter** abre a busca no player atual:

| Player | Como |
|---|---|
| Spotify | `spotify:search:<q>` |
| YouTube Music | aba aberta: `{cmd:"navigate", url:"…/search?q=…"}` na extensão; senão, abre a URL no navegador |
| Apple Music (Mac) | AppleScript `search playlist "Library" for "<q>"`, **com resultados no app** |
| Outros | URL de busca configurável |

Fica um histórico local das últimas 5 buscas, que dá para apagar.

### Item 1: abrir uma playlist pelo app
- **Playlists fixadas (todos os sistemas):**
  - o usuário cola um link do Spotify, do YT Music ou do Apple Music;
  - o app reconhece o player e guarda em `pinned`;
  - um clique abre a playlist no player;
  - um botão "Fixar a playlist que está tocando".
- **Listas automáticas** (onde o sistema dá):
  - Apple Music no Mac, por AppleScript (`get name of every user playlist`,
    `play playlist "X"`, usando a infraestrutura de `mac/media.rs:634`). Usar o
    timeout do D22.
  - Linux: MPRIS `Playlists` (`GetPlaylists`, `ActivatePlaylist`).
  - YT Music: lendo o DOM pela extensão. É frágil, então fica atrás de uma opção.

---

## Etapa 6 — Configurações e primeira execução

### Item 6: tela de configurações
**Causas hoje (Windows, `build_layout`, `settings.rs:1902`):**
- **Proporção fixa:** a coluna de opções ocupa `0,47 × largura`, com limite de
  880 px.
- **Prévia do widget pequena:** a altura natural é `150·s` (`settings.rs:1749`),
  e a escala vai no máximo a 1,8× (`settings.rs:1778`).
- **Largura estreita:** a prévia é espremida a no máximo 42% da altura.
- **Controles desalinhados:** uma coluna só, com controles de largura fixa (200-320
  e 220 px), cada um à direita da sua própria largura.

**Proposta:**
1. **Grade rótulo | controle.** A coluna de controle tem a maior largura da
   seção, então todos os controles começam no mesmo x.
2. **Proporção responsiva:**
   - abaixo de 1000 px, palco em cima;
   - entre 1000 e 1400 px, 55/45;
   - acima de 1400 px, opções em **duas colunas de cartões** e o palco *sticky*
     à direita.
3. **Prévia em contexto:** uma faixa de barra de tarefas com papel de parede (ou
   o notch), com o widget e o card juntos na escala real.
4. **Mostrar a mudança:**
   - o hover numa opção contorna a região da prévia que ela afeta (uma tabela
     `key → região`, ao lado da `requirement`, `settings.rs:786`);
   - mudar um valor faz um crossfade de antes para depois;
   - um botão "antes / depois" segurável.
5. **Mac:** liberar o redimensionamento, que hoje é fixo em 620 px
   (`mac/settings.rs:507`); acima de ~900 px, a prévia fica ao lado.
6. **Validar antes:** testar o layout novo nos PNGs do `settings_mock.rs` antes
   de mexer na tela real. Depende da divisão de `settings.rs` (1.5.7).

### Item 11: tutorial na primeira instalação
- **Quando aparece:** `first_run` (`main.rs:184`) mais `onboarding_done == false`.
  Na migração v6, quem já tinha config ganha `onboarding_done = true` e vê um
  "Novidades da 2.0" (no aviso de `update/popup.rs`). O instalador hoje abre com
  `--settings` (`setup.rs:932`); na primeira vez, abre o tutorial.
- **Passos** (todos com **Pular tutorial**, **Voltar** e uma barra de progresso):
  1. boas-vindas, com a animação de `demo_gif.rs`;
  2. o visual (Palco, Estúdio ou Ilha), com prévia ao vivo (`draw_preview`,
     `settings.rs:3560`);
  3. o widget: estilo e fundo, incluindo o animado;
  4. a capa: barras ou vinil, e o que o clique faz;
  5. os apps ouvidos (item 10);
  6. a letra e a extensão (o atalho `Win+Alt+L`);
  7. pronto: "Iniciar com o sistema" e um botão para as configurações.
- **Por sistema:**
  - Windows: uma janela própria com o `Canvas` e as skins de
    `settings_controls.rs`, na família visual do `setup.rs`;
  - Mac: uma janela AppKit;
  - Linux: 3 ou 4 dicas na própria ilha.
- **"Rever o tutorial"** em Configurações › Geral.

---

## Ordem e dependências

```
Etapa 1  1.1 bugs ──────────> 1.4.3 (hotfix na main)
         1.2 / 1.3 / 1.4 ──┐
         1.5 (divisão dos arquivos, lints) ──┐
         1.6 CI + medição ──┴───────────────┤
Etapa 2  F1 config v6 · F2 render · F3 páginas · F4 TextField · F5 mola
Etapa 3  3 · 9 · 10                    (depende de F1, F5)
Etapa 4  4 → 5, 7 · 12                 (depende de F2, F5, 1.3, A1)
Etapa 5  8 → 2 → 1                     (depende de F3, F4)
Etapa 6  6 → 11                        (depende de tudo acima)
                         └──> 2.0.0-beta.N a cada etapa ──> 2.0.0
```

Podem correr em paralelo:
- a Etapa 3 com a Etapa 4, porque mexem em arquivos diferentes;
- dentro da Etapa 1, Windows, Mac e Linux, porque são módulos separados.

## Critérios de pronto para a 2.0
- [ ] Zero panic vindo de dados externos: os testes cobrem B1, B2, B3, B6 e B7.
- [ ] O clippy passa com `-D warnings`, e todo `unsafe` tem `// SAFETY:`.
- [ ] *Soak* de 5 h 30 no CI e de 8 h local sem crescimento contínuo de memória, handles ou threads nos
      três sistemas.
- [ ] CPU parado ≤ ao da 1.4.2 (tabela do README), e cada efeito novo anota seu
      custo.
- [ ] A config v6 migra de v1 a v5, e a 1.4.3 não apaga uma config v6.
- [ ] Capturas em 6 escalas nos três sistemas, e as quatro línguas completas.
- [ ] O README e o `THIRD-PARTY-NOTICES` estão atualizados.

## Decisões em aberto
1. **Item 9:** "nada tocando" inclui "pausado há mais de N minutos"?
2. **Itens 1 e 2:** "abrir no player" basta para a 2.0, ou vale a Web API do
   Spotify com login?
3. **Item 5:** o fundo animado vira o padrão ou fica como opção?
4. **Item 11:** no Linux, as dicas na ilha bastam por enquanto?
5. **1.5.3:** manter `panic = "abort"` (recomendado) ou passar a `unwind` no Mac?
6. **1.5.9:** adotar o `rustfmt` com um `rustfmt.toml` perto do estilo atual, ou
   manter a formatação à mão e não cobrar `fmt` no CI?
7. **1.4.3:** lançar as correções de bug na `main` antes da 2.0 (recomendado)?
