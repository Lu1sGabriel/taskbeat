//! Propostas de identidade da tela de configurações, para escolher a direção
//! antes de mudar o app: três estilos desenhados pelo próprio renderizador,
//! com a música de exemplo e a configuração padrão. Gera PNGs na pasta de
//! `TASKBEAT_SHOTS_DIR` (ou na pasta temporária):
//!
//! ```powershell
//! $env:TASKBEAT_SHOTS_DIR="..."; cargo test settings_mock -- --ignored --nocapture
//! ```

use crate::config::*;
use crate::render::*;
use crate::shared::MediaInfo;
use crate::text::*;
use crate::ui::{self, CLayout, CardFrame, Interact, Playback, WLayout, WidgetFrame};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Dir {
    /// Escuro com a cor da música, cartões como o card expandido.
    Palco,
    /// Grafite liso, linhas finas e o degradê da marca; controles de mesa de som.
    Estudio,
    /// Vidro fosco sobre a capa desfocada, cantos grandes e ícones coloridos.
    Ilha,
}

/// Nível de cantos arredondados (opção "Cantos": Menos é o padrão).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Round {
    Menos,
    Normal,
    Mais,
}

struct Look {
    dir: Dir,
    round: Round,
    /// Cor de destaque (da capa no Palco e na Ilha; a da marca no Estúdio).
    accent: Rgb,
    /// Fim do degradê do destaque.
    accent2: Rgb,
}

const WHITE: Rgb = [1.0; 3];
const INK: Rgb = [0.06, 0.06, 0.08];
/// Degradê da marca (o mesmo do logo).
const BRAND_A: Rgb = [0.93, 0.27, 0.62];
const BRAND_B: Rgb = [1.0, 0.55, 0.28];

impl Look {
    fn new(dir: Dir, round: Round, media: &MediaInfo) -> Look {
        let vib = media.art.as_ref().map(|a| a.vibrant).unwrap_or([0.6, 0.5, 1.0]);
        let album = readable(vib, true);
        match dir {
            Dir::Estudio => Look { dir, round, accent: BRAND_A, accent2: BRAND_B },
            Dir::Palco => Look { dir, round, accent: album, accent2: lerp3(album, WHITE, 0.25) },
            Dir::Ilha => Look { dir, round, accent: lerp3(album, WHITE, 0.08), accent2: lerp3(album, WHITE, 0.3) },
        }
    }

    /// Raio de um controle de altura `h` (px): `base` (px lógicos) no "Menos",
    /// o dobro no "Normal" e pílula no "Mais". O Estúdio é sempre mais reto.
    fn r(&self, base: f32, h: f32, s: f32) -> f32 {
        let k = if self.dir == Dir::Estudio { 0.5 } else { 1.0 };
        match self.round {
            Round::Menos => base * k * s,
            Round::Normal => (base * 2.0 * k * s).min(h / 2.0),
            Round::Mais => h / 2.0,
        }
    }

    /// Raio de um painel ou cartão grande (nunca vira pílula).
    fn rp(&self, base: f32, s: f32) -> f32 {
        let k = if self.dir == Dir::Estudio { 0.5 } else { 1.0 };
        base * k * s * match self.round {
            Round::Menos => 1.0,
            Round::Normal => 1.6,
            Round::Mais => 2.3,
        }
    }

    /// Cantos do card, do "A seguir" e do aviso na prévia (o mesmo nível).
    fn corners(&self) -> Corners {
        match self.round {
            Round::Menos => Corners::Small,
            Round::Normal => Corners::Medium,
            Round::Mais => Corners::Large,
        }
    }

    /// Preenchimento do destaque (degradê horizontal no Estúdio, vertical suave nos outros).
    fn fill_accent(&self, cv: &mut Canvas, r: RectF, radius: f32, a: f32) {
        let (a0, a1) = (self.accent, self.accent2);
        let (x0, y0) = (r.x, r.y);
        match self.dir {
            Dir::Estudio => cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), radius, |x, _| (lerp3(a0, a1, ((x - x0) / r.w.max(1.0)).clamp(0.0, 1.0)), a)),
            _ => cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), radius, |_, y| (lerp3(a1, a0, ((y - y0) / r.h.max(1.0)).clamp(0.0, 1.0)), a)),
        }
    }
}

