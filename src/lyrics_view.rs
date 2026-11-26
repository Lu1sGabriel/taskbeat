//! Desenho da sobreposição de letra (Windows): os layouts (só a letra, só o
//! player, player e letra, compacto, legenda), a tela cheia e a barrinha de
//! ferramentas da janela destravada. Usado pela janela (`lyrics_overlay.rs`)
//! e pela prévia das configurações.
//!
//! Mesma família do card expandido: o fundo, a capa, os botões e o player são
//! os de `ui.rs`. A letra usa máscaras de texto guardadas (cada linha é
//! rasterizada uma vez); a troca de linha só recompõe máscaras.

use crate::config::{CardStyle, Config, Size3};
use crate::lyrics::config::{Background, Layout, LyricsConfig};
use crate::lyrics::{Line, Lyrics};
use crate::render::{Canvas, Colors, RectF, Rgb};
use crate::shared::MediaInfo;
use crate::text::*;
use crate::ui::{self, CLayout, CardFrame, Hit, Interact, Playback, Theme};
use std::cell::RefCell;
use std::rc::Rc;

/// Onde o mouse está na sobreposição.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LHit {
    /// Um botão do player (card, compacto ou tela cheia).
    Card(Hit),
    Lock,
    Full,
    Close,
    /// O resto: arrasta a janela (destravada).
    Body,
}

/// A letra como aparece agora.
#[derive(Clone, Copy)]
pub struct LyricFrame<'a> {
    pub lyrics: &'a Lyrics,
    /// Linhas sincronizadas com as pausas (`lyrics::with_gaps`).
    pub lines: &'a [Line],
    /// Linha em foco, contínua durante a troca (2,4 = 40% do caminho da 2 para a 3).
    pub focus: f32,
    /// Letra simples: quanto rolou (px lógicos).
    pub scroll: f32,
}

