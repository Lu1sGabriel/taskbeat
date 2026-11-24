//! Desenho dos componentes visuais (funções puras, sem estado de janela):
//! widget da barra, card expandido, cartão "A seguir" e aviso de Caps/Num Lock.
//! Usado pelas janelas reais (overlay.rs) e pelas pré-visualizações ao vivo da
//! tela de configurações (settings.rs).
//!
//! Referências de design: Fluent 2 (raios 4/8, elevação com sombra suave,
//! Segoe UI Variable) e cards de "now playing" populares no Dribbble (capa
//! desfocada como fundo, cor extraída da capa, vidro, play circular).

use crate::config::{CardLayout, CardStyle, Config, OsdIcon, OsdMaterial, OsdShape, OsdSize, Shape, ShuffleRepeat, TaskbarPlay, ThemeMode, WidgetShape};
use crate::render::*;
use crate::shared::MediaInfo;
use crate::text::*;
use std::cell::RefCell;
use std::sync::Arc;

/// Margem (lógica) em volta dos cards para caber a sombra.
pub const SHADOW_MARGIN: f32 = 26.0;
/// Desfoque (px lógicos) do fundo acrílico do card e do "A seguir".
pub const GLASS_BLUR: f32 = 30.0;
/// Cartão "A seguir" (sem a margem da sombra), em px lógicos.
const UPNEXT_W: f32 = 360.0;
const UPNEXT_H: f32 = 92.0;

/// Desfoque (px físicos) do fundo acrílico do aviso de teclas.
pub fn osd_blur_px(cfg: &Config, s: f32) -> f32 {
    (4.0 + cfg.osd_blur as f32 * 0.4) * s
}

/// O componente usa fundo acrílico (precisa da imagem do que está atrás)?
pub fn card_wants_backdrop(cfg: &Config) -> bool {
    cfg.card_style == CardStyle::Glass
}

pub fn osd_wants_backdrop(cfg: &Config) -> bool {
    cfg.osd_material == OsdMaterial::Acrylic
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Info,
    Shuffle,
    Prev,
    Play,
    Next,
    Repeat,
    Seek,
    VolBtn,
    Vol,
}

