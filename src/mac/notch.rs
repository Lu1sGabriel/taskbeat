//! Janela da ilha no notch: posição na tela, mouse, animações e o quadro.
//!
//! A janela tem o tamanho do card aberto (com a sombra) e fica presa no topo
//! da tela com o notch, acima da barra de menus. Ela deixa o mouse passar
//! (`ignoresMouseEvents`) fora da parte visível da ilha: um monitor global de
//! movimento do mouse liga os cliques quando o ponteiro entra nela.
//!
//! Desempenho: fechada e parada, nada roda; tocando, o quadro é refeito a
//! 30 fps só na área da ilha fechada (capa + barras); as animações seguem o
//! vblank (CADisplayLink) e param quando terminam.

use super::volume::Volume;
use super::compat::Ticker;
use super::{gfx, shared, statusbar, system, target};
use crate::config::Config;
use crate::dsp::{Analyzer, FFT_SIZE};
use crate::notch::{self as nd, Geom, NLayout, NotchFrame};
use crate::render::{Canvas, Colors, RectF};
use crate::shared::{Launcher, MediaCmd, MediaInfo};
use std::sync::Arc;
use crate::text::TextRenderer;
use crate::ui::{Hit, IconAnims, Interact, Playback};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSPanel, NSResponder, NSScreen, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGSize};
use objc2_foundation::{NSObject, NSRect, NSTimer};
use objc2_quartz_core::{CALayer, CATransaction};
use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const OPEN_DUR: f32 = 0.48;
const CLOSE_DUR: f32 = 0.30;
/// Mouse parado sobre a ilha por este tempo: abre (com "Abrir ao passar o mouse").
const HOVER_DELAY: Duration = Duration::from_millis(140);
/// Mouse fora do card aberto por este tempo: fecha.
const LEAVE_DELAY: Duration = Duration::from_millis(320);
const CHANGE_DUR: f32 = 0.55;
const HINT_DUR: f32 = 2.4;
/// Espera a capa nova antes de mostrar o "A seguir".
const PEEK_ART_WAIT: Duration = Duration::from_millis(1500);
/// Nível acima da barra de menus (NSMainMenuWindowLevel + 3).
const LEVEL: isize = 24 + 3;

/// Valor animado entre `from` e `to` com a curva de abrir (com quique) ou fechar.
#[derive(Clone, Copy)]
struct Anim {
    from: f32,
    to: f32,
    start: Instant,
    dur: f32,
}

impl Anim {
    fn new() -> Self {
        Self { from: 0.0, to: 0.0, start: Instant::now(), dur: 0.0 }
    }

    fn u(&self, now: Instant) -> f32 {
        if self.dur <= 0.0 { 1.0 } else { (now.saturating_duration_since(self.start).as_secs_f32() / self.dur).min(1.0) }
    }

    fn value(&self, now: Instant) -> f32 {
        let u = self.u(now);
        let k = if self.to > self.from { nd::grow(u) } else { nd::shrink(u) };
        self.from + (self.to - self.from) * k
    }

    fn done(&self, now: Instant) -> bool {
        self.u(now) >= 1.0
    }

    fn set(&mut self, to: f32, now: Instant) {
        if (to - self.to).abs() < 1e-4 {
            return;
        }
        let v = self.value(now).clamp(0.0, 1.0);
        let dur = if to > v { OPEN_DUR } else { CLOSE_DUR };
        // Do meio do caminho: dura proporcionalmente menos.
        *self = Self { from: v, to, start: now, dur: dur * (to - v).abs().max(0.35) };
    }
}

