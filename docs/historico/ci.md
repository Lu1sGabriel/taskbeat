# Histórico: CI

## 1.4.3 (branch `hotfix/1.4.3`, etapa 1)

**PR-02 `test(ci): job de soak de 5 h 30 e soak local de 8 h`**
- `.github/workflows/soak.yml`: só `workflow_dispatch`, que aparece apenas com
  o arquivo na `main`; por isso vai junto com a 1.4.3. Matriz Windows, macOS e
  Ubuntu com `fail-fast: false`, `timeout-minutes: 350`,
  `permissions: contents: read` e o log sempre enviado (`if: always()`).
  - Input `alvo` (padrão `v1.4.2`): o ref compilado e testado. Os scripts e os
    players de teste vêm do ref do próprio workflow (checkout em
    `ferramentas/`).
  - Input `minutos` (padrão 330 = 5 h 30), passado por variável de ambiente.
    O script para de medir 8 min antes do timeout do job (`SOAK_DEADLINE`),
    para a análise e o envio do log caberem.
  - **5 h 30**, não 8 h: um job hospedado tem limite de 6 h, e quebrar em
    vários jobs zeraria o processo a cada um. As 8 h contínuas são locais.
- `tools/soak-local.ps1` e `tools/soak-local.sh`: o mesmo teste no CI e numa
  máquina sua. Rodam o **binário de release** (os testes ignoram o
  `panic = "abort"`). Uma amostra a cada 5 min; falha se o app fechar sozinho
  ou se a regressão linear, depois de 30 min de aquecimento, passar de 1 MB/h,
  10 handles/h (kernel, GDI e USER no Windows; descritores no Unix) ou
  1 thread/h. Com menos de 6 amostras depois do aquecimento o resultado é
  "inconclusivo". A config é limpa e não procura atualização nem liga o
  "abrir ao iniciar". `-AttachPid`/`--pid` só mede um processo já aberto.
- `tools/proc-sample.ps1`: as três contagens do Windows (handles do kernel
  com `Refresh()`, GDI e USER por `GetGuiResources` com `GetLastError`), com
  `PROCESS_QUERY_LIMITED_INFORMATION`. O PR-00 reaproveita.
- `examples/ci_player.rs`: toca um WAV em loop pelo `MediaPlayer`, que aparece
  no SMTC (conferido localmente com o `smtc_probe`). As features
  `Media_Core`/`Media_Playback` são só `dev-dependencies`: o binário de
  release não muda.
- No Mac o soak usa o `.app` do `build-macos.sh` (que faz `cargo build
  --release`), porque o adaptador do Tocando Agora vem dentro dele. No Linux,
  Xvfb (X11) com `dbus-run-session` e o `ci-mpris.py`.

**Correção da revisão:** o `soak-local.sh` matava só o processo do player. O
`ci-mpris.py` não trata SIGTERM, e o laço `while true; do paplay …; done` que
ele abre ficava órfão, tocando. O `pkill -x paplay` também matava qualquer
`paplay` da pessoa. Agora o script mata a árvore do player (o pai primeiro) e
mais nada.

**Correção da revisão do M0:** o `soak-local.ps1` mata o `ci_player` à força, e
o player não chegava a apagar o WAV dele (~1,9 MB em `%TEMP%` por soak). Agora o
script apaga o arquivo.

**Não verificado:**
- o soak hospedado (precisa do arquivo na `main` para o `workflow_dispatch`);
- se o runner do Windows, sem placa de som, deixa o `MediaPlayer` tocar. Se
  não deixar, o script registra e o soak mede o app parado.