/// Cores de um componente já resolvidas (tema + cor de destaque).
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub fg: Rgb,
    pub accent: Rgb,
    pub on_accent: Rgb,
    pub dark: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Interact {
    pub hover: Option<Hit>,
    pub pressed: Option<Hit>,
    pub drag: Option<Hit>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Playback {
    pub playing: bool,
    pub shuffle: bool,
    pub repeat: u8,
    pub pos: f64,
    pub seek_preview: Option<f32>,
    pub volume: f32,
    pub muted: bool,
}

fn system_dark(colors: &Colors) -> bool {
    !colors.light_taskbar
}

fn card_is_dark(cfg: &Config, colors: &Colors) -> bool {
    match cfg.theme {
        ThemeMode::Auto => system_dark(colors),
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
    }
}

/// Fundo do estilo "Cor do álbum".
fn album_bg(cfg: &Config, colors: &Colors, media: &MediaInfo) -> Rgb {
    let base = media.art.as_ref().map(|a| a.vibrant).unwrap_or(colors.accent);
    if card_is_dark(cfg, colors) { lerp3(readable(base, true), [0.0; 3], 0.52) } else { lerp3(base, [1.0; 3], 0.72) }
}

/// Cores do card expandido / "A seguir" / aviso de teclas.
pub fn card_theme(cfg: &Config, colors: &Colors, media: &MediaInfo) -> Theme {
    let has_art = media.art.is_some();
    let dark = match cfg.card_style {
        CardStyle::Artwork if has_art => true,
        _ => card_is_dark(cfg, colors),
    };
    let fg = if dark { [1.0; 3] } else { [0.08, 0.08, 0.09] };
    if cfg.card_style == CardStyle::AlbumColor {
        let bg = album_bg(cfg, colors, media);
        return Theme { fg, accent: fg, on_accent: readable(bg, !dark), dark };
    }
    let accent = match (cfg.accent_source, &media.art) {
        (crate::config::AccentSource::Album, Some(a)) => readable(a.vibrant, dark),
        _ => {
            if dark {
                colors.accent_light
            } else {
                colors.accent
            }
        }
    };
    Theme { fg, accent, on_accent: on_color(accent), dark }
}

/// Cores do widget na barra (texto segue a barra; destaque segue a capa ou o Windows).
pub fn widget_theme(cfg: &Config, colors: &Colors, media: &MediaInfo) -> Theme {
    let dark = !colors.light_taskbar;
    let fg = colors.foreground();
    let accent = match (cfg.accent_source, &media.art) {
        (crate::config::AccentSource::Album, Some(a)) => readable(a.vibrant, dark),
        _ => {
            if dark {
                colors.accent_light
            } else {
                colors.accent
            }
        }
    };
    Theme { fg, accent, on_accent: on_color(accent), dark }
}

// ---------------------------------------------------------------- utilidades

thread_local! {
    static BLUR_CACHE: RefCell<Option<(usize, Arc<Image>)>> = const { RefCell::new(None) };
}

/// Capa desfocada (em cache por imagem).
fn blurred(img: &Arc<Image>) -> Arc<Image> {
    let key = Arc::as_ptr(img) as usize;
    BLUR_CACHE.with(|c| {
        if let Some((k, b)) = c.borrow().as_ref() {
            if *k == key {
                return b.clone();
            }
        }
        // Desfoca uma versão pequena (rápido); o desfoque esconde a ampliação.
        let small = img.mip(if img.w >= 128 { 2 } else { 1 });
        let b = Arc::new(small.blurred((small.w / 14).max(2)));
        *c.borrow_mut() = Some((key, b.clone()));
        b
    })
}

pub fn glyph(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, g: &str, font: Font, c: Rgb, a: f32) {
    let m = tr.mask(g, font, r.w.round().max(1.0) as i32, r.h.round().max(1.0) as i32, Align::Center);
    cv.blit_mask(r.x.round() as i32, r.y.round() as i32, &m, c, a);
}

pub fn text(cv: &mut Canvas, tr: &mut TextRenderer, t: &str, font: Font, x: f32, y: f32, w: f32, h: f32, c: Rgb, a: f32) {
    if t.is_empty() {
        return;
    }
    let m = tr.mask(t, font, w.max(1.0) as i32, h.max(1.0) as i32, Align::Left);
    cv.blit_mask(x.round() as i32, y.round() as i32, &m, c, a);
}

pub fn text_center(cv: &mut Canvas, tr: &mut TextRenderer, t: &str, font: Font, r: RectF, c: Rgb, a: f32) {
    let m = tr.mask(t, font, r.w.max(1.0) as i32, r.h.max(1.0) as i32, Align::Center);
    cv.blit_mask(r.x.round() as i32, r.y.round() as i32, &m, c, a);
}

pub fn text_right(cv: &mut Canvas, tr: &mut TextRenderer, t: &str, font: Font, right: f32, y: f32, h: f32, c: Rgb, a: f32, s: f32) {
    let m = tr.mask(t, font, (360.0 * s) as i32, h.max(1.0) as i32, Align::Left);
    let used = m.used_width();
    cv.blit_mask((right - used).round() as i32, y.round() as i32, &m, c, a);
}

/// Texto que rola quando não cabe (letreiro), com bordas esmaecidas.
fn marquee(cv: &mut Canvas, tr: &mut TextRenderer, t: &str, font: Font, r: RectF, c: Rgb, a: f32, s: f32, time: f32, enabled: bool) {
    if t.is_empty() {
        return;
    }
    let wide = tr.mask(t, font, (1600.0 * s) as i32, r.h.max(1.0) as i32, Align::Left);
    let used = wide.used_width();
    if used <= r.w || !enabled {
        let m = tr.mask(t, font, r.w.max(1.0) as i32, r.h.max(1.0) as i32, Align::Left);
        cv.blit_mask(r.x.round() as i32, r.y.round() as i32, &m, c, a);
        return;
    }
    let (pause, speed, gap) = (2.5f32, 30.0 * s, 42.0 * s);
    let cycle = pause + (used + gap) / speed;
    let phase = time.rem_euclid(cycle);
    let off = if phase < pause { 0.0 } else { (phase - pause) * speed };
    let fade = 12.0 * s;
    let fl = if off > 0.5 { fade } else { 0.0 };
    for k in [0.0, used + gap] {
        cv.blit_mask_faded(r.x - off + k, r.y.round() as i32, &wide, c, a, r, fl, fade);
    }
}

fn hover_bg(cv: &mut Canvas, r: RectF, radius: f32, th: &Theme, pressed: bool) {
    let a = if pressed { 0.06 } else if th.dark { 0.10 } else { 0.07 };
    cv.fill_round(r, radius, th.fg, a);
}

/// Capa com o formato escolhido (ou ícone do app / nota musical).
/// `source`: `None` esconde o app de origem; `Some(badge)` mostra o ícone do app
/// (sobre a capa quando `badge > 0`, ou no lugar dela quando não há capa).
pub fn draw_art(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, media: &MediaInfo, th: &Theme, shape: Shape, s: f32, source: Option<f32>) {
    let badge = source.unwrap_or(0.0);
    let icon = if source.is_some() { &media.icon } else { &None };
    let radius = match shape {
        Shape::Circle => r.w / 2.0,
        Shape::Rounded => (r.w * 0.16).max(3.0 * s),
        Shape::Square => 2.0 * s,
    };
    match (&media.art, icon) {
        (Some(a), icon) => {
            cv.draw_image_cover(r, a, radius, 1.0);
            cv.stroke_round(r, radius, s, th.fg, 0.08);
            if let (Some(i), true) = (icon, badge > 0.0) {
                let br = RectF::new(r.right() - badge + 3.0 * s, r.bottom() - badge + 3.0 * s, badge, badge);
                cv.fill_round(br.inset(-1.2 * s), 4.0 * s, if th.dark { [0.1; 3] } else { [1.0; 3] }, 0.95);
                cv.draw_image(br, i, 3.0 * s, 1.0);
            }
        }
        (None, Some(i)) => {
            cv.fill_round(r, radius, th.accent, 0.18);
            let k = r.w * 0.6;
            cv.draw_image(RectF::new(r.cx() - k / 2.0, r.cy() - k / 2.0, k, k), i, 3.0 * s, 1.0);
        }
        (None, None) => {
            cv.fill_round(r, radius, th.accent, 0.28);
            glyph(cv, tr, r, G_MUSIC, if r.w > 64.0 * s { Font::IconXl } else { Font::Icon }, th.fg, 0.9);
        }
    }
}

/// Mini equalizador animado ("tocando agora").
fn eq_indicator(cv: &mut Canvas, r: RectF, color: Rgb, playing: bool, time: f32, s: f32, alpha: f32) {
    let bw = 2.6 * s;
    let gap = 1.8 * s;
    let total = 3.0 * bw + 2.0 * gap;
    let x0 = r.cx() - total / 2.0;
    let base = r.cy() + 6.0 * s;
    for i in 0..3 {
        let h = if playing {
            let ph = time * (7.0 + i as f32 * 1.7) + i as f32 * 2.1;
            (0.3 + 0.7 * (0.5 + 0.5 * ph.sin())) * 12.0 * s
        } else {
            3.0 * s
        };
        let x = x0 + i as f32 * (bw + gap);
        cv.fill_rrect(x, base - h, x + bw, base, bw / 2.0, alpha, |_, _| color);
    }
}

fn play_button(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, size: f32, shape: Shape, th: &Theme, it: &Interact, playing: bool, enabled: bool, s: f32) {
    let hot = it.hover == Some(Hit::Play);
    let pressed = it.pressed == Some(Hit::Play);
    let k = size * if pressed { 0.94 } else if hot { 1.05 } else { 1.0 };
    let b = RectF::new(r.cx() - k / 2.0, r.cy() - k / 2.0, k, k);
    let radius = match shape {
        Shape::Circle => k / 2.0,
        Shape::Rounded => 8.0 * s,
        Shape::Square => 4.0 * s,
    };
    let en = if enabled { 1.0 } else { 0.35 };
    if enabled {
        cv.drop_shadow(b, radius, 8.0 * s, 2.0 * s, if th.dark { 0.35 } else { 0.18 });
    }
    cv.fill_round(b, radius, th.accent, en);
    let font = if k > 30.0 * s { Font::Icon } else { Font::IconSmall };
    glyph(cv, tr, b, if playing { G_PAUSE } else { G_PLAY }, font, th.on_accent, en);
}

fn toggle_glyph(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, g: &str, on: bool, enabled: bool, th: &Theme, s: f32) {
    let c = if on && enabled { th.accent } else { th.fg };
    let a = if !enabled { 0.3 } else if on { 1.0 } else { 0.8 };
    glyph(cv, tr, r, g, Font::IconSmall, c, a);
    if on && enabled {
        cv.fill_circle(r.cx(), r.bottom() - 5.0 * s, 1.7 * s, th.accent, 1.0);
    }
}

fn slider(cv: &mut Canvas, x0: f32, x1: f32, cy: f32, t: f32, th: &Theme, hot: bool, thumb: bool, s: f32) {
    let h = 4.0 * s;
    cv.fill_rrect(x0, cy - h / 2.0, x1, cy + h / 2.0, h / 2.0, if th.dark { 0.24 } else { 0.18 }, |_, _| th.fg);
    let xv = x0 + (x1 - x0) * t.clamp(0.0, 1.0);
    if xv > x0 + 0.5 {
        cv.fill_rrect(x0, cy - h / 2.0, xv, cy + h / 2.0, h / 2.0, 1.0, |_, _| th.accent);
    }
    if thumb {
        let ring = if th.dark { [0.24, 0.24, 0.25] } else { [1.0; 3] };
        cv.drop_shadow(RectF::new(xv - 8.0 * s, cy - 8.0 * s, 16.0 * s, 16.0 * s), 8.0 * s, 4.0 * s, 1.0 * s, 0.25);
        cv.fill_circle(xv, cy, 8.5 * s, ring, 1.0);
        cv.fill_circle(xv, cy, if hot { 5.2 * s } else { 4.2 * s }, th.accent, 1.0);
    }
}

/// Acrílico do Fluent: o que está atrás (já desfocado) + tinta do tema + granulado.
pub fn acrylic(cv: &mut Canvas, r: RectF, radius: f32, backdrop: &Image, tint: Rgb, tint_alpha: f32) {
    cv.draw_image_cover(r, backdrop, radius, 1.0);
    cv.fill_round(r, radius, tint, tint_alpha);
    cv.grain(r, radius, 0.026);
}

/// "Artista · Álbum" (sem repetir o álbum quando ele é igual ao título/artista).
pub fn artist_album(m: &MediaInfo) -> String {
    let album = m.album.trim();
    let show_album = !album.is_empty() && album != m.title.trim() && album != m.artist.trim();
    match (m.artist.is_empty(), show_album) {
        (false, true) => format!("{} · {}", m.artist, album),
        (true, true) => album.to_string(),
        _ => m.artist.clone(),
    }
}

pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

// ---------------------------------------------------------------- widget da barra

/// Widget da barra: um único "quadrado" (o fundo) com tudo dentro — capa,
/// título/artista, indicador, botões e barras de áudio.
#[derive(Default, Clone, Debug)]
pub struct WLayout {
    pub width: f32,
    /// Fundo que envolve todo o conteúdo.
    pub bg: Option<RectF>,
    pub radius: f32,
    /// Área da música (clique abre o card).
    pub info: Option<RectF>,
    pub art: Option<RectF>,
    pub text: Option<RectF>,
    pub indicator: Option<RectF>,
    /// Separador vertical entre a música e os botões: (x, y0, y1).
    pub divider: Option<(f32, f32, f32)>,
    pub shuffle: Option<RectF>,
    pub prev: Option<RectF>,
    pub play: Option<RectF>,
    pub next: Option<RectF>,
    pub repeat: Option<RectF>,
    pub viz: Option<RectF>,
}

impl WLayout {
    pub fn build(s: f32, h: f32, cfg: &Config, panel: bool, viz: bool) -> Self {
        let mut l = WLayout::default();
        let (top, bh) = (3.0 * s, h - 6.0 * s);
        l.radius = match cfg.widget_shape {
            WidgetShape::Rounded => 6.0 * s,
            WidgetShape::Pill => bh / 2.0,
            WidgetShape::Square => 2.0 * s,
        };
        let x0 = 2.0 * s;
        let mut x = x0;
        let art = (28.0 * s).min(h - 10.0 * s);
        // Na pílula a capa fica concêntrica com a ponta arredondada.
        let lead = if cfg.widget_shape == WidgetShape::Pill && cfg.widget_pill { (bh - art) / 2.0 } else { 7.0 * s };
        if panel {
            l.art = Some(RectF::new(x + lead, (h - art) * 0.5, art, art));
            let tx = x + lead + art + 10.0 * s;
            let tw = cfg.title_width as f32 * s;
            l.text = Some(RectF::new(tx, 0.0, tw, h));
            x = tx + tw + 4.0 * s;
            if cfg.playing_indicator && !viz {
                l.indicator = Some(RectF::new(x, 0.0, 18.0 * s, h));
                x += 18.0 * s;
            }
            x += 4.0 * s;
            l.info = Some(RectF::new(x0, top, x - x0, bh));
            if cfg.taskbar_controls {
                if cfg.widget_divider {
                    l.divider = Some((x.round() + 0.5, h * 0.3, h * 0.7));
                }
                x += 4.0 * s;
                let btn = 28.0 * s;
                let mut slot = |w: f32| {
                    let r = RectF::new(x, top, w, bh);
                    x += w;
                    Some(r)
                };
                if cfg.taskbar_shuffle_repeat {
                    l.shuffle = slot(btn);
                }
                l.prev = slot(btn);
                l.play = slot(32.0 * s);
                l.next = slot(btn);
                if cfg.taskbar_shuffle_repeat {
                    l.repeat = slot(btn);
                }
                x += 2.0 * s;
            }
        }
        if viz {
            let vw = cfg.width as f32 * s;
            let vh = (h * 0.5).round();
            l.viz = Some(RectF::new(x + 6.0 * s, (h - vh) * 0.5, vw, vh));
            x += vw + 12.0 * s;
        }
        // A ponta direita da pílula precisa do mesmo respiro da esquerda.
        if panel && cfg.widget_shape == WidgetShape::Pill && cfg.widget_pill {
            x += (lead - 7.0 * s).max(0.0);
        }
        if panel && cfg.widget_pill {
            l.bg = Some(RectF::new(x0, top, x - x0, bh));
        }
        l.width = if panel || viz { x + 2.0 * s } else { 0.0 };
        l
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        [
            (self.shuffle, Hit::Shuffle),
            (self.prev, Hit::Prev),
            (self.play, Hit::Play),
            (self.next, Hit::Next),
            (self.repeat, Hit::Repeat),
            (self.info, Hit::Info),
        ]
        .iter()
        .find(|(r, _)| r.is_some_and(|r| r.contains(x, y)))
        .map(|(_, h)| *h)
    }

    fn rect(&self, h: Hit) -> Option<RectF> {
        match h {
            Hit::Info => self.info,
            Hit::Shuffle => self.shuffle,
            Hit::Prev => self.prev,
            Hit::Play => self.play,
            Hit::Next => self.next,
            Hit::Repeat => self.repeat,
            _ => None,
        }
    }
}

pub struct WidgetFrame<'a> {
    pub cfg: &'a Config,
    pub media: &'a MediaInfo,
    pub theme: Theme,
    pub colors: &'a Colors,
    pub it: Interact,
    pub pb: Playback,
    pub time: f32,
    pub viz: Option<(&'a [f32], &'a [f32])>,
    pub flyout_open: bool,
    pub s: f32,
}