// ---------------------------------------------------------------- conteúdo de exemplo

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum Ctl {
    Toggle(bool),
    Seg(&'static [&'static str], usize),
    List(&'static str),
    Slider(f32, &'static str),
    Button(&'static str, bool),
}

#[derive(Clone, Copy)]
struct RowM {
    title: &'static str,
    desc: &'static str,
    ctl: Ctl,
    enabled: bool,
    hover: bool,
}

const fn row(title: &'static str, desc: &'static str, ctl: Ctl) -> RowM {
    RowM { title, desc, ctl, enabled: true, hover: false }
}

/// Aba "Fundo" da página da barra: um pouco de cada controle.
const FUNDO: &[RowM] = &[
    row("Fundo atrás da música", "Fundo sutil sempre visível, com a música e os botões dentro", Ctl::Toggle(true)),
    row("Formato", "Cantos do fundo do widget", Ctl::Seg(&["Arredondado", "Pílula", "Reto"], 0)),
    RowM { hover: true, ..row("Cor do fundo", "Neutro, cor da capa ou de destaque; ou fundos fortes: o degradê vivo da capa ou a própria capa desfocada", Ctl::List("Capa desfocada")) },
    row("Intensidade da cor", "Quanto o degradê ou a capa desfocada cobrem a barra", Ctl::Slider(0.78, "80%")),
    RowM { enabled: false, ..row("Opacidade do fundo", "Com o degradê ou a capa desfocada, quem manda é a intensidade", Ctl::Slider(0.06, "6%")) },
    row("Brilho da capa", "Luz suave na cor da capa atrás dela", Ctl::Toggle(true)),
    row("Contorno", "Borda fina em volta do fundo", Ctl::Toggle(false)),
];

const TABS: &[&str] = &["Conteúdo", "Botões", "Fundo", "Posição", "Barras de áudio"];

/// Menu: (ícone, nome, cor do ícone na Ilha). `None` = título de grupo.
const NAV: &[(&str, &str, Rgb)] = &[
    ("\u{E80F}", "Início", [0.36, 0.55, 1.0]),
    ("\u{E7F4}", "Barra de tarefas", [0.18, 0.70, 0.95]),
    ("\u{E7C4}", "Card", [0.60, 0.40, 1.0]),
    ("\u{E7F0}", "Letras", [1.0, 0.38, 0.55]),
    ("\u{E765}", "Caps Lock", [1.0, 0.60, 0.22]),
    ("", "PARA TODOS", [0.0; 3]),
    ("\u{E790}", "Cores e ícones", [0.95, 0.35, 0.72]),
    ("\u{E8F1}", "Apps de música", [0.25, 0.78, 0.45]),
    ("\u{E713}", "Geral", [0.55, 0.56, 0.62]),
];

// ---------------------------------------------------------------- utilidades

fn render(w: f32, h: f32, f: impl FnOnce(&mut Canvas)) -> (usize, usize, Vec<u32>) {
    let (w, h) = (w.ceil() as usize, h.ceil() as usize);
    let mut px = vec![0u32; w * h];
    f(&mut Canvas::new(w, h, &mut px));
    (w, h, px)
}

fn blit(cv: &mut Canvas, x: f32, y: f32, layer: &(usize, usize, Vec<u32>)) {
    cv.composite(x.round() as i32, y.round() as i32, &layer.2, layer.0, layer.1, 1.0);
}

fn text_w(tr: &mut TextRenderer, t: &str, font: Font, s: f32) -> f32 {
    tr.mask(t, font, (900.0 * s) as i32, (30.0 * s) as i32, Align::Left).used_width()
}

fn out_dir() -> std::path::PathBuf {
    let d = std::env::var_os("TASKBEAT_SHOTS_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("taskbeat-mock"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Grava sem transparência (como a janela aparece na tela).
fn save(name: &str, img: &(usize, usize, Vec<u32>)) {
    let px: Vec<u32> = img.2.iter().map(|&p| p | 0xFF00_0000).collect();
    let path = out_dir().join(name);
    crate::png::save(&path, img.0, img.1, &px);
    println!("{} ({}x{})", path.display(), img.0, img.1);
}

fn colors(cfg: &Config, media: &MediaInfo) -> Colors {
    Colors { palette: cfg.palette, accent: [0.93, 0.32, 0.62], accent_light: [0.98, 0.62, 0.80], light_taskbar: false, album: media.art.as_ref().map(|a| a.vibrant) }
}

fn playback(media: &MediaInfo) -> Playback {
    Playback { playing: true, shuffle: media.shuffle, repeat: media.repeat, pos: media.pos, seek_preview: None, volume: 0.62, muted: false }
}

// ---------------------------------------------------------------- fundo da janela

fn window_bg(cv: &mut Canvas, look: &Look, w: f32, h: f32, s: f32, media: &MediaInfo) {
    let full = RectF::new(0.0, 0.0, w, h);
    match look.dir {
        Dir::Palco => {
            cv.fill_round(full, 0.0, [0.043, 0.043, 0.054], 1.0);
            // Luz da cor da música, como no card: forte atrás do menu, fraca no palco.
            let (a, b) = (look.accent, media.art.as_ref().map(|i| i.avg).unwrap_or(look.accent));
            cv.fill_rrect_ca(0.0, 0.0, w, h, 0.0, |x, y| {
                let (u, v) = (x / w, y / h);
                let l1 = (-(((u - 0.05) * 2.4).powi(2) + ((v - 0.0) * 2.0).powi(2))).exp();
                let l2 = (-(((u - 0.95) * 2.0).powi(2) + ((v - 1.0) * 2.2).powi(2))).exp();
                let c = lerp3(a, b, l2 / (l1 + l2 + 1e-3));
                (c, 0.16 * l1 + 0.10 * l2)
            });
            cv.grain(full, 0.0, 0.016);
        }
        Dir::Estudio => {
            cv.fill_round(full, 0.0, [0.071, 0.073, 0.082], 1.0);
            let nav = RectF::new(0.0, 0.0, 236.0 * s, h);
            cv.fill_round(nav, 0.0, [0.052, 0.053, 0.060], 1.0);
            cv.fill_rrect(nav.right() - s, 0.0, nav.right(), h, 0.0, 0.07, WHITE);
        }
        Dir::Ilha => {
            // A capa desfocada cobrindo a janela, bem escura: as cores da música
            // atrás do vidro fosco.
            if let Some(art) = &media.art {
                let small = art.resampled(RectF::new(0.0, 0.0, art.w as f32, art.h as f32), 5, 5);
                cv.draw_image_cover(full, &small, 0.0, 1.0);
            }
            cv.fill_rrect_ca(0.0, 0.0, w, h, 0.0, |x, y| {
                let (u, v) = (x / w, y / h);
                ([0.03, 0.03, 0.045], (0.62 + 0.18 * v - 0.08 * (1.0 - u)).clamp(0.0, 0.9))
            });
        }
    }
}

/// Barra de título (a do sistema, escura).
fn title_bar(cv: &mut Canvas, tr: &mut TextRenderer, look: &Look, w: f32, s: f32) {
    let h = 32.0 * s;
    if look.dir == Dir::Estudio {
        cv.fill_rrect(0.0, 0.0, w, h, 0.0, 1.0, [0.052, 0.053, 0.060]);
        cv.fill_rrect(0.0, h - s, w, h, 0.0, 0.07, WHITE);
    }
    ui::text(cv, tr, "Taskbeat — Configurações", Font::Caption, 14.0 * s, 0.0, 400.0 * s, h, WHITE, 0.72);
    for (k, g) in ["\u{E8BB}", "\u{E922}", "\u{E921}"].iter().enumerate() {
        ui::glyph(cv, tr, RectF::new(w - (k as f32 + 1.0) * 46.0 * s, 0.0, 46.0 * s, h), g, Font::IconSmall, WHITE, 0.8);
    }
}

// ---------------------------------------------------------------- menu lateral

fn nav(cv: &mut Canvas, tr: &mut TextRenderer, look: &Look, h: f32, s: f32, selected: usize, search: Option<&str>) {
    let top = 32.0 * s;
    let x0 = match look.dir {
        Dir::Ilha => 12.0 * s,
        _ => 0.0,
    };
    let nw = 236.0 * s;
    if look.dir == Dir::Ilha {
        let p = RectF::new(x0, top + 8.0 * s, nw - 2.0 * x0 + 6.0 * s, h - top - 20.0 * s);
        frost(cv, p, look.rp(10.0, s), s);
    }
    // Logo e nome.
    let ly = top + 22.0 * s;
    crate::settings::draw_logo(cv, RectF::new(x0 + 20.0 * s, ly, 34.0 * s, 34.0 * s));
    ui::text(cv, tr, "Taskbeat", Font::Display, x0 + 64.0 * s, ly - 2.0 * s, 160.0 * s, 24.0 * s, WHITE, 1.0);
    let sub = if look.dir == Dir::Estudio { "CONFIGURAÇÕES" } else { "Configurações" };
    let sub_font = if look.dir == Dir::Estudio { Font::Label } else { Font::Caption };
    ui::text(cv, tr, sub, sub_font, x0 + 65.0 * s, ly + 18.0 * s, 160.0 * s, 18.0 * s, WHITE, 0.5);

    // Busca.
    let sr = RectF::new(x0 + 16.0 * s, ly + 52.0 * s, nw - 2.0 * x0 - 26.0 * s, 34.0 * s);
    let focused = search.is_some();
    match look.dir {
        Dir::Palco => {
            cv.fill_round(sr, look.r(6.0, sr.h, s), WHITE, if focused { 0.10 } else { 0.06 });
            cv.stroke_round(sr, look.r(6.0, sr.h, s), s, if focused { look.accent } else { WHITE }, if focused { 0.9 } else { 0.07 });
        }
        Dir::Estudio => {
            cv.fill_round(sr, look.r(6.0, sr.h, s), WHITE, 0.035);
            cv.stroke_round(sr, look.r(6.0, sr.h, s), s, if focused { look.accent } else { WHITE }, if focused { 1.0 } else { 0.14 });
            let k = RectF::new(sr.right() - 52.0 * s, sr.y + 8.0 * s, 44.0 * s, sr.h - 16.0 * s);
            cv.stroke_round(k, 3.0 * s, s, WHITE, 0.2);
            ui::text_center(cv, tr, "Ctrl F", Font::Small, k, WHITE, 0.5);
        }
        Dir::Ilha => {
            cv.fill_round(sr, look.r(6.0, sr.h, s), WHITE, if focused { 0.16 } else { 0.10 });
            if focused {
                cv.stroke_round(sr, look.r(6.0, sr.h, s), 1.5 * s, look.accent, 0.9);
            }
        }
    }
    ui::glyph(cv, tr, RectF::new(sr.x + 8.0 * s, sr.y, 22.0 * s, sr.h), "\u{E721}", Font::IconSmall, WHITE, 0.6);
    match search {
        Some(q) => {
            ui::text(cv, tr, q, Font::Body, sr.x + 34.0 * s, sr.y, sr.w - 60.0 * s, sr.h, WHITE, 1.0);
            let cx = sr.x + 35.0 * s + text_w(tr, q, Font::Body, s);
            cv.fill_rrect(cx, sr.y + 9.0 * s, cx + 1.5 * s, sr.bottom() - 9.0 * s, 0.5 * s, 1.0, WHITE);
            ui::glyph(cv, tr, RectF::new(sr.right() - 30.0 * s, sr.y, 22.0 * s, sr.h), "\u{E711}", Font::IconSmall, WHITE, 0.6);
        }
        None => ui::text(cv, tr, "Buscar opção", Font::Body, sr.x + 34.0 * s, sr.y, sr.w - 40.0 * s, sr.h, WHITE, 0.42),
    }

    // Itens.
    let mut y = sr.bottom() + 18.0 * s;
    let mut k = 0;
    for (icon, name, col) in NAV {
        if icon.is_empty() {
            y += 10.0 * s;
            let (font, a) = (Font::Label, 0.45);
            let label = if look.dir == Dir::Ilha { "Para todos" } else { *name };
            let font = if look.dir == Dir::Ilha { Font::CaptionStrong } else { font };
            ui::text(cv, tr, label, font, x0 + 24.0 * s, y, 180.0 * s, 20.0 * s, WHITE, a);
            y += 24.0 * s;
            continue;
        }
        let sel = k == selected && search.is_none();
        let r = RectF::new(x0 + 10.0 * s, y, nw - 2.0 * x0 - 14.0 * s, 40.0 * s);
        let chip = RectF::new(r.x + 10.0 * s, r.cy() - 14.0 * s, 28.0 * s, 28.0 * s);
        match look.dir {
            Dir::Palco => {
                if sel {
                    cv.fill_round(r, look.r(6.0, r.h, s), WHITE, 0.075);
                    cv.glow(chip, look.r(5.0, chip.h, s), 10.0 * s, 2.0 * s, look.accent, 0.45);
                    look.fill_accent(cv, chip, look.r(5.0, chip.h, s), 1.0);
                    ui::glyph(cv, tr, chip, icon, Font::IconSmall, WHITE, 1.0);
                } else {
                    cv.fill_round(chip, look.r(5.0, chip.h, s), WHITE, 0.05);
                    ui::glyph(cv, tr, chip, icon, Font::IconSmall, WHITE, 0.78);
                }
                let font = if sel { Font::BodyStrong } else { Font::Body };
                ui::text(cv, tr, name, font, chip.right() + 12.0 * s, r.y, 150.0 * s, r.h, WHITE, if sel { 1.0 } else { 0.78 });
            }
            Dir::Estudio => {
                if sel {
                    cv.fill_round(r, look.r(6.0, r.h, s), WHITE, 0.045);
                    let bar = RectF::new(r.x, r.y + 9.0 * s, 3.0 * s, r.h - 18.0 * s);
                    cv.fill_rrect_ca(bar.x, bar.y, bar.right(), bar.bottom(), 1.5 * s, |_, yy| (lerp3(BRAND_A, BRAND_B, ((yy - bar.y) / bar.h).clamp(0.0, 1.0)), 1.0));
                }
                ui::glyph(cv, tr, chip, icon, Font::IconSmall, if sel { look.accent } else { WHITE }, if sel { 1.0 } else { 0.6 });
                ui::text(cv, tr, name, Font::Body, chip.right() + 10.0 * s, r.y, 150.0 * s, r.h, WHITE, if sel { 1.0 } else { 0.62 });
            }
            Dir::Ilha => {
                if sel {
                    cv.fill_round(r, look.r(6.0, r.h, s), WHITE, 0.14);
                }
                let c = RectF::new(chip.x, chip.y + 1.0 * s, 26.0 * s, 26.0 * s);
                let top = lerp3(*col, WHITE, 0.18);
                let (cy0, ch) = (c.y, c.h);
                cv.fill_rrect_ca(c.x, c.y, c.right(), c.bottom(), look.r(5.0, c.h, s), |_, yy| (lerp3(top, *col, ((yy - cy0) / ch).clamp(0.0, 1.0)), 1.0));
                ui::glyph(cv, tr, c, icon, Font::IconSmall, WHITE, 1.0);
                ui::text(cv, tr, name, if sel { Font::BodyStrong } else { Font::Body }, chip.right() + 12.0 * s, r.y, 150.0 * s, r.h, WHITE, if sel { 1.0 } else { 0.85 });
            }
        }
        y += 44.0 * s;
        k += 1;
    }
    let vy = h - 34.0 * s;
    ui::text(cv, tr, "v1.3.0 · feito em Rust", Font::Caption, x0 + 24.0 * s, vy, 200.0 * s, 18.0 * s, WHITE, 0.38);
}

/// Painel de vidro fosco (Ilha).
fn frost(cv: &mut Canvas, r: RectF, radius: f32, s: f32) {
    cv.drop_shadow(r, radius, 24.0 * s, 8.0 * s, 0.30);
    cv.fill_round(r, radius, WHITE, 0.075);
    let y0 = r.y;
    cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), radius, |_, y| (WHITE, if y < y0 + 1.5 * s { 0.14 } else { 0.0 }));
    cv.stroke_round(r, radius, s, WHITE, 0.13);
}

// ---------------------------------------------------------------- controles

fn toggle(cv: &mut Canvas, look: &Look, cx_right: f32, cy: f32, on: bool, hot: bool, en: f32, s: f32) -> RectF {
    let (w, h, inset) = match look.dir {
        Dir::Palco => (42.0, 24.0, 3.0),
        Dir::Estudio => (36.0, 20.0, 3.0),
        Dir::Ilha => (46.0, 26.0, 3.0),
    };
    let r = RectF::new(cx_right - w * s, cy - h * s / 2.0, w * s, h * s);
    let rad = look.r(6.0, r.h, s);
    let ea = en.max(0.45);
    if on {
        if look.dir == Dir::Palco && en > 0.5 {
            cv.glow(r, rad, 10.0 * s, 2.0 * s, look.accent, 0.40);
        }
        match look.dir {
            Dir::Ilha => cv.fill_round(r, rad, look.accent, ea),
            _ => look.fill_accent(cv, r, rad, ea),
        }
    } else {
        match look.dir {
            Dir::Estudio => cv.stroke_round(r, rad, s, WHITE, 0.32 * ea),
            Dir::Palco => cv.fill_round(r, rad, WHITE, 0.15 * ea),
            Dir::Ilha => cv.fill_round(r, rad, WHITE, 0.22 * ea),
        }
    }
    let kh = r.h - 2.0 * inset * s;
    let grow = if hot { 1.0 * s } else { 0.0 };
    let kx = if on { r.right() - inset * s - kh } else { r.x + inset * s };
    let k = RectF::new(kx - grow / 2.0, r.y + inset * s - grow / 2.0, kh + grow, kh + grow);
    let krad = look.r(4.0, k.h, s);
    if look.dir != Dir::Estudio {
        cv.drop_shadow(k, krad, 4.0 * s, 1.0 * s, 0.35 * en);
    }
    let ka = if on { 1.0 } else if look.dir == Dir::Estudio { 0.45 } else { 0.88 };
    cv.fill_round(k, krad, WHITE, ka * ea);
    r
}

fn segmented(cv: &mut Canvas, tr: &mut TextRenderer, look: &Look, right: f32, cy: f32, opts: &[&str], sel: usize, hover: Option<usize>, en: f32, s: f32) -> RectF {
    let pad = match look.dir {
        Dir::Estudio => 16.0,
        _ => 18.0,
    } * s;
    let widths: Vec<f32> = opts.iter().map(|o| (text_w(tr, o, Font::BodyStrong, s) + 2.0 * pad).max(72.0 * s).round()).collect();
    let inset = if look.dir == Dir::Estudio { 0.0 } else { 3.0 * s };
    let h = if look.dir == Dir::Estudio { 30.0 * s } else { 34.0 * s };
    let total = widths.iter().sum::<f32>() + 2.0 * inset;
    let r = RectF::new((right - total).round(), (cy - h / 2.0).round(), total, h);
    let mut cells = Vec::new();
    let mut x = r.x + inset;
    for w in &widths {
        cells.push(RectF::new(x, r.y + inset, *w, r.h - 2.0 * inset));
        x += w;
    }
    match look.dir {
        Dir::Palco => {
            cv.fill_round(r, look.r(7.0, r.h, s), WHITE, 0.06 * en.max(0.5));
            if let Some(hv) = hover.filter(|h| *h != sel) {
                cv.fill_round(cells[hv], look.r(5.0, cells[hv].h, s), WHITE, 0.06);
            }
            let p = cells[sel];
            cv.drop_shadow(p, look.r(5.0, p.h, s), 6.0 * s, 1.5 * s, 0.35 * en);
            cv.fill_round(p, look.r(5.0, p.h, s), WHITE, 0.96 * en.max(0.5));
            for (k, c) in cells.iter().enumerate() {
                let on = k == sel;
                ui::text_center(cv, tr, opts[k], if on { Font::BodyStrong } else { Font::Body }, c.offset(0.0, -0.5 * s), if on { INK } else { WHITE }, if on { 1.0 } else { 0.78 } * en);
            }
        }
        Dir::Estudio => {
            for (k, c) in cells.iter().enumerate() {
                if k == sel {
                    look.fill_accent(cv, *c, if k == 0 || k + 1 == cells.len() { look.r(5.0, c.h, s) } else { 0.0 }, en.max(0.45));
                } else if hover == Some(k) {
                    cv.fill_round(*c, 0.0, WHITE, 0.06);
                }
            }
            cv.stroke_round(r, look.r(5.0, r.h, s), s, WHITE, 0.16 * en.max(0.5));
            for c in cells.iter().skip(1) {
                cv.fill_rrect(c.x, r.y + s, c.x + s, r.bottom() - s, 0.0, 0.16 * en.max(0.5), WHITE);
            }
            for (k, c) in cells.iter().enumerate() {
                let on = k == sel;
                ui::text_center(cv, tr, opts[k], if on { Font::BodyStrong } else { Font::Body }, c.offset(0.0, -0.5 * s), WHITE, if on { 1.0 } else { 0.66 } * en);
            }
        }
        Dir::Ilha => {
            cv.fill_round(r, look.r(7.0, r.h, s), WHITE, 0.10 * en.max(0.5));
            let p = cells[sel];
            cv.drop_shadow(p, look.r(5.0, p.h, s), 6.0 * s, 1.5 * s, 0.30 * en);
            cv.fill_round(p, look.r(5.0, p.h, s), WHITE, 0.26 * en.max(0.5));
            cv.stroke_round(p, look.r(5.0, p.h, s), s, WHITE, 0.18 * en);
            for (k, c) in cells.iter().enumerate() {
                let on = k == sel;
                ui::text_center(cv, tr, opts[k], if on { Font::BodyStrong } else { Font::Body }, c.offset(0.0, -0.5 * s), WHITE, if on { 1.0 } else { 0.75 } * en);
            }
        }
    }
    r
}

fn list(cv: &mut Canvas, tr: &mut TextRenderer, look: &Look, right: f32, cy: f32, value: &str, hot: bool, en: f32, s: f32) -> RectF {
    let w = (text_w(tr, value, Font::Body, s) + 64.0 * s).max(200.0 * s).round();
    match look.dir {
        Dir::Palco => {
            let r = RectF::new(right - w, cy - 17.0 * s, w, 34.0 * s);
            cv.fill_round(r, look.r(6.0, r.h, s), WHITE, if hot { 0.11 } else { 0.07 } * en.max(0.5));
            cv.stroke_round(r, look.r(6.0, r.h, s), s, WHITE, 0.06);
            ui::text(cv, tr, value, Font::Body, r.x + 16.0 * s, r.y, r.w - 44.0 * s, r.h, WHITE, 0.95 * en);
            ui::glyph(cv, tr, RectF::new(r.right() - 34.0 * s, r.y, 22.0 * s, r.h), "\u{E70D}", Font::IconSmall, WHITE, 0.7 * en);
            r
        }
        Dir::Estudio => {
            let r = RectF::new(right - w, cy - 15.0 * s, w, 30.0 * s);
            cv.fill_round(r, look.r(6.0, r.h, s), WHITE, if hot { 0.07 } else { 0.03 });
            cv.stroke_round(r, look.r(6.0, r.h, s), s, if hot { look.accent } else { WHITE }, if hot { 0.8 } else { 0.16 });
            ui::text(cv, tr, value, Font::Body, r.x + 12.0 * s, r.y, r.w - 40.0 * s, r.h, WHITE, 0.95 * en);
            let a = RectF::new(r.right() - 28.0 * s, r.y, 28.0 * s, r.h);
            cv.fill_rrect(a.x, r.y + 6.0 * s, a.x + s, r.bottom() - 6.0 * s, 0.0, 0.16, WHITE);
            ui::glyph(cv, tr, a, "\u{E70D}", Font::IconSmall, WHITE, 0.7 * en);
            r
        }
        Dir::Ilha => {
            let r = RectF::new(right - w, cy - 16.0 * s, w, 32.0 * s);
            cv.drop_shadow(r, look.r(6.0, r.h, s), 4.0 * s, 1.0 * s, 0.18);
            cv.fill_round(r, look.r(6.0, r.h, s), WHITE, if hot { 0.20 } else { 0.14 } * en.max(0.5));
            cv.stroke_round(r, look.r(6.0, r.h, s), s, WHITE, 0.14);
            ui::text(cv, tr, value, Font::Body, r.x + 14.0 * s, r.y, r.w - 44.0 * s, r.h, WHITE, en);
            let a = RectF::new(r.right() - 28.0 * s, r.y + 5.0 * s, 20.0 * s, r.h - 10.0 * s);
            cv.fill_round(a, look.r(4.0, a.h, s), look.accent, en.max(0.5));
            ui::glyph(cv, tr, a.offset(0.0, -3.0 * s), "\u{E70E}", Font::IconSmall, WHITE, 0.95);
            ui::glyph(cv, tr, a.offset(0.0, 3.0 * s), "\u{E70D}", Font::IconSmall, WHITE, 0.95);
            r
        }
    }
}

fn slider(cv: &mut Canvas, tr: &mut TextRenderer, look: &Look, right: f32, cy: f32, t: f32, label: &str, hot: bool, en: f32, s: f32) -> RectF {
    let w = 220.0 * s;
    let r = RectF::new(right - w, cy - 12.0 * s, w, 24.0 * s);
    let (x0, x1) = (r.x + 8.0 * s, r.right() - 8.0 * s);
    let xv = x0 + (x1 - x0) * t;
    let ea = en.max(0.45);
    match look.dir {
        Dir::Palco => {
            let th = if hot { 6.0 } else { 4.0 } * s;
            let tr_r = look.r(1.5, th, s);
            cv.fill_rrect(x0, cy - th / 2.0, x1, cy + th / 2.0, tr_r, 0.16 * ea, WHITE);
            let bar = RectF::new(x0, cy - th / 2.0, (xv - x0).max(th), th);
            if en > 0.5 {
                cv.glow(bar, tr_r, 7.0 * s, 0.0, look.accent, 0.35);
            }
            look.fill_accent(cv, bar, tr_r, ea);
            let k = if look.round == Round::Mais { RectF::new(xv - 8.5 * s, cy - 8.5 * s, 17.0 * s, 17.0 * s) } else { RectF::new(xv - 6.0 * s, cy - 10.0 * s, 12.0 * s, 20.0 * s) };
            let kr = look.r(3.5, k.w, s);
            cv.drop_shadow(k, kr, 5.0 * s, 1.0 * s, 0.4 * en);
            cv.fill_round(k, kr, WHITE, ea);
            // Valor numa etiqueta à esquerda.
            let tw = text_w(tr, label, Font::CaptionStrong, s);
            let p = RectF::new(r.x - tw - 30.0 * s, cy - 12.0 * s, tw + 18.0 * s, 24.0 * s);
            cv.fill_round(p, look.r(4.0, p.h, s), WHITE, 0.07 * ea);
            ui::text_center(cv, tr, label, Font::CaptionStrong, p, WHITE, 0.9 * en.max(0.4));
        }
        Dir::Estudio => {
            for k in 0..=10 {
                let x = x0 + (x1 - x0) * k as f32 / 10.0;
                let big = k % 5 == 0;
                let th = if big { 5.0 } else { 3.0 } * s;
                cv.fill_rrect(x - 0.5 * s, cy + 6.0 * s, x + 0.5 * s, cy + 6.0 * s + th, 0.0, 0.22 * ea, WHITE);
            }
            cv.fill_rrect(x0, cy - s, x1, cy + s, 0.0, 0.22 * ea, WHITE);
            let bar = RectF::new(x0, cy - s, (xv - x0).max(2.0 * s), 2.0 * s);
            look.fill_accent(cv, bar, 0.0, ea);
            let k = RectF::new(xv - 4.5 * s, cy - 10.0 * s, 9.0 * s, 20.0 * s);
            cv.drop_shadow(k, 2.0 * s, 4.0 * s, 1.0 * s, 0.4 * en);
            cv.fill_round(k, 2.0 * s, if en > 0.5 { WHITE } else { [0.5; 3] }, 1.0);
            cv.fill_rrect(k.x + 2.0 * s, cy - 0.5 * s, k.right() - 2.0 * s, cy + 0.5 * s, 0.0, 0.8, INK);
            let b = RectF::new(r.x - 62.0 * s, cy - 12.0 * s, 50.0 * s, 24.0 * s);
            cv.stroke_round(b, look.r(5.0, b.h, s), s, WHITE, 0.16 * ea);
            ui::text_center(cv, tr, label, Font::Caption, b, WHITE, 0.9 * en.max(0.4));
        }
        Dir::Ilha => {
            let th = 6.0 * s;
            let tr_r = look.r(2.0, th, s);
            cv.fill_rrect(x0, cy - th / 2.0, x1, cy + th / 2.0, tr_r, 0.20 * ea, WHITE);
            cv.fill_rrect(x0, cy - th / 2.0, xv.max(x0 + th), cy + th / 2.0, tr_r, ea, look.accent);
            let k = if look.round == Round::Mais { RectF::new(xv - 10.5 * s, cy - 10.5 * s, 21.0 * s, 21.0 * s) } else { RectF::new(xv - 8.0 * s, cy - 11.0 * s, 16.0 * s, 22.0 * s) };
            let kr = look.r(4.5, k.w, s);
            cv.drop_shadow(k, kr, 6.0 * s, 1.5 * s, 0.45 * en);
            cv.fill_round(k, kr, WHITE, ea);
            ui::text_right(cv, tr, label, Font::Body, r.x - 12.0 * s, cy - 11.0 * s, 22.0 * s, WHITE, 0.7 * en.max(0.4), s);
        }
    }
    r
}

fn button(cv: &mut Canvas, tr: &mut TextRenderer, look: &Look, right: f32, cy: f32, label: &str, primary: bool, en: f32, s: f32) -> RectF {
    let w = (text_w(tr, label, Font::BodyStrong, s) + 40.0 * s).round();
    let ea = en.max(0.4);
    match look.dir {
        Dir::Palco => {
            let r = RectF::new(right - w, cy - 17.0 * s, w, 34.0 * s);
            if primary {
                cv.drop_shadow(r, look.r(6.0, r.h, s), 8.0 * s, 2.0 * s, 0.35 * en);
                cv.fill_round(r, look.r(6.0, r.h, s), WHITE, 0.96 * ea);
                ui::text_center(cv, tr, label, Font::BodyStrong, r.offset(0.0, -0.5 * s), INK, ea);
            } else {
                cv.fill_round(r, look.r(6.0, r.h, s), WHITE, 0.09 * ea);
                cv.stroke_round(r, look.r(6.0, r.h, s), s, WHITE, 0.08);
                ui::text_center(cv, tr, label, Font::BodyStrong, r.offset(0.0, -0.5 * s), WHITE, 0.95 * ea);
            }
            r
        }
        Dir::Estudio => {
            let r = RectF::new(right - w, cy - 15.0 * s, w, 30.0 * s);
            if primary {
                look.fill_accent(cv, r, look.r(6.0, r.h, s), ea);
            } else {
                cv.stroke_round(r, look.r(6.0, r.h, s), s, WHITE, 0.3 * ea);
            }
            ui::text_center(cv, tr, label, Font::BodyStrong, r.offset(0.0, -0.5 * s), WHITE, ea);
            r
        }
        Dir::Ilha => {
            let r = RectF::new(right - w, cy - 16.0 * s, w, 32.0 * s);
            let rr = look.r(6.0, r.h, s);
            cv.drop_shadow(r, rr, 5.0 * s, 1.0 * s, 0.2 * en);
            if primary {
                cv.fill_round(r, rr, look.accent, ea);
            } else {
                cv.fill_round(r, rr, WHITE, 0.16 * ea);
                cv.stroke_round(r, rr, s, WHITE, 0.14);
            }
            ui::text_center(cv, tr, label, Font::BodyStrong, r.offset(0.0, -0.5 * s), WHITE, ea);
            r
        }
    }
}

fn control(cv: &mut Canvas, tr: &mut TextRenderer, look: &Look, right: f32, cy: f32, c: Ctl, hot: bool, en: f32, s: f32) {
    match c {
        Ctl::Toggle(on) => {
            toggle(cv, look, right, cy, on && en > 0.5, hot, en, s);
        }
        Ctl::Seg(o, k) => {
            segmented(cv, tr, look, right, cy, o, k, None, en, s);
        }
        Ctl::List(v) => {
            list(cv, tr, look, right, cy, v, hot, en, s);
        }
        Ctl::Slider(t, l) => {
            slider(cv, tr, look, right, cy, t, l, hot, en, s);
        }
        Ctl::Button(l, p) => {
            button(cv, tr, look, right, cy, l, p, en, s);
        }
    }
}

// ---------------------------------------------------------------- coluna de opções

/// Título da página e as abas internas.
fn page_head(cv: &mut Canvas, tr: &mut TextRenderer, look: &Look, x: f32, y: f32, w: f32, s: f32, title: &str, subtitle: &str, tabs: &[&str], sel: usize) -> f32 {
    let tfont = if look.dir == Dir::Ilha { Font::Hero } else { Font::Heading };
    ui::text(cv, tr, title, tfont, x, y, w, 40.0 * s, WHITE, 1.0);
    ui::text(cv, tr, subtitle, Font::Body, x + s, y + 38.0 * s, w, 22.0 * s, WHITE, 0.55);
    let ty = y + 78.0 * s;
    if tabs.is_empty() {
        return ty;
    }
    match look.dir {
        Dir::Palco => {
            let mut tx = x;
            for (k, t) in tabs.iter().enumerate() {
                let on = k == sel;
                let font = if on { Font::BodyStrong } else { Font::Body };
                let tw = (text_w(tr, t, font, s) + 32.0 * s).round();
                let r = RectF::new(tx, ty, tw, 34.0 * s);
                if on {
                    cv.drop_shadow(r, look.r(6.0, r.h, s), 8.0 * s, 2.0 * s, 0.35);
                    cv.fill_round(r, look.r(6.0, r.h, s), WHITE, 0.96);
                } else {
                    cv.fill_round(r, look.r(6.0, r.h, s), WHITE, 0.045);
                }
                ui::text_center(cv, tr, t, font, r.offset(0.0, -0.5 * s), if on { INK } else { WHITE }, if on { 1.0 } else { 0.72 });
                tx += tw + 8.0 * s;
            }
            ty + 34.0 * s + 22.0 * s
        }
        Dir::Estudio => {
            let mut tx = x;
            for (k, t) in tabs.iter().enumerate() {
                let on = k == sel;
                let tw = text_w(tr, t, Font::BodyStrong, s).round();
                let r = RectF::new(tx, ty, tw, 30.0 * s);
                ui::text(cv, tr, t, if on { Font::BodyStrong } else { Font::Body }, r.x, r.y, tw + 40.0 * s, r.h, WHITE, if on { 1.0 } else { 0.55 });
                if on {
                    let u = RectF::new(r.x, r.bottom() + 4.0 * s, tw, 2.0 * s);
                    look.fill_accent(cv, u, 0.0, 1.0);
                }
                tx += tw + 28.0 * s;
            }
            cv.fill_rrect(x, ty + 36.0 * s, x + w, ty + 37.0 * s, 0.0, 0.08, WHITE);
            ty + 37.0 * s + 10.0 * s
        }
        Dir::Ilha => {
            let r = segmented(cv, tr, look, x + 10_000.0, ty + 17.0 * s, tabs, sel, None, 1.0, s);
            // Desenhado de novo no lugar certo (à esquerda).
            let _ = r;
            ty + 34.0 * s + 22.0 * s
        }
    }
}

/// As linhas num grupo; devolve o fim.
fn rows_block(cv: &mut Canvas, tr: &mut TextRenderer, look: &Look, x: f32, y: f32, w: f32, s: f32, rows: &[RowM], mark: Option<&str>) -> f32 {
    let rh = match look.dir {
        Dir::Estudio => 52.0,
        Dir::Palco => 56.0,
        Dir::Ilha => 54.0,
    } * s;
    let h = rh * rows.len() as f32;
    let panel = RectF::new(x, y, w, h);
    let radius = match look.dir {
        Dir::Palco => look.rp(8.0, s),
        Dir::Estudio => 0.0,
        Dir::Ilha => look.rp(10.0, s),
    };
    match look.dir {
        Dir::Palco => {
            cv.fill_round(panel, radius, WHITE, 0.042);
            let y0 = panel.y;
            cv.fill_rrect_ca(panel.x, panel.y, panel.right(), panel.bottom(), radius, |_, yy| (WHITE, if yy < y0 + 1.5 * s { 0.07 } else { 0.0 }));
            cv.stroke_round(panel, radius, s, WHITE, 0.065);
        }
        Dir::Ilha => frost(cv, panel, radius, s),
        Dir::Estudio => {
            cv.fill_rrect(x, y, x + w, y + s, 0.0, 0.08, WHITE);
        }
    }
    for (k, r) in rows.iter().enumerate() {
        let ry = y + k as f32 * rh;
        let rr = RectF::new(x, ry, w, rh);
        let en = if r.enabled { 1.0 } else { 0.4 };
        if r.hover {
            let old = cv.clip_rect();
            cv.set_clip(old.intersect(rr));
            match look.dir {
                Dir::Estudio => cv.fill_round(rr, 0.0, WHITE, 0.035),
                _ => cv.fill_round(panel, radius, WHITE, 0.04),
            }
            cv.set_clip(old);
        }
        if k > 0 {
            let inset = if look.dir == Dir::Estudio { 0.0 } else { 20.0 * s };
            cv.fill_rrect(x + inset, ry, x + w - inset, ry + s, 0.0, if look.dir == Dir::Estudio { 0.07 } else { 0.06 }, WHITE);
        }
        if look.dir == Dir::Estudio {
            cv.fill_rrect(x, ry + rh - s, x + w, ry + rh, 0.0, 0.07, WHITE);
        }
        let pad = if look.dir == Dir::Estudio { 4.0 } else { 20.0 } * s;
        // Busca: o trecho que bateu ganha um fundo na cor de destaque.
        if let Some(q) = mark
            && let Some(at) = r.title.to_lowercase().find(&q.to_lowercase())
        {
            let pre = &r.title[..at];
            let gap = if pre.ends_with(' ') { 3.5 * s } else { 0.0 };
            let x0 = x + pad + if pre.is_empty() { 0.0 } else { text_w(tr, pre, Font::Body, s) + gap };
            let mw = text_w(tr, &r.title[at..at + q.len()], Font::Body, s);
            let hr = RectF::new(x0 - 3.0 * s, ry + rh / 2.0 - 12.0 * s, mw + 6.0 * s, 24.0 * s);
            cv.fill_round(hr, look.r(3.0, hr.h, s), look.accent, 0.30);
        }
        ui::text(cv, tr, r.title, Font::Body, x + pad, ry, w * 0.5, rh, WHITE, 0.94 * en);
        if !r.enabled {
            let tw = text_w(tr, r.title, Font::Body, s);
            ui::glyph(cv, tr, RectF::new(x + pad + tw + 6.0 * s, ry, 20.0 * s, rh), "\u{E946}", Font::IconSmall, WHITE, 0.45);
        }
        if r.hover && look.dir == Dir::Estudio {
            cv.fill_rrect(x, ry + 8.0 * s, x + 2.0 * s, ry + rh - 8.0 * s, 0.0, 1.0, look.accent);
        }
        control(cv, tr, look, x + w - pad, ry + rh / 2.0, r.ctl, r.hover, en, s);
    }
    y + h
}

/// "Ajuste fino" recolhido (as opções raras ficam guardadas aqui).
fn expander(cv: &mut Canvas, tr: &mut TextRenderer, look: &Look, x: f32, y: f32, w: f32, s: f32, summary: &str) -> f32 {
    let h = 52.0 * s;
    let r = RectF::new(x, y, w, h);
    match look.dir {
        Dir::Palco => {
            cv.fill_round(r, look.rp(8.0, s), WHITE, 0.028);
            cv.stroke_round(r, look.rp(8.0, s), s, WHITE, 0.06);
        }
        Dir::Ilha => frost(cv, r, look.rp(10.0, s), s),
        Dir::Estudio => cv.fill_rrect(x, y + h - s, x + w, y + h, 0.0, 0.07, WHITE),
    }
    let pad = if look.dir == Dir::Estudio { 4.0 } else { 20.0 } * s;
    let label = if look.dir == Dir::Estudio { "AJUSTE FINO" } else { "Ajuste fino" };
    let font = if look.dir == Dir::Estudio { Font::Label } else { Font::BodyStrong };
    ui::text(cv, tr, label, font, x + pad, y, 200.0 * s, h, WHITE, 0.85);
    let lw = text_w(tr, label, font, s);
    ui::text(cv, tr, summary, Font::Caption, x + pad + lw + 12.0 * s, y, w - 200.0 * s, h, WHITE, 0.45);
    ui::glyph(cv, tr, RectF::new(x + w - pad - 24.0 * s, y, 24.0 * s, h), "\u{E70D}", Font::IconSmall, WHITE, 0.7);
    y + h
}

// ---------------------------------------------------------------- palco (prévia)

#[derive(Clone, Copy, PartialEq)]
enum Scene {
    /// Widget na barra (páginas da barra).
    Widget,
    /// Card aberto sobre o widget (Início, cores).
    Overview,
}

fn wallpaper(cv: &mut Canvas, r: RectF, radius: f32, look: &Look, _s: f32) {
    let (x0, y0) = (r.x, r.y);
    let muted = look.dir == Dir::Estudio;
    // Degradê com manchas de luz bem suaves (soma de gaussianas, sem degraus),
    // como os papéis de parede do Windows 11.
    let blobs: [(f32, f32, f32, f32, Rgb, f32); 4] = [
        (0.82, 0.22, 0.34, 0.30, [0.55, 0.30, 0.95], 0.60),
        (0.22, 0.58, 0.30, 0.26, [0.95, 0.40, 0.62], 0.45),
        (0.86, 0.70, 0.22, 0.20, [0.25, 0.55, 1.0], 0.45),
        (0.42, 0.12, 0.20, 0.14, [1.0, 0.70, 0.40], 0.22),
    ];
    let k = if muted { 0.30 } else { 1.0 };
    cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), radius, |x, y| {
        let u = ((x - x0) / r.w).clamp(0.0, 1.0);
        let v = ((y - y0) / r.h).clamp(0.0, 1.0);
        let (a, b) = if muted { ([0.08, 0.085, 0.11], [0.13, 0.11, 0.16]) } else { ([0.035, 0.05, 0.16], [0.14, 0.07, 0.27]) };
        let mut c = lerp3(a, b, (u * 0.55 + v * 0.45).clamp(0.0, 1.0));
        for (bu, bv, rx, ry, col, al) in blobs {
            let d = ((u - bu) / rx).powi(2) + ((v - bv) / ry).powi(2);
            c = lerp3(c, col, (-d).exp() * al * k);
        }
        // Pontilhado de ±1,5/255: sem ele o degradê escuro vira degraus.
        let mut hsh = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
        hsh ^= hsh >> 15;
        hsh = hsh.wrapping_mul(0x2C1B_3C6D);
        hsh ^= hsh >> 12;
        let n = ((hsh & 0xffff) as f32 / 65535.0 - 0.5) * (3.0 / 255.0);
        ([c[0] + n, c[1] + n, c[2] + n], 1.0)
    });
}

