//! Widget no notch do MacBook: desenho puro (sem janela), como `ui.rs`.
//!
//! Uma "ilha" preta presa no topo da tela, emendada no notch, em três formas:
//! - **fechada**: capa à esquerda do notch e barras de áudio (ou o indicador
//!   "tocando agora") à direita; em telas sem notch o título fica no meio;
//! - **aberta** (mouse em cima ou clique): cresce para baixo com a capa grande,
//!   título, progresso, botões e volume;
//! - **"A seguir"**: abre pela metade com a próxima música e fecha sozinha.
//!
//! A janela (`mac/notch.rs`) só anima `open`/`peek` e mostra o quadro; as
//! prévias das configurações e as capturas usam as mesmas funções.

// No Windows o módulo só é usado pelas capturas e pelos GIFs.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use crate::config::{AccentSource, Config, NotchBg, ShuffleRepeat};
use crate::render::*;
use crate::shared::MediaInfo;
use crate::text::*;
use crate::ui::{self, Hit, Interact, Playback, Theme};

/// Margem da sombra do card aberto (dos lados e embaixo), em pontos.
pub const SHADOW: f32 = 24.0;
const OPEN_W: f32 = 460.0;
const PEEK_W: f32 = 392.0;
/// Largura do título no meio da ilha em telas sem notch.
const CENTER_W: f32 = 190.0;

/// Notch da tela, em pixels físicos. Largura 0 = tela sem notch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geom {
    pub notch_w: f32,
    pub notch_h: f32,
}

#[derive(Clone, Debug, Default)]
pub struct NLayout {
    pub s: f32,
    /// Tamanho da superfície (cabe o card aberto com a sombra).
    pub w: f32,
    pub h: f32,
    /// Altura da faixa do notch (o conteúdo aberto começa abaixo dela).
    pub band: f32,
    pub has_notch: bool,
    pub notch: RectF,
    pub closed: RectF,
    pub open: RectF,
    pub peek: RectF,
    // ---- fechada
    pub c_art: RectF,
    pub c_right: RectF,
    pub c_title: Option<RectF>,
    // ---- aberta
    pub art: RectF,
    pub title: RectF,
    pub artist: RectF,
    pub viz: RectF,
    pub seek: Option<RectF>,
    pub shuffle: RectF,
    pub prev: RectF,
    pub play: RectF,
    pub play_size: f32,
    pub next: RectF,
    pub repeat: RectF,
    pub vol_btn: Option<RectF>,
    pub vol: Option<RectF>,
    // ---- "A seguir"
    pub p_art: RectF,
    pub p_text: RectF,
    // ---- saídas de áudio (setinha embaixo do card aberto)
    /// Setinha ligada (opção "Mixer de volume").
    pub mix_on: bool,
    /// Quanto a lista das saídas está aberta (0..1), atualizado a cada quadro.
    pub mix_p: f32,
    /// Onde a lista começa, a altura dela aberta e as linhas.
    pub mix_top: f32,
    pub mix_full: f32,
    pub mix_rows: Vec<RectF>,
}