pub fn draw_widget(cv: &mut Canvas, tr: &mut TextRenderer, l: &WLayout, f: &WidgetFrame) {
    let (s, th, m, cfg) = (f.s, &f.theme, f.media, f.cfg);
    // Fundo único com tudo dentro; o card aberto deixa o fundo um pouco mais forte.
    if let Some(bg) = l.bg {
        let a = if f.flyout_open { 0.10 } else { 0.06 };
        cv.fill_round(bg, l.radius, th.fg, a);
        cv.stroke_round(bg, l.radius, s, th.fg, if th.dark { 0.08 } else { 0.10 });
        // Brilho fino no topo (elevação do Fluent).
        cv.fill_rrect_ca(bg.x, bg.y, bg.right(), bg.bottom(), l.radius, |_, y| ([1.0; 3], if y < bg.y + 1.5 * s { 0.05 } else { 0.0 }));
    }
    // Hover dentro do fundo: a área da música e cada botão têm o próprio destaque.
    if let Some(r) = l.info {
        let hot = f.it.hover == Some(Hit::Info) || (f.flyout_open && l.bg.is_none());
        if hot {
            let inner = if l.bg.is_some() { r.inset(2.0 * s) } else { r };
            let rad = if l.bg.is_some() { (l.radius - 2.0 * s).max(3.0 * s) } else { 6.0 * s };
            hover_bg(cv, inner, rad, th, f.it.pressed == Some(Hit::Info));
        }
    }
    for h in [Hit::Shuffle, Hit::Prev, Hit::Next, Hit::Repeat, Hit::Play] {
        if h == Hit::Play && cfg.taskbar_play == TaskbarPlay::Filled {
            continue;
        }
        if f.it.hover == Some(h) {
            if let Some(r) = l.rect(h) {
                let k = (r.w.min(r.h) - 4.0 * s).max(1.0);
                hover_bg(cv, RectF::new(r.cx() - k / 2.0, r.cy() - k / 2.0, k, k), k / 2.0, th, f.it.pressed == Some(h));
            }
        }
    }
    if let Some((x, y0, y1)) = l.divider {
        cv.fill_rrect(x - 0.5 * s, y0, x + 0.5 * s, y1, 0.5 * s, 0.14, |_, _| th.fg);
    }

    if let Some(r) = l.art {
        draw_art(cv, tr, r, m, th, cfg.cover_shape, s, cfg.show_source.then_some(12.0 * s));
    }
    if let Some(r) = l.text {
        let title = if !m.present() {
            crate::i18n::tr("Nada tocando").to_string()
        } else if m.title.is_empty() {
            m.app.clone()
        } else {
            m.title.clone()
        };
        let sub = if !cfg.show_artist || m.title.is_empty() {
            String::new()
        } else if m.artist.is_empty() {
            if cfg.show_source { m.app.clone() } else { String::new() }
        } else {
            m.artist.clone()
        };
        let line = (17.0 * s).round();
        if sub.is_empty() {
            let rr = RectF::new(r.x, ((r.h - line) / 2.0).round(), r.w, line);
            marquee(cv, tr, &title, Font::Title, rr, th.fg, 1.0, s, f.time, cfg.marquee && f.pb.playing);
        } else {
            let top = ((r.h - 2.0 * line) / 2.0).round();
            marquee(cv, tr, &title, Font::Title, RectF::new(r.x, top, r.w, line), th.fg, 1.0, s, f.time, cfg.marquee && f.pb.playing);
            text(cv, tr, &sub, Font::Subtitle, r.x, top + line, r.w, line, th.fg, 0.68);
        }
        if cfg.taskbar_progress && m.dur > 0.0 {
            let (x0, x1) = (r.x, r.right());
            let y = r.bottom() - 6.0 * s;
            let t = (f.pb.pos / m.dur) as f32;
            cv.fill_rrect(x0, y, x1, y + 2.0 * s, s, 0.16, |_, _| th.fg);
            let xv = x0 + (x1 - x0) * t.clamp(0.0, 1.0);
            if xv > x0 {
                cv.fill_rrect(x0, y, xv, y + 2.0 * s, s, 1.0, |_, _| th.accent);
            }
        }
    }
    if let Some(r) = l.indicator {
        eq_indicator(cv, r, th.accent, f.pb.playing, f.time, s, if f.pb.playing { 1.0 } else { 0.5 });
    }

    let dim = |e: bool| if e { 1.0 } else { 0.3 };
    // Aleatório/repetir: apagados quando o player não deixa mudar pelo Windows.
    if let Some(r) = l.shuffle {
        toggle_glyph(cv, tr, r, G_SHUFFLE, f.pb.shuffle, m.can_shuffle, th, s);
    }
    if let Some(r) = l.prev {
        glyph(cv, tr, r, G_PREV, Font::IconSmall, th.fg, dim(m.can_prev));
    }
    if let Some(r) = l.play {
        match cfg.taskbar_play {
            TaskbarPlay::Filled => play_button(cv, tr, r, (24.0 * s).min(r.h - 6.0 * s), f.cfg.play_shape, th, &f.it, f.pb.playing, m.can_toggle, s),
            TaskbarPlay::Icon => glyph(cv, tr, r, if f.pb.playing { G_PAUSE } else { G_PLAY }, Font::Icon, th.fg, dim(m.can_toggle)),
        }
    }
    if let Some(r) = l.next {
        glyph(cv, tr, r, G_NEXT, Font::IconSmall, th.fg, dim(m.can_next));
    }
    if let Some(r) = l.repeat {
        toggle_glyph(cv, tr, r, if f.pb.repeat == 1 { G_REPEAT_ONE } else { G_REPEAT_ALL }, f.pb.repeat != 0, m.can_repeat, th, s);
    }
    if let (Some(r), Some((values, peaks))) = (l.viz, f.viz) {
        draw(cv, r, &Frame { style: cfg.style, values, peaks, show_peaks: cfg.show_peaks, colors: f.colors, scale: s });
    }
}

