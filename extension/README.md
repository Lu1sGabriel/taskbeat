# Extensão do Taskbeat (Chrome, Brave, Edge)

Opcional. Com ela, o Taskbeat sabe o estado real do **aleatório** e do **repetir**
do YouTube Music no navegador (numa aba ou no app instalado pelo navegador) e
comanda os dois, lendo a própria página. Ela também manda o endereço da **capa em
alta resolução** (o navegador só passa ao Windows uma de 120 px), que a letra em
tela cheia usa. Sem ela, o Taskbeat continua usando a acessibilidade do Windows
(UI Automation) e a cor do botão do aleatório.

> **Em breve na Chrome Web Store.** Até ela ser publicada, a extensão já vem com
> o Taskbeat, para instalar sem a loja (os passos abaixo).

## Instalar

Primeiro instale ou atualize o Taskbeat pelo `Taskbeat-Setup`: o instalador registra
o programa que a extensão chama (Native Messaging) e grava a pasta da extensão
junto do app (`%LOCALAPPDATA%\Programs\Taskbeat\extension`, na pasta padrão). A
pasta também sai em `.zip` em cada release.

1. No Taskbeat, abra **Configurações › Apps de música › Extensão do navegador** e
   clique em **Instalar extensão**: abre a pasta da extensão e esta página.
2. No navegador, abra `brave://extensions` (ou `chrome://extensions`,
   `edge://extensions`) e ligue o **Modo do desenvolvedor**.
3. Clique em **Carregar sem compactação** e escolha a pasta `extension`.

A linha das configurações mostra quando ela conecta. As abas do YouTube Music já
abertas recebem a extensão na hora. Quando o Taskbeat atualiza, a pasta é posta em
dia junto: recarregue a extensão no navegador (o botão ↻ dela) para usar a nova.

Carregada assim, o ID é fixo (`abofmfclmpmmbgecemcglhcklnmcapfh`, pela chave
pública do `manifest.json`), e só ele (e, depois de publicada, o da loja) pode abrir
o programa do Taskbeat. Publicar: [`store/PUBLICAR.md`](store/PUBLICAR.md).

## Como funciona

```text
aba do YouTube Music (content.js)
  └─ porta ─ service worker (sw.js)
       └─ Native Messaging (stdin/stdout) ─ Taskbeat.exe aberto pelo navegador
            └─ pipe nomeado (só do seu usuário) ─ Taskbeat em execução
```

- **Leitura:** o elemento `ytmusic-player-bar` tem o atributo `shuffle-on` quando o
  aleatório está ligado e `repeat-mode` (`NONE`, `ALL`, `ONE`) para o repetir. Um
  `MutationObserver` avisa quando eles mudam; título e artista vêm do
  `navigator.mediaSession` (o mesmo que o Windows recebe). Nada de consulta
  periódica: a extensão só envia quando algo muda.
- **Comandos:** o Taskbeat manda `shuffle` (alterna) ou `repeat` (próximo do ciclo
  desativado → tudo → uma), e a extensão aperta o botão da página.
- **Várias abas:** cada aba manda o próprio estado; o Taskbeat usa a que toca a
  música que o Windows mostra (mesmo título e mesmo navegador).
- **Leve:** a conexão com o Taskbeat só fica aberta enquanto houver uma aba do
  YouTube Music; sem abas, o navegador põe a extensão para dormir. Sem o Taskbeat
  aberto, o programa aberto pelo navegador espera um aviso dele, parado.
- **Sem rede:** nenhuma porta é aberta; tudo passa pelo navegador e por um pipe
  local do Windows.

### Mensagens

Formato do Native Messaging: 4 bytes com o tamanho (ordem do processador) e o JSON.

| De | Mensagem |
|---|---|
| extensão | `{"t":"hello","browser":"Brave","version":"1.1.0"}` |
| extensão | `{"t":"player","id":"123","title":"…","artist":"…","album":"…","playing":true,"shuffle":false,"repeat":"ALL","art":"https://lh3.googleusercontent.com/…=w1200-h1200"}` |
| extensão | `{"t":"gone","id":"123"}` |
| programa | `{"t":"host"}` (abriu) e `{"t":"sync"}` (o Taskbeat conectou: mande tudo) |
| Taskbeat | `{"t":"cmd","id":"123","cmd":"shuffle"}` ou `"repeat"` |

`shuffle` e `repeat` chegam como `null` se a página não tiver mais o atributo do
repetir (o YouTube mudou a página): aí o Taskbeat volta ao caminho antigo. `art`
(1.1+) é a maior capa do `navigator.mediaSession`, pedida em 1200 px; o Taskbeat
só baixa endereços HTTPS dos servidores de imagem do Google e do YouTube.

## Créditos

A ideia de ler e comandar o YouTube Music pela página veio do
[WebNowPlaying Redux](https://github.com/keifufu/WebNowPlaying-Redux) (o atributo
`repeat-mode` e o clique nos botões) e do
[YouTube Music Desktop App](https://github.com/th-ch/youtube-music) (o atributo
`shuffle-on`), ambos MIT. Veja `THIRD-PARTY-NOTICES.md`.