#[allow(clippy::too_many_arguments)]
fn stage(cv: &mut Canvas, tr: &mut TextRenderer, ptr: &mut TextRenderer, look: &Look, r: RectF, s: f32, scene: Scene, caption: Option<(&str, &str)>, cfg: &Config, media: &MediaInfo) {
    let (radius, inner) = match look.dir {
        Dir::Palco => (look.rp(9.0, s), r),
        Dir::Estudio => (look.rp(6.0, s), r),
        Dir::Ilha => {
            frost(cv, r, look.rp(11.0, s), s);
            (look.rp(7.0, s), r.inset(8.0 * s))
        }
    };
    if look.dir == Dir::Palco {
        cv.drop_shadow(r, radius, 30.0 * s, 10.0 * s, 0.45);
    }
    wallpaper(cv, inner, radius, look, s);

    // Barra de tarefas e o widget, ampliados (a prévia é grande).
    let ps = if scene == Scene::Widget { 1.7 * s } else { 1.25 * s };
    ptr.set_scale(ps);
    let bar_h = 48.0 * ps;
    let bar = RectF::new(inner.x, inner.bottom() - bar_h, inner.w, bar_h);
    let bc: Rgb = [0.11, 0.11, 0.12];
    cv.fill_rrect_ca(bar.x, bar.y, bar.right(), bar.bottom(), radius, |_, y| (bc, if y < bar.y + radius { 0.0 } else { 0.95 }));
    cv.fill_rrect(bar.x, bar.y, bar.right(), bar.y + radius, 0.0, 0.95, bc);
    cv.fill_rrect(bar.x, bar.y, bar.right(), bar.y + s, 0.0, 0.10, WHITE);
    let clock = bar.right() - 70.0 * ps;
    ui::text(&mut *cv, ptr, "12:34", Font::Caption, clock, bar.y + 6.0 * ps, 60.0 * ps, 18.0 * ps, [0.92; 3], 1.0);
    ui::text(&mut *cv, ptr, "26/11", Font::Caption, clock, bar.y + 24.0 * ps, 60.0 * ps, 18.0 * ps, [0.92; 3], 0.75);
    for k in 0..4 {
        let ix = bar.x + 20.0 * ps + k as f32 * 40.0 * ps;
        cv.fill_round(RectF::new(ix, bar.cy() - 11.0 * ps, 22.0 * ps, 22.0 * ps), 5.0 * ps, [0.85; 3], if k == 0 { 0.9 } else { 0.3 });
    }
    let c = colors(cfg, media);
    let pb = playback(media);
    let wl = WLayout::build_for(ps, 40.0 * ps, cfg, true, false, 0, ui::bar_toggles(cfg, media));
    let widget = render(wl.width, 40.0 * ps, |wcv| {
        let f = WidgetFrame { cfg, media, theme: ui::widget_theme(cfg, &c, media), colors: &c, it: Interact::default(), pb, time: 1.0, viz: None, flyout_open: scene == Scene::Overview, s: ps, change: None, launchers: &[] };
        ui::draw_widget(wcv, ptr, &wl, &f);
    });
    let wx = (clock - 16.0 * ps - wl.width).round();
    blit(cv, wx, bar.y + 4.0 * ps, &widget);
    if scene == Scene::Overview {
        let cl = CLayout::build(ps, cfg, true);
        let card = render(cl.w, cl.h, |ccv| {
            const VIZ: &[f32] = &[0.55, 0.9, 0.62, 0.38, 0.74, 0.5];
            let f = CardFrame { cfg, colors: &c, media, theme: ui::card_theme(cfg, &c, media), it: Interact::default(), pb, s: ps, backdrop: None, hint: None, launchers: &[], viz: Some(VIZ), mixer: None };
            ui::draw_card(ccv, ptr, &cl, &f);
        });
        let m = ui::SHADOW_MARGIN * ps;
        let cx = (wx + wl.art.map(|a| a.x).unwrap_or(0.0) - m - 6.0 * ps).min(inner.right() - cl.w + m - 10.0 * ps);
        blit(cv, cx, bar.y - cl.h + m - 10.0 * ps, &card);
    }

    // Rótulo e botões da prévia.
    let label = match look.dir {
        Dir::Estudio => "PRÉVIA · 1,7×",
        _ => "PRÉVIA",
    };
    ui::text(cv, tr, label, Font::Label, inner.x + 18.0 * s, inner.y + 14.0 * s, 200.0 * s, 16.0 * s, WHITE, 0.7);
    let groups: [&[&str]; 2] = [&["Tocando", "Pausado", "Sem música"], &["Escuro", "Claro"]];
    let mut x = inner.right() - 14.0 * s;
    for (gi, g) in groups.iter().enumerate().rev() {
        let widths: Vec<f32> = g.iter().map(|t| (text_w(tr, t, Font::Caption, s) + 22.0 * s).round()).collect();
        let total: f32 = widths.iter().sum::<f32>() + 4.0 * s;
        let gr = RectF::new(x - total, inner.y + 10.0 * s, total, 28.0 * s);
        let rad = look.r(6.0, gr.h, s);
        cv.fill_round(gr, rad, [0.0; 3], 0.34);
        cv.stroke_round(gr, rad, s, WHITE, 0.10);
        let mut cx = gr.x + 2.0 * s;
        for (k, t) in g.iter().enumerate() {
            let cr = RectF::new(cx, gr.y + 2.0 * s, widths[k], gr.h - 4.0 * s);
            let on = k == 0 && gi < 2;
            if on {
                match look.dir {
                    Dir::Estudio => look.fill_accent(cv, cr, look.r(4.0, cr.h, s), 1.0),
                    _ => cv.fill_round(cr, look.r(4.0, cr.h, s), WHITE, 0.92),
                }
            }
            let tc = if on && look.dir != Dir::Estudio { INK } else { WHITE };
            ui::text_center(cv, tr, t, Font::Caption, cr.offset(0.0, -0.5 * s), tc, if on { 1.0 } else { 0.8 });
            cx += widths[k];
        }
        x = gr.x - 8.0 * s;
    }

    // Legenda da opção sob o mouse: o que ela faz (as linhas só têm o título).
    if let Some((t, d)) = caption {
        let cw = (inner.w - 36.0 * s).min(720.0 * s);
        let ch = 64.0 * s;
        let cr = RectF::new(inner.x + 18.0 * s, inner.y + 48.0 * s, cw, ch);
        match look.dir {
            Dir::Estudio => {
                cv.fill_round(cr, 4.0 * s, [0.04; 3], 0.86);
                cv.fill_rrect(cr.x, cr.y, cr.x + 3.0 * s, cr.bottom(), 0.0, 1.0, look.accent);
            }
            _ => {
                cv.drop_shadow(cr, look.rp(7.0, s), 16.0 * s, 4.0 * s, 0.35);
                cv.fill_round(cr, look.rp(7.0, s), [0.06, 0.06, 0.08], 0.80);
                cv.stroke_round(cr, look.rp(7.0, s), s, WHITE, 0.10);
            }
        }
        ui::glyph(cv, tr, RectF::new(cr.x + 12.0 * s, cr.y + 10.0 * s, 22.0 * s, 22.0 * s), "\u{E946}", Font::IconSmall, look.accent, 1.0);
        ui::text(cv, tr, t, Font::BodyStrong, cr.x + 40.0 * s, cr.y + 9.0 * s, cw - 52.0 * s, 24.0 * s, WHITE, 1.0);
        ui::text(cv, tr, d, Font::Caption, cr.x + 40.0 * s, cr.y + 32.0 * s, cw - 52.0 * s, 22.0 * s, WHITE, 0.75);
    }
}