// ---------------------------------------------------------------- card expandido

#[derive(Default, Clone, Debug)]
pub struct CLayout {
    /// Tamanho total da superfície (card + margem da sombra).
    pub w: f32,
    pub h: f32,
    pub card: RectF,
    pub radius: f32,
    pub art: RectF,
    pub chip: RectF,
    pub title: RectF,
    pub artist: RectF,
    pub shuffle: RectF,
    pub prev: RectF,
    pub play: RectF,
    pub play_size: f32,
    pub next: RectF,
    pub repeat: RectF,
    pub seek: Option<RectF>,
    pub vol_btn: Option<RectF>,
    pub vol: Option<RectF>,
    pub vertical: bool,
}

impl CLayout {
    pub fn build(s: f32, cfg: &Config, has_seek: bool) -> Self {
        let m = SHADOW_MARGIN * s;
        let pad = 16.0 * s;
        let mut l = CLayout { radius: cfg.corners.radius() * s, vertical: cfg.card_layout == CardLayout::Vertical, ..Default::default() };
        let seek = has_seek && cfg.card_seek;
        // Cinco botões com os ícones das pontas alinhados às bordas da coluna
        // (aleatório começa onde o título começa; repetir termina onde ele termina).
        let row = |x0: f32, x1: f32, y: f32, h: f32, inset: f32| {
            let (c0, c4) = (x0 + inset, x1 - inset);
            let step = (c4 - c0) / 4.0;
            let slot = |k: f32| RectF::new(c0 + k * step - step / 2.0, y, step, h);
            (slot(0.0), slot(1.0), slot(2.0), slot(3.0), slot(4.0))
        };
        let (cw, mut y);
        if !l.vertical {
            cw = 372.0 * s;
            let art = 112.0 * s;
            l.art = RectF::new(m + pad, m + pad, art, art);
            let tx = l.art.right() + 18.0 * s;
            let tw = m + cw - pad - tx;
            l.chip = RectF::new(tx, m + pad, tw, 22.0 * s);
            let by = l.art.bottom() - 40.0 * s;
            (l.shuffle, l.prev, l.play, l.next, l.repeat) = row(tx, tx + tw, by, 40.0 * s, 9.0 * s);
            l.play_size = 38.0 * s;
            // Título e artista ficam centralizados entre o topo da capa (ou o nome
            // do app) e os botões; `draw_card` desce um pouco se houver álbum.
            let top = if cfg.show_source { l.chip.bottom() + 4.0 * s } else { l.art.y };
            let ty = top + ((by - top) - 43.0 * s) / 2.0;
            l.title = RectF::new(tx, ty.round(), tw, 24.0 * s);
            l.artist = RectF::new(tx, (ty + 24.0 * s).round(), tw, 19.0 * s);
            y = l.art.bottom() + 16.0 * s;
        } else {
            cw = 304.0 * s;
            let art = cw - 2.0 * pad;
            l.art = RectF::new(m + pad, m + pad, art, art);
            l.chip = RectF::new(l.art.x + 10.0 * s, l.art.y + 10.0 * s, art - 20.0 * s, 24.0 * s);
            l.title = RectF::new(m + pad, l.art.bottom() + 14.0 * s, art, 26.0 * s);
            l.artist = RectF::new(m + pad, l.art.bottom() + 40.0 * s, art, 19.0 * s);
            let by = l.art.bottom() + 66.0 * s;
            (l.shuffle, l.prev, l.play, l.next, l.repeat) = row(m + pad, m + cw - pad, by, 46.0 * s, 20.0 * s);
            l.play_size = 46.0 * s;
            y = by + 46.0 * s + 12.0 * s;
        }
        // Progresso e volume com os trilhos nas mesmas bordas (as da capa e do texto).
        if seek {
            l.seek = Some(RectF::new(m + pad, y, cw - 2.0 * pad, 30.0 * s));
            y += 34.0 * s;
        }
        if cfg.show_volume {
            let b = RectF::new(m + pad - 8.0 * s, y, 32.0 * s, 32.0 * s);
            l.vol_btn = Some(b);
            let vx = m + pad + 26.0 * s;
            l.vol = Some(RectF::new(vx, y, m + cw - pad - vx, 32.0 * s));
            y += 36.0 * s;
        }
        let ch = y + 8.0 * s - m;
        l.card = RectF::new(m, m, cw, ch);
        l.w = cw + 2.0 * m;
        l.h = ch + 2.0 * m;
        l
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        let all = [
            (Some(self.shuffle), Hit::Shuffle),
            (Some(self.prev), Hit::Prev),
            (Some(self.play), Hit::Play),
            (Some(self.next), Hit::Next),
            (Some(self.repeat), Hit::Repeat),
            (self.seek, Hit::Seek),
            (self.vol_btn, Hit::VolBtn),
            (self.vol, Hit::Vol),
        ];
        if let Some(h) = all.iter().find(|(r, _)| r.is_some_and(|r| r.contains(x, y))).map(|(_, h)| *h) {
            return Some(h);
        }
        self.card.contains(x, y).then_some(Hit::Info)
    }

