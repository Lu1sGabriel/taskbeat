//! Informações da mídia tocando via Global System Media Transport Controls
//! (a mesma fonte do painel de mídia do Windows). Funciona com qualquer app que
//! publique sessão de mídia: Spotify, YouTube / YouTube Music no Chrome/Edge/
//! Firefox, Apple Music, Deezer, TIDAL, Amazon Music, Media Player, VLC etc.
//!
//! Também executa anterior/play-pause/próximo e detecta troca *automática* de
//! faixa: a anterior estava a poucos segundos do fim e ninguém pediu para pular.

use crate::appicon;
use crate::render::Image;
use crate::shared::{MediaCmd, MediaInfo, Shared};
use crate::tray::WM_APP_MEDIA;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapBounds, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
    ColorManagementMode, ExifOrientationMode,
};
use windows::Foundation::TypedEventHandler;
use windows::Media::MediaPlaybackAutoRepeatMode;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionPlaybackControls as Controls,
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as Props,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Checagem de segurança; o normal é acordar pelos eventos do Windows.
const POLL: Duration = Duration::from_millis(2000);
/// Lado máximo da capa decodificada: nítida no card vertical e em telas com
/// 200% (capas menores que isso ficam no tamanho original, sem esticar).
const ART_SIZE: u32 = 384;
/// A faixa anterior precisa estar a menos disso do fim para a troca contar como automática
/// (folga para o intervalo de consulta e para o crossfade do Spotify).
const NEAR_END: f64 = 10.0;
/// Faixas mais curtas que isso (ou sem linha do tempo) não dão para julgar.
const MIN_TRACK: f64 = 20.0;
/// Um comando nosso de anterior/próximo marca a troca seguinte como manual.
const MANUAL_WINDOW: Duration = Duration::from_secs(4);
/// Entre uma faixa e outra o player pode ficar "pausado", "trocando" ou sem
/// título enquanto carrega a próxima (o YouTube Music no navegador leva alguns
/// segundos): a faixa vista tocando perto do fim há menos que isso ainda conta
/// como "acabou sozinha".
const ENDED_WINDOW: Duration = Duration::from_secs(15);
/// Quanto a posição pode ser estimada além da última leitura tocando (cobre o
/// intervalo entre consultas sem "inventar" que uma música pausada chegou ao fim).
const ESTIMATE_AHEAD: Duration = Duration::from_secs(3);
/// A sessão mostrada continua escolhida por este tempo depois de parar de
/// tocar, mesmo que outra sessão esteja tocando (troca de faixa).
const STICKY: Duration = Duration::from_secs(8);
/// Sem nenhuma sessão por menos que isso: mantém a última música na tela.
const NO_SESSION_GRACE: Duration = Duration::from_secs(5);
/// Depois da troca de faixa, a capa continua sendo relida enquanto for igual à
/// da faixa anterior (o player publica a imagem nova depois do título).
const ART_SETTLE: Duration = Duration::from_secs(6);

pub fn spawn(shared: Arc<Shared>) {
    let (tx, rx) = channel::<MediaCmd>();
    *shared.media_tx.lock().unwrap() = Some(tx.clone());
    std::thread::Builder::new()
        .name("media-sessions".into())
        .spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            crate::system::background_thread();
            let manager = loop {
                if !shared.is_running() {
                    return;
                }
                match Manager::RequestAsync().and_then(|op| op.join()) {
                    Ok(m) => break m,
                    Err(_) => std::thread::sleep(Duration::from_secs(5)),
                }
            };
            // Eventos do gerenciador: sessão atual mudou / sessões abriram ou fecharam.
            let (t1, t2) = (tx.clone(), tx.clone());
            let _ = manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
                let _ = t1.send(MediaCmd::Refresh);
                Ok(())
            }));
            let _ = manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
                let _ = t2.send(MediaCmd::Refresh);
                Ok(())
            }));

            let mut st = State { tx: Some(tx), ..State::default() };
            while shared.is_running() {
                match rx.recv_timeout(POLL) {
                    Ok(MediaCmd::Refresh) => {
                        // Junta rajadas de eventos (o player costuma mandar vários
                        // seguidos), sem perder um comando que chegue no meio.
                        loop {
                            match rx.recv_timeout(Duration::from_millis(60)) {
                                Ok(MediaCmd::Refresh) => continue,
                                Ok(cmd) => {
                                    st.run_command(cmd);
                                    std::thread::sleep(Duration::from_millis(120));
                                    break;
                                }
                                Err(RecvTimeoutError::Timeout) => break,
                                Err(RecvTimeoutError::Disconnected) => return,
                            }
                        }
                    }
                    Ok(cmd) => {
                        st.run_command(cmd);
                        std::thread::sleep(Duration::from_millis(120));
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                let info = st.poll(&manager);
                let changed = {
                    let mut cur = shared.media.lock().unwrap();
                    let changed = *cur != info;
                    // A posição avança sozinha; só republica quando algo visível mudou
                    // ou para corrigir a posição a cada checagem.
                    *cur = info;
                    changed
                };
                shared.media_version.fetch_add(1, Ordering::Relaxed);
                // A sobreposição de letra relê a posição (corrige a linha atual).
                crate::lyrics_overlay::media_changed();
                if changed {
                    let hwnd = shared.tray_hwnd.load(Ordering::Relaxed);
                    if hwnd != 0 {
                        unsafe {
                            let _ = PostMessageW(Some(HWND(hwnd as _)), WM_APP_MEDIA, WPARAM(0), LPARAM(0));
                        }
                    }
                }
            }
        })
        .expect("thread de mídia");
}