// ---------------------------------------------------------------- telas inteiras

const W: f32 = 1680.0;
const H: f32 = 1000.0;
const NAV_W: f32 = 236.0;

/// Página "Barra de tarefas", aba "Fundo", com a "Cor do fundo" sob o mouse.
fn page_shot(look: &Look, tr: &mut TextRenderer, ptr: &mut TextRenderer, s: f32, cfg: &Config, media: &MediaInfo) -> (usize, usize, Vec<u32>) {
    let (w, h) = (W * s, H * s);
    render(w, h, |cv| {
        window_bg(cv, look, w, h, s, media);
        title_bar(cv, tr, look, w, s);
        nav(cv, tr, look, h, s, 1, None);
        let x = NAV_W * s + 32.0 * s;
        let split = (x + (w - x) * 0.47).round();
        let cw = split - x - 28.0 * s;
        let y = page_head(cv, tr, look, x, 52.0 * s, cw, s, "Barra de tarefas", "O widget que fica ao lado do relógio", TABS, 2);
        if look.dir == Dir::Ilha {
            // As abas da Ilha são um segmentado encostado à esquerda.
            let total: f32 = TABS.iter().map(|t| (text_w(tr, t, Font::BodyStrong, s) + 36.0 * s).max(72.0 * s).round()).sum::<f32>() + 6.0 * s;
            segmented(cv, tr, look, x + total, 52.0 * s + 78.0 * s + 17.0 * s, TABS, 2, None, 1.0, s);
        }
        let y = rows_block(cv, tr, look, x, y, cw, s, FUNDO, None);
        expander(cv, tr, look, x, y + 14.0 * s, cw, s, "Separador entre a música e os botões");
        let sr = RectF::new(split, 52.0 * s, w - split - 24.0 * s, h - 52.0 * s - 24.0 * s);
        let hov = FUNDO.iter().find(|r| r.hover).map(|r| (r.title, r.desc));
        stage(cv, tr, ptr, look, sr, s, Scene::Widget, hov, cfg, media);
    })
}

