# Histórico: atualização

## 1.4.3 (branch `hotfix/1.4.3`, etapa 1)

- **B14** (`src/update/mod.rs`, `check_due`): `last_check` no futuro (relógio
  adiantado uma vez) conta como vencido. Antes o app só voltava a procurar
  quando chegasse aquela data.
- **B16** (`src/update/sig.rs`): o `assert!` sobre o `BCryptHash` virou
  `Result`. Quem confere o instalador trata a falha como "assinatura
  inválida", e o `verify_p256` devolve `false`.
- **B13** (`src/main.rs`): depois de atualizar, a versão nova espera a antiga
  fechar por 60 s (eram 15). Sem evento com nome: a antiga segura o mutex até
  morrer, o `WAIT_ABANDONED` já libera a espera (até num abort), e a 1.4.2,
  que é a antiga na primeira atualização, não teria o evento de qualquer
  forma. O estouro vai para o `update.log`.
- **B12** (Mac): o aviso de versão não é mais modal. Ver `mac.md`.
- `update.toml` gravado de forma atômica (ver `config.md`).

**Pendente:** o `enum UpdateError` (item 1.5.6 do plano) fica para o M2.