struct Notch {
    mtm: MainThreadMarker,
    window: Retained<NSPanel>,
    view: Retained<NotchView>,
    content: Retained<CALayer>,
    ticker: Option<Ticker>,
    _timer: Option<Retained<NSTimer>>,
    _monitor: Option<Retained<AnyObject>>,
    tr: TextRenderer,
    s: f32,
    geom: Geom,
    /// Origem da janela na tela (pontos, canto de baixo à esquerda do AppKit).
    origin: CGPoint,
    screen_frame: NSRect,
    layout: NLayout,
    /// Atalhos dos apps de música à mostra no card e o ícone da ilha fechada (`launchers::pick`).
    apps: Arc<Vec<Launcher>>,
    wing: Option<Launcher>,
    buf: Vec<u32>,
    cfg: Config,
    cfg_version: u64,
    colors: Colors,
    media: MediaInfo,
    media_version: u64,
    open: Anim,
    open_target: bool,
    peek: Anim,
    peek_start: Option<Instant>,
    peek_until: Option<Instant>,
    peek_pending: Option<Instant>,
    change: Option<(MediaInfo, Instant)>,
    hint: Option<(&'static str, Instant)>,
    it: Interact,
    /// Animação dos ícones dos botões.
    icons: IconAnims,
    seek_preview: Option<f32>,
    hover_since: Option<Instant>,
    leave_since: Option<Instant>,
    inside: bool,
    ignoring: bool,
    analyzer: Analyzer,
    samples: Vec<f32>,
    last_written: u64,
    last_audio: Instant,
    volume: Volume,
    last_volume_poll: Instant,
    t0: Instant,
    last_frame: Instant,
    /// Assinatura do último quadro mostrado (pula quadros iguais).
    last_sig: Option<u64>,
    hidden: bool,
}

thread_local! {
    static N: RefCell<Option<Notch>> = const { RefCell::new(None) };
    static PANICS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Pânicos seguidos tolerados antes de parar a ilha (o resto do app segue).
const MAX_PANICS: u32 = 5;

/// Roda `f` com a ilha. Tudo aqui vem do AppKit (vblank, mouse, avisos): um
/// pânico que chegasse ao Objective-C fecharia o app. Ele é pego, registrado
/// (`crash.log`, pelo gancho de pânico) e o quadro é descartado; se repetir,
/// a ilha para e o app continua na barra de menus.
fn with<R>(f: impl FnOnce(&mut Notch) -> R) -> Option<R> {
    if PANICS.with(|p| p.get()) >= MAX_PANICS {
        return None;
    }
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| N.with(|n| n.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))));
    match r {
        Ok(v) => {
            PANICS.with(|p| p.set(0));
            v
        }
        Err(_) => {
            let k = PANICS.with(|p| {
                p.set(p.get() + 1);
                p.get()
            });
            if k >= MAX_PANICS {
                system::log::crash("ilha do notch parada depois de falhas seguidas");
                N.with(|n| {
                    if let Ok(mut g) = n.try_borrow_mut() {
                        if let Some(n) = g.as_mut() {
                            if let Some(t) = n.ticker.as_mut() {
                                t.set_rate(0);
                            }
                            n.window.orderOut(None);
                        }
                    }
                });
            }
            None
        }
    }
}

// ---------------------------------------------------------------- tela e notch

/// Tela com o notch (MacBook) ou a principal, e a medida do notch em pontos.
fn pick_screen(mtm: MainThreadMarker) -> Option<(Retained<NSScreen>, f64, f64)> {
    let screens = NSScreen::screens(mtm);
    let fake = std::env::var("TASKBEAT_FAKE_NOTCH").ok().and_then(|v| {
        let (w, h) = v.split_once('x')?;
        Some((w.trim().parse::<f64>().ok()?, h.trim().parse::<f64>().ok()?))
    });
    for sc in screens.iter() {
        let top = sc.safeAreaInsets().top;
        if top > 0.0 {
            let f = sc.frame();
            let (l, r) = (sc.auxiliaryTopLeftArea(), sc.auxiliaryTopRightArea());
            let w = (f.size.width - l.size.width - r.size.width).max(0.0);
            return Some((sc, w, top));
        }
    }
    let sc = screens.firstObject().or_else(|| NSScreen::mainScreen(mtm))?;
    if let Some((w, h)) = fake {
        return Some((sc, w, h));
    }
    // Sem notch: altura da barra de menus.
    let (f, v) = (sc.frame(), sc.visibleFrame());
    let bar = (f.origin.y + f.size.height) - (v.origin.y + v.size.height);
    Some((sc, 0.0, if bar > 8.0 { bar } else { 24.0 }))
}

// ---------------------------------------------------------------- início

