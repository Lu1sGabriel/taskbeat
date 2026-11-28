//! Aviso de Caps Lock no Mac: o mesmo cartão do Windows (`ui::draw_osd`) numa
//! janela que não recebe cliques, por cima de tudo (inclusive tela cheia).
//!
//! O estado da tecla é lido do sistema (CGEventSourceFlagsState) num
//! temporizador curto e barato: não precisa da permissão de Acessibilidade nem
//! de Monitoramento de Entrada, e nada do que é digitado é lido.
//! Material acrílico: desfoque de verdade do macOS (NSVisualEffectView) por
//! baixo do cartão, sem capturar a tela.

use super::compat::Ticker;
use super::{gfx, shared, system, target};
use crate::config::{Config, OsdAnim, OsdMaterial, OsdPosition, OsdShape};
use crate::render::{Canvas, Colors, Image, RectF};
use crate::text::TextRenderer;
use crate::ui;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSBackingStoreType, NSColor, NSPanel, NSScreen, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGSize};
use objc2_core_graphics::{CGEventFlags, CGEventSource, CGEventSourceStateID};
use objc2_foundation::{NSRect, NSTimer};
use objc2_quartz_core::{CALayer, CATransaction};
use std::cell::RefCell;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Acima de quase tudo (inclusive da barra de menus e da ilha do notch).
const LEVEL: isize = 24 + 4;
/// Leitura do Caps Lock (8 vezes por segundo).
const POLL: f64 = 0.125;

struct Osd {
    mtm: MainThreadMarker,
    window: Retained<NSPanel>,
    root: Retained<NSView>,
    blur: Retained<NSVisualEffectView>,
    image: Retained<CALayer>,
    ticker: Ticker,
    _timer: Retained<NSTimer>,
    tr: TextRenderer,
    s: f32,
    cfg: Config,
    cfg_version: u64,
    colors: Colors,
    /// Aviso na tela: quando começou, se a tecla ligou, posição final e lado de entrada.
    start: Option<Instant>,
    on: bool,
    origin: CGPoint,
    dir: f64,
    caps: bool,
    /// Demonstração (CI): fica na tela.
    hold: bool,
    buf: Vec<u32>,
    tmp: Vec<u32>,
    clear: Image,
}

thread_local! {
    static O: RefCell<Option<Osd>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Osd) -> R) -> Option<R> {
    O.with(|o| o.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn caps_on() -> bool {
    CGEventSource::flags_state(CGEventSourceStateID::CombinedSessionState).contains(CGEventFlags::MaskAlphaShift)
}

pub fn start(mtm: MainThreadMarker) {
    let sh = shared();
    let cfg = sh.config();
    let rect = NSRect::new(CGPoint::new(0.0, 0.0), CGSize::new(300.0, 120.0));
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
    window.setIgnoresMouseEvents(true);
    window.setHidesOnDeactivate(false);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::Stationary | NSWindowCollectionBehavior::IgnoresCycle | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );

    let root = NSView::initWithFrame(NSView::alloc(mtm), rect);
    root.setWantsLayer(true);
    let blur = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), rect);
    blur.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    blur.setState(NSVisualEffectState::Active);
    blur.setWantsLayer(true);
    blur.setHidden(true);
    root.addSubview(&blur);
    // Camada com o desenho do cartão (por cima do desfoque).
    let holder = NSView::initWithFrame(NSView::alloc(mtm), rect);
    let image = CALayer::new();
    holder.setLayer(Some(&image));
    holder.setWantsLayer(true);
    holder.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable);
    root.addSubview(&holder);
    window.setContentView(Some(&root));

    let timer = unsafe { NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(POLL, &target(), sel!(osdTick:), None, true) };
    timer.setTolerance(0.04);

    let s = NSScreen::mainScreen(mtm).map(|sc| sc.backingScaleFactor() as f32).unwrap_or(2.0);
    let mut tr = TextRenderer::new();
    tr.set_scale(s);
    // A camada é nossa (não é a que o AppKit cria para a vista): a escala da
    // tela não vem sozinha. Os pixels do quadro já estão nela.
    image.setContentsScale(s as f64);
    let ticker = Ticker::new(&root, sel!(osdFrame:));
    let o = Osd {
        ticker,
        mtm,
        window,
        root,
        blur,
        image,
        _timer: timer,
        tr,
        s,
        colors: system::colors(mtm, cfg.palette),
        cfg,
        cfg_version: sh.config_version.load(Ordering::Relaxed),
        start: None,
        on: false,
        origin: CGPoint::new(0.0, 0.0),
        dir: 1.0,
        caps: caps_on(),
        hold: false,
        buf: Vec::new(),
        tmp: Vec::new(),
        clear: Image::plain(1, 1, vec![0]),
    };
    O.with(|c| *c.borrow_mut() = Some(o));
    if std::env::var_os("TV_DEMO_CAPS").is_some() {
        with(|o| {
            o.hold = true;
            o.show(true);
        });
    }
}