#[derive(Default)]
struct State {
    tx: Option<std::sync::mpsc::Sender<MediaCmd>>,
    /// Sessão com eventos assinados: (AUMID, tokens de MediaProperties, PlaybackInfo e TimelineProperties).
    subscribed: Option<(String, Session, i64, i64, i64)>,
    session: Option<Session>,
    /// O player avisou que título/artista/capa mudaram desde a última leitura.
    props_dirty: Arc<AtomicBool>,
    tracks: Tracks,
    /// AUMID da sessão mostrada e quando ela foi vista tocando.
    shown: String,
    shown_playing_at: Option<Instant>,
    /// Última informação publicada (para segurar alguns segundos sem sessão).
    last_info: Option<(MediaInfo, Instant)>,
    last_manual: Option<Instant>,
    auto_seq: u64,
    art: ArtState,
    /// Cache por AUMID: (nome amigável, ícone).
    apps: HashMap<String, (String, Option<Arc<Image>>)>,
}

/// Faixa atual e como ela terminou: decide se a troca foi automática (a anterior
/// acabou sozinha) ou manual (alguém pulou / escolheu outra).
#[derive(Default)]
struct Tracks {
    /// (aumid, título, artista)
    key: (String, String, String),
    /// Última leitura com a faixa atual tocando.
    last_playing: Option<Timeline>,
}

/// Resultado de uma leitura: a faixa mudou? Foi sozinha?
#[derive(Debug, PartialEq)]
enum Change {
    Same,
    Manual,
    Auto,
}

impl Tracks {
    /// Uma leitura do player. `manual`: houve comando nosso de anterior/próximo há pouco.
    fn observe(&mut self, key: (String, String, String), playing: bool, pos: f64, dur: f64, now: Instant, manual: bool) -> Change {
        // Sem título no meio da troca (carregando a próxima): ignora, sem esquecer
        // como a faixa anterior estava.
        if key.1.is_empty() && key.0 == self.key.0 && !self.key.1.is_empty() {
            return Change::Same;
        }
        let mut change = Change::Same;
        if key != self.key {
            let same_app = self.key.0 == key.0 && !self.key.1.is_empty();
            // A anterior estava tocando perto do fim há pouco tempo.
            let ended = self.last_playing.is_some_and(|tl| {
                let since = now.saturating_duration_since(tl.at);
                since < ENDED_WINDOW && tl.near_end_at(tl.at + since.min(ESTIMATE_AHEAD))
            });
            change = if same_app && ended && !manual { Change::Auto } else { Change::Manual };
            self.key = key;
            self.last_playing = None;
        }
        if playing {
            self.last_playing = Some(Timeline { pos, dur, at: now });
        }
        change
    }
}

/// Posição (s) lida em `at` e duração (s) de uma faixa tocando.
#[derive(Clone, Copy, Debug)]
struct Timeline {
    pos: f64,
    dur: f64,
    at: Instant,
}

impl Timeline {
    /// A faixa está (estimando a partir desta leitura) a menos de `NEAR_END` do fim em `now`?
    fn near_end_at(&self, now: Instant) -> bool {
        self.dur >= MIN_TRACK && self.pos + now.saturating_duration_since(self.at).as_secs_f64() >= self.dur - NEAR_END
    }
}

