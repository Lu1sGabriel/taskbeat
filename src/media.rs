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
use std::sync::atomic::Ordering;
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
/// Lado da capa decodificada: nítida no card mesmo em telas com 150%.
const ART_SIZE: u32 = 192;
/// A faixa anterior precisa estar a menos disso do fim para a troca contar como automática
/// (folga para o intervalo de consulta e para o crossfade do Spotify).
const NEAR_END: f64 = 10.0;
/// Um comando nosso de anterior/próximo marca a troca seguinte como manual.
const MANUAL_WINDOW: Duration = Duration::from_secs(4);

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
                        // Junta rajadas de eventos (o player costuma mandar vários seguidos).
                        while let Ok(MediaCmd::Refresh) = rx.recv_timeout(Duration::from_millis(60)) {}
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
    /// Sessão com eventos assinados: (AUMID, tokens de MediaProperties e PlaybackInfo).
    subscribed: Option<(String, Session, i64, i64)>,
    session: Option<Session>,
    /// Faixa atual: (aumid, título, artista).
    key: (String, String, String),
    was_playing: bool,
    near_end: bool,
    last_manual: Option<Instant>,
    auto_seq: u64,
    art: Option<Arc<Image>>,
    art_tries: u32,
    /// Cache por AUMID: (nome amigável, ícone).
    apps: HashMap<String, (String, Option<Arc<Image>>)>,
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

    fn pick_session(m: &Manager) -> Option<(Session, bool)> {
        let mut sessions: Vec<Session> = Vec::new();
        if let Ok(cur) = m.GetCurrentSession() {
            sessions.push(cur);
        }
        if let Ok(all) = m.GetSessions() {
            sessions.extend(all);
        }
        for s in &sessions {
            if s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()) == Ok(Status::Playing) {
                return Some((s.clone(), true));
            }
        }
        sessions.into_iter().next().map(|s| (s, false))
    }

    fn poll(&mut self, m: &Manager) -> MediaInfo {
        let Some((s, playing)) = Self::pick_session(m) else {
            self.session = None;
            self.key = Default::default();
            self.was_playing = false;
            self.near_end = false;
            return MediaInfo { auto_change_seq: self.auto_seq, ..Default::default() };
        };
        let aumid = s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
        let (app, icon) = self.apps.entry(aumid.clone()).or_insert_with(|| identify_app(&aumid)).clone();
        let props: Option<Props> = s.TryGetMediaPropertiesAsync().and_then(|op| op.join()).ok();
        let title = props.as_ref().and_then(|p| p.Title().ok()).map(|t| t.to_string()).unwrap_or_default();
        let artist = props.as_ref().and_then(|p| p.Artist().ok()).map(|t| t.to_string()).unwrap_or_default();
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

        // Troca de faixa: automática se a anterior estava tocando, perto do fim,
        // era o mesmo app e não houve comando manual nosso.
        let key = (aumid.clone(), title.clone(), artist.clone());
        if key != self.key {
            let same_app = self.key.0 == aumid && !self.key.1.is_empty();
            let manual = self.last_manual.is_some_and(|t| t.elapsed() < MANUAL_WINDOW);
            let auto = same_app && !title.is_empty() && self.was_playing && self.near_end && !manual;
            if auto {
                self.auto_seq += 1;
            }
            crate::audio::log::info(&format!(
                "faixa: \"{title}\" ({}) — {}",
                friendly_or_raw(&aumid),
                if auto { "troca automática → cartão" } else { "troca manual/início → sem cartão" }
            ));
            self.key = key;
            self.art = None;
            self.art_tries = 0;
            self.near_end = false;
        }
        self.was_playing = playing;
        if playing {
            self.near_end = near_end(&s);
        }

        // Capa: alguns players publicam a imagem um pouco depois do título.
        if self.art.is_none() && self.art_tries < 6 {
            self.art_tries += 1;
            self.art = props.as_ref().and_then(load_thumbnail).map(Arc::new);
        }

        self.subscribe(&s, &aumid);
        self.session = Some(s);
        MediaInfo {
            playing,
            aumid,
            app,
            title,
            artist,
            can_prev,
            can_next,
            can_toggle,
            can_shuffle,
            can_repeat,
            shuffle,
            repeat,
            art: self.art.clone(),
            icon,
            auto_change_seq: self.auto_seq,
            pos,
            dur,
            pos_at: Some(Instant::now()),
            can_seek,
        }
    }
}

