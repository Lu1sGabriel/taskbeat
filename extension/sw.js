// Taskbeat: ponte entre as abas do YouTube Music e o Taskbeat.
//
// Cada aba (ou janela do app instalado) do YouTube Music abre uma porta com
// este service worker e manda o estado dela quando muda. Enquanto houver uma
// aba, fica aberta a conexão com o programa do Taskbeat (Native Messaging: o
// navegador abre `Taskbeat.exe`, que repassa tudo ao Taskbeat em execução).
// Sem abas, a conexão fecha e o navegador pode pôr a extensão para dormir.
//
// Mensagens para o Taskbeat:
//   {t:"hello", browser, version}
//   {t:"player", id, title, artist, album, playing, shuffle, repeat}
//   {t:"gone", id}
// Do Taskbeat (ou do programa que faz a ponte):
//   {t:"host"}              o programa abriu
//   {t:"sync"}              o Taskbeat conectou: mande tudo
//   {t:"cmd", id, cmd}      aperte "shuffle" ou "repeat" na aba `id`
"use strict";

const HOST = "com.taskbeat.ytmusic";
/** Novas tentativas de abrir o programa (s) depois que ele fecha sozinho. */
const RETRY = [1, 3, 10, 30];
/** Sem abas por este tempo (ms): fecha a conexão. */
const IDLE_CLOSE = 5000;

/** id da aba → { port, state } */
const players = new Map();
let native = null;
/** O programa respondeu (true), falhou de vez (false) ou ainda não se sabe (null). */
let nativeOk = null;
let retryIdx = 0;
let retryTimer = 0;
let closeTimer = 0;

function browserName() {
  if (navigator.brave) return "Brave";
  const brands = (navigator.userAgentData && navigator.userAgentData.brands) || [];
  const b = brands.map((x) => x.brand).find((n) => !/Chromium|Not.?A.?Brand/i.test(n));
  return b ? b.replace(/^Google /, "").replace(/^Microsoft /, "") : "Chromium";
}

function post(msg) {
  if (!native) return;
  try {
    native.postMessage(msg);
  } catch (_) {}
}

function broadcast(msg) {
  for (const p of players.values()) {
    try {
      p.port.postMessage(msg);
    } catch (_) {}
  }
}

function setNativeOk(ok) {
  if (nativeOk === ok) return;
  nativeOk = ok;
  broadcast({ t: "native", ok });
}

/** Manda ao Taskbeat tudo que se sabe (ele acabou de conectar). */
function sync() {
  post({ t: "hello", browser: browserName(), version: chrome.runtime.getManifest().version });
  for (const [id, p] of players) if (p.state) post({ t: "player", id, ...p.state });
}

function onNative(msg) {
  if (!msg || typeof msg !== "object") return;
  if (msg.t === "host") {
    retryIdx = 0;
    setNativeOk(true);
  } else if (msg.t === "sync") {
    sync();
  } else if (msg.t === "cmd" && typeof msg.id === "string") {
    const p = players.get(msg.id);
    if (p) p.port.postMessage({ t: "cmd", cmd: msg.cmd });
  }
}

function openNative() {
  clearTimeout(closeTimer);
  closeTimer = 0;
  if (native || retryTimer || !players.size) return;
  let port;
  try {
    port = chrome.runtime.connectNative(HOST);
  } catch (_) {
    setNativeOk(false);
    return;
  }
  native = port;
  port.onMessage.addListener(onNative);
  port.onDisconnect.addListener(() => {
    // Ler o erro evita o aviso de "erro não verificado" no console.
    const err = chrome.runtime.lastError && chrome.runtime.lastError.message;
    if (native === port) native = null;
    if (!players.size) return;
    // Programa não registrado (Taskbeat não instalado) ou bloqueado: não adianta insistir.
    const missing = !!err && /not found|forbidden/i.test(err);
    if (!missing && retryIdx < RETRY.length) {
      if (err) console.info(`[Taskbeat] programa fechou (${err}); tentando de novo`);
      retryTimer = setTimeout(() => {
        retryTimer = 0;
        openNative();
      }, RETRY[retryIdx++] * 1000);
    } else {
      // Sem o Taskbeat instalado: as abas só tentam de novo quando algo mudar.
      setNativeOk(false);
    }
  });
}

function closeNativeSoon() {
  clearTimeout(closeTimer);
  closeTimer = setTimeout(() => {
    closeTimer = 0;
    if (players.size || !native) return;
    const n = native;
    native = null;
    try {
      n.disconnect();
    } catch (_) {}
  }, IDLE_CLOSE);
}

chrome.runtime.onConnect.addListener((port) => {
  const tab = port.sender && port.sender.tab;
  if (port.name !== "ytm" || !tab || tab.id === undefined) {
    port.disconnect();
    return;
  }
  const id = String(tab.id);
  players.set(id, { port, state: null });
  port.onMessage.addListener((msg) => {
    const p = players.get(id);
    if (!p || p.port !== port || !msg || msg.t !== "state" || !msg.state) return;
    p.state = msg.state;
    post({ t: "player", id, ...msg.state });
  });
  port.onDisconnect.addListener(() => {
    const p = players.get(id);
    if (p && p.port === port) {
      players.delete(id);
      post({ t: "gone", id });
    }
    if (!players.size) closeNativeSoon();
  });
  if (nativeOk !== null) port.postMessage({ t: "native", ok: nativeOk });
  if (nativeOk === false) {
    // Nova tentativa (o Taskbeat pode ter sido instalado nesse meio-tempo).
    nativeOk = null;
    retryIdx = 0;
  }
  openNative();
});

// Instalada, atualizada, recarregada ou reativada: as abas já abertas do
// YouTube Music não recebem o script sozinhas (só as carregadas depois). Põe
// nelas agora, uma vez por carga da extensão (a marca no storage.session some
// a cada carga e quando o navegador fecha; o script novo para o antigo).
(async () => {
  try {
    if ((await chrome.storage.session.get("injected")).injected) return;
    await chrome.storage.session.set({ injected: true });
    const tabs = await chrome.tabs.query({ url: "https://music.youtube.com/*" });
    for (const t of tabs) {
      if (t.id === undefined) continue;
      chrome.scripting.executeScript({ target: { tabId: t.id }, files: ["content.js"] }).catch(() => {});
    }
  } catch (_) {}
})();