/// Capa da faixa atual. Na troca de faixa o player costuma publicar o título
/// novo ainda com a imagem antiga e só depois a nova: por isso a capa é relida
/// quando o player avisa que algo mudou e, por alguns segundos depois da troca,
/// enquanto ela for igual à da faixa anterior.
#[derive(Default)]
struct ArtState {
    art: Option<Arc<Image>>,
    /// Assinatura dos pixels da capa atual (0 = sem capa).
    sig: u64,
    /// Assinatura da capa da faixa anterior.
    prev_sig: u64,
    tries: u32,
    changed_at: Option<Instant>,
}

impl ArtState {
    fn track_changed(&mut self, now: Instant) {
        self.prev_sig = self.sig;
        self.sig = 0;
        self.art = None;
        self.tries = 0;
        self.changed_at = Some(now);
    }

    /// Logo depois da troca, com a capa ainda igual à da faixa anterior (ou nenhuma).
    fn settling(&self, now: Instant) -> bool {
        self.changed_at.is_some_and(|t| now.saturating_duration_since(t) < ART_SETTLE) && (self.art.is_none() || self.sig == self.prev_sig)
    }

    fn wants_reload(&self, props_changed: bool, now: Instant) -> bool {
        (self.art.is_none() && self.tries < 6) || props_changed || self.settling(now)
    }

    /// Guarda a imagem lida. Só troca o `Arc` quando os pixels mudaram: quem
    /// desenha compara ponteiros para saber se precisa refazer o desfoque.
    fn offer(&mut self, img: Option<Image>) {
        self.tries += 1;
        if let Some(img) = img {
            let sig = image_sig(&img);
            if sig != self.sig {
                self.sig = sig;
                self.art = Some(Arc::new(img));
            }
        }
    }
}

/// FNV-1a dos pixels (nunca 0, que significa "sem capa").
fn image_sig(img: &Image) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in &img.px {
        h ^= *p as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h.max(1)
}

/// Uma sessão de mídia candidata a aparecer no widget.
#[derive(Clone, Copy)]
struct Cand {
    /// É a que está na tela agora.
    shown: bool,
    playing: bool,
    /// É de um app de música (`launchers::is_music`).
    music: bool,
}

/// Qual candidata mostrar (índice) e se ela está tocando:
/// - quem toca ganha de quem está pausado;
/// - entre as que tocam, um app de música ganha do resto (vídeo, live, aba do
///   navegador), mesmo com outra já na tela: a música começou, o widget vai para ela;
/// - fora isso, fica a que já está na tela enquanto ela toca ou parou há pouco
///   (`sticky`: troca de faixa, carregando a próxima); pular para outra nesse
///   instante fazia a troca parecer manual e o widget piscar;
/// - sem nenhuma tocando, a que está na tela, ou a primeira (a do Windows).
fn choose(c: &[Cand], sticky: bool) -> Option<(usize, bool)> {
    let shown = c.iter().position(|s| s.shown);
    let music_playing = c.iter().position(|s| s.playing && s.music);
    if let Some(i) = shown {
        let s = c[i];
        if s.playing && (s.music || music_playing.is_none()) {
            return Some((i, true));
        }
        if !s.playing && sticky && music_playing.is_none() {
            return Some((i, false));
        }
    }
    if let Some(i) = music_playing.or_else(|| c.iter().position(|s| s.playing)) {
        return Some((i, true));
    }
    shown.or((!c.is_empty()).then_some(0)).map(|i| (i, false))
}

impl State {
    fn run_command(&mut self, cmd: MediaCmd) {
        let Some(s) = &self.session else { return };
        let r = match cmd {
            MediaCmd::Previous => s.TrySkipPreviousAsync(),
            MediaCmd::Next => s.TrySkipNextAsync(),
            MediaCmd::PlayPause => s.TryTogglePlayPauseAsync(),
            MediaCmd::Shuffle(on) => s.TryChangeShuffleActiveAsync(on),
            MediaCmd::Repeat(mode) => s.TryChangeAutoRepeatModeAsync(MediaPlaybackAutoRepeatMode(mode as i32)),
            MediaCmd::Refresh => return,
            MediaCmd::Seek(secs) => {
                let start = s.GetTimelineProperties().and_then(|t| t.StartTime()).map(|t| t.Duration).unwrap_or(0);
                s.TryChangePlaybackPositionAsync(start + (secs * 1e7) as i64)
            }
        };
        if matches!(cmd, MediaCmd::Previous | MediaCmd::Next) {
            self.last_manual = Some(Instant::now());
        }
        if let Ok(op) = r {
            let _ = op.join();
        }
    }