impl NLayout {
    pub fn build(s: f32, g: Geom, cfg: &Config, has_seek: bool) -> Self {
        let band = g.notch_h.max(22.0 * s).round();
        let has_notch = g.notch_w > 1.0;
        let wing = (band + 8.0 * s).round();
        let center = if has_notch { g.notch_w } else { CENTER_W * s };
        let closed_w = center + 2.0 * wing;
        let open_w = (OPEN_W * s).max(closed_w + 48.0 * s).round();
        let pad = 20.0 * s;

        // Alturas do card aberto.
        let top = band + 12.0 * s;
        let art = 72.0 * s;
        let controls_y = top + art + 8.0 * s;
        let controls_h = 46.0 * s;
        let mut y = controls_y + controls_h;
        let vol_y = y + 2.0 * s;
        if cfg.show_volume {
            y = vol_y + 30.0 * s;
        }
        let open_h = (y + 12.0 * s).round();
        let peek_w = (PEEK_W * s).max(closed_w + 40.0 * s).round();
        let peek_h = (band + 8.0 * s + 64.0 * s + 14.0 * s).round();

        let m = SHADOW * s;
        let w = open_w + 2.0 * m;
        let h = open_h + m;
        let cx = (w / 2.0).round();
        let mut l = NLayout { s, w, h, band, has_notch, ..Default::default() };
        l.notch = RectF::new(cx - g.notch_w / 2.0, 0.0, if has_notch { g.notch_w } else { 0.0 }, band);
        l.closed = RectF::new((cx - closed_w / 2.0).round(), 0.0, closed_w.round(), band);
        l.open = RectF::new(cx - open_w / 2.0, 0.0, open_w, open_h);
        l.peek = RectF::new(cx - peek_w / 2.0, 0.0, peek_w, peek_h);

        // Fechada: capa centrada na asa esquerda, barras na direita.
        let a = (band - 12.0 * s).max(14.0 * s).round();
        l.c_art = RectF::new((l.closed.x + (wing - a) / 2.0 + 3.0 * s).round(), ((band - a) / 2.0).round(), a, a);
        let rw = (wing - 16.0 * s).round();
        let rh = (a * 0.78).round();
        l.c_right = RectF::new((l.closed.right() - wing + (wing - rw) / 2.0 - 3.0 * s).round(), ((band - rh) / 2.0).round(), rw, rh);
        if !has_notch {
            l.c_title = Some(RectF::new(l.closed.x + wing, 0.0, center, band));
        }

        // Aberta.
        let ox = l.open.x;
        l.art = RectF::new(ox + pad, top, art, art);
        let col_x = l.art.right() + 16.0 * s;
        let col_w = l.open.right() - pad - col_x;
        let viz_w = 40.0 * s;
        l.viz = RectF::new(l.open.right() - pad - viz_w, top + 5.0 * s, viz_w, 24.0 * s);
        let text_w = col_w - viz_w - 10.0 * s;
        let seek = has_seek && cfg.card_seek;
        // Título e artista centralizados na capa quando não há progresso.
        let ty = if seek { top + 2.0 * s } else { top + (art - 43.0 * s) / 2.0 };
        l.title = RectF::new(col_x, ty.round(), text_w, 24.0 * s);
        l.artist = RectF::new(col_x, (ty + 24.0 * s).round(), text_w, 19.0 * s);
        if seek {
            l.seek = Some(RectF::new(col_x, top + art - 26.0 * s, col_w, 26.0 * s));
        }
        let slot = 58.0 * s;
        let x0 = l.open.cx() - 2.5 * slot;
        let sl = |k: f32| RectF::new((x0 + k * slot).round(), controls_y, slot, controls_h);
        (l.shuffle, l.prev, l.play, l.next, l.repeat) = (sl(0.0), sl(1.0), sl(2.0), sl(3.0), sl(4.0));
        l.play_size = 40.0 * s;
        if cfg.show_volume {
            l.vol_btn = Some(RectF::new(ox + pad - 7.0 * s, vol_y, 30.0 * s, 30.0 * s));
            let vx = ox + pad + 28.0 * s;
            l.vol = Some(RectF::new(vx, vol_y, l.open.right() - pad - vx, 30.0 * s));
        }

        // "A seguir".
        l.p_art = RectF::new(l.peek.x + 16.0 * s, band + 8.0 * s, 64.0 * s, 64.0 * s);
        let px = l.p_art.right() + 14.0 * s;
        l.p_text = RectF::new(px, l.p_art.y, l.peek.right() - 18.0 * s - px, l.p_art.h);
        l
    }

    /// Acrescenta a setinha embaixo do card aberto, que abre a lista de `n`
    /// saídas de áudio (`None`: opção desligada). A janela já fica com a
    /// altura da lista aberta (fora da ilha visível ela deixa o mouse passar).
    pub fn with_mixer(mut self, n: Option<usize>) -> Self {
        let Some(n) = n else { return self };
        let s = self.s;
        let pad = 20.0 * s;
        self.mix_on = true;
        self.mix_top = self.open.h - 8.0 * s;
        let rows = n.clamp(1, crate::shared::MIXER_ROWS);
        self.mix_full = (30.0 * s + rows as f32 * 32.0 * s + 6.0 * s).round();
        self.mix_rows = (0..n.min(crate::shared::MIXER_ROWS))
            .map(|i| RectF::new(self.open.x + pad - 6.0 * s, self.mix_top + 30.0 * s + i as f32 * 32.0 * s, self.open.w - 2.0 * pad + 12.0 * s, 30.0 * s))
            .collect();
        self.open.h += 18.0 * s;
        self.h = self.h.max(self.open.h + self.mix_full + SHADOW * s);
        self
    }

    /// O card aberto com a lista das saídas aberta até `mix_p`.
    pub fn open_rect(&self) -> RectF {
        RectF { h: self.open.h + self.mix_full * self.mix_p.clamp(0.0, 1.0), ..self.open }
    }

    /// A setinha (desce junto com a lista abrindo).
    pub fn mix_btn(&self) -> Option<RectF> {
        let s = self.s;
        self.mix_on.then(|| RectF::new(self.open.cx() - 24.0 * s, self.mix_top + self.mix_full * self.mix_p.clamp(0.0, 1.0) + 2.0 * s, 48.0 * s, 18.0 * s))
    }

    /// O que está sob o mouse. Com a ilha fechada, tudo nela é `Info` (abre).
    pub fn hit(&self, x: f32, y: f32, open: bool) -> Option<Hit> {
        if !open {
            return self.closed.inset(-2.0 * self.s).contains(x, y).then_some(Hit::Info);
        }
        if self.mix_btn().is_some_and(|b| b.contains(x, y)) {
            return Some(Hit::MixToggle);
        }
        if self.mix_p > 0.5
            && let Some(i) = self.mix_rows.iter().position(|r| r.contains(x, y))
        {
            return Some(Hit::Output(i as u8));
        }
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
        self.open_rect().contains(x, y).then_some(Hit::Info)
    }

    pub fn vol_track(&self) -> Option<(f32, f32)> {
        self.vol.map(|r| (r.x + 4.0 * self.s, r.right() - 1.0 * self.s))
    }

