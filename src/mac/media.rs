//! O que está tocando no Mac.
//!
//! Fonte principal: o "Tocando Agora" do macOS (MediaRemote), que vale para
//! qualquer player — Spotify, Música, navegadores (YouTube, YouTube Music),
//! Podcasts, VLC… Desde o macOS 15.4 só processos da Apple podem lê-lo, então
//! usamos o `mediaremote-adapter` (BSD-3): o `/usr/bin/perl` do sistema carrega
//! um framework pequeno que vem dentro do app e manda cada mudança como uma
//! linha JSON. Os comandos (play, próximo, posição…) usam o mesmo adaptador.
//!
//! Reserva: sem o adaptador (ou se o macOS bloqueá-lo), Spotify e Música são
//! lidos por AppleScript (pede a permissão "Automação" na primeira vez).
//!
//! Aleatório e repetir: o Spotify não os oferece pelo Tocando Agora (só
//! tocar, pausar, pular e posição). Nele (e no Música, se faltar), esses dois
//! vão por AppleScript: o estado é lido quando o card abre e depois de cada
//! clique — nada fica consultando o player em segundo plano.
//!
//! Também detecta troca *automática* de faixa (a anterior acabou sozinha) para
//! o "A seguir", com as mesmas regras do Windows.

use super::gfx;
use super::system::{self, log};
use crate::render::Image;
use crate::shared::{MediaCmd, MediaInfo, Shared};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Lado máximo da capa guardada. O macOS não deixa pedir um tamanho: a capa
/// vem como o player a publicou (Spotify e a maioria dos sites: 300 a 640 px;
/// alguns, 1000 px ou mais). Até este lado ela fica no tamanho original; acima,
/// é reduzida uma vez por média de área (`gfx::cover_square`). A maior capa na
/// tela é a do card aberto: 72 pt, 144 px em Retina; 768 é mais de 5× isso,
/// com folga para prévias e telas maiores, e ocupa até ~3 MB (com as reduções
/// pela metade que o desenho cria).
const ART_SIZE: usize = 768;
/// A faixa anterior precisa estar a menos disso do fim para a troca contar como automática.
const NEAR_END: f64 = 10.0;
/// Faixas mais curtas que isso (ou sem linha do tempo) não dão para julgar.
const MIN_TRACK: f64 = 20.0;
/// Um comando nosso de anterior/próximo marca a troca seguinte como manual.
const MANUAL_WINDOW: Duration = Duration::from_secs(4);
/// A faixa vista tocando perto do fim há menos que isso ainda conta como "acabou sozinha".
const ENDED_WINDOW: Duration = Duration::from_secs(15);
/// Quanto a posição pode ser estimada além da última leitura tocando.
const ESTIMATE_AHEAD: Duration = Duration::from_secs(3);
/// Depois da troca de faixa, espera a capa nova por até este tempo.
const ART_SETTLE: Duration = Duration::from_secs(6);
/// Consulta por AppleScript (só na reserva, sem o adaptador).
const SCRIPT_POLL: Duration = Duration::from_secs(3);
/// Sem nenhuma resposta do adaptador por este tempo com um player aberto: usa a reserva.
const ADAPTER_SILENT: Duration = Duration::from_secs(4);

