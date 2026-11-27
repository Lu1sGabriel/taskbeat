//! O que está tocando no Linux: MPRIS pelo D-Bus.
//!
//! Todo player de desktop publica um objeto `org.mpris.MediaPlayer2.*`
//! (Spotify, VLC, mpv com o plugin, Rhythmbox, Elisa, navegadores…) com a
//! música, a capa (`mpris:artUrl`), a linha do tempo, os comandos e — nos que
//! implementam — aleatório (`Shuffle`) e repetir (`LoopStatus`).
//!
//! A thread fica parada no `poll` até chegar uma mensagem do D-Bus ou um
//! comando: sem música tocando, não roda nada. Tocando, relê a posição a
//! cada poucos segundos (o MPRIS não avisa o avanço normal da posição).
//!
//! Também detecta troca *automática* de faixa (a anterior acabou sozinha) para
//! o "A seguir", com as mesmas regras do Windows e do Mac.

use super::audio::{self, PlayerHint};
use super::gfx;
use super::system::{self, log};
use crate::render::Image;
use crate::shared::{MediaCmd, MediaInfo, Shared};
use dbus::Message;
use dbus::arg::{PropMap, RefArg, Variant};
use dbus::blocking::LocalConnection;
use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
use dbus::message::MessageType;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const CALL: Duration = Duration::from_millis(900);

/// Lado máximo da capa decodificada (nítida no card em telas 2x).
const ART_SIZE: usize = 384;
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
/// Tocando: relê a posição a cada…
const POS_POLL: Duration = Duration::from_secs(5);

enum Ev {
    Cmd(MediaCmd),
    /// Capa carregada: (URL, imagem).
    Art(String, Option<Arc<Image>>),
}

#[derive(Default, Clone)]
struct Player {
    /// Nome no D-Bus (`org.mpris.MediaPlayer2.spotify`) e o dono atual (`:1.42`).
    bus: String,
    owner: String,
    identity: String,
    desktop: String,
    status: String,
    title: String,
    artist: String,
    album: String,
    art_url: String,
    track_id: String,
    length: f64,
    can_next: bool,
    can_prev: bool,
    can_play: bool,
    can_pause: bool,
    can_seek: bool,
    can_control: bool,
    shuffle: Option<bool>,
    loop_status: Option<String>,
    rate: f64,
    pos: f64,
    pos_at: Option<Instant>,
    /// Começou a tocar (para escolher o mais recente).
    started: Option<Instant>,
    /// Última vez visto tocando ou mudando.
    active: Option<Instant>,
    pid: u32,
}

impl Player {
    fn playing(&self) -> bool {
        self.status == "Playing"
    }

    fn position_now(&self) -> f64 {
        let mut p = self.pos;
        if let (true, Some(t)) = (self.playing(), self.pos_at) {
            p += t.elapsed().as_secs_f64() * if self.rate > 0.0 { self.rate } else { 1.0 };
        }
        if self.length > 0.0 { p.clamp(0.0, self.length) } else { p.max(0.0) }
    }

    /// Nomes para achar o fluxo de áudio do player.
    fn hint(&self) -> PlayerHint {
        let mut names: Vec<String> = Vec::new();
        let mut add = |s: &str| {
            let s = s.trim().to_lowercase();
            if s.len() >= 3 && !names.contains(&s) {
                names.push(s);
            }
        };
        add(&self.identity);
        add(&self.desktop);
        if let Some(last) = self.desktop.rsplit('.').next() {
            add(last);
        }
        let short = self.bus.strip_prefix(PREFIX).unwrap_or(&self.bus);
        add(short.split(".instance").next().unwrap_or(short));
        PlayerHint { pid: self.pid, names }
    }
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
    conn: LocalConnection,
    tx: Sender<Ev>,
    players: Vec<Player>,
    /// Player mostrado (nome no D-Bus).
    current: Option<String>,
    tracks: Tracks,
    last_manual: Option<Instant>,
    auto_seq: u64,
    /// Capa da URL `art.0` (carregada ou carregando).
    art: (String, Option<Arc<Image>>),
    loading: Option<String>,
    art_cache: HashMap<String, Option<Arc<Image>>>,
    changed_at: Option<Instant>,
    art_at_change: String,
    last_pos_poll: Instant,
    hint: Option<PlayerHint>,
    /// Acorda esta thread (capa carregada em outra thread).
    wake_w: i32,
}

