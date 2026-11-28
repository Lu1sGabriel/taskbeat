# Histórico: diagnóstico

## 2.0 (branch `release/2.0`, etapa 1, M0)

**PR-00 `feat(diagnostico): medição de quadros e consumo`**
- `src/stats.rs`, ligado por `TASKBEAT_FRAMESTATS=1`. Desligado, cada chamada
  custa a leitura de um `OnceLock<bool>` e um `if`: os pontos de medição só
  leem o relógio quando `stats::enabled()`.
- Ligado, guarda o tempo de desenho e de apresentação de cada quadro por
  superfície e conta os despertares por origem. A cada 60 s acrescenta um
  bloco em `stats.log`, na pasta da config (vira `stats.old.log` passando de
  1 MB), com p50/p95/p99 e os despertares por segundo. No Windows o bloco traz
  também os handles do kernel (`GetProcessHandleCount`) e os objetos GDI e
  USER (`GetGuiResources`), que já estão em `Win32_System_Threading`: nenhuma
  feature nova.
- Superfícies: `widget`, `card` e `card-barras` (Windows), `ilha` (Mac e
  Linux). Despertares: `overlay`, `vsync`, `audio-loopback`, `audio-player`,
  `media` e `mixer` (Windows); `ilha-tick`, `letra-tick` e `teclas-tick`
  (timers do Mac); `ilha` e `media` (Linux).
- `tools/measure.ps1`: cenários `parado`, `tocando` (com o player de teste no
  SMTC) e `card` (TV_DEMO_FLYOUT), 5 min cada, depois de 1 min de aquecimento;
  os cenários à mão (mixer, letra em tela cheia, uso real) por `-AttachPid`.
  Três contagens separadas pelo `tools/proc-sample.ps1` (handles do kernel com
  `Refresh()`, GDI e USER com `GetLastError`), o working set e a memória
  privada.
- `tools/measure.sh` (Mac e Linux): RSS e threads pelo `ps`/`/proc`,
  descritores pelo `lsof`/`/proc/<pid>/fd`; não há equivalente de GDI.
- **Entrega:** `docs/medicoes/1.4.2.md`.

**Correções da revisão (mesmo branch):**
- `measure.ps1` marca o cenário como pulado se o player de teste sair (antes
  media o app parado com o rótulo `tocando`), acrescenta à `medicao.md` em vez
  de recriar (o `-AttachPid` apagava os automáticos) e apaga o WAV do player.
- `measure.sh` recusa abrir com outro Taskbeat aberto e confere que o app
  continua de pé logo depois de abrir.
- Linux: o "desenho" da ilha mede só o canvas, como no Mac e no Windows.

**Descartados na revisão:** o relatório de 60 s montado na thread do quadro
(só com a medição ligada, e o disco fica fora da trava; os três verificadores
refutaram); os despertares do CADisplayLink do Mac (o vblank só anda quando a
ilha anima, e então desenha); o primeiro bloco do `stats.log` com a abertura
(o plano não pede alinhar as janelas).

**Pendente:** a medição da 1.4.2 no Windows (ver `docs/medicoes/1.4.2.md`) e
no Mac.