    pub fn vol_track(&self, s: f32) -> Option<(f32, f32)> {
        self.vol.map(|r| (r.x + 4.0 * s, r.right() - 1.0 * s))
    }

    pub fn seek_track(&self, s: f32) -> Option<(f32, f32)> {
        self.seek.map(|r| (r.x + 1.0 * s, r.right() - 1.0 * s))
    }
}

/// Sombra + fundo do card no estilo escolhido. `backdrop`: o que está atrás,
/// já desfocado (estilo acrílico); sem ele o acrílico vira um vidro sólido.
pub fn card_background(cv: &mut Canvas, card: RectF, radius: f32, cfg: &Config, colors: &Colors, media: &MediaInfo, th: &Theme, s: f32, backdrop: Option<&Image>) {
    cv.drop_shadow(card, radius, 24.0 * s, 8.0 * s, if th.dark { 0.55 } else { 0.22 });
    match (cfg.card_style, &media.art) {
        (CardStyle::Artwork, Some(art)) => {
            let b = blurred(art);
            cv.draw_image_cover(card, &b, radius, 1.0);
            // Escurece mais embaixo para o texto e os controles ficarem legíveis.
            let (y0, h) = (card.y, card.h);
            cv.fill_rrect_ca(card.x, card.y, card.right(), card.bottom(), radius, |_, y| {
                let t = ((y - y0) / h).clamp(0.0, 1.0);
                ([0.02, 0.02, 0.03], 0.38 + 0.30 * t)
            });
            cv.stroke_round(card, radius, s, [1.0; 3], 0.12);
        }
        (CardStyle::AlbumColor, _) | (CardStyle::Artwork, None) => {
            let bg = album_bg(cfg, colors, media);
            let (y0, h) = (card.y, card.h);
            let dark = th.dark;
            cv.fill_rrect_ca(card.x, card.y, card.right(), card.bottom(), radius, |_, y| {
                let t = ((y - y0) / h).clamp(0.0, 1.0);
                (lerp3(bg, if dark { [0.0; 3] } else { [1.0; 3] }, 0.25 * t), 1.0)
            });
            cv.stroke_round(card, radius, s, if dark { [1.0; 3] } else { [0.0; 3] }, 0.10);
        }
        (CardStyle::Glass, _) => {
            let base: Rgb = if th.dark { [0.12, 0.12, 0.13] } else { [0.96, 0.96, 0.965] };
            let tint = media.art.as_ref().map(|a| a.avg).unwrap_or(th.accent);
            let c = lerp3(base, tint, if th.dark { 0.14 } else { 0.06 });
            match backdrop {
                Some(b) => acrylic(cv, card, radius, b, c, if th.dark { 0.70 } else { 0.66 }),
                None => cv.fill_round(card, radius, c, 0.96),
            }
            cv.stroke_round(card, radius, s, if th.dark { [1.0; 3] } else { [0.0; 3] }, if th.dark { 0.12 } else { 0.08 });
        }
    }
}

/// Pílula com ícone + nome do app de origem.
fn app_chip(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, media: &MediaInfo, th: &Theme, s: f32, on_art: bool) {
    if media.app.is_empty() {
        return;
    }
    let name_w = tr.mask(&media.app, Font::Caption, (240.0 * s) as i32, r.h as i32, Align::Left).used_width();
    let icon_w = if media.icon.is_some() { 18.0 * s } else { 0.0 };
    let w = (name_w + icon_w + 18.0 * s).min(r.w);
    let pill = RectF::new(r.x, r.y, w, r.h);
    if on_art {
        cv.fill_round(pill, r.h / 2.0, [0.0; 3], 0.45);
    } else {
        cv.fill_round(pill, r.h / 2.0, th.fg, if th.dark { 0.12 } else { 0.07 });
    }
    let fg = if on_art { [1.0; 3] } else { th.fg };
    let mut x = pill.x + 8.0 * s;
    if let Some(i) = &media.icon {
        let k = 13.0 * s;
        cv.draw_image(RectF::new(x, pill.cy() - k / 2.0, k, k), i, 3.0 * s, 1.0);
        x += icon_w;
    }
    text(cv, tr, &media.app, Font::Caption, x, pill.y, pill.right() - x - 6.0 * s, pill.h, fg, 0.9);
}

pub struct CardFrame<'a> {
    pub cfg: &'a Config,
    pub colors: &'a Colors,
    pub media: &'a MediaInfo,
    pub theme: Theme,
    pub it: Interact,
    pub pb: Playback,
    pub s: f32,
    /// O que está atrás do card, já desfocado (estilo acrílico).
    pub backdrop: Option<&'a Image>,
    /// Aviso rápido sobre o card: (texto, opacidade).
    pub hint: Option<(&'a str, f32)>,
}

/// Sombra da capa: brilho na cor dela + sombra curta, como capas "flutuando".
fn art_shadow(cv: &mut Canvas, r: RectF, radius: f32, art: &Image, th: &Theme, s: f32) {
    cv.glow(r, radius, 22.0 * s, 7.0 * s, art.vibrant, if th.dark { 0.42 } else { 0.30 });
    cv.drop_shadow(r, radius, 12.0 * s, 4.0 * s, if th.dark { 0.40 } else { 0.22 });
}

