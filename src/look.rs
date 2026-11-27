//! Identidade visual do app inteiro: Palco, Estúdio ou Ilha (opção "Visual"),
//! e o nível de cantos arredondados (opção "Cantos": Menos, Normal, Mais).
//!
//! Vale para tudo que o app desenha: configurações, widget da barra, card,
//! "A seguir", aviso de teclas, letra e menus. Cada peça recebe o `Style`
//! junto das cores (`ui::Theme::style`) e pede aqui os raios e os acabamentos.
//!
//! - **Palco**: escuro com a cor da música; brilho na cor de destaque, granulado
//!   e contorno claro. Com os cantos "Normal" é exatamente o desenho de antes
//!   (o Mac e o Linux continuam assim).
//! - **Estúdio**: grafite liso, linha fina no lugar de brilho e sombra colorida,
//!   destaque em degradê, controles retos como os de uma mesa de som.
//! - **Ilha**: vidro fosco: a cor da capa aparece mais, película clara, reflexo
//!   suave no alto e contorno claro; botões brancos com sombra macia.

use crate::config::{Config, Corners, Look};
use crate::render::{Canvas, RectF, Rgb, hsv, lerp3, rgb_to_hsv};

/// Identidade e cantos em uso.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Style {
    pub look: Look,
    pub round: Corners,
}

impl Style {
    pub fn of(cfg: &Config) -> Style {
        Style { look: cfg.look, round: cfg.corners }
    }

    /// O Estúdio é mais reto em todos os níveis.
    fn straight(self) -> f32 {
        if self.look == Look::Estudio { 0.55 } else { 1.0 }
    }

    /// Escala um raio que antes era fixo (`normal`, em px já escalados): o de
    /// antes no "Normal", bem menor no "Menos", maior no "Mais".
    pub fn scale(self, normal: f32) -> f32 {
        let k = match self.round {
            Corners::Small => 0.5,
            Corners::Medium => 1.0,
            Corners::Large => 1.45,
        };
        normal * k * self.straight()
    }

    /// Raio de um controle de altura `h` (px): `base` (px lógicos × `s`) no
    /// "Menos", o dobro no "Normal" (sem passar de pílula) e pílula no "Mais".
    pub fn r(self, base: f32, h: f32, s: f32) -> f32 {
        let k = self.straight();
        match self.round {
            Corners::Small => base * k * s,
            Corners::Medium => (base * 2.0 * k * s).min(h / 2.0),
            Corners::Large => h / 2.0 * if self.look == Look::Estudio { 0.6 } else { 1.0 },
        }
    }

    /// Raio de um botão que antes era redondo (hover dos ícones, alças): um
    /// quadrado de cantos suaves no "Menos"; redondo no "Normal" e no "Mais"
    /// (no Estúdio, sempre mais reto).
    pub fn round_btn(self, h: f32) -> f32 {
        match (self.round, self.look) {
            (Corners::Small, Look::Estudio) => h * 0.12,
            (Corners::Small, _) => h * 0.26,
            (_, Look::Estudio) => h * 0.22,
            _ => h / 2.0,
        }
    }

    /// Raio de uma pílula (selos, botões compridos): pílula no "Normal" e no
    /// "Mais", cantos curtos no "Menos".
    pub fn pill(self, h: f32) -> f32 {
        match (self.round, self.look) {
            (_, Look::Estudio) => (h * 0.18).max(1.0),
            (Corners::Small, _) => h * 0.28,
            _ => h / 2.0,
        }
    }

    /// Raio do card, do "A seguir", da letra e do aviso (px lógicos).
    pub fn card_radius(self) -> f32 {
        self.round.radius() * if self.look == Look::Estudio { 0.6 } else { 1.0 }
    }

    /// Raio da capa no formato "Arredondada" (fração do lado).
    pub fn cover(self) -> f32 {
        let f = match self.round {
            Corners::Small => 0.10,
            Corners::Medium => 0.16,
            Corners::Large => 0.24,
        };
        f * self.straight()
    }
}

/// Par do degradê do Estúdio: a cor de destaque puxada para o laranja da marca.
pub fn partner(c: Rgb) -> Rgb {
    let (h, s, v) = rgb_to_hsv(c);
    let toward = hsv((h + 38.0) % 360.0, (s * 1.05).min(1.0), (v * 1.08).min(1.0));
    lerp3(toward, [1.0, 0.56, 0.30], 0.25)
}

/// Preenche com a cor de destaque no jeito do visual: degradê horizontal até o
/// par (Estúdio) ou a cor lisa (os outros).
pub fn fill_accent(cv: &mut Canvas, r: RectF, radius: f32, st: Style, accent: Rgb, a: f32) {
    if st.look == Look::Estudio {
        let b = partner(accent);
        let (x0, w) = (r.x, r.w.max(1.0));
        cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), radius, |x, _| (lerp3(accent, b, ((x - x0) / w).clamp(0.0, 1.0)), a));
    } else {
        cv.fill_round(r, radius, accent, a);
    }
}

