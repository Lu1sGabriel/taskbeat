# Taskbeat: regras do projeto

Widget de música em Rust (edição 2024) para a barra de tarefas do Windows, o
notch do Mac e uma ilha no Linux. Os planos estão em `docs/` e o que já foi
feito, por área, em `docs/historico/`.

## O que nunca muda

- **`panic = "abort"` na release.** Qualquer panic fecha o app, e o
  `catch_unwind` não pega nada. Um panic vindo de dado externo (tag de música,
  arquivo, rede, config) é bug de severidade alta. Nada de `unwrap`, `expect`
  ou índice sobre dados externos.
- **Os testes ignoram o `panic = "abort"`.** Um teste verde não prova que a
  release não aborta: os casos de panic têm testes unitários diretos, e o soak
  (`soak.yml`, `tools/soak-local.*`) roda o **binário de `cargo build
  --release`**, nunca `cargo test`.

## Config (`src/config.rs`)

- **Carga tolerante.** Um `config.toml` que não abre inteiro não vira os
  padrões calado: o arquivo lido vai para `config.toml.bad` (sem sobrescrever
  um que existe: `.bad.1` … `.bad.5`), e ficam as chaves que ainda servem, uma
  a uma. Depois de uma carga tolerante, **não gravar** de imediato.
- **Nunca rebaixar a `version`.** Um arquivo de uma versão mais nova continua
  com o número dela, na carga e na gravação. A migração só anda para a frente.
- `sanitize()` antes de qualquer gravação, e todo `f32`/`f64` passa por
  `is_finite()` (o `clamp` deixa o NaN passar).
- Gravação sempre por `crate::files::write_atomic` (`.tmp` com o PID e um contador na mesma
  pasta, depois `rename`). Vale para `update.toml` e o cache de letras.
- Campo novo: com default (`#[serde(default)]` no `Config`) e teste de
  migração.

## Tempo

- **`Option<Instant>` para "nunca" ou "há muito tempo"**, e nada de subtrair
  `Duration` de `Instant` (`Instant::now() - d`). Use `crate::clock`:
  `elapsed_or_max`, `since`, `back`, `secs_f32`. O crate tem
  `#![warn(clippy::unchecked_time_subtraction)]`.
- `Duration::from_secs_f32` dá panic com NaN, negativo ou infinito: use
  `clock::secs_f32`.
- Relógio que só soma por horas (animação contínua): `f64`, e ângulos
  reduzidos a uma volta antes de virar `f32`.

## Texto

- **Nunca fatiar uma string pelo índice achado em `to_lowercase()`.** O
  `to_lowercase` muda o tamanho em bytes ('İ' 2→3, 'K' Kelvin 3→1, 'ẞ' 3→2).
  Para achar separadores ASCII e cortar, use `to_ascii_lowercase()`, que
  preserva os offsets. O `to_lowercase` só serve para comparar.
- Textos novos nas 4 línguas (PT é a chave da tabela em `i18n.rs`).

## Plataformas

- **Mac:** nada de `runModal` dentro de um bloco da fila principal (congela a
  ilha). Timers por `compat::common_timer` (common modes); os `with` de cada
  janela usam `try_borrow_mut`, nunca `borrow_mut`, porque um tick pode
  chegar no meio de outro.
- **Linux/Wayland:** uma superfície fechada é recriada com um `wl_surface`
  novo; nada de anexar buffer antes do primeiro `configure`. Arquivos vindos
  do player (capa) abrem com `O_NONBLOCK`, `is_file()` no arquivo aberto e
  limite de bytes; imagens têm limite de área antes de decodificar.

## Trabalho

- Commits `tipo(área): descrição em português` (`fix(letras): …`,
  `ci(...)`, `docs(...)`), um por PR do plano, terminando com a linha
  `Co-Authored-By:` combinada. Commit só de docs leva `[skip ci]`.
- Suíte antes de entregar, uma por vez na máquina: `cargo test --locked` e
  `--release` no Windows; `cargo clippy --all-targets --locked` no Windows e
  com `--target aarch64-apple-darwin` (sem aviso novo); `cargo test` no
  Linux pelo WSL (`CARGO_TARGET_DIR` no ext4); o CI do Mac no branch.
- `graphify-out/` é gerado e não vai para o git.
- Toolchain fixo em `rust-toolchain.toml` (a partir da `release/2.0`): trocar
  de versão é um PR próprio. Nada de `RUSTUP_TOOLCHAIN` nos workflows.

## Medição

- PR `perf`, ou mudança que roda por quadro: medir antes e depois com
  `tools/measure.ps1`/`tools/measure.sh` e `TASKBEAT_FRAMESTATS=1`, contra a
  base em `docs/medicoes/`.
- Ponto de medição novo: `crate::stats::frame`/`wake`, lendo o relógio só
  dentro de `stats::enabled()`, para desligado custar um `if`.
