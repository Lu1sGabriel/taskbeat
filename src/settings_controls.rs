//! Controles da tela de configurações nos três visuais (Palco, Estúdio, Ilha)
//! e nos três níveis de cantos: fundo da janela, menu, abas, painéis,
//! interruptor, segmentado, lista, slider, botões, busca, palco da prévia e
//! legenda. Nada de cópia do WinUI: cada visual tem os seus.
//!
//! Filho de `settings.rs` (as posições vêm do layout de lá; aqui só se desenha).

use super::*;
use crate::config::Look;
use crate::look::{self, Style};

/// Branco do texto (os três visuais são escuros, como o card).
pub(super) const WHITE: Rgb = [1.0; 3];
/// Texto escuro sobre o branco (seleção do Palco).
pub(super) const INK: Rgb = [0.06, 0.06, 0.08];

/// Tudo que os controles precisam: visual, cantos, cor de destaque e escala.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Skin {
    pub st: Style,
    pub accent: Rgb,
    pub s: f32,
}

/// Cor do ícone de cada página na Ilha (os "quadradinhos" coloridos do menu).
pub(super) fn nav_color(page: usize) -> Rgb {
    const C: [Rgb; 9] = [[0.36, 0.55, 1.0], [0.18, 0.70, 0.95], [0.60, 0.40, 1.0], [1.0, 0.38, 0.55], [1.0, 0.60, 0.22], [0.95, 0.35, 0.72], [0.25, 0.78, 0.45], [0.55, 0.56, 0.62], [0.30, 0.72, 0.85]];
    C[page % C.len()]
}

impl Skin {
    pub fn look(&self) -> Look {
        self.st.look
    }

    /// Tamanho do interruptor (px).
    pub fn toggle_size(&self) -> (f32, f32) {
        let s = self.s;
        match self.look() {
            Look::Palco => (42.0 * s, 24.0 * s),
            Look::Estudio => (36.0 * s, 20.0 * s),
            Look::Ilha => (46.0 * s, 26.0 * s),
        }
    }

    /// Altura de uma linha comum (px).
    pub fn row_h(&self) -> f32 {
        self.s
            * match self.look() {
                Look::Palco => 54.0,
                Look::Estudio => 50.0,
                Look::Ilha => 54.0,
            }
    }

    /// Altura dos controles de uma linha (segmentado, lista, botão).
    pub fn ctl_h(&self) -> f32 {
        self.s
            * match self.look() {
                Look::Estudio => 30.0,
                _ => 34.0,
            }
    }

    /// Recuo das linhas dentro do painel.
    pub fn pad(&self) -> f32 {
        self.s * if self.look() == Look::Estudio { 4.0 } else { 18.0 }
    }

    // ---------------------------------------------------------------- fundo da janela

    /// Uma linha de pixels do fundo da janela (`w` px): o fundo só muda na
    /// horizontal, então a lista pode rolar copiando pixels sem emendas.
    /// `nav`: largura do menu; `art`: cores da capa (Palco e Ilha).
    pub fn window_row(&self, w: usize, nav: f32, art: Option<(Rgb, Rgb)>) -> Vec<u32> {
        let wf = w.max(1) as f32;
        let (vib, avg) = art.unwrap_or((self.accent, self.accent));
        (0..w)
            .map(|x| {
                let xf = x as f32 + 0.5;
                let u = xf / wf;
                let c: Rgb = match self.look() {
                    Look::Palco => {
                        let base = [0.043, 0.043, 0.054];
                        let l1 = (-((u / 0.22).powi(2))).exp();
                        let l2 = (-(((u - 1.0) / 0.40).powi(2))).exp();
                        lerp3(lerp3(base, vib, 0.13 * l1), avg, 0.07 * l2)
                    }
                    Look::Estudio => {
                        if xf < nav {
                            if xf > nav - 1.0 { lerp3([0.052, 0.053, 0.060], WHITE, 0.07) } else { [0.052, 0.053, 0.060] }
                        } else {
                            [0.071, 0.073, 0.082]
                        }
                    }
                    Look::Ilha => {
                        // A cor da capa espalhada da esquerda para a direita, bem escura.
                        let t = (u * 1.15).min(1.0);
                        let tint = lerp3(vib, avg, t);
                        lerp3([0.035, 0.035, 0.05], tint, 0.20 - 0.08 * u)
                    }
                };
                // Pontilhado fino por coluna (±1 nível): sem ele o degradê
                // escuro vira faixas.
                let mut hh = (x as u32).wrapping_mul(0x9E37_79B1);
                hh ^= hh >> 15;
                hh = hh.wrapping_mul(0x2C1B_3C6D);
                hh ^= hh >> 12;
                let n = ((hh & 0xffff) as f32 / 65535.0 - 0.5) * (1.0 / 255.0);
                premul([c[0] + n, c[1] + n, c[2] + n], 1.0)
            })
            .collect()
    }