/// (posição estimada agora, duração) em segundos.
impl State {
    /// Assina "música mudou" e "play/pause mudou" da sessão exibida (troca a
    /// assinatura quando a sessão muda), para atualizar na hora sem polling.
    fn subscribe(&mut self, s: &Session, aumid: &str) {
        if self.subscribed.as_ref().is_some_and(|(a, ..)| a == aumid) {
            return;
        }
        if let Some((_, old, t1, t2)) = self.subscribed.take() {
            let _ = old.RemoveMediaPropertiesChanged(t1);
            let _ = old.RemovePlaybackInfoChanged(t2);
        }
        let Some(tx) = self.tx.clone() else { return };
        let tx2 = tx.clone();
        let t1 = s.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx.send(MediaCmd::Refresh);
            Ok(())
        }));
        let t2 = s.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx2.send(MediaCmd::Refresh);
            Ok(())
        }));
        if let (Ok(t1), Ok(t2)) = (t1, t2) {
            self.subscribed = Some((aumid.to_string(), s.clone(), t1, t2));
        }
    }
}

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

/// Posição estimada agora (o player só atualiza a linha do tempo de vez em quando).
fn near_end(s: &Session) -> bool {
    let Ok(t) = s.GetTimelineProperties() else { return false };
    let (Ok(pos), Ok(end), Ok(start), Ok(upd)) = (t.Position(), t.EndTime(), t.StartTime(), t.LastUpdatedTime()) else {
        return false;
    };
    let length = (end.Duration - start.Duration) as f64 / 1e7;
    if length < 20.0 {
        return false; // sem linha do tempo (ou faixa curtíssima): não dá para saber
    }
    // DateTime: 100 ns desde 1601-01-01 UTC.
    let now_1601 = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0) + 11_644_473_600.0;
    let upd_s = upd.UniversalTime as f64 / 1e7;
    let elapsed = if upd.UniversalTime > 0 { (now_1601 - upd_s).clamp(0.0, 3600.0) } else { 0.0 };
    let est = (pos.Duration - start.Duration) as f64 / 1e7 + elapsed;
    est >= length - NEAR_END
}

/// Decodifica a miniatura (capa) em um quadrado ART_SIZE com recorte central.
fn load_thumbnail(props: &Props) -> Option<Image> {
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.join().ok()?;
    let dec = BitmapDecoder::CreateAsync(&stream).ok()?.join().ok()?;
    let (w, h) = (dec.PixelWidth().ok()?, dec.PixelHeight().ok()?);
    if w == 0 || h == 0 {
        return None;
    }
    let k = ART_SIZE as f64 / w.min(h) as f64;
    let sw = ((w as f64 * k).round() as u32).max(ART_SIZE);
    let sh = ((h as f64 * k).round() as u32).max(ART_SIZE);
    let t = BitmapTransform::new().ok()?;
    t.SetScaledWidth(sw).ok()?;
    t.SetScaledHeight(sh).ok()?;
    t.SetInterpolationMode(BitmapInterpolationMode::Fant).ok()?;
    t.SetBounds(BitmapBounds { X: (sw - ART_SIZE) / 2, Y: (sh - ART_SIZE) / 2, Width: ART_SIZE, Height: ART_SIZE }).ok()?;
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
    let n = (ART_SIZE * ART_SIZE) as usize;
    if data.len() < n * 4 {
        return None;
    }
    let px = data.chunks_exact(4).take(n).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    Some(Image::new(ART_SIZE as usize, ART_SIZE as usize, px))
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