/// Acabamento de uma superfície grande (card, "A seguir", letra, aviso) por
/// cima do fundo dela: `grain` é o granulado do Palco (0 = sem).
pub fn finish(cv: &mut Canvas, r: RectF, radius: f32, st: Style, dark: bool, grain: f32, s: f32) {
    let (y0, h) = (r.y, r.h.max(1.0));
    match st.look {
        Look::Palco => {
            cv.grain(r, radius, grain);
            cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), radius, |_, y| ([1.0; 3], if y < y0 + 1.5 * s { 0.10 } else { 0.0 }));
            cv.stroke_round(r, radius, s, if dark { [1.0; 3] } else { [0.0; 3] }, if dark { 0.12 } else { 0.08 });
        }
        Look::Estudio => {
            // Liso: só a linha fina em volta e uma régua clara no alto.
            cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), radius, |_, y| ([1.0; 3], if y < y0 + s { 0.07 } else { 0.0 }));
            cv.stroke_round(r, radius, s, if dark { [1.0; 3] } else { [0.0; 3] }, if dark { 0.11 } else { 0.12 });
        }
        Look::Ilha => {
            // Vidro: reflexo suave que some até o meio, granulado fino e borda clara.
            cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), radius, |_, y| {
                let t = ((y - y0) / (h * 0.45)).clamp(0.0, 1.0);
                ([1.0; 3], (if dark { 0.11 } else { 0.20 }) * (1.0 - t) * (1.0 - t))
            });
            cv.grain(r, radius, grain * 0.7);
            cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), radius, |_, y| ([1.0; 3], if y < y0 + 1.5 * s { 0.16 } else { 0.0 }));
            cv.stroke_round(r, radius, s, [1.0; 3], if dark { 0.24 } else { 0.50 });
        }
    }
}

/// Película do Estúdio sobre o fundo de uma superfície (o grafite que dá a
/// cara dele): quanto cobre depende do fundo (a capa precisa de mais).
pub fn graphite(cv: &mut Canvas, r: RectF, radius: f32, dark: bool, amount: f32) {
    let c: Rgb = if dark { [0.075, 0.078, 0.09] } else { [0.93, 0.93, 0.945] };
    cv.fill_round(r, radius, c, amount);
}

/// Película clara da Ilha (o "fosco" do vidro).
pub fn frost(cv: &mut Canvas, r: RectF, radius: f32, dark: bool) {
    cv.fill_round(r, radius, [1.0; 3], if dark { 0.085 } else { 0.24 });
}

/// Interruptor no jeito do visual (aviso de teclas, configurações). `e`:
/// posição do botão (0 desligado … 1 ligado, já com a curva da animação);
/// `on_c`: trilho ligado; `knob_on`: botão ligado; `fg`: cor do texto.
#[allow(clippy::too_many_arguments)]
pub fn switch(cv: &mut Canvas, r: RectF, e: f32, st: Style, on_c: Rgb, knob_on: Rgb, fg: Rgb, en: f32, hot: f32, s: f32) {
    let e = e.clamp(0.0, 1.0);
    let rad = st.r(6.0, r.h, s);
    let inset = r.h * 0.2;
    let kd = r.h - 2.0 * inset + hot * 1.0 * s;
    let travel = r.w - 2.0 * inset - (r.h - 2.0 * inset);
    let kx = r.x + inset + travel * e - hot * 0.5 * s;
    let knob = RectF::new(kx, r.cy() - kd / 2.0, kd, kd);
    let krad = st.round_btn(kd);
    match st.look {
        Look::Palco => {
            if e < 1.0 {
                cv.stroke_round(r, rad, s, fg, 0.55 * (1.0 - e) * en);
            }
            cv.fill_round(r, rad, on_c, e * en.max(0.5));
            cv.fill_round(knob, krad, lerp3(fg, knob_on, e), (0.8 + 0.2 * e) * en);
        }
        Look::Estudio => {
            if e < 1.0 {
                cv.stroke_round(r, rad, s, fg, 0.38 * (1.0 - e) * en);
            }
            if e > 0.0 {
                fill_accent(cv, r, rad, st, on_c, e * en.max(0.5));
            }
            let k = RectF { w: kd * 0.9, ..knob };
            cv.fill_round(k, krad.min(2.0 * s), lerp3(fg, [1.0; 3], e), (0.5 + 0.5 * e) * en);
        }
        Look::Ilha => {
            cv.fill_round(r, rad, fg, 0.20 * (1.0 - e) * en);
            cv.fill_round(r, rad, on_c, e * en.max(0.5));
            cv.drop_shadow(knob, krad, 4.0 * s, 1.0 * s, 0.35 * en);
            cv.fill_round(knob, krad, [1.0; 3], en.max(0.5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Palco com os cantos "Normal" é o desenho de antes das opções (Mac e Linux).
    #[test]
    fn palco_normal_e_o_de_antes() {
        let st = Style { look: Look::Palco, round: Corners::Medium };
        assert_eq!(st.scale(8.0), 8.0);
        assert_eq!(st.round_btn(30.0), 15.0);
        assert_eq!(st.pill(24.0), 12.0);
        assert_eq!(st.card_radius(), 14.0);
        assert_eq!(st.cover(), 0.16);
    }

    #[test]
    fn menos_e_mais_reto_que_normal() {
        for look in [Look::Palco, Look::Estudio, Look::Ilha] {
            let r = |round| Style { look, round };
            assert!(r(Corners::Small).card_radius() < r(Corners::Medium).card_radius());
            assert!(r(Corners::Small).r(6.0, 30.0, 1.0) < r(Corners::Large).r(6.0, 30.0, 1.0));
            assert!(r(Corners::Small).round_btn(30.0) < 15.0);
        }
    }
}