pub fn spawn(shared: Arc<Shared>) {
    let (cmd_tx, cmd_rx) = channel::<MediaCmd>();
    *shared.media_tx.lock().unwrap() = Some(cmd_tx);
    let (tx, rx) = channel::<Ev>();
    let fwd = tx.clone();
    let (wake_r, wake_w) = pipe();
    // Comandos do app → fila da thread de mídia (e a acorda).
    std::thread::spawn(move || {
        while let Ok(c) = cmd_rx.recv() {
            if fwd.send(Ev::Cmd(c)).is_err() {
                break;
            }
            poke(wake_w);
        }
    });
    std::thread::Builder::new()
        .name("media-mpris".into())
        .spawn(move || {
            system::background_thread();
            // Conexão própria com o descritor exposto (para o `poll`).
            let Ok(mut ch) = dbus::channel::Channel::get_private(dbus::channel::BusType::Session) else {
                log::warn("mídia: sem D-Bus da sessão");
                return;
            };
            ch.set_watch_enabled(true);
            let conn = LocalConnection::from(ch);
            let mut st = State {
                shared: shared.clone(),
                conn,
                tx,
                players: Vec::new(),
                current: None,
                tracks: Tracks::default(),
                last_manual: None,
                auto_seq: 0,
                art: (String::new(), None),
                loading: None,
                art_cache: HashMap::new(),
                changed_at: None,
                art_at_change: String::new(),
                last_pos_poll: Instant::now(),
                hint: None,
                wake_w,
            };
            st.run(rx, wake_r);
        })
        .expect("thread de mídia");
}

fn pipe() -> (i32, i32) {
    let mut fds = [0; 2];
    unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
    (fds[0], fds[1])
}

fn poke(fd: i32) {
    let b = 1u8;
    unsafe { libc::write(fd, (&b as *const u8).cast(), 1) };
}

// ---------------------------------------------------------------- leitura de valores

/// Desembrulha variants (`v` dentro de `v`).
fn inner(v: &dyn RefArg) -> &dyn RefArg {
    if v.signature().starts_with('v') {
        if let Some(x) = v.as_iter().and_then(|mut it| it.next()) {
            return inner(x);
        }
    }
    v
}

fn as_bool(v: &dyn RefArg) -> Option<bool> {
    let v = inner(v);
    v.as_u64().map(|x| x != 0).or_else(|| v.as_i64().map(|x| x != 0))
}

fn as_num(v: &dyn RefArg) -> Option<f64> {
    let v = inner(v);
    v.as_i64().map(|x| x as f64).or_else(|| v.as_u64().map(|x| x as f64)).or_else(|| v.as_f64())
}

fn as_string(v: &dyn RefArg) -> Option<String> {
    inner(v).as_str().map(str::to_string)
}