pub fn start(mtm: MainThreadMarker) {
    let sh = shared();
    let cfg = sh.config();
    let Some((screen, nw, nh)) = pick_screen(mtm) else { return };
    let s = screen.backingScaleFactor() as f32;
    let geom = Geom { notch_w: nw as f32 * s, notch_h: nh as f32 * s };
    let media = sh.media.lock().unwrap().clone();
    let (apps, wing) = super::launchers::pick(&sh.launchers.lock().unwrap(), &cfg, &media.aumid);
    let apps = Arc::new(apps);
    let layout = card_layout(s, geom, &cfg, apps.len(), wing.is_some());

    let rect = NSRect::new(CGPoint::new(0.0, 0.0), CGSize::new((layout.w / s) as f64, (layout.h / s) as f64));
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
    window.setIgnoresMouseEvents(true);
    window.setCollectionBehavior(behavior(&cfg));

    let view = NotchView::new(mtm, rect);
    let root = CALayer::new();
    root.setGeometryFlipped(true);
    let content = CALayer::new();
    content.setContentsScale(s as f64);
    root.addSublayer(&content);
    view.setLayer(Some(&root));
    view.setWantsLayer(true);
    let opts = NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::MouseMoved | NSTrackingAreaOptions::ActiveAlways | NSTrackingAreaOptions::InVisibleRect;
    let area = unsafe { NSTrackingArea::initWithRect_options_owner_userInfo(NSTrackingArea::alloc(), rect, opts, Some(&view), None) };
    view.addTrackingArea(&area);
    window.setContentView(Some(&view));

    // Movimento do mouse em qualquer app: liga os cliques quando ele entra na ilha.
    let block = RcBlock::new(|_e: NonNull<NSEvent>| global_mouse());
    let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::MouseMoved | NSEventMask::LeftMouseDragged, &block);

    // Temporizador lento (2 Hz): volume com o card aberto, sessão, etc.
    // Common modes (B12): segue com o menu aberto. A tolerância e o invalidate
    // quando não precisa ficam para o PR-24 (D4), que mexe nestas linhas.
    let timer = super::compat::common_timer(0.5, &target(), sel!(tick:), 0.0);

    let mut tr = TextRenderer::new();
    tr.set_scale(s);
    let colors = system::colors(mtm, cfg.palette);
    let rate = sh.sample_rate.load(Ordering::Relaxed);
    let mut n = Notch {
        mtm,
        window,
        view,
        content,
        ticker: None,
        _timer: Some(timer),
        _monitor: monitor,
        tr,
        s,
        geom,
        origin: CGPoint::new(0.0, 0.0),
        screen_frame: rect,
        buf: vec![0; (layout.w.ceil() * layout.h.ceil()) as usize],
        layout,
        apps,
        wing,
        analyzer: Analyzer::new(cfg.bars, rate),
        cfg,
        cfg_version: sh.config_version.load(Ordering::Relaxed),
        colors,
        media: sh.media.lock().unwrap().clone(),
        media_version: sh.media_version.load(Ordering::Relaxed),
        open: Anim::new(),
        open_target: false,
        peek: Anim::new(),
        peek_start: None,
        peek_until: None,
        peek_pending: None,
        change: None,
        hint: None,
        it: Interact::default(),
        icons: IconAnims::default(),
        seek_preview: None,
        hover_since: None,
        leave_since: None,
        inside: false,
        ignoring: true,
        samples: vec![0.0; FFT_SIZE],
        last_written: 0,
        last_audio: Instant::now(),
        volume: Volume::default(),
        last_volume_poll: Instant::now(),
        t0: Instant::now(),
        last_frame: Instant::now(),
        last_sig: None,
        hidden: true,
    };
    n.place(&screen);
    if demo("TV_DEMO_MEDIA") {
        n.media = crate::ui::sample_media();
    }
    if demo("TV_DEMO_FLYOUT") {
        n.open_target = true;
        n.open = Anim { from: 1.0, to: 1.0, start: Instant::now(), dur: 0.0 };
    }
    if demo("TV_DEMO_POPUP") {
        let now = Instant::now();
        n.peek = Anim { from: 1.0, to: 1.0, start: now, dur: 0.0 };
        n.peek_start = Some(now);
        n.peek_until = Some(now + Duration::from_secs(3600));
    }
    N.with(|c| *c.borrow_mut() = Some(n));
    with(|n| {
        n.update_visibility();
        n.render(true);
        n.schedule();
    });
}

fn demo(var: &str) -> bool {
    std::env::var_os(var).is_some()
}

fn behavior(cfg: &Config) -> NSWindowCollectionBehavior {
    let mut b = NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::Stationary | NSWindowCollectionBehavior::IgnoresCycle;
    if !cfg.hide_on_fullscreen {
        b |= NSWindowCollectionBehavior::FullScreenAuxiliary;
    }
    b
}

/// Layout com os extras do Mac: botão da letra e atalhos dos apps no card, e
/// o ícone do app na ilha fechada (`wing`).
fn card_layout(s: f32, geom: Geom, cfg: &Config, apps: usize, wing: bool) -> NLayout {
    let l = NLayout::build(s, geom, cfg, true);
    let l = if wing { l.with_wing_app(cfg.notch_wing) } else { l };
    let l = if cfg.lyrics.card_button { l.with_lyrics_button() } else { l };
    l.with_launchers(apps)
}

// ---------------------------------------------------------------- avisos de fora

/// A lista de apps de música instalados mudou (thread dos atalhos).
pub fn launchers_changed() {
    with(|n| {
        if n.pull_launchers() {
            n.relayout();
            n.reposition();
        }
        n.render(true);
        n.schedule();
    });
}

pub fn media_changed() {
    with(|n| {
        n.pull_media();
        n.render(false);
        n.schedule();
    });
}

