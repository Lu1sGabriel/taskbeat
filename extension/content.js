// Taskbeat: lê o aleatório e o repetir do YouTube Music e aperta os botões.
//
// Tudo vem de atributos do elemento `ytmusic-player-bar` (o mesmo estado que
// a página usa para desenhar os botões):
//   - `shuffle-on`: existe quando o aleatório está ligado;
//   - `repeat-mode`: "NONE", "ALL" ou "ONE".
// Título e artista vêm do `navigator.mediaSession` (o mesmo que o Windows
// recebe do navegador), e o tocando/pausado do <video> da página.
//
// Nada de consulta periódica: observadores (MutationObserver) e eventos do
// <video> avisam quando algo muda, e só então o estado é lido e enviado (se
// mudou de fato).
"use strict";

(() => {
  // Injetado de novo (extensão recarregada ou atualizada): o anterior para.
  if (typeof globalThis.__taskbeatStop === "function") {
    try {
      globalThis.__taskbeatStop();
    } catch (_) {}
  }

  const REPEAT = ["NONE", "ALL", "ONE"];
  let port = null;
  /** O Taskbeat responde (o programa do navegador abriu)? null: ainda não se sabe. */
  let nativeOk = null;
  let last = "";
  let bar = null;
  let video = null;
  let stopped = false;
  let reconnectTimer = 0;
  const timers = new Set();
  const observers = [];

  const alive = () => !stopped && !!(globalThis.chrome && chrome.runtime && chrome.runtime.id);

  function playerBar() {
    return document.querySelector("ytmusic-player-bar");
  }

  /** Estado atual da página (ou null sem a barra do player). */
  function read() {
    const b = bar && bar.isConnected ? bar : playerBar();
    if (!b) return null;
    const md = navigator.mediaSession ? navigator.mediaSession.metadata : null;
    const text = (sel) => {
      const el = b.querySelector(sel);
      return el ? (el.getAttribute("title") || el.textContent || "").trim() : "";
    };
    const v = video && video.isConnected ? video : document.querySelector("video");
    const repeat = b.getAttribute("repeat-mode");
    // Sem o atributo do repetir a página mudou: o Taskbeat volta ao caminho
    // antigo (null = não se sabe).
    const known = repeat !== null;
    return {
      title: (md && md.title) || text(".title"),
      artist: (md && md.artist) || "",
      album: (md && md.album) || "",
      playing: !!v && !v.paused && !v.ended,
      shuffle: known ? b.hasAttribute("shuffle-on") : null,
      repeat: known ? (REPEAT.includes(repeat) ? repeat : "NONE") : null,
    };
  }

  /** Envia o estado se mudou (ou sempre, com `force`). */
  function send(force) {
    if (!alive()) return stop();
    attach();
    const state = read();
    if (!state) return;
    const key = JSON.stringify(state);
    if (!force && key === last) return;
    if (!port) {
      // Sem conexão (o Taskbeat não estava aberto): reconecta agora, que
      // há algo novo para contar.
      connect();
      return;
    }
    last = key;
    try {
      port.postMessage({ t: "state", state });
    } catch (_) {
      port = null;
    }
  }

  let queued = false;
  /** Lê no fim da tarefa atual (junta várias mudanças seguidas numa leitura). */
  function schedule() {
    if (queued) return;
    queued = true;
    queueMicrotask(() => {
      queued = false;
      send(false);
    });
  }

  /** Lê de novo daqui a `ms` (o título do mediaSession às vezes chega depois). */
  function later(ms) {
    const id = setTimeout(() => {
      timers.delete(id);
      send(false);
    }, ms);
    timers.add(id);
  }

  function onTitleChange() {
    schedule();
    later(400);
    later(1500);
  }

  /** Liga os observadores na barra do player e os eventos no <video>. */
  function attach() {
    const b = playerBar();
    if (b && b !== bar) {
      for (const o of observers.splice(0)) o.disconnect();
      bar = b;
      const attrs = new MutationObserver(schedule);
      attrs.observe(b, { attributes: true, attributeFilter: ["shuffle-on", "repeat-mode"] });
      // Título e artista (só a parte do meio da barra: o tempo da música, que
      // muda a cada segundo, fica fora).
      const middle = b.querySelector(".middle-controls") || b.querySelector(".content-info-wrapper");
      const titles = new MutationObserver(onTitleChange);
      if (middle) titles.observe(middle, { subtree: true, childList: true, characterData: true });
      observers.push(attrs, titles);
    }
    const v = document.querySelector("video");
    if (v && v !== video) {
      if (video) for (const e of VIDEO_EVENTS) video.removeEventListener(e, onVideo);
      video = v;
      for (const e of VIDEO_EVENTS) v.addEventListener(e, onVideo);
    }
  }

  const VIDEO_EVENTS = ["play", "pause", "ended", "emptied", "loadedmetadata"];
  function onVideo(e) {
    if (e.type === "loadedmetadata" || e.type === "emptied") onTitleChange();
    else schedule();
  }

  /** Aperta um botão da barra do player ("shuffle" ou "repeat"). */
  function press(name) {
    const b = playerBar();
    const el = b && (b.querySelector(`.${name}.ytmusic-player-bar`) || b.querySelector(`.${name}`));
    if (!el) return false;
    (el.querySelector("button") || el).click();
    return true;
  }

  function onMessage(msg) {
    if (!msg || typeof msg !== "object") return;
    if (msg.t === "native") {
      nativeOk = !!msg.ok;
    } else if (msg.t === "cmd" && (msg.cmd === "shuffle" || msg.cmd === "repeat")) {
      if (!press(msg.cmd)) console.warn(`[Taskbeat] botão "${msg.cmd}" não achado`);
      // O observador do atributo avisa o estado novo; esta leitura cobre uma
      // página que não mudou nada (o Taskbeat volta ao estado real).
      later(800);
    } else if (msg.t === "sync") {
      send(true);
    }
  }

  function connect() {
    clearTimeout(reconnectTimer);
    if (!alive()) return stop();
    try {
      port = chrome.runtime.connect({ name: "ytm" });
    } catch (_) {
      return stop();
    }
    port.onMessage.addListener(onMessage);
    port.onDisconnect.addListener(() => {
      port = null;
      last = "";
      if (!alive()) return stop();
      // A extensão dormiu ou foi reiniciada. Com o Taskbeat conectado,
      // reconecta logo; sem ele, só na próxima mudança da página.
      if (nativeOk !== false) reconnectTimer = setTimeout(connect, 1000);
    });
    send(true);
  }

  function stop() {
    if (stopped) return;
    stopped = true;
    for (const o of observers.splice(0)) o.disconnect();
    if (video) for (const e of VIDEO_EVENTS) video.removeEventListener(e, onVideo);
    for (const id of timers) clearTimeout(id);
    timers.clear();
    clearTimeout(reconnectTimer);
    try {
      if (port) port.disconnect();
    } catch (_) {}
    port = null;
    if (globalThis.__taskbeatStop === stop) delete globalThis.__taskbeatStop;
  }
  globalThis.__taskbeatStop = stop;

  // A barra do player é montada pela página depois de carregar: espera por ela
  // (só neste começo; depois, só os observadores dela).
  if (playerBar()) {
    connect();
  } else {
    const wait = new MutationObserver(() => {
      if (!playerBar()) return;
      wait.disconnect();
      connect();
    });
    wait.observe(document.documentElement, { childList: true, subtree: true });
    observers.push(wait);
  }
})();
