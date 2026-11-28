<!-- Título: `tipo(área): descrição em português` (ex.: `fix(letras): …`).
     Tipos: feat, fix, perf, refactor, ci, docs, chore, test.
     Áreas: windows, mac, linux, card, widget, ilha, letras, config, render,
     atualizacao, extensao, ci. -->

Fecha: <!-- código do plano, ex.: B1, B2 -->

<!-- Em PR `perf` (ou quando o efeito roda por quadro): medição de antes e
     depois com `tools/measure.ps1`/`tools/measure.sh` e `TASKBEAT_FRAMESTATS=1`. -->

- [ ] `cargo test` (debug e release) passa no(s) sistema(s) afetado(s)
- [ ] Textos novos nas 4 línguas
- [ ] Capturas atualizadas, quando muda o visual
- [ ] Medição antes e depois, quando é `perf` ou quando o efeito roda por quadro
- [ ] Config: campo novo com default e teste de migração
- [ ] Nada de `unwrap`/`expect`/índice sobre dados externos
