//! Desenho dos componentes visuais (funções puras, sem estado de janela):
//! widget da barra, card expandido, cartão "A seguir" e aviso de Caps/Num Lock.
//! Usado pelas janelas reais (overlay.rs) e pelas pré-visualizações ao vivo da
//! tela de configurações (settings.rs).
//!
//! Referências de design: Fluent 2 (raios 4/8, elevação com sombra suave,
//! Segoe UI Variable) e cards de "now playing" populares no Dribbble (capa
//! desfocada como fundo, cor extraída da capa, vidro, play circular).

use crate::config::{CardLayout, CardStyle, Config, OsdIcon, OsdMaterial, OsdShape, OsdSize, SecondLine, Shape, ShuffleRepeat, Size3, TaskbarPlay, ThemeMode, WidgetShape, WidgetTint};
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
    /// Animação dos ícones neste quadro.
    pub fx: IconFx,
}

// ---------------------------------------------------------------- ícones animados

/// Botões com ícone animado, na ordem de `IconAnims`/`IconFx`.
const FX_HITS: [Hit; 6] = [Hit::Shuffle, Hit::Prev, Hit::Play, Hit::Next, Hit::Repeat, Hit::VolBtn];

fn fx_slot(h: Hit) -> Option<usize> {
    FX_HITS.iter().position(|x| *x == h)
}

/// Duração da animação de cada ícone (s).
fn fx_dur(h: Hit) -> f32 {
    match h {
        Hit::Play => 0.42,
        Hit::Prev | Hit::Next => 0.44,
        Hit::Shuffle => 0.56,
        Hit::Repeat => 0.62,
        _ => 0.5,
    }
}

/// Quando cada ícone foi acionado (clique ou mudança vinda do player) e o
/// último estado visto. Fica na janela; o quadro recebe só um `IconFx`.
#[derive(Clone, Copy, Debug, Default)]
pub struct IconAnims {
    kicks: [Option<std::time::Instant>; 6],
    /// (tocando, aleatório, repetir, mudo) do quadro anterior.
    seen: Option<(bool, bool, u8, bool)>,
}

impl IconAnims {
    /// Começa a animação do ícone `h` agora.
    pub fn kick(&mut self, h: Hit) {
        if let Some(i) = fx_slot(h) {
            self.kicks[i] = Some(std::time::Instant::now());
        }
    }

    /// Anima o que mudou desde o quadro anterior: play↔pausa, aleatório,
    /// repetir e mudo (pelo clique ou pelo próprio player).
    pub fn observe(&mut self, pb: &Playback) {
        let now = (pb.playing, pb.shuffle, pb.repeat, pb.muted);
        if let Some(old) = self.seen.replace(now) {
            if old.0 != now.0 {
                self.kick(Hit::Play);
            }
            if old.1 != now.1 {
                self.kick(Hit::Shuffle);
            }
            if old.2 != now.2 {
                self.kick(Hit::Repeat);
            }
            if old.3 != now.3 {
                self.kick(Hit::VolBtn);
            }
        }
    }

    /// Esquece o estado visto (ex.: a música sumiu): a próxima não anima a chegada.
    pub fn reset_seen(&mut self) {
        self.seen = None;
    }

    /// Alguma animação em andamento (a janela precisa de quadros)?
    pub fn running(&self) -> bool {
        FX_HITS.iter().zip(&self.kicks).any(|(h, k)| k.is_some_and(|t| t.elapsed().as_secs_f32() < fx_dur(*h)))
    }

    /// O que o quadro de agora precisa saber. `on`: opção "Animar ícones".
    pub fn fx(&self, on: bool) -> IconFx {
        let mut age = [f32::INFINITY; 6];
        if on {
            for (a, k) in age.iter_mut().zip(&self.kicks) {
                if let Some(t) = k {
                    *a = t.elapsed().as_secs_f32();
                }
            }
        }
        IconFx { on, age }
    }
}

/// Animação dos ícones num quadro: tempo (s) desde o acionamento de cada um.
#[derive(Clone, Copy, Debug)]
pub struct IconFx {
    pub on: bool,
    age: [f32; 6],
}

impl Default for IconFx {
    fn default() -> Self {
        Self { on: false, age: [f32::INFINITY; 6] }
    }
}