/// Folha dos controles: cada um nos estados que importam.
fn controls_sheet(look: &Look, tr: &mut TextRenderer, s: f32, media: &MediaInfo) -> (usize, usize, Vec<u32>) {
    let (w, h) = (820.0 * s, 640.0 * s);
    render(w, h, |cv| {
        window_bg(cv, look, w, h, s, media);
        let lx = 28.0 * s;
        let cx = 230.0 * s;
        let mut y = 34.0 * s;
        let name = match look.dir {
            Dir::Palco => "Palco",
            Dir::Estudio => "Estúdio",
            Dir::Ilha => "Ilha",
        };
        ui::text(cv, tr, &format!("Controles — {name}"), Font::Display, lx, y - 12.0 * s, 600.0 * s, 28.0 * s, WHITE, 1.0);
        y += 44.0 * s;
        let label = |cv: &mut Canvas, tr: &mut TextRenderer, t: &str, y: f32| ui::text(cv, tr, t, Font::CaptionStrong, lx, y - 12.0 * s, 190.0 * s, 24.0 * s, WHITE, 0.55);
        // Interruptor: desligado, ligado, mouse em cima, desativado.
        label(cv, tr, "Interruptor", y);
        let mut x = cx + 50.0 * s;
        for (on, hot, en) in [(false, false, 1.0), (true, false, 1.0), (true, true, 1.0), (false, false, 0.4)] {
            toggle(cv, look, x, y, on, hot, en, s);
            x += 90.0 * s;
        }
        y += 64.0 * s;
        label(cv, tr, "Segmentado", y);
        segmented(cv, tr, look, cx + 400.0 * s, y, &["Pequeno", "Normal", "Grande"], 1, Some(2), 1.0, s);
        y += 64.0 * s;
        label(cv, tr, "Lista", y);
        list(cv, tr, look, cx + 230.0 * s, y, "Capa desfocada", false, 1.0, s);
        list(cv, tr, look, cx + 480.0 * s, y, "Cor da capa", true, 1.0, s);
        y += 64.0 * s;
        label(cv, tr, "Slider", y);
        slider(cv, tr, look, cx + 300.0 * s, y, 0.3, "30%", false, 1.0, s);
        y += 52.0 * s;
        slider(cv, tr, look, cx + 300.0 * s, y, 0.75, "75%", true, 1.0, s);
        slider(cv, tr, look, cx + 640.0 * s, y, 0.5, "50%", false, 0.4, s);
        y += 64.0 * s;
        label(cv, tr, "Botões", y);
        let r1 = button(cv, tr, look, cx + 150.0 * s, y, "Mostrar agora", true, 1.0, s);
        let r2 = button(cv, tr, look, r1.right() + 140.0 * s, y, "Restaurar", false, 1.0, s);
        button(cv, tr, look, r2.right() + 150.0 * s, y, "Limpar letras", false, 0.4, s);
        y += 72.0 * s;
        label(cv, tr, "Texto", y);
        let tfont = if look.dir == Dir::Ilha { Font::Hero } else { Font::Heading };
        ui::text(cv, tr, "Título da página", tfont, cx, y - 20.0 * s, 400.0 * s, 40.0 * s, WHITE, 1.0);
        y += 40.0 * s;
        let (sec, sfont) = if look.dir == Dir::Estudio { ("GRUPO DE OPÇÕES", Font::Label) } else if look.dir == Dir::Palco { ("GRUPO DE OPÇÕES", Font::Label) } else { ("Grupo de opções", Font::BodyStrong) };
        ui::text(cv, tr, sec, sfont, cx, y - 10.0 * s, 400.0 * s, 20.0 * s, WHITE, 0.5);
        y += 30.0 * s;
        ui::text(cv, tr, "Nome da opção", Font::Body, cx, y - 11.0 * s, 300.0 * s, 22.0 * s, WHITE, 0.94);
        ui::text(cv, tr, "O que ela faz, na legenda da prévia", Font::Caption, cx + 160.0 * s, y - 10.0 * s, 400.0 * s, 20.0 * s, WHITE, 0.6);
        y += 30.0 * s;
        let accent_note = match look.dir {
            Dir::Estudio => "Destaque: o degradê da marca (rosa → laranja), sempre igual",
            _ => "Destaque: a cor da música que está tocando (muda com a capa)",
        };
        cv.fill_round(RectF::new(cx, y - 7.0 * s, 14.0 * s, 14.0 * s), 7.0 * s, look.accent, 1.0);
        ui::text(cv, tr, accent_note, Font::Caption, cx + 22.0 * s, y - 10.0 * s, 560.0 * s, 20.0 * s, WHITE, 0.75);
    })
}