    /// Qual sessão mostrar (regras em `choose`).
    fn pick_session(&self, m: &Manager, now: Instant) -> Option<(Session, bool)> {
        let mut sessions: Vec<Session> = Vec::new();
        if let Ok(cur) = m.GetCurrentSession() {
            sessions.push(cur);
        }
        if let Ok(all) = m.GetSessions() {
            sessions.extend(all);
        }
        let id = |s: &Session| s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
        let playing = |s: &Session| s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()) == Ok(Status::Playing);
        let cands: Vec<Cand> = sessions
            .iter()
            .map(|s| {
                let a = id(s);
                // Nome do app (já conhecido) para os apps de música sem id fixo, como o Shazam.
                let name = self.apps.get(&a).map(|(n, _)| n.as_str()).unwrap_or("");
                Cand { shown: !self.shown.is_empty() && a == self.shown, playing: playing(s), music: crate::launchers::is_music(name, &a) }
            })
            .collect();
        let sticky = self.shown_playing_at.is_some_and(|t| now.saturating_duration_since(t) < STICKY);
        let (i, on) = choose(&cands, sticky)?;
        sessions.into_iter().nth(i).map(|s| (s, on))
    }

    fn poll(&mut self, m: &Manager) -> MediaInfo {
        let now = Instant::now();
        let Some((s, playing)) = self.pick_session(m, now) else {
            self.session = None;
            // A sessão some por um instante entre faixas em alguns players:
            // mantém o que estava na tela (e a faixa anterior, para a próxima
            // ainda contar como troca automática) em vez de sumir com o widget.
            if let Some((info, at)) = &self.last_info {
                if now.saturating_duration_since(*at) < NO_SESSION_GRACE {
                    crate::audio::log::info("sem sessão de mídia (mantendo a última por alguns segundos)");
                    return MediaInfo { playing: false, auto_change_seq: self.auto_seq, ..info.clone() };
                }
            }
            if crate::audio::log::verbose() {
                crate::audio::log::info("sem sessão de mídia");
            }
            return MediaInfo { auto_change_seq: self.auto_seq, ..Default::default() };
        };
        let aumid = s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
        if aumid != self.shown {
            crate::audio::log::info(&format!("sessão: {} → {}", friendly_or_raw(&self.shown), friendly_or_raw(&aumid)));
            self.shown = aumid.clone();
            self.shown_playing_at = None;
        }
        if playing {
            self.shown_playing_at = Some(now);
        }
        let (app, icon) = self.apps.entry(aumid.clone()).or_insert_with(|| identify_app(&aumid)).clone();
        let props: Option<Props> = s.TryGetMediaPropertiesAsync().and_then(|op| op.join()).ok();
        let title = props.as_ref().and_then(|p| p.Title().ok()).map(|t| t.to_string()).unwrap_or_default();
        let artist = props.as_ref().and_then(|p| p.Artist().ok()).map(|t| t.to_string()).unwrap_or_default();
        let album = props.as_ref().and_then(|p| p.AlbumTitle().ok()).map(|t| t.to_string()).unwrap_or_default();
        let pb = s.GetPlaybackInfo().ok();
        let ctl: Option<Controls> = pb.as_ref().and_then(|i| i.Controls().ok());
        let flag = |f: fn(&Controls) -> windows::core::Result<bool>, d: bool| ctl.as_ref().and_then(|c| f(c).ok()).unwrap_or(d);
        let can_prev = flag(|c| c.IsPreviousEnabled(), false);
        let can_next = flag(|c| c.IsNextEnabled(), false);
        let can_toggle = flag(|c| c.IsPlayPauseToggleEnabled(), true);
        let can_shuffle = flag(|c| c.IsShuffleEnabled(), false);
        let can_repeat = flag(|c| c.IsRepeatEnabled(), false);
        let can_seek = flag(|c| c.IsPlaybackPositionEnabled(), false);
        let (pos, dur) = timeline(&s, playing);
        let shuffle = pb.as_ref().and_then(|i| i.IsShuffleActive().ok()).and_then(|r| r.Value().ok()).unwrap_or(false);
        let repeat = pb
            .as_ref()
            .and_then(|i| i.AutoRepeatMode().ok())
            .and_then(|r| r.Value().ok())
            .map(|m| m.0.clamp(0, 2) as u8)
            .unwrap_or(0);

        // Troca de faixa: automática se a anterior estava tocando perto do fim
        // há pouco tempo, era o mesmo app e não houve comando manual nosso.
        let key = (aumid.clone(), title.clone(), artist.clone());
        if crate::audio::log::verbose() {
            crate::audio::log::info(&format!("leitura: \"{title}\" tocando={playing} pos={pos:.1}/{dur:.1}"));
        }
        let manual = self.last_manual.is_some_and(|t| t.elapsed() < MANUAL_WINDOW);
        let change = self.tracks.observe(key, playing, pos, dur, now, manual);
        if change != Change::Same {
            if change == Change::Auto {
                self.auto_seq += 1;
            }
            crate::audio::log::info(&format!(
                "faixa: \"{title}\" ({}) — {}",
                friendly_or_raw(&aumid),
                if change == Change::Auto { "troca automática → cartão" } else { "troca manual/início → sem cartão" }
            ));
            self.art.track_changed(now);
        }

        // Capa: relida quando o player avisa que algo mudou e logo depois da troca.
        let props_changed = self.props_dirty.swap(false, Ordering::Relaxed);
        if self.art.wants_reload(props_changed, now) {
            self.art.offer(props.as_ref().and_then(load_thumbnail));
        }

        self.subscribe(&s, &aumid);
        self.session = Some(s);
        let info = MediaInfo {
            playing,
            aumid,
            app,
            title,
            artist,
            album,
            can_prev,
            can_next,
            can_toggle,
            can_shuffle,
            can_repeat,
            shuffle,
            repeat,
            art: self.art.art.clone(),
            icon,
            auto_change_seq: self.auto_seq,
            art_pending: self.art.settling(now),
            pos,
            dur,
            pos_at: Some(Instant::now()),
            can_seek,
            web_shuffle: false,
            web_repeat: false,
        };
        self.last_info = Some((info.clone(), now));
        info
    }
}

