# Histórico: letras

## 1.4.3 (branch `hotfix/1.4.3`, etapa 1)

**PR-10 `fix(letras): artistas e títulos com letras especiais não fecham o app`** (B1, B2)
- **O que mudou:** `first_artist` e `clean_title` (`src/lyrics/lrclib.rs`)
  usam `to_ascii_lowercase()` para achar os separadores.
- **Por quê:** o índice achado em `to_lowercase()` cortava o texto original, e
  o `to_lowercase` muda o tamanho em bytes ('İ' 2→3, K de Kelvin 3→1, 'ẞ'
  3→2). "BİRİCİK, ŞEBNEM" cortava no meio de um caractere: panic na thread de
  letras e, com `panic = "abort"`, o app fechava.
- **O que não mudou:** `is_noise` e `channel_like` continuam com
  `to_lowercase`. Ali o índice não é reaproveitado, e "VÍDEO" precisa casar
  com "vídeo" da lista `NOISE`.
- **Testes:** os casos com 'İ', 'K' e 'ẞ' e uma propriedade sem panic com 20
  mil textos montados com essas letras e os separadores (gerador
  determinístico, sem crate nova).

**PR-13 `fix(letras,atualizacao): relógio e números inválidos`** (B7, B14, B15)
- **B7** (`src/lyrics/mod.rs`): `[offset:NaN]` ou `inf` num LRC jogava todas as
  linhas para 0 ou +inf. O offset agora precisa ser finito e fica em ±600 000
  ms, limitado já em ms.
- **B14** (`src/lyrics/cache.rs`): a marca "sem letra" com data no futuro (de
  um relógio adiantado) conta como vencida, para procurar de novo.
- **B15** (`src/lyrics_overlay.rs`, `src/mac/lyrics.rs`, `src/lyrics_bg.rs`):
  o relógio do fundo animado da tela cheia era `f32` e só somava; a partir de
  ~36 h andava em passos de 15,6 ms. Agora é `f64` até o `lyrics_bg`, que
  reduz cada ângulo a uma volta (`rem_euclid(TAU)`) em `f64` antes do `f32`.
- **Gravação do cache** de letras: atômica (`files::write_atomic`), como a
  config (PR-12).

**Pendente:**
- V13 (o `prune()` só na partida e a chave `#key` ignorada no `get`): M2.
- O `lyrics_bg` ainda aloca 3 buffers por quadro (D15): M2.
