//! Menu suspenso no visual do app (Fluent 2 / Windows 11): acrílico, cantos
//! arredondados, sombra, ícones, interruptores animados e cabeçalho com a
//! música. Usado no botão direito do ícone da bandeja e nas listas da tela de
//! configurações (no lugar do menu antigo do Win32).
//!
//! `popup` é modal, como o `TrackPopupMenu`: roda um loop de mensagens próprio
//! (as outras janelas da thread continuam sendo atendidas) e devolve o item
//! escolhido. Interruptores e botões de mídia não fecham o menu: chamam
//! `on_keep`, que devolve os itens atualizados.

use crate::i18n::tr;
use crate::overlay::{Surface, capture_backdrop};
use crate::render::{Canvas, Image, RectF, Rgb, lerp3, on_color};

use crate::text::{Align, Font, G_NEXT, G_PAUSE, G_PLAY, G_PREV, TextRenderer};
use crate::ui;
use crate::vsync::{Pacer, Target};
use std::cell::RefCell;
use std::sync::Arc;
use std::time::Instant;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetCapture, ReleaseCapture, SetCapture, SetFocus, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_SPACE, VK_UP};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::System::Threading::INFINITE;
use windows::core::w;

/// Botões de mídia do cabeçalho (não fecham o menu).
pub const ID_PREV: u32 = 0xFF01;
pub const ID_PLAY: u32 = 0xFF02;
pub const ID_NEXT: u32 = 0xFF03;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// Executa e fecha.
    Action,
    /// Interruptor: alterna sem fechar o menu.
    Toggle(bool),
    /// Opção de uma lista (a escolhida fica marcada); fecha.
    Radio(bool),
}

pub struct Item {
    pub id: u32,
    /// Glifo da fonte de ícones ("" = sem ícone).
    pub icon: &'static str,
    pub text: String,
    pub kind: Kind,
    pub enabled: bool,
}

/// Cabeçalho "tocando agora": capa, título, artista e botões de mídia.
pub struct Header {
    pub art: Option<Arc<Image>>,
    pub title: String,
    pub subtitle: String,
    pub playing: bool,
    pub can_prev: bool,
    pub can_next: bool,
    pub can_toggle: bool,
}

pub enum Entry {
    Header(Header),
    Item(Item),
    Separator,
}

impl Entry {
    pub fn action(id: u32, icon: &'static str, text: &str) -> Entry {
        Entry::Item(Item { id, icon, text: text.to_string(), kind: Kind::Action, enabled: true })
    }
    pub fn toggle(id: u32, icon: &'static str, text: &str, on: bool, enabled: bool) -> Entry {
        Entry::Item(Item { id, icon, text: text.to_string(), kind: Kind::Toggle(on), enabled })
    }
    pub fn radio(id: u32, text: &str, selected: bool) -> Entry {
        Entry::Item(Item { id, icon: "", text: text.to_string(), kind: Kind::Radio(selected), enabled: true })
    }
}

/// Onde o menu aparece.
pub enum Anchor {
    /// Junto ao ponteiro (bandeja): acima dele, alinhado à direita, se couber.
    Cursor(POINT),
    /// Logo abaixo de um controle (lista suspensa), com pelo menos a largura dele.
    Below(RECT),
}

pub struct Options {
    pub dark: bool,
    pub accent: Rgb,
    /// Largura mínima em px lógicos.
    pub min_width: f32,
}

// ---------------------------------------------------------------- layout

const MARGIN: f32 = 18.0;
const ITEM_H: f32 = 36.0;
const SEP_H: f32 = 9.0;
const HEADER_H: f32 = 68.0;
const PAD: f32 = 4.0;

#[derive(Clone, Copy)]
struct Row {
    entry: usize,
    rect: RectF,
}

struct Layout {
    w: f32,
    h: f32,
    card: RectF,
    rows: Vec<Row>,
    /// Botões de mídia do cabeçalho: (id, retângulo).
    media: Vec<(u32, RectF)>,
    has_icons: bool,
}