enum Ev {
    Cmd(MediaCmd),
    /// Linha do adaptador: (conteúdo, é diferença?).
    Data(Map<String, Value>, bool),
    /// Aleatório/repetir lidos por AppleScript (app, resultado).
    Modes(&'static str, script::ModesRead),
    AdapterGone,
}

/// Apps cujo AppleScript o macOS bloqueou (Automação negada): bit 1 = Spotify, 2 = Música.
static DENIED: AtomicU8 = AtomicU8::new(0);

fn denied_bit(app: &str) -> u8 {
    if app == "Spotify" { 1 } else { 2 }
}

/// O macOS não deixa o Taskbeat controlar este app (Ajustes › Privacidade › Automação)?
pub fn automation_denied(bundle: &str) -> bool {
    script::app_for(bundle).is_some_and(|app| DENIED.load(Ordering::Relaxed) & denied_bit(app) != 0)
}

/// PID do adaptador em execução (para encerrá-lo junto com o app).
static ADAPTER_PID: AtomicU32 = AtomicU32::new(0);

/// Encerra o processo do adaptador (chamado ao sair do app).
pub fn shutdown() {
    let pid = ADAPTER_PID.swap(0, Ordering::Relaxed);
    if pid != 0 {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        unsafe {
            kill(pid as i32, 15);
        }
    }
}

struct Adapter {
    script: PathBuf,
    framework: PathBuf,
}

impl Adapter {
    /// Dentro do pacote: `Contents/Resources/mediaremote-adapter.pl` e
    /// `Contents/Frameworks/MediaRemoteAdapter.framework`. Para testar fora do
    /// pacote: `TASKBEAT_ADAPTER_DIR` com os dois na mesma pasta.
    fn locate() -> Option<Self> {
        let (script, framework) = match std::env::var_os("TASKBEAT_ADAPTER_DIR") {
            Some(d) => {
                let d = PathBuf::from(d);
                (d.join("mediaremote-adapter.pl"), d.join("MediaRemoteAdapter.framework"))
            }
            None => {
                let app = system::app_bundle()?;
                (app.join("Contents/Resources/mediaremote-adapter.pl"), app.join("Contents/Frameworks/MediaRemoteAdapter.framework"))
            }
        };
        (script.exists() && framework.exists()).then_some(Self { script, framework })
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new("/usr/bin/perl");
        c.arg(&self.script).arg(&self.framework).args(args);
        c
    }

    /// Roda um comando curto (send/seek/shuffle/repeat) sem bloquear.
    fn run(&self, args: &[&str]) {
        match self.command(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => log::warn(&format!("adaptador: {e}")),
        }
    }

    /// Inicia o `stream` e encaminha cada linha para `tx`.
    fn stream(&self, tx: Sender<Ev>) -> bool {
        let child = self.command(&["stream", "--micros", "--debounce=40"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                log::warn(&format!("adaptador não iniciou: {e}"));
                return false;
            }
        };
        ADAPTER_PID.store(child.id(), Ordering::Relaxed);
        log::info(&format!("adaptador do Tocando Agora iniciado (pid {})", child.id()));
        let out = child.stdout.take().unwrap();
        if let Some(err) = child.stderr.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    log::warn(&format!("adaptador: {line}"));
                }
            });
        }
        std::thread::Builder::new()
            .name("media-adapter".into())
            .spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    let Ok(Value::Object(mut v)) = serde_json::from_str::<Value>(&line) else { continue };
                    if v.get("type").and_then(Value::as_str) != Some("data") {
                        continue;
                    }
                    let diff = v.get("diff").and_then(Value::as_bool).unwrap_or(false);
                    let payload = match v.remove("payload") {
                        Some(Value::Object(p)) => p,
                        _ => Map::new(),
                    };
                    if tx.send(Ev::Data(payload, diff)).is_err() {
                        break;
                    }
                }
                let status = child.wait();
                log::warn(&format!("adaptador terminou: {status:?}"));
                ADAPTER_PID.store(0, Ordering::Relaxed);
                let _ = tx.send(Ev::AdapterGone);
            })
            .is_ok()
    }
}