/// "Mostrar agora" das configurações: mostra com o estado atual da tecla.
pub fn test() {
    with(|o| {
        o.pull_config();
        let on = caps_on();
        o.show(on);
    });
}

/// Temporizador: o Caps Lock mudou?
pub fn tick() {
    with(|o| {
        o.pull_config();
        if !o.cfg.caps_lock_osd {
            return;
        }
        let now = caps_on();
        if now != o.caps {
            o.caps = now;
            if now || o.cfg.osd_show_off {
                o.show(now);
            }
        }
    });
}

/// Quadro do vblank enquanto o aviso está na tela.
pub fn frame() {
    with(|o| o.render());
}

impl Osd {
    fn pull_config(&mut self) {
        let sh = shared();
        let v = sh.config_version.load(Ordering::Relaxed);
        if v != self.cfg_version {
            self.cfg_version = v;
            self.cfg = sh.config();
            self.colors = system::colors(self.mtm, self.cfg.palette);
        }
    }

    fn duration(&self) -> f32 {
        if self.hold { 3600.0 } else { (self.cfg.osd_duration as f32 / 10.0).max(0.8) }
    }

    fn show(&mut self, on: bool) {
        let Some(screen) = NSScreen::mainScreen(self.mtm).or_else(|| NSScreen::screens(self.mtm).firstObject()) else { return };
        let s = screen.backingScaleFactor() as f32;
        if (s - self.s).abs() > 0.01 {
            self.s = s;
            self.tr.set_scale(s);
            self.image.setContentsScale(s as f64);
        }
        // Já estava na tela: troca o estado sem entrar de novo.
        let shown = self.start.is_some_and(|t| t.elapsed().as_secs_f32() > 0.3);
        self.start = Some(if shown { crate::clock::back(Instant::now(), Duration::from_millis(350)) } else { Instant::now() });
        self.on = on;

        let (w, h) = ui::osd_size(s, &self.cfg);
        let (wp, hp) = ((w / s) as f64, (h / s) as f64);
        let m = (ui::SHADOW_MARGIN) as f64;
        let (f, v) = (screen.frame(), screen.visibleFrame());
        let cx = f.origin.x + (f.size.width - wp) / 2.0;
        let top = f.origin.y + f.size.height;
        let (x, y, dir) = match self.cfg.osd_position {
            OsdPosition::Bottom => (cx, v.origin.y + 60.0 - m, 1.0),
            OsdPosition::Center => (cx, f.origin.y + (f.size.height - hp) / 2.0, 1.0),
            OsdPosition::Top => (cx, (v.origin.y + v.size.height) - 40.0 + m - hp, -1.0),
            OsdPosition::AbovePlayer => {
                // Logo abaixo da ilha do notch.
                let band = super::notch::band_points().unwrap_or(32.0);
                (cx, top - band - 12.0 + m - hp, -1.0)
            }
        };
        let x = (x + self.cfg.osd_offset_x as f64).clamp(f.origin.x - m, f.origin.x + f.size.width - wp + m);
        let y = (y - self.cfg.osd_offset_y as f64).clamp(f.origin.y - m, top - hp + m);
        self.origin = CGPoint::new(x.round(), y.round());
        self.dir = dir;
        self.window.setFrame_display(NSRect::new(self.origin, CGSize::new(wp, hp)), false);
        self.root.setFrame(NSRect::new(CGPoint::new(0.0, 0.0), CGSize::new(wp, hp)));
        self.buf = vec![0; (w.ceil() * h.ceil()) as usize];

        // Desfoque do sistema só no material acrílico, no tema do aviso.
        let acrylic = self.cfg.osd_material == OsdMaterial::Acrylic;
        self.blur.setHidden(!acrylic);
        if acrylic {
            let dark = ui::osd_theme(&self.cfg, &self.colors).dark;
            self.blur.setMaterial(if dark { NSVisualEffectMaterial::HUDWindow } else { NSVisualEffectMaterial::Popover });
            let name = unsafe { if dark { NSAppearanceNameDarkAqua } else { NSAppearanceNameAqua } };
            self.blur.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
        }

        self.window.setAlphaValue(0.0);
        self.window.orderFrontRegardless();
        self.ticker.set_rate(120);
        self.render();
    }