fn layout(entries: &[Entry], s: f32, min_w: f32, text: &mut TextRenderer) -> Layout {
    let m = MARGIN * s;
    let has_icons = entries.iter().any(|e| matches!(e, Entry::Item(i) if !i.icon.is_empty()));
    let has_toggles = entries.iter().any(|e| matches!(e, Entry::Item(i) if matches!(i.kind, Kind::Toggle(_))));
    let mut w = min_w * s;
    for e in entries {
        let need = match e {
            Entry::Item(i) => {
                let tw = text.mask(&i.text, Font::Body, (600.0 * s) as i32, (20.0 * s) as i32, Align::Left).used_width();
                (if has_icons { 44.0 } else { 18.0 }) * s + tw + if has_toggles { 70.0 * s } else { 22.0 * s }
            }
            Entry::Header(_) => 300.0 * s,
            Entry::Separator => 0.0,
        };
        w = w.max(need);
    }
    let w = w.min(460.0 * s).round();
    let mut y = m + PAD * s;
    let mut rows = Vec::new();
    let mut media = Vec::new();
    for (k, e) in entries.iter().enumerate() {
        let h = match e {
            Entry::Header(_) => HEADER_H,
            Entry::Item(_) => ITEM_H,
            Entry::Separator => SEP_H,
        } * s;
        let r = RectF::new(m, y.round(), w, h.round());
        if let Entry::Header(_) = e {
            let b = 32.0 * s;
            let cy = r.cy();
            for (j, id) in [ID_PREV, ID_PLAY, ID_NEXT].iter().enumerate() {
                let x = r.right() - 12.0 * s - (3 - j) as f32 * b;
                media.push((*id, RectF::new(x.round(), (cy - b / 2.0).round(), b, b)));
            }
        }
        rows.push(Row { entry: k, rect: r });
        y += h;
    }
    let card_h = (y + PAD * s - m).round();
    Layout { w: w + 2.0 * m, h: card_h + 2.0 * m, card: RectF::new(m, m, w, card_h), rows, media, has_icons }
}

// ---------------------------------------------------------------- estado

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hot {
    Row(usize),
    Media(u32),
}