pub fn config_changed() {
    with(|n| {
        n.pull_config();
        n.render(true);
        n.schedule();
    });
}

pub fn screens_changed() {
    let mtm = MainThreadMarker::new().unwrap();
    with(|n| {
        if let Some((screen, nw, nh)) = pick_screen(mtm) {
            let s = screen.backingScaleFactor() as f32;
            n.s = s;
            n.tr.set_scale(s);
            n.content.setContentsScale(s as f64);
            n.geom = Geom { notch_w: nw as f32 * s, notch_h: nh as f32 * s };
            n.relayout();
            n.place(&screen);
            n.colors = system::colors(mtm, n.cfg.palette);
            n.render(true);
            n.schedule();
        }
    });
}

/// Altura da faixa do notch, em pontos (o aviso de Caps Lock fica abaixo dela).
pub fn band_points() -> Option<f64> {
    with(|n| (n.layout.band / n.s) as f64)
}

/// Quadro do vblank.
pub fn frame() {
    with(|n| {
        n.step();
        n.render(false);
        n.schedule();
    });
}

/// Temporizador lento.
pub fn tick() {
    with(|n| {
        n.pull_media();
        n.pull_config();
        if n.open_target && n.last_volume_poll.elapsed() > Duration::from_millis(900) {
            n.volume.poll();
            n.last_volume_poll = Instant::now();
        }
        n.step();
        n.render(false);
        n.schedule();
    });
}

/// Movimento do mouse fora do app (monitor global).
fn global_mouse() {
    let p = NSEvent::mouseLocation();
    with(|n| {
        let (x, y) = n.screen_to_px(p);
        // Longe da janela: só importa se o mouse acabou de sair da ilha.
        if y > n.layout.h + 40.0 * n.s && !n.inside {
            return;
        }
        n.mouse_move(x, y);
    });
}

// ---------------------------------------------------------------- estado

impl Notch {
    fn place(&mut self, screen: &NSScreen) {
        self.screen_frame = screen.frame();
        self.reposition();
    }