pub fn draw_card(cv: &mut Canvas, tr: &mut TextRenderer, l: &CLayout, f: &CardFrame) {
    let (s, th, m, cfg) = (f.s, &f.theme, f.media, f.cfg);
    card_background(cv, l.card, l.radius, cfg, f.colors, m, th, s, f.backdrop);

    // Capa com sombra.
    let art_r = (l.radius * 0.6).max(6.0 * s);
    if let Some(a) = &m.art {
        art_shadow(cv, l.art, art_r, a, th, s);
    }
    let art_shape = if cfg.cover_shape == Shape::Circle && !l.vertical { Shape::Circle } else { Shape::Rounded };
    match art_shape {
        Shape::Circle => draw_art(cv, tr, l.art, m, th, Shape::Circle, s, cfg.show_source.then_some(0.0)),
        _ => match &m.art {
            Some(a) => {
                cv.draw_image_cover(l.art, a, art_r, 1.0);
                cv.stroke_round(l.art, art_r, s, [1.0; 3], 0.10);
            }
            None => draw_art(cv, tr, l.art, m, th, Shape::Rounded, s, cfg.show_source.then_some(0.0)),
        },
    }

    // App, título, artista.
    if cfg.show_source {
        app_chip(cv, tr, l.chip, m, th, s, l.vertical && m.art.is_some());
    }
    let title = if !m.present() {
        crate::i18n::tr("Nada tocando").to_string()
    } else if m.title.is_empty() {
        m.app.clone()
    } else {
        m.title.clone()
    };
    if l.vertical {
        // Coluna larga: "Artista · Álbum" numa linha só.
        text(cv, tr, &title, Font::CardTitle, l.title.x, l.title.y, l.title.w, l.title.h, th.fg, 1.0);
        text(cv, tr, &artist_album(m), Font::Subtitle, l.artist.x, l.artist.y, l.artist.w, l.artist.h, th.fg, 0.72);
    } else {
        // Álbum numa terceira linha; o bloco sobe metade dela para seguir centralizado.
        let sub = artist_album(m);
        let album = (sub != m.artist && !m.artist.is_empty()).then(|| m.album.trim());
        let dy = if album.is_some() { -8.0 * s } else { 0.0 };
        let (t, a) = (l.title.offset(0.0, dy), l.artist.offset(0.0, dy));
        text(cv, tr, &title, Font::CardTitle, t.x, t.y, t.w, t.h, th.fg, 1.0);
        text(cv, tr, if m.artist.is_empty() { &sub } else { &m.artist }, Font::Subtitle, a.x, a.y, a.w, a.h, th.fg, 0.72);
        if let Some(album) = album {
            text(cv, tr, album, Font::Small, a.x, a.bottom() - 1.0 * s, a.w, 17.0 * s, th.fg, 0.52);
        }
    }

    // Controles. Aleatório/repetir: sempre (apagados quando o player não deixa
    // mudar pelo Windows, como os navegadores) ou só quando dá, conforme a opção.
    let always = cfg.card_shuffle_repeat == ShuffleRepeat::Always;
    for h in [Hit::Shuffle, Hit::Prev, Hit::Next, Hit::Repeat, Hit::VolBtn] {
        let shown = match h {
            Hit::Shuffle => m.can_shuffle || always,
            Hit::Repeat => m.can_repeat || always,
            _ => true,
        };
        if f.it.hover == Some(h) && shown {
            let r = match h {
                Hit::Shuffle => l.shuffle,
                Hit::Prev => l.prev,
                Hit::Next => l.next,
                Hit::Repeat => l.repeat,
                _ => l.vol_btn.unwrap_or_default(),
            };
            let k = r.h.min(r.w) - 4.0 * s;
            hover_bg(cv, RectF::new(r.cx() - k / 2.0, r.cy() - k / 2.0, k, k), k / 2.0, th, f.it.pressed == Some(h));
        }
    }
    let dim = |e: bool| if e { 1.0 } else { 0.3 };
    if m.can_shuffle || always {
        toggle_glyph(cv, tr, l.shuffle, G_SHUFFLE, f.pb.shuffle, m.can_shuffle, th, s);
    }
    glyph(cv, tr, l.prev, G_PREV, Font::Icon, th.fg, dim(m.can_prev));
    play_button(cv, tr, l.play, l.play_size, cfg.play_shape, th, &f.it, f.pb.playing, m.can_toggle, s);
    glyph(cv, tr, l.next, G_NEXT, Font::Icon, th.fg, dim(m.can_next));
    if m.can_repeat || always {
        toggle_glyph(cv, tr, l.repeat, if f.pb.repeat == 1 { G_REPEAT_ONE } else { G_REPEAT_ALL }, f.pb.repeat != 0, m.can_repeat, th, s);
    }

    // Progresso com tempos embaixo.
    if let (Some(r), Some((x0, x1))) = (l.seek, l.seek_track(s)) {
        let mut pos = f.pb.pos;
        let mut t = if m.dur > 0.0 { (pos / m.dur) as f32 } else { 0.0 };
        if let Some(p) = f.pb.seek_preview {
            t = p;
            pos = p as f64 * m.dur;
        }
        let hot = (f.it.hover == Some(Hit::Seek) || f.it.drag == Some(Hit::Seek)) && m.can_seek;
        slider(cv, x0, x1, r.y + 7.0 * s, t, th, hot, hot, s);
        text(cv, tr, &fmt_time(pos), Font::Small, x0, r.y + 13.0 * s, 60.0 * s, 16.0 * s, th.fg, 0.62);
        text_right(cv, tr, &format!("-{}", fmt_time((m.dur - pos).max(0.0))), Font::Small, x1, r.y + 13.0 * s, 16.0 * s, th.fg, 0.62, s);
    }

    // Volume.
    if let (Some(b), Some(r), Some((x0, x1))) = (l.vol_btn, l.vol, l.vol_track(s)) {
        let v = f.pb.volume;
        let g = if f.pb.muted || v <= 0.0 {
            G_MUTE
        } else if v < 0.34 {
            G_VOL1
        } else if v < 0.67 {
            G_VOL2
        } else {
            G_VOL3
        };
        glyph(cv, tr, b, g, Font::IconSmall, th.fg, 0.9);
        let hot = f.it.hover == Some(Hit::Vol) || f.it.drag == Some(Hit::Vol);
        slider(cv, x0, x1, r.cy(), if f.pb.muted { 0.0 } else { v }, th, hot, true, s);
    }

    // Aviso rápido (ex.: clicou em aleatório num player que não deixa).
    if let Some((msg, a)) = f.hint {
        let h = 26.0 * s;
        let tw = tr.mask(msg, Font::Caption, (l.card.w - 40.0 * s) as i32, h as i32, Align::Left).used_width();
        let w = (tw + 28.0 * s).min(l.card.w - 24.0 * s);
        let y = l.shuffle.bottom() + 4.0 * s;
        let r = RectF::new((l.card.cx() - w / 2.0).round(), y.round(), w, h);
        cv.drop_shadow(r, h / 2.0, 10.0 * s, 2.0 * s, 0.35 * a);
        cv.fill_round(r, h / 2.0, [0.10, 0.10, 0.11], 0.92 * a);
        cv.stroke_round(r, h / 2.0, s, [1.0; 3], 0.10 * a);
        text_center(cv, tr, msg, Font::Caption, r, [1.0; 3], a);
    }
}

// ---------------------------------------------------------------- "A seguir"

pub fn upnext_size(s: f32) -> (f32, f32) {
    let m = SHADOW_MARGIN * s;
    (UPNEXT_W * s + 2.0 * m, UPNEXT_H * s + 2.0 * m)
}

/// Retângulo do cartão "A seguir" dentro da sua superfície (sem a sombra).
pub fn upnext_card(s: f32) -> RectF {
    let m = SHADOW_MARGIN * s;
    RectF::new(m, m, UPNEXT_W * s, UPNEXT_H * s)
}

/// Cartão "A seguir" (t = segundos desde que apareceu; `total` = duração).
///
/// Grade: capa de 64 px à esquerda; à direita, quatro linhas cujo topo e base
/// coincidem com os da capa: selo "A SEGUIR", título, "artista · álbum" e a
/// barrinha do tempo restante do cartão.
pub fn draw_upnext(cv: &mut Canvas, tr: &mut TextRenderer, cfg: &Config, colors: &Colors, media: &MediaInfo, th: &Theme, s: f32, t: f32, total: f32, backdrop: Option<&Image>) {
    let card = upnext_card(s);
    let radius = cfg.corners.radius() * s;
    card_background(cv, card, radius, cfg, colors, media, th, s, backdrop);

    let pad = 14.0 * s;
    let full = card.h - 2.0 * pad;
    let slot = RectF::new(card.x + pad, card.y + pad, full, full);
    let art_r = (radius * 0.55).max(6.0 * s);
    let k = 0.86 + 0.14 * ease_out_back(t / 0.6);
    let a = full * k;
    let art = RectF::new(slot.cx() - a / 2.0, slot.cy() - a / 2.0, a, a);
    match &media.art {
        Some(img) => {
            art_shadow(cv, art, art_r, img, th, s);
            cv.draw_image_cover(art, img, art_r, 1.0);
            cv.stroke_round(art, art_r, s, [1.0; 3], 0.10);
        }
        None => draw_art(cv, tr, art, media, th, Shape::Rounded, s, cfg.show_source.then_some(0.0)),
    }

    let tx0 = slot.right() + 14.0 * s;
    let right = card.right() - 16.0 * s;
    let slide = (1.0 - ease_out_cubic((t - 0.1) / 0.5)) * 12.0 * s;
    let ta = ((t - 0.1) / 0.35).clamp(0.0, 1.0);
    let tx = tx0 + slide;
    let tw = right - tx0;
    let top = slot.y;

    // Selo "A SEGUIR" com o mini equalizador, numa pílula na cor de destaque.
    let label = crate::i18n::tr("A SEGUIR");
    let lh = 18.0 * s;
    let lw = tr.mask(label, Font::Label, (160.0 * s) as i32, lh as i32, Align::Left).used_width();
    let badge = RectF::new(tx, top - 1.0 * s, 26.0 * s + lw + 9.0 * s, lh);
    cv.fill_round(badge, lh / 2.0, th.accent, 0.18 * ta);
    eq_indicator(cv, RectF::new(badge.x + 7.0 * s, badge.y - 0.5 * s, 13.0 * s, lh), th.accent, true, t, s, ta);
    text(cv, tr, label, Font::Label, badge.x + 25.0 * s, badge.y, lw + 4.0 * s, lh, th.accent, ta);
    if cfg.show_source && !media.app.is_empty() {
        let avail = (right - badge.right() - 26.0 * s).max(0.0);
        let mm = tr.mask(&media.app, Font::Caption, avail as i32, lh as i32, Align::Left);
        let ax = right - mm.used_width();
        cv.blit_mask(ax as i32, badge.y as i32, &mm, th.fg, ta * 0.6);
        if let Some(icon) = &media.icon {
            let ik = 13.0 * s;
            cv.draw_image(RectF::new(ax - ik - 5.0 * s, badge.cy() - ik / 2.0, ik, ik), icon, 3.0 * s, ta);
        }
    }

    let title = if media.title.is_empty() { media.app.clone() } else { media.title.clone() };
    text(cv, tr, &title, Font::CardTitle, tx, top + 20.0 * s, tw, 22.0 * s, th.fg, ta);
    text(cv, tr, &artist_album(media), Font::Subtitle, tx, top + 42.0 * s, tw, 17.0 * s, th.fg, ta * 0.72);

    // Tempo restante do cartão: a base da barrinha encosta na base da capa.
    let hold = ((t - 0.45) / (total - 0.85).max(0.5)).clamp(0.0, 1.0);
    let (bx0, bx1) = (tx0, right);
    let by = slot.bottom() - 3.0 * s;
    cv.fill_rrect(bx0, by, bx1, by + 3.0 * s, 1.5 * s, if th.dark { 0.18 } else { 0.12 }, |_, _| th.fg);
    let xe = bx0 + (bx1 - bx0) * (1.0 - hold);
    if xe > bx0 + 1.0 {
        cv.fill_rrect(bx0, by, xe, by + 3.0 * s, 1.5 * s, 0.95, |_, _| th.accent);
    }
}

