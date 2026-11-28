//! Letra na tela no Mac: a mesma sobreposição do Windows (`lyrics_view.rs`
//! desenha os layouts, a barrinha e a tela cheia), numa janela flutuante.
//!
//! - Ligada pelo botão da letra no card do notch (ou `lyrics.show` na
//!   configuração); a janela só existe enquanto ela está ligada.
//! - A letra vem do mesmo buscador (pasta do usuário, cache, LRCLIB), com o
//!   `curl` do sistema como transporte.
//! - Arrastar o painel muda o lugar (perto de um dos nove lugares prontos ele
//!   gruda neles); a barrinha (mouse em cima) trava, compacta, abre a tela
//!   cheia e esconde.
//!
//! Tudo roda na thread principal: um temporizador de 10 Hz acompanha a música
//! e troca a linha; as animações seguem o vblank (`Ticker`) só enquanto duram.
//! Ainda não há no Mac: mixer de volume, atalho de teclado e a capa em alta
//! resolução da tela cheia.

use super::compat::Ticker;
use super::volume::Volume;
use super::{gfx, shared, system, target};
use crate::config::Config;
use crate::lyrics::config::{Layout, LyricsConfig, Position};
use crate::lyrics::fetch::{Fetcher, Response};
use crate::lyrics::{self, Line, Lyrics, Track};
use crate::lyrics_view::{self as view, Geo, LHit, LyricFrame, View};
use crate::render::{Canvas, Colors};
use crate::shared::{MediaCmd, MediaInfo};
use crate::text::TextRenderer;
use crate::ui::{self, Hit, IconAnims, Interact, Playback};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSPanel, NSResponder, NSScreen, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGSize};
use objc2_foundation::{NSObject, NSRect, NSTimer};
use objc2_quartz_core::{CALayer, CATransaction};
use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Duração da troca de linha, da troca de música, do aparecer da letra e da barrinha (s).
const LINE_ANIM: f32 = 0.30;
const TRACK_ANIM: f32 = 0.45;
const LYRIC_FADE: f32 = 0.30;
const TOOLS_ANIM: f32 = 0.14;
/// Espera a música "assentar" antes de buscar a letra (pulando faixas rápido).
const FETCH_DELAY: Duration = Duration::from_millis(450);
/// Falhou (sem rede): tenta de novo depois disto.
const RETRY: Duration = Duration::from_secs(60);
/// Temporizador lento: acompanha a música e troca a linha.
const TICK: f64 = 0.1;
/// Janela acima das outras (NSFloatingWindowLevel) e, na tela cheia, acima da barra de menus.
const LEVEL: isize = 3;
const LEVEL_FULL: isize = 24 + 2;
/// Margem dos lugares prontos até a borda da tela (pontos).
const MARGIN: f64 = 24.0;
/// Soltando o painel a menos disto do centro de um lugar pronto, ele gruda lá (pontos).
const SNAP: f64 = 56.0;

/// Letra que chegou do buscador (outra thread): (música, letra).
static FETCHED: Mutex<Option<(String, Lyrics)>> = Mutex::new(None);

thread_local! {
    static O: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Overlay) -> R) -> Option<R> {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| O.with(|o| o.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))));
    match r {
        Ok(v) => v,
        Err(_) => {
            // Um pânico aqui não pode fechar o app: a letra some e o resto segue.
            system::log::crash("letra: falha no desenho; janela da letra fechada");
            O.with(|o| {
                if let Ok(mut g) = o.try_borrow_mut() {
                    if let Some(o) = g.take() {
                        o.close();
                    }
                }
            });
            None
        }
    }
}

/// Transporte do buscador: o `curl` do sistema (`crate::http`).
fn http_get(host: &str, path: &str, ua: &str) -> Result<Response, String> {
    let r = crate::http::get(&format!("https://{host}{path}"), &[("User-Agent", ua), ("Accept", "application/json")], 2 << 20)?;
    Ok(Response { status: r.status, body: String::from_utf8_lossy(&r.body).into_owned(), retry_after: None })
}

// ---------------------------------------------------------------- de fora