pub fn spawn(shared: Arc<Shared>, notify: fn()) {
    let (cmd_tx, cmd_rx) = channel::<MediaCmd>();
    *shared.media_tx.lock().unwrap() = Some(cmd_tx);
    let (tx, rx) = channel::<Ev>();
    let fwd = tx.clone();
    std::thread::spawn(move || {
        while let Ok(c) = cmd_rx.recv() {
            if fwd.send(Ev::Cmd(c)).is_err() {
                break;
            }
        }
    });
    std::thread::Builder::new()
        .name("media-nowplaying".into())
        .spawn(move || {
            system::background_thread();
            let adapter = Adapter::locate();
            if adapter.is_none() {
                log::warn("adaptador do Tocando Agora não encontrado: usando AppleScript (Spotify e Música)");
            }
            let mut st = State::new(shared.clone(), notify, tx.clone());
            let mut live = adapter.as_ref().is_some_and(|a| a.stream(tx.clone()));
            let mut restarts = 0u32;
            let mut last_script = Instant::now() - SCRIPT_POLL;
            let started = Instant::now();
            while shared.is_running() {
                match rx.recv_timeout(SCRIPT_POLL) {
                    Ok(Ev::Data(p, diff)) => {
                        st.adapter_data(p, diff);
                    }
                    Ok(Ev::Cmd(c)) => st.command(c, if live { adapter.as_ref() } else { None }),
                    Ok(Ev::Modes(app, r)) => st.modes_read(app, r),
                    Ok(Ev::AdapterGone) => {
                        live = false;
                        // Reinicia algumas vezes (ex.: o perl caiu); depois fica na reserva.
                        if restarts < 3 && shared.is_running() {
                            restarts += 1;
                            std::thread::sleep(Duration::from_secs(2 * restarts as u64));
                            live = adapter.as_ref().is_some_and(|a| a.stream(tx.clone()));
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                // Reserva por AppleScript: sem adaptador, ou adaptador mudo com um player aberto.
                let silent = !live || (st.adapter_empty() && started.elapsed() > ADAPTER_SILENT);
                if silent && last_script.elapsed() >= SCRIPT_POLL {
                    last_script = Instant::now();
                    if let Some(info) = script::read() {
                        st.script_data(info);
                    } else if !live {
                        st.publish_empty();
                    }
                }
            }
            shutdown();
        })
        .expect("thread de mídia");
}

/// Faixa atual e como ela terminou (troca automática × manual).
#[derive(Default)]
struct Tracks {
    key: (String, String, String),
    last_playing: Option<(f64, f64, Instant)>,
}

impl Tracks {
    fn observe(&mut self, key: (String, String, String), playing: bool, pos: f64, dur: f64, now: Instant, manual: bool) -> (bool, bool) {
        if key.1.is_empty() && key.0 == self.key.0 && !self.key.1.is_empty() {
            return (false, false);
        }
        let mut changed = false;
        let mut auto = false;
        if key != self.key {
            let same_app = self.key.0 == key.0 && !self.key.1.is_empty();
            let ended = self.last_playing.is_some_and(|(p, d, at)| {
                let since = now.saturating_duration_since(at);
                since < ENDED_WINDOW && d >= MIN_TRACK && p + since.min(ESTIMATE_AHEAD).as_secs_f64() >= d - NEAR_END
            });
            changed = true;
            auto = same_app && ended && !manual;
            self.key = key;
            self.last_playing = None;
        }
        if playing {
            self.last_playing = Some((pos, dur, now));
        }
        (changed, auto)
    }
}

struct State {
    shared: Arc<Shared>,
    notify: fn(),
    payload: Map<String, Value>,
    tracks: Tracks,
    last_manual: Option<Instant>,
    auto_seq: u64,
    /// Capa decodificada e o texto base64 de onde veio.
    art: Option<Arc<Image>>,
    art_src: String,
    /// Troca de faixa: quando foi e qual capa havia (espera a nova chegar).
    changed_at: Option<Instant>,
    art_at_change: String,
    apps: HashMap<String, (String, Option<Arc<Image>>)>,
    /// Origem da última informação publicada pelo AppleScript (para os comandos).
    script_app: Option<&'static str>,
    /// Para as leituras de aleatório/repetir feitas em outra thread.
    tx: Sender<Ev>,
    /// App tocando pelo adaptador cujo aleatório/repetir vão por AppleScript.
    scripted: Option<&'static str>,
    /// Último aleatório/repetir conhecido desse app.
    modes: Option<(&'static str, script::Modes)>,
}

fn epoch_micros() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as f64).unwrap_or(0.0)
}

fn num(p: &Map<String, Value>, k: &str) -> Option<f64> {
    p.get(k).and_then(Value::as_f64)
}

fn text(p: &Map<String, Value>, k: &str) -> String {
    p.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

impl State {
    fn new(shared: Arc<Shared>, notify: fn(), tx: Sender<Ev>) -> Self {
        Self {
            tx,
            scripted: None,
            modes: None,
            shared,
            notify,
            payload: Map::new(),
            tracks: Tracks::default(),
            last_manual: None,
            auto_seq: 0,
            art: None,
            art_src: String::new(),
            changed_at: None,
            art_at_change: String::new(),
            apps: HashMap::new(),
            script_app: None,
        }
    }

    fn adapter_empty(&self) -> bool {
        self.payload.get("title").and_then(Value::as_str).is_none_or(str::is_empty)
    }

    fn adapter_data(&mut self, p: Map<String, Value>, diff: bool) {
        if diff {
            for (k, v) in p {
                if v.is_null() {
                    self.payload.remove(&k);
                } else {
                    self.payload.insert(k, v);
                }
            }
        } else {
            self.payload = p;
        }
        self.adapter_publish();
    }

    /// Publica o que o adaptador informou por último.
    fn adapter_publish(&mut self) {
        if self.adapter_empty() {
            self.scripted = None;
            self.publish_empty();
            return;
        }
        self.script_app = None;
        let p = &self.payload;
        let playing = p.get("playing").and_then(Value::as_bool).unwrap_or(false);
        let dur = num(p, "durationMicros").map(|v| v / 1e6).or_else(|| num(p, "duration")).unwrap_or(0.0).max(0.0);
        let elapsed = num(p, "elapsedTimeMicros").map(|v| v / 1e6).or_else(|| num(p, "elapsedTime")).unwrap_or(0.0);
        let rate = num(p, "playbackRate").unwrap_or(1.0);
        let mut pos = elapsed;
        if let (true, Some(ts)) = (playing, num(p, "timestampEpochMicros")) {
            pos += ((epoch_micros() - ts) / 1e6).max(0.0) * if rate > 0.0 { rate } else { 1.0 };
        }
        if dur > 0.0 {
            pos = pos.clamp(0.0, dur);
        }
        let bundle = {
            let parent = text(p, "parentApplicationBundleIdentifier");
            if parent.is_empty() { text(p, "bundleIdentifier") } else { parent }
        };
        // Comandos aceitos pelo player (quando ele informa).
        let cmds: Vec<(i64, String, bool)> = p
            .get("supportedCommands")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        let id = c.get("id")?.as_i64()?;
                        let name = c.get("name").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
                        let enabled = c.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                        Some((id, name, enabled))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let has = |ids: &[i64], words: &[&str], default: bool| {
            if cmds.is_empty() {
                return default;
            }
            cmds.iter().any(|(id, name, en)| *en && (ids.contains(id) || words.iter().any(|w| name.contains(w))))
        };
        let shuffle_mode = num(p, "shuffleMode").unwrap_or(1.0) as i64;
        let repeat_mode = num(p, "repeatMode").unwrap_or(1.0) as i64;
        let mut info = MediaInfo {
            playing,
            aumid: bundle.clone(),
            title: text(p, "title"),
            artist: text(p, "artist"),
            album: text(p, "album"),
            can_prev: has(&[5], &["previous"], true),
            can_next: has(&[4], &["next"], true),
            can_toggle: has(&[0, 1, 2], &["toggleplaypause", "play"], true),
            can_shuffle: has(&[], &["shuffle"], p.contains_key("shuffleMode")),
            can_repeat: has(&[], &["repeat"], p.contains_key("repeatMode")),
            shuffle: shuffle_mode > 1,
            repeat: match repeat_mode {
                2 => 1,
                3 => 2,
                _ => 0,
            },
            pos,
            dur,
            pos_at: Some(Instant::now()),
            can_seek: dur > 0.0 && has(&[24], &["position", "seek"], true),
            ..Default::default()
        };
        // Sem aleatório/repetir no Tocando Agora (Spotify): por AppleScript.
        self.scripted = None;
        if !(info.can_shuffle && info.can_repeat) {
            if let Some(app) = script::app_for(&bundle) {
                self.scripted = Some(app);
                let allowed = DENIED.load(Ordering::Relaxed) & denied_bit(app) == 0;
                info.can_shuffle = allowed;
                info.can_repeat = allowed;
                if let Some((_, md)) = self.modes.filter(|(a, _)| *a == app) {
                    info.shuffle = md.shuffle;
                    info.repeat = md.repeat;
                }
            }
        }
        // Capa (base64): só decodifica quando muda.
        let old_src = self.art_src.clone();
        let src = p.get("artworkData").and_then(Value::as_str).unwrap_or("");
        if src != self.art_src {
            self.art_src = src.to_string();
            self.art = if src.is_empty() { None } else { gfx::base64_decode(src).and_then(|b| gfx::decode_square(&b, ART_SIZE)).map(Arc::new) };
        }
        info.art = self.art.clone();
        self.finish(info, bundle, old_src);
    }

    fn script_data(&mut self, s: script::Info) {
        self.script_app = Some(s.app);
        let info = MediaInfo {
            playing: s.playing,
            aumid: s.bundle.to_string(),
            title: s.title,
            artist: s.artist,
            album: s.album,
            can_prev: true,
            can_next: true,
            can_toggle: true,
            can_shuffle: true,
            can_repeat: true,
            shuffle: s.shuffle,
            repeat: s.repeat,
            pos: s.pos,
            dur: s.dur,
            pos_at: Some(Instant::now()),
            can_seek: s.dur > 0.0,
            ..Default::default()
        };
        let old_src = self.art_src.clone();
        if s.art_url != self.art_src {
            self.art_src = s.art_url.clone();
            self.art = script::fetch_art(&s.art_url).and_then(|b| gfx::decode_square(&b, ART_SIZE)).map(Arc::new);
        }
        let mut info = info;
        info.art = self.art.clone();
        self.finish(info, s.bundle.to_string(), old_src);
    }

    /// Troca de faixa, app de origem e publicação.
    /// `old_art`: a capa que havia antes desta leitura (a da faixa anterior, se trocou).
    fn finish(&mut self, mut info: MediaInfo, bundle: String, old_art: String) {
        let now = Instant::now();
        let manual = self.last_manual.is_some_and(|t| t.elapsed() < MANUAL_WINDOW);
        let key = (bundle.clone(), info.title.clone(), info.artist.clone());
        let (changed, auto) = self.tracks.observe(key, info.playing, info.pos, info.dur, now, manual);
        if changed {
            self.changed_at = Some(now);
            self.art_at_change = old_art;
            if auto {
                self.auto_seq += 1;
                log::info(&format!("troca automática → \"A seguir\": {}", info.title));
            }
        }
        info.auto_change_seq = self.auto_seq;
        info.art_pending = self.changed_at.is_some_and(|t| now - t < ART_SETTLE) && !self.art_src.is_empty() && self.art_src == self.art_at_change;
        let (app, icon) = self.app(&bundle);
        info.app = app;
        info.icon = icon;
        self.publish(info);
    }

    fn app(&mut self, bundle: &str) -> (String, Option<Arc<Image>>) {
        if bundle.is_empty() {
            return (String::new(), None);
        }
        if let Some(a) = self.apps.get(bundle) {
            return a.clone();
        }
        let a = app_info(bundle);
        self.apps.insert(bundle.to_string(), a.clone());
        a
    }

    fn publish_empty(&mut self) {
        self.publish(MediaInfo { auto_change_seq: self.auto_seq, ..Default::default() });
    }

    fn publish(&mut self, info: MediaInfo) {
        let mut m = self.shared.media.lock().unwrap();
        // Posição: republica quando pula (o usuário arrastou o progresso no player).
        let jumped = (m.position_now() - info.position_now()).abs() > 1.5;
        if *m != info || jumped || m.playing != info.playing {
            log::info(&format!(
                "mídia: \"{}\" — {} [{} / {}] tocando={} {:.0}/{:.0}s capa={}",
                info.title,
                info.artist,
                info.app,
                info.aumid,
                info.playing,
                info.position_now(),
                info.dur,
                info.art.as_ref().map(|a| format!("{}x{}", a.w, a.h)).unwrap_or_else(|| "não".into())
            ));
            *m = info;
            drop(m);
            self.shared.media_version.fetch_add(1, Ordering::Relaxed);
            (self.notify)();
        }
    }

    fn command(&mut self, c: MediaCmd, adapter: Option<&Adapter>) {
        if matches!(c, MediaCmd::Next | MediaCmd::Previous) {
            self.last_manual = Some(Instant::now());
        }
        // Aleatório/repetir por AppleScript (o player não os oferece no Tocando Agora).
        if let (Some(app), None) = (self.scripted, self.script_app) {
            match c {
                // O card abriu: lê o estado atual (pede a permissão na primeira vez).
                MediaCmd::Refresh => {
                    self.read_modes(app, None);
                    return;
                }
                MediaCmd::Shuffle(_) | MediaCmd::Repeat(_) => {
                    let mut md = self.modes.filter(|(a, _)| *a == app).map(|m| m.1).unwrap_or_default();
                    match c {
                        MediaCmd::Shuffle(on) => md.shuffle = on,
                        MediaCmd::Repeat(r) => md.repeat = script::repeat_for(app, r),
                        _ => {}
                    }
                    // Mostra já o novo estado; a leitura depois do comando confirma.
                    self.modes = Some((app, md));
                    self.adapter_publish();
                    self.read_modes(app, Some(c));
                    return;
                }
                _ => {}
            }
        }
        if let (Some(a), None) = (adapter, self.script_app) {
            match c {
                MediaCmd::PlayPause => a.run(&["send", "2"]),
                MediaCmd::Next => a.run(&["send", "4"]),
                MediaCmd::Previous => a.run(&["send", "5"]),
                MediaCmd::Seek(s) => a.run(&["seek", &((s.max(0.0) * 1e6) as u64).to_string()]),
                MediaCmd::Shuffle(on) => a.run(&["shuffle", if on { "3" } else { "1" }]),
                MediaCmd::Repeat(r) => a.run(&["repeat", ["1", "2", "3"][(r as usize).min(2)]]),
                MediaCmd::Refresh => {}
            }
            return;
        }
        if let Some(app) = self.script_app {
            script::command(app, c);
        }
    }

    /// Em outra thread: aplica `cmd` (se houver) e lê aleatório/repetir do app.
    fn read_modes(&self, app: &'static str, cmd: Option<MediaCmd>) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            if let Some(c) = cmd {
                script::command_now(app, c);
                // O player aplica a mudança um instante depois.
                std::thread::sleep(Duration::from_millis(350));
            }
            let _ = tx.send(Ev::Modes(app, script::read_modes(app)));
        });
    }

    fn modes_read(&mut self, app: &'static str, r: script::ModesRead) {
        let bit = denied_bit(app);
        match r {
            script::ModesRead::Ok(md) => {
                DENIED.fetch_and(!bit, Ordering::Relaxed);
                self.modes = Some((app, md));
            }
            script::ModesRead::Denied => {
                if DENIED.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
                    log::warn(&format!("Automação negada para {app}: aleatório/repetir indisponíveis"));
                }
            }
            script::ModesRead::Failed => return,
        }
        if self.scripted == Some(app) && self.script_app.is_none() {
            self.adapter_publish();
        }
    }
}