// ---------------------------------------------------------------- Caps Lock / Num Lock

/// Tamanho do cartão do aviso (sem a margem da sombra), em pixels físicos.
pub fn osd_card(s: f32, cfg: &Config) -> (f32, f32) {
    let k = s * cfg.osd_size.factor();
    let w = if cfg.osd_switch { 236.0 } else { 184.0 };
    (w * k, 64.0 * k)
}

pub fn osd_size(s: f32, cfg: &Config) -> (f32, f32) {
    let m = SHADOW_MARGIN * s;
    let (w, h) = osd_card(s, cfg);
    (w + 2.0 * m, h + 2.0 * m)
}

fn osd_dark(cfg: &Config, colors: &Colors) -> bool {
    match cfg.osd_theme {
        ThemeMode::Auto => system_dark(colors),
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
    }
}

/// Fundo do aviso no material "Cor de destaque".
fn osd_accent_bg(cfg: &Config, colors: &Colors) -> Rgb {
    if osd_dark(cfg, colors) { lerp3(colors.accent, [0.0; 3], 0.18) } else { colors.accent }
}

/// Cores do aviso de teclas (tema próprio; destaque do Windows). No material
/// "Cor de destaque" o cartão é a cor e o ícone aceso fica no contraste dela.
pub fn osd_theme(cfg: &Config, colors: &Colors) -> Theme {
    if cfg.osd_material == OsdMaterial::Accent {
        let bg = osd_accent_bg(cfg, colors);
        let fg = on_color(bg);
        return Theme { fg, accent: fg, on_accent: bg, dark: luma(bg) < 0.5 };
    }
    let dark = osd_dark(cfg, colors);
    let fg = if dark { [1.0; 3] } else { [0.08, 0.08, 0.09] };
    let accent = if dark { colors.accent_light } else { colors.accent };
    Theme { fg, accent, on_accent: on_color(accent), dark }
}

/// Aviso de tecla de bloqueio: cartão (sólido, acrílico ou colorido) com ícone
/// que "acende" quando liga. `num`: Num Lock (senão Caps Lock). `backdrop`: o
/// que está atrás, já desfocado, para o material acrílico.
pub fn draw_osd(cv: &mut Canvas, tr: &mut TextRenderer, cfg: &Config, colors: &Colors, th: &Theme, s: f32, key: &str, num: bool, on: bool, t: f32, backdrop: Option<&Image>) {
    let m = SHADOW_MARGIN * s;
    let k = s * cfg.osd_size.factor();
    let (cw, ch) = osd_card(s, cfg);
    let card = RectF::new(m, m, cw, ch);
    let radius = match cfg.osd_shape {
        OsdShape::Pill => card.h / 2.0,
        OsdShape::Rounded => cfg.corners.radius() * k,
        OsdShape::Square => 5.0 * k,
    };
    let opacity = cfg.osd_opacity as f32 / 100.0;
    cv.drop_shadow(card, radius, 22.0 * s, 7.0 * s, (if th.dark { 0.5 } else { 0.22 }) * (0.4 + 0.6 * opacity));
    let base: Rgb = if th.dark { [0.13, 0.13, 0.14] } else { [0.975, 0.975, 0.98] };
    match (cfg.osd_material, backdrop) {
        (OsdMaterial::Accent, _) => cv.fill_round(card, radius, osd_accent_bg(cfg, colors), opacity),
        (OsdMaterial::Acrylic, Some(b)) => acrylic(cv, card, radius, b, base, opacity * 0.78),
        _ => cv.fill_round(card, radius, base, opacity),
    }
    cv.stroke_round(card, radius, s, if th.dark { [1.0; 3] } else { [0.0; 3] }, if th.dark { 0.12 } else { 0.08 });

    let (f_sym, f_key, f_state, f_icon) = match cfg.osd_size {
        OsdSize::Compact => (Font::CardTitle, Font::Title, Font::Small, Font::Icon),
        OsdSize::Normal => (Font::Display, Font::BodyStrong, Font::Caption, Font::IconLg),
        OsdSize::Large => (Font::Hero, Font::CardTitle, Font::Body, Font::IconHero),
    };

    // Ícone: círculo (ou quadrado, seguindo os cantos) que acende na cor de destaque.
    let pop = ease_out_back(t / 0.45);
    let full = 40.0 * k;
    let d = full * (0.75 + 0.25 * pop);
    let c = RectF::new(card.x + 12.0 * k + (full - d) / 2.0, card.cy() - d / 2.0, d, d);
    let icon_r = match cfg.osd_shape {
        OsdShape::Pill => d / 2.0,
        OsdShape::Rounded => (cfg.corners.radius() * 0.7).max(6.0) * k,
        OsdShape::Square => 4.0 * k,
    };
    let lit = if on { ease_out_cubic(t / 0.3) } else { 1.0 - ease_out_cubic(t / 0.3) };
    cv.fill_round(c, icon_r, th.fg, 0.10 * (1.0 - lit));
    cv.fill_round(c, icon_r, th.accent, lit);
    let sym_c = lerp3(th.fg, th.on_accent, lit);
    match cfg.osd_icon {
        OsdIcon::Letter => text_center(cv, tr, if num { "1" } else { "A" }, f_sym, c, sym_c, 1.0),
        OsdIcon::Lock => glyph(cv, tr, c, if on { G_LOCK } else { G_UNLOCK }, f_icon, sym_c, 1.0),
        OsdIcon::Keyboard => glyph(cv, tr, c, if num { G_NUMPAD } else { G_KEYBOARD }, f_icon, sym_c, 1.0),
    }

    let tx = card.x + 64.0 * k;
    let right = if cfg.osd_switch { card.right() - 64.0 * k } else { card.right() - 14.0 * k };
    text(cv, tr, key, f_key, tx, card.y + 11.0 * k, right - tx, 22.0 * k, th.fg, 1.0);
    let state = crate::i18n::tr(if on { "Ativado" } else { "Desativado" });
    text(cv, tr, state, f_state, tx, card.y + 33.0 * k, right - tx, 19.0 * k, if on { th.accent } else { th.fg }, if on { 1.0 } else { 0.65 });

    if cfg.osd_switch {
        let sw = RectF::new(card.right() - 56.0 * k, card.cy() - 10.0 * k, 40.0 * k, 20.0 * k);
        let kx = sw.x + 10.0 * k + 20.0 * k * if on { ease_out_cubic(t / 0.35) } else { 1.0 - ease_out_cubic(t / 0.35) };
        if on {
            cv.fill_round(sw, 10.0 * k, th.accent, 1.0);
            cv.fill_circle(kx, sw.cy(), 6.0 * k, th.on_accent, 1.0);
        } else {
            cv.stroke_round(sw, 10.0 * k, s, th.fg, 0.55);
            cv.fill_circle(kx, sw.cy(), 6.0 * k, th.fg, 0.8);
        }
    }
}