    /// Painel de vidro fosco (Ilha) ou de cartão (Palco).
    pub fn frost(&self, cv: &mut Canvas, r: RectF, radius: f32) {
        let s = self.s;
        cv.drop_shadow(r, radius, 22.0 * s, 8.0 * s, 0.28);
        cv.fill_round(r, radius, WHITE, 0.075);
        let y0 = r.y;
        cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), radius, |_, y| (WHITE, if y < y0 + 1.5 * s { 0.14 } else { 0.0 }));
        cv.stroke_round(r, radius, s, WHITE, 0.13);
    }

    /// Vidro fosco sem sombra (painéis recortados por linha e o menu).
    pub fn frost_flat(&self, cv: &mut Canvas, r: RectF, radius: f32) {
        let s = self.s;
        cv.fill_round(r, radius, WHITE, 0.075);
        let y0 = r.y;
        cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), radius, |_, y| (WHITE, if y < y0 + 1.5 * s { 0.14 } else { 0.0 }));
        cv.stroke_round(r, radius, s, WHITE, 0.13);
    }

    /// Raio dos painéis de opções.
    pub fn panel_r(&self) -> f32 {
        match self.look() {
            Look::Palco => self.st.r(8.0, 1e6, self.s).min(self.st.scale(14.0 * self.s).max(8.0 * self.s)),
            Look::Estudio => 0.0,
            Look::Ilha => (self.st.scale(16.0 * self.s)).max(9.0 * self.s),
        }
    }

    /// Fundo de um painel de opções (`clip`: parte a desenhar, para refazer só
    /// uma linha dele).
    pub fn panel(&self, cv: &mut Canvas, r: RectF) {
        let s = self.s;
        let rad = self.panel_r();
        match self.look() {
            Look::Palco => {
                cv.fill_round(r, rad, WHITE, 0.042);
                let y0 = r.y;
                cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), rad, |_, y| (WHITE, if y < y0 + 1.5 * s { 0.07 } else { 0.0 }));
                cv.stroke_round(r, rad, s, WHITE, 0.065);
            }
            Look::Estudio => {
                cv.fill_rrect(r.x, r.y, r.right(), r.y + s, 0.0, 0.08, WHITE);
                cv.fill_rrect(r.x, r.bottom() - s, r.right(), r.bottom(), 0.0, 0.07, WHITE);
            }
            Look::Ilha => self.frost_flat(cv, r, rad),
        }
    }

    /// Realce da linha sob o mouse (dentro do painel `panel`).
    pub fn row_hover(&self, cv: &mut Canvas, panel: RectF, row: RectF, a: f32) {
        if a <= 0.0 {
            return;
        }
        let old = cv.clip_rect();
        cv.set_clip(old.intersect(row));
        match self.look() {
            Look::Estudio => {
                cv.fill_round(row, 0.0, WHITE, 0.035 * a);
                cv.fill_rrect(row.x, row.y + 8.0 * self.s, row.x + 2.0 * self.s, row.bottom() - 8.0 * self.s, 0.0, a, self.accent);
            }
            _ => cv.fill_round(panel, self.panel_r(), WHITE, 0.045 * a),
        }
        cv.set_clip(old);
    }

    /// Separador entre duas linhas de um painel.
    pub fn divider(&self, cv: &mut Canvas, x0: f32, x1: f32, y: f32) {
        let s = self.s;
        let inset = if self.look() == Look::Estudio { 0.0 } else { 18.0 * s };
        cv.fill_rrect(x0 + inset, y, x1 - inset, y + s.round().max(1.0), 0.0, if self.look() == Look::Estudio { 0.07 } else { 0.06 }, WHITE);
    }

    // ---------------------------------------------------------------- menu lateral

    /// Item do menu: ícone (num quadradinho), nome; `sel` 0..1 (animado).
    #[allow(clippy::too_many_arguments)]
    pub fn nav_item(&self, cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, icon: &str, name: &str, color: Rgb, sel: bool, hot: bool) {
        let s = self.s;
        let chip = RectF::new(r.x + 10.0 * s, r.cy() - 14.0 * s, 28.0 * s, 28.0 * s);
        let row_r = self.st.r(6.0, r.h, s);
        match self.look() {
            Look::Palco => {
                if hot && !sel {
                    cv.fill_round(r, row_r, WHITE, 0.04);
                }
                if sel {
                    cv.glow(chip, self.st.r(5.0, chip.h, s), 10.0 * s, 2.0 * s, self.accent, 0.45);
                    look::fill_accent(cv, chip, self.st.r(5.0, chip.h, s), self.st, self.accent, 1.0);
                    ui::glyph(cv, tr, chip, icon, Font::IconSmall, on_color(self.accent), 1.0);
                } else {
                    cv.fill_round(chip, self.st.r(5.0, chip.h, s), WHITE, 0.05);
                    ui::glyph(cv, tr, chip, icon, Font::IconSmall, WHITE, 0.78);
                }
                let font = if sel { Font::BodyStrong } else { Font::Body };
                ui::text(cv, tr, name, font, chip.right() + 12.0 * s, r.y, r.right() - chip.right() - 14.0 * s, r.h, WHITE, if sel { 1.0 } else { 0.78 });
            }
            Look::Estudio => {
                if hot && !sel {
                    cv.fill_round(r, row_r, WHITE, 0.03);
                }
                ui::glyph(cv, tr, chip, icon, Font::IconSmall, if sel { self.accent } else { WHITE }, if sel { 1.0 } else { 0.6 });
                ui::text(cv, tr, name, Font::Body, chip.right() + 10.0 * s, r.y, r.right() - chip.right() - 12.0 * s, r.h, WHITE, if sel { 1.0 } else { 0.62 });
            }
            Look::Ilha => {
                if hot && !sel {
                    cv.fill_round(r, row_r, WHITE, 0.07);
                }
                let c = RectF::new(chip.x, chip.y + 1.0 * s, 26.0 * s, 26.0 * s);
                let top = lerp3(color, WHITE, 0.18);
                let (cy0, ch) = (c.y, c.h);
                cv.fill_rrect_ca(c.x, c.y, c.right(), c.bottom(), self.st.r(5.0, c.h, s), |_, yy| (lerp3(top, color, ((yy - cy0) / ch).clamp(0.0, 1.0)), 1.0));
                ui::glyph(cv, tr, c, icon, Font::IconSmall, WHITE, 1.0);
                ui::text(cv, tr, name, if sel { Font::BodyStrong } else { Font::Body }, chip.right() + 12.0 * s, r.y, r.right() - chip.right() - 14.0 * s, r.h, WHITE, if sel { 1.0 } else { 0.85 });
            }
        }
    }

    /// Fundo do item escolhido no menu (desliza entre os itens).
    pub fn nav_selection(&self, cv: &mut Canvas, r: RectF, travel: f32) {
        let s = self.s;
        match self.look() {
            Look::Palco => cv.fill_round(r, self.st.r(6.0, r.h, s), WHITE, 0.075),
            Look::Estudio => {
                cv.fill_round(r, self.st.r(6.0, r.h, s), WHITE, 0.045);
                let ih = (r.h - 18.0 * s) * (1.0 + 0.5 * travel);
                let bar = RectF::new(r.x, r.cy() - ih / 2.0, 3.0 * s, ih);
                look::fill_accent(cv, bar, 0.0, self.st, self.accent, 1.0);
            }
            Look::Ilha => cv.fill_round(r, self.st.r(6.0, r.h, s), WHITE, 0.14),
        }
    }

    /// Campo de busca do menu.
    pub fn search(&self, cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, text: &str, focused: bool, hot: bool) {
        let s = self.s;
        let rad = self.st.r(6.0, r.h, s);
        match self.look() {
            Look::Palco => {
                cv.fill_round(r, rad, WHITE, if focused { 0.10 } else if hot { 0.08 } else { 0.06 });
                cv.stroke_round(r, rad, s, if focused { self.accent } else { WHITE }, if focused { 0.9 } else { 0.07 });
            }
            Look::Estudio => {
                cv.fill_round(r, rad, WHITE, 0.035);
                cv.stroke_round(r, rad, s, if focused { self.accent } else { WHITE }, if focused { 1.0 } else { 0.14 });
            }
            Look::Ilha => {
                cv.fill_round(r, rad, WHITE, if focused { 0.16 } else if hot { 0.13 } else { 0.10 });
                if focused {
                    cv.stroke_round(r, rad, 1.5 * s, self.accent, 0.9);
                }
            }
        }
        ui::glyph(cv, tr, RectF::new(r.x + 8.0 * s, r.y, 22.0 * s, r.h), "\u{E721}", Font::IconSmall, WHITE, 0.6);
        if text.is_empty() {
            ui::text(cv, tr, tr_("Buscar opção"), Font::Body, r.x + 34.0 * s, r.y, r.w - 40.0 * s, r.h, WHITE, 0.42);
            if self.look() == Look::Estudio && !focused {
                let k = RectF::new(r.right() - 52.0 * s, r.y + 8.0 * s, 44.0 * s, r.h - 16.0 * s);
                cv.stroke_round(k, 3.0 * s, s, WHITE, 0.2);
                ui::text_center(cv, tr, "Ctrl F", Font::Small, k, WHITE, 0.5);
            }
        } else {
            ui::text(cv, tr, text, Font::Body, r.x + 34.0 * s, r.y, r.w - 64.0 * s, r.h, WHITE, 1.0);
            ui::glyph(cv, tr, RectF::new(r.right() - 30.0 * s, r.y, 22.0 * s, r.h), "\u{E711}", Font::IconSmall, WHITE, 0.6);
        }
        if focused {
            let tw = if text.is_empty() { 0.0 } else { tr.mask(text, Font::Body, (r.w - 64.0 * s) as i32, r.h as i32, Align::Left).used_width() };
            let cx = r.x + 35.0 * s + tw;
            cv.fill_rrect(cx, r.y + 9.0 * s, cx + 1.5 * s, r.bottom() - 9.0 * s, 0.5 * s, 1.0, WHITE);
        }
    }

    // ---------------------------------------------------------------- abas

    /// Abas da página (`cells` já medidas pelo layout).
    pub fn tabs(&self, cv: &mut Canvas, tr: &mut TextRenderer, cells: &[RectF], labels: &[&str], sel: usize, hover: Option<usize>) {
        let s = self.s;
        if cells.is_empty() {
            return;
        }
        match self.look() {
            Look::Palco => {
                for (k, r) in cells.iter().enumerate() {
                    let on = k == sel;
                    let rad = self.st.r(6.0, r.h, s);
                    if on {
                        cv.drop_shadow(*r, rad, 8.0 * s, 2.0 * s, 0.35);
                        cv.fill_round(*r, rad, WHITE, 0.96);
                    } else {
                        cv.fill_round(*r, rad, WHITE, if hover == Some(k) { 0.08 } else { 0.045 });
                    }
                    let font = if on { Font::BodyStrong } else { Font::Body };
                    ui::text_center(cv, tr, labels[k], font, r.offset(0.0, -0.5 * s), if on { INK } else { WHITE }, if on { 1.0 } else { 0.72 });
                }
            }
            Look::Estudio => {
                let (x0, x1) = (cells[0].x, cells[cells.len() - 1].right());
                let y = cells[0].bottom() + 6.0 * s;
                for (k, r) in cells.iter().enumerate() {
                    let on = k == sel;
                    ui::text_center(cv, tr, labels[k], if on { Font::BodyStrong } else { Font::Body }, r.offset(0.0, -s), WHITE, if on { 1.0 } else if hover == Some(k) { 0.8 } else { 0.55 });
                    if on {
                        let u = RectF::new(r.x + 6.0 * s, y - 3.0 * s, r.w - 12.0 * s, 2.0 * s);
                        look::fill_accent(cv, u, 0.0, self.st, self.accent, 1.0);
                    }
                }
                let _ = (x0, x1);
            }
            Look::Ilha => {
                let track = RectF::new(cells[0].x - 3.0 * s, cells[0].y - 3.0 * s, cells[cells.len() - 1].right() - cells[0].x + 6.0 * s, cells[0].h + 6.0 * s);
                cv.fill_round(track, self.st.r(7.0, track.h, s), WHITE, 0.10);
                for (k, r) in cells.iter().enumerate() {
                    let on = k == sel;
                    let rad = self.st.r(5.0, r.h, s);
                    if on {
                        cv.drop_shadow(*r, rad, 6.0 * s, 1.5 * s, 0.30);
                        cv.fill_round(*r, rad, WHITE, 0.26);
                        cv.stroke_round(*r, rad, s, WHITE, 0.18);
                    } else if hover == Some(k) {
                        cv.fill_round(*r, rad, WHITE, 0.08);
                    }
                    ui::text_center(cv, tr, labels[k], if on { Font::BodyStrong } else { Font::Body }, r.offset(0.0, -0.5 * s), WHITE, if on { 1.0 } else { 0.75 });
                }
            }
        }
    }

    // ---------------------------------------------------------------- controles

    /// Interruptor (`t`: posição animada 0..1).
    pub fn switch(&self, cv: &mut Canvas, r: RectF, t: f32, hot: f32, en: f32) {
        let fg = WHITE;
        let knob_on = if self.look() == Look::Palco { [1.0; 3] } else { WHITE };
        let e = ui::ease_move(t);
        if self.look() == Look::Palco && e > 0.0 && en > 0.5 {
            cv.glow(r, self.st.r(6.0, r.h, self.s), 10.0 * self.s, 2.0 * self.s, self.accent, 0.40 * e);
        }
        match self.look() {
            Look::Palco => {
                // Trilho ligado na cor de destaque, botão branco (como o play do card).
                let rad = self.st.r(6.0, r.h, self.s);
                cv.fill_round(r, rad, WHITE, 0.15 * (1.0 - e) * en.max(0.5));
                look::fill_accent(cv, r, rad, self.st, self.accent, e * en.max(0.45));
                let inset = 3.0 * self.s;
                let kd = r.h - 2.0 * inset + hot * self.s;
                let kx = r.x + inset + (r.w - 2.0 * inset - (r.h - 2.0 * inset)) * e - hot * 0.5 * self.s;
                let k = RectF::new(kx, r.cy() - kd / 2.0, kd, kd);
                let krad = self.st.round_btn(kd);
                cv.drop_shadow(k, krad, 4.0 * self.s, 1.0 * self.s, 0.35 * en);
                cv.fill_round(k, krad, knob_on, (0.88 + 0.12 * e) * en.max(0.5));
            }
            _ => look::switch(cv, r, e, self.st, self.accent, knob_on, fg, en, hot, self.s),
        }
    }

    /// Controle segmentado: `track` em volta, `cells` (uma por opção), `pos`
    /// (posição animada da escolhida).
    #[allow(clippy::too_many_arguments)]
    pub fn segmented(&self, cv: &mut Canvas, tr: &mut TextRenderer, track: RectF, cells: &[RectF], opts: &[&str], sel: usize, pos: f32, hover: i8, en: f32) {
        let s = self.s;
        if cells.is_empty() {
            return;
        }
        let p = pos.clamp(0.0, (cells.len() - 1) as f32);
        let (i0, i1) = (p.floor() as usize, (p.ceil() as usize).min(cells.len() - 1));
        let pill = cells[i0].lerp(cells[i1], p - i0 as f32);
        let ea = en.max(0.5);
        match self.look() {
            Look::Palco => {
                cv.fill_round(track, self.st.r(7.0, track.h, s), WHITE, 0.06 * ea);
                if hover >= 0 && hover as usize != sel {
                    cv.fill_round(cells[hover as usize], self.st.r(5.0, cells[0].h, s), WHITE, 0.06);
                }
                let rad = self.st.r(5.0, pill.h, s);
                cv.drop_shadow(pill, rad, 6.0 * s, 1.5 * s, 0.35 * en);
                cv.fill_round(pill, rad, WHITE, 0.96 * ea);
                for (k, c) in cells.iter().enumerate() {
                    let on = k == sel;
                    // Texto escuro sobre a pílula branca (onde ela está agora).
                    let cover = 1.0 - ((k as f32 - p).abs()).min(1.0);
                    let col = lerp3(WHITE, INK, cover);
                    ui::text_center(cv, tr, opts[k], if on { Font::BodyStrong } else { Font::Body }, c.offset(0.0, -0.5 * s), col, (0.78 + 0.22 * cover) * en);
                }
            }
            Look::Estudio => {
                let rad = self.st.r(5.0, track.h, s);
                if hover >= 0 && hover as usize != sel {
                    cv.fill_round(cells[hover as usize], 0.0, WHITE, 0.06);
                }
                let ends = |k: usize| if k == 0 || k + 1 == cells.len() { rad } else { 0.0 };
                look::fill_accent(cv, pill, ends(sel), self.st, self.accent, ea);
                cv.stroke_round(track, rad, s, WHITE, 0.16 * ea);
                for c in cells.iter().skip(1) {
                    cv.fill_rrect(c.x, track.y + s, c.x + s, track.bottom() - s, 0.0, 0.16 * ea, WHITE);
                }
                for (k, c) in cells.iter().enumerate() {
                    let on = k == sel;
                    ui::text_center(cv, tr, opts[k], if on { Font::BodyStrong } else { Font::Body }, c.offset(0.0, -0.5 * s), WHITE, if on { 1.0 } else { 0.66 } * en);
                }
            }
            Look::Ilha => {
                cv.fill_round(track, self.st.r(7.0, track.h, s), WHITE, 0.10 * ea);
                if hover >= 0 && hover as usize != sel {
                    cv.fill_round(cells[hover as usize], self.st.r(5.0, cells[0].h, s), WHITE, 0.07);
                }
                let rad = self.st.r(5.0, pill.h, s);
                cv.drop_shadow(pill, rad, 6.0 * s, 1.5 * s, 0.30 * en);
                cv.fill_round(pill, rad, WHITE, 0.26 * ea);
                cv.stroke_round(pill, rad, s, WHITE, 0.18 * en);
                for (k, c) in cells.iter().enumerate() {
                    let on = k == sel;
                    ui::text_center(cv, tr, opts[k], if on { Font::BodyStrong } else { Font::Body }, c.offset(0.0, -0.5 * s), WHITE, if on { 1.0 } else { 0.75 } * en);
                }
            }
        }
    }

    /// Lista (abre o menu ao clicar): valor e setinha.
    pub fn choice(&self, cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, value: &str, hot: f32, en: f32) {
        let s = self.s;
        let ea = en.max(0.5);
        let rad = self.st.r(6.0, r.h, s);
        match self.look() {
            Look::Palco => {
                cv.fill_round(r, rad, WHITE, (0.07 + 0.04 * hot) * ea);
                cv.stroke_round(r, rad, s, WHITE, 0.06);
                ui::text(cv, tr, value, Font::Body, r.x + 14.0 * s, r.y, r.w - 44.0 * s, r.h, WHITE, 0.95 * en);
                ui::glyph(cv, tr, RectF::new(r.right() - 32.0 * s, r.y, 22.0 * s, r.h), "\u{E70D}", Font::IconSmall, WHITE, 0.7 * en);
            }
            Look::Estudio => {
                cv.fill_round(r, rad, WHITE, 0.03 + 0.04 * hot);
                cv.stroke_round(r, rad, s, if hot > 0.5 { self.accent } else { WHITE }, if hot > 0.5 { 0.8 } else { 0.16 });
                ui::text(cv, tr, value, Font::Body, r.x + 12.0 * s, r.y, r.w - 44.0 * s, r.h, WHITE, 0.95 * en);
                let a = RectF::new(r.right() - 28.0 * s, r.y, 28.0 * s, r.h);
                cv.fill_rrect(a.x, r.y + 6.0 * s, a.x + s, r.bottom() - 6.0 * s, 0.0, 0.16, WHITE);
                ui::glyph(cv, tr, a, "\u{E70D}", Font::IconSmall, WHITE, 0.7 * en);
            }
            Look::Ilha => {
                cv.drop_shadow(r, rad, 4.0 * s, 1.0 * s, 0.18);
                cv.fill_round(r, rad, WHITE, (0.14 + 0.06 * hot) * ea);
                cv.stroke_round(r, rad, s, WHITE, 0.14);
                ui::text(cv, tr, value, Font::Body, r.x + 14.0 * s, r.y, r.w - 44.0 * s, r.h, WHITE, en);
                let a = RectF::new(r.right() - 28.0 * s, r.y + 5.0 * s, 20.0 * s, r.h - 10.0 * s);
                cv.fill_round(a, self.st.r(4.0, a.h, s), self.accent, ea);
                ui::glyph(cv, tr, a.offset(0.0, -3.0 * s), "\u{E70E}", Font::IconSmall, WHITE, 0.95);
                ui::glyph(cv, tr, a.offset(0.0, 3.0 * s), "\u{E70D}", Font::IconSmall, WHITE, 0.95);
            }
        }
    }

    /// Slider (`r`: área do trilho; `t` 0..1) e o valor à esquerda dele.
    #[allow(clippy::too_many_arguments)]
    pub fn slider(&self, cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, t: f32, label: &str, hot: f32, en: f32) {
        let s = self.s;
        let cy = r.cy();
        let (x0, x1) = (r.x + 8.0 * s, r.right() - 8.0 * s);
        let xv = x0 + (x1 - x0) * t.clamp(0.0, 1.0);
        let ea = en.max(0.45);
        match self.look() {
            Look::Palco => {
                let th = (4.0 + 2.0 * hot) * s;
                let rr = self.st.r(1.5, th, s);
                cv.fill_rrect(x0, cy - th / 2.0, x1, cy + th / 2.0, rr, 0.16 * ea, WHITE);
                let bar = RectF::new(x0, cy - th / 2.0, (xv - x0).max(th), th);
                if en > 0.5 {
                    cv.glow(bar, rr, 7.0 * s, 0.0, self.accent, 0.35);
                }
                look::fill_accent(cv, bar, rr, self.st, self.accent, ea);
                let k = if self.st.round == Corners::Large { RectF::new(xv - 8.5 * s, cy - 8.5 * s, 17.0 * s, 17.0 * s) } else { RectF::new(xv - 6.0 * s, cy - 10.0 * s, 12.0 * s, 20.0 * s) };
                let kr = self.st.r(3.5, k.w, s);
                cv.drop_shadow(k, kr, 5.0 * s, 1.0 * s, 0.4 * en);
                cv.fill_round(k, kr, WHITE, ea);
                let tw = tr.mask(label, Font::CaptionStrong, (200.0 * s) as i32, (24.0 * s) as i32, Align::Left).used_width();
                let p = RectF::new(r.x - tw - 30.0 * s, cy - 12.0 * s, tw + 18.0 * s, 24.0 * s);
                cv.fill_round(p, self.st.r(4.0, p.h, s), WHITE, 0.07 * ea);
                ui::text_center(cv, tr, label, Font::CaptionStrong, p, WHITE, 0.9 * en.max(0.4));
            }
            Look::Estudio => {
                for k in 0..=10 {
                    let x = x0 + (x1 - x0) * k as f32 / 10.0;
                    let th = if k % 5 == 0 { 5.0 } else { 3.0 } * s;
                    cv.fill_rrect(x - 0.5 * s, cy + 6.0 * s, x + 0.5 * s, cy + 6.0 * s + th, 0.0, 0.22 * ea, WHITE);
                }
                cv.fill_rrect(x0, cy - s, x1, cy + s, 0.0, 0.22 * ea, WHITE);
                look::fill_accent(cv, RectF::new(x0, cy - s, (xv - x0).max(2.0 * s), 2.0 * s), 0.0, self.st, self.accent, ea);
                let k = RectF::new(xv - 4.5 * s, cy - 10.0 * s, 9.0 * s, 20.0 * s);
                cv.drop_shadow(k, 2.0 * s, 4.0 * s, 1.0 * s, 0.4 * en);
                cv.fill_round(k, 2.0 * s, if en > 0.5 { WHITE } else { [0.5; 3] }, 1.0);
                cv.fill_rrect(k.x + 2.0 * s, cy - 0.5 * s, k.right() - 2.0 * s, cy + 0.5 * s, 0.0, 0.8, INK);
                let b = RectF::new(r.x - 66.0 * s, cy - 12.0 * s, 54.0 * s, 24.0 * s);
                cv.stroke_round(b, self.st.r(5.0, b.h, s), s, WHITE, 0.16 * ea);
                ui::text_center(cv, tr, label, Font::Caption, b, WHITE, 0.9 * en.max(0.4));
            }
            Look::Ilha => {
                let th = 6.0 * s;
                let rr = self.st.r(2.0, th, s);
                cv.fill_rrect(x0, cy - th / 2.0, x1, cy + th / 2.0, rr, 0.20 * ea, WHITE);
                cv.fill_rrect(x0, cy - th / 2.0, xv.max(x0 + th), cy + th / 2.0, rr, ea, self.accent);
                let k = if self.st.round == Corners::Large { RectF::new(xv - 10.5 * s, cy - 10.5 * s, 21.0 * s, 21.0 * s) } else { RectF::new(xv - 8.0 * s, cy - 11.0 * s, 16.0 * s, 22.0 * s) };
                let kr = self.st.r(4.5, k.w, s);
                cv.drop_shadow(k, kr, 6.0 * s, 1.5 * s, 0.45 * en);
                cv.fill_round(k, kr + hot * 0.0, WHITE, ea);
                ui::text_right(cv, tr, label, Font::Body, r.x - 12.0 * s, cy - 11.0 * s, 22.0 * s, WHITE, 0.7 * en.max(0.4), s);
            }
        }
    }

    /// Botão (`primary`: o branco/destaque do visual).
    #[allow(clippy::too_many_arguments)]
    pub fn button(&self, cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, label: &str, primary: bool, hot: f32, en: f32) {
        let s = self.s;
        let ea = en.max(0.4);
        let rad = self.st.r(6.0, r.h, s);
        match self.look() {
            Look::Palco => {
                if primary {
                    cv.drop_shadow(r, rad, 8.0 * s, 2.0 * s, 0.35 * en);
                    cv.fill_round(r, rad, WHITE, (0.96 - 0.06 * hot) * ea);
                    ui::text_center(cv, tr, label, Font::BodyStrong, r.offset(0.0, -0.5 * s), INK, ea);
                } else {
                    cv.fill_round(r, rad, WHITE, (0.09 + 0.05 * hot) * ea);
                    cv.stroke_round(r, rad, s, WHITE, 0.08);
                    ui::text_center(cv, tr, label, Font::BodyStrong, r.offset(0.0, -0.5 * s), WHITE, 0.95 * ea);
                }
            }
            Look::Estudio => {
                if primary {
                    look::fill_accent(cv, r, rad, self.st, self.accent, ea);
                } else {
                    cv.fill_round(r, rad, WHITE, 0.05 * hot);
                    cv.stroke_round(r, rad, s, WHITE, 0.3 * ea);
                }
                ui::text_center(cv, tr, label, Font::BodyStrong, r.offset(0.0, -0.5 * s), WHITE, ea);
            }
            Look::Ilha => {
                cv.drop_shadow(r, rad, 5.0 * s, 1.0 * s, 0.2 * en);
                if primary {
                    cv.fill_round(r, rad, lerp3(self.accent, WHITE, 0.10 * hot), ea);
                } else {
                    cv.fill_round(r, rad, WHITE, (0.16 + 0.06 * hot) * ea);
                    cv.stroke_round(r, rad, s, WHITE, 0.14);
                }
                ui::text_center(cv, tr, label, Font::BodyStrong, r.offset(0.0, -0.5 * s), WHITE, ea);
            }
        }
    }

    /// Setinha do "Ajuste fino" (aberto: para cima).
    pub fn chevron(&self, cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, open: bool, en: f32) {
        ui::glyph(cv, tr, r, if open { "\u{E70E}" } else { "\u{E70D}" }, Font::IconSmall, WHITE, 0.75 * en);
    }

    // ---------------------------------------------------------------- palco da prévia

    /// Moldura do palco da prévia; devolve a área de dentro e o raio dela.
    pub fn stage_frame(&self, cv: &mut Canvas, r: RectF) -> (RectF, f32) {
        let s = self.s;
        match self.look() {
            Look::Palco => {
                let rad = self.st.scale(16.0 * s).max(8.0 * s);
                cv.drop_shadow(r, rad, 30.0 * s, 10.0 * s, 0.45);
                (r, rad)
            }
            Look::Estudio => {
                let rad = self.st.scale(8.0 * s);
                cv.stroke_round(r.inset(-1.0), rad + 1.0, s, WHITE, 0.08);
                (r, rad)
            }
            Look::Ilha => {
                let rad = self.st.scale(18.0 * s).max(10.0 * s);
                self.frost(cv, r, rad);
                (r.inset(8.0 * s), (rad - 6.0 * s).max(5.0 * s))
            }
        }
    }

    /// Grupo de botões da prévia (tocando/pausado/sem música, escuro/claro).
    pub fn chip_group(&self, cv: &mut Canvas, g: RectF, a: f32) {
        let s = self.s;
        let rad = self.st.r(6.0, g.h, s);
        cv.fill_round(g, rad, [0.0; 3], 0.34 * a);
        cv.stroke_round(g, rad, s, WHITE, 0.10 * a);
    }

    /// Um botão da prévia; `sel`: o escolhido.
    #[allow(clippy::too_many_arguments)]
    pub fn chip(&self, cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, label: &str, sel: bool, hot: bool, a: f32) {
        let s = self.s;
        let pill = r.inset(2.0 * s);
        let rad = self.st.r(4.0, pill.h, s);
        if sel {
            match self.look() {
                Look::Estudio => look::fill_accent(cv, pill, rad, self.st, self.accent, a),
                _ => cv.fill_round(pill, rad, WHITE, 0.92 * a),
            }
        } else if hot {
            cv.fill_round(pill, rad, WHITE, 0.12 * a);
        }
        let tc = if sel && self.look() != Look::Estudio { INK } else { WHITE };
        ui::text_center(cv, tr, label, Font::Caption, r.offset(0.0, -0.5 * s), tc, if sel { 1.0 } else { 0.85 } * a);
    }

    /// Legenda sobre a prévia: o que a opção sob o mouse faz.
    pub fn caption(&self, cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, title: &str, desc: &str, a: f32) {
        let s = self.s;
        let rad = self.st.scale(10.0 * s).max(4.0 * s);
        match self.look() {
            Look::Estudio => {
                cv.fill_round(r, rad, [0.04; 3], 0.88 * a);
                cv.fill_rrect(r.x, r.y, r.x + 3.0 * s, r.bottom(), 0.0, a, self.accent);
            }
            _ => {
                cv.drop_shadow(r, rad, 16.0 * s, 4.0 * s, 0.35 * a);
                cv.fill_round(r, rad, [0.06, 0.06, 0.08], 0.84 * a);
                cv.stroke_round(r, rad, s, WHITE, 0.10 * a);
            }
        }
        ui::glyph(cv, tr, RectF::new(r.x + 12.0 * s, r.y + 9.0 * s, 22.0 * s, 22.0 * s), "\u{E946}", Font::IconSmall, self.accent, a);
        ui::text(cv, tr, title, Font::BodyStrong, r.x + 40.0 * s, r.y + 8.0 * s, r.w - 52.0 * s, 24.0 * s, WHITE, a);
        ui::text(cv, tr, desc, Font::Caption, r.x + 40.0 * s, r.y + 31.0 * s, r.w - 52.0 * s, 22.0 * s, WHITE, 0.78 * a);
    }

    /// Dica de opção desativada (o que ligar antes).
    pub fn tip(&self, cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, msg: &str, a: f32) {
        let s = self.s;
        let rad = self.st.scale(6.0 * s).max(3.0 * s);
        cv.drop_shadow(r, rad, 12.0 * s, 3.0 * s, 0.45 * a);
        cv.fill_round(r, rad, [0.12, 0.12, 0.14], a);
        match self.look() {
            Look::Estudio => cv.fill_rrect(r.x, r.y, r.x + 3.0 * s, r.bottom(), 0.0, a, self.accent),
            _ => cv.stroke_round(r, rad, s, WHITE, 0.12 * a),
        }
        ui::glyph(cv, tr, RectF::new(r.x + 9.0 * s, r.y, 20.0 * s, r.h), "\u{E946}", Font::IconSmall, self.accent, a);
        ui::text(cv, tr, msg, Font::Caption, r.x + 33.0 * s, r.y, r.w - 36.0 * s, r.h, WHITE, 0.92 * a);
    }
}

/// `tr` com outro nome (o `tr` daqui é o renderizador de texto).
fn tr_(t: &'static str) -> &'static str {
    crate::i18n::tr(t)
}