/// Nome e ícone do app a partir do identificador do pacote.
fn app_info(bundle: &str) -> (String, Option<Arc<Image>>) {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSFileManager, NSString};
    let ws = NSWorkspace::sharedWorkspace();
    let Some(url) = ws.URLForApplicationWithBundleIdentifier(&NSString::from_str(bundle)) else {
        return (bundle.rsplit('.').next().unwrap_or(bundle).to_string(), None);
    };
    let Some(path) = url.path() else { return (bundle.to_string(), None) };
    let name = NSFileManager::defaultManager().displayNameAtPath(&path).to_string();
    let name = name.strip_suffix(".app").unwrap_or(&name).to_string();
    let icon = ws.iconForFile(&path);
    let mut rect = objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(0.0, 0.0), objc2_foundation::NSSize::new(64.0, 64.0));
    let img = unsafe { icon.CGImageForProposedRect_context_hints(&mut rect, None, None) }.and_then(|cg| gfx::fit(&cg, 64, 64)).map(Arc::new);
    (name, img)
}

/// Reserva: Spotify e Música por AppleScript.
mod script {
    use crate::shared::MediaCmd;
    use std::process::Command;

    pub struct Info {
        pub app: &'static str,
        pub bundle: &'static str,
        pub playing: bool,
        pub title: String,
        pub artist: String,
        pub album: String,
        pub dur: f64,
        pub pos: f64,
        pub art_url: String,
        pub shuffle: bool,
        pub repeat: u8,
    }

