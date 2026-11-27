# Publicar a extensão na Chrome Web Store

A loja serve para Chrome, Brave e Edge (no Edge, o usuário permite "extensões de outras
lojas" uma vez; ou publique também na loja do Edge, passo 9).

## 1. Conta

Crie a conta de desenvolvedor em https://chrome.google.com/webstore/devconsole (taxa única
de US$ 5). Use a conta do Google com que você quer aparecer como publicador.

## 2. Pacote

Na raiz do projeto:

```powershell
.\extension\store\pack.ps1
```

Gera `dist\taskbeat-extension-<versão>.zip`, sem o campo `key` do manifest (a loja não
aceita a chave de desenvolvimento e dá a própria). Para refazer as imagens:
`.\extension\store\images\make-images.ps1`.

## 3. Novo item

No painel: **Novo item** → envie o zip. Logo depois do envio (ainda em rascunho) o item já
tem um **ID** (32 letras de a a p) e uma página `https://chromewebstore.google.com/detail/<ID>`.
Anote os dois: o Taskbeat precisa deles (passo 8).

## 4. Página do item (Store listing)

Copie de [`listing.md`](listing.md): descrição em inglês e em português, categoria,
ícone, capturas e bloco promocional (pasta `images/`).

## 5. Privacidade (Privacy practices)

- **Finalidade única:** Show and control YouTube Music shuffle and repeat in the Taskbeat
  app for Windows. / Mostrar e controlar o aleatório e o repetir do YouTube Music no app
  Taskbeat para Windows.
- **Justificativa das permissões:**
  - `nativeMessaging`: talk to the Taskbeat app installed on the same computer, the only
    place the messages go. / falar com o app Taskbeat instalado no computador, o único
    destino das mensagens.
  - `scripting`: add the script to YouTube Music tabs that were already open when the
    extension is installed, updated or re-enabled (otherwise it only works after reloading
    the page). / colocar o script nas abas do YouTube Music já abertas quando a extensão é
    instalada, atualizada ou religada (senão, só depois de recarregar a página).
  - `storage`: keep, in session memory only (`storage.session`), a flag saying the open tabs
    were already prepared. / guardar, só na memória da sessão, a marca de que as abas
    abertas já foram preparadas.
  - Acesso a `https://music.youtube.com/*`: read shuffle, repeat, title and artist on the
    YouTube Music page and press the shuffle and repeat buttons when the user asks in
    Taskbeat. / ler aleatório, repetir, título e artista na página e apertar os botões de
    aleatório e repetir quando o usuário pede no Taskbeat.
- **Código remoto:** Não.
- **Uso de dados:** a extensão lê da página título, artista, álbum, tocando/pausado,
  aleatório e repetir, e manda só ao app no mesmo computador; nada vai para servidores, para
  você ou para terceiros. Minha leitura é que isso não é "coleta" (nada sai do aparelho),
  então nenhuma categoria marcada. Se preferir ser conservador, marque **Conteúdo do site**;
  não muda nada para o usuário.
  Marque as três declarações (não vende, não usa para outros fins, não usa para crédito).
- **Política de privacidade:** `https://github.com/Lu1sGabriel/taskbeat/blob/main/extension/store/PRIVACY.md`
  (vale depois que esta pasta estiver no `main`).

## 6. Distribuição

Pública, todos os países. Envie para revisão (costuma levar de alguns dias a algumas semanas;
`nativeMessaging` e acesso a site podem pedir revisão mais detalhada).

## 7. Testar a versão da loja

Ela terá outro ID que a carregada sem compactação: remova a de desenvolvimento antes de
instalar a da loja, e só funciona com um Taskbeat que conheça o ID novo (passo 8).

## 8. Taskbeat com o ID da loja

Em `src/webext.rs`:

```rust
pub const EXTENSION_IDS: &[&str] = &["abofmfclmpmmbgecemcglhcklnmcapfh", "<ID da loja>"];
pub const STORE_URL: &str = "https://chromewebstore.google.com/detail/<ID da loja>";
```

Gere o instalador (`.\build-installer.ps1`) e publique a versão nova do Taskbeat: o
instalador grava o host aceitando os dois IDs, e o botão **Instalar extensão** das
configurações passa a abrir a loja. Atualize o link em `extension/README.md` e no README.

Opcional: para a extensão de desenvolvimento ter o mesmo ID da loja, copie a chave pública
do item (painel → Pacote → "Chave pública") para o campo `key` do `manifest.json`.

## 9. Loja do Edge (opcional)

Conta gratuita em https://partner.microsoft.com/dashboard/microsoftedge; envie o mesmo zip,
com os mesmos textos. Ela dá outro ID: acrescente em `EXTENSION_IDS` como no passo 8.