/// Início (Palco): o que mais se usa num lugar só.
fn home_shot(look: &Look, tr: &mut TextRenderer, ptr: &mut TextRenderer, s: f32, cfg: &Config, media: &MediaInfo) -> (usize, usize, Vec<u32>) {
    let (w, h) = (W * s, H * s);
    render(w, h, |cv| {
        window_bg(cv, look, w, h, s, media);
        title_bar(cv, tr, look, w, s);
        nav(cv, tr, look, h, s, 0, None);
        let x = NAV_W * s + 32.0 * s;
        let split = (x + (w - x) * 0.47).round();
        let cw = split - x - 28.0 * s;
        let mut y = page_head(cv, tr, look, x, 52.0 * s, cw, s, "Início", "O que você mais usa, num lugar só", &[], 0);
        // Interruptores grandes, 3 por linha.
        ui::text(cv, tr, "LIGADO E DESLIGADO", Font::Label, x + 2.0 * s, y, 300.0 * s, 16.0 * s, WHITE, 0.5);
        y += 24.0 * s;
        let tiles: [(&str, &str, &str, bool); 6] = [
            ("\u{E7F4}", "Música na barra", "Widget ao lado do relógio", true),
            ("\u{E9E9}", "Barras de áudio", "Mini visualizador", false),
            ("\u{E7F0}", "Letra na tela", "Win+Alt+L", false),
            ("\u{E765}", "Aviso de teclas", "Caps Lock e Num Lock", true),
            ("\u{E768}", "A seguir", "Próxima música", true),
            ("\u{E7E8}", "Iniciar com o Windows", "Abre sozinho", true),
        ];
        let gap = 12.0 * s;
        let tw = ((cw - 2.0 * gap) / 3.0).floor();
        let th = 96.0 * s;
        for (k, (icon, name, sub, on)) in tiles.iter().enumerate() {
            let r = RectF::new(x + (k % 3) as f32 * (tw + gap), y + (k / 3) as f32 * (th + gap), tw, th);
            cv.fill_round(r, look.rp(8.0, s), WHITE, if *on { 0.065 } else { 0.035 });
            cv.stroke_round(r, look.rp(8.0, s), s, WHITE, 0.065);
            let chip = RectF::new(r.x + 14.0 * s, r.y + 14.0 * s, 30.0 * s, 30.0 * s);
            if *on {
                cv.glow(chip, look.r(5.0, chip.h, s), 10.0 * s, 2.0 * s, look.accent, 0.4);
                look.fill_accent(cv, chip, look.r(5.0, chip.h, s), 1.0);
            } else {
                cv.fill_round(chip, look.r(5.0, chip.h, s), WHITE, 0.08);
            }
            ui::glyph(cv, tr, chip, icon, Font::IconSmall, WHITE, if *on { 1.0 } else { 0.7 });
            toggle(cv, look, r.right() - 14.0 * s, chip.cy(), *on, false, 1.0, s);
            ui::text(cv, tr, name, Font::BodyStrong, r.x + 14.0 * s, r.y + 50.0 * s, tw - 24.0 * s, 22.0 * s, WHITE, if *on { 1.0 } else { 0.8 });
            ui::text(cv, tr, sub, Font::Caption, r.x + 14.0 * s, r.y + 70.0 * s, tw - 24.0 * s, 18.0 * s, WHITE, 0.5);
        }
        y += 2.0 * (th + gap) + 18.0 * s;
        // Visual rápido: fundo do card em miniaturas e a cor de destaque.
        ui::text(cv, tr, "VISUAL", Font::Label, x + 2.0 * s, y, 300.0 * s, 16.0 * s, WHITE, 0.5);
        y += 24.0 * s;
        let panel = RectF::new(x, y, cw, 214.0 * s);
        cv.fill_round(panel, look.rp(8.0, s), WHITE, 0.042);
        cv.stroke_round(panel, look.rp(8.0, s), s, WHITE, 0.065);
        ui::text(cv, tr, "Fundo do card", Font::Body, x + 20.0 * s, y + 14.0 * s, 300.0 * s, 22.0 * s, WHITE, 0.94);
        let names = ["Capa desfocada", "Cor do álbum", "Acrílico"];
        let styles = [CardStyle::Artwork, CardStyle::AlbumColor, CardStyle::Glass];
        let mw = ((cw - 40.0 * s - 2.0 * gap) / 3.0).floor();
        let mh = 92.0 * s;
        for k in 0..3 {
            let r = RectF::new(x + 20.0 * s + k as f32 * (mw + gap), y + 44.0 * s, mw, mh);
            // Miniatura do card no estilo (o próprio card, pequeno).
            let mcfg = Config { card_style: styles[k], corners: look.corners(), ..cfg.clone() };
            let ms = 0.62 * s;
            ptr.set_scale(ms);
            let cl = CLayout::build(ms, &mcfg, true);
            let c = colors(&mcfg, media);
            let card = render(cl.w, cl.h, |ccv| {
                let f = CardFrame { cfg: &mcfg, colors: &c, media, theme: ui::card_theme(&mcfg, &c, media), it: Interact::default(), pb: playback(media), s: ms, backdrop: None, hint: None, launchers: &[], viz: None, mixer: None };
                ui::draw_card(ccv, ptr, &cl, &f);
            });
            let saved = cv.clip_rect();
            cv.set_clip(saved.intersect(r));
            cv.fill_round(r, look.rp(5.0, s), [0.0; 3], 0.25);
            blit(cv, r.cx() - cl.w / 2.0, r.y + 6.0 * s - ui::SHADOW_MARGIN * ms, &card);
            cv.set_clip(saved);
            let sel = k == 0;
            cv.stroke_round(r.inset(-3.0 * s), look.rp(5.0, s) + 3.0 * s, if sel { 2.0 * s } else { s }, if sel { look.accent } else { WHITE }, if sel { 1.0 } else { 0.10 });
            ui::text_center(cv, tr, names[k], if sel { Font::CaptionStrong } else { Font::Caption }, RectF::new(r.x, r.bottom() + 4.0 * s, r.w, 20.0 * s), WHITE, if sel { 1.0 } else { 0.7 });
        }
        let ry = y + 44.0 * s + mh + 30.0 * s;
        cv.fill_rrect(x + 20.0 * s, ry, x + cw - 20.0 * s, ry + s, 0.0, 0.06, WHITE);
        ui::text(cv, tr, "Cor de destaque", Font::Body, x + 20.0 * s, ry, 300.0 * s, 52.0 * s, WHITE, 0.94);
        segmented(cv, tr, look, x + cw - 20.0 * s, ry + 26.0 * s, &["Cor da capa", "Cor do Windows", "Personalizada"], 0, None, 1.0, s);
        y = panel.bottom() + 22.0 * s;
        // Atalhos para as páginas.
        ui::text(cv, tr, "TODAS AS OPÇÕES", Font::Label, x + 2.0 * s, y, 300.0 * s, 16.0 * s, WHITE, 0.5);
        y += 24.0 * s;
        let links: [(&str, &str, &str); 4] = [("\u{E7F4}", "Barra de tarefas", "Conteúdo, botões, fundo, posição, barras"), ("\u{E7C4}", "Card", "Estilo, controles, animação, \"A seguir\""), ("\u{E7F0}", "Letras", "Janela, aparência, de onde vem a letra"), ("\u{E765}", "Caps Lock", "Avisos, aparência, onde aparece")];
        let lw = ((cw - gap) / 2.0).floor();
        for (k, (icon, name, sub)) in links.iter().enumerate() {
            let r = RectF::new(x + (k % 2) as f32 * (lw + gap), y + (k / 2) as f32 * (60.0 * s + gap), lw, 60.0 * s);
            cv.fill_round(r, look.rp(7.0, s), WHITE, 0.035);
            cv.stroke_round(r, look.rp(7.0, s), s, WHITE, 0.06);
            let chip = RectF::new(r.x + 14.0 * s, r.cy() - 14.0 * s, 28.0 * s, 28.0 * s);
            cv.fill_round(chip, look.r(5.0, chip.h, s), WHITE, 0.06);
            ui::glyph(cv, tr, chip, icon, Font::IconSmall, WHITE, 0.85);
            ui::text(cv, tr, name, Font::BodyStrong, chip.right() + 12.0 * s, r.y + 9.0 * s, lw - 90.0 * s, 22.0 * s, WHITE, 0.95);
            ui::text(cv, tr, sub, Font::Caption, chip.right() + 12.0 * s, r.y + 30.0 * s, lw - 90.0 * s, 18.0 * s, WHITE, 0.5);
            ui::glyph(cv, tr, RectF::new(r.right() - 34.0 * s, r.y, 22.0 * s, r.h), "\u{E76C}", Font::IconSmall, WHITE, 0.6);
        }
        let sr = RectF::new(split, 52.0 * s, w - split - 24.0 * s, h - 52.0 * s - 24.0 * s);
        stage(cv, tr, ptr, look, sr, s, Scene::Overview, None, cfg, media);
    })
}