impl IconFx {
    /// Só o ícone `h`, `age` segundos depois de acionado (capturas).
    #[cfg(test)]
    pub fn single(h: Hit, age: f32) -> Self {
        let mut fx = Self { on: true, ..Self::default() };
        if let Some(i) = fx_slot(h) {
            fx.age[i] = age;
        }
        fx
    }

    /// Duração da animação de `h` (s).
    #[cfg(test)]
    pub fn duration(h: Hit) -> f32 {
        fx_dur(h)
    }

    /// Progresso (0..1) da animação de `h`, se ela estiver rodando.
    fn progress(&self, h: Hit) -> Option<f32> {
        let a = self.age[fx_slot(h)?];
        let d = fx_dur(h);
        (self.on && a < d).then(|| (a / d).clamp(0.0, 1.0))
    }
}

/// Transformação de um ícone em volta do centro: deslocamento, escala,
/// rotação (radianos, horário) e opacidade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pose {
    dx: f32,
    scale: f32,
    rot: f32,
    alpha: f32,
}

impl Pose {
    const REST: Pose = Pose { dx: 0.0, scale: 1.0, rot: 0.0, alpha: 1.0 };
}

/// Pose do ícone `h` agora: aperto do clique + a animação de cada botão.
fn pose(it: &Interact, h: Hit, s: f32) -> Pose {
    let fx = &it.fx;
    if !fx.on {
        return Pose::REST;
    }
    let mut p = Pose::REST;
    if let Some(u) = fx.progress(h) {
        let bump = (std::f32::consts::PI * u).sin();
        match h {
            // Pula: o ícone dá um empurrão para o lado da troca e volta.
            Hit::Prev | Hit::Next => {
                let dir = if h == Hit::Next { 1.0 } else { -1.0 };
                let push = (std::f32::consts::PI * ease_out_cubic(u)).sin();
                p.dx = dir * 6.0 * s * push;
                p.alpha = 1.0 - 0.45 * push;
                p.scale = 1.0 - 0.06 * bump;
            }
            // Aleatório: cresce e balança (cruza as setas).
            Hit::Shuffle => {
                p.scale = 1.0 + 0.2 * bump;
                p.rot = 0.22 * (std::f32::consts::TAU * 1.5 * u).sin() * (1.0 - u);
            }
            // Repetir: uma volta completa, assentando devagar.
            Hit::Repeat => {
                p.rot = std::f32::consts::TAU * ease_enter(u);
                p.scale = 1.0 + 0.1 * bump;
            }
            // Volume (mudo): chacoalha um pouco.
            Hit::VolBtn => {
                p.scale = 1.0 + 0.16 * bump;
                p.rot = 0.28 * (std::f32::consts::TAU * 1.5 * u).sin() * (1.0 - u);
            }
            _ => {}
        }
    }
    // O play já encolhe o botão inteiro ao ser apertado.
    if it.pressed == Some(h) && it.hover == Some(h) && h != Hit::Play {
        p.scale *= 0.86;
    }
    p
}

/// Ícone com pose (parado = desenho normal, alinhado aos pixels).
pub(crate) fn glyph_pose(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, g: &str, font: Font, c: Rgb, a: f32, p: Pose) {
    if p == Pose::REST {
        return glyph(cv, tr, r, g, font, c, a);
    }
    let (w, h) = (r.w.round().max(1.0), r.h.round().max(1.0));
    let m = tr.mask(g, font, w as i32, h as i32, Align::Center);
    // O símbolo está centrado na caixa: gira e escala em volta do centro dela.
    cv.blit_mask_xform(r.x.round() + p.dx, r.y.round(), &m, w / 2.0, h / 2.0, p.scale, p.rot, c, a * p.alpha);
}

/// Ícone de um botão de controle (pular, play só ícone, volume), animado.
pub(crate) fn ctrl_glyph(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, g: &str, font: Font, c: Rgb, a: f32, it: &Interact, h: Hit, s: f32) {
    glyph_pose(cv, tr, r, g, font, c, a, pose(it, h, s));
}