/// Liga, avisa ou desliga a janela conforme as opções (`lyrics.show`).
pub fn sync(mtm: MainThreadMarker) {
    let want = shared().config().lyrics.show;
    let exists = O.with(|o| o.try_borrow().map(|g| g.is_some()).unwrap_or(true));
    if want && !exists {
        let o = Overlay::new(mtm);
        O.with(|c| {
            if let Ok(mut g) = c.try_borrow_mut() {
                *g = Some(o);
            }
        });
        with(|o| o.tick());
    } else if !want && exists {
        O.with(|o| {
            if let Ok(mut g) = o.try_borrow_mut() {
                if let Some(o) = g.take() {
                    o.close();
                }
            }
        });
    } else {
        with(|o| o.tick());
    }
}

/// Temporizador lento (10 Hz).
pub fn tick() {
    with(|o| o.tick());
}

/// Quadro do vblank (animações).
pub fn frame() {
    with(|o| {
        o.render();
        o.schedule();
    });
}

fn arrived() {
    with(|o| o.lyrics_arrived());
}

// ---------------------------------------------------------------- estado

#[derive(Clone, Copy)]
struct Drag {
    /// Mouse e origem da janela quando o botão desceu (pontos da tela).
    mouse: CGPoint,
    origin: CGPoint,
    moved: bool,
}

struct Overlay {
    mtm: MainThreadMarker,
    window: Retained<NSPanel>,
    view: Retained<LyricsView>,
    content: Retained<CALayer>,
    ticker: Option<Ticker>,
    timer: Retained<NSTimer>,
    cfg: Config,
    card_cfg: Config,
    cfg_version: u64,
    colors: Colors,
    media: MediaInfo,
    media_version: u64,
    track: Track,
    track_key: String,
    lyrics: Lyrics,
    lines: Vec<Line>,
    fetcher: Option<Fetcher>,
    fetch_at: Option<Instant>,
    /// Linha atual (-1: antes da primeira) e a animação da troca (foco de onde veio, quando).
    cur: i32,
    anim: Option<(f32, Instant)>,
    /// Letra simples: quanto rolou (px lógicos).
    scroll: f32,
    lyr_at: Option<Instant>,
    change: Option<(Instant, MediaInfo)>,
    visible: bool,
    fullscreen: bool,
    tr: TextRenderer,
    buf: Vec<u32>,
    geo: Geo,
    /// Escala da tela (Retina = 2) e a do desenho (com o zoom da letra).
    bs: f32,
    s: f32,
    origin: CGPoint,
    size: CGSize,
    hover: Option<LHit>,
    pressed: Option<LHit>,
    tools: f32,
    last_step: Instant,
    drag: Option<Drag>,
    seek: Option<f32>,
    seek_sent: Option<(f64, Instant)>,
    vol_drag: bool,
    volume: Volume,
    last_vol_poll: Option<Instant>,
    icons: IconAnims,
    play_override: Option<(bool, Instant)>,
    /// Relógio do fundo animado. `f64`: em `f32` passa a andar aos trancos depois
    /// de ~36 h em tela cheia (B15).
    bg_clock: f64,
    last_sig: Option<u64>,
    ignoring: bool,
}