/// Busca (Palco): resultados de várias páginas, já com os controles.
fn search_shot(look: &Look, tr: &mut TextRenderer, ptr: &mut TextRenderer, s: f32, cfg: &Config, media: &MediaInfo) -> (usize, usize, Vec<u32>) {
    let (w, h) = (W * s, H * s);
    render(w, h, |cv| {
        window_bg(cv, look, w, h, s, media);
        title_bar(cv, tr, look, w, s);
        nav(cv, tr, look, h, s, 1, Some("cor"));
        let x = NAV_W * s + 32.0 * s;
        let split = (x + (w - x) * 0.47).round();
        let cw = split - x - 28.0 * s;
        let mut y = page_head(cv, tr, look, x, 52.0 * s, cw, s, "Resultados para “cor”", "6 opções em 3 páginas · Esc volta", &[], 0);
        let groups: [(&str, &[RowM]); 3] = [
            ("BARRA DE TAREFAS › BOTÕES", &[row("Cor dos controles", "", Ctl::List("Branco")), row("Botões laterais", "", Ctl::List("Igual ao botão do meio"))]),
            ("BARRA DE TAREFAS › FUNDO", &[RowM { hover: true, ..row("Cor do fundo", "", Ctl::List("Capa desfocada")) }, row("Intensidade da cor", "", Ctl::Slider(0.78, "80%"))]),
            ("CORES E ÍCONES", &[row("Cor de destaque", "", Ctl::Seg(&["Cor da capa", "Do Windows", "Personalizada"], 0)), row("Barras de áudio: cores", "", Ctl::List("Cor da capa"))]),
        ];
        for (head, rows) in groups {
            ui::text(cv, tr, head, Font::Label, x + 2.0 * s, y, cw, 16.0 * s, WHITE, 0.5);
            y += 22.0 * s;
            y = rows_block(cv, tr, look, x, y, cw, s, rows, Some("cor")) + 18.0 * s;
        }
        let sr = RectF::new(split, 52.0 * s, w - split - 24.0 * s, h - 52.0 * s - 24.0 * s);
        stage(cv, tr, ptr, look, sr, s, Scene::Widget, Some(("Cor do fundo", "Neutro, cor da capa ou de destaque; ou o degradê vivo da capa ou a capa desfocada")), cfg, media);
    })
}