/// Play↔pausa animado: o ícone antigo encolhe e some, o novo cresce com um
/// pequeno salto. Sem animação, desenha só o atual.
fn play_glyph(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, playing: bool, font: Font, c: Rgb, a: f32, it: &Interact, s: f32) {
    let (now_g, old_g) = if playing { (G_PAUSE, G_PLAY) } else { (G_PLAY, G_PAUSE) };
    let base = pose(it, Hit::Play, s);
    match it.fx.progress(Hit::Play) {
        Some(u) => {
            let out = (u / 0.4).min(1.0);
            if out < 1.0 {
                let po = Pose { scale: base.scale * (1.0 - 0.5 * ease_out_cubic(out)), alpha: 1.0 - out, ..base };
                glyph_pose(cv, tr, r, old_g, font, c, a, po);
            }
            let e = ease_out_back(((u - 0.12) / 0.88).clamp(0.0, 1.0));
            let pn = Pose { scale: base.scale * (0.45 + 0.55 * e), alpha: ((u - 0.12) / 0.3).clamp(0.0, 1.0), ..base };
            glyph_pose(cv, tr, r, now_g, font, c, a, pn);
        }
        None => glyph_pose(cv, tr, r, now_g, font, c, a, base),
    }
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
pub(crate) fn blurred(img: &Arc<Image>) -> Arc<Image> {
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
pub(crate) fn marquee(cv: &mut Canvas, tr: &mut TextRenderer, t: &str, font: Font, r: RectF, c: Rgb, a: f32, s: f32, time: f32, enabled: bool) {
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

pub(crate) fn hover_bg(cv: &mut Canvas, r: RectF, radius: f32, th: &Theme, pressed: bool) {
    let a = if pressed { 0.06 } else if th.dark { 0.10 } else { 0.07 };
    cv.fill_round(r, radius, th.fg, a);
}

/// Capa com o formato escolhido (ou ícone do app / nota musical).
/// `source`: `None` esconde o app de origem; `Some(badge)` mostra o ícone do app
/// (sobre a capa quando `badge > 0`, ou no lugar dela quando não há capa).
pub fn draw_art(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, media: &MediaInfo, th: &Theme, shape: Shape, s: f32, source: Option<f32>) {
    draw_art_a(cv, tr, r, media, th, shape, s, source, 1.0);
}

/// Selo do app de origem no canto da capa: círculo com o ícone do app e um vão
/// transparente em volta (recorte), como os selos de notificação do Windows/iOS.
fn source_badge(cv: &mut Canvas, art: RectF, icon: &Image, size: f32, th: &Theme, s: f32, alpha: f32) {
    let (cx, cy) = (art.right() - size * 0.22, art.bottom() - size * 0.22);
    let b = RectF::new(cx - size / 2.0, cy - size / 2.0, size, size);
    let gap = 1.6 * s;
    if alpha >= 0.999 {
        cv.erase_round(b.inset(-gap), size / 2.0 + gap);
    }
    let base: Rgb = if th.dark { [0.16, 0.16, 0.17] } else { [0.98, 0.98, 0.985] };
    cv.fill_circle(cx, cy, size / 2.0, base, alpha);
    cv.stroke_round(b, size / 2.0, (0.8 * s).max(0.8), th.fg, 0.12 * alpha);
    let k = size * 0.74;
    cv.draw_image(RectF::new(cx - k / 2.0, cy - k / 2.0, k, k), icon, k * 0.22, alpha);
}

pub fn draw_art_a(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, media: &MediaInfo, th: &Theme, shape: Shape, s: f32, source: Option<f32>, alpha: f32) {
    let badge = source.unwrap_or(0.0);
    let icon = if source.is_some() { &media.icon } else { &None };
    let radius = match shape {
        Shape::Circle => r.w / 2.0,
        Shape::Rounded => (r.w * 0.16).max(3.0 * s),
        Shape::Square => 2.0 * s,
    };
    match (&media.art, icon) {
        (Some(a), icon) => {
            cv.draw_image_cover(r, a, radius, alpha);
            cv.stroke_round(r, radius, s, th.fg, 0.08 * alpha);
            if let (Some(i), true) = (icon, badge > 0.0) {
                source_badge(cv, r, i, badge, th, s, alpha);
            }
        }
        (None, Some(i)) => {
            cv.fill_round(r, radius, th.accent, 0.18 * alpha);
            let k = r.w * 0.6;
            cv.draw_image(RectF::new(r.cx() - k / 2.0, r.cy() - k / 2.0, k, k), i, 3.0 * s, alpha);
        }
        (None, None) => {
            cv.fill_round(r, radius, th.accent, 0.28 * alpha);
            glyph(cv, tr, r, G_MUSIC, if r.w > 64.0 * s { Font::IconXl } else { Font::Icon }, th.fg, 0.9 * alpha);
        }
    }
}

/// Mini equalizador animado ("tocando agora").
pub(crate) fn eq_indicator(cv: &mut Canvas, r: RectF, color: Rgb, playing: bool, time: f32, s: f32, alpha: f32) {
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
        cv.fill_rrect(x, base - h, x + bw, base, bw / 2.0, alpha, color);
    }
}

pub(crate) fn play_button(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, size: f32, shape: Shape, th: &Theme, it: &Interact, playing: bool, enabled: bool, s: f32) {
    let hot = it.hover == Some(Hit::Play);
    let pressed = it.pressed == Some(Hit::Play);
    let mut k = size * if pressed { 0.94 } else if hot { 1.05 } else { 1.0 };
    // Na troca play↔pausa o botão dá um pulsinho junto com o ícone.
    if let Some(u) = it.fx.progress(Hit::Play) {
        k *= 1.0 + 0.07 * (std::f32::consts::PI * u).sin();
    }
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
    // Caixa do ícone no tamanho normal (o mesmo desenho em cache em qualquer quadro).
    let font = if size > 30.0 * s { Font::Icon } else { Font::IconSmall };
    let gb = RectF::new(r.cx() - size / 2.0, r.cy() - size / 2.0, size, size);
    play_glyph(cv, tr, gb, playing, font, th.on_accent, en, it, s);
}

pub(crate) fn toggle_glyph(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, g: &str, on: bool, enabled: bool, th: &Theme, s: f32, it: &Interact, h: Hit) {
    let c = if on && enabled { th.accent } else { th.fg };
    let a = if !enabled { 0.3 } else if on { 1.0 } else { 0.8 };
    ctrl_glyph(cv, tr, r, g, Font::IconSmall, c, a, it, h, s);
    if on && enabled {
        cv.fill_circle(r.cx(), r.bottom() - 5.0 * s, 1.7 * s, th.accent, 1.0);
    }
}

pub(crate) fn slider(cv: &mut Canvas, x0: f32, x1: f32, cy: f32, t: f32, th: &Theme, hot: bool, thumb: bool, s: f32) {
    let h = 4.0 * s;
    cv.fill_rrect(x0, cy - h / 2.0, x1, cy + h / 2.0, h / 2.0, if th.dark { 0.24 } else { 0.18 }, th.fg);
    let xv = x0 + (x1 - x0) * t.clamp(0.0, 1.0);
    if xv > x0 + 0.5 {
        cv.fill_rrect(x0, cy - h / 2.0, xv, cy + h / 2.0, h / 2.0, 1.0, th.accent);
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
        let art = match cfg.cover_size {
            Size3::Small => (22.0 * s).min(h - 10.0 * s),
            Size3::Normal => (28.0 * s).min(h - 10.0 * s),
            Size3::Large => (32.0 * s).min(h - 8.0 * s),
        };
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
    /// Troca de música em andamento: (música anterior, progresso do texto e
    /// da capa, 0..1; 1 = sem animação).
    pub change: Option<(&'a MediaInfo, f32, f32)>,
}

/// Fontes do título e da segunda linha do widget, e a altura de cada linha.
fn widget_fonts(cfg: &Config, s: f32) -> (Font, Font, f32) {
    match cfg.text_size {
        Size3::Small => (Font::TitleSm, Font::SubtitleSm, (15.5 * s).round()),
        Size3::Normal => (Font::Title, Font::Subtitle, (17.0 * s).round()),
        Size3::Large => (Font::TitleLg, Font::SubtitleLg, (19.0 * s).round()),
    }
}

/// Texto da segunda linha do widget, conforme a opção.
pub fn widget_second_line(m: &MediaInfo, cfg: &Config, pos: f64) -> String {
    if !cfg.show_artist || m.title.is_empty() {
        return String::new();
    }
    let artist = if m.artist.is_empty() {
        if cfg.show_source { m.app.clone() } else { String::new() }
    } else {
        m.artist.clone()
    };
    match cfg.second_line {
        SecondLine::Artist => artist,
        SecondLine::ArtistAlbum => {
            let a = artist_album(m);
            if a.is_empty() { artist } else { a }
        }
        SecondLine::Album => {
            let al = m.album.trim();
            if al.is_empty() || al == m.title.trim() { artist } else { al.to_string() }
        }
        SecondLine::Time if m.dur > 0.0 => format!("{} / {}", fmt_time(pos), fmt_time(m.dur)),
        SecondLine::Time => artist,
    }
}

/// Título e segunda linha do widget dentro de `r` (com opacidade `alpha`).
fn widget_text(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, m: &MediaInfo, f: &WidgetFrame, alpha: f32) {
    let (s, th, cfg) = (f.s, &f.theme, f.cfg);
    let title = if !m.present() {
        crate::i18n::tr("Nada tocando").to_string()
    } else if m.title.is_empty() {
        m.app.clone()
    } else {
        m.title.clone()
    };
    let sub = widget_second_line(m, cfg, f.pb.pos);
    let (ft, fs, line) = widget_fonts(cfg, s);
    let scroll = cfg.marquee && f.pb.playing;
    if sub.is_empty() {
        let rr = RectF::new(r.x, (r.y + (r.h - line) / 2.0).round(), r.w, line);
        marquee(cv, tr, &title, ft, rr, th.fg, alpha, s, f.time, scroll);
    } else {
        let top = (r.y + (r.h - 2.0 * line) / 2.0).round();
        marquee(cv, tr, &title, ft, RectF::new(r.x, top, r.w, line), th.fg, alpha, s, f.time, scroll);
        text(cv, tr, &sub, fs, r.x, top + line, r.w, line, th.fg, 0.68 * alpha);
    }
}

/// Cor do fundo do widget conforme a opção.
fn widget_tint(cfg: &Config, th: &Theme, m: &MediaInfo) -> Rgb {
    match cfg.widget_tint {
        WidgetTint::Neutral => th.fg,
        WidgetTint::Album => m.art.as_ref().map(|a| readable(a.vibrant, th.dark)).unwrap_or(th.accent),
        WidgetTint::Accent => th.accent,
    }
}

pub fn draw_widget(cv: &mut Canvas, tr: &mut TextRenderer, l: &WLayout, f: &WidgetFrame) {
    let (s, th, m, cfg) = (f.s, &f.theme, f.media, f.cfg);
    // Fundo único com tudo dentro; o card aberto deixa o fundo um pouco mais forte.
    if let Some(bg) = l.bg {
        let a = cfg.widget_opacity as f32 / 100.0 + if f.flyout_open { 0.04 } else { 0.0 };
        cv.fill_round(bg, l.radius, widget_tint(cfg, th, m), a.min(1.0));
        if cfg.widget_border {
            cv.stroke_round(bg, l.radius, s, th.fg, if th.dark { 0.08 } else { 0.10 });
        }
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
        cv.fill_rrect(x - 0.5 * s, y0, x + 0.5 * s, y1, 0.5 * s, 0.14, th.fg);
    }

    let badge = cfg.show_source.then_some((13.0 * s).round());
    let art_change = f.change.filter(|c| c.2 < 1.0).map(|c| (c.0, c.2));
    let text_change = f.change.filter(|c| c.1 < 1.0).map(|c| (c.0, c.1));
    if let Some(r) = l.art {
        match art_change {
            // Troca de música: a capa nova cresce um pouco e aparece sobre a antiga.
            Some((old, t)) => {
                let e = ease_enter(t);
                draw_art_a(cv, tr, r, old, th, cfg.cover_shape, s, None, 1.0 - e * e);
                let k = 0.82 + 0.18 * e;
                let rr = RectF::new(r.cx() - r.w * k / 2.0, r.cy() - r.h * k / 2.0, r.w * k, r.h * k);
                draw_art_a(cv, tr, rr, m, th, cfg.cover_shape, s, badge.filter(|_| t > 0.98), e);
            }
            None => draw_art(cv, tr, r, m, th, cfg.cover_shape, s, badge),
        }
    }
    if let Some(r) = l.text {
        match text_change {
            // Texto antigo sobe e some; o novo entra de baixo.
            Some((old, t)) => {
                // O antigo sai primeiro; o novo entra logo atrás (pouca sobreposição).
                let gone = ease_exit((t / 0.4).min(1.0));
                let e = ease_enter(((t - 0.22) / 0.78).clamp(0.0, 1.0));
                let d = 12.0 * s;
                let clip = cv.clip_rect();
                cv.set_clip(clip.intersect(RectF::new(r.x - 2.0 * s, r.y + 2.0 * s, r.w + 4.0 * s, r.h - 4.0 * s)));
                widget_text(cv, tr, r.offset(0.0, (-d * gone).round()), old, f, 1.0 - gone);
                widget_text(cv, tr, r.offset(0.0, (d * (1.0 - e)).round()), m, f, e);
                cv.set_clip(clip);
            }
            None => widget_text(cv, tr, r, m, f, 1.0),
        }
        if cfg.taskbar_progress && m.dur > 0.0 {
            let (x0, x1) = (r.x, r.right());
            let y = r.bottom() - 6.0 * s;
            let t = (f.pb.pos / m.dur) as f32;
            cv.fill_rrect(x0, y, x1, y + 2.0 * s, s, 0.16, th.fg);
            let xv = x0 + (x1 - x0) * t.clamp(0.0, 1.0);
            if xv > x0 {
                cv.fill_rrect(x0, y, xv, y + 2.0 * s, s, 1.0, th.accent);
            }
        }
    }
    if let Some(r) = l.indicator {
        eq_indicator(cv, r, th.accent, f.pb.playing, f.time, s, if f.pb.playing { 1.0 } else { 0.5 });
    }

    let dim = |e: bool| if e { 1.0 } else { 0.3 };
    // Aleatório/repetir: apagados quando o player não deixa mudar pelo Windows.
    if let Some(r) = l.shuffle {
        toggle_glyph(cv, tr, r, G_SHUFFLE, f.pb.shuffle, m.can_shuffle, th, s, &f.it, Hit::Shuffle);
    }
    if let Some(r) = l.prev {
        ctrl_glyph(cv, tr, r, G_PREV, Font::IconSmall, th.fg, dim(m.can_prev), &f.it, Hit::Prev, s);
    }
    if let Some(r) = l.play {
        match cfg.taskbar_play {
            TaskbarPlay::Filled => play_button(cv, tr, r, (24.0 * s).min(r.h - 6.0 * s), f.cfg.play_shape, th, &f.it, f.pb.playing, m.can_toggle, s),
            TaskbarPlay::Icon => play_glyph(cv, tr, r, f.pb.playing, Font::Icon, th.fg, dim(m.can_toggle), &f.it, s),
        }
    }
    if let Some(r) = l.next {
        ctrl_glyph(cv, tr, r, G_NEXT, Font::IconSmall, th.fg, dim(m.can_next), &f.it, Hit::Next, s);
    }
    if let Some(r) = l.repeat {
        toggle_glyph(cv, tr, r, if f.pb.repeat == 1 { G_REPEAT_ONE } else { G_REPEAT_ALL }, f.pb.repeat != 0, m.can_repeat, th, s, &f.it, Hit::Repeat);
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

/// Selo com ícone + nome do app de origem ("de onde vem a música").
/// `align_right`: encosta a pílula na direita de `r`. Devolve o retângulo usado.
pub fn app_chip(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, media: &MediaInfo, th: &Theme, s: f32, on_art: bool, align_right: bool, alpha: f32) -> Option<RectF> {
    if media.app.is_empty() || r.w < 40.0 * s || alpha <= 0.0 {
        return None;
    }
    let name_w = tr.mask(&media.app, Font::CaptionStrong, (240.0 * s) as i32, r.h as i32, Align::Left).used_width();
    let icon = (r.h - 8.0 * s).clamp(10.0 * s, 15.0 * s).round();
    let pad = ((r.h - icon) / 2.0).max(3.0 * s);
    let w = (pad + icon + 6.0 * s + name_w + 9.0 * s).min(r.w).round();
    let pill = RectF::new(if align_right { r.right() - w } else { r.x }, r.y, w, r.h);
    let rad = r.h / 2.0;
    if on_art {
        cv.fill_round(pill, rad, [0.0; 3], 0.42 * alpha);
        cv.stroke_round(pill, rad, s, [1.0; 3], 0.16 * alpha);
    } else {
        cv.fill_round(pill, rad, th.fg, (if th.dark { 0.09 } else { 0.06 }) * alpha);
        cv.stroke_round(pill, rad, s, th.fg, (if th.dark { 0.10 } else { 0.09 }) * alpha);
    }
    let fg = if on_art { [1.0; 3] } else { th.fg };
    let ic = RectF::new(pill.x + pad, (pill.cy() - icon / 2.0).round(), icon, icon);
    match &media.icon {
        Some(i) => cv.draw_image(ic, i, icon * 0.24, alpha),
        None => {
            cv.fill_circle(ic.cx(), ic.cy(), icon / 2.0, th.accent, alpha);
            glyph(cv, tr, ic, G_MUSIC, Font::Small, on_color(th.accent), alpha);
        }
    }
    let x = ic.right() + 6.0 * s;
    text(cv, tr, &media.app, Font::CaptionStrong, x, pill.y, pill.right() - x - 7.0 * s, pill.h, fg, 0.92 * alpha);
    Some(pill)
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
pub(crate) fn art_shadow(cv: &mut Canvas, r: RectF, radius: f32, art: &Image, th: &Theme, s: f32) {
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
        app_chip(cv, tr, l.chip, m, th, s, l.vertical && m.art.is_some(), false, 1.0);
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
        // Com o selo do app em cima não sobra espaço: "Artista · Álbum" numa linha só.
        let sub = artist_album(m);
        let album = (sub != m.artist && !m.artist.is_empty() && !cfg.show_source).then(|| m.album.trim());
        let dy = if album.is_some() { -8.0 * s } else { 0.0 };
        let (t, a) = (l.title.offset(0.0, dy), l.artist.offset(0.0, dy));
        text(cv, tr, &title, Font::CardTitle, t.x, t.y, t.w, t.h, th.fg, 1.0);
        text(cv, tr, if m.artist.is_empty() || cfg.show_source { &sub } else { &m.artist }, Font::Subtitle, a.x, a.y, a.w, a.h, th.fg, 0.72);
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
        toggle_glyph(cv, tr, l.shuffle, G_SHUFFLE, f.pb.shuffle, m.can_shuffle, th, s, &f.it, Hit::Shuffle);
    }
    ctrl_glyph(cv, tr, l.prev, G_PREV, Font::Icon, th.fg, dim(m.can_prev), &f.it, Hit::Prev, s);
    play_button(cv, tr, l.play, l.play_size, cfg.play_shape, th, &f.it, f.pb.playing, m.can_toggle, s);
    ctrl_glyph(cv, tr, l.next, G_NEXT, Font::Icon, th.fg, dim(m.can_next), &f.it, Hit::Next, s);
    if m.can_repeat || always {
        toggle_glyph(cv, tr, l.repeat, if f.pb.repeat == 1 { G_REPEAT_ONE } else { G_REPEAT_ALL }, f.pb.repeat != 0, m.can_repeat, th, s, &f.it, Hit::Repeat);
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
        ctrl_glyph(cv, tr, b, g, Font::IconSmall, th.fg, 0.9, &f.it, Hit::VolBtn, s);
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
    let _ = tx;
    let top = slot.y;

    // Selo "A SEGUIR": pílula preenchida na cor de destaque, com brilho suave
    // e o mini equalizador; entra crescendo da esquerda.
    let label = crate::i18n::tr("A SEGUIR");
    let lh = 19.0 * s;
    let lw = tr.mask(label, Font::Label, (160.0 * s) as i32, lh as i32, Align::Left).used_width();
    let grow = ease_enter(((t - 0.05) / 0.45).clamp(0.0, 1.0));
    let full_w = 27.0 * s + lw + 10.0 * s;
    let badge = RectF::new(tx0, top - 1.5 * s, (lh + (full_w - lh) * grow).round(), lh);
    let ba = ((t - 0.05) / 0.2).clamp(0.0, 1.0);
    let on = th.on_accent;
    cv.glow(badge, lh / 2.0, 10.0 * s, 2.0 * s, th.accent, 0.38 * ba);
    let (by0, bh) = (badge.y, badge.h);
    let (c0, c1) = (lerp3(th.accent, [1.0; 3], 0.12), lerp3(th.accent, [0.0; 3], 0.10));
    cv.fill_rrect_ca(badge.x, badge.y, badge.right(), badge.bottom(), lh / 2.0, |_, y| (lerp3(c0, c1, ((y - by0) / bh).clamp(0.0, 1.0)), ba));
    cv.stroke_round(badge, lh / 2.0, s, [1.0; 3], 0.14 * ba);
    eq_indicator(cv, RectF::new(badge.x + 7.5 * s, badge.y - 0.5 * s, 13.0 * s, lh), on, true, t, s, ba);
    let clip = cv.clip_rect();
    cv.set_clip(clip.intersect(badge));
    text(cv, tr, label, Font::Label, badge.x + 25.5 * s, badge.y, lw + 4.0 * s, lh, on, ba * grow);
    cv.set_clip(clip);
    if cfg.show_source {
        let avail = RectF::new(tx0 + full_w + 10.0 * s, badge.y, (right - tx0 - full_w - 10.0 * s).max(0.0), lh);
        app_chip(cv, tr, avail, media, th, s, false, true, ta);
    }

    let title = if media.title.is_empty() { media.app.clone() } else { media.title.clone() };
    text(cv, tr, &title, Font::CardTitle, tx, top + 20.0 * s, tw, 22.0 * s, th.fg, ta);
    text(cv, tr, &artist_album(media), Font::Subtitle, tx, top + 42.0 * s, tw, 17.0 * s, th.fg, ta * 0.72);

    // Tempo restante do cartão: a base da barrinha encosta na base da capa.
    let hold = ((t - 0.45) / (total - 0.85).max(0.5)).clamp(0.0, 1.0);
    let (bx0, bx1) = (tx0, right);
    let by = slot.bottom() - 3.0 * s;
    cv.fill_rrect(bx0, by, bx1, by + 3.0 * s, 1.5 * s, if th.dark { 0.18 } else { 0.12 }, th.fg);
    let xe = bx0 + (bx1 - bx0) * (1.0 - hold);
    if xe > bx0 + 1.0 {
        cv.fill_rrect(bx0, by, xe, by + 3.0 * s, 1.5 * s, 0.95, th.accent);
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
        (OsdMaterial::Acrylic, Some(b)) => {
            // Sem a sombra por baixo: no Mac o fundo é translúcido (desfoque do sistema).
            cv.erase_round(card, radius);
            acrylic(cv, card, radius, b, base, opacity * 0.78)
        }
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

/// Curva de Bézier cúbica de (0,0) a (1,1), como `cubic-bezier()` do CSS (as
/// curvas de movimento do Fluent são definidas assim).
pub fn bezier(t: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 || t >= 1.0 {
        return t;
    }
    let c = |a: f32, b: f32, u: f32| {
        let v = 1.0 - u;
        3.0 * v * v * u * a + 3.0 * v * u * u * b + u * u * u
    };
    let d = |a: f32, b: f32, u: f32| {
        let v = 1.0 - u;
        3.0 * v * v * a + 6.0 * v * u * (b - a) + 3.0 * u * u * (1.0 - b)
    };
    let mut u = t;
    for _ in 0..8 {
        let e = c(x1, x2, u) - t;
        if e.abs() < 1e-4 {
            return c(y1, y2, u);
        }
        let dv = d(x1, x2, u);
        if dv.abs() < 1e-6 {
            break;
        }
        u = (u - e / dv).clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let m = (lo + hi) / 2.0;
        if c(x1, x2, m) < t {
            lo = m;
        } else {
            hi = m;
        }
    }
    c(y1, y2, (lo + hi) / 2.0)
}

/// Entradas (Fluent "decelerate"): sai rápido e assenta devagar.
pub fn ease_enter(t: f32) -> f32 {
    bezier(t, 0.1, 0.9, 0.2, 1.0)
}

/// Saídas (Fluent "accelerate"): começa devagar e some rápido.
pub fn ease_exit(t: f32) -> f32 {
    bezier(t, 0.7, 0.0, 1.0, 0.5)
}

/// Movimento de um ponto a outro (Fluent "standard"), ex.: indicador que desliza.
pub fn ease_move(t: f32) -> f32 {
    bezier(t, 0.55, 0.0, 0.1, 1.0)
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
    // 384 px com bordas suaves: nítida também nas prévias e capturas em 2x.
    const N: usize = 384;
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

    #[test]
    fn icon_anims_follow_state_changes() {
        let mut a = IconAnims::default();
        let pb = Playback { playing: true, ..Default::default() };
        // Primeiro estado visto: nada anima (a música só apareceu).
        a.observe(&pb);
        assert!(!a.running());
        a.observe(&Playback { playing: false, ..pb });
        assert!(a.running());
        let fx = a.fx(true);
        assert!(fx.progress(Hit::Play).is_some());
        assert!(fx.progress(Hit::Shuffle).is_none());
        // Opção desligada: nenhum ícone se mexe.
        assert!(a.fx(false).progress(Hit::Play).is_none());
        let it = Interact { fx: a.fx(false), pressed: Some(Hit::Prev), hover: Some(Hit::Prev), ..Default::default() };
        assert_eq!(pose(&it, Hit::Prev, 1.0), Pose::REST);
    }

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