/// Lista de textos (`as`) ou um texto só.
fn as_strings(v: &dyn RefArg) -> Vec<String> {
    let v = inner(v);
    if let Some(s) = v.as_str() {
        return vec![s.to_string()];
    }
    v.as_iter().map(|it| it.filter_map(|x| inner(x).as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// Pares chave → valor de um dicionário (`a{sv}`).
fn dict(v: &dyn RefArg) -> Vec<(String, &dyn RefArg)> {
    let mut out = Vec::new();
    if let Some(mut it) = inner(v).as_iter() {
        while let (Some(k), Some(val)) = (it.next(), it.next()) {
            if let Some(k) = k.as_str() {
                out.push((k.to_string(), inner(val)));
            }
        }
    }
    out
}

impl Player {
    fn apply(&mut self, iface: &str, props: &PropMap) {
        if iface == ROOT {
            for (k, v) in props {
                match k.as_str() {
                    "Identity" => self.identity = as_string(&*v.0).unwrap_or_default(),
                    "DesktopEntry" => self.desktop = as_string(&*v.0).unwrap_or_default(),
                    _ => {}
                }
            }
            return;
        }
        if iface != PLAYER {
            return;
        }
        for (k, v) in props {
            let v = &*v.0;
            match k.as_str() {
                "PlaybackStatus" => {
                    let s = as_string(v).unwrap_or_default();
                    if s == "Playing" && self.status != "Playing" {
                        self.started = Some(Instant::now());
                    }
                    // A posição avança a partir de agora (ou para).
                    self.pos = self.position_now();
                    self.pos_at = Some(Instant::now());
                    self.status = s;
                }
                "Metadata" => self.metadata(v),
                "CanGoNext" => self.can_next = as_bool(v).unwrap_or(false),
                "CanGoPrevious" => self.can_prev = as_bool(v).unwrap_or(false),
                "CanPlay" => self.can_play = as_bool(v).unwrap_or(false),
                "CanPause" => self.can_pause = as_bool(v).unwrap_or(false),
                "CanSeek" => self.can_seek = as_bool(v).unwrap_or(false),
                "CanControl" => self.can_control = as_bool(v).unwrap_or(false),
                "Shuffle" => self.shuffle = as_bool(v),
                "LoopStatus" => self.loop_status = as_string(v),
                "Rate" => self.rate = as_num(v).unwrap_or(1.0),
                "Position" => {
                    self.pos = as_num(v).unwrap_or(0.0) / 1e6;
                    self.pos_at = Some(Instant::now());
                }
                _ => {}
            }
        }
        self.active = Some(Instant::now());
    }

    fn metadata(&mut self, v: &dyn RefArg) {
        let (mut title, mut artist, mut album, mut art, mut id, mut len) = (String::new(), String::new(), String::new(), String::new(), String::new(), 0.0);
        for (k, val) in dict(v) {
            match k.as_str() {
                "xesam:title" => title = as_string(val).unwrap_or_default(),
                "xesam:artist" => artist = as_strings(val).join(", "),
                "xesam:album" => album = as_string(val).unwrap_or_default(),
                "mpris:artUrl" => art = as_string(val).unwrap_or_default(),
                "mpris:trackid" => id = as_string(val).unwrap_or_default(),
                "mpris:length" => len = as_num(val).unwrap_or(0.0) / 1e6,
                _ => {}
            }
        }
        let changed = title != self.title || artist != self.artist;
        self.title = title.trim().to_string();
        self.artist = artist.trim().to_string();
        self.album = album.trim().to_string();
        self.art_url = art;
        self.track_id = id;
        self.length = len.max(0.0);
        if changed {
            // Faixa nova: começa do zero até a posição ser lida.
            self.pos = 0.0;
            self.pos_at = Some(Instant::now());
        }
    }
}

// ---------------------------------------------------------------- laço

impl State {
    fn run(&mut self, rx: Receiver<Ev>, wake_r: i32) {
        for rule in [
            "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',member='NameOwnerChanged',arg0namespace='org.mpris.MediaPlayer2'",
            "type='signal',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged',path='/org/mpris/MediaPlayer2'",
            "type='signal',interface='org.mpris.MediaPlayer2.Player',member='Seeked',path='/org/mpris/MediaPlayer2'",
        ] {
            if let Err(e) = self.conn.add_match_no_cb(rule) {
                log::warn(&format!("mídia: regra do D-Bus recusada: {e}"));
            }
        }
        self.scan();
        self.publish_current();
        let fd = self.conn.channel().watch().fd;
        while self.shared.is_running() {
            // Tudo que já chegou.
            loop {
                let _ = self.conn.channel().read_write(Some(Duration::ZERO));
                let Some(msg) = self.conn.channel().pop_message() else { break };
                self.message(msg);
            }
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    Ev::Cmd(c) => self.command(c),
                    Ev::Art(url, img) => self.art_loaded(url, img),
                }
            }
            // Tocando: relê a posição de tempos em tempos.
            let playing = self.cur().is_some_and(Player::playing);
            if playing && self.last_pos_poll.elapsed() >= POS_POLL {
                self.poll_position();
                self.publish_current();
            }
            let timeout = if playing { POS_POLL.as_millis() as i32 } else { -1 };
            let mut fds = [libc::pollfd { fd, events: libc::POLLIN, revents: 0 }, libc::pollfd { fd: wake_r, events: libc::POLLIN, revents: 0 }];
            unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout) };
            if fds[1].revents != 0 {
                let mut b = [0u8; 64];
                while unsafe { libc::read(wake_r, b.as_mut_ptr().cast(), b.len()) } > 0 {}
            }
        }
    }

    fn cur(&self) -> Option<&Player> {
        let c = self.current.as_ref()?;
        self.players.iter().find(|p| &p.bus == c)
    }

    /// Lê os players que já estão abertos.
    fn scan(&mut self) {
        let proxy = self.conn.with_proxy("org.freedesktop.DBus", "/org/freedesktop/DBus", CALL);
        let names: Vec<String> = proxy.method_call::<(Vec<String>,), _, _, _>("org.freedesktop.DBus", "ListNames", ()).map(|r| r.0).unwrap_or_default();
        for n in names.into_iter().filter(|n| n.starts_with(PREFIX)) {
            self.add(&n, None);
        }
    }

    fn owner_of(&self, name: &str) -> Option<String> {
        let proxy = self.conn.with_proxy("org.freedesktop.DBus", "/org/freedesktop/DBus", CALL);
        proxy.method_call::<(String,), _, _, _>("org.freedesktop.DBus", "GetNameOwner", (name,)).ok().map(|r| r.0)
    }

    /// Lê tudo de um player novo (ou que trocou de dono).
    fn add(&mut self, bus: &str, owner: Option<String>) {
        // O playerctld só repete os outros players.
        if bus.ends_with(".playerctld") {
            return;
        }
        let Some(owner) = owner.or_else(|| self.owner_of(bus)) else { return };
        let mut p = Player { bus: bus.to_string(), owner: owner.clone(), rate: 1.0, ..Default::default() };
        {
            let proxy = self.conn.with_proxy(bus, PATH, CALL);
            if let Ok(props) = proxy.get_all(ROOT) {
                p.apply(ROOT, &props);
            }
            if let Ok(props) = proxy.get_all(PLAYER) {
                p.apply(PLAYER, &props);
            }
        }
        if p.playing() {
            p.started = Some(Instant::now());
        }
        let dbus = self.conn.with_proxy("org.freedesktop.DBus", "/org/freedesktop/DBus", CALL);
        p.pid = dbus.method_call::<(u32,), _, _, _>("org.freedesktop.DBus", "GetConnectionUnixProcessID", (owner.as_str(),)).map(|r| r.0).unwrap_or(0);
        log::info(&format!("mídia: player {} ({}) {}", p.bus, p.identity, p.status));
        self.players.retain(|x| x.bus != bus);
        self.players.push(p);
    }

    fn message(&mut self, msg: Message) {
        if msg.msg_type() != MessageType::Signal {
            return;
        }
        let member = msg.member().map(|m| m.to_string()).unwrap_or_default();
        let sender = msg.sender().map(|s| s.to_string()).unwrap_or_default();
        match member.as_str() {
            "NameOwnerChanged" => {
                let Ok((name, _old, new)) = msg.read3::<String, String, String>() else { return };
                if !name.starts_with(PREFIX) {
                    return;
                }
                if new.is_empty() {
                    log::info(&format!("mídia: player fechou: {name}"));
                    self.players.retain(|p| p.bus != name);
                } else {
                    self.add(&name, Some(new));
                }
            }
            "PropertiesChanged" => {
                let Ok((iface, changed, invalidated)) = msg.read3::<String, PropMap, Vec<String>>() else { return };
                let idx: Vec<usize> = self.players.iter().enumerate().filter(|(_, p)| p.owner == sender).map(|(i, _)| i).collect();
                for i in idx {
                    self.players[i].apply(&iface, &changed);
                    if !invalidated.is_empty() {
                        // Valores que o player só diz que mudaram: lê de novo.
                        let bus = self.players[i].bus.clone();
                        let props = self.conn.with_proxy(bus.as_str(), PATH, CALL).get_all(&iface);
                        if let Ok(props) = props {
                            self.players[i].apply(&iface, &props);
                        }
                    }
                    if changed.contains_key("Metadata") || changed.contains_key("PlaybackStatus") {
                        let bus = self.players[i].bus.clone();
                        self.read_position(&bus);
                    }
                }
            }
            "Seeked" => {
                let Ok(us) = msg.read1::<i64>() else { return };
                for p in self.players.iter_mut().filter(|p| p.owner == sender) {
                    p.pos = us as f64 / 1e6;
                    p.pos_at = Some(Instant::now());
                }
            }
            _ => return,
        }
        self.publish_current();
    }

    fn read_position(&mut self, bus: &str) {
        let pos: Option<i64> = self.conn.with_proxy(bus, PATH, CALL).get(PLAYER, "Position").ok();
        if let (Some(us), Some(p)) = (pos, self.players.iter_mut().find(|p| p.bus == bus)) {
            p.pos = us as f64 / 1e6;
            p.pos_at = Some(Instant::now());
        }
    }

    fn poll_position(&mut self) {
        self.last_pos_poll = Instant::now();
        if let Some(bus) = self.current.clone() {
            self.read_position(&bus);
        }
    }

    /// Player a mostrar: o que começou a tocar por último; sem nenhum
    /// tocando, o que tocou/mudou por último (pausado antes de parado).
    fn pick(&self) -> Option<String> {
        let playing = self.players.iter().filter(|p| p.playing()).max_by_key(|p| p.started);
        if let Some(p) = playing {
            // Dois tocando: fica no atual, se ele for um deles.
            if let Some(c) = self.cur().filter(|c| c.playing()) {
                if self.players.iter().filter(|p| p.playing()).count() > 1 && c.started.is_some() {
                    return Some(c.bus.clone());
                }
            }
            return Some(p.bus.clone());
        }
        if let Some(c) = self.cur() {
            if !c.title.is_empty() || c.status == "Paused" {
                return Some(c.bus.clone());
            }
        }
        self.players
            .iter()
            .filter(|p| !p.title.is_empty() || p.status == "Paused")
            .max_by_key(|p| (p.status == "Paused", p.active))
            .map(|p| p.bus.clone())
    }

    fn publish_current(&mut self) {
        let pick = self.pick();
        if pick != self.current {
            self.current = pick;
            if let Some(c) = self.cur() {
                log::info(&format!("mídia: mostrando {}", c.bus));
            }
        }
        // Áudio de "só o player".
        let hint = self.cur().map(Player::hint);
        if hint != self.hint {
            self.hint = hint.clone();
            audio::set_player(hint);
        }
        let Some(p) = self.cur().cloned() else {
            self.publish(MediaInfo { auto_change_seq: self.auto_seq, ..Default::default() });
            return;
        };
        // Capa: carrega fora desta thread quando a URL muda.
        let old_art = self.art.0.clone();
        if p.art_url != self.art.0 {
            self.art.0 = p.art_url.clone();
            self.art.1 = None;
            if !p.art_url.is_empty() {
                if let Some(img) = self.art_cache.get(&p.art_url) {
                    self.art.1 = img.clone();
                } else if self.loading.as_deref() != Some(&p.art_url) {
                    self.loading = Some(p.art_url.clone());
                    let (tx, url, wake) = (self.tx.clone(), p.art_url.clone(), self.wake_w);
                    std::thread::spawn(move || {
                        system::background_thread();
                        let img = gfx::fetch(&url).and_then(|b| gfx::decode_square(&b, ART_SIZE)).map(Arc::new);
                        let _ = tx.send(Ev::Art(url, img));
                        poke(wake);
                    });
                }
            }
        }
        let now = Instant::now();
        let playing = p.playing();
        let control = p.can_control;
        let mut info = MediaInfo {
            playing,
            aumid: p.bus.clone(),
            app: p.identity.clone(),
            title: p.title.clone(),
            artist: p.artist.clone(),
            album: p.album.clone(),
            can_prev: control && p.can_prev,
            can_next: control && p.can_next,
            can_toggle: control && (p.can_play || p.can_pause),
            can_shuffle: control && p.shuffle.is_some(),
            can_repeat: control && p.loop_status.is_some(),
            shuffle: p.shuffle.unwrap_or(false),
            repeat: match p.loop_status.as_deref() {
                Some("Track") => 1,
                Some("Playlist") => 2,
                _ => 0,
            },
            pos: p.position_now(),
            dur: p.length,
            pos_at: Some(now),
            can_seek: control && p.can_seek && p.length > 0.0,
            art: self.art.1.clone(),
            ..Default::default()
        };
        let manual = self.last_manual.is_some_and(|t| t.elapsed() < MANUAL_WINDOW);
        let key = (p.bus.clone(), info.title.clone(), info.artist.clone());
        let (changed, auto) = self.tracks.observe(key, playing, info.pos, info.dur, now, manual);
        if changed {
            self.changed_at = Some(now);
            self.art_at_change = old_art;
            if auto {
                self.auto_seq += 1;
                log::info(&format!("troca automática → \"A seguir\": {}", info.title));
            }
        }
        info.auto_change_seq = self.auto_seq;
        let waiting = self.loading.as_deref() == Some(self.art.0.as_str()) && !self.art.0.is_empty();
        info.art_pending = self.changed_at.is_some_and(|t| now - t < ART_SETTLE) && (waiting || (!self.art.0.is_empty() && self.art.0 == self.art_at_change));
        self.publish(info);
    }

    fn art_loaded(&mut self, url: String, img: Option<Arc<Image>>) {
        if self.loading.as_deref() == Some(url.as_str()) {
            self.loading = None;
        }
        if img.is_none() {
            log::warn(&format!("mídia: capa não carregou: {}", url.chars().take(120).collect::<String>()));
        }
        // Guarda poucas (a capa atual e as recentes).
        if self.art_cache.len() > 8 {
            self.art_cache.clear();
        }
        self.art_cache.insert(url.clone(), img.clone());
        if self.art.0 == url {
            self.art.1 = img;
            self.publish_current();
        }
    }

    fn publish(&mut self, info: MediaInfo) {
        let mut m = self.shared.media.lock().unwrap();
        let jumped = (m.position_now() - info.position_now()).abs() > 1.5;
        if *m != info || jumped || m.playing != info.playing {
            log::info(&format!(
                "mídia: \"{}\" — {} [{}] tocando={} {:.0}/{:.0}s capa={} aleatório={} repetir={}",
                info.title,
                info.artist,
                info.app,
                info.playing,
                info.position_now(),
                info.dur,
                info.art.as_ref().map(|a| format!("{}x{}", a.w, a.h)).unwrap_or_else(|| "não".into()),
                if info.can_shuffle { info.shuffle.to_string() } else { "-".into() },
                if info.can_repeat { info.repeat.to_string() } else { "-".into() },
            ));
            *m = info;
            drop(m);
            self.shared.media_version.fetch_add(1, Ordering::Relaxed);
            system::wake();
        }
    }

    fn command(&mut self, c: MediaCmd) {
        if matches!(c, MediaCmd::Next | MediaCmd::Previous) {
            self.last_manual = Some(Instant::now());
        }
        let Some(p) = self.cur().cloned() else { return };
        let proxy = self.conn.with_proxy(p.bus.as_str(), PATH, CALL);
        let r: Result<(), dbus::Error> = match c {
            MediaCmd::PlayPause => proxy.method_call(PLAYER, "PlayPause", ()),
            MediaCmd::Next => proxy.method_call(PLAYER, "Next", ()),
            MediaCmd::Previous => proxy.method_call(PLAYER, "Previous", ()),
            MediaCmd::Seek(s) => {
                let us = (s.max(0.0) * 1e6) as i64;
                match dbus::Path::new(p.track_id.clone()) {
                    Ok(id) if !p.track_id.is_empty() && !p.track_id.ends_with("/NoTrack") => proxy.method_call(PLAYER, "SetPosition", (id, us)),
                    _ => proxy.method_call(PLAYER, "Seek", (us - (p.position_now() * 1e6) as i64,)),
                }
            }
            MediaCmd::Shuffle(on) => proxy.set(PLAYER, "Shuffle", Variant(on)),
            MediaCmd::Repeat(r) => proxy.set(PLAYER, "LoopStatus", Variant(["None", "Track", "Playlist"][(r as usize).min(2)].to_string())),
            MediaCmd::Refresh => {
                if let Ok(props) = proxy.get_all(PLAYER) {
                    if let Some(x) = self.players.iter_mut().find(|x| x.bus == p.bus) {
                        x.apply(PLAYER, &props);
                    }
                }
                self.publish_current();
                Ok(())
            }
        };
        if let Err(e) = r {
            log::warn(&format!("mídia: comando {c:?} falhou em {}: {e}", p.bus));
        }
    }
}