/// Os três níveis da opção "Cantos" lado a lado, nas três direções.
fn round_sheet(tr: &mut TextRenderer, s: f32, media: &MediaInfo) -> (usize, usize, Vec<u32>) {
    let (cw, ch) = (430.0 * s, 250.0 * s);
    let (lw, hh) = (130.0 * s, 54.0 * s);
    let (w, h) = (lw + 3.0 * cw, hh + 3.0 * ch);
    render(w, h, |cv| {
        cv.fill_round(RectF::new(0.0, 0.0, w, h), 0.0, [0.05, 0.05, 0.06], 1.0);
        for (k, t) in ["Menos (padrão)", "Normal", "Mais"].iter().enumerate() {
            ui::text(cv, tr, t, Font::Display, lw + k as f32 * cw + 22.0 * s, 10.0 * s, cw, 36.0 * s, WHITE, if k == 0 { 1.0 } else { 0.75 });
        }
        for (row, dir) in [Dir::Palco, Dir::Estudio, Dir::Ilha].into_iter().enumerate() {
            let y0 = hh + row as f32 * ch;
            ui::text(cv, tr, ["Palco", "Estúdio", "Ilha"][row], Font::BodyStrong, 18.0 * s, y0 + 16.0 * s, lw, 24.0 * s, WHITE, 0.8);
            for (col, round) in [Round::Menos, Round::Normal, Round::Mais].into_iter().enumerate() {
                let look = Look::new(dir, round, media);
                let cell = RectF::new(lw + col as f32 * cw, y0, cw, ch);
                let saved = cv.clip_rect();
                cv.set_clip(saved.intersect(cell));
                window_bg(cv, &look, w, h, s, media);
                let panel = RectF::new(cell.x + 16.0 * s, cell.y + 12.0 * s, cell.w - 32.0 * s, cell.h - 24.0 * s);
                match dir {
                    Dir::Palco => {
                        cv.fill_round(panel, look.rp(8.0, s), WHITE, 0.042);
                        cv.stroke_round(panel, look.rp(8.0, s), s, WHITE, 0.065);
                    }
                    Dir::Ilha => frost(cv, panel, look.rp(10.0, s), s),
                    Dir::Estudio => cv.stroke_round(panel, look.rp(6.0, s), s, WHITE, 0.10),
                }
                let right = panel.right() - 16.0 * s;
                let x0 = panel.x + 16.0 * s;
                let mut y = panel.y + 30.0 * s;
                toggle(cv, &look, x0 + 46.0 * s, y, false, false, 1.0, s);
                toggle(cv, &look, x0 + 106.0 * s, y, true, false, 1.0, s);
                button(cv, tr, &look, right, y, "Mostrar agora", true, 1.0, s);
                y += 50.0 * s;
                segmented(cv, tr, &look, right, y, &["Pequeno", "Normal", "Grande"], 1, None, 1.0, s);
                y += 50.0 * s;
                list(cv, tr, &look, right, y, "Capa desfocada", false, 1.0, s);
                y += 50.0 * s;
                slider(cv, tr, &look, right - 4.0 * s, y, 0.6, "60%", false, 1.0, s);
                cv.set_clip(saved);
            }
        }
    })
}

#[test]
#[ignore]
fn settings_mock() {
    let media = ui::sample_media();
    let s = 1.0;
    let mut tr = TextRenderer::new();
    tr.set_scale(s);
    let mut ptr = TextRenderer::new();
    for (k, dir) in [Dir::Palco, Dir::Estudio, Dir::Ilha].into_iter().enumerate() {
        // Tudo no nível padrão da opção "Cantos" (Menos), gadgets da prévia inclusive.
        let look = Look::new(dir, Round::Menos, &media);
        let cfg = Config { corners: look.corners(), ..Config::default() };
        let name = ["1-palco", "2-estudio", "3-ilha"][k];
        save(&format!("{name}-pagina.png"), &page_shot(&look, &mut tr, &mut ptr, s, &cfg, &media));
        let mut tr2 = TextRenderer::new();
        tr2.set_scale(2.0);
        save(&format!("{name}-controles.png"), &controls_sheet(&look, &mut tr2, 2.0, &media));
    }
    let look = Look::new(Dir::Palco, Round::Menos, &media);
    let cfg = Config { corners: look.corners(), ..Config::default() };
    save("1-palco-inicio.png", &home_shot(&look, &mut tr, &mut ptr, s, &cfg, &media));
    save("1-palco-busca.png", &search_shot(&look, &mut tr, &mut ptr, s, &cfg, &media));
    let mut tr15 = TextRenderer::new();
    tr15.set_scale(1.5);
    save("4-cantos-menos-normal-mais.png", &round_sheet(&mut tr15, 1.5, &media));
}
