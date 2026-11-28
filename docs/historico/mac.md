# Histórico: Mac

## 1.4.3 (branch `hotfix/1.4.3`, etapa 1)

**PR-16 `fix(mac,atualizacao): aviso de versão não congela a ilha`** (B12)
- **O que congelava** (`src/mac/update.rs:15` e `:38`): o `runModal` rodava
  dentro de um `exec_async` na fila principal. O laço modal ignora mouse e
  teclado das outras janelas, e a fila serial deixava `media_changed`,
  `launchers_changed` e `config_changed` esperando.
- **Agora:** um `NSPanel` próprio, não modal (`Titled | Closable |
  UtilityWindow`, nível flutuante, `setHidesOnDeactivate(false)`). `show()`
  volta logo; os botões chamam seletores novos do `Target` (`updateOpen:`,
  `updateLater:`, `updateSkip:`). Enter abre a página; Esc, o x da janela e
  "Depois" contam como "Depois" (`windowWillClose:`). Painel e oferta ficam
  num `thread_local` até a escolha, com guarda contra uma segunda oferta, e o
  painel fechado fica guardado até o próximo (não é solto de dentro do
  próprio `close`). Nada de `NSAlert::window()` sem `runModal`.
- **Timers** da ilha (`notch.rs`), da letra (`lyrics.rs`) e do aviso de
  teclas (`osd.rs`): `compat::common_timer`, em `NSRunLoopCommonModes`.
  Seguem com o menu da barra aberto. Os três `with` usam `try_borrow_mut`,
  então um tick reentrante é pulado (conferido no código).
- Nenhuma feature nova no `Cargo.toml`.

**PR-11** (B3): `mac/media.rs` e `mac/osd.rs` trocam `Instant::now() - d` por
`clock::back` (no Unix não era crash; foi para padronizar).

**Não verificado:** o aviso num Mac de verdade (só o CI compila e roda os
testes); a reentrância dos ticks da letra e do aviso em tempo de execução.

**Pendente:** a tolerância e o invalidate dos timers (D4) ficam para o PR-24,
que mexe nas mesmas linhas.