impl State {
    /// Assina "música mudou", "play/pause mudou" e "posição mudou" da sessão
    /// exibida (troca a assinatura quando a sessão muda), para atualizar na hora
    /// sem polling. A posição importa para saber, logo depois de a pessoa pular
    /// para o fim da música, que ela está terminando.
    fn subscribe(&mut self, s: &Session, aumid: &str) {
        if self.subscribed.as_ref().is_some_and(|(a, ..)| a == aumid) {
            return;
        }
        if let Some((_, old, t1, t2, t3)) = self.subscribed.take() {
            let _ = old.RemoveMediaPropertiesChanged(t1);
            let _ = old.RemovePlaybackInfoChanged(t2);
            let _ = old.RemoveTimelinePropertiesChanged(t3);
        }
        let Some(tx) = self.tx.clone() else { return };
        let tx2 = tx.clone();
        let tx3 = tx.clone();
        let t3 = s.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx3.send(MediaCmd::Refresh);
            Ok(())
        }));
        let dirty = self.props_dirty.clone();
        let t1 = s.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
            dirty.store(true, Ordering::Relaxed);
            let _ = tx.send(MediaCmd::Refresh);
            Ok(())
        }));
        let t2 = s.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx2.send(MediaCmd::Refresh);
            Ok(())
        }));
        if let (Ok(t1), Ok(t2), Ok(t3)) = (t1, t2, t3) {
            self.subscribed = Some((aumid.to_string(), s.clone(), t1, t2, t3));
        }
    }
}

/// (posição estimada agora, duração) em segundos. O player só atualiza a linha
/// do tempo de vez em quando; tocando, soma o tempo desde a última atualização
/// (DateTime: 100 ns desde 1601-01-01 UTC).
fn timeline(s: &Session, playing: bool) -> (f64, f64) {
    let Ok(t) = s.GetTimelineProperties() else { return (0.0, 0.0) };
    let (Ok(pos), Ok(end), Ok(start), Ok(upd)) = (t.Position(), t.EndTime(), t.StartTime(), t.LastUpdatedTime()) else {
        return (0.0, 0.0);
    };
    let dur = ((end.Duration - start.Duration) as f64 / 1e7).max(0.0);
    let mut p = (pos.Duration - start.Duration) as f64 / 1e7;
    if playing && upd.UniversalTime > 0 {
        let now_1601 = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0) + 11_644_473_600.0;
        p += (now_1601 - upd.UniversalTime as f64 / 1e7).clamp(0.0, 3600.0);
    }
    (if dur > 0.0 { p.clamp(0.0, dur) } else { p.max(0.0) }, dur)
}

/// Capa da sessão atual em até `max` px (a tela cheia da letra quer mais
/// que os 384 do card), com o título a que ela pertence. Bloqueia: chame
/// numa thread à parte.
pub fn current_cover(max: u32) -> Option<(String, Image)> {
    let manager = Manager::RequestAsync().ok()?.join().ok()?;
    let session = manager.GetCurrentSession().ok()?;
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let title = props.Title().map(|t| t.to_string()).unwrap_or_default();
    Some((title, load_thumbnail_max(&props, max)?))
}