/// Tudo que a sobreposição mostra num quadro.
pub struct View<'a> {
    /// Opções do card já ajustadas para a sobreposição (`card_config`).
    pub card_cfg: &'a Config,
    pub lc: &'a LyricsConfig,
    pub colors: &'a Colors,
    pub media: &'a MediaInfo,
    pub pb: Playback,
    pub it: Interact,
    pub hover: Option<LHit>,
    /// Barrinha de ferramentas (destravada, mouse em cima): 0 escondida, 1 inteira.
    pub tools: f32,
    /// Tela cheia: segundos da animação do fundo (`None`: fundo parado) e a
    /// capa em alta resolução, se já chegou.
    pub bg_time: Option<f32>,
    pub hires: Option<&'a crate::render::Image>,
    /// Troca de música em andamento: (música anterior, 0..1).
    pub change: Option<(&'a MediaInfo, f32)>,
    /// Mixer de volume (layouts com player, opção ligada).
    pub mixer: Option<&'a crate::shared::MixerState>,
    /// Opacidade da letra (a letra nova aparece suave quando chega).
    pub lyr_alpha: f32,
    /// Dica do botão da barrinha sob o mouse: (texto, opacidade).
    pub tip: Option<(&'a str, f32)>,
    /// Aviso rápido no meio do painel ("Travada · Win+Alt+Shift+L destrava").
    pub toast: Option<(&'a str, f32)>,
    pub frame: LyricFrame<'a>,
    pub s: f32,
}

/// As opções do card que a sobreposição usa: o fundo dela, volume e mixer
/// conforme as opções dela, sem atalhos, barras nem selo do app.
pub fn card_config(base: &Config, lc: &LyricsConfig) -> Config {
    Config {
        card_style: match lc.background {
            Background::AlbumColor => CardStyle::AlbumColor,
            Background::Artwork => CardStyle::Artwork,
            // Escuro / sem fundo: o card vira o vidro escuro sólido.
            Background::Dark | Background::None => CardStyle::Glass,
        },
        theme: match lc.background {
            Background::Dark | Background::None => crate::config::ThemeMode::Dark,
            _ => base.theme,
        },
        card_layout: crate::config::CardLayout::Horizontal,
        card_seek: true,
        show_volume: lc.volume,
        card_mixer: lc.mixer,
        card_launchers: false,
        card_viz: false,
        show_source: false,
        ..base.clone()
    }
}

/// Cores da sobreposição: as do card; sem fundo ou no escuro, texto branco.
pub fn theme(card_cfg: &Config, lc: &LyricsConfig, colors: &Colors, media: &MediaInfo) -> Theme {
    let th = ui::card_theme(card_cfg, colors, media);
    match lc.background {
        Background::Dark | Background::None => Theme { fg: [1.0; 3], dark: true, ..th },
        _ => th,
    }
}

// ---------------------------------------------------------------- geometria

/// Posições de tudo na superfície da janela (px).
#[derive(Clone, Debug, Default)]
pub struct Geo {
    pub w: f32,
    pub h: f32,
    /// O painel (fundo) dentro da superfície (a margem é a da sombra).
    pub card: RectF,
    pub radius: f32,
    /// Área da letra (vazia em "Só o player").
    pub lyr: RectF,
    /// Card dos layouts com player.
    pub cl: Option<CLayout>,
    /// Compacto e tela cheia: capa, textos, botões e progresso.
    pub art: RectF,
    pub title: RectF,
    pub artist: RectF,
    pub shuffle: RectF,
    pub prev: RectF,
    pub play: RectF,
    pub play_size: f32,
    pub next: RectF,
    pub repeat: RectF,
    pub bar: RectF,
    /// Barrinha (destravada): alça, travar, tela cheia, esconder.
    pub toolbar: RectF,
    pub lock: RectF,
    pub full: RectF,
    pub close: RectF,
    /// Linha atual e as outras.
    pub big: Font,
    pub dim: Font,
    pub fullscreen: bool,
}

fn fonts(layout: Layout, size: Size3) -> (Font, Font) {
    use Font::*;
    match (layout, size) {
        (Layout::Lyrics, Size3::Small) => (LyricSm, LyricDimSm),
        (Layout::Lyrics, Size3::Normal) => (LyricMd, LyricDimMd),
        (Layout::Lyrics, Size3::Large) => (LyricLg, LyricDimLg),
        (Layout::Caption, Size3::Small) => (LyricSm, Caption),
        (Layout::Caption, Size3::Normal) => (LyricMd, Body),
        (Layout::Caption, Size3::Large) => (LyricLg, LyricDimSm),
        (_, Size3::Small) => (LyricDimMd, LyricDimSm),
        (_, Size3::Normal) => (LyricSm, LyricDimSm),
        (_, Size3::Large) => (LyricMd, LyricDimMd),
    }
}

fn pick(size: Size3, sm: f32, md: f32, lg: f32) -> f32 {
    match size {
        Size3::Small => sm,
        Size3::Normal => md,
        Size3::Large => lg,
    }
}

/// Largura máxima da legenda (px lógicos).
const CAPTION_MAX: f32 = 760.0;

/// Tamanho do texto da legenda (px): a linha atual (até 2 linhas) e a
/// próxima. Durante a troca, o tamanho vai do da linha antiga ao da nova
/// (a pílula cresce ou encolhe junto). Devolve (largura, altura) do conteúdo.
pub fn caption_content(tr: &mut TextRenderer, f: &LyricFrame, lc: &LyricsConfig, s: f32) -> (f32, f32) {
    let (big, dim) = fonts(Layout::Caption, lc.size);
    let maxw = ((CAPTION_MAX - 48.0) * s) as i32;
    let mut size = |i: usize| {
        let (cur, next) = caption_texts(f, i);
        let m = tr.mask_wrapped(&cur, big, maxw, 2, Align::Center);
        let n = tr.mask_wrapped(next, dim, maxw, 1, Align::Center);
        // Máscara centrada em `maxw` (cortada à direita): o texto vai de
        // (maxw - t)/2 até (maxw + t)/2, então t = 2 × usado - maxw.
        let used = m.used_width().max(if next.is_empty() { 0.0 } else { n.used_width() });
        let w = (2.0 * used - maxw as f32).max(0.0);
        (w, m.h as f32 + if next.is_empty() { 0.0 } else { n.h as f32 + 2.0 * s })
    };
    match caption_lines(f) {
        (Some(a), Some(b)) => {
            let (wa, ha) = size(a);
            let (wb, hb) = size(b);
            let e = ui::ease_out_cubic(f.focus - f.focus.floor());
            (wa + (wb - wa) * e, ha + (hb - ha) * e)
        }
        (None, Some(b)) => size(b),
        _ => {
            let m = tr.mask(&message(f.lyrics, true).0, big, maxw, (40.0 * s) as i32, Align::Center);
            ((2.0 * m.used_width() - maxw as f32).max(0.0), 40.0 * s)
        }
    }
}

/// Linhas da legenda: a que sai (durante a troca) e a que entra.
fn caption_lines(f: &LyricFrame) -> (Option<usize>, Option<usize>) {
    if !f.lyrics.is_text() || !matches!(f.lyrics, Lyrics::Synced(_)) || f.lines.is_empty() {
        return (None, None);
    }
    let n = f.lines.len() as i32;
    let k = f.focus.floor() as i32;
    let frac = f.focus - k as f32;
    let at = |i: i32| (i >= 0 && i < n).then_some(i as usize);
    if frac > 0.001 { (at(k), at(k + 1)) } else { (None, at(k)) }
}

/// (linha atual, próxima) da legenda na linha `i` (pausa: "• • •").
fn caption_texts<'a>(f: &'a LyricFrame, i: usize) -> (String, &'a str) {
    let cur = if f.lines[i].text.is_empty() { "• • •".to_string() } else { f.lines[i].text.clone() };
    let next = f.lines.get(i + 1).map(|l| l.text.as_str()).unwrap_or("");
    (cur, next)
}

/// Mixer aberto na geometria: (apps, quanto abriu 0..1, rolagem px lógicos).
pub type MixerGeo = Option<(usize, f32, f32)>;

/// Geometria da janela. `caption`: conteúdo da legenda (px, ver
/// `caption_content`); `mixer`: o mixer dos layouts com player.
pub fn geometry(lc: &LyricsConfig, card_cfg: &Config, s: f32, caption: (f32, f32), mixer: MixerGeo) -> Geo {
    let m = ui::SHADOW_MARGIN * s;
    let radius = card_cfg.corners.radius() * s;
    let (big, dim) = fonts(lc.layout, lc.size);
    let mut g = Geo { radius, big, dim, ..Default::default() };
    match lc.layout {
        Layout::Lyrics => {
            let (w, h) = (pick(lc.size, 380.0, 440.0, 540.0) * s, pick(lc.size, 164.0, 196.0, 240.0) * s);
            g.card = RectF::new(m, m, w, h);
            g.lyr = g.card.inset(16.0 * s);
        }
        Layout::Caption => {
            let w = (caption.0 + 48.0 * s).clamp(220.0 * s, CAPTION_MAX * s).round();
            let h = (caption.1 + 26.0 * s).max(52.0 * s).round();
            g.card = RectF::new(m, m, w, h);
            g.radius = (18.0 * s).min(h / 2.0);
            g.lyr = RectF::new(g.card.x + 24.0 * s, g.card.y + 13.0 * s, w - 48.0 * s, h - 26.0 * s);
        }
        Layout::Compact => {
            let w = pick(lc.size, 320.0, 360.0, 420.0) * s;
            let time = if lc.show_time { 14.0 * s } else { 0.0 };
            let h = pick(lc.size, 180.0, 200.0, 236.0) * s + time;
            g.card = RectF::new(m, m, w, h);
            let pad = 12.0 * s;
            g.art = RectF::new(g.card.x + pad, g.card.y + pad, 44.0 * s, 44.0 * s);
            let bx = g.card.right() - pad - 108.0 * s;
            let cy = g.art.cy();
            g.prev = RectF::new(bx, cy - 18.0 * s, 36.0 * s, 36.0 * s);
            g.play = RectF::new(bx + 36.0 * s, cy - 18.0 * s, 36.0 * s, 36.0 * s);
            g.next = RectF::new(bx + 72.0 * s, cy - 18.0 * s, 36.0 * s, 36.0 * s);
            g.play_size = 30.0 * s;
            let tx = g.art.right() + 12.0 * s;
            g.title = RectF::new(tx, g.art.y + 3.0 * s, (bx - 6.0 * s - tx).max(0.0), 20.0 * s);
            g.artist = RectF::new(tx, g.art.y + 22.0 * s, g.title.w, 18.0 * s);
            g.bar = RectF::new(g.card.x + pad, g.art.bottom() + 9.0 * s, w - 2.0 * pad, 3.0 * s);
            let top = g.bar.bottom() + 6.0 * s + time;
            g.lyr = RectF::new(g.card.x + pad + 2.0 * s, top, w - 2.0 * pad - 4.0 * s, g.card.bottom() - top - 8.0 * s);
        }
        Layout::Player | Layout::PlayerLyrics => {
            let mut l = CLayout::build(s, card_cfg, true);
            l.times = lc.show_time;
            let mut l = l.with_mixer(mixer.map(|m| m.0), mixer.map(|m| m.1).unwrap_or(0.0), mixer.map(|m| m.2).unwrap_or(0.0), 5, s);
            if lc.layout == Layout::PlayerLyrics {
                let extra = pick(lc.size, 150.0, 176.0, 212.0) * s;
                let top = l.card.bottom() - 4.0 * s;
                l.card.h += extra;
                l.h += extra;
                g.lyr = RectF::new(l.card.x + 16.0 * s, top + 8.0 * s, l.card.w - 32.0 * s, extra - 14.0 * s);
            }
            g.card = l.card;
            g.radius = l.radius;
            g.cl = Some(l);
        }
    }
    g.w = (g.card.right() + m).ceil();
    g.h = (g.card.bottom() + m).ceil();
    toolbar(&mut g, s);
    g
}

/// Barrinha centrada na borda de cima do painel (metade para fora, na margem da sombra).
fn toolbar(g: &mut Geo, s: f32) {
    let (bw, h) = (32.0 * s, 32.0 * s);
    let w = bw * 4.0 + 6.0 * s;
    let y = if g.fullscreen { g.card.y + 20.0 * s } else { (g.card.y - h / 2.0).max(1.0) };
    g.toolbar = RectF::new((g.card.cx() - w / 2.0).round(), y.round(), w, h);
    let x0 = g.toolbar.x + 3.0 * s + bw;
    g.lock = RectF::new(x0, g.toolbar.y, bw, h);
    g.full = RectF::new(x0 + bw, g.toolbar.y, bw, h);
    g.close = RectF::new(x0 + 2.0 * bw, g.toolbar.y, bw, h);
}

/// Geometria da tela cheia num monitor de `w`×`h` px.
pub fn fullscreen_geometry(w: f32, h: f32, s: f32) -> Geo {
    let mut g = Geo { w, h, fullscreen: true, big: Font::LyricXl, dim: Font::LyricDimXl, ..Default::default() };
    g.card = RectF::new(0.0, 0.0, w, h);
    let art = (h * 0.42).min(w * 0.27).round();
    let left = (w * 0.08).round();
    let block = art + 200.0 * s;
    let top = ((h - block) / 2.0).round();
    g.art = RectF::new(left, top, art, art);
    g.title = RectF::new(left, g.art.bottom() + 22.0 * s, art, 44.0 * s);
    g.artist = RectF::new(left, g.title.bottom(), art, 26.0 * s);
    g.bar = RectF::new(left, g.artist.bottom() + 22.0 * s, art, 4.0 * s);
    let by = g.bar.bottom() + 26.0 * s;
    let (c0, c4) = (left + 22.0 * s, left + art - 22.0 * s);
    let slot = |k: f32| {
        let cx = c0 + (c4 - c0) * k / 4.0;
        RectF::new(cx - 30.0 * s, by, 60.0 * s, 60.0 * s)
    };
    (g.shuffle, g.prev, g.play, g.next, g.repeat) = (slot(0.0), slot(1.0), slot(2.0), slot(3.0), slot(4.0));
    g.play_size = 58.0 * s;
    let lx = (left + art + w * 0.07).round();
    g.lyr = RectF::new(lx, (h * 0.06).round(), (w - lx - w * 0.06).max(100.0), (h * 0.88).round());
    // Só "sair da tela cheia" (no canto).
    g.full = RectF::new(w - 64.0 * s, 24.0 * s, 40.0 * s, 40.0 * s);
    g
}

/// O que está sob (x, y).
pub fn hit(g: &Geo, lc: &LyricsConfig, x: f32, y: f32, tools_on: bool) -> Option<LHit> {
    if g.fullscreen {
        if g.full.contains(x, y) {
            return Some(LHit::Full);
        }
        for (r, h) in [(g.shuffle, Hit::Shuffle), (g.prev, Hit::Prev), (g.play, Hit::Play), (g.next, Hit::Next), (g.repeat, Hit::Repeat)] {
            if r.contains(x, y) {
                return Some(LHit::Card(h));
            }
        }
        let seek = RectF::new(g.bar.x, g.bar.y - 8.0 * (g.bar.h / 4.0), g.bar.w, g.bar.h + 16.0 * (g.bar.h / 4.0));
        if seek.contains(x, y) {
            return Some(LHit::Card(Hit::Seek));
        }
        return Some(LHit::Body);
    }
    if tools_on {
        for (r, h) in [(g.lock, LHit::Lock), (g.full, LHit::Full), (g.close, LHit::Close)] {
            if r.contains(x, y) {
                return Some(h);
            }
        }
        if g.toolbar.contains(x, y) {
            return Some(LHit::Body);
        }
    }
    if let Some(l) = &g.cl {
        if g.lyr.contains(x, y) {
            return Some(LHit::Body);
        }
        return match l.hit(x, y) {
            Some(Hit::Info) => Some(LHit::Body),
            Some(h) => Some(LHit::Card(h)),
            None => g.card.contains(x, y).then_some(LHit::Body),
        };
    }
    if lc.layout == Layout::Compact {
        for (r, h) in [(g.prev, Hit::Prev), (g.play, Hit::Play), (g.next, Hit::Next)] {
            if r.contains(x, y) {
                return Some(LHit::Card(h));
            }
        }
        // Barra de progresso (alvo mais alto que ela, para pegar fácil).
        let pad = g.bar.h * 2.4;
        if RectF::new(g.bar.x - pad, g.bar.y - pad, g.bar.w + 2.0 * pad, g.bar.h + 2.0 * pad).contains(x, y) {
            return Some(LHit::Card(Hit::Seek));
        }
    }
    g.card.contains(x, y).then_some(LHit::Body)
}

/// Faixa do progresso clicável da tela cheia (x0, x1).
/// Trilho do progresso (x0, x1) onde clicar/arrastar muda o tempo.
pub fn seek_track(g: &Geo, s: f32) -> Option<(f32, f32)> {
    match &g.cl {
        Some(l) if !g.fullscreen => l.seek_track(s),
        _ => (g.bar.w > 0.0).then_some((g.bar.x, g.bar.right())),
    }
}

// ---------------------------------------------------------------- desenho

thread_local! {
    /// Fundo do painel guardado (refeito quando muda tamanho, capa, estilo…).
    static BG: RefCell<Option<(u64, usize, usize, Rc<Vec<u32>>)>> = const { RefCell::new(None) };
}

/// Solta o que fica guardado entre quadros (a janela escondeu).
pub fn forget() {
    BG.with(|b| *b.borrow_mut() = None);
    FS_LEFT.with(|c| c.borrow_mut().clear());
    crate::lyrics_bg::forget();
    ui::forget_layers(ui::LayerTag::Card);
}

fn bits(r: RectF) -> [u32; 4] {
    [r.x.to_bits(), r.y.to_bits(), r.w.to_bits(), r.h.to_bits()]
}

/// Fundo do painel (com a opacidade escolhida), guardado entre quadros.
fn panel(cv: &mut Canvas, g: &Geo, v: &View, th: &Theme) {
    let lc = v.lc;
    if lc.background == Background::None {
        return;
    }
    let (w, h) = (cv.w, cv.h);
    let key = ui::layer_key((
        lc.background as u8,
        bits(g.card),
        g.radius.to_bits(),
        g.fullscreen,
        v.media.art.as_ref().map(|a| a.id),
        (th.dark, th.accent.map(f32::to_bits), v.colors.accent.map(f32::to_bits)),
        v.card_cfg.theme as u8,
    ));
    let px = BG.with(|b| b.borrow().as_ref().filter(|c| c.0 == key && c.1 == w && c.2 == h).map(|c| c.3.clone()));
    let px = px.unwrap_or_else(|| {
        let mut px = vec![0u32; w * h];
        {
            let mut c = Canvas::new(w, h, &mut px);
            draw_panel(&mut c, g, v, th);
        }
        let px = Rc::new(px);
        BG.with(|b| *b.borrow_mut() = Some((key, w, h, px.clone())));
        px
    });
    cv.composite(0, 0, &px, w, h, lc.opacity as f32 / 100.0);
}

fn draw_panel(cv: &mut Canvas, g: &Geo, v: &View, th: &Theme) {
    let s = v.s;
    match v.lc.background {
        Background::Artwork | Background::AlbumColor => ui::card_background(cv, g.card, g.radius, v.card_cfg, v.colors, v.media, th, s, None),
        Background::Dark => {
            cv.drop_shadow(g.card, g.radius, 18.0 * s, 4.0 * s, 0.35);
            cv.fill_round(g.card, g.radius, [0.04, 0.04, 0.05], 0.66);
            cv.stroke_round(g.card, g.radius, s, [1.0; 3], 0.10);
        }
        Background::None => {}
    }
}

/// Texto da mensagem no lugar da letra: (título, detalhe).
pub fn message(l: &Lyrics, present: bool) -> (String, &'static str) {
    use crate::i18n::tr;
    if !present {
        return (tr("Nada tocando").into(), "");
    }
    match l {
        Lyrics::Loading => (tr("Procurando a letra…").into(), ""),
        Lyrics::Instrumental => (tr("Instrumental").into(), ""),
        Lyrics::NotFound => (tr("Esta música não tem letra").into(), ""),
        Lyrics::Failed => (tr("Não deu para buscar a letra").into(), tr("Tenta de novo em instantes")),
        Lyrics::Offline => (tr("Busca de letras desligada").into(), tr("Ligue nas configurações ou ponha a letra na pasta")),
        _ => (String::new(), ""),
    }
}

thread_local! {
    /// Opacidade extra de tudo que a área da letra desenha (a letra nova
    /// aparece suave); 1 fora dela.
    static FADE: std::cell::Cell<f32> = const { std::cell::Cell::new(1.0) };
}

/// Máscara com sombra (texto sem fundo).
fn blit_text(cv: &mut Canvas, m: &crate::render::Mask, x: f32, y: f32, c: Rgb, a: f32, shadow: bool, s: f32) {
    let a = a * FADE.with(|f| f.get());
    if a <= 0.003 {
        return;
    }
    let (xi, yi) = (x.round() as i32, y.round() as i32);
    if shadow {
        let k = (1.2 * s).round().max(1.0) as i32;
        cv.blit_mask(xi, yi + k, m, [0.0; 3], 0.55 * a);
        cv.blit_mask(xi + k, yi + k, m, [0.0; 3], 0.30 * a);
        cv.blit_mask(xi - k, yi, m, [0.0; 3], 0.20 * a);
    }
    cv.blit_mask(xi, yi, m, c, a);
}

/// Três pontos (pausa sem letra).
fn dots(cv: &mut Canvas, x: f32, cy: f32, center_w: Option<f32>, size: f32, c: Rgb, a: f32) {
    let a = a * FADE.with(|f| f.get());
    let r = size * 0.11;
    let step = r * 3.4;
    let span = step * 2.0 + 2.0 * r;
    let x0 = match center_w {
        Some(w) => x + (w - span) / 2.0 + r,
        None => x + r,
    };
    for k in 0..3 {
        cv.fill_circle(x0 + k as f32 * step, cy, r, c, a);
    }
}

/// Tamanho da fonte em px (para os pontos da pausa): a altura de uma linha
/// é ≈ 1,33 × o tamanho.
fn font_px(tr: &mut TextRenderer, f: Font) -> f32 {
    tr.mask_wrapped("Hg", f, 400, 1, Align::Left).h as f32 / 1.33
}

/// Máscara escalada `k` vezes em volta do ponto (`px`, `py`) dela, com canto
/// superior esquerdo (sem escala) em (`x`, `y`). Bilinear, só na caixa de
/// destino (a troca de linha dura uns 300 ms).
#[allow(clippy::too_many_arguments)]
fn blit_scaled(cv: &mut Canvas, m: &crate::render::Mask, x: f32, y: f32, px: f32, py: f32, k: f32, c: Rgb, a: f32) {
    let a = a * FADE.with(|f| f.get());
    if a <= 0.003 || m.w == 0 || m.h == 0 || k <= 0.01 {
        return;
    }
    let (x0, y0) = (x + px - px * k, y + py - py * k);
    let clip = cv.clip_rect();
    let ix0 = x0.floor().max(clip.x).max(0.0) as i32;
    let iy0 = y0.floor().max(clip.y).max(0.0) as i32;
    let ix1 = (x0 + m.w as f32 * k).ceil().min(clip.right()).min(cv.w as f32) as i32;
    let iy1 = (y0 + m.h as f32 * k).ceil().min(clip.bottom()).min(cv.h as f32) as i32;
    let at = |u: i32, v: i32| -> f32 { if u < 0 || v < 0 || u >= m.w as i32 || v >= m.h as i32 { 0.0 } else { m.cov[v as usize * m.w + u as usize] as f32 } };
    let inv = 1.0 / k;
    for oy in iy0..iy1 {
        let v = (oy as f32 + 0.5 - y0) * inv - 0.5;
        let (iv, fv) = (v.floor() as i32, v - v.floor());
        for ox in ix0..ix1 {
            let u = (ox as f32 + 0.5 - x0) * inv - 0.5;
            let (iu, fu) = (u.floor() as i32, u - u.floor());
            let top = at(iu, iv) * (1.0 - fu) + at(iu + 1, iv) * fu;
            let bot = at(iu, iv + 1) * (1.0 - fu) + at(iu + 1, iv + 1) * fu;
            let cov = (top * (1.0 - fv) + bot * fv) / 255.0;
            if cov > 0.002 {
                cv.blend(ox as usize, oy as usize, c, cov * a);
            }
        }
    }
}

/// Letra sincronizada em `r`: a linha em foco grande (em `anchor`, 0..1 da
/// altura), as outras menores, apagadas e mais fracas quanto mais longe. As
/// linhas apagadas quebram na mesma proporção da grande, então a troca é a
/// mesma máscara crescendo ou encolhendo (e parada, cada uma na sua nitidez).
#[allow(clippy::too_many_arguments)]
fn synced(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, lines: &[Line], focus: f32, big: Font, dim: Font, center: bool, fg: Rgb, anchor: f32, shadow: bool, s: f32) {
    let n = lines.len();
    if n == 0 || r.w < 10.0 || r.h < 10.0 {
        return;
    }
    let align = if center { Align::Center } else { Align::Left };
    let w = r.w as i32;
    let gap = 3.0 * s;
    let dot_big = font_px(tr, big);
    let dot_dim = font_px(tr, dim);
    let ratio = (dot_dim / dot_big).clamp(0.3, 1.0);
    let wd = (r.w * ratio) as i32;
    // x da máscara apagada (centrada: a caixa dela é mais estreita).
    let xd = if center { r.x + (r.w - wd as f32) / 2.0 } else { r.x };
    let big_m = |tr: &mut TextRenderer, i: usize| tr.mask_wrapped(&lines[i].text, big, w, 2, align);
    let dim_m = |tr: &mut TextRenderer, i: usize| tr.mask_wrapped(&lines[i].text, dim, wd, 2, align);
    let hb = |tr: &mut TextRenderer, i: usize| -> f32 { if lines[i].text.is_empty() { dot_big * 1.1 } else { big_m(tr, i).h as f32 } };
    let hd = |tr: &mut TextRenderer, i: usize| -> f32 { if lines[i].text.is_empty() { dot_dim * 0.9 } else { dim_m(tr, i).h as f32 } };
    let ay = r.y + r.h * anchor;
    // Centros das linhas com a linha `k` em foco (k = -1: antes da primeira).
    let span = 7usize;
    let place = |tr: &mut TextRenderer, k: i32| -> Vec<(usize, f32)> {
        let mut out = Vec::new();
        if k >= 0 && (k as usize) < n {
            let k = k as usize;
            out.push((k, ay));
            let (mut y, mut prev) = (ay, hb(tr, k));
            for i in k + 1..(k + span).min(n) {
                let h = hd(tr, i);
                y += prev / 2.0 + gap + h / 2.0;
                prev = h;
                out.push((i, y));
            }
            let (mut y, mut prev) = (ay, hb(tr, k));
            for i in (k.saturating_sub(span)..k).rev() {
                let h = hd(tr, i);
                y -= prev / 2.0 + gap + h / 2.0;
                prev = h;
                out.push((i, y));
            }
        } else if k < 0 {
            let mut y = ay;
            for i in 0..span.min(n) {
                let h = hd(tr, i);
                y += gap + h / 2.0;
                out.push((i, y));
                y += h / 2.0;
            }
        }
        out
    };
    let k0 = focus.floor() as i32;
    let frac = focus - k0 as f32;
    let a = place(tr, k0);
    let b = if frac > 0.001 { place(tr, k0 + 1) } else { Vec::new() };
    let clip = cv.clip_rect();
    cv.set_clip(clip.intersect(r));
    for &(i, ya) in &a {
        let y = if frac > 0.001 { b.iter().find(|(j, _)| *j == i).map(|(_, yb)| ya + (yb - ya) * frac).unwrap_or(ya) } else { ya };
        let d = (i as f32 - focus).abs();
        let e = (1.0 - d).clamp(0.0, 1.0);
        let fall = (1.0 - 0.30 * (d - 1.0).max(0.0)).clamp(0.16, 1.0);
        let dim_a = if (i as f32) < focus { 0.42 } else { 0.60 } * fall;
        let alpha = dim_a + (1.0 - dim_a) * e;
        // Some perto das bordas de cima e de baixo.
        let lh = hd(tr, i);
        let edge = ((y - r.y) / (lh * 0.9)).min((r.bottom() - y) / (lh * 0.9)).clamp(0.0, 1.0);
        if edge <= 0.0 {
            continue;
        }
        let a = alpha * edge;
        if lines[i].text.is_empty() {
            let size = dot_dim + (dot_big - dot_dim) * e;
            dots(cv, r.x, y, center.then_some(r.w), size, fg, a * 0.9);
            continue;
        }
        if e >= 0.999 {
            let m = big_m(tr, i);
            blit_text(cv, &m, r.x, y - m.h as f32 / 2.0, fg, a, shadow, s);
        } else if e <= 0.001 {
            let m = dim_m(tr, i);
            blit_text(cv, &m, xd, y - m.h as f32 / 2.0, fg, a, shadow, s);
        } else {
            // No meio da troca: a máscara grande, escalada.
            let m = big_m(tr, i);
            let k = ratio + (1.0 - ratio) * e;
            let (px, py) = (if center { r.w / 2.0 } else { 0.0 }, m.h as f32 / 2.0);
            let top = y - m.h as f32 / 2.0;
            if shadow {
                blit_scaled(cv, &m, r.x, top + 1.2 * s, px, py, k, [0.0; 3], 0.55 * a);
            }
            blit_scaled(cv, &m, r.x, top, px, py, k, fg, a);
        }
    }
    cv.set_clip(clip);
}

/// Altura total da letra simples (px), para limitar a rolagem.
pub fn plain_height(tr: &mut TextRenderer, lines: &[String], font: Font, w: f32, s: f32) -> f32 {
    lines.iter().map(|l| if l.is_empty() { 12.0 * s } else { tr.mask_wrapped(l, font, w as i32, 3, Align::Left).h as f32 + 3.0 * s }).sum()
}

fn plain(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, lines: &[String], scroll: f32, font: Font, center: bool, fg: Rgb, shadow: bool, s: f32) {
    let align = if center { Align::Center } else { Align::Left };
    let clip = cv.clip_rect();
    cv.set_clip(clip.intersect(r));
    let mut y = r.y + 4.0 * s - scroll * s;
    for l in lines {
        if l.is_empty() {
            y += 12.0 * s;
            continue;
        }
        let m = tr.mask_wrapped(l, font, r.w as i32, 3, align);
        let h = m.h as f32;
        if y + h > r.y && y < r.bottom() {
            let cy = y + h / 2.0;
            let edge = ((cy - r.y) / (h * 0.8)).min((r.bottom() - cy) / (h * 0.8)).clamp(0.0, 1.0);
            blit_text(cv, &m, r.x, y, fg, 0.86 * edge, shadow, s);
        }
        y += h + 3.0 * s;
        if y > r.bottom() {
            break;
        }
    }
    cv.set_clip(clip);
}

/// Mensagem centralizada (sem letra, procurando…).
fn message_box(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, l: &Lyrics, present: bool, fg: Rgb, shadow: bool, s: f32, small: bool) {
    let (title, sub) = message(l, present);
    if title.is_empty() {
        return;
    }
    let font = if small { Font::Body } else { Font::BodyStrong };
    let icon = if matches!(l, Lyrics::Instrumental) || !present { G_MUSIC } else { "\u{E8D2}" };
    let th = 22.0 * s;
    let total = if small { th } else { 30.0 * s + th + if sub.is_empty() { 0.0 } else { 18.0 * s } };
    let mut y = r.cy() - total / 2.0;
    if !small {
        ui::glyph(cv, tr, RectF::new(r.x, y, r.w, 26.0 * s), icon, Font::IconLg, fg, 0.5);
        y += 30.0 * s;
    }
    let m = tr.mask(&title, font, r.w as i32, th as i32, Align::Center);
    blit_text(cv, &m, r.x, y, fg, 0.72, shadow, s);
    if !sub.is_empty() && !small {
        let m = tr.mask(sub, Font::Caption, r.w as i32, (18.0 * s) as i32, Align::Center);
        blit_text(cv, &m, r.x, y + th, fg, 0.5, shadow, s);
    }
}

/// A letra (ou a mensagem) na área `r`.
#[allow(clippy::too_many_arguments)]
fn lyrics_area(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, v: &View, big: Font, dim: Font, anchor: f32, fg: Rgb, small_msg: bool) {
    FADE.with(|f| f.set(v.lyr_alpha.clamp(0.0, 1.0)));
    lyrics_area_inner(cv, tr, r, v, big, dim, anchor, fg, small_msg);
    FADE.with(|f| f.set(1.0));
}

#[allow(clippy::too_many_arguments)]
fn lyrics_area_inner(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, v: &View, big: Font, dim: Font, anchor: f32, fg: Rgb, small_msg: bool) {
    let (s, lc) = (v.s, v.lc);
    let shadow = lc.background == Background::None && !matches!(lc.layout, Layout::Player | Layout::PlayerLyrics);
    let center = lc.centered();
    match v.frame.lyrics {
        Lyrics::Synced(_) if v.media.present() => synced(cv, tr, r, v.frame.lines, v.frame.focus, big, dim, center, fg, anchor, shadow, s),
        Lyrics::Plain(lines) if v.media.present() => {
            plain(cv, tr, r, lines, v.frame.scroll, dim, center, fg, shadow, s);
            // Selo "sem sincronia" no canto de cima.
            let label = crate::i18n::tr("sem sincronia");
            let m = tr.mask(label, Font::Small, (r.w * 0.5) as i32, (16.0 * s) as i32, Align::Left);
            let x = r.right() - m.used_width();
            blit_text(cv, &m, x, r.y - 2.0 * s, fg, 0.42, shadow, s);
        }
        l => message_box(cv, tr, r, l, v.media.present(), fg, shadow, s, small_msg),
    }
}

/// Barrinha da janela destravada: alça, travar, tela cheia e esconder.
fn draw_toolbar(cv: &mut Canvas, tr: &mut TextRenderer, g: &Geo, v: &View) {
    let (s, a) = (v.s, v.tools);
    if a <= 0.0 {
        return;
    }
    // Sólida (sem transparência): entra crescendo um pouco, sem esmaecer.
    let k = 0.86 + 0.14 * a;
    let t0 = g.toolbar;
    let t = RectF::new(t0.cx() - t0.w * k / 2.0, t0.cy() - t0.h * k / 2.0, t0.w * k, t0.h * k);
    let rad = t.h / 2.0;
    let locked = v.lc.locked;
    cv.drop_shadow(t, rad, 10.0 * s, 2.0 * s, 0.45);
    cv.fill_round(t, rad, [0.125, 0.125, 0.135], 1.0);
    cv.stroke_round(t, rad, s, [0.27, 0.27, 0.29], 1.0);
    let map = |r: RectF| RectF::new(t.x + (r.x - t0.x) * k, t.y + (r.y - t0.y) * k, r.w * k, r.h * k);
    // Alça (seis pontinhos), só destravada (é por onde se arrasta).
    let gx = t.x + (3.0 + 14.0) * s * k;
    let dot: Rgb = if locked { [0.36, 0.36, 0.38] } else { [0.78, 0.78, 0.80] };
    for row in 0..3 {
        for col in 0..2 {
            cv.fill_circle(gx + (col as f32 * 7.0 - 3.5) * s * k, t.cy() + (row as f32 * 5.0 - 5.0) * s * k, 1.4 * s * k, dot, 1.0);
        }
    }
    let lock_glyph = if locked { "\u{E72E}" } else { "\u{E785}" };
    for (r, glyph, h) in [(g.lock, lock_glyph, LHit::Lock), (g.full, "\u{E740}", LHit::Full), (g.close, "\u{E711}", LHit::Close)] {
        let r = map(r);
        if v.hover == Some(h) {
            cv.fill_round(r.inset(2.5 * s * k), 11.0 * s * k, [0.27, 0.27, 0.29], 1.0);
        }
        ui::glyph(cv, tr, r, glyph, Font::IconMd, [1.0; 3], 1.0);
    }
    // Borda na cor de destaque: destravada, a janela pode ser arrastada.
    if !g.fullscreen && !locked {
        let th = theme(v.card_cfg, v.lc, v.colors, v.media);
        cv.stroke_round(g.card.inset(-1.0 * s), g.radius + 1.0 * s, 1.5 * s, th.accent, 0.85 * a);
    }
    // Dica do botão sob o mouse, logo abaixo da barrinha.
    if let (Some((msg, ta)), Some(b)) = (v.tip, [(g.lock, LHit::Lock), (g.full, LHit::Full), (g.close, LHit::Close)].iter().find(|(_, h)| v.hover == Some(*h)).map(|(r, _)| *r)) {
        let ta = ta * a;
        let h = 26.0 * s;
        let cw = cv.w as f32;
        let tw = tr.mask(msg, Font::Caption, (cw - 40.0 * s) as i32, h as i32, Align::Left).used_width();
        let w = (tw + 20.0 * s).round().min(cw - 8.0 * s);
        let x = (b.cx() - w / 2.0).clamp(4.0 * s, cw - 4.0 * s - w);
        let y = t.bottom() + 6.0 * s - (1.0 - ui::ease_enter(ta)) * 3.0 * s;
        let r = RectF::new(x.round(), y.round(), w, h);
        cv.drop_shadow(r, 6.0 * s, 12.0 * s, 3.0 * s, 0.45 * ta);
        cv.fill_round(r, 6.0 * s, [0.12, 0.12, 0.13], 0.97 * ta);
        cv.stroke_round(r, 6.0 * s, s, [1.0; 3], 0.12 * ta);
        ui::text_center(cv, tr, msg, Font::Caption, r, [1.0; 3], 0.94 * ta);
    }
}

/// Aviso rápido no meio do painel (ex.: acabou de travar: como destravar).
fn draw_toast(cv: &mut Canvas, tr: &mut TextRenderer, g: &Geo, msg: &str, a: f32, s: f32) {
    if a <= 0.0 {
        return;
    }
    let h = 34.0 * s;
    let maxw = (g.card.w - 16.0 * s).max(60.0 * s);
    let tw = tr.mask(msg, Font::BodyStrong, (maxw - 52.0 * s) as i32, h as i32, Align::Left).used_width();
    let w = (tw + 52.0 * s).min(maxw).round();
    let r = RectF::new((g.card.cx() - w / 2.0).round(), (g.card.cy() - h / 2.0).round(), w, h);
    let k = 0.96 + 0.04 * ui::ease_enter(a);
    let r = RectF::new(r.cx() - r.w * k / 2.0, r.cy() - r.h * k / 2.0, r.w * k, r.h * k);
    cv.drop_shadow(r, h / 2.0, 16.0 * s, 4.0 * s, 0.5 * a);
    cv.fill_round(r, h / 2.0, [0.08, 0.08, 0.09], 0.94 * a);
    cv.stroke_round(r, h / 2.0, s, [1.0; 3], 0.14 * a);
    ui::glyph(cv, tr, RectF::new(r.x + 12.0 * s, r.y, 20.0 * s, r.h), "\u{E72E}", Font::IconSmall, [1.0; 3], 0.9 * a);
    ui::text(cv, tr, msg, Font::BodyStrong, r.x + 36.0 * s, r.y, r.w - 46.0 * s, r.h, [1.0; 3], 0.95 * a);
}

/// Mini player do compacto: capa, título, artista, botões e progresso.
fn compact_header(cv: &mut Canvas, tr: &mut TextRenderer, g: &Geo, v: &View, th: &Theme) {
    let (s, m) = (v.s, v.media);
    let rad = 7.0 * s;
    let shadow = v.lc.background == Background::None;
    // Troca de música: a capa antiga dá lugar à nova e o texto antigo sobe
    // enquanto o novo entra por baixo (como no widget).
    let e = v.change.map(|(_, p)| ui::ease_out_cubic(p)).unwrap_or(1.0);
    match v.change {
        Some((old, _)) => {
            ui::draw_art_a(cv, tr, g.art, old, th, crate::config::Shape::Rounded, s, None, 1.0 - e);
            ui::draw_art_a(cv, tr, g.art, m, th, crate::config::Shape::Rounded, s, None, e);
        }
        None => {
            if let Some(a) = &m.art {
                ui::art_shadow(cv, g.art, rad, a, th, s);
            }
            ui::draw_art(cv, tr, g.art, m, th, crate::config::Shape::Rounded, s, None);
        }
    }
    let clip = cv.clip_rect();
    cv.set_clip(clip.intersect(RectF::new(g.title.x, g.art.y - 2.0 * s, g.title.w, g.art.h + 4.0 * s)));
    let mut texts = |cv: &mut Canvas, mi: &MediaInfo, alpha: f32, dy: f32| {
        let title = if !mi.present() { crate::i18n::tr("Nada tocando").to_string() } else if mi.title.is_empty() { mi.app.clone() } else { mi.title.clone() };
        let tm = tr.mask(&title, Font::Title, g.title.w as i32, g.title.h as i32, Align::Left);
        blit_text(cv, &tm, g.title.x, g.title.y + dy, th.fg, alpha, shadow, s);
        if !mi.artist.is_empty() {
            let am = tr.mask(&mi.artist, Font::Subtitle, g.artist.w as i32, g.artist.h as i32, Align::Left);
            blit_text(cv, &am, g.artist.x, g.artist.y + dy, th.fg, 0.7 * alpha, shadow, s);
        }
    };
    if let Some((old, _)) = v.change {
        texts(cv, old, 1.0 - e, -14.0 * s * e);
    }
    texts(cv, m, e, 14.0 * s * (1.0 - e));
    cv.set_clip(clip);
    let it = v.it.styled(v.card_cfg);
    let ct = ui::controls_theme(v.card_cfg.card_controls, &v.card_cfg.card_controls_rgb, th, m);
    for (r, h) in [(g.prev, Hit::Prev), (g.next, Hit::Next)] {
        if v.it.hover == Some(h) {
            let k = r.h - 4.0 * s;
            ui::hover_bg(cv, RectF::new(r.cx() - k / 2.0, r.cy() - k / 2.0, k, k), k / 2.0, th, v.it.pressed == Some(h));
        }
    }
    ui::ctrl_glyph(cv, tr, g.prev, G_PREV, Font::IconSmall, ct.fg, if m.can_prev || !m.present() { 1.0 } else { 0.35 }, &it, Hit::Prev, s);
    ui::play_button(cv, tr, g.play, g.play_size, v.card_cfg.play_shape, &ct, &it, v.pb.playing, m.can_toggle, s);
    ui::ctrl_glyph(cv, tr, g.next, G_NEXT, Font::IconSmall, ct.fg, if m.can_next || !m.present() { 1.0 } else { 0.35 }, &it, Hit::Next, s);
    // Progresso: fino; com o mouse em cima (dá para clicar e arrastar), engrossa e mostra a bolinha.
    let b = g.bar;
    let mut t = if m.dur > 0.0 { (v.pb.pos / m.dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
    if let Some(p) = v.pb.seek_preview {
        t = p;
    }
    let hot = m.can_seek && (v.it.hover == Some(Hit::Seek) || v.it.drag == Some(Hit::Seek));
    ui::slider(cv, b.x, b.right(), b.cy(), t, th, hot, hot, s * 0.8);
    if v.lc.show_time && m.dur > 0.0 {
        let pos = t as f64 * m.dur;
        ui::text(cv, tr, &ui::fmt_time(pos), Font::Small, b.x, b.bottom() + 3.0 * s, 60.0 * s, 14.0 * s, th.fg, 0.6);
        ui::text_right(cv, tr, &format!("-{}", ui::fmt_time((m.dur - pos).max(0.0))), Font::Small, b.right(), b.bottom() + 3.0 * s, 14.0 * s, th.fg, 0.6, s);
    }
}

thread_local! {
    /// Tela cheia: capa (com sombra), título e artista já desenhados, numa
    /// camada do tamanho da coluna da esquerda (a da música atual e a da
    /// anterior, para a troca cruzar as duas).
    static FS_LEFT: RefCell<Vec<(u64, RectF, Rc<Vec<u32>>)>> = const { RefCell::new(Vec::new()) };
}

/// Capa, título e artista da tela cheia (guardados entre quadros), com
/// opacidade `alpha` e deslocados `dy` px (troca de música).
#[allow(clippy::too_many_arguments)]
fn fullscreen_left(cv: &mut Canvas, tr: &mut TextRenderer, g: &Geo, m: &MediaInfo, art: Option<&crate::render::Image>, th: &Theme, s: f32, alpha: f32, dy: f32) {
    if alpha <= 0.003 {
        return;
    }
    let pad = 60.0 * s;
    let area = RectF::new((g.art.x - pad).floor(), (g.art.y - pad).floor(), (g.art.w + 2.0 * pad).ceil(), (g.artist.bottom() - g.art.y + 2.0 * pad).ceil());
    let key = ui::layer_key((bits(area), art.map(|a| (a.id, a.w)), &m.title, &m.artist, &m.album, &m.app, m.present(), th.dark));
    let (w, h) = (area.w as usize, area.h as usize);
    let px = FS_LEFT.with(|c| c.borrow().iter().find(|(k, r, _)| *k == key && *r == area).map(|(_, _, p)| p.clone()));
    let px = px.unwrap_or_else(|| {
        let mut px = vec![0u32; w * h];
        {
            let mut lc = Canvas::new(w, h, &mut px);
            let off = |r: RectF| r.offset(-area.x, -area.y);
            let rad = 14.0 * s;
            match art {
                Some(a) => {
                    ui::art_shadow(&mut lc, off(g.art), rad, a, th, s * 1.4);
                    lc.draw_image_cover(off(g.art), a, rad, 1.0);
                    lc.stroke_round(off(g.art), rad, s, [1.0; 3], 0.10);
                }
                None => ui::draw_art(&mut lc, tr, off(g.art), m, th, crate::config::Shape::Rounded, s, None),
            }
            let title = if !m.present() { crate::i18n::tr("Nada tocando").to_string() } else if m.title.is_empty() { m.app.clone() } else { m.title.clone() };
            let (t, a) = (off(g.title), off(g.artist));
            ui::text(&mut lc, tr, &title, Font::TitleXl, t.x, t.y, t.w, t.h, [1.0; 3], 1.0);
            ui::text(&mut lc, tr, &ui::artist_album(m), Font::SubtitleXl, a.x, a.y, a.w, a.h, [1.0; 3], 0.7);
        }
        let px = Rc::new(px);
        FS_LEFT.with(|c| {
            let mut c = c.borrow_mut();
            c.insert(0, (key, area, px.clone()));
            c.truncate(2);
        });
        px
    });
    cv.composite(area.x as i32, (area.y + dy).round() as i32, &px, w, h, alpha);
}

fn fullscreen(cv: &mut Canvas, tr: &mut TextRenderer, g: &Geo, v: &View, th: &Theme) {
    let (s, m) = (v.s, v.media);
    let e = v.change.map(|(_, p)| ui::ease_out_cubic(p)).unwrap_or(1.0);
    if let Some((old, _)) = v.change {
        fullscreen_left(cv, tr, g, old, old.art.as_deref(), th, s, 1.0 - e, -24.0 * s * e);
    }
    fullscreen_left(cv, tr, g, m, v.hires.or(m.art.as_deref()), th, s, e, 24.0 * s * (1.0 - e));
    let fg = [1.0; 3];
    // Progresso (clicável) e tempos.
    let hot = m.can_seek && (v.hover == Some(LHit::Card(Hit::Seek)) || v.it.drag == Some(Hit::Seek));
    let mut t = if m.dur > 0.0 { (v.pb.pos / m.dur) as f32 } else { 0.0 };
    if let Some(p) = v.pb.seek_preview {
        t = p;
    }
    ui::slider(cv, g.bar.x, g.bar.right(), g.bar.cy(), t, th, hot, hot, s);
    let pos = t as f64 * m.dur;
    if v.lc.show_time {
        ui::text(cv, tr, &ui::fmt_time(pos), Font::Body, g.bar.x, g.bar.bottom() + 8.0 * s, 90.0 * s, 20.0 * s, fg, 0.6);
        ui::text_right(cv, tr, &format!("-{}", ui::fmt_time((m.dur - pos).max(0.0))), Font::Body, g.bar.right(), g.bar.bottom() + 8.0 * s, 20.0 * s, fg, 0.6, s);
    }
    // Controles.
    let it = v.it.styled(v.card_cfg);
    let ct = ui::controls_theme(v.card_cfg.card_controls, &v.card_cfg.card_controls_rgb, th, m);
    for (r, h) in [(g.shuffle, Hit::Shuffle), (g.prev, Hit::Prev), (g.next, Hit::Next), (g.repeat, Hit::Repeat)] {
        if v.it.hover == Some(h) {
            let k = 48.0 * s;
            ui::hover_bg(cv, RectF::new(r.cx() - k / 2.0, r.cy() - k / 2.0, k, k), k / 2.0, th, v.it.pressed == Some(h));
        }
    }
    // Ícones maiores que os do card (em vetor, escalam com `s`).
    let k = s * 1.6;
    let toggle = |cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, gl: &str, on: bool, h: Hit| {
        let c = if on { ct.accent } else { ct.fg };
        ui::ctrl_glyph(cv, tr, r, gl, Font::Icon, c, if on { 1.0 } else { 0.8 }, &it, h, k);
        if on {
            cv.fill_circle(r.cx(), r.bottom() - 4.0 * s, 2.4 * s, ct.accent, 1.0);
        }
    };
    if m.can_shuffle {
        toggle(cv, tr, g.shuffle, G_SHUFFLE, v.pb.shuffle, Hit::Shuffle);
    }
    ui::ctrl_glyph(cv, tr, g.prev, G_PREV, Font::Icon, ct.fg, 1.0, &it, Hit::Prev, k);
    ui::play_button(cv, tr, g.play, g.play_size, v.card_cfg.play_shape, &ct, &it, v.pb.playing, m.can_toggle, s * 1.5);
    ui::ctrl_glyph(cv, tr, g.next, G_NEXT, Font::Icon, ct.fg, 1.0, &it, Hit::Next, k);
    if m.can_repeat {
        let gl = if v.pb.repeat == 1 { G_REPEAT_ONE } else { G_REPEAT_ALL };
        toggle(cv, tr, g.repeat, gl, v.pb.repeat != 0, Hit::Repeat);
    }
    // A letra, grande.
    lyrics_area(cv, tr, g.lyr, v, g.big, g.dim, 0.44, fg, false);
    // Sair da tela cheia.
    let hot = v.hover == Some(LHit::Full);
    cv.fill_round(g.full, g.full.h / 2.0, [1.0; 3], if hot { 0.20 } else { 0.10 });
    ui::glyph(cv, tr, g.full, "\u{E73F}", Font::IconSmall, fg, 0.9);
}

/// Borda tracejada de um retângulo arredondado (os cantos inteiros, as
/// retas em traços).
#[allow(clippy::too_many_arguments)]
fn dashed(cv: &mut Canvas, r: RectF, radius: f32, width: f32, dash: f32, gap: f32, c: Rgb, a: f32) {
    let clip = cv.clip_rect();
    for (cx, cy) in [(r.x, r.y), (r.right() - radius, r.y), (r.x, r.bottom() - radius), (r.right() - radius, r.bottom() - radius)] {
        cv.set_clip(clip.intersect(RectF::new(cx - width, cy - width, radius + 2.0 * width, radius + 2.0 * width)));
        cv.stroke_round(r, radius, width, c, a);
    }
    cv.set_clip(clip);
    let mut seg = |x0: f32, y0: f32, len: f32, horiz: bool| {
        let mut t = (len % (dash + gap)) / 2.0 + gap / 2.0;
        while t + dash <= len {
            if horiz {
                cv.fill_rrect(x0 + t, y0 - width / 2.0, x0 + t + dash, y0 + width / 2.0, width / 2.0, a, c);
            } else {
                cv.fill_rrect(x0 - width / 2.0, y0 + t, x0 + width / 2.0, y0 + t + dash, width / 2.0, a, c);
            }
            t += dash + gap;
        }
    };
    let (hl, vl) = (r.w - 2.0 * radius, r.h - 2.0 * radius);
    seg(r.x + radius, r.y + width / 2.0, hl, true);
    seg(r.x + radius, r.bottom() - width / 2.0, hl, true);
    seg(r.x + width / 2.0, r.y + radius, vl, false);
    seg(r.right() - width / 2.0, r.y + radius, vl, false);
}

/// Lugares onde soltar a janela (durante o arrasto): borda tracejada e o
/// nome de cada um; o que vai receber a janela fica destacado ("Solte aqui").
pub fn draw_guides(cv: &mut Canvas, tr: &mut TextRenderer, slots: &[(RectF, &str, bool)], accent: Rgb, radius: f32, s: f32) {
    for &(r, name, on) in slots {
        if on {
            cv.fill_round(r, radius, accent, 0.20);
            dashed(cv, r, radius, 2.5 * s, 10.0 * s, 6.0 * s, accent, 1.0);
        } else {
            cv.fill_round(r, radius, [0.05, 0.05, 0.06], 0.28);
            dashed(cv, r, radius, 2.0 * s, 9.0 * s, 7.0 * s, [1.0; 3], 0.70);
        }
        let label = if on { crate::i18n::tr("Solte aqui") } else { name };
        let font = if on { Font::BodyStrong } else { Font::Caption };
        let h = 24.0 * s;
        let m = tr.mask(label, font, (r.w - 16.0 * s).max(10.0) as i32, h as i32, Align::Center);
        let x = r.x + 8.0 * s;
        let y = (r.cy() - h / 2.0).round();
        cv.blit_mask((x + 1.0 * s).round() as i32, (y + 1.0 * s) as i32, &m, [0.0; 3], 0.6);
        cv.blit_mask(x.round() as i32, y as i32, &m, [1.0; 3], if on { 1.0 } else { 0.85 });
    }
}

/// Desenha a sobreposição inteira.
pub fn draw(cv: &mut Canvas, tr: &mut TextRenderer, g: &Geo, v: &View) {
    let th = theme(v.card_cfg, v.lc, v.colors, v.media);
    if g.fullscreen {
        let mix = v.change.map(|(_, p)| p).unwrap_or(1.0);
        crate::lyrics_bg::fill(cv, v.media.art.as_deref(), v.bg_time, mix);
        fullscreen(cv, tr, g, v, &th);
        return;
    }
    panel(cv, g, v, &th);
    let lc = v.lc;
    match lc.layout {
        Layout::Player | Layout::PlayerLyrics => {
            if let Some(l) = &g.cl {
                let f = CardFrame {
                    cfg: v.card_cfg,
                    colors: v.colors,
                    media: v.media,
                    theme: th,
                    it: v.it,
                    pb: v.pb,
                    s: v.s,
                    backdrop: None,
                    hint: None,
                    launchers: &[],
                    viz: None,
                    mixer: v.mixer,
                };
                ui::draw_card(cv, tr, l, &f);
                if lc.layout == Layout::PlayerLyrics {
                    let top = g.lyr.y - 8.0 * v.s;
                    cv.fill_rrect(g.lyr.x, top, g.lyr.right(), top + v.s, 0.0, 0.10, th.fg);
                    lyrics_area(cv, tr, g.lyr, v, g.big, g.dim, 0.42, th.fg, false);
                }
            }
        }
        Layout::Compact => {
            compact_header(cv, tr, g, v, &th);
            lyrics_area(cv, tr, g.lyr, v, g.big, g.dim, 0.40, th.fg, true);
        }
        Layout::Lyrics => lyrics_area(cv, tr, g.lyr, v, g.big, g.dim, 0.5, th.fg, false),
        Layout::Caption => caption(cv, tr, g, v, &th),
    }
    draw_toolbar(cv, tr, g, v);
    if let Some((msg, a)) = v.toast {
        draw_toast(cv, tr, g, msg, a, v.s);
    }
}

/// Legenda: a linha atual (até 2 linhas) e a próxima, menor; na troca a
/// antiga sobe e some enquanto a nova entra.
fn caption(cv: &mut Canvas, tr: &mut TextRenderer, g: &Geo, v: &View, th: &Theme) {
    let s = v.s;
    let shadow = v.lc.background == Background::None;
    let r = g.lyr;
    let (a, b) = caption_lines(&v.frame);
    if a.is_none() && b.is_none() {
        let (title, _) = message(v.frame.lyrics, v.media.present());
        let m = tr.mask(&title, g.big, r.w as i32, r.h as i32, Align::Center);
        blit_text(cv, &m, r.x, r.y, th.fg, 0.75, shadow, s);
        return;
    }
    let frac = v.frame.focus - v.frame.focus.floor();
    let maxw = ((CAPTION_MAX - 48.0) * s) as i32;
    let clip = cv.clip_rect();
    cv.set_clip(clip.intersect(g.card));
    let mut draw_one = |cv: &mut Canvas, i: usize, alpha: f32, dy: f32| {
        let (cur, next) = caption_texts(&v.frame, i);
        let m = tr.mask_wrapped(&cur, g.big, maxw, 2, Align::Center);
        let n = tr.mask_wrapped(next, g.dim, maxw, 1, Align::Center);
        let total = m.h as f32 + if next.is_empty() { 0.0 } else { n.h as f32 + 2.0 * s };
        let y = r.cy() - total / 2.0 + dy;
        let x = r.cx() - maxw as f32 / 2.0;
        blit_text(cv, &m, x, y, th.fg, alpha, shadow, s);
        if !next.is_empty() {
            blit_text(cv, &n, x, y + m.h as f32 + 2.0 * s, th.fg, 0.55 * alpha, shadow, s);
        }
    };
    let e = ui::ease_out_cubic(frac);
    if let (Some(old), Some(new)) = (a, b) {
        // Uma depois da outra (cruzadas, as duas ficavam ilegíveis no meio).
        draw_one(cv, old, (1.0 - 2.0 * e).max(0.0), -12.0 * s * e);
        draw_one(cv, new, (2.0 * e - 1.0).max(0.0), 12.0 * s * (1.0 - e));
    } else if let Some(i) = b {
        draw_one(cv, i, 1.0, 0.0);
    }
    cv.set_clip(clip);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::config::Position;
    use crate::render::lerp3;

    /// Letra inventada para as prévias e capturas.
    pub const SAMPLE_LRC: &str = "[00:06.00]As luzes da cidade acendem devagar\n[00:12.00]E o rádio toca a nossa canção\n[00:18.00]Corre comigo até o sol nascer\n[00:24.00]Ninguém vai dormir esta noite, nem amanhã, nem depois do fim do mundo\n[00:30.00]O céu inteiro cabe na janela\n[00:36.00]E a estrada some no retrovisor\n[00:42.00]";

    fn colors() -> Colors {
        Colors { palette: crate::config::Palette::Album, accent: [0.93, 0.32, 0.62], accent_light: [0.98, 0.62, 0.80], light_taskbar: false, album: None }
    }

    fn save(name: &str, w: usize, h: usize, px: &[u32]) {
        use windows::Win32::Foundation::GENERIC_WRITE;
        use windows::Win32::Graphics::Imaging::*;
        use windows::Win32::System::Com::StructuredStorage::IPropertyBag2;
        use windows::Win32::System::Com::*;
        let Some(dir) = std::env::var_os("TASKBEAT_SHOTS_DIR") else { return };
        let _ = std::fs::create_dir_all(&dir);
        let mut buf = Vec::with_capacity(w * h * 4);
        for &p in px {
            let a = (p >> 24) & 0xff;
            let un = |c: u32| if a == 0 { 0 } else { ((c * 255 + a / 2) / a).min(255) as u8 };
            buf.extend_from_slice(&[un(p & 0xff), un((p >> 8) & 0xff), un((p >> 16) & 0xff), a as u8]);
        }
        let path = std::path::Path::new(&dir).join(name);
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let f: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
            let stream = f.CreateStream().unwrap();
            stream.InitializeFromFilename(&windows::core::HSTRING::from(path.as_os_str()), GENERIC_WRITE.0).unwrap();
            let enc = f.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()).unwrap();
            enc.Initialize(&stream, WICBitmapEncoderNoCache).unwrap();
            let mut frame = None;
            enc.CreateNewFrame(&mut frame, std::ptr::null_mut()).unwrap();
            let frame = frame.unwrap();
            frame.Initialize(None::<&IPropertyBag2>).unwrap();
            frame.SetSize(w as u32, h as u32).unwrap();
            let mut fmt = GUID_WICPixelFormat32bppBGRA;
            frame.SetPixelFormat(&mut fmt).unwrap();
            frame.WritePixels(h as u32, (w * 4) as u32, &buf).unwrap();
            frame.Commit().unwrap();
            enc.Commit().unwrap();
        }
    }

    /// Desenha cada layout (e a tela cheia) com a música e a letra de exemplo:
    /// `$env:TASKBEAT_SHOTS_DIR="…"; cargo test lyrics_view_shots -- --ignored`
    #[test]
    #[ignore]
    fn lyrics_view_shots() {
        let s = 2.0;
        let mut tr = TextRenderer::new();
        tr.set_scale(s);
        let base = Config::default();
        let media = ui::sample_media();
        let c = Colors { album: media.art.as_ref().map(|a| a.vibrant), ..colors() };
        let lines = crate::lyrics::with_gaps(&crate::lyrics::parse_lrc(SAMPLE_LRC));
        let lyr = Lyrics::Synced(crate::lyrics::parse_lrc(SAMPLE_LRC));
        let pb = Playback { playing: true, pos: 110.0, ..Default::default() };
        let cases: Vec<(&str, Layout, Background, f32, f32, bool)> = vec![
            ("lyrics", Layout::Lyrics, Background::Artwork, 3.0, 0.0, false),
            ("lyrics-anim", Layout::Lyrics, Background::Artwork, 2.5, 0.0, false),
            ("lyrics-tools", Layout::Lyrics, Background::Artwork, 3.0, 1.0, false),
            ("player", Layout::Player, Background::Artwork, 3.0, 0.0, false),
            ("player-lyrics", Layout::PlayerLyrics, Background::Artwork, 4.0, 0.0, false),
            ("compact", Layout::Compact, Background::Artwork, 3.0, 0.0, false),
            ("compact-album", Layout::Compact, Background::AlbumColor, 0.0, 0.0, false),
            ("caption", Layout::Caption, Background::Dark, 3.0, 0.0, false),
            ("caption-anim", Layout::Caption, Background::Dark, 3.5, 0.0, false),
            ("caption-none", Layout::Caption, Background::None, 4.0, 0.0, false),
            ("lyrics-notfound", Layout::Lyrics, Background::Dark, 0.0, 0.0, true),
        ];
        for (name, layout, bg, focus, tools, notfound) in cases {
            let lc = LyricsConfig { layout, background: bg, position: Position::TopRight, ..Default::default() };
            let cc = card_config(&base, &lc);
            let l = if notfound { &Lyrics::NotFound } else { &lyr };
            let frame = LyricFrame { lyrics: l, lines: &lines, focus, scroll: 0.0 };
            let cap = if layout == Layout::Caption { caption_content(&mut tr, &frame, &lc, s) } else { (0.0, 0.0) };
            let g = geometry(&lc, &cc, s, cap, None);
            let (w, h) = (g.w as usize, g.h as usize);
            let mut px = vec![0u32; w * h];
            {
                let mut cv = Canvas::new(w, h, &mut px);
                // Fundo de "área de trabalho" para ver as sombras e o "sem fundo".
                cv.fill_rrect_ca(0.0, 0.0, w as f32, h as f32, 0.0, |x, y| (lerp3([0.10, 0.18, 0.45], [0.35, 0.20, 0.55], (x / w as f32 + y / h as f32) / 2.0), 1.0));
                let hover = (tools > 0.0).then_some(LHit::Lock);
                let tip = (tools > 0.0).then_some(("Travar a posição (Win+Alt+Shift+L)", 1.0));
                let toast = notfound.then_some(("Posição travada", 1.0));
                let v = View { card_cfg: &cc, lc: &lc, colors: &c, media: &media, pb, it: Interact::default(), hover, tools, bg_time: None, hires: None, change: None, mixer: None, lyr_alpha: 1.0, tip, toast, frame, s };
                draw(&mut cv, &mut tr, &g, &v);
            }
            save(&format!("{name}.png"), w, h, &px);
        }
        // Tela cheia (monitor 1600×900 a 100%).
        let lc = LyricsConfig::default();
        let cc = card_config(&base, &lc);
        let g = fullscreen_geometry(1600.0, 900.0, 1.0);
        let mut tr1 = TextRenderer::new();
        tr1.set_scale(1.0);
        let (w, h) = (1600, 900);
        let mut px = vec![0u32; w * h];
        {
            let mut cv = Canvas::new(w, h, &mut px);
            let frame = LyricFrame { lyrics: &lyr, lines: &lines, focus: 3.0, scroll: 0.0 };
            let v = View { card_cfg: &cc, lc: &lc, colors: &c, media: &media, pb, it: Interact::default(), hover: None, tools: 0.0, bg_time: Some(7.0), hires: None, change: None, mixer: None, lyr_alpha: 1.0, tip: None, toast: None, frame, s: 1.0 };
            draw(&mut cv, &mut tr1, &g, &v);
        }
        save("fullscreen.png", w, h, &px);
    }
}