impl Overlay {
    fn new(mtm: MainThreadMarker) -> Self {
        let rect = NSRect::new(CGPoint::new(0.0, 0.0), CGSize::new(10.0, 10.0));
        let window = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            rect,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        unsafe { window.setReleasedWhenClosed(false) };
        window.setOpaque(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setHasShadow(false);
        window.setLevel(LEVEL);
        window.setHidesOnDeactivate(false);
        window.setMovable(false);
        window.setBecomesKeyOnlyIfNeeded(true);
        window.setAcceptsMouseMovedEvents(true);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::Stationary | NSWindowCollectionBehavior::IgnoresCycle | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        let view = LyricsView::new(mtm, rect);
        let root = CALayer::new();
        root.setGeometryFlipped(true);
        let content = CALayer::new();
        root.addSublayer(&content);
        view.setLayer(Some(&root));
        view.setWantsLayer(true);
        let opts = NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::MouseMoved | NSTrackingAreaOptions::ActiveAlways | NSTrackingAreaOptions::InVisibleRect;
        let area = unsafe { NSTrackingArea::initWithRect_options_owner_userInfo(NSTrackingArea::alloc(), rect, opts, Some(&view), None) };
        view.addTrackingArea(&area);
        window.setContentView(Some(&view));
        let timer = unsafe { NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(TICK, &target(), sel!(lyricsTick:), None, true) };
        timer.setTolerance(0.02);
        let cfg = shared().config();
        let now = Instant::now();
        let mut o = Overlay {
            mtm,
            window,
            view,
            content,
            ticker: None,
            timer,
            card_cfg: card_config(&cfg),
            colors: system::colors(mtm, cfg.palette),
            cfg_version: u64::MAX,
            cfg,
            media: MediaInfo::default(),
            media_version: u64::MAX,
            track: Track::default(),
            track_key: String::new(),
            lyrics: Lyrics::Loading,
            lines: Vec::new(),
            fetcher: None,
            fetch_at: None,
            cur: -1,
            anim: None,
            scroll: 0.0,
            lyr_at: None,
            change: None,
            visible: false,
            fullscreen: false,
            tr: TextRenderer::new(),
            buf: Vec::new(),
            geo: Geo::default(),
            bs: 2.0,
            s: 2.0,
            origin: CGPoint::new(0.0, 0.0),
            size: CGSize::new(0.0, 0.0),
            hover: None,
            pressed: None,
            tools: 0.0,
            last_step: now,
            drag: None,
            seek: None,
            seek_sent: None,
            vol_drag: false,
            volume: Volume::default(),
            last_vol_poll: None,
            icons: IconAnims::default(),
            play_override: None,
            bg_clock: 0.0,
            last_sig: None,
            ignoring: false,
        };
        o.tr.set_capacity(260);
        system::log::info("letra: janela criada");
        o
    }

    fn close(self) {
        self.timer.invalidate();
        drop(self.ticker);
        self.window.orderOut(None);
        view::forget();
        system::log::info("letra: janela fechada");
    }

    fn lc(&self) -> &LyricsConfig {
        &self.cfg.lyrics
    }

    fn has_volume(&self) -> bool {
        !self.fullscreen && self.lc().volume && matches!(self.lc().layout, Layout::Compact | Layout::Player | Layout::PlayerLyrics)
    }

    // ------------------------------------------------------------ ritmo

    fn tick(&mut self) {
        self.reload_config();
        self.reload_media();
        let now = Instant::now();
        if self.fetch_at.is_some_and(|t| now >= t) {
            self.fetch_at = None;
            self.fetch_now();
        }
        if self.play_override.is_some_and(|(p, t)| p == self.media.playing || t.elapsed() > Duration::from_secs(2)) {
            self.play_override = None;
        }
        if self.has_volume() && self.visible && self.last_vol_poll.is_none_or(|t| t.elapsed() > Duration::from_millis(900)) {
            self.volume.poll();
            self.last_vol_poll = Some(now);
        }
        self.update_line(true);
        self.render();
        self.schedule();
    }

    /// Animando: vblank a 60 Hz; parado, só o temporizador lento.
    fn schedule(&mut self) {
        let busy = self.visible && (self.anim.is_some() || self.change.is_some() || self.icons.running() || self.tools != self.tools_target() || self.lyr_at.is_some_and(|t| t.elapsed().as_secs_f32() < LYRIC_FADE) || self.bg_anim());
        let view = self.view.clone();
        self.ticker.get_or_insert_with(|| Ticker::new(&view, sel!(lyricsFrame:))).set_rate(if busy { 60 } else { 0 });
    }

    fn tools_target(&self) -> f32 {
        if self.hover.is_some() && !self.fullscreen { 1.0 } else { 0.0 }
    }

    fn bg_anim(&self) -> bool {
        self.fullscreen && self.lc().animated_bg && self.media.playing
    }

    // ------------------------------------------------------------ opções e música

    fn reload_config(&mut self) {
        let v = shared().config_version.load(Ordering::Relaxed);
        if v == self.cfg_version {
            return;
        }
        self.cfg_version = v;
        let old = self.cfg.lyrics.clone();
        self.cfg = shared().config();
        self.card_cfg = card_config(&self.cfg);
        self.colors = system::colors(self.mtm, self.cfg.palette);
        if old.online != self.lc().online && matches!(self.lyrics, Lyrics::Offline | Lyrics::Failed) {
            self.track_key.clear();
        }
        if old.layout != self.lc().layout {
            self.scroll = 0.0;
        }
        self.apply_mouse();
        self.last_sig = None;
        self.media_version = u64::MAX;
        self.reload_media();
    }

    fn reload_media(&mut self) {
        let v = shared().media_version.load(Ordering::Relaxed);
        if v == self.media_version {
            return;
        }
        self.media_version = v;
        let new = shared().media.lock().unwrap().clone();
        let old = std::mem::replace(&mut self.media, new);
        let track = Track { artist: self.media.artist.clone(), title: self.media.title.clone(), album: self.media.album.clone(), dur: self.media.dur };
        let key = track.key();
        if key != self.track_key {
            let had = !self.track_key.is_empty() && old.present();
            self.change = (had && self.visible && self.cfg.track_anim).then(|| (Instant::now(), old));
            self.track_key = key;
            self.track = track;
            self.lyrics = Lyrics::Loading;
            self.lines.clear();
            self.cur = -1;
            self.anim = None;
            self.scroll = 0.0;
            self.lyr_at = None;
            if self.media.title.trim().is_empty() {
                self.lyrics = Lyrics::NotFound;
                self.fetch_at = None;
            } else {
                self.fetch_at = Some(Instant::now() + FETCH_DELAY);
            }
            self.last_sig = None;
        } else {
            // A duração costuma chegar depois do título.
            self.track.dur = self.media.dur;
        }
        self.update_visibility();
    }

    fn fetch_now(&mut self) {
        if !self.visible || (!self.lc().layout.has_lyrics() && !self.fullscreen) || self.track.title.trim().is_empty() {
            return;
        }
        if self.fetcher.is_none() {
            self.fetcher = Fetcher::spawn(http_get, |key, l| {
                *FETCHED.lock().unwrap() = Some((key, l));
                dispatch2::DispatchQueue::main().exec_async(arrived);
            });
        }
        if let Some(f) = &self.fetcher {
            f.request(self.track.clone(), self.lc().online);
        }
    }

    fn lyrics_arrived(&mut self) {
        let Some((key, l)) = FETCHED.lock().unwrap().take() else { return };
        if key != self.track_key {
            return;
        }
        if l == Lyrics::Failed {
            self.fetch_at = Some(Instant::now() + RETRY);
        }
        self.lines = match &l {
            Lyrics::Synced(lines) => lyrics::with_gaps(lines),
            _ => Vec::new(),
        };
        self.lyrics = l;
        self.lyr_at = Some(Instant::now());
        self.cur = -2;
        self.anim = None;
        self.last_sig = None;
        self.update_line(false);
        self.render();
        self.schedule();
    }

    /// Posição da música agora, com o ajuste de tempo.
    fn position(&self) -> f64 {
        let mut pos = self.media.position_now();
        if let Some((p, t)) = self.seek_sent.filter(|(_, t)| t.elapsed() < Duration::from_millis(1500)) {
            pos = p + if self.media.playing { t.elapsed().as_secs_f64() } else { 0.0 };
        }
        pos + self.lc().offset_ms as f64 / 1000.0
    }

    fn update_line(&mut self, animate: bool) {
        if !matches!(self.lyrics, Lyrics::Synced(_)) || self.lines.is_empty() {
            self.cur = -1;
            return;
        }
        let new = lyrics::line_at(&self.lines, self.position()).map(|i| i as i32).unwrap_or(-1);
        if new != self.cur {
            let from = self.focus();
            // Anima só a troca de uma linha para a seguinte (pulo = busca na música).
            self.anim = (animate && self.visible && self.cur >= -1 && new == self.cur + 1).then(|| (from, Instant::now()));
            self.cur = new;
            self.last_sig = None;
        }
    }

    fn focus(&self) -> f32 {
        match self.anim {
            Some((from, t0)) => {
                let u = (t0.elapsed().as_secs_f32() / LINE_ANIM).min(1.0);
                from + (self.cur as f32 - from) * ui::ease_out_cubic(u)
            }
            None => self.cur as f32,
        }
    }

    fn update_visibility(&mut self) {
        let lc = self.lc();
        let want = lc.show && (self.fullscreen || !(lc.hide_idle && !self.media.present()));
        if want == self.visible {
            return;
        }
        self.visible = want;
        if want {
            if self.fetch_at.is_none() && matches!(self.lyrics, Lyrics::Loading) && !self.track.title.trim().is_empty() {
                self.fetch_at = Some(Instant::now());
            }
            self.last_sig = None;
        } else {
            self.window.orderOut(None);
            self.hover = None;
            self.drag = None;
            self.change = None;
            self.fetcher = None;
            self.buf = Vec::new();
            view::forget();
        }
    }

    /// Travada com "o clique atravessa": a janela deixa o mouse passar.
    fn apply_mouse(&mut self) {
        let through = self.lc().locked && self.lc().click_through && !self.fullscreen;
        if through != self.ignoring {
            self.ignoring = through;
            self.window.setIgnoresMouseEvents(through);
            if through {
                self.hover = None;
                self.pressed = None;
                self.drag = None;
            }
        }
    }

    // ------------------------------------------------------------ tela e lugar

    fn screen(&self) -> Option<Retained<NSScreen>> {
        let screens = NSScreen::screens(self.mtm);
        let i = (self.lc().monitor.max(0) as usize).min(screens.count().saturating_sub(1));
        if screens.count() > 0 { Some(screens.objectAtIndex(i)) } else { NSScreen::mainScreen(self.mtm) }
    }

    /// Onde fica a janela (canto de baixo à esquerda, pontos do AppKit) para
    /// o painel ir ao lugar escolhido em `screen`.
    fn place(&self, g: &Geo, screen: &NSScreen) -> CGPoint {
        if g.fullscreen {
            return screen.frame().origin;
        }
        let bs = self.bs as f64;
        let vf = screen.visibleFrame();
        let (cw, ch) = (g.card.w as f64 / bs, g.card.h as f64 / bs);
        let top = vf.origin.y + vf.size.height;
        let (left, card_top) = match self.lc().position.frac() {
            Some((fx, fy)) => {
                let (aw, ah) = (vf.size.width - 2.0 * MARGIN, vf.size.height - 2.0 * MARGIN);
                (vf.origin.x + MARGIN + (aw - cw) * fx as f64, top - MARGIN - (ah - ch) * fy as f64)
            }
            None => {
                let mx = vf.origin.x + self.lc().free_x as f64 * vf.size.width;
                let my = top - self.lc().free_y as f64 * vf.size.height;
                let left = (mx - cw / 2.0).clamp(vf.origin.x, (vf.origin.x + vf.size.width - cw).max(vf.origin.x));
                let card_top = (my + ch / 2.0).clamp((vf.origin.y + ch).min(top), top);
                (left, card_top)
            }
        };
        // A janela tem a margem da sombra em volta do painel.
        CGPoint::new((left - g.card.x as f64 / bs).round(), (card_top + g.card.y as f64 / bs - g.h as f64 / bs).round())
    }

    /// Soltou o painel arrastado: grava o lugar (um dos prontos, se perto).
    fn drop_here(&mut self) {
        let Some(screen) = self.screen() else { return };
        let bs = self.bs as f64;
        let g = &self.geo;
        let vf = screen.visibleFrame();
        let top = vf.origin.y + vf.size.height;
        let (cw, ch) = (g.card.w as f64 / bs, g.card.h as f64 / bs);
        let cx = self.origin.x + (g.card.x + g.card.w / 2.0) as f64 / bs;
        let cy = self.origin.y + g.h as f64 / bs - (g.card.y + g.card.h / 2.0) as f64 / bs;
        let (aw, ah) = (vf.size.width - 2.0 * MARGIN, vf.size.height - 2.0 * MARGIN);
        let snap = Position::GRID
            .iter()
            .filter_map(|p| {
                let (fx, fy) = p.frac()?;
                let sx = vf.origin.x + MARGIN + (aw - cw) * fx as f64 + cw / 2.0;
                let sy = top - MARGIN - (ah - ch) * fy as f64 - ch / 2.0;
                let d = ((sx - cx).powi(2) + (sy - cy).powi(2)).sqrt();
                (d < SNAP).then_some((d, *p))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, p)| p);
        let fx = ((cx - vf.origin.x) / vf.size.width.max(1.0)).clamp(0.0, 1.0) as f32;
        let fy = ((top - cy) / vf.size.height.max(1.0)).clamp(0.0, 1.0) as f32;
        shared().update_config(|c| match snap {
            Some(p) => c.lyrics.position = p,
            None => {
                c.lyrics.position = Position::Free;
                c.lyrics.free_x = fx;
                c.lyrics.free_y = fy;
            }
        });
        self.reload_config();
    }

    fn set_fullscreen(&mut self, on: bool) {
        if on == self.fullscreen {
            return;
        }
        self.fullscreen = on;
        self.window.setLevel(if on { LEVEL_FULL } else { LEVEL });
        self.hover = None;
        self.pressed = None;
        self.drag = None;
        self.apply_mouse();
        self.update_visibility();
        if on && matches!(self.lyrics, Lyrics::Loading) {
            self.fetch_at = Some(Instant::now());
        }
        self.last_sig = None;
    }

    // ------------------------------------------------------------ mouse

    fn playback(&self) -> Playback {
        let playing = self.play_override.map(|(p, _)| p).unwrap_or(self.media.playing);
        Playback {
            playing,
            shuffle: self.media.shuffle,
            repeat: self.media.repeat,
            pos: self.position() - self.lc().offset_ms as f64 / 1000.0,
            seek_preview: self.seek,
            volume: self.volume.level,
            muted: self.volume.muted,
        }
    }

    fn hit_at(&self, x: f32, y: f32) -> Option<LHit> {
        view::hit(&self.geo, self.lc(), x, y, !self.fullscreen)
    }

    fn frac(track: Option<(f32, f32)>, x: f32) -> Option<f32> {
        let (x0, x1) = track?;
        Some(((x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0))
    }

    fn mouse_move(&mut self, x: f32, y: f32) {
        let h = if x < 0.0 { None } else { self.hit_at(x, y) };
        if h != self.hover {
            self.hover = h;
            self.last_sig = None;
        }
        self.render();
        self.schedule();
    }

    fn mouse_down(&mut self, x: f32, y: f32) {
        let h = self.hit_at(x, y);
        self.pressed = h;
        match h {
            Some(LHit::Card(Hit::Seek)) if self.media.can_seek && self.media.dur > 0.0 => self.seek = Self::frac(view::seek_track(&self.geo, self.s), x),
            Some(LHit::Card(Hit::Vol)) => {
                self.vol_drag = true;
                if let Some(v) = Self::frac(view::vol_track(&self.geo, self.s), x) {
                    self.volume.set_level(v);
                }
            }
            Some(LHit::Body) if !self.lc().locked && !self.fullscreen => {
                self.drag = Some(Drag { mouse: NSEvent::mouseLocation(), origin: self.origin, moved: false });
            }
            _ => {}
        }
        self.last_sig = None;
        self.render();
        self.schedule();
    }

    fn mouse_dragged(&mut self, x: f32, y: f32) {
        if self.seek.is_some() {
            self.seek = Self::frac(view::seek_track(&self.geo, self.s), x).or(self.seek);
        } else if self.vol_drag {
            if let Some(v) = Self::frac(view::vol_track(&self.geo, self.s), x) {
                self.volume.set_level(v);
            }
        } else if let Some(mut d) = self.drag {
            let p = NSEvent::mouseLocation();
            let (dx, dy) = (p.x - d.mouse.x, p.y - d.mouse.y);
            if d.moved || dx.abs() + dy.abs() > 3.0 {
                d.moved = true;
                self.origin = CGPoint::new((d.origin.x + dx).round(), (d.origin.y + dy).round());
                self.window.setFrameOrigin(self.origin);
            }
            self.drag = Some(d);
            return;
        } else {
            let _ = y;
        }
        self.last_sig = None;
        self.render();
    }

    fn mouse_up(&mut self, x: f32, y: f32) {
        let h = self.hit_at(x, y);
        if let Some(f) = self.seek.take() {
            let pos = f as f64 * self.media.dur;
            shared().send_media(MediaCmd::Seek(pos));
            self.seek_sent = Some((pos, Instant::now()));
            self.update_line(false);
        } else if self.vol_drag {
            self.vol_drag = false;
        } else if let Some(d) = self.drag.take() {
            if d.moved {
                self.drop_here();
            } else if h == self.pressed {
                self.click(h);
            }
        } else if h == self.pressed {
            self.click(h);
        }
        self.pressed = None;
        self.last_sig = None;
        self.render();
        self.schedule();
    }

    fn scroll(&mut self, dy: f32) {
        if matches!(self.lyrics, Lyrics::Plain(_)) {
            self.scroll = (self.scroll - dy).max(0.0);
            self.last_sig = None;
            self.render();
        }
    }

    fn click(&mut self, h: Option<LHit>) {
        let sh = shared();
        match h {
            Some(LHit::Lock) => sh.update_config(|c| c.lyrics.locked = !c.lyrics.locked),
            Some(LHit::Compact) => sh.update_config(|c| c.lyrics.toggle_compact()),
            Some(LHit::Full) => self.set_fullscreen(!self.fullscreen),
            Some(LHit::Close) => {
                self.set_fullscreen(false);
                sh.update_config(|c| c.lyrics.show = false);
                // Fora deste clique (a janela está emprestada): fecha e avisa a ilha.
                dispatch2::DispatchQueue::main().exec_async(super::config_changed);
                return;
            }
            Some(LHit::Card(Hit::Play)) if self.media.can_toggle => {
                self.play_override = Some((!self.playback().playing, Instant::now()));
                self.icons.kick(Hit::Play);
                sh.send_media(MediaCmd::PlayPause);
            }
            Some(LHit::Card(Hit::Prev)) => {
                self.icons.kick(Hit::Prev);
                sh.send_media(MediaCmd::Previous);
            }
            Some(LHit::Card(Hit::Next)) => {
                self.icons.kick(Hit::Next);
                sh.send_media(MediaCmd::Next);
            }
            Some(LHit::Card(Hit::Shuffle)) if self.media.can_shuffle => {
                self.icons.kick(Hit::Shuffle);
                sh.send_media(MediaCmd::Shuffle(!self.media.shuffle));
            }
            Some(LHit::Card(Hit::Repeat)) if self.media.can_repeat => {
                self.icons.kick(Hit::Repeat);
                sh.send_media(MediaCmd::Repeat((self.media.repeat + 1) % 3));
            }
            Some(LHit::Card(Hit::VolBtn)) => {
                self.icons.kick(Hit::VolBtn);
                self.volume.toggle_mute();
            }
            _ => {}
        }
        self.reload_config();
        self.last_sig = None;
    }

    // ------------------------------------------------------------ desenho

    fn render(&mut self) {
        if !self.visible {
            return;
        }
        let Some(screen) = self.screen() else { return };
        let now = Instant::now();
        let dt = (now - self.last_step).as_secs_f32().min(0.1);
        self.last_step = now;
        let tt = self.tools_target();
        if self.tools != tt {
            let step = dt / TOOLS_ANIM;
            self.tools = if tt > self.tools { (self.tools + step).min(1.0) } else { (self.tools - step).max(0.0) };
        }
        if self.anim.is_some_and(|(_, t0)| t0.elapsed().as_secs_f32() >= LINE_ANIM) {
            self.anim = None;
        }
        if self.change.as_ref().is_some_and(|(t0, _)| t0.elapsed().as_secs_f32() >= TRACK_ANIM) {
            self.change = None;
        }
        if self.bg_anim() {
            self.bg_clock += dt as f64;
        }
        let lyr_alpha = self.lyr_at.map(|t| (t.elapsed().as_secs_f32() / LYRIC_FADE).min(1.0)).unwrap_or(1.0);
        let pb = self.playback();
        self.icons.observe(&pb);
        let busy = self.anim.is_some() || self.change.is_some() || self.icons.running() || self.tools != tt || lyr_alpha < 1.0 || self.seek.is_some() || self.vol_drag || self.bg_anim();
        let sig = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (pb.pos.floor() as i64, format!("{:?}{:?}", self.hover, self.pressed), self.cur, pb.playing, (pb.volume * 200.0) as i32, pb.muted, self.scroll.to_bits(), self.tools.to_bits()).hash(&mut h);
            h.finish()
        };
        if !busy && self.last_sig == Some(sig) {
            return;
        }
        self.last_sig = Some(sig);

        // Escala: a da tela (e o zoom da letra fora da tela cheia).
        self.bs = screen.backingScaleFactor() as f32;
        self.s = if self.fullscreen { self.bs } else { self.bs * self.lc().zoom.clamp(crate::lyrics::config::ZOOM_MIN, crate::lyrics::config::ZOOM_MAX) as f32 / 100.0 };
        let s = self.s;
        self.tr.set_scale(s);
        let focus = self.focus();
        let geo = if self.fullscreen {
            let f = screen.frame();
            view::fullscreen_geometry((f.size.width as f32 * self.bs).round(), (f.size.height as f32 * self.bs).round(), s)
        } else {
            let frame = LyricFrame { lyrics: &self.lyrics, lines: &self.lines, focus, scroll: self.scroll };
            let caption = if self.lc().layout == Layout::Caption { view::caption_content(&mut self.tr, &frame, &self.cfg.lyrics, s) } else { (0.0, 0.0) };
            view::geometry(&self.cfg.lyrics, &self.card_cfg, s, caption, None)
        };
        let (w, h) = (geo.w.ceil().max(1.0) as usize, geo.h.ceil().max(1.0) as usize);
        self.buf.clear();
        self.buf.resize(w * h, 0);
        let card_hit = |h: Option<LHit>| match h {
            Some(LHit::Card(h)) => Some(h),
            _ => None,
        };
        let it = Interact {
            hover: card_hit(self.hover),
            pressed: card_hit(self.pressed),
            drag: if self.seek.is_some() {
                Some(Hit::Seek)
            } else if self.vol_drag {
                Some(Hit::Vol)
            } else {
                None
            },
            fx: self.icons.fx(self.cfg.icon_anim),
            ..Default::default()
        };
        let change = self.change.as_ref().map(|(t0, old)| (old, (t0.elapsed().as_secs_f32() / TRACK_ANIM).min(1.0)));
        {
            let mut cv = Canvas::new(w, h, &mut self.buf);
            cv.clear();
            let v = View {
                card_cfg: &self.card_cfg,
                lc: &self.cfg.lyrics,
                colors: &self.colors,
                media: &self.media,
                pb,
                it,
                hover: self.hover,
                tools: ui::ease_out_cubic(self.tools),
                bg_time: Some(self.bg_clock),
                hires: None,
                change,
                mixer: None,
                lyr_alpha,
                tip: None,
                toast: None,
                frame: LyricFrame { lyrics: &self.lyrics, lines: &self.lines, focus, scroll: self.scroll },
                s,
            };
            view::draw(&mut cv, &mut self.tr, &geo, &v);
        }
        let Some(img) = gfx::to_cgimage(&self.buf, w, h) else { return };
        let bs = self.bs as f64;
        let size = CGSize::new(w as f64 / bs, h as f64 / bs);
        // Arrastando, a janela fica onde o mouse a levou.
        let origin = if self.drag.is_some_and(|d| d.moved) { self.origin } else { self.place(&geo, &screen) };
        if origin != self.origin || size != self.size {
            self.origin = origin;
            self.size = size;
            self.window.setFrame_display(NSRect::new(origin, size), false);
            self.view.setFrame(NSRect::new(CGPoint::new(0.0, 0.0), size));
        }
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.content.setContentsScale(bs);
        self.content.setFrame(NSRect::new(CGPoint::new(0.0, 0.0), size));
        let obj: &AnyObject = unsafe { &*((&*img as *const objc2_core_graphics::CGImage).cast::<AnyObject>()) };
        unsafe { self.content.setContents(Some(obj)) };
        CATransaction::commit();
        self.geo = geo;
        if !self.window.isVisible() {
            self.window.orderFrontRegardless();
        }
    }
}

/// Opções do card usadas pela letra (sem mixer no Mac).
fn card_config(cfg: &Config) -> Config {
    Config { card_mixer: false, ..view::card_config(cfg, &cfg.lyrics) }
}

// ---------------------------------------------------------------- vista

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    struct LyricsView;

