# Histórico: config

## 1.4.3 (branch `hotfix/1.4.3`, etapa 1)

**PR-12 `fix(config): config com erro não apaga mais as preferências`** (B4, B5, B6)

**Antes:** um `config.toml` que não abria (erro de digitação, ou um valor que
só uma versão mais nova conhece) virava os padrões calado, e a próxima
gravação apagava as preferências. O `save()` usava `fs::write`, que trunca o
arquivo antes de gravar, e a migração gravava antes do `sanitize()`.

**Formato:** o mesmo TOML de sempre, `version = 5`. Nenhum campo novo.

**Carga (`Config::load` → `load_from`):**
1. `toml::from_str::<Config>`. Deu certo: acabou.
2. Senão, o arquivo lido de fato (o atual ou o do nome antigo do app) é
   **copiado** para `config.toml.bad`, sem sobrescrever um que já existe
   (`.bad.1` … `.bad.5`; com as seis vagas ocupadas, não copia e registra).
3. O texto é lido como `toml::Table`. Com erro de sintaxe: os padrões.
4. As chaves entram uma a uma (`try_into::<Config>` com `clone()` por
   tentativa). Uma tabela que não serve inteira, como `[lyrics]`, fica com as
   chaves dela que servem. As que caem vão para o log (`TASKBEAT_LOG`).
5. **Não grava** depois de uma carga tolerante: a variante desconhecida não é
   regravada por cima. A próxima gravação é a da próxima mudança da pessoa.
6. `sanitize()` antes de qualquer gravação (B5); `sensitivity_db` NaN ou
   infinito volta a 0 (B6). `lyrics.free_x`/`free_y` já tinham `is_finite`.

**Compatibilidade e downgrade:**
- **1.4.2 → 1.4.3:** nada muda no arquivo.
- **2.0 → 1.4.3:** a `version` lida (6) é **mantida**, na carga e na gravação;
  a migração só anda para a frente. Assim a 2.0 não "remigra" um arquivo que a
  1.4.3 regravou, e a 1.4.3 não apaga o número que a 2.0 precisa ver. Um valor
  que a 1.4.3 não conhece cai para o padrão dela (e a cópia fica no `.bad`);
  as chaves que ela não tem são descartadas na próxima gravação, porque o
  `Config` não usa `deny_unknown_fields`.
- **1.4.3 → 2.0 de novo:** a 2.0 vê `version = 6` e não migra. O que a 1.4.3
  descartou ao gravar volta ao padrão da 2.0 (o PR-40 decide se lê o `.bad`).

**Gravação (`save` → `save_to`):** `files::write_atomic` grava
`config.toml.<pid>.<n>.tmp` na mesma pasta, faz `sync_all` e troca com `rename`.
No Windows o `rename` do std usa `MoveFileExW(REPLACE_EXISTING)`; um erro
(como `SHARING_VIOLATION`) vai para o log, e o arquivo anterior continua. O
mesmo vale para `update.toml` e o cache de letras. Um `.tmp` de outra execução
é ignorado pela carga.

**Testes:** variante desconhecida (as outras chaves ficam, sem gravar),
tabela com uma chave ruim, erro de sintaxe com `.bad`, `.bad` existente não
sobrescrito, NaN e infinito, `version = 6` mantida, migração já sanitizada,
`.tmp` velho ignorado, primeira execução e nome antigo do app.

**Correções da revisão (mesmo branch):**
- Numa carga tolerante (inclusive com erro de sintaxe), `login_default_applied`
  vem `true`. Antes, os padrões traziam `false`, e o `apply_login_default` da
  inicialização gravava os padrões por cima na mesma abertura (e, no Mac,
  religava o item de login de quem o tinha desligado).
- Com erro de sintaxe, a `version = N` do topo do texto é lida assim mesmo, e
  a maior vale: um arquivo da 2.0 com erro continua 6.
- O `.tmp` leva o PID **e um contador por gravação**: duas threads gravando
  juntas não dividem mais o mesmo `.tmp`.
- Um `config.toml` que é link simbólico (dotfiles) continua link: a gravação
  vai para o arquivo de destino.

**Pendente:**
- O `save()` chamado de fora (`Shared::update_config`) não roda o `sanitize()`;
  os valores vêm da interface, que já limita. O M2 pode unificar.
- Os `.tmp` órfãos de uma execução morta no meio da gravação não são
  apagados (são poucos bytes e ignorados).
