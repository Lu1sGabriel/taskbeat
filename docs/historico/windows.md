# Histórico: Windows

## 1.4.3 (branch `hotfix/1.4.3`, etapa 1)

**PR-11 `fix: subtração de Instant sem risco de panic`** (B3, severidade baixa)
- `src/clock.rs`: `elapsed_or_max`/`since` (`checked_duration_since`), `back`
  (`checked_sub`) e `secs_f32` (`try_from_secs_f32`).
- `volume.rs` (`last_endpoint`, `last_read`) e `overlay.rs` (`last_input`,
  `last_sound`, `last_layout`) viram `Option<Instant>`. `last_layout = None`
  quer dizer "refazer o layout no próximo tick", o que antes era
  `now - 10 s`. O `osd_start` (`overlay.rs:1322`) e a prévia das
  configurações (`settings.rs:2366`) usam o `clock::back`.
- `#![warn(clippy::unchecked_time_subtraction)]` no crate (atributo; o
  `[lints]` fica para o PR-29).
- **Por que severidade baixa:** desde o rustc 1.97 o `Instant` do Windows
  começa em `u64::MAX/4` ns, então com o 1.100 não há panic logo depois do
  boot. Continua valendo porque não é garantia da API, o `rust-version =
  1.88` do PR-29 aceita compiladores em que o panic é real, e o
  `from_secs_f32(inf)` também dá panic.

**Não verificado:** se o binário publicado da 1.4.2 foi compilado com rustc
≥ 1.97 (o CI usava `rustup update stable`, sem registro da versão no
artefato).