    pub fn seek_track(&self) -> Option<(f32, f32)> {
        self.seek.map(|r| (r.x + 1.0 * self.s, r.right() - 1.0 * self.s))
    }

    /// Forma da ilha para o progresso dado (aberta ou "A seguir").
    pub fn shape(&self, open: f32, peek: f32) -> (RectF, f32) {
        let (target, p) = if open >= peek { (self.open_rect(), open) } else { (self.peek, peek) };
        let r = lerp_rect(self.closed, target, p);
        let rc = (self.band * 0.36).round();
        let ro = 22.0 * self.s;
        (r, rc + (ro - rc) * p.clamp(0.0, 1.0))
    }
}

/// Interpolação que aceita `t` um pouco acima de 1 (o "quique" da mola).
fn lerp_rect(a: RectF, b: RectF, t: f32) -> RectF {
    RectF::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, a.w + (b.w - a.w) * t, a.h + (b.h - a.h) * t)
}

/// Crescimento com um quique leve (mola amortecida): usado ao abrir.
pub fn grow(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let k = 0.9;
    1.0 + (1.0 + k) * (t - 1.0).powi(3) + k * (t - 1.0).powi(2)
}

/// Fechamento sem quique.
pub fn shrink(t: f32) -> f32 {
    ui::ease_move(t.clamp(0.0, 1.0))
}

/// Cores da ilha: sempre escura (emenda no notch preto).
pub fn theme(cfg: &Config, colors: &Colors, media: &MediaInfo) -> Theme {
    let accent = match (cfg.accent_source, &media.art) {
        (AccentSource::Album, Some(a)) => readable(a.vibrant, true),
        _ => colors.accent_light,
    };
    Theme { fg: [1.0; 3], accent, on_accent: on_color(accent), dark: true }
}