/// Decodifica a miniatura (capa) num quadrado de até ART_SIZE com recorte central.
fn load_thumbnail(props: &Props) -> Option<Image> {
    load_thumbnail_max(props, ART_SIZE)
}

fn load_thumbnail_max(props: &Props, max: u32) -> Option<Image> {
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.join().ok()?;
    let dec = BitmapDecoder::CreateAsync(&stream).ok()?.join().ok()?;
    let (w, h) = (dec.PixelWidth().ok()?, dec.PixelHeight().ok()?);
    if w == 0 || h == 0 {
        return None;
    }
    let side = max.min(w.min(h)).max(64);
    let k = side as f64 / w.min(h) as f64;
    let sw = ((w as f64 * k).round() as u32).max(side);
    let sh = ((h as f64 * k).round() as u32).max(side);
    let t = BitmapTransform::new().ok()?;
    t.SetScaledWidth(sw).ok()?;
    t.SetScaledHeight(sh).ok()?;
    t.SetInterpolationMode(BitmapInterpolationMode::Fant).ok()?;
    t.SetBounds(BitmapBounds { X: (sw - side) / 2, Y: (sh - side) / 2, Width: side, Height: side }).ok()?;
    let data = dec
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Premultiplied,
            &t,
            ExifOrientationMode::RespectExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )
        .ok()?
        .join()
        .ok()?
        .DetachPixelData()
        .ok()?;
    let n = (side * side) as usize;
    if data.len() < n * 4 {
        return None;
    }
    let px = data.chunks_exact(4).take(n).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    Some(Image::new(side as usize, side as usize, px))
}

/// Nome amigável + ícone do app da sessão.
fn identify_app(aumid: &str) -> (String, Option<Arc<Image>>) {
    let known = known_app_name(aumid);
    let (shell_name, shell_icon) = appicon::from_apps_folder(aumid);
    let name = known.or(shell_name).unwrap_or_else(|| fallback_name(aumid));
    let icon = shell_icon.or_else(|| appicon::app_icon(aumid, &name));
    (name, icon.map(Arc::new))
}

/// "5319275A.WhatsAppDesktop_cv1g…!App" -> "WhatsAppDesktop"; "C:\…\foo.exe" -> "foo".
fn fallback_name(aumid: &str) -> String {
    let base = aumid.rsplit(['\\', '/']).next().unwrap_or(aumid);
    let base = base.split('!').next().unwrap_or(base);
    let base = base.split('_').next().unwrap_or(base);
    let base = base.trim_end_matches(".exe").trim_end_matches(".EXE");
    base.rsplit('.').next().unwrap_or(base).to_string()
}

/// Nomes conhecidos a partir do AppUserModelId (ex.: "Spotify.exe", "MSEdge").
pub fn known_app_name(aumid: &str) -> Option<String> {
    let l = aumid.to_lowercase();
    const KNOWN: &[(&str, &str)] = &[
        ("spotify", "Spotify"),
        ("youtube music", "YouTube Music"),
        ("ytmdesktop", "YouTube Music"),
        ("ytmusic", "YouTube Music"),
        ("applemusic", "Apple Music"),
        ("itunes", "iTunes"),
        ("deezer", "Deezer"),
        ("tidal", "TIDAL"),
        ("amazon music", "Amazon Music"),
        ("amazonmusic", "Amazon Music"),
        ("shazam", "Shazam"),
        ("soundcloud", "SoundCloud"),
        ("zunemusic", "Media Player"),
        ("vlc", "VLC"),
        ("foobar2000", "foobar2000"),
        ("aimp", "AIMP"),
        ("musicbee", "MusicBee"),
        ("winamp", "Winamp"),
        ("whatsapp", "WhatsApp"),
        ("msedge", "Microsoft Edge"),
        ("chrome", "Google Chrome"),
        ("firefox", "Firefox"),
        ("308046b0af4a39cb", "Firefox"),
        ("opera", "Opera"),
        ("brave", "Brave"),
        ("vivaldi", "Vivaldi"),
        ("arc.exe", "Arc"),
    ];
    KNOWN.iter().find(|(needle, _)| l.contains(needle)).map(|(_, name)| (*name).to_string())
}

fn friendly_or_raw(aumid: &str) -> String {
    known_app_name(aumid).unwrap_or_else(|| fallback_name(aumid))
}