struct State {
    hwnd: HWND,
    entries: Vec<Entry>,
    opts: Options,
    s: f32,
    l: Layout,
    /// Posição final do cartão na tela.
    x: i32,
    y: i32,
    /// +1: abre de baixo para cima (desliza para cima); -1: o contrário.
    dir: f32,
    hot: Option<Hot>,
    pressed: Option<Hot>,
    /// Interruptores: posição animada da bolinha (0..1), por entrada.
    knobs: Vec<f32>,
    opened: Instant,
    last: Instant,
    backdrop: Option<Arc<Image>>,
    surface: Option<Surface>,
    result: Option<Option<u32>>,
    keyboard: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static TEXT: RefCell<Option<TextRenderer>> = const { RefCell::new(None) };
    /// Clique "para manter aberto" esperando o `on_keep` do loop modal.
    static KEEP: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

fn with_text<R>(s: f32, f: impl FnOnce(&mut TextRenderer) -> R) -> R {
    TEXT.with(|t| {
        let mut g = t.borrow_mut();
        let tr = g.get_or_insert_with(|| {
            let mut t = TextRenderer::new();
            t.set_capacity(300);
            t
        });
        tr.set_scale(s);
        f(tr)
    })
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|st| st.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn knob_targets(entries: &[Entry]) -> Vec<f32> {
    entries.iter().map(|e| if let Entry::Item(Item { kind: Kind::Toggle(true), .. }) = e { 1.0 } else { 0.0 }).collect()
}

fn selectable(e: &Entry) -> bool {
    matches!(e, Entry::Item(i) if i.enabled)
}

impl State {
    fn hit(&self, x: f32, y: f32) -> Option<Hot> {
        if let Some((id, _)) = self.l.media.iter().find(|(_, r)| r.contains(x, y)) {
            return Some(Hot::Media(*id));
        }
        let r = self.l.rows.iter().position(|r| r.rect.contains(x, y) && x >= self.l.card.x && x < self.l.card.right())?;
        selectable(&self.entries[self.l.rows[r].entry]).then_some(Hot::Row(r))
    }

    fn animating(&self) -> bool {
        let targets = knob_targets(&self.entries);
        self.opened.elapsed().as_secs_f32() < 0.26 || self.knobs.iter().zip(&targets).any(|(a, b)| (a - b).abs() > 0.002)
    }

    fn step(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().clamp(0.0, 0.05);
        self.last = now;
        let targets = knob_targets(&self.entries);
        for (k, t) in self.knobs.iter_mut().zip(targets) {
            let d = t - *k;
            *k = if d.abs() < 0.002 { t } else { *k + d * (1.0 - (-dt * 22.0).exp()) };
        }
    }

    fn render(&mut self) {
        let s = self.s;
        let (w, h) = (self.l.w.round() as i32, self.l.h.round() as i32);
        Surface::ensure(&mut self.surface, w, h);
        let Some(mut surf) = self.surface.take() else { return };
        let t = self.opened.elapsed().as_secs_f32();
        let e = ui::ease_enter(t / 0.24);
        {
            let mut cv = surf.canvas();
            cv.clear();
            with_text(s, |tr| self.draw(&mut cv, tr));
        }
        let slide = (1.0 - e) * 10.0 * s * self.dir;
        surf.present_at(self.hwnd, self.x - (MARGIN * s) as i32, self.y as f32 - MARGIN * s + slide, ui::ease_enter((t / 0.16).min(1.0)));
        self.surface = Some(surf);
    }

    fn draw(&self, cv: &mut Canvas, tr: &mut TextRenderer) {
        let s = self.s;
        let dark = self.opts.dark;
        let fg: Rgb = if dark { [1.0; 3] } else { [0.0; 3] };
        let accent = self.opts.accent;
        let card = self.l.card;
        let radius = 8.0 * s;
        cv.drop_shadow(card, radius, 16.0 * s, 5.0 * s, if dark { 0.45 } else { 0.20 });
        let base: Rgb = if dark { [0.17, 0.17, 0.18] } else { [0.975, 0.975, 0.98] };
        match &self.backdrop {
            Some(b) => ui::acrylic(cv, card, radius, b, base, if dark { 0.82 } else { 0.78 }),
            None => cv.fill_round(card, radius, base, 0.98),
        }
        cv.stroke_round(card, radius, s, fg, if dark { 0.11 } else { 0.08 });
        // Brilho fino no topo (elevação).
        cv.fill_rrect_ca(card.x, card.y, card.right(), card.bottom(), radius, |_, y| ([1.0; 3], if y < card.y + 1.5 * s { 0.06 } else { 0.0 }));

        let tx0 = if self.l.has_icons { 44.0 * s } else { 18.0 * s };
        for (ri, row) in self.l.rows.iter().enumerate() {
            let r = row.rect;
            match &self.entries[row.entry] {
                Entry::Separator => {
                    cv.fill_rrect(r.x, r.cy().round(), r.right(), r.cy().round() + s, 0.0, if dark { 0.09 } else { 0.08 }, fg);
                }
                Entry::Header(hd) => self.draw_header(cv, tr, r, hd, fg, accent),
                Entry::Item(it) => {
                    let hl = RectF::new(r.x + PAD * s, r.y + 2.0 * s, r.w - 2.0 * PAD * s, r.h - 4.0 * s);
                    let hot = self.hot == Some(Hot::Row(ri));
                    let selected = matches!(it.kind, Kind::Radio(true));
                    if hot && it.enabled {
                        let a = if self.pressed == Some(Hot::Row(ri)) { 0.035 } else if dark { 0.075 } else { 0.055 };
                        cv.fill_round(hl, 4.0 * s, fg, a);
                    } else if selected {
                        cv.fill_round(hl, 4.0 * s, fg, if dark { 0.05 } else { 0.04 });
                    }
                    if selected {
                        cv.fill_round(RectF::new(hl.x, hl.cy() - 8.0 * s, 3.0 * s, 16.0 * s), 1.5 * s, accent, 1.0);
                    }
                    let a = if it.enabled { 1.0 } else { 0.38 };
                    if !it.icon.is_empty() {
                        ui::glyph(cv, tr, RectF::new(r.x + 12.0 * s, r.y, 22.0 * s, r.h), it.icon, Font::IconMd, fg, 0.88 * a);
                    }
                    let right_pad = if matches!(it.kind, Kind::Toggle(_)) { 64.0 * s } else { 14.0 * s };
                    ui::text(cv, tr, &it.text, if selected { Font::BodyStrong } else { Font::Body }, r.x + tx0, r.y, r.w - tx0 - right_pad, r.h, fg, 0.95 * a);
                    if let Kind::Toggle(_) = it.kind {
                        let k = self.knobs.get(row.entry).copied().unwrap_or(0.0);
                        let sw = RectF::new(r.right() - 16.0 * s - 36.0 * s, r.cy() - 9.0 * s, 36.0 * s, 18.0 * s);
                        mini_switch(cv, sw, k, fg, accent, dark, s, a);
                    }
                }
            }
        }
    }

    fn draw_header(&self, cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, hd: &Header, fg: Rgb, accent: Rgb) {
        let s = self.s;
        let art = RectF::new(r.x + 12.0 * s, r.cy() - 22.0 * s, 44.0 * s, 44.0 * s);
        match &hd.art {
            Some(a) => {
                cv.drop_shadow(art, 6.0 * s, 8.0 * s, 2.0 * s, 0.25);
                cv.draw_image_cover(art, a, 6.0 * s, 1.0);
                cv.stroke_round(art, 6.0 * s, s, [1.0; 3], 0.10);
            }
            None => {
                cv.fill_round(art, 6.0 * s, accent, 0.22);
                ui::glyph(cv, tr, art, crate::text::G_MUSIC, Font::Icon, fg, 0.9);
            }
        }
        let tx = art.right() + 12.0 * s;
        let tw = self.l.media.first().map(|m| m.1.x).unwrap_or(r.right()) - tx - 8.0 * s;
        ui::text(cv, tr, &hd.title, Font::BodyStrong, tx, r.cy() - 20.0 * s, tw, 20.0 * s, fg, 1.0);
        ui::text(cv, tr, &hd.subtitle, Font::Caption, tx, r.cy() + 1.0 * s, tw, 18.0 * s, fg, 0.66);
        for (id, b) in &self.l.media {
            let hot = self.hot == Some(Hot::Media(*id));
            let enabled = match *id {
                ID_PREV => hd.can_prev,
                ID_NEXT => hd.can_next,
                _ => hd.can_toggle,
            };
            let a = if enabled { 1.0 } else { 0.35 };
            if *id == ID_PLAY {
                let k = if self.pressed == Some(Hot::Media(*id)) { 0.92 } else if hot { 1.06 } else { 1.0 };
                let d = 30.0 * s * k;
                let c = RectF::new(b.cx() - d / 2.0, b.cy() - d / 2.0, d, d);
                cv.fill_circle(c.cx(), c.cy(), d / 2.0, accent, a);
                ui::glyph(cv, tr, c, if hd.playing { G_PAUSE } else { G_PLAY }, Font::IconSmall, on_color(accent), a);
            } else {
                if hot && enabled {
                    cv.fill_circle(b.cx(), b.cy(), 15.0 * s, fg, if self.opts.dark { 0.08 } else { 0.06 });
                }
                ui::glyph(cv, tr, *b, if *id == ID_PREV { G_PREV } else { G_NEXT }, Font::IconSmall, fg, 0.9 * a);
            }
        }
        // Linha sutil separando o cabeçalho dos itens.
        let _ = lerp3;
    }

    fn activate(&mut self, hot: Hot) {
        match hot {
            Hot::Media(id) => KEEP.with(|k| k.borrow_mut().push(id)),
            Hot::Row(r) => {
                let Entry::Item(it) = &self.entries[self.l.rows[r].entry] else { return };
                if !it.enabled {
                    return;
                }
                match it.kind {
                    Kind::Toggle(_) => KEEP.with(|k| k.borrow_mut().push(it.id)),
                    _ => self.result = Some(Some(it.id)),
                }
            }
        }
    }

    fn move_key(&mut self, step: i32) {
        self.keyboard = true;
        let n = self.l.rows.len() as i32;
        let mut i = match self.hot {
            Some(Hot::Row(r)) => r as i32,
            _ => if step > 0 { -1 } else { n },
        };
        for _ in 0..n {
            i = (i + step).rem_euclid(n);
            if selectable(&self.entries[self.l.rows[i as usize].entry]) {
                self.hot = Some(Hot::Row(i as usize));
                return;
            }
        }
    }
}

/// Interruptor pequeno (36×18) com a bolinha em `k` (0 = desligado, 1 = ligado).
fn mini_switch(cv: &mut Canvas, r: RectF, k: f32, fg: Rgb, accent: Rgb, dark: bool, s: f32, a: f32) {
    let rad = r.h / 2.0;
    let on = ui::ease_move(k);
    cv.stroke_round(r, rad, s, fg, 0.55 * (1.0 - on) * a);
    cv.fill_round(r, rad, accent, on * a);
    let x = r.x + rad + (r.w - 2.0 * rad) * on;
    let knob = lerp3(fg, on_color(accent), on);
    let _ = dark;
    cv.fill_circle(x, r.cy(), 5.0 * s + 0.5 * s * on, knob, (0.8 + 0.2 * on) * a);
}

// ---------------------------------------------------------------- janela

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let xy = || ((lp.0 & 0xffff) as i16 as f32, ((lp.0 >> 16) & 0xffff) as i16 as f32);
        match msg {
            WM_MOUSEACTIVATE => return LRESULT(MA_ACTIVATE as isize),
            WM_MOUSEMOVE => {
                let (x, y) = xy();
                with_state(|st| {
                    let h = st.hit(x, y);
                    if h != st.hot && (h.is_some() || !st.keyboard) {
                        st.hot = h;
                        st.keyboard = false;
                        st.render();
                    }
                });
                return LRESULT(0);
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let (x, y) = xy();
                with_state(|st| {
                    // Fora do cartão: fecha (o clique é "gasto" aqui, como nos menus do Windows).
                    if !st.l.card.contains(x, y) {
                        st.result = Some(None);
                        return;
                    }
                    st.pressed = st.hit(x, y);
                    st.render();
                });
                return LRESULT(0);
            }
            WM_LBUTTONUP | WM_RBUTTONUP => {
                let (x, y) = xy();
                with_state(|st| {
                    let h = st.hit(x, y);
                    if h.is_some() && h == st.pressed {
                        st.activate(h.unwrap());
                    }
                    st.pressed = None;
                    st.render();
                });
                return LRESULT(0);
            }
            WM_KEYDOWN => {
                let vk = wp.0 as u16;
                with_state(|st| {
                    if vk == VK_ESCAPE.0 {
                        st.result = Some(None);
                    } else if vk == VK_DOWN.0 {
                        st.move_key(1);
                    } else if vk == VK_UP.0 {
                        st.move_key(-1);
                    } else if vk == VK_RETURN.0 || vk == VK_SPACE.0 {
                        if let Some(h) = st.hot {
                            st.activate(h);
                        }
                    }
                    st.render();
                });
                return LRESULT(0);
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => {
                with_state(|st| {
                    if st.result.is_none() {
                        st.result = Some(None);
                    }
                });
            }
            WM_CAPTURECHANGED => {
                if HWND(lp.0 as *mut _) != hwnd {
                    with_state(|st| {
                        if st.result.is_none() {
                            st.result = Some(None);
                        }
                    });
                }
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

fn class() -> windows::core::PCWSTR {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let name = w!("TaskbeatMenu");
    ONCE.call_once(|| unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None).map(Into::into).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: name,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc);
    });
    name
}

fn monitor_info(pt: POINT) -> (RECT, f32) {
    unsafe {
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        (mi.rcWork, dx.max(96) as f32 / 96.0)
    }
}

/// Mostra o menu e espera a escolha. `on_keep(id)`: interruptor ou botão de
/// mídia clicado (o menu continua aberto); devolve os itens atualizados.
pub fn popup(owner: HWND, entries: Vec<Entry>, anchor: Anchor, opts: Options, on_keep: &mut dyn FnMut(u32) -> Option<Vec<Entry>>) -> Option<u32> {
    if STATE.with(|s| s.borrow().is_some()) {
        return None;
    }
    let pt = match &anchor {
        Anchor::Cursor(p) => *p,
        Anchor::Below(r) => POINT { x: r.left, y: r.bottom },
    };
    let (work, s) = monitor_info(pt);
    let min_w = match &anchor {
        Anchor::Below(r) => opts.min_width.max((r.right - r.left) as f32 / s),
        Anchor::Cursor(_) => opts.min_width,
    };
    let l = with_text(s, |tr| layout(&entries, s, min_w, tr));
    let (cw, ch) = (l.card.w as i32, l.card.h as i32);
    let gap = (4.0 * s) as i32;
    let (mut x, mut y, dir) = match &anchor {
        Anchor::Cursor(p) => {
            // Acima e à esquerda do ponteiro (barra embaixo); abaixo se não couber.
            let up = p.y - ch - gap >= work.top;
            (p.x - cw, if up { p.y - ch - gap } else { p.y + gap }, if up { 1.0 } else { -1.0 })
        }
        Anchor::Below(r) => {
            let down = r.bottom + gap + ch <= work.bottom;
            (r.left, if down { r.bottom + gap } else { r.top - gap - ch }, if down { -1.0 } else { 1.0 })
        }
    };
    x = x.clamp(work.left + gap, (work.right - cw - gap).max(work.left));
    y = y.clamp(work.top + gap, (work.bottom - ch - gap).max(work.top));

    let backdrop = capture_backdrop((x, y, cw, ch), 30.0 * s).map(Arc::new);
    let hwnd = unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None).map(Into::into).unwrap_or_default();
        match CreateWindowExW(WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST, class(), w!("Taskbeat"), WS_POPUP, 0, 0, 1, 1, Some(owner), None, Some(hinst), None) {
            Ok(h) => h,
            Err(_) => return None,
        }
    };
    let knobs = knob_targets(&entries);
    let now = Instant::now();
    STATE.with(|st| {
        *st.borrow_mut() = Some(State {
            hwnd,
            entries,
            opts,
            s,
            l,
            x,
            y,
            dir,
            hot: None,
            pressed: None,
            knobs,
            opened: now,
            last: now,
            backdrop,
            surface: None,
            result: None,
            keyboard: false,
        });
    });
    with_state(|st| st.render());
    let pacer = Pacer::new(Target::Event);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        SetCapture(hwnd);
    }
    if let Some(p) = &pacer {
        p.set_monitor(unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) });
    }

    let mut quit = None;
    let mut msg = MSG::default();
    loop {
        if with_state(|st| st.result.is_some()).unwrap_or(true) {
            break;
        }
        let animating = with_state(|st| st.animating()).unwrap_or(false);
        if let Some(p) = &pacer {
            p.set_active(animating);
        }
        let handles: Vec<HANDLE> = pacer.as_ref().map(|p| vec![p.event()]).unwrap_or_default();
        let timeout = if animating && pacer.is_none() { 16 } else { INFINITE };
        let r = unsafe { MsgWaitForMultipleObjects(if handles.is_empty() { None } else { Some(&handles) }, false, timeout, QS_ALLINPUT) };
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    quit = Some(msg.wParam.0 as i32);
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if quit.is_some() {
            break;
        }
        // Interruptor / mídia: atualiza os itens sem fechar.
        let keep: Vec<u32> = KEEP.with(|k| std::mem::take(&mut *k.borrow_mut()));
        for id in keep {
            if let Some(new) = on_keep(id) {
                with_state(|st| {
                    let knobs = std::mem::take(&mut st.knobs);
                    st.entries = new;
                    st.knobs = knob_targets(&st.entries);
                    // Mantém a posição atual das bolinhas para animar até o novo estado.
                    for (k, old) in st.knobs.iter_mut().zip(knobs) {
                        *k = old;
                    }
                });
            }
        }
        let _ = r;
        with_state(|st| {
            st.step();
            st.render();
        });
    }
    let result = STATE.with(|st| st.borrow_mut().take()).and_then(|st| st.result).flatten();
    unsafe {
        if GetCapture() == hwnd {
            let _ = ReleaseCapture();
        }
        let _ = DestroyWindow(hwnd);
        if let Some(code) = quit {
            PostQuitMessage(code);
        }
    }
    drop(pacer);
    let _ = tr;
    result
}