pub struct NotchFrame<'a> {
    pub cfg: &'a Config,
    pub colors: &'a Colors,
    pub media: &'a MediaInfo,
    pub theme: Theme,
    pub it: Interact,
    pub pb: Playback,
    pub time: f32,
    pub viz: Option<(&'a [f32], &'a [f32])>,
    /// Abertura já com a curva aplicada (0 = fechada, 1 = aberta; pode passar de 1 no quique).
    pub open: f32,
    /// "A seguir": abertura (com a curva), segundos desde que apareceu e duração.
    pub peek: f32,
    pub peek_t: f32,
    pub peek_total: f32,
    /// Aviso rápido no card aberto: (texto, opacidade).
    pub hint: Option<(&'a str, f32)>,
    /// Troca de música: (música anterior, progresso 0..1).
    pub change: Option<(&'a MediaInfo, f32)>,
    /// Saídas de áudio (nome, é a atual) da lista que a setinha abre.
    pub outputs: &'a [(String, bool)],
}

pub fn draw(cv: &mut Canvas, tr: &mut TextRenderer, l: &NLayout, f: &NotchFrame) {
    let (s, th, m, cfg) = (l.s, &f.theme, f.media, f.cfg);
    let (shape, radius) = l.shape(f.open, f.peek);
    let p = f.open.max(f.peek).clamp(0.0, 1.0);
    let band = l.band;

    // Sombra só com a ilha aberta (fechada ela é parte do notch).
    if p > 0.01 {
        cv.drop_shadow(RectF::new(shape.x, shape.y - radius, shape.w, shape.h + radius).inset(2.0 * s), radius, 26.0 * s, 10.0 * s, 0.55 * p);
    }
    // Fundo preto: cantos de baixo arredondados, os de cima colados no topo da tela.
    cv.fill_rrect(shape.x, shape.y - radius, shape.right(), shape.bottom(), radius, 1.0, [0.0; 3]);
    // "Orelhas" côncavas nos cantos de cima, como as do próprio notch.
    let ear = (5.0 + 5.0 * p) * s;
    for (x, dir) in [(shape.x, -1.0f32), (shape.right(), 1.0)] {
        // Quadradinho preto menos um círculo encostado nele (sem apagar a sombra).
        let ex = if dir < 0.0 { x - ear } else { x };
        let (cx, cy) = (x + dir * ear, ear);
        cv.fill_rrect_ca(ex, 0.0, ex + ear, ear, 0.0, |px, py| {
            let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            ([0.0; 3], (d - ear + 0.5).clamp(0.0, 1.0))
        });
    }

    // Cor por baixo do conteúdo aberto (o topo continua preto).
    if p > 0.01 {
        let (y0, y1) = (band, shape.bottom());
        let ramp = move |y: f32| ((y - y0) / (y1 - y0).max(1.0)).clamp(0.0, 1.0);
        match (cfg.notch_bg, &m.art) {
            (NotchBg::Album, _) => {
                let base = m.art.as_ref().map(|a| a.vibrant).unwrap_or(th.accent);
                let c = lerp3(readable(base, true), [0.0; 3], 0.42);
                cv.fill_rrect_ca(shape.x, shape.y, shape.right(), shape.bottom(), radius, |_, y| (c, 0.85 * p * ramp(y).powf(0.8)));
            }
            (NotchBg::Artwork, Some(art)) => {
                let b = ui::blurred(art);
                let clip = cv.clip_rect();
                cv.set_clip(clip.intersect(RectF::new(shape.x, band, shape.w, shape.h)));
                cv.draw_image_cover(shape, &b, radius, 0.9 * p);
                cv.set_clip(clip);
                cv.fill_rrect_ca(shape.x, shape.y, shape.right(), shape.bottom(), radius, |_, y| ([0.0; 3], 1.0 - 0.62 * ramp(y).powf(0.7)));
            }
            _ => {}
        }
        // Contorno sutil só embaixo e dos lados (separa do papel de parede escuro).
        let clip = cv.clip_rect();
        cv.set_clip(clip.intersect(RectF::new(shape.x - 2.0, band, shape.w + 4.0, shape.h)));
        cv.stroke_round(RectF::new(shape.x, shape.y - radius, shape.w, shape.h + radius).inset(-0.5 * s), radius, s, [1.0; 3], 0.07 * p);
        cv.set_clip(clip);
    }

    let open_a = ((f.open - 0.45) / 0.55).clamp(0.0, 1.0);
    let peek_a = ((f.peek - 0.45) / 0.55).clamp(0.0, 1.0) * (1.0 - open_a);
    let closed_a = (1.0 - p * 3.0).clamp(0.0, 1.0);

    // Capa: sai da asa esquerda e vai para o lugar dela no card (ou no "A seguir").
    let target = if f.open >= f.peek { l.art } else { l.p_art };
    let art_r = lerp_rect(l.c_art, target, p);
    let rad = (l.c_art.w * 0.24) + (12.0 * s - l.c_art.w * 0.24) * p;
    if p > 0.5 {
        if let Some(a) = &m.art {
            cv.glow(art_r, rad, 22.0 * s, 7.0 * s, a.vibrant, 0.40 * (p - 0.5) * 2.0);
        }
    }
    match f.change {
        Some((old, t)) if t < 1.0 => {
            let e = ui::ease_enter(t);
            art(cv, tr, art_r, rad, old, th, s, 1.0 - e * e);
            let k = 0.82 + 0.18 * e;
            let rr = RectF::new(art_r.cx() - art_r.w * k / 2.0, art_r.cy() - art_r.h * k / 2.0, art_r.w * k, art_r.h * k);
            art(cv, tr, rr, rad * k, m, th, s, e);
        }
        _ => art(cv, tr, art_r, rad, m, th, s, 1.0),
    }

    // ---- fechada
    if closed_a > 0.0 {
        right_wing(cv, l.c_right, f, closed_a, s);
        if let Some(r) = l.c_title {
            let title = title_of(m);
            ui::text_center(cv, tr, &title, Font::TitleSm, r.inset(6.0 * s), th.fg, 0.92 * closed_a);
        }
    }

    // ---- aberta
    if open_a > 0.0 {
        draw_open(cv, tr, l, f, open_a);
    }

    // ---- "A seguir"
    if peek_a > 0.0 {
        draw_peek(cv, tr, l, f, peek_a);
    }
}

fn title_of(m: &MediaInfo) -> String {
    if !m.present() {
        crate::i18n::tr("Nada tocando").to_string()
    } else if m.title.is_empty() {
        m.app.clone()
    } else {
        m.title.clone()
    }
}

/// Capa (ou nota musical) num retângulo arredondado.
fn art(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, radius: f32, m: &MediaInfo, th: &Theme, s: f32, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    match &m.art {
        Some(a) => {
            cv.draw_image_cover(r, a, radius, alpha);
            cv.stroke_round(r, radius, s.max(1.0), [1.0; 3], 0.10 * alpha);
        }
        None => {
            cv.fill_round(r, radius, th.fg, 0.14 * alpha);
            let font = if r.w > 48.0 * s { Font::IconXl } else if r.w > 26.0 * s { Font::Icon } else { Font::IconSmall };
            ui::glyph(cv, tr, r, G_MUSIC, font, th.fg, 0.85 * alpha);
        }
    }
}

/// Asa direita: barras de áudio ou, sem áudio, o indicador "tocando agora".
fn right_wing(cv: &mut Canvas, r: RectF, f: &NotchFrame, alpha: f32, s: f32) {
    match f.viz {
        Some((values, peaks)) if f.cfg.show_visualizer && f.media.present() => {
            let mut px = vec![0u32; (r.w.ceil() * r.h.ceil()) as usize];
            {
                let mut layer = Canvas::new(r.w.ceil() as usize, r.h.ceil() as usize, &mut px);
                crate::render::draw(&mut layer, RectF::new(0.0, 0.0, r.w, r.h), &Frame { style: f.cfg.style, values, peaks, show_peaks: f.cfg.show_peaks, colors: f.colors, scale: s });
            }
            cv.composite(r.x.round() as i32, r.y.round() as i32, &px, r.w.ceil() as usize, r.h.ceil() as usize, alpha);
        }
        _ if f.cfg.playing_indicator && f.media.present() => {
            ui::eq_indicator(cv, r, f.theme.accent, f.pb.playing, f.time, s, alpha * if f.pb.playing { 1.0 } else { 0.5 });
        }
        _ => {}
    }
}

fn draw_open(cv: &mut Canvas, tr: &mut TextRenderer, l: &NLayout, f: &NotchFrame, a: f32) {
    let (s, th, m, cfg) = (l.s, &f.theme, f.media, f.cfg);
    // Sobe um pouco ao aparecer.
    let dy = ((1.0 - a) * 6.0 * s).round();
    let title = title_of(m);
    let sub = ui::artist_album(m);
    match f.change {
        Some((old, t)) if t < 1.0 => {
            let gone = ui::ease_exit((t / 0.4).min(1.0));
            let e = ui::ease_enter(((t - 0.22) / 0.78).clamp(0.0, 1.0));
            let d = 10.0 * s;
            let clip = cv.clip_rect();
            cv.set_clip(clip.intersect(RectF::new(l.title.x - 2.0 * s, l.title.y, l.title.w + 4.0 * s, l.artist.bottom() - l.title.y)));
            text_pair(cv, tr, l, &title_of(old), &ui::artist_album(old), th, -d * gone + dy, a * (1.0 - gone));
            text_pair(cv, tr, l, &title, &sub, th, d * (1.0 - e) + dy, a * e);
            cv.set_clip(clip);
        }
        _ => text_pair(cv, tr, l, &title, &sub, th, dy, a),
    }

    // Barras (ou indicador) à direita do título.
    right_wing(cv, l.viz.offset(0.0, dy), f, a, s);

    // Progresso com os tempos embaixo.
    if let (Some(r), Some((x0, x1))) = (l.seek, l.seek_track()) {
        let r = r.offset(0.0, dy);
        let mut pos = f.pb.pos;
        let mut t = if m.dur > 0.0 { (pos / m.dur) as f32 } else { 0.0 };
        if let Some(p) = f.pb.seek_preview {
            t = p;
            pos = p as f64 * m.dur;
        }
        let hot = (f.it.hover == Some(Hit::Seek) || f.it.drag == Some(Hit::Seek)) && m.can_seek;
        slider(cv, x0, x1, r.y + 7.0 * s, t, th, hot, s, a);
        ui::text(cv, tr, &ui::fmt_time(pos), Font::Small, x0, r.y + 12.0 * s, 60.0 * s, 15.0 * s, th.fg, 0.6 * a);
        let rem = format!("-{}", ui::fmt_time((m.dur - pos).max(0.0)));
        ui::text_right(cv, tr, &rem, Font::Small, x1, r.y + 12.0 * s, 15.0 * s, th.fg, 0.6 * a, s);
    }

    // Botões.
    let always = cfg.card_shuffle_repeat == ShuffleRepeat::Always;
    let dim = |e: bool| if e { 1.0 } else { 0.3 };
    for h in [Hit::Shuffle, Hit::Prev, Hit::Next, Hit::Repeat, Hit::VolBtn] {
        if f.it.hover != Some(h) {
            continue;
        }
        let r = match h {
            Hit::Shuffle if m.can_shuffle || always => l.shuffle,
            Hit::Prev => l.prev,
            Hit::Next => l.next,
            Hit::Repeat if m.can_repeat || always => l.repeat,
            Hit::VolBtn => l.vol_btn.unwrap_or_default(),
            _ => continue,
        };
        let k = r.h.min(r.w) - 6.0 * s;
        let pressed = f.it.pressed == Some(h);
        cv.fill_circle(r.cx(), r.cy() + dy, k / 2.0, th.fg, if pressed { 0.08 } else { 0.12 } * a);
    }
    if a >= 0.999 {
        if m.can_shuffle || always {
            ui::toggle_glyph(cv, tr, l.shuffle, G_SHUFFLE, f.pb.shuffle, m.can_shuffle, th, s, &f.it, Hit::Shuffle);
        }
        ui::ctrl_glyph(cv, tr, l.prev, G_PREV, Font::IconLg, th.fg, dim(m.can_prev), &f.it, Hit::Prev, s);
        ui::play_button(cv, tr, l.play, l.play_size, cfg.play_shape, th, &f.it, f.pb.playing, m.can_toggle, s);
        ui::ctrl_glyph(cv, tr, l.next, G_NEXT, Font::IconLg, th.fg, dim(m.can_next), &f.it, Hit::Next, s);
        if m.can_repeat || always {
            ui::toggle_glyph(cv, tr, l.repeat, if f.pb.repeat == 1 { G_REPEAT_ONE } else { G_REPEAT_ALL }, f.pb.repeat != 0, m.can_repeat, th, s, &f.it, Hit::Repeat);
        }
    } else {
        // Durante a animação: só os ícones, esmaecendo (sem sombra do play).
        let o = |r: RectF| r.offset(0.0, dy);
        if m.can_shuffle || always {
            ui::glyph(cv, tr, o(l.shuffle), G_SHUFFLE, Font::IconSmall, th.fg, 0.8 * a * dim(m.can_shuffle));
        }
        ui::glyph(cv, tr, o(l.prev), G_PREV, Font::IconLg, th.fg, a * dim(m.can_prev));
        let pr = o(l.play);
        let k = l.play_size;
        let b = RectF::new(pr.cx() - k / 2.0, pr.cy() - k / 2.0, k, k);
        cv.fill_circle(b.cx(), b.cy(), k / 2.0, th.accent, a * dim(m.can_toggle));
        ui::glyph(cv, tr, b, if f.pb.playing { G_PAUSE } else { G_PLAY }, Font::Icon, th.on_accent, a);
        ui::glyph(cv, tr, o(l.next), G_NEXT, Font::IconLg, th.fg, a * dim(m.can_next));
        if m.can_repeat || always {
            ui::glyph(cv, tr, o(l.repeat), G_REPEAT_ALL, Font::IconSmall, th.fg, 0.8 * a * dim(m.can_repeat));
        }
    }

    // Volume.
    if let (Some(b), Some(r), Some((x0, x1))) = (l.vol_btn, l.vol, l.vol_track()) {
        let (b, r) = (b.offset(0.0, dy), r.offset(0.0, dy));
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
        ui::ctrl_glyph(cv, tr, b, g, Font::IconSmall, th.fg, 0.85 * a, &f.it, Hit::VolBtn, s);
        let hot = f.it.hover == Some(Hit::Vol) || f.it.drag == Some(Hit::Vol);
        slider(cv, x0, x1, r.cy(), if f.pb.muted { 0.0 } else { v }, th, hot, s, a);
    }

    // Saídas de áudio: a setinha embaixo e, aberta, a lista para escolher.
    if l.mix_on {
        let mp = l.mix_p.clamp(0.0, 1.0);
        if mp > 0.01 {
            let ma = a * ui::ease_enter(mp);
            let clip = cv.clip_rect();
            cv.set_clip(clip.intersect(RectF::new(l.open.x, l.mix_top + dy, l.open.w, l.mix_full * mp)));
            let top = l.mix_top + dy;
            cv.fill_rrect(l.open.x + 20.0 * s, top, l.open.right() - 20.0 * s, top + s, 0.5 * s, 0.10 * ma, th.fg);
            ui::text(cv, tr, crate::i18n::tr("SAÍDA DE ÁUDIO"), Font::Label, l.open.x + 20.0 * s, top + 6.0 * s, 200.0 * s, 22.0 * s, th.fg, 0.55 * ma);
            if f.outputs.is_empty() {
                ui::text(cv, tr, crate::i18n::tr("Nenhuma saída de áudio"), Font::Caption, l.open.x + 20.0 * s, top + 30.0 * s, 260.0 * s, 30.0 * s, th.fg, 0.55 * ma);
            }
            for (i, ((name, cur), r)) in f.outputs.iter().zip(&l.mix_rows).enumerate() {
                let r = r.offset(0.0, dy);
                let h = Hit::Output(i as u8);
                if f.it.hover == Some(h) || *cur {
                    cv.fill_round(r, 8.0 * s, th.fg, (if *cur { 0.08 } else { 0.06 }) * ma);
                }
                ui::glyph(cv, tr, RectF::new(r.x + 6.0 * s, r.y, 22.0 * s, r.h), G_VOL2, Font::IconSmall, if *cur { th.accent } else { th.fg }, 0.85 * ma);
                ui::text(cv, tr, name, Font::Caption, r.x + 36.0 * s, r.y, r.w - 72.0 * s, r.h, th.fg, (if *cur { 1.0 } else { 0.8 }) * ma);
                if *cur {
                    ui::check(cv, RectF::new(r.right() - 30.0 * s, r.y, 24.0 * s, r.h), th.accent, ma, s);
                }
            }
            cv.set_clip(clip);
        }
        if let Some(b) = l.mix_btn() {
            let b = b.offset(0.0, dy);
            let hot = f.it.hover == Some(Hit::MixToggle);
            if hot {
                cv.fill_round(b.inset(2.0 * s), b.h / 2.0, th.fg, 0.10 * a);
            }
            ui::chevron(cv, b, mp, th.fg, (if hot { 0.9 } else { 0.55 }) * a, s);
        }
    }

    // Aviso rápido (ex.: aleatório num player que não deixa).
    if let Some((msg, ha)) = f.hint {
        let ha = ha * a;
        let h = 26.0 * s;
        let tw = tr.mask(msg, Font::Caption, (l.open.w - 40.0 * s) as i32, h as i32, Align::Left).used_width();
        let w = (tw + 28.0 * s).min(l.open.w - 24.0 * s);
        let r = RectF::new((l.open.cx() - w / 2.0).round(), (l.shuffle.y - h - 2.0 * s).round(), w, h);
        cv.fill_round(r, h / 2.0, [0.16, 0.16, 0.17], 0.96 * ha);
        cv.stroke_round(r, h / 2.0, s, [1.0; 3], 0.12 * ha);
        ui::text_center(cv, tr, msg, Font::Caption, r, [1.0; 3], ha);
    }
}

fn text_pair(cv: &mut Canvas, tr: &mut TextRenderer, l: &NLayout, title: &str, sub: &str, th: &Theme, dy: f32, a: f32) {
    if a <= 0.0 {
        return;
    }
    let (t, ar) = (l.title.offset(0.0, dy.round()), l.artist.offset(0.0, dy.round()));
    ui::text(cv, tr, title, Font::CardTitle, t.x, t.y, t.w, t.h, th.fg, a);
    ui::text(cv, tr, sub, Font::Subtitle, ar.x, ar.y, ar.w, ar.h, th.fg, 0.68 * a);
}

/// Trilho fino do macOS: preenchido na cor de destaque; a bolinha só aparece com o mouse em cima.
fn slider(cv: &mut Canvas, x0: f32, x1: f32, cy: f32, t: f32, th: &Theme, hot: bool, s: f32, a: f32) {
    let h = if hot { 6.0 } else { 4.0 } * s;
    cv.fill_rrect(x0, cy - h / 2.0, x1, cy + h / 2.0, h / 2.0, 0.22 * a, th.fg);
    let xv = x0 + (x1 - x0) * t.clamp(0.0, 1.0);
    if xv > x0 + 0.5 {
        cv.fill_rrect(x0, cy - h / 2.0, xv.max(x0 + h), cy + h / 2.0, h / 2.0, a, if hot { th.accent } else { th.fg });
    }
    if hot {
        cv.drop_shadow(RectF::new(xv - 6.0 * s, cy - 6.0 * s, 12.0 * s, 12.0 * s), 6.0 * s, 4.0 * s, 1.0 * s, 0.3 * a);
        cv.fill_circle(xv, cy, 6.5 * s, [1.0; 3], a);
    }
}

fn draw_peek(cv: &mut Canvas, tr: &mut TextRenderer, l: &NLayout, f: &NotchFrame, a: f32) {
    let (s, th, m) = (l.s, &f.theme, f.media);
    let r = l.p_text;
    let t = f.peek_t;
    let slide = ((1.0 - ui::ease_out_cubic((t - 0.1) / 0.5)) * 10.0 * s).round();
    // Selo "A SEGUIR" com o mini equalizador.
    let label = crate::i18n::tr("A SEGUIR");
    let lh = 17.0 * s;
    let lw = tr.mask(label, Font::Label, (160.0 * s) as i32, lh as i32, Align::Left).used_width();
    let badge = RectF::new(r.x, r.y + 2.0 * s, (22.0 * s + lw + 9.0 * s).round(), lh);
    cv.fill_round(badge, lh / 2.0, th.accent, 0.95 * a);
    ui::eq_indicator(cv, RectF::new(badge.x + 5.5 * s, badge.y - 1.5 * s, 13.0 * s, lh), th.on_accent, true, t, s * 0.85, a);
    ui::text(cv, tr, label, Font::Label, badge.x + 20.0 * s, badge.y, lw + 4.0 * s, lh, th.on_accent, a);

    ui::text(cv, tr, &title_of(m), Font::Big, r.x + slide, r.y + 22.0 * s, r.w - slide, 21.0 * s, th.fg, a);
    ui::text(cv, tr, &ui::artist_album(m), Font::Subtitle, r.x + slide, r.y + 42.0 * s, r.w - slide, 17.0 * s, th.fg, 0.68 * a);

    // Tempo restante do "A seguir".
    let hold = ((t - 0.45) / (f.peek_total - 0.85).max(0.5)).clamp(0.0, 1.0);
    let (x0, x1) = (r.x, r.right());
    let y = r.bottom() - 3.0 * s;
    cv.fill_rrect(x0, y, x1, y + 3.0 * s, 1.5 * s, 0.18 * a, th.fg);
    let xe = x0 + (x1 - x0) * (1.0 - hold);
    if xe > x0 + 1.0 {
        cv.fill_rrect(x0, y, xe, y + 3.0 * s, 1.5 * s, 0.95 * a, th.accent);
    }
}

/// Prévia da ilha sobre uma tela de mentira (papel de parede, barra de menus e
/// notch de hardware): usada na tela de configurações e nas capturas.
pub fn preview(tr: &mut TextRenderer, s: f32, g: Geom, cfg: &Config, colors: &Colors, media: &MediaInfo, open: f32, peek: f32, time: f32) -> (usize, usize, Vec<u32>) {
    preview_sized(tr, s, g, cfg, colors, media, open, peek, time, None)
}

/// Como `preview`, numa imagem de tamanho fixo (`size` em pixels) com a ilha
/// centralizada no topo: as prévias das configurações aparecem 1:1, sem
/// reamostrar (nítidas em telas Retina).
pub fn preview_sized(tr: &mut TextRenderer, s: f32, g: Geom, cfg: &Config, colors: &Colors, media: &MediaInfo, open: f32, peek: f32, time: f32, size: Option<(usize, usize)>) -> (usize, usize, Vec<u32>) {
    let l = NLayout::build(s, g, cfg, true);
    let viz_v: Vec<f32> = (0..cfg.bars).map(|i| 0.35 + 0.6 * ((i as f32 * 1.7 + time * 3.0).sin() * 0.5 + 0.5)).collect();
    let peaks: Vec<f32> = viz_v.iter().map(|v| (v + 0.12).min(1.0)).collect();
    let colors = Colors { album: media.art.as_ref().map(|a| a.vibrant), ..*colors };
    let f = NotchFrame {
        cfg,
        colors: &colors,
        media,
        theme: theme(cfg, &colors, media),
        it: Interact::default(),
        pb: Playback { playing: true, shuffle: media.shuffle, repeat: media.repeat, pos: media.pos, seek_preview: None, volume: 0.62, muted: false },
        time,
        viz: Some((&viz_v, &peaks)),
        open,
        peek,
        peek_t: 2.0,
        peek_total: 5.0,
        hint: None,
        change: None,
        outputs: &[],
    };
    let (sw, sh) = (l.w.ceil() as usize, l.h.ceil() as usize);
    let mut layer = vec![0u32; sw * sh];
    draw(&mut Canvas::new(sw, sh, &mut layer), tr, &l, &f);

    let (iw, ih) = size.unwrap_or(((sw as f32 + 120.0 * s) as usize, sh + (24.0 * s) as usize));
    let mut px = vec![0u32; iw * ih];
    {
        let mut cv = Canvas::new(iw, ih, &mut px);
        let (fw, fh) = (iw as f32, ih as f32);
        cv.fill_rrect_ca(0.0, 0.0, fw, fh, 0.0, |x, y| {
            let u = x / fw;
            let v = y / fh;
            let base = lerp3([0.16, 0.36, 0.78], [0.62, 0.30, 0.62], (u * 0.6 + v * 0.4).clamp(0.0, 1.0));
            (lerp3(base, [0.95, 0.62, 0.38], ((v - 0.5) * 1.6).clamp(0.0, 1.0) * 0.6), 1.0)
        });
        // Barra de menus (translúcida) com itens genéricos dos dois lados.
        let bar = l.band;
        cv.fill_rrect(0.0, 0.0, fw, bar, 0.0, 0.28, [0.0; 3]);
        ui::glyph(&mut cv, tr, RectF::new(12.0 * s, 0.0, 22.0 * s, bar), G_MUSIC, Font::IconSmall, [1.0; 3], 0.95);
        ui::text(&mut cv, tr, "Arquivo   Editar   Ver", Font::Body, 44.0 * s, 0.0, 220.0 * s, bar, [1.0; 3], 0.95);
        ui::text_right(&mut cv, tr, "12:34", Font::Body, fw - 14.0 * s, 0.0, bar, [1.0; 3], 0.95, s);
        let x0 = ((fw - sw as f32) / 2.0).round();
        if g.notch_w > 1.0 {
            let n = l.notch.offset(x0, 0.0);
            cv.fill_rrect(n.x, -12.0 * s, n.right(), n.bottom(), 10.0 * s, 1.0, [0.0; 3]);
        }
        cv.composite(x0 as i32, 0, &layer, sw, sh, 1.0);
    }
    (iw, ih, px)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, NotchBg};

    const S: f32 = 2.0;

    fn out(name: &str) -> std::path::PathBuf {
        let d = std::env::var_os("TASKBEAT_SHOTS_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("notch-shots"));
        d.join(name)
    }

    fn scene(name: &str, g: Geom, cfg: &Config, media: &MediaInfo, open: f32, peek: f32) {
        let mut tr = TextRenderer::new();
        tr.set_scale(S);
        let c = Colors { palette: crate::config::Palette::Album, accent: [0.0, 0.48, 1.0], accent_light: [0.25, 0.6, 1.0], light_taskbar: false, album: None };
        let (w, h, px) = preview(&mut tr, S, g, cfg, &c, media, open, peek, 1.3);
        crate::png::save(&out(name), w, h, &px);
    }

    #[test]
    #[ignore]
    fn notch_shots() {
        let notch = Geom { notch_w: 196.0 * S, notch_h: 32.0 * S };
        let flat = Geom { notch_w: 0.0, notch_h: 24.0 * S };
        let media = ui::sample_media();
        let cfg = Config::default();
        scene("notch-closed.png", notch, &cfg, &media, 0.0, 0.0);
        scene("notch-open.png", notch, &cfg, &media, 1.0, 0.0);
        scene("notch-opening.png", notch, &cfg, &media, grow(0.45), 0.0);
        scene("notch-peek.png", notch, &cfg, &media, 0.0, 1.0);
        for (name, bg) in [("notch-open-black.png", NotchBg::Black), ("notch-open-artwork.png", NotchBg::Artwork)] {
            scene(name, notch, &Config { notch_bg: bg, ..cfg.clone() }, &media, 1.0, 0.0);
        }
        scene("notch-open-novol.png", notch, &Config { show_volume: false, show_visualizer: false, ..cfg.clone() }, &media, 1.0, 0.0);
        scene("flat-closed.png", flat, &cfg, &media, 0.0, 0.0);
        scene("flat-open.png", flat, &cfg, &media, 1.0, 0.0);
        scene("notch-idle.png", notch, &cfg, &MediaInfo::default(), 0.0, 0.0);
        scene("notch-idle-open.png", notch, &cfg, &MediaInfo::default(), 1.0, 0.0);
    }
}