// ---------------------------------------------------------------- animação

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0) - 1.0;
    t * t * t + 1.0
}

pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

// ---------------------------------------------------------------- prévias

/// Música de exemplo para as prévias quando nada está tocando.
pub fn sample_media() -> MediaInfo {
    thread_local! {
        static ART: Arc<Image> = Arc::new(sample_art());
    }
    MediaInfo {
        playing: true,
        aumid: "Spotify.exe".into(),
        app: "Spotify".into(),
        title: "Midnight City".into(),
        artist: "M83".into(),
        album: "Hurry Up, We're Dreaming".into(),
        can_prev: true,
        can_next: true,
        can_toggle: true,
        can_shuffle: true,
        can_repeat: true,
        shuffle: true,
        repeat: 0,
        art: Some(ART.with(|a| a.clone())),
        icon: None,
        auto_change_seq: 0,
        art_pending: false,
        pos: 110.0,
        dur: 243.0,
        pos_at: None,
        can_seek: true,
    }
}

/// Capa de exemplo gerada: céu em gradiente com um sol e "montanhas".
fn sample_art() -> Image {
    // 256 px com bordas suaves: fica nítida também nas prévias em 2x.
    const N: usize = 256;
    let px_size = 1.0 / N as f32;
    let mut px = vec![0u32; N * N];
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = ((x as f32 + 0.5) / N as f32, (y as f32 + 0.5) / N as f32);
            let mut c = lerp3([0.35, 0.18, 0.62], [0.98, 0.45, 0.35], fy);
            let d = ((fx - 0.5).powi(2) + (fy - 0.52).powi(2)).sqrt();
            c = lerp3(c, [1.0, 0.86, 0.55], ((0.2 - d) / 0.03).clamp(0.0, 1.0));
            let ridge = 0.72 + 0.06 * (fx * 9.0).sin() + 0.03 * (fx * 23.0).cos();
            // Cobertura da montanha com 1 px de transição (sem serrilhado).
            let k = ((fy - ridge) / px_size + 0.5).clamp(0.0, 1.0);
            let hill = lerp3([0.16, 0.08, 0.28], [0.08, 0.04, 0.16], ((fy - ridge) * 3.0).clamp(0.0, 1.0));
            c = lerp3(c, hill, k);
            let p = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u32;
            px[y * N + x] = 0xff00_0000 | (p(c[0]) << 16) | (p(c[1]) << 8) | p(c[2]);
        }
    }
    Image::new(N, N, px)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn stray(px: &[u32], w: usize, h: usize) -> Vec<(usize, usize, u32)> {
        let mut out = Vec::new();
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let p = px[y * w + x];
                let a = p >> 24;
                let r = (p >> 16) & 0xff;
                if a > 200 && r > 230 {
                    let n = [px[y * w + x - 1], px[y * w + x + 1], px[(y - 1) * w + x], px[(y + 1) * w + x]];
                    if n.iter().all(|q| ((q >> 16) & 0xff) < 120) {
                        out.push((x, y, p));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn card_has_no_stray_pixels() {
        let cfg = Config::default();
        let colors = crate::render::Colors { palette: cfg.palette, accent: [0.3, 0.5, 0.9], accent_light: [0.6, 0.8, 1.0], light_taskbar: false, album: None };
        let media = sample_media();
        let mut tr = TextRenderer::new();
        tr.set_scale(1.0);
        let l = CLayout::build(1.0, &cfg, true);
        let (w, h) = (l.w as usize, l.h as usize);
        let mut px = vec![0u32; w * h];
        {
            let mut cv = Canvas::new(w, h, &mut px);
            let f = CardFrame { cfg: &cfg, colors: &colors, media: &media, theme: card_theme(&cfg, &colors, &media), it: Interact::default(), pb: Playback { volume: 0.5, ..Default::default() }, s: 1.0, backdrop: None, hint: None };
            draw_card(&mut cv, &mut tr, &l, &f);
        }
        let mut s = stray(&px, w, h);
        println!("card {}x{} stray: {:?}", w, h, s);
        // Estados variados: estilos alternativos, música no fim, pausada.
        let ucfg = Config { card_style: crate::config::CardStyle::AlbumColor, card_layout: crate::config::CardLayout::Vertical, ..Config::default() };
        let mut m2 = media.clone();
        // Capa de alto contraste (ruído) para estressar desfoque e amostragem.
        let mut seed = 12345u32;
        let noise: Vec<u32> = (0..96 * 96)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                0xff00_0000 | (seed >> 8)
            })
            .collect();
        m2.art = Some(Arc::new(Image::new(96, 96, noise)));
        m2.pos = m2.dur;
        m2.playing = false;
        for pos in [0.0, m2.dur * 0.5, m2.dur] {
            let l = CLayout::build(1.0, &ucfg, true);
            let (w, h) = (l.w as usize, l.h as usize);
            let mut px = vec![0u32; w * h];
            {
                let mut cv = Canvas::new(w, h, &mut px);
                let f = CardFrame { cfg: &ucfg, colors: &colors, media: &m2, theme: card_theme(&ucfg, &colors, &m2), it: Interact::default(), pb: Playback { volume: 0.6, pos, ..Default::default() }, s: 1.0, backdrop: None, hint: None };
                draw_card(&mut cv, &mut tr, &l, &f);
            }
            let st = stray(&px, w, h);
            println!("alt cfg pos={pos}: seek={:?} vol={:?} stray={:?}", l.seek, l.vol, st);
            s.extend(st);
        }
        // máscaras de glifos isolados
        for g in [G_MUTE, G_VOL1, G_VOL2, G_VOL3, G_PREV, G_NEXT, G_SHUFFLE, G_REPEAT_ALL, G_PLAY, G_PAUSE] {
            for font in [Font::IconSmall, Font::Icon] {
                let m = tr.mask(g, font, 34, 32, Align::Center);
                if m.cov[0] > 0 || m.cov[m.w - 1] > 0 {
                    println!("glyph {:?} {:?} corner cov {} {}", g, font, m.cov[0], m.cov[m.w - 1]);
                }
            }
        }
        for t in ["2:53", "-0:00", "Midnight City", "M83"] {
            for font in [Font::Small, Font::Subtitle, Font::CardTitle] {
                let m = tr.mask(t, font, 60, 16, Align::Left);
                if m.cov[0] > 0 {
                    println!("text {t:?} {font:?} corner {}", m.cov[0]);
                }
            }
        }
        assert!(s.is_empty(), "pixels soltos: {:?}", s);
    }
}
