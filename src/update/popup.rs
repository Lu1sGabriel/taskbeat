//! Pop-up da atualização, no estilo do cartão "A seguir": o mesmo fundo,
//! sombra, cantos e cores do card (com o logo do Taskbeat no lugar da capa),
//! o selo na cor de destaque, título e linha de baixo.
//!
//! Janela própria, numa thread que só existe enquanto o aviso está na tela
//! (parado, não custa nada). Fica no canto da área de trabalho perto do
//! relógio, por cima das outras janelas, sem roubar o foco. Com jogo, vídeo
//! ou apresentação em tela cheia, espera para aparecer.

use crate::config::{CardStyle, Config};
use crate::i18n::tr;
use crate::overlay::{Surface, capture_backdrop};
use crate::render::{Canvas, Colors, Image, RectF, lerp3};
use crate::shared::MediaInfo;
use crate::text::{Align, Font, TextRenderer};
use crate::ui::{self, Theme};
use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{QUNS_APP, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_APP_VIEW: u32 = WM_APP + 31;
const TIMER: usize = 1;
/// Largura do cartão (sem a margem da sombra), em px lógicos.
const CARD_W: f32 = 404.0;
/// Margem interna e logo do mesmo tamanho da capa do "A seguir" (cartão de 92 px).
const PAD: f32 = 14.0;
const LOGO: f32 = 64.0;
const NOTE_H: f32 = 17.0;
const BTN_H: f32 = 32.0;
const ENTER: f32 = 0.45;
const EXIT: f32 = 0.25;

/// O que o aviso mostra.
#[derive(Clone, Debug, PartialEq)]
pub enum View {
    /// Versão nova, já baixada e conferida. `install`: esta cópia é a
    /// instalada (senão, o botão abre a página da release).
    Offer { version: String, current: String, notes: String, install: bool },
    /// Instalando; o app reabre sozinho.
    Working { version: String },
    /// A versão nova acabou de abrir (some sozinho).
    Updated { version: String },
    /// "Procurar agora" sem versão nova (some sozinho).
    UpToDate { current: String },
    /// Erro. `page`: a página da release, para baixar à mão; sem ela, some sozinho.
    Failed { title: &'static str, reason: String, page: Option<String> },
}

/// Botão apertado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Update,
    Later,
    Skip,
    Page,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Btn(Choice),
    Link,
    Card,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bar {
    None,
    /// Barra indeterminada (instalando).
    Busy,
    /// Tempo que falta para o aviso sumir.
    Countdown,
}

/// Textos e partes de cada aviso.
struct Texts {
    badge: &'static str,
    icon: &'static str,
    title: String,
    subtitle: String,
    notes: String,
    buttons: Vec<(Choice, &'static str)>,
    link: bool,
    /// Some sozinho depois de tantos segundos.
    life: Option<f32>,
    bar: Bar,
}

fn texts(v: &View) -> Texts {
    let fill = |t: &str, x: &str| t.replace("{}", x);
    let base = |badge, icon, title: String, subtitle: String| Texts { badge, icon, title, subtitle, notes: String::new(), buttons: Vec::new(), link: false, life: None, bar: Bar::None };
    match v {
        View::Offer { version, current, notes, install } => Texts {
            notes: notes.clone(),
            buttons: vec![(Choice::Update, if *install { "Atualizar agora" } else { "Abrir página" }), (Choice::Later, "Depois"), (Choice::Skip, "Pular esta versão")],
            link: true,
            ..base("ATUALIZAÇÃO", "\u{E896}", fill(tr("Taskbeat {} disponível"), version), fill(tr("Você está na versão {}"), current))
        },
        View::Working { version } => Texts { bar: Bar::Busy, ..base("ATUALIZAÇÃO", "\u{E895}", fill(tr("Instalando o Taskbeat {}…"), version), tr("O Taskbeat reabre sozinho em instantes").into()) },
        View::Updated { version } => Texts { life: Some(6.0), bar: Bar::Countdown, ..base("ATUALIZADO", "\u{E73E}", fill(tr("Taskbeat atualizado para {}"), version), tr("Suas configurações foram mantidas.").into()) },
        View::UpToDate { current } => Texts { life: Some(5.0), bar: Bar::Countdown, ..base("ATUALIZADO", "\u{E73E}", tr("O Taskbeat está atualizado").into(), fill(tr("A versão {} é a mais recente"), current)) },
        View::Failed { title, reason, page } => Texts {
            notes: reason.clone(),
            buttons: if page.is_some() { vec![(Choice::Page, "Abrir página"), (Choice::Close, "Fechar")] } else { vec![(Choice::Close, "Fechar")] },
            life: page.is_none().then_some(10.0),
            ..base(
                "ATUALIZAÇÃO",
                "\u{E7BA}",
                tr(title).into(),
                tr(if page.is_some() { "Tente de novo mais tarde ou baixe pela página" } else { "Tente de novo mais tarde" }).into(),
            )
        },
    }
}

// ---------------------------------------------------------------- layout

/// Posições já calculadas (px físicos, dentro da superfície com a sombra).
struct Layout {
    w: f32,
    h: f32,
    card: RectF,
    logo: RectF,
    tx: f32,
    right: f32,
    notes: Vec<String>,
    notes_y: f32,
    buttons: Vec<(Choice, RectF, &'static str)>,
    link: Option<RectF>,
}

fn measure(tr: &mut TextRenderer, t: &str, font: Font, limit: f32, h: f32) -> f32 {
    // Caixa com o dobro do espaço: se passar de `limit`, não cabe.
    tr.mask(t, font, (limit * 2.0).max(1.0) as i32, h.max(1.0) as i32, Align::Left).used_width()
}

/// Quebra `text` em até `max` linhas de largura `width` (a última termina
/// em "…" se não couber; o desenho do texto já corta assim).
fn wrap(tr: &mut TextRenderer, text: &str, font: Font, width: f32, h: f32, max: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut lines = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if lines.len() + 1 == max {
            lines.push(words[i..].join(" "));
            break;
        }
        // Quantas palavras cabem nesta linha (pelo menos uma).
        let mut n = 1;
        while i + n < words.len() && measure(tr, &words[i..i + n + 1].join(" "), font, width, h) <= width {
            n += 1;
        }
        lines.push(words[i..i + n].join(" "));
        i += n;
    }
    lines
}

fn layout(tr: &mut TextRenderer, t: &Texts, s: f32) -> Layout {
    let m = ui::SHADOW_MARGIN * s;
    let (pad, cw) = (PAD * s, CARD_W * s);
    let logo = RectF::new(m + pad, m + pad, LOGO * s, LOGO * s);
    let tx = logo.right() + 14.0 * s;
    let right = m + cw - pad;
    let mut y = logo.bottom();
    let inner = cw - 2.0 * pad;
    let notes = if t.notes.is_empty() { Vec::new() } else { wrap(tr, &t.notes, Font::Caption, inner, NOTE_H * s, 3) };
    let notes_y = y + 12.0 * s;
    if !notes.is_empty() {
        y = notes_y + notes.len() as f32 * NOTE_H * s;
    }
    let mut buttons = Vec::new();
    if !t.buttons.is_empty() {
        y += 14.0 * s;
        // Larguras pelo texto; a sobra é dividida igualmente (a fila inteira).
        let gap = 8.0 * s;
        let ws: Vec<f32> = t.buttons.iter().map(|(_, l)| measure(tr, crate::i18n::tr(l), Font::CaptionStrong, inner, 18.0 * s) + 28.0 * s).collect();
        let total: f32 = ws.iter().sum::<f32>() + gap * (ws.len() - 1) as f32;
        let extra = ((inner - total) / ws.len() as f32).max(0.0);
        let scale = if total > inner { (inner - gap * (ws.len() - 1) as f32) / (total - gap * (ws.len() - 1) as f32) } else { 1.0 };
        let mut x = m + pad;
        for ((c, l), bw) in t.buttons.iter().zip(&ws) {
            let bw = bw * scale + extra;
            buttons.push((*c, RectF::new(x.round(), y.round(), bw.round(), (BTN_H * s).round()), *l));
            x += bw + gap;
        }
        y += BTN_H * s;
    }
    let card = RectF::new(m, m, cw, y + pad - m);
    let link = t.link.then(|| RectF::new(right - 24.0 * s + 4.0 * s, logo.y - 6.0 * s, 26.0 * s, 26.0 * s));
    Layout { w: (cw + 2.0 * m).ceil(), h: (card.h + 2.0 * m).ceil(), card, logo, tx, right, notes, notes_y, buttons, link }
}

impl Layout {
    fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        if let Some((c, _, _)) = self.buttons.iter().find(|(_, r, _)| r.contains(x, y)) {
            return Some(Hit::Btn(*c));
        }
        if self.link.is_some_and(|r| r.contains(x, y)) {
            return Some(Hit::Link);
        }
        self.card.contains(x, y).then_some(Hit::Card)
    }
}

// ---------------------------------------------------------------- desenho

/// Logo do Taskbeat como "capa" (o fundo do card tira dele as cores).
fn logo_image() -> Arc<Image> {
    let n = 128;
    let mut px = vec![0u32; n * n];
    {
        let mut cv = Canvas::new(n, n, &mut px);
        crate::settings::draw_logo(&mut cv, RectF::new(0.0, 0.0, n as f32, n as f32));
    }
    Arc::new(Image::new(n, n, px))
}

fn media_with(logo: &Arc<Image>) -> MediaInfo {
    MediaInfo { art: Some(logo.clone()), ..Default::default() }
}

struct Frame<'a> {
    cfg: &'a Config,
    colors: &'a Colors,
    media: &'a MediaInfo,
    th: &'a Theme,
    texts: &'a Texts,
    s: f32,
    /// Segundos desde que apareceu.
    t: f32,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    backdrop: Option<&'a Image>,
}

fn badge(cv: &mut Canvas, tr: &mut TextRenderer, (x, y): (f32, f32), label: &str, icon: &str, th: &Theme, s: f32) {
    let lh = 19.0 * s;
    let lw = tr.mask(label, Font::Label, (180.0 * s) as i32, lh as i32, Align::Left).used_width();
    let b = RectF::new(x, y, (27.0 * s + lw + 10.0 * s).round(), lh);
    cv.glow(b, lh / 2.0, 10.0 * s, 2.0 * s, th.accent, 0.38);
    let (c0, c1) = (lerp3(th.accent, [1.0; 3], 0.12), lerp3(th.accent, [0.0; 3], 0.10));
    let (by0, bh) = (b.y, b.h);
    cv.fill_rrect_ca(b.x, b.y, b.right(), b.bottom(), lh / 2.0, |_, y| (lerp3(c0, c1, ((y - by0) / bh).clamp(0.0, 1.0)), 1.0));
    cv.stroke_round(b, lh / 2.0, s, [1.0; 3], 0.14);
    ui::glyph(cv, tr, RectF::new(b.x + 5.0 * s, b.y, 17.0 * s, lh), icon, Font::IconSmall, th.on_accent, 1.0);
    ui::text(cv, tr, label, Font::Label, b.x + 25.5 * s, b.y, lw + 4.0 * s, lh, th.on_accent, 1.0);
}

/// O que não muda com o mouse: fundo, logo, selo, textos.
fn draw_static(cv: &mut Canvas, tr: &mut TextRenderer, l: &Layout, f: &Frame) {
    let s = f.s;
    let radius = f.cfg.corners.radius() * s;
    ui::card_background(cv, l.card, radius, f.cfg, f.colors, f.media, f.th, s, f.backdrop);
    let r = (radius * 0.55).max(6.0 * s);
    if let Some(img) = &f.media.art {
        ui::art_shadow(cv, l.logo, r, img, f.th, s);
    }
    crate::settings::draw_logo(cv, l.logo);
    let top = l.logo.y;
    let tw = l.right - l.tx;
    badge(cv, tr, (l.tx, top - 1.5 * s), crate::i18n::tr(f.texts.badge), f.texts.icon, f.th, s);
    ui::text(cv, tr, &f.texts.title, Font::CardTitle, l.tx, top + 20.0 * s, tw, 22.0 * s, f.th.fg, 1.0);
    ui::text(cv, tr, &f.texts.subtitle, Font::Subtitle, l.tx, top + 42.0 * s, tw, 17.0 * s, f.th.fg, 0.72);
    for (i, line) in l.notes.iter().enumerate() {
        let y = l.notes_y + i as f32 * NOTE_H * s;
        ui::text(cv, tr, line, Font::Caption, l.card.x + PAD * s, y, l.card.w - 2.0 * PAD * s, NOTE_H * s, f.th.fg, 0.86);
    }
}

/// Botões (com o mouse em cima ou apertados) e barras animadas.
fn draw_dynamic(cv: &mut Canvas, tr: &mut TextRenderer, l: &Layout, f: &Frame) {
    let s = f.s;
    let th = f.th;
    let br = (f.cfg.corners.radius() * 0.45).clamp(4.0, 10.0) * s;
    for (i, (c, r, label)) in l.buttons.iter().enumerate() {
        let hot = f.hover == Some(Hit::Btn(*c));
        let down = hot && f.pressed == Some(Hit::Btn(*c));
        let label = crate::i18n::tr(label);
        if i == 0 {
            let base = th.accent;
            let col = if down { lerp3(base, [0.0; 3], 0.14) } else if hot { lerp3(base, [1.0; 3], 0.12) } else { base };
            cv.glow(*r, br, 8.0 * s, 1.5 * s, base, if hot { 0.30 } else { 0.20 });
            cv.fill_round(*r, br, col, 1.0);
            cv.stroke_round(*r, br, s, [1.0; 3], 0.14);
            ui::text_center(cv, tr, label, Font::CaptionStrong, *r, th.on_accent, 1.0);
        } else {
            let a = if down { 0.06 } else if hot { 0.17 } else { 0.10 };
            cv.fill_round(*r, br, th.fg, a);
            cv.stroke_round(*r, br, s, th.fg, 0.10);
            ui::text_center(cv, tr, label, Font::CaptionStrong, *r, th.fg, 0.95);
        }
    }
    if let Some(r) = l.link {
        let hot = f.hover == Some(Hit::Link);
        if hot {
            ui::hover_bg(cv, r, 6.0 * s, th, f.pressed == Some(Hit::Link));
        }
        ui::glyph(cv, tr, r, "\u{E8A7}", Font::IconSmall, th.fg, if hot { 1.0 } else { 0.72 });
    }
    // Barrinha embaixo do título, encostada na base do logo (como no "A seguir").
    let (x0, x1) = (l.tx, l.right);
    let by = l.logo.bottom() - 3.0 * s;
    let track = if th.dark { 0.18 } else { 0.12 };
    match f.texts.bar {
        Bar::None => {}
        Bar::Busy => {
            cv.fill_rrect(x0, by, x1, by + 3.0 * s, 1.5 * s, track, th.fg);
            let w = (x1 - x0) * 0.32;
            let p = (f.t * 0.75).fract();
            let a = (x0 - w + (x1 - x0 + w) * ui::ease_move(p)).max(x0);
            let b = (x0 + (x1 - x0 + w) * ui::ease_move(p)).min(x1);
            if b > a + 1.0 {
                cv.fill_rrect(a, by, b, by + 3.0 * s, 1.5 * s, 0.95, th.accent);
            }
        }
        Bar::Countdown => {
            let life = f.texts.life.unwrap_or(5.0);
            let hold = ((f.t - ENTER) / (life - ENTER - EXIT).max(0.5)).clamp(0.0, 1.0);
            cv.fill_rrect(x0, by, x1, by + 3.0 * s, 1.5 * s, track, th.fg);
            let xe = x0 + (x1 - x0) * (1.0 - hold);
            if xe > x0 + 1.0 {
                cv.fill_rrect(x0, by, xe, by + 3.0 * s, 1.5 * s, 0.95, th.accent);
            }
        }
    }
}

// ---------------------------------------------------------------- janela

struct Inner {
    view: Mutex<View>,
    hwnd: AtomicIsize,
}

/// Aviso aberto. Soltar (`drop`) não fecha: os avisos que somem sozinhos
/// seguem até o fim.
pub struct Popup {
    inner: Arc<Inner>,
    rx: Receiver<Choice>,
}

impl Popup {
    /// Espera um botão. Janela fechada (sumiu sozinha, por exemplo): `Close`.
    pub fn wait(&self) -> Choice {
        self.rx.recv().unwrap_or(Choice::Close)
    }

    /// Troca o que o aviso mostra.
    pub fn set(&self, v: View) {
        *self.inner.view.lock().unwrap() = v;
        self.post(WM_APP_VIEW);
    }

    fn post(&self, msg: u32) {
        let h = self.inner.hwnd.load(Ordering::Relaxed);
        if h != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(h as _)), msg, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// Abre o aviso (numa thread própria) e devolve o controle dele.
pub fn open(cfg: &Config, view: View) -> Popup {
    let inner = Arc::new(Inner { view: Mutex::new(view), hwnd: AtomicIsize::new(0) });
    let (tx, rx) = channel();
    let (i2, cfg) = (inner.clone(), cfg.clone());
    let _ = std::thread::Builder::new().name("update-popup".into()).spawn(move || run(i2, cfg, tx));
    Popup { inner, rx }
}

/// Aviso que some sozinho, sem esperar resposta.
pub fn toast(cfg: &Config, view: View) {
    drop(open(cfg, view));
}

/// Jogo, vídeo ou apresentação em tela cheia (o Windows também segura as
/// notificações nesses casos).
fn screen_busy() -> bool {
    matches!(unsafe { SHQueryUserNotificationState() }, Ok(q) if q == QUNS_BUSY || q == QUNS_RUNNING_D3D_FULL_SCREEN || q == QUNS_PRESENTATION_MODE || q == QUNS_APP)
}

struct St {
    hwnd: HWND,
    inner: Arc<Inner>,
    tx: Sender<Choice>,
    cfg: Config,
    colors: Colors,
    logo: Arc<Image>,
    media: MediaInfo,
    view: View,
    texts: Texts,
    layout: Layout,
    s: f32,
    /// Canto da superfície na tela e de que lado ela entra (1 = de baixo).
    x: i32,
    y: i32,
    dir: f32,
    surface: Option<Surface>,
    /// Fundo, logo e textos já desenhados (os quadros só copiam).
    base: Vec<u32>,
    backdrop: Option<Image>,
    text: TextRenderer,
    shown_at: Instant,
    closing_at: Option<Instant>,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    tracking: bool,
    timer: bool,
    visible: bool,
}

thread_local! {
    static ST: RefCell<Option<St>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut St) -> R) -> Option<R> {
    ST.with(|s| s.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

/// Monitor da barra de tarefas: área de trabalho, área total e escala.
fn monitor() -> (RECT, RECT, f32) {
    unsafe {
        let tray = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).unwrap_or_default();
        let hmon = MonitorFromWindow(tray, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(hmon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        (mi.rcWork, mi.rcMonitor, dx.max(96) as f32 / 96.0)
    }
}

impl St {
    /// Recalcula textos, tamanho e posição (no canto perto do relógio).
    fn relayout(&mut self) {
        let (work, mon, s) = monitor();
        self.s = s;
        self.text.set_scale(s);
        self.texts = texts(&self.view);
        self.layout = layout(&mut self.text, &self.texts, s);
        let m = ui::SHADOW_MARGIN * s;
        let gap = 12.0 * s;
        let l = &self.layout;
        self.x = (work.right as f32 - l.card.w - gap - m).round() as i32;
        let taskbar_on_top = work.top > mon.top;
        if taskbar_on_top {
            self.y = (work.top as f32 + gap - m).round() as i32;
            self.dir = -1.0;
        } else {
            self.y = (work.bottom as f32 - l.card.h - gap - m).round() as i32;
            self.dir = 1.0;
        }
        self.backdrop = None;
        if self.cfg.card_style == CardStyle::Glass {
            if self.visible {
                // Captura o que está atrás sem o próprio aviso.
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                    let _ = DwmFlush();
                }
                self.visible = false;
            }
            let c = l.card;
            self.backdrop = capture_backdrop((self.x + c.x.round() as i32, self.y + c.y.round() as i32, c.w.round() as i32, c.h.round() as i32), ui::GLASS_BLUR * s);
        }
        self.media = media_with(&self.logo);
        let th = ui::card_theme(&self.cfg, &self.colors, &self.media);
        let (w, h) = (l.w as i32, l.h as i32);
        let mut base = vec![0u32; (w * h) as usize];
        {
            let mut cv = Canvas::new(w as usize, h as usize, &mut base);
            let f = Frame { cfg: &self.cfg, colors: &self.colors, media: &self.media, th: &th, texts: &self.texts, s, t: 0.0, hover: None, pressed: None, backdrop: self.backdrop.as_ref() };
            draw_static(&mut cv, &mut self.text, &self.layout, &f);
        }
        self.base = base;
    }

    fn t(&self) -> f32 {
        self.shown_at.elapsed().as_secs_f32()
    }

    /// Precisa de quadros (entrando, saindo, barra animada)?
    fn animating(&self) -> bool {
        self.closing_at.is_some() || self.t() < ENTER || self.texts.bar != Bar::None
    }

    fn frame(&mut self) {
        let t = self.t();
        let mut e = ui::ease_enter((t / ENTER).clamp(0.0, 1.0));
        if let Some(c) = self.closing_at {
            let k = (c.elapsed().as_secs_f32() / EXIT).clamp(0.0, 1.0);
            e *= 1.0 - ui::ease_exit(k);
            if k >= 1.0 {
                unsafe {
                    let _ = DestroyWindow(self.hwnd);
                }
                return;
            }
        } else if self.texts.life.is_some_and(|life| t >= life - EXIT) && self.hover.is_none() {
            self.start_close();
        }
        let (w, h) = (self.layout.w as i32, self.layout.h as i32);
        Surface::ensure(&mut self.surface, w, h);
        let Some(mut surf) = self.surface.take() else { return };
        {
            let mut cv = surf.canvas();
            cv.copy_from(0, 0, &self.base, w as usize, h as usize, 0);
            let th = ui::card_theme(&self.cfg, &self.colors, &self.media);
            let f = Frame { cfg: &self.cfg, colors: &self.colors, media: &self.media, th: &th, texts: &self.texts, s: self.s, t, hover: self.hover, pressed: self.pressed, backdrop: None };
            draw_dynamic(&mut cv, &mut self.text, &self.layout, &f);
        }
        let slide = (1.0 - e) * 26.0 * self.s * self.dir;
        surf.present_at(self.hwnd, self.x, self.y as f32 + slide, e);
        self.surface = Some(surf);
        if !self.visible {
            unsafe {
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
            }
            self.visible = true;
        }
        self.update_timer();
    }

    fn update_timer(&mut self) {
        let want = self.animating();
        if want != self.timer {
            unsafe {
                if want {
                    SetTimer(Some(self.hwnd), TIMER, 15, None);
                } else {
                    let _ = KillTimer(Some(self.hwnd), TIMER);
                }
            }
            self.timer = want;
        }
    }

    fn start_close(&mut self) {
        if self.closing_at.is_none() {
            self.closing_at = Some(Instant::now());
            self.update_timer();
        }
    }

    fn click(&mut self, hit: Hit) {
        match hit {
            Hit::Btn(c) => {
                let _ = self.tx.send(c);
                if matches!(c, Choice::Later | Choice::Skip | Choice::Close) {
                    self.start_close();
                }
            }
            Hit::Link => {
                let _ = self.tx.send(Choice::Page);
            }
            // Os avisos que somem sozinhos fecham com um clique.
            Hit::Card if self.texts.life.is_some() => {
                let _ = self.tx.send(Choice::Close);
                self.start_close();
            }
            Hit::Card => {}
        }
    }
}

fn xy(lp: LPARAM) -> (f32, f32) {
    ((lp.0 & 0xffff) as i16 as f32, ((lp.0 >> 16) & 0xffff) as i16 as f32)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            WM_TIMER => {
                with(|s| s.frame());
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                with(|s| {
                    if !s.tracking {
                        let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                        let _ = TrackMouseEvent(&mut tme);
                        s.tracking = true;
                    }
                    let h = s.layout.hit(x, y);
                    if h != s.hover {
                        s.hover = h;
                        s.frame();
                    }
                });
                return LRESULT(0);
            }
            WM_MOUSELEAVE => {
                with(|s| {
                    s.tracking = false;
                    s.hover = None;
                    s.frame();
                });
                return LRESULT(0);
            }
            WM_SETCURSOR => {
                let hand = with(|s| matches!(s.hover, Some(Hit::Btn(_) | Hit::Link))).unwrap_or(false);
                let _ = SetCursor(LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }).ok());
                return LRESULT(1);
            }
            WM_LBUTTONDOWN => {
                SetCapture(hwnd);
                let (x, y) = xy(lp);
                with(|s| {
                    s.pressed = s.layout.hit(x, y);
                    s.hover = s.pressed;
                    s.frame();
                });
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                let (x, y) = xy(lp);
                with(|s| {
                    let h = s.layout.hit(x, y);
                    let p = s.pressed.take();
                    if let Some(hit) = h
                        && h == p
                    {
                        s.click(hit);
                    }
                    s.frame();
                });
                return LRESULT(0);
            }
            WM_APP_VIEW => {
                with(|s| {
                    let v = s.inner.view.lock().unwrap().clone();
                    if v != s.view {
                        s.view = v;
                        s.hover = None;
                        s.pressed = None;
                        // Já está na tela: conta o tempo do aviso novo de agora, sem entrar de novo.
                        s.shown_at = Instant::now().checked_sub(Duration::from_secs_f32(ENTER)).unwrap_or_else(Instant::now);
                        s.relayout();
                        s.frame();
                    }
                });
                return LRESULT(0);
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

fn run(inner: Arc<Inner>, cfg: Config, tx: Sender<Choice>) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    crate::i18n::apply(cfg.language);
    // Tela cheia: espera (os avisos que somem sozinhos desistem em 1 minuto).
    let toast = texts(&inner.view.lock().unwrap()).life.is_some();
    let waited = Instant::now();
    while screen_busy() {
        if toast && waited.elapsed() > Duration::from_secs(60) {
            return;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = module.into();
        let class = w!("TaskbeatUpdate");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
        let Ok(hwnd) = CreateWindowExW(ex, class, w!("Taskbeat"), WS_POPUP, 0, 0, 1, 1, None, None, Some(hinst), None) else { return };
        inner.hwnd.store(hwnd.0 as isize, Ordering::Relaxed);
        let view = inner.view.lock().unwrap().clone();
        let logo = logo_image();
        let mut st = St {
            hwnd,
            inner: inner.clone(),
            tx,
            colors: crate::system::colors(cfg.palette),
            cfg,
            media: media_with(&logo),
            logo,
            texts: texts(&view),
            view,
            layout: Layout { w: 1.0, h: 1.0, card: RectF::new(0.0, 0.0, 1.0, 1.0), logo: RectF::new(0.0, 0.0, 1.0, 1.0), tx: 0.0, right: 0.0, notes: Vec::new(), notes_y: 0.0, buttons: Vec::new(), link: None },
            s: 1.0,
            x: 0,
            y: 0,
            dir: 1.0,
            surface: None,
            base: Vec::new(),
            backdrop: None,
            text: TextRenderer::new(),
            shown_at: Instant::now(),
            closing_at: None,
            hover: None,
            pressed: None,
            tracking: false,
            timer: false,
            visible: false,
        };
        st.relayout();
        st.shown_at = Instant::now();
        ST.with(|s| *s.borrow_mut() = Some(st));
        with(|s| s.frame());
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        inner.hwnd.store(0, Ordering::Relaxed);
        ST.with(|s| *s.borrow_mut() = None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn views() -> Vec<(&'static str, View)> {
        let notes = crate::update::summary("A primeira versão do **Taskbeat** para **Windows e Mac juntos**. No Windows, o visual ficou bem mais bonito e chegou o **mixer de volume**; no Mac, sai a versão estável depois de três betas.", 280);
        vec![
            ("offer", View::Offer { version: "1.4.0".into(), current: "1.3.0".into(), notes, install: true }),
            ("offer-portable", View::Offer { version: "1.4.0".into(), current: "1.3.0".into(), notes: String::new(), install: false }),
            ("working", View::Working { version: "1.4.0".into() }),
            ("updated", View::Updated { version: "1.4.0".into() }),
            ("uptodate", View::UpToDate { current: "1.3.0".into() }),
            ("failed", View::Failed { title: "Não foi possível atualizar", reason: "Taskbeat-Setup-1.4.0.exe: assinatura inválida; não instala".into(), page: Some("https://github.com/Lu1sGabriel/taskbeat/releases/tag/v1.4.0".into()) }),
            ("check-failed", View::Failed { title: "Não foi possível procurar atualizações", reason: "envio: tempo esgotado".into(), page: None }),
        ]
    }

    #[test]
    fn textos_traduzidos() {
        let mut missing = Vec::new();
        for (_, v) in views() {
            let t = texts(&v);
            let mut keys = vec![t.badge];
            keys.extend(t.buttons.iter().map(|(_, l)| *l));
            if let View::Failed { title, .. } = v {
                keys.push(title);
            }
            for k in keys {
                if !crate::i18n::has(k) {
                    missing.push(k);
                }
            }
        }
        for k in [
            "Taskbeat {} disponível",
            "Você está na versão {}",
            "Instalando o Taskbeat {}…",
            "O Taskbeat reabre sozinho em instantes",
            "Taskbeat atualizado para {}",
            "O Taskbeat está atualizado",
            "A versão {} é a mais recente",
            "Tente de novo mais tarde ou baixe pela página",
            "Tente de novo mais tarde",
        ] {
            if !crate::i18n::has(k) {
                missing.push(k);
            }
        }
        assert!(missing.is_empty(), "sem tradução: {missing:?}");
    }

    #[test]
    fn layout_cabe_no_cartao() {
        let mut tr = TextRenderer::new();
        for s in [1.0, 1.5, 2.0] {
            tr.set_scale(s);
            for (name, v) in views() {
                let t = texts(&v);
                let l = layout(&mut tr, &t, s);
                for (_, r, _) in &l.buttons {
                    assert!(r.x >= l.card.x && r.right() <= l.card.right() + 0.5 && r.bottom() <= l.card.bottom(), "{name} {s}: botão fora do cartão");
                }
                assert!(l.notes.len() <= 3, "{name}");
                assert_eq!(l.buttons.is_empty(), t.buttons.is_empty());
                // Clique no primeiro botão acha o botão.
                if let Some((c, r, _)) = l.buttons.first() {
                    assert_eq!(l.hit(r.cx(), r.cy()), Some(Hit::Btn(*c)));
                }
                assert_eq!(l.hit(0.0, 0.0), None);
            }
        }
    }

    /// Desenha cada aviso em PNG para conferir o visual:
    /// `cargo test popup_shots -- --ignored --nocapture` (grava em target/update-shots).
    #[test]
    #[ignore]
    fn popup_shots() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("update-shots");
        let mut tr = TextRenderer::new();
        let logo = logo_image();
        let media = media_with(&logo);
        let colors = Colors { palette: crate::config::Palette::Album, accent: [0.93, 0.32, 0.62], accent_light: [0.98, 0.62, 0.80], light_taskbar: false, album: None };
        for (style, sname) in [(CardStyle::Artwork, "capa"), (CardStyle::AlbumColor, "cor")] {
            let cfg = Config { card_style: style, ..Config::default() };
            let s = 2.0;
            tr.set_scale(s);
            for (name, v) in views() {
                let t = texts(&v);
                let l = layout(&mut tr, &t, s);
                let th = ui::card_theme(&cfg, &colors, &media);
                let (w, h) = (l.w as usize, l.h as usize);
                let mut px = vec![0u32; w * h];
                let mut cv = Canvas::new(w, h, &mut px);
                let hover = l.buttons.first().map(|(c, _, _)| Hit::Btn(*c));
                let f = Frame { cfg: &cfg, colors: &colors, media: &media, th: &th, texts: &t, s, t: 2.0, hover, pressed: None, backdrop: None };
                draw_static(&mut cv, &mut tr, &l, &f);
                draw_dynamic(&mut cv, &mut tr, &l, &f);
                crate::png::save(&dir.join(format!("{sname}-{name}.png")), w, h, &px);
            }
        }
    }
}
