# Histórico: Linux

## 1.4.3 (branch `hotfix/1.4.3`, etapa 1)

**PR-14 `fix(linux): capas enormes ou caminhos especiais`** (B8, B9)
- `src/linux/gfx.rs`, `read_capped`: comum a `file://` e a caminho puro. Abre
  com `O_NONBLOCK` (um FIFO não prende a thread no `open`), confere
  `is_file()` no arquivo já aberto (sem `metadata` antes, por causa do TOCTOU)
  e lê no máximo 16 MB. `artUrl=/dev/zero`, pastas e FIFOs são recusados.
- Limite de área `w·h ≤ 4096²` no cabeçalho, antes de decodificar, mantendo
  `MAX_SIDE = 8192` por lado: PNG depois do `decode_headers`; JPEG com o
  `dimensions()` movido para antes do `decode()`; WebP no lugar da checagem
  por lado (o `set_memory_limit` do image-webp não limita os pixels).
- `src/linux/media.rs`: uma thread só de capas (`Covers`), com fila de 1; a
  URL nova substitui a que esperava, e a que já carregava termina e vai para o
  cache. Antes era uma thread por URL, sem limite.

**PR-15 `fix(linux): ilha sobrevive a monitor removido e a compositores lentos`** (B10, B11)
- **B10:** o `closed` da layer-surface fechava o app. Agora `Wl::recreate`
  cria um `wl_surface` novo, com viewport e escala fracionada novos (a antiga
  agora é destruída). Ordem de destruição: layer, viewport, escala
  fracionada, superfície. Depois: os `set_*` iniciais, região de entrada
  vazia e commit sem buffer; o `configure` chega pelo laço normal (sem o
  `roundtrip` de 3 s do `open`). `frame_pending` zerado e pool novo.
  Só recria com alguma tela; sem nenhuma, espera o `global` de um
  `wl_output` novo. No máximo 5 recriações em 10 s, depois desiste (o app
  fecha como antes). Eventos de uma superfície destruída são ignorados pela
  geração no userdata.
- Telas: `global`/`global_remove` de `wl_output` no `Dispatch` do registro;
  na remoção, `release()` (versão ≥ 3), sai de `outputs` e de `entered`, e
  `rescale()`.
- **B11:** `configured` só zera quando o tamanho **lógico** muda, com espera
  máxima de 500 ms (o protocolo não promete `configure` a um `set_size`). O
  timeout não vale antes do primeiro `configure` de uma superfície nova.

**Correção da revisão:** sem nenhuma tela, a ilha esperava girando o laço a
100% de CPU (o `due` do quadro já tinha passado e o `poll` voltava na hora).
Agora o `Window::suspended()` faz o `timeout()` esperar só o descritor da
janela, que acorda com o `global` da tela nova ou com o `configure`.

**Testado num Sway headless (WSL, `WLR_BACKENDS=headless`):** com
`swaymsg create_output` e `swaymsg output HEADLESS-1 disable`, a ilha é
recriada na tela que sobrou; com todas desligadas por 10 s, o app fica vivo e
sem gastar CPU; com uma tela nova, é recriada de novo, recebe o `configure` e
aparece na captura (`grim`).

**Não verificado:** Wayland real com monitor físico. O KDE (desligar o monitor externo) e o Sway
com um monitor de verdade ficaram para uma máquina com compositor.

**Pendente (M2):** V10, V11 (o `curl` morto sem `wait()`), V12, D11 a D14, A9
e A10.