#[cfg(test)]
mod tests {
    /// A música tem a vez: uma live do YouTube tocando na tela e o YouTube
    /// Music começa a tocar -> o widget vai para o YouTube Music; o contrário
    /// não acontece. Pausado perde para quem toca; dois apps de música
    /// tocando: fica o que já está na tela.
    #[test]
    fn prioridade_dos_apps_de_musica() {
        use super::{Cand, choose};
        let c = |shown, playing, music| Cand { shown, playing, music };
        // Live (na tela, tocando) + YouTube Music tocando.
        assert_eq!(choose(&[c(true, true, false), c(false, true, true)], false), Some((1, true)));
        // YouTube Music na tela tocando + vídeo começa a tocar: fica a música.
        assert_eq!(choose(&[c(false, true, false), c(true, true, true)], false), Some((1, true)));
        // Música pausada na tela, vídeo tocando: vai para o vídeo (depois da troca de faixa).
        assert_eq!(choose(&[c(true, false, true), c(false, true, false)], false), Some((1, true)));
        // ... mas não no meio da troca de faixa da música (sticky).
        assert_eq!(choose(&[c(true, false, true), c(false, true, false)], true), Some((0, false)));
        // Vídeo na tela trocando de faixa e uma música tocando: vai para a música.
        assert_eq!(choose(&[c(true, false, false), c(false, true, true)], true), Some((1, true)));
        // Dois apps de música tocando: fica o que está na tela.
        assert_eq!(choose(&[c(false, true, true), c(true, true, true)], false), Some((1, true)));
        // Nada tocando: a que está na tela; sem ela, a primeira.
        assert_eq!(choose(&[c(false, false, true), c(true, false, false)], false), Some((1, false)));
        assert_eq!(choose(&[c(false, false, false), c(false, false, true)], false), Some((0, false)));
        assert_eq!(choose(&[], false), None);
    }

    #[test]
    fn apps_de_musica() {
        use crate::launchers::is_music;
        assert!(is_music("YouTube Music", "Brave._crx_cinhimbnkkghhklpknlkffjgod"));
        assert!(is_music("Spotify", "Spotify.exe"));
        assert!(is_music("Shazam", "Chrome._crx_abcdef"));
        assert!(is_music("", "AppleInc.AppleMusicWin_nzyj5cx40ttqa!App"));
        assert!(!is_music("Brave", "Brave"));
        assert!(!is_music("VLC media player", "{6D809377-6AF0-444B-8957-A3773F02200E}\\VideoLAN\\VLC\\vlc.exe"));
        assert!(!is_music("Media Player", "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"));
    }
    use super::*;

    fn img(v: u32) -> Image {
        Image::new(4, 4, vec![0xff00_0000 | v; 16])
    }

    #[test]
    fn pulou_para_o_fim_conta_como_perto_do_fim() {
        let t0 = Instant::now();
        // Leitura feita 4 s depois de pular para 2:37 numa música de 2:43.
        let tl = Timeline { pos: 157.0, dur: 163.0, at: t0 };
        assert!(tl.near_end_at(t0));
        // Leitura antiga (1:00), mas a música tocou até o fim desde então.
        let old = Timeline { pos: 60.0, dur: 163.0, at: t0 };
        assert!(!old.near_end_at(t0));
        assert!(old.near_end_at(t0 + Duration::from_secs(100)));
        // Sem linha do tempo: não dá para saber.
        assert!(!Timeline { pos: 0.0, dur: 0.0, at: t0 }.near_end_at(t0 + Duration::from_secs(5)));
    }

    fn k(title: &str) -> (String, String, String) {
        ("Brave".into(), title.into(), "Artista".into())
    }

    /// Música de 3:42; a pessoa pula para 3:38 e dá play.
    fn tocando_ate_o_fim(t: &mut Tracks, t0: Instant) {
        assert_eq!(t.observe(k("A"), false, 40.0, 222.0, t0, false), Change::Manual);
        t.observe(k("A"), false, 218.0, 222.0, t0 + Duration::from_secs(1), false);
        t.observe(k("A"), true, 218.0, 222.0, t0 + Duration::from_secs(2), false);
        t.observe(k("A"), true, 220.0, 222.0, t0 + Duration::from_secs(4), false);
    }

    #[test]
    fn troca_logo_depois_do_fim_e_automatica() {
        let t0 = Instant::now();
        let mut t = Tracks::default();
        tocando_ate_o_fim(&mut t, t0);
        assert_eq!(t.observe(k("B"), true, 0.2, 180.0, t0 + Duration::from_secs(6), false), Change::Auto);
    }