    fn osascript(src: &str) -> Option<String> {
        osascript_full(src).ok()
    }

    /// Saída do script ou, se falhar, o erro (stderr).
    fn osascript_full(src: &str) -> Result<String, String> {
        let out = Command::new("/usr/bin/osascript").arg("-e").arg(src).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim_end().to_string())
        }
    }

    /// Nome do app no AppleScript a partir do identificador do pacote.
    pub fn app_for(bundle: &str) -> Option<&'static str> {
        match bundle {
            "com.spotify.client" => Some("Spotify"),
            "com.apple.Music" => Some("Music"),
            _ => None,
        }
    }

    /// Aleatório e repetir de um player (repetir: 0 = não, 1 = música, 2 = lista).
    #[derive(Clone, Copy, Default, PartialEq, Debug)]
    pub struct Modes {
        pub shuffle: bool,
        pub repeat: u8,
    }

    pub enum ModesRead {
        Ok(Modes),
        /// O usuário não deixou o Taskbeat controlar o app (Automação).
        Denied,
        Failed,
    }

    /// O Spotify só liga ou desliga a repetição (da lista): "música" vira "lista".
    pub fn repeat_for(app: &str, r: u8) -> u8 {
        if app == "Spotify" { if r != 0 { 2 } else { 0 } } else { r.min(2) }
    }

    pub fn read_modes(app: &str) -> ModesRead {
        let src = if app == "Spotify" {
            r#"if application "Spotify" is running then tell application "Spotify" to return (shuffling as text) & linefeed & (repeating as text)"#
        } else {
            r#"if application "Music" is running then tell application "Music" to return (shuffle enabled as text) & linefeed & (song repeat as text)"#
        };
        match osascript_full(src) {
            Ok(out) => {
                let mut l = out.lines().map(str::trim);
                match (l.next(), l.next()) {
                    (Some(sh), Some(rep)) => ModesRead::Ok(Modes {
                        shuffle: sh == "true",
                        repeat: match rep {
                            "true" | "all" => 2,
                            "one" => 1,
                            _ => 0,
                        },
                    }),
                    _ => ModesRead::Failed,
                }
            }
            // -1743: "Not authorized to send Apple events".
            Err(e) if e.contains("-1743") => ModesRead::Denied,
            Err(e) => {
                super::log::warn(&format!("AppleScript ({app}): {e}"));
                ModesRead::Failed
            }
        }
    }

    const SPOTIFY: &str = r#"if application "Spotify" is running then