    impl LyricsView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _e: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, e: &NSEvent) {
            let (x, y) = self.px(e);
            with(|o| o.mouse_move(x, y));
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, e: &NSEvent) {
            let (x, y) = self.px(e);
            with(|o| o.mouse_move(x, y));
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _e: &NSEvent) {
            with(|o| {
                if o.drag.is_none() && o.seek.is_none() && !o.vol_drag {
                    o.mouse_move(-1.0, -1.0);
                }
            });
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, e: &NSEvent) {
            let (x, y) = self.px(e);
            with(|o| o.mouse_down(x, y));
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, e: &NSEvent) {
            let (x, y) = self.px(e);
            with(|o| o.mouse_dragged(x, y));
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, e: &NSEvent) {
            let (x, y) = self.px(e);
            with(|o| o.mouse_up(x, y));
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, e: &NSEvent) {
            let k = if e.hasPreciseScrollingDeltas() { 1.0 } else { 12.0 };
            let dy = e.scrollingDeltaY() as f32 * k;
            with(|o| o.scroll(dy));
        }
    }
);

impl LyricsView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// Posição do evento em pixels da superfície (origem no canto de cima).
    fn px(&self, e: &NSEvent) -> (f32, f32) {
        let p = self.convertPoint_fromView(e.locationInWindow(), None);
        let s = self.window().map(|w| w.backingScaleFactor()).unwrap_or(2.0) as f32;
        (p.x as f32 * s, p.y as f32 * s)
    }
}