    #[test]
    fn player_carregando_a_proxima_por_alguns_segundos_ainda_e_automatica() {
        let t0 = Instant::now();
        let mut t = Tracks::default();
        tocando_ate_o_fim(&mut t, t0);
        // Fim da faixa: o player fica "pausado" e sem título enquanto carrega.
        assert_eq!(t.observe(k("A"), false, 222.0, 222.0, t0 + Duration::from_secs(6), false), Change::Same);
        assert_eq!(t.observe(k(""), false, 0.0, 0.0, t0 + Duration::from_secs(8), false), Change::Same);
        assert_eq!(t.observe(k(""), false, 0.0, 0.0, t0 + Duration::from_secs(10), false), Change::Same);
        assert_eq!(t.observe(k("B"), false, 0.0, 180.0, t0 + Duration::from_secs(12), false), Change::Auto);
    }

    #[test]
    fn pular_no_meio_ou_pelo_nosso_botao_e_manual() {
        let t0 = Instant::now();
        let mut t = Tracks::default();
        t.observe(k("A"), true, 60.0, 222.0, t0, false);
        assert_eq!(t.observe(k("B"), true, 0.0, 180.0, t0 + Duration::from_secs(2), false), Change::Manual);
        // Perto do fim, mas pelo botão "próximo" do Taskbeat.
        t.observe(k("B"), true, 175.0, 180.0, t0 + Duration::from_secs(4), false);
        assert_eq!(t.observe(k("C"), true, 0.0, 200.0, t0 + Duration::from_secs(5), true), Change::Manual);
    }

    #[test]
    fn pausada_perto_do_fim_e_trocada_bem_depois_e_manual() {
        let t0 = Instant::now();
        let mut t = Tracks::default();
        t.observe(k("A"), true, 214.0, 222.0, t0, false);
        t.observe(k("A"), false, 215.0, 222.0, t0 + Duration::from_secs(1), false);
        assert_eq!(t.observe(k("B"), true, 0.0, 180.0, t0 + Duration::from_secs(40), false), Change::Manual);
    }

    #[test]
    fn capa_antiga_na_troca_e_substituida_pela_nova() {
        let t0 = Instant::now();
        let mut a = ArtState::default();
        a.offer(Some(img(1)));
        let first = a.art.clone().unwrap();

        // Troca de faixa: o player ainda entrega a capa antiga.
        a.track_changed(t0);
        assert!(a.wants_reload(false, t0));
        a.offer(Some(img(1)));
        assert!(a.settling(t0), "capa igual à anterior: ainda esperando a nova");
        assert!(a.wants_reload(false, t0 + Duration::from_secs(1)));

        // A capa nova chega (evento do player).
        a.offer(Some(img(2)));
        assert!(!a.settling(t0));
        assert!(!Arc::ptr_eq(&first, a.art.as_ref().unwrap()));
        assert!(!a.wants_reload(false, t0 + Duration::from_secs(1)));
        assert!(a.wants_reload(true, t0 + Duration::from_secs(1)), "evento do player relê a capa");

        // Mesma imagem de novo: mantém o mesmo Arc (nada para redesenhar).
        let cur = a.art.clone().unwrap();
        a.offer(Some(img(2)));
        assert!(Arc::ptr_eq(&cur, a.art.as_ref().unwrap()));
    }

    #[test]
    fn mesmo_album_para_de_esperar_depois_de_um_tempo() {
        let t0 = Instant::now();
        let mut a = ArtState::default();
        a.offer(Some(img(7)));
        a.track_changed(t0);
        a.offer(Some(img(7)));
        assert!(a.settling(t0 + Duration::from_secs(2)));
        assert!(!a.settling(t0 + ART_SETTLE + Duration::from_millis(10)));
        assert!(!a.wants_reload(false, t0 + ART_SETTLE + Duration::from_millis(10)));
    }
}

#[cfg(test)]
mod probe {
    use super::*;

    /// O que o Windows informa de cada sessão de mídia agora (diagnóstico).
    /// `cargo test --release smtc_probe -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn smtc_probe() {
        let m = Manager::RequestAsync().and_then(|op| op.join()).unwrap();
        for s in m.GetSessions().unwrap() {
            let id = s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
            let c = s.GetPlaybackInfo().and_then(|i| i.Controls()).unwrap();
            let pi = s.GetPlaybackInfo().unwrap();
            println!(
                "{id}: tocando={:?} aleatório={:?}/{:?} repetir={:?}/{:?}",
                pi.PlaybackStatus().map(|v| v.0),
                c.IsShuffleEnabled(),
                pi.IsShuffleActive().ok().and_then(|v| v.Value().ok()),
                c.IsRepeatEnabled(),
                pi.AutoRepeatMode().ok().and_then(|v| v.Value().ok()).map(|v| v.0)
            );
        }
    }
}