tell application "Spotify"
set t to current track
return (player state as string) & linefeed & (name of t) & linefeed & (artist of t) & linefeed & (album of t) & linefeed & ((duration of t) / 1000) & linefeed & player position & linefeed & (artwork url of t) & linefeed & shuffling & linefeed & repeating
end tell
end if"#;

    const MUSIC: &str = r#"if application "Music" is running then
tell application "Music"
if player state is stopped then return ""
set t to current track
return (player state as string) & linefeed & (name of t) & linefeed & (artist of t) & linefeed & (album of t) & linefeed & (duration of t) & linefeed & player position & linefeed & "" & linefeed & shuffle enabled & linefeed & (song repeat as string)
end tell
end if"#;

    fn parse(app: &'static str, bundle: &'static str, s: &str) -> Option<Info> {
        let l: Vec<&str> = s.split('\n').collect();
        if l.len() < 9 || l[1].is_empty() {
            return None;
        }
        let f = |v: &str| v.trim().replace(',', ".").parse::<f64>().unwrap_or(0.0);
        let repeat = match l[8].trim() {
            "true" | "all" => 2,
            "one" => 1,
            _ => 0,
        };
        Some(Info {
            app,
            bundle,
            playing: l[0].trim() == "playing",
            title: l[1].to_string(),
            artist: l[2].to_string(),
            album: l[3].to_string(),
            dur: f(l[4]),
            pos: f(l[5]),
            art_url: l[6].trim().to_string(),
            shuffle: l[7].trim() == "true",
            repeat,
        })
    }

    /// Lê o player aberto (prefere o que está tocando).
    pub fn read() -> Option<Info> {
        let sp = osascript(SPOTIFY).and_then(|s| parse("Spotify", "com.spotify.client", &s));
        if sp.as_ref().is_some_and(|i| i.playing) {
            return sp;
        }
        let mu = osascript(MUSIC).and_then(|s| parse("Music", "com.apple.Music", &s));
        if mu.as_ref().is_some_and(|i| i.playing) { mu } else { sp.or(mu) }
    }

    pub fn fetch_art(url: &str) -> Option<Vec<u8>> {
        if !url.starts_with("https://") {
            return None;
        }
        let out = Command::new("/usr/bin/curl").args(["-fsL", "--max-time", "5", url]).output().ok()?;
        (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
    }

    pub fn command(app: &str, c: MediaCmd) {
        let app = app.to_string();
        std::thread::spawn(move || command_now(&app, c));
    }

    /// Como `command`, mas espera o script terminar.
    pub fn command_now(app: &str, c: MediaCmd) {
        let spotify = app == "Spotify";
        let what = match c {
            MediaCmd::PlayPause => "playpause".to_string(),
            MediaCmd::Next => "next track".to_string(),
            MediaCmd::Previous => "previous track".to_string(),
            MediaCmd::Seek(s) => format!("set player position to {}", s.max(0.0)),
            MediaCmd::Shuffle(on) if spotify => format!("set shuffling to {on}"),
            MediaCmd::Shuffle(on) => format!("set shuffle enabled to {on}"),
            MediaCmd::Repeat(r) if spotify => format!("set repeating to {}", r != 0),
            MediaCmd::Repeat(r) => format!("set song repeat to {}", ["off", "one", "all"][(r as usize).min(2)]),
            MediaCmd::Refresh => return,
        };
        let src = format!("tell application \"{app}\" to {what}");
        if let Err(e) = osascript_full(&src) {
            super::log::warn(&format!("AppleScript ({app}): {e}"));
        }
    }
}