    fn hide(&mut self) {
        self.start = None;
        self.window.orderOut(None);
        self.ticker.set_rate(0);
        self.buf = Vec::new();
        self.tmp = Vec::new();
    }

    fn render(&mut self) {
        let Some(start) = self.start else {
            self.ticker.set_rate(0);
            return;
        };
        let t = start.elapsed().as_secs_f32();
        let secs = self.duration();
        if t >= secs {
            self.hide();
            return;
        }
        let e = if t < 0.3 {
            ui::ease_enter(t / 0.3)
        } else if t > secs - 0.22 {
            1.0 - ui::ease_exit((t - (secs - 0.22)) / 0.22)
        } else {
            1.0
        };
        let s = self.s;
        let (slide, zoom) = match self.cfg.osd_anim {
            OsdAnim::Slide => ((1.0 - e) * 18.0, 1.0),
            OsdAnim::Zoom => (0.0, 0.88 + 0.12 * e),
            OsdAnim::Fade => (0.0, 1.0),
        };
        let (w, h) = ui::osd_size(s, &self.cfg);
        let (sw, sh) = (w.ceil() as usize, h.ceil() as usize);
        if self.buf.len() != sw * sh {
            self.buf = vec![0; sw * sh];
        }
        let th = ui::osd_theme(&self.cfg, &self.colors);
        let acrylic = self.cfg.osd_material == OsdMaterial::Acrylic;
        // Acrílico: a tinta fica translúcida sobre o desfoque do sistema.
        let backdrop = acrylic.then_some(&self.clear);
        let key = crate::i18n::tr("Caps Lock");
        if zoom < 0.999 {
            self.tmp.clear();
            self.tmp.resize(sw * sh, 0);
            ui::draw_osd(&mut Canvas::new(sw, sh, &mut self.tmp), &mut self.tr, &self.cfg, &self.colors, &th, s, key, false, self.on, t, backdrop);
            let img = Image::plain(sw, sh, std::mem::take(&mut self.tmp));
            let (zw, zh) = (sw as f32 * zoom, sh as f32 * zoom);
            let mut cv = Canvas::new(sw, sh, &mut self.buf);
            cv.clear();
            cv.draw_image(RectF::new((sw as f32 - zw) / 2.0, (sh as f32 - zh) / 2.0, zw, zh), &img, 0.0, 1.0);
            self.tmp = img.px;
        } else {
            let mut cv = Canvas::new(sw, sh, &mut self.buf);
            cv.clear();
            ui::draw_osd(&mut cv, &mut self.tr, &self.cfg, &self.colors, &th, s, key, false, self.on, t, backdrop);
        }
        let Some(img) = gfx::to_cgimage(&self.buf, sw, sh) else { return };

        CATransaction::begin();
        CATransaction::setDisableActions(true);
        let obj: &AnyObject = unsafe { &*((&*img as *const objc2_core_graphics::CGImage).cast::<AnyObject>()) };
        unsafe { self.image.setContents(Some(obj)) };
        if acrylic {
            // Desfoque do tamanho do cartão (acompanha o zoom).
            let (cw, ch) = ui::osd_card(s, &self.cfg);
            let k = s * self.cfg.osd_size.factor();
            let radius = match self.cfg.osd_shape {
                OsdShape::Pill => ch / 2.0,
                OsdShape::Rounded => self.cfg.corners.radius() * k,
                OsdShape::Square => 5.0 * k,
            };
            let (cwp, chp) = ((cw * zoom / s) as f64, (ch * zoom / s) as f64);
            let (wp, hp) = ((w / s) as f64, (h / s) as f64);
            self.blur.setFrame(NSRect::new(CGPoint::new((wp - cwp) / 2.0, (hp - chp) / 2.0), CGSize::new(cwp, chp)));
            if let Some(layer) = self.blur.layer() {
                layer.setCornerRadius((radius * zoom / s) as f64);
                layer.setMasksToBounds(true);
            }
        }
        CATransaction::commit();
        let y = self.origin.y - self.dir * slide as f64;
        self.window.setFrameOrigin(CGPoint::new(self.origin.x, y));
        self.window.setAlphaValue(e as f64);
    }
}