    /// Centraliza a janela no topo da tela (o tamanho muda com as opções).
    fn reposition(&mut self) {
        let f = self.screen_frame;
        let (w, h) = ((self.layout.w / self.s) as f64, (self.layout.h / self.s) as f64);
        let x = (f.origin.x + (f.size.width - w) / 2.0).round();
        let y = f.origin.y + f.size.height - h;
        self.origin = CGPoint::new(x, y);
        self.window.setFrame_display(NSRect::new(self.origin, CGSize::new(w, h)), true);
        self.view.setFrame(NSRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w, h)));
    }

    fn relayout(&mut self) {
        self.layout = card_layout(self.s, self.geom, &self.cfg, self.apps.len(), self.wing.is_some());
        self.buf = vec![0; (self.layout.w.ceil() * self.layout.h.ceil()) as usize];
        self.last_sig = None;
    }

    fn screen_to_px(&self, p: CGPoint) -> (f32, f32) {
        let h = (self.layout.h / self.s) as f64;
        let x = (p.x - self.origin.x) as f32 * self.s;
        let y = (self.origin.y + h - p.y) as f32 * self.s;
        (x, y)
    }

    fn pull_media(&mut self) {
        let sh = shared();
        let v = sh.media_version.load(Ordering::Relaxed);
        if v == self.media_version || demo("TV_DEMO_MEDIA") {
            return;
        }
        self.media_version = v;
        let m = sh.media.lock().unwrap().clone();
        let now = Instant::now();
        // "A seguir": o player trocou de faixa sozinho.
        if m.auto_change_seq != self.media.auto_change_seq && self.cfg.next_track_popup && !self.open_target {
            self.peek_pending = Some(now);
        }
        // Troca de música: título desliza e a capa nova aparece.
        let same = m.title == self.media.title && m.artist == self.media.artist;
        if !same && self.media.present() && m.present() && self.cfg.track_anim {
            self.change = Some((self.media.clone(), now));
        }
        let player_changed = m.aumid != self.media.aumid;
        self.media = m;
        // Outro player: o ícone da ilha (e o "Só um" do card) segue o que está tocando.
        if player_changed && self.pull_launchers() {
            self.relayout();
            self.reposition();
        }
        self.update_visibility();
    }

    /// Relê os atalhos do card e o ícone da ilha; `true` se o layout muda
    /// (quantos atalhos, ou o ícone apareceu/sumiu).
    fn pull_launchers(&mut self) -> bool {
        let found = shared().launchers.lock().unwrap().clone();
        let (apps, wing) = super::launchers::pick(&found, &self.cfg, &self.media.aumid);
        let ids = |v: &[Launcher]| v.iter().map(|l| l.id.clone()).collect::<Vec<_>>();
        let layout = apps.len() != self.apps.len() || wing.is_some() != self.wing.is_some();
        if ids(&apps) != ids(&self.apps) || wing.as_ref().map(|l| &l.id) != self.wing.as_ref().map(|l| &l.id) {
            self.apps = Arc::new(apps);
            self.wing = wing;
            self.last_sig = None;
        }
        layout
    }

    fn pull_config(&mut self) {
        let sh = shared();
        let v = sh.config_version.load(Ordering::Relaxed);
        if v == self.cfg_version {
            return;
        }
        self.cfg_version = v;
        self.cfg = sh.config();
        self.colors = system::colors(self.mtm, self.cfg.palette);
        self.window.setCollectionBehavior(behavior(&self.cfg));
        let rate = sh.sample_rate.load(Ordering::Relaxed);
        self.analyzer.configure(self.cfg.bars, rate);
        self.pull_launchers();
        self.relayout();
        self.reposition();
        self.update_visibility();
    }

    /// Esconde a ilha com "Mostrar música no notch" desligado, ou sem música
    /// com "Sempre visível" desligado.
    fn update_visibility(&mut self) {
        let want = self.cfg.show_media && (self.media.present() || self.cfg.always_show_widget || self.open_target || self.peek_until.is_some());
        if want == !self.hidden {
            return;
        }
        self.hidden = !want;
        if want {
            self.window.orderFrontRegardless();
        } else {
            self.window.orderOut(None);
        }
    }

    fn set_open(&mut self, open: bool) {
        if open == self.open_target {
            return;
        }
        let now = Instant::now();
        self.open_target = open;
        self.open.set(if open { 1.0 } else { 0.0 }, now);
        if open {
            // O card aberto já mostra a música: o "A seguir" some.
            self.peek.set(0.0, now);
            self.peek_until = None;
            self.peek_pending = None;
            self.volume.poll();
            self.last_volume_poll = now;
            // Aleatório/repetir lidos na hora (Spotify: por AppleScript).
            shared().send_media(MediaCmd::Refresh);
        } else {
            self.it = Interact::default();
            self.seek_preview = None;
        }
        self.hover_since = None;
        self.leave_since = None;
        self.update_visibility();
    }

    /// Avança os temporizadores (abrir com o mouse, fechar, "A seguir").
    fn step(&mut self) {
        let now = Instant::now();
        if let Some(t) = self.hover_since {
            if now - t >= HOVER_DELAY && self.inside && self.cfg.notch_hover {
                self.set_open(true);
            }
        }
        if let Some(t) = self.leave_since {
            if now - t >= LEAVE_DELAY && !self.inside && self.it.drag.is_none() {
                self.set_open(false);
            }
        }
        if let Some(t) = self.peek_pending {
            if !self.media.art_pending || now - t >= PEEK_ART_WAIT {
                self.peek_pending = None;
                if !self.open_target && self.media.present() {
                    self.peek.set(1.0, now);
                    self.peek_start = Some(now);
                    self.peek_until = Some(now + Duration::from_secs(self.cfg.popup_secs.max(2) as u64));
                    self.update_visibility();
                }
            }
        }
        if let Some(u) = self.peek_until {
            if now >= u && !(self.inside && !self.open_target) {
                self.peek.set(0.0, now);
                self.peek_until = None;
            }
        }
        if self.change.as_ref().is_some_and(|(_, t)| (now - *t).as_secs_f32() > CHANGE_DUR) {
            self.change = None;
        }
        if self.hint.is_some_and(|(_, t)| (now - t).as_secs_f32() > HINT_DUR) {
            self.hint = None;
        }
        if self.peek.done(now) && self.peek.to == 0.0 && self.peek_until.is_none() && self.peek_start.is_some() {
            self.peek_start = None;
            self.update_visibility();
        }
    }

    fn animating(&self, now: Instant) -> bool {
        !self.open.done(now)
            || !self.peek.done(now)
            || self.change.is_some()
            || self.hint.is_some()
            || self.hover_since.is_some()
            || self.leave_since.is_some()
            || self.peek_pending.is_some()
            || self.it.drag.is_some()
            || (self.cfg.icon_anim && self.icons.running())
    }

    /// Algo muda sozinho com o tempo (barras, indicador, tempo da música)?
    fn continuous(&self) -> bool {
        if self.hidden || !self.media.present() {
            return false;
        }
        let playing = self.media.playing;
        (playing && (self.cfg.show_visualizer || self.cfg.playing_indicator)) || self.peek_until.is_some() || (self.open_target && playing)
    }

    /// Liga o vblank no ritmo necessário (ou desliga quando nada muda).
    fn schedule(&mut self) {
        let now = Instant::now();
        let rate = if self.animating(now) {
            120
        } else if self.continuous() {
            30
        } else {
            0
        };
        let view = self.view.clone();
        self.ticker.get_or_insert_with(|| Ticker::new(&view, sel!(frame:))).set_rate(rate);
    }

    // ------------------------------------------------------------ quadro

    fn render(&mut self, force: bool) {
        if self.hidden {
            return;
        }
        // Ícones: anima o que mudou (clique ou o próprio player).
        if self.media.present() {
            let st = Playback { playing: self.media.playing, shuffle: self.media.shuffle, repeat: self.media.repeat, muted: self.volume.muted, ..Default::default() };
            self.icons.observe(&st);
        } else {
            self.icons.reset_seen();
        }
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let time = (now - self.t0).as_secs_f32();
        let open = self.open.value(now).max(0.0);
        let peek = self.peek.value(now).max(0.0);
        let p = open.max(peek);

        // Áudio -> espectro.
        let viz_on = self.cfg.show_visualizer && self.media.present();
        if viz_on {
            let sh = shared();
            let rate = sh.sample_rate.load(Ordering::Relaxed);
            self.analyzer.configure(self.cfg.bars, rate);
            {
                let ring = sh.audio.lock().unwrap();
                if ring.written != self.last_written {
                    self.last_written = ring.written;
                    self.last_audio = now;
                    ring.latest(&mut self.samples);
                }
            }
            let media_ok = !self.cfg.only_when_media || self.media.playing;
            if now - self.last_audio > Duration::from_millis(150) || !media_ok {
                self.samples.fill(0.0);
            }
            self.analyzer.update(&self.samples, dt, self.cfg.sensitivity_db);
        }
        // Sem som capturado (sem permissão / pausado): indicador em vez de barras paradas.
        let has_sound = viz_on && now - self.last_audio < Duration::from_secs(2) && self.analyzer.energy() > 0.0005;

        let l = &self.layout;
        let s = self.s;
        let (shape, _) = l.shape(open, peek);
        let region = if p <= 0.0005 {
            RectF::new(l.closed.x - 12.0 * s, 0.0, l.closed.w + 24.0 * s, l.band + 2.0 * s)
        } else {
            let m = nd::SHADOW * s + 12.0 * s;
            RectF::new(shape.x - m, 0.0, shape.w + 2.0 * m, shape.bottom() + m)
        };
        let (sw, sh_) = (l.w.ceil() as usize, l.h.ceil() as usize);
        let rx0 = (region.x.floor().max(0.0) as usize).min(sw);
        let ry0 = 0usize;
        let rx1 = (region.right().ceil().max(0.0) as usize).min(sw);
        let ry1 = (region.bottom().ceil().max(0.0) as usize).min(sh_);
        if rx1 <= rx0 || ry1 <= ry0 {
            return;
        }

        // Pula quadros iguais (sem animação e sem nada que mude sozinho).
        let sig = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            ((open * 1000.0) as i32, (peek * 1000.0) as i32, rx0, rx1, ry1).hash(&mut h);
            (self.media_version, self.cfg_version, format!("{:?}", self.it), self.seek_preview.map(|v| (v * 1000.0) as i32)).hash(&mut h);
            (self.volume.level.to_bits(), self.volume.muted, self.hint.map(|h| h.0)).hash(&mut h);
            if self.continuous() || self.animating(now) {
                ((time * 60.0) as u64).hash(&mut h);
            }
            if self.open_target {
                (self.media.position_now() as u64).hash(&mut h);
            }
            h.finish()
        };
        if !force && self.last_sig == Some(sig) {
            return;
        }
        self.last_sig = Some(sig);

        let theme = nd::theme(&self.cfg, &self.colors, &self.media);
        let colors = Colors { album: self.media.art.as_ref().map(|a| a.vibrant), ..self.colors };
        let peek_t = self.peek_start.map(|t| (now - t).as_secs_f32()).unwrap_or(0.0);
        let hint = self.hint.map(|(m, t)| {
            let e = (now - t).as_secs_f32();
            (m, (e / 0.2).min(1.0) * ((HINT_DUR - e) / 0.4).clamp(0.0, 1.0))
        });
        let change = self.change.as_ref().map(|(m, t)| (m, ((now - *t).as_secs_f32() / CHANGE_DUR).min(1.0)));
        let f = NotchFrame {
            cfg: &self.cfg,
            colors: &colors,
            media: &self.media,
            theme,
            it: Interact { fx: self.icons.fx(self.cfg.icon_anim), ..self.it },
            pb: Playback {
                playing: self.media.playing,
                shuffle: self.media.shuffle,
                repeat: self.media.repeat,
                pos: self.media.position_now(),
                seek_preview: self.seek_preview,
                volume: self.volume.level,
                muted: self.volume.muted,
            },
            time,
            viz: has_sound.then_some((&self.analyzer.values[..], &self.analyzer.peaks[..])),
            open,
            peek,
            peek_t,
            peek_total: self.cfg.popup_secs.max(2) as f32,
            hint,
            change,
            launchers: &self.apps,
            wing_app: self.wing.as_ref(),
        };
        {
            let mut cv = Canvas::new(sw, sh_, &mut self.buf);
            cv.set_clip(RectF::new(rx0 as f32, ry0 as f32, (rx1 - rx0) as f32, (ry1 - ry0) as f32));
            cv.fill_clip(0);
            nd::draw(&mut cv, &mut self.tr, l, &f);
        }
        // Copia só a região para a camada.
        let (rw, rh) = (rx1 - rx0, ry1 - ry0);
        let mut px = Vec::with_capacity(rw * rh);
        for y in ry0..ry1 {
            px.extend_from_slice(&self.buf[y * sw + rx0..y * sw + rx1]);
        }
        let Some(img) = gfx::to_cgimage(&px, rw, rh) else { return };
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        let frame = NSRect::new(CGPoint::new((rx0 as f32 / s) as f64, (ry0 as f32 / s) as f64), CGSize::new((rw as f32 / s) as f64, (rh as f32 / s) as f64));
        self.content.setFrame(frame);
        let obj: &AnyObject = unsafe { &*((&*img as *const objc2_core_graphics::CGImage).cast::<AnyObject>()) };
        unsafe { self.content.setContents(Some(obj)) };
        CATransaction::commit();
    }

    // ------------------------------------------------------------ mouse

    /// Parte visível da ilha agora (onde o mouse conta).
    fn hot_rect(&self) -> RectF {
        let now = Instant::now();
        let (shape, _) = self.layout.shape(self.open.value(now).max(0.0), self.peek.value(now).max(0.0));
        shape.inset(-4.0 * self.s)
    }

    /// Fechada ou no "A seguir", qualquer ponto da ilha abre o card (menos o
    /// ícone do app, com a ilha fechada de todo: ele abre o app).
    fn hit(&self, x: f32, y: f32, open: bool) -> Option<Hit> {
        let now = Instant::now();
        let closed = self.open.value(now) < 0.05 && self.peek.value(now) < 0.05;
        if !open && closed {
            if let Some(h @ Hit::Launch(_)) = self.layout.hit(x, y, false) {
                return Some(h);
            }
        }
        if !open && self.hot_rect().contains(x, y) {
            return Some(Hit::Info);
        }
        self.layout.hit(x, y, open)
    }

    fn mouse_move(&mut self, x: f32, y: f32) {
        let inside = !self.hidden && self.hot_rect().contains(x, y);
        if inside != !self.ignoring {
            self.ignoring = !inside;
            self.window.setIgnoresMouseEvents(!inside);
        }
        let now = Instant::now();
        let open = self.open_target && self.open.value(now) > 0.85;
        let hover = if inside { self.hit(x, y, open) } else { None };
        // O ícone do app na ilha fechada é um botão: parar nele não abre o card.
        let on_app = |h: Option<Hit>| !self.open_target && matches!(h, Some(Hit::Launch(_)));
        let (over_app, was_app) = (on_app(hover), on_app(self.it.hover));
        let hover_open = !self.open_target && self.cfg.notch_hover;
        if inside && !self.inside {
            self.leave_since = None;
            if hover_open && !over_app {
                self.hover_since = Some(now);
            }
        } else if !inside && self.inside {
            self.hover_since = None;
            if self.open_target {
                self.leave_since = Some(now);
            }
        } else if inside && hover_open {
            if over_app {
                self.hover_since = None;
            } else if was_app {
                self.hover_since = Some(now);
            }
        }
        self.inside = inside;
        if self.it.hover != hover {
            self.it.hover = hover;
        }
        if self.it.drag.is_some() {
            self.drag_to(x);
        }
        self.schedule();
        self.render(false);
    }

    fn mouse_down(&mut self, x: f32, y: f32) {
        let open = self.open_target && self.open.value(Instant::now()) > 0.85;
        let hit = self.hit(x, y, open);
        self.it.pressed = hit;
        if matches!(hit, Some(Hit::Seek) if self.media.can_seek) || hit == Some(Hit::Vol) {
            self.it.drag = hit;
            self.drag_to(x);
        }
        self.schedule();
        self.render(false);
    }

    fn drag_to(&mut self, x: f32) {
        match self.it.drag {
            Some(Hit::Seek) => {
                if let Some((x0, x1)) = self.layout.seek_track() {
                    self.seek_preview = Some(((x - x0) / (x1 - x0)).clamp(0.0, 1.0));
                }
            }
            Some(Hit::Vol) => {
                if let Some((x0, x1)) = self.layout.vol_track() {
                    self.volume.set_level(((x - x0) / (x1 - x0)).clamp(0.0, 1.0));
                }
            }
            _ => {}
        }
    }

    fn mouse_up(&mut self, x: f32, y: f32) {
        let open = self.open_target && self.open.value(Instant::now()) > 0.85;
        let hit = self.hit(x, y, open);
        let sh = shared();
        if self.it.drag == Some(Hit::Seek) {
            if let Some(p) = self.seek_preview.take() {
                if self.media.dur > 0.0 {
                    let pos = p as f64 * self.media.dur;
                    sh.send_media(MediaCmd::Seek(pos));
                    // Mostra a posição nova já (o player confirma logo depois).
                    self.media.pos = pos;
                    self.media.pos_at = Some(Instant::now());
                }
            }
        } else if self.it.pressed == hit && self.it.drag.is_none() {
            self.activate(hit);
        }
        self.it.pressed = None;
        self.it.drag = None;
        self.schedule();
        self.render(false);
    }

    fn activate(&mut self, hit: Option<Hit>) {
        let sh = shared();
        let m = &self.media;
        let not_here = if super::media::automation_denied(&m.aumid) {
            "Permita em Ajustes › Privacidade › Automação"
        } else {
            "Este player não deixa mudar isso pelo macOS"
        };
        match hit {
            Some(Hit::Info) if !self.open_target => self.set_open(true),
            Some(Hit::Play) if m.can_toggle => {
                sh.send_media(MediaCmd::PlayPause);
                self.media.playing = !self.media.playing;
            }
            Some(Hit::Prev) if m.can_prev => {
                self.icons.kick(Hit::Prev);
                sh.send_media(MediaCmd::Previous);
            }
            Some(Hit::Next) if m.can_next => {
                self.icons.kick(Hit::Next);
                sh.send_media(MediaCmd::Next);
            }
            Some(Hit::Shuffle) => {
                if m.can_shuffle {
                    sh.send_media(MediaCmd::Shuffle(!m.shuffle));
                    self.media.shuffle = !m.shuffle;
                } else {
                    self.hint = Some((crate::i18n::tr(not_here), Instant::now()));
                }
            }
            Some(Hit::Repeat) => {
                if m.can_repeat {
                    let r = (m.repeat + 1) % 3;
                    sh.send_media(MediaCmd::Repeat(r));
                    self.media.repeat = r;
                } else {
                    self.hint = Some((crate::i18n::tr(not_here), Instant::now()));
                }
            }
            Some(Hit::VolBtn) => self.volume.toggle_mute(),
            Some(Hit::Launch(i)) => {
                // Card aberto: o atalho da linha; ilha fechada: o ícone dela.
                let app = if self.open_target { self.apps.get(i as usize).cloned() } else { self.wing.clone() };
                if let Some(app) = app {
                    super::launchers::open(&app.id);
                    self.set_open(false);
                }
            }
            Some(Hit::Lyrics) => {
                sh.update_config(|c| c.lyrics.show = !c.lyrics.show);
                // Fora deste quadro (a ilha está emprestada agora): a letra e a ilha reagem.
                dispatch2::DispatchQueue::main().exec_async(super::config_changed);
            }
            _ => {}
        }
    }

    fn scroll(&mut self, dy: f32) {
        if self.open_target && self.cfg.show_volume {
            self.volume.set_level(self.volume.level + dy * 0.01);
            self.render(false);
        }
    }
}

// ---------------------------------------------------------------- vista

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    struct NotchView;

    impl NotchView {
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
            with(|n| n.mouse_move(x, y));
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, e: &NSEvent) {
            let (x, y) = self.px(e);
            with(|n| n.mouse_move(x, y));
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _e: &NSEvent) {
            with(|n| n.mouse_move(-1.0, -1.0));
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, e: &NSEvent) {
            let (x, y) = self.px(e);
            with(|n| n.mouse_down(x, y));
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, e: &NSEvent) {
            let (x, y) = self.px(e);
            with(|n| n.mouse_up(x, y));
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, e: &NSEvent) {
            // Botão direito: o mesmo menu do ícone da barra de menus.
            let p = self.convertPoint_fromView(e.locationInWindow(), None);
            statusbar::popup(self.mtm(), p, self);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, e: &NSEvent) {
            let k = if e.hasPreciseScrollingDeltas() { 0.25 } else { 2.0 };
            let dy = e.scrollingDeltaY() as f32 * k;
            with(|n| n.scroll(dy));
        }
    }
);

impl NotchView {
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
