//! Animações de abrir e fechar dos cartões (card expandido e "A seguir"): a
//! duração de cada estilo e a "pose" do cartão em cada instante. Só contas e
//! desenho em `Canvas`, sem janelas: o overlay aplica a pose (posição da
//! janela, recorte na barra, escala, opacidade) e a prévia das configurações
//! e os GIFs usam as mesmas funções.
//!
//! Nada aqui roda parado: só enquanto um cartão abre ou fecha.

use crate::config::{AnimSpeed, CardAnim};
use crate::render::{Canvas, Image, RectF, Rgb};
use crate::ui::{bezier, ease_enter, ease_exit, ease_out_cubic};

/// Para que lado o cartão está indo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Open,
    Close,
}

/// Qual cartão anima: o card expandido ou o "A seguir" (que, no estilo de
/// sempre, tem o próprio ritmo, mais calmo).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Card,
    UpNext,
}

/// De onde a escala parte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Center,
    /// Borda do cartão do lado da barra, na altura do widget: o cartão sai
    /// dele.
    Anchor,
}

/// Como o cartão aparece num instante da animação.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// Deslocamento vertical em px lógicos (positivo = para baixo na tela).
    pub dy: f32,
    pub scale: f32,
    pub origin: Origin,
    /// Opacidade do cartão inteiro.
    pub alpha: f32,
    /// Não aparece nada além da borda da barra: o cartão passa por trás dela.
    pub clip: bool,
    /// Brilho em volta do cartão (0..1) e quanto o anel já se afastou (0..1).
    pub glow: f32,
    pub ring: f32,
}

impl Pose {
    /// Parado, aberto.
    pub const REST: Pose = Pose { dy: 0.0, scale: 1.0, origin: Origin::Center, alpha: 1.0, clip: false, glow: 0.0, ring: 0.0 };

    /// Precisa desenhar o cartão escalado ou com brilho (não só mover a janela).
    pub fn transforms(&self) -> bool {
        (self.scale - 1.0).abs() > 1e-4 || self.glow > 0.003
    }
}

/// Onde o cartão está em relação à barra.
#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    /// A barra fica embaixo do cartão (o normal); com a barra no topo, `false`.
    pub bar_below: bool,
    /// Quanto o cartão anda (px lógicos) para sumir inteiro atrás da barra.
    pub travel: f32,
}

impl AnimSpeed {
    /// Multiplica as durações.
    pub fn factor(self) -> f32 {
        match self {
            AnimSpeed::Fast => 0.6,
            AnimSpeed::Normal => 1.0,
            AnimSpeed::Slow => 1.6,
        }
    }
}

/// Duração (s) de abrir ou fechar.
pub fn duration(style: CardAnim, phase: Phase, speed: AnimSpeed, which: Which) -> f32 {
    let (open, close) = match (style, which) {
        // Os valores de antes desta opção existir (o padrão continua igual).
        (CardAnim::Smooth, Which::Card) => (0.28, 0.16),
        (CardAnim::Smooth, Which::UpNext) => (0.5, 0.3),
        (CardAnim::Slide, _) => (0.34, 0.22),
        (CardAnim::Rise | CardAnim::Drop, _) => (0.32, 0.2),
        (CardAnim::Pop, _) => (0.46, 0.18),
        (CardAnim::Zoom, _) => (0.26, 0.16),
        (CardAnim::Fade, _) => (0.22, 0.14),
        (CardAnim::None, _) => (0.0, 0.0),
    };
    speed.factor() * if phase == Phase::Open { open } else { close }
}

/// Aproximação suave de 0 a 1 (`smoothstep`).
fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// "Pulo" de mola de `from` até 1: passa um pouco de 1 (~3%), volta e
/// assenta; termina parado em `t` = 1.
pub fn spring(t: f32, from: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t >= 1.0 {
        return 1.0;
    }
    // Pico em t ≈ 0,3; o resto já é menor que meio pixel num card de 400 px.
    let (k, w) = (5.5, std::f32::consts::PI / 0.3);
    // Mola que parte parada (velocidade zero): ganha corpo, passa e volta.
    let v = 1.0 - (1.0 - from) * (-k * t).exp() * ((w * t).cos() + k / w * (w * t).sin());
    // Termina exatamente em 1 (sem degrau no último quadro).
    let tail = smooth((t - 0.8) / 0.2);
    v + (1.0 - v) * tail
}

/// Saída que acelera (Fluent "accelerate"), mais curta que `ease_exit` no fim.
fn accelerate(t: f32) -> f32 {
    bezier(t, 0.7, 0.0, 1.0, 0.5)
}

/// Pose em `t` (0..1 da fase: 0 = começo de abrir/fechar).
pub fn pose(style: CardAnim, phase: Phase, t: f32, which: Which, ctx: Ctx) -> Pose {
    let t = t.clamp(0.0, 1.0);
    // Sentido "para a barra" na tela (+1: a barra está embaixo).
    let tb = if ctx.bar_below { 1.0 } else { -1.0 };
    let mut p = Pose::REST;
    let open = phase == Phase::Open;
    match style {
        CardAnim::Smooth => {
            let (d_open, d_close) = if which == Which::Card { (24.0, 12.0) } else { (26.0, 26.0) };
            if open {
                let e = ease_enter(t);
                p.dy = tb * (1.0 - e) * d_open;
                p.alpha = e;
            } else {
                let e = ease_exit(t);
                p.dy = tb * e * d_close;
                p.alpha = 1.0 - e;
            }
        }
        CardAnim::Slide => {
            // Opaco o tempo todo: quem esconde é a barra (recorte na borda).
            p.clip = true;
            // Caminho longo: desacelera mais suave que o "decelerate" (que
            // andaria metade dele no primeiro quadro).
            // Fechando, acelera para dentro da barra, sem a arrancada final do
            // "accelerate" (que andaria metade do caminho no último quadro).
            let hidden = if open { 1.0 - bezier(t, 0.2, 0.8, 0.2, 1.0) } else { bezier(t, 0.3, 0.0, 0.8, 0.6) };
            p.dy = tb * hidden * ctx.travel;
        }
        CardAnim::Rise | CardAnim::Drop => {
            // Sobe: entra vindo de baixo e sai por cima (desce: o contrário).
            let up = if style == CardAnim::Rise { -1.0 } else { 1.0 };
            if open {
                let e = bezier(t, 0.2, 0.8, 0.2, 1.0);
                p.dy = -up * (1.0 - e) * 40.0;
                p.alpha = ease_out_cubic(t / 0.7);
            } else {
                let e = accelerate(t);
                p.dy = up * e * 32.0;
                // Some junto com o movimento (não antes de ele começar).
                p.alpha = 1.0 - smooth(t);
            }
        }
        CardAnim::Pop => {
            p.origin = Origin::Anchor;
            if open {
                p.scale = spring(t, 0.8);
                p.alpha = ease_out_cubic(t / 0.28);
                // Brilho: acende rápido até o exagero da mola e apaga devagar.
                p.glow = if t < 0.26 { smooth((t - 0.04) / 0.22) } else { 1.0 - smooth((t - 0.26) / 0.56) };
                p.ring = ease_out_cubic((t - 0.04) / 0.76);
            } else {
                let e = accelerate(t);
                p.scale = 1.0 - 0.14 * e;
                p.alpha = 1.0 - smooth(t);
            }
        }
        CardAnim::Zoom => {
            if open {
                let e = ease_enter(t);
                p.scale = 0.9 + 0.1 * e;
                p.alpha = e;
            } else {
                let e = ease_exit(t);
                p.scale = 1.0 - 0.06 * e;
                p.alpha = 1.0 - e;
            }
        }
        CardAnim::Fade => p.alpha = if open { ease_enter(t) } else { 1.0 - ease_exit(t) },
        CardAnim::None => p.alpha = if open { 1.0 } else { 0.0 },
    }
    // Começo de abrir e fim de fechar: invisível, seja qual for a curva (no
    // deslizar, quem esconde é a barra).
    if !p.clip && ((open && t <= 0.0 && style != CardAnim::None) || (!open && t >= 1.0)) {
        p.alpha = 0.0;
    }
    if open && t >= 1.0 {
        p = Pose::REST;
    }
    p
}

/// Abrir/fechar de um cartão: quanto está aberto (0..1) e qual curva está em
/// uso. Se mandarem fechar no meio da abertura (ou o contrário), o cartão
/// volta pela mesma curva, de onde está: sem pulo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Toggle {
    /// 0 = fechado, 1 = aberto.
    pub v: f32,
    pub phase: Phase,
}

impl Default for Toggle {
    fn default() -> Self {
        Self { v: 0.0, phase: Phase::Open }
    }
}

impl Toggle {
    /// Avança `dt` segundos rumo a aberto (`open`) ou fechado.
    pub fn step(&mut self, open: bool, dt: f32, style: CardAnim, speed: AnimSpeed, which: Which) {
        // Só escolhe a curva parado; no meio, inverte pela curva atual.
        if self.v <= 0.0 && open {
            self.phase = Phase::Open;
        } else if self.v >= 1.0 && !open {
            self.phase = Phase::Close;
        }
        let target = if open { 1.0 } else { 0.0 };
        // Saindo do repouso, o primeiro passo vale no máximo um quadro: o
        // anterior pode ter sido há muito (parado, o app desenha a 4 fps) e a
        // animação começaria pela metade.
        let dt = if self.v <= 0.0 || self.v >= 1.0 { dt.min(1.0 / 60.0) } else { dt };
        let dur = duration(style, self.phase, speed, which);
        if dur <= 0.0 {
            self.v = target;
            return;
        }
        let k = dt.max(0.0) / dur;
        self.v += (target - self.v).clamp(-k, k);
    }

    pub fn moving(&self) -> bool {
        self.v > 0.0 && self.v < 1.0
    }

    pub fn pose(&self, style: CardAnim, which: Which, ctx: Ctx) -> Pose {
        match self.phase {
            Phase::Open => pose(style, Phase::Open, self.v, which, ctx),
            Phase::Close => pose(style, Phase::Close, 1.0 - self.v, which, ctx),
        }
    }
}

/// Pose de um cartão que entra, fica `total` segundos e sai sozinho ("A
/// seguir"), `t` segundos depois de aparecer.
pub fn timed_pose(style: CardAnim, speed: AnimSpeed, which: Which, t: f32, total: f32, ctx: Ctx) -> Pose {
    let open = duration(style, Phase::Open, speed, which);
    let close = duration(style, Phase::Close, speed, which).min(total * 0.4);
    if t >= total {
        pose(style, Phase::Close, 1.0, which, ctx)
    } else if t < open {
        pose(style, Phase::Open, t / open, which, ctx)
    } else if t > total - close {
        pose(style, Phase::Close, (t - (total - close)) / close.max(1e-3), which, ctx)
    } else {
        Pose::REST
    }
}

/// O cartão dentro da imagem dele (a imagem tem a margem da sombra em volta).
#[derive(Clone, Copy, Debug)]
pub struct Stage {
    /// Corpo do cartão (sem a sombra), em px da imagem, e o raio dos cantos.
    pub body: RectF,
    pub radius: f32,
    /// x (px da imagem) do ponto de onde o cartão cresce no estilo "Estourar".
    pub anchor_x: f32,
    pub bar_below: bool,
    /// Cor do brilho.
    pub accent: Rgb,
    pub s: f32,
}

impl Stage {
    /// Ponto fixo da escala, em px da imagem.
    fn origin(&self, o: Origin) -> (f32, f32) {
        match o {
            Origin::Center => (self.body.cx(), self.body.cy()),
            Origin::Anchor => (self.anchor_x.clamp(self.body.x, self.body.right()), if self.bar_below { self.body.bottom() } else { self.body.y }),
        }
    }

    /// Onde a imagem inteira vai parar com a escala da pose (px da imagem).
    pub fn scaled(&self, w: f32, h: f32, p: &Pose) -> RectF {
        let (ox, oy) = self.origin(p.origin);
        RectF::new(ox * (1.0 - p.scale), oy * (1.0 - p.scale), w * p.scale, h * p.scale)
    }
}

/// Desenha `img` (o cartão parado, já pronto) com a pose: em (`x`, `y`) mais
/// o deslocamento, escalado a partir da origem, com a opacidade e o brilho;
/// com `clip`, só do lado de cá de `bar_edge` (y da borda da barra).
pub fn draw_posed(cv: &mut Canvas, img: &Image, x: f32, y: f32, st: &Stage, p: &Pose, bar_edge: Option<f32>) {
    if p.alpha <= 0.002 {
        return;
    }
    let old = cv.clip_rect();
    if let (true, Some(edge)) = (p.clip, bar_edge) {
        let r = if st.bar_below { RectF::new(old.x, old.y, old.w, edge - old.y) } else { RectF::new(old.x, edge, old.w, old.bottom() - edge) };
        cv.set_clip(r.intersect(old));
    }
    let dy = p.dy * st.s;
    let r = st.scaled(img.w as f32, img.h as f32, p).offset(x, y + dy);
    if p.glow > 0.003 {
        let k = r.w / img.w as f32;
        let body = RectF::new(r.x + st.body.x * k, r.y + st.body.y * k, st.body.w * k, st.body.h * k);
        draw_burst(cv, body, st.radius * k, p.glow * p.alpha, p.ring, st.accent, st.s);
    }
    cv.draw_image(r, img, 0.0, p.alpha);
    cv.set_clip(old);
}

/// Brilho do "Estourar": luz suave em volta do cartão, um anel que se afasta
/// e apaga, e faíscas que saem dos lados. Tudo dentro da margem da sombra.
pub fn draw_burst(cv: &mut Canvas, body: RectF, radius: f32, glow: f32, ring: f32, c: Rgb, s: f32) {
    if glow <= 0.003 {
        return;
    }
    cv.glow(body, radius, 18.0 * s, 0.0, c, 0.45 * glow);
    let d = ring * 14.0 * s;
    cv.stroke_round(body.inset(-d), radius + d, (2.4 - 1.4 * ring) * s, c, 0.75 * glow * (1.0 - ring * 0.6));
    // Faíscas: pontos que saem da borda (sempre nos mesmos lugares).
    let (cx, cy, hx, hy) = (body.cx(), body.cy(), body.w / 2.0, body.h / 2.0);
    for i in 0..14 {
        let a = i as f32 * std::f32::consts::TAU / 14.0 + 0.35 + (i % 3) as f32 * 0.09;
        let (dx, dy) = (a.cos(), a.sin());
        // Ponto da borda do retângulo nesse sentido.
        let k = (hx / dx.abs().max(1e-3)).min(hy / dy.abs().max(1e-3));
        let out = (6.0 + 16.0 * ring + (i % 2) as f32 * 4.0 * ring) * s;
        let (px, py) = (cx + dx * (k + out), cy + dy * (k + out));
        let rr = (2.2 - 1.2 * ring) * s;
        cv.fill_circle(px, py, rr, c, glow * (1.0 - ring * 0.7));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTX: Ctx = Ctx { bar_below: true, travel: 220.0 };
    const ALL: [CardAnim; 8] = [CardAnim::Smooth, CardAnim::Slide, CardAnim::Rise, CardAnim::Drop, CardAnim::Pop, CardAnim::Zoom, CardAnim::Fade, CardAnim::None];

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    /// O estilo padrão é exatamente a animação de antes desta opção.
    #[test]
    fn padrao_igual_ao_de_antes() {
        for i in 0..=20 {
            let a = i as f32 / 20.0;
            // Antes: abre com 24 px e "decelerate", fecha com 12 px e "accelerate".
            let e = ease_enter(a);
            let p = pose(CardAnim::Smooth, Phase::Open, a, Which::Card, CTX);
            if a < 1.0 {
                assert!(close(p.dy, (1.0 - e) * 24.0) && (a == 0.0 || close(p.alpha, e)), "abrindo em {a}: {p:?}");
            }
            let e = 1.0 - ease_exit(1.0 - a);
            let p = pose(CardAnim::Smooth, Phase::Close, 1.0 - a, Which::Card, CTX);
            if a > 0.0 {
                assert!(close(p.dy, (1.0 - e) * 12.0) && close(p.alpha, e), "fechando em {a}: {p:?}");
            }
        }
        assert_eq!(duration(CardAnim::Smooth, Phase::Open, AnimSpeed::Normal, Which::Card), 0.28);
        assert_eq!(duration(CardAnim::Smooth, Phase::Close, AnimSpeed::Normal, Which::Card), 0.16);
        // "A seguir": 26 px, 0,5 s para entrar e 0,3 s para sair.
        let p = pose(CardAnim::Smooth, Phase::Open, 0.5, Which::UpNext, CTX);
        assert!(close(p.dy, (1.0 - ease_enter(0.5)) * 26.0));
        assert_eq!(duration(CardAnim::Smooth, Phase::Open, AnimSpeed::Normal, Which::UpNext), 0.5);
        assert_eq!(duration(CardAnim::Smooth, Phase::Close, AnimSpeed::Normal, Which::UpNext), 0.3);
    }

    /// Todo estilo começa invisível (ou escondido atrás da barra), termina
    /// parado no lugar e some ao fechar.
    #[test]
    fn comeca_e_termina_no_lugar() {
        for style in ALL {
            for which in [Which::Card, Which::UpNext] {
                let start = pose(style, Phase::Open, 0.0, which, CTX);
                let hidden = start.alpha <= 0.0 || (start.clip && start.dy >= CTX.travel - 1e-3);
                assert!(hidden || style == CardAnim::None, "{style:?} começa visível: {start:?}");
                assert_eq!(pose(style, Phase::Open, 1.0, which, CTX), Pose::REST, "{style:?}");
                let end = pose(style, Phase::Close, 1.0, which, CTX);
                assert!(end.alpha <= 0.0 || (end.clip && end.dy >= CTX.travel - 1e-3), "{style:?} termina visível: {end:?}");
                // Fechar parte de onde abrir terminou.
                let c0 = pose(style, Phase::Close, 0.0, which, CTX);
                assert!(close(c0.dy, 0.0) && close(c0.scale, 1.0) && close(c0.alpha, 1.0) || style == CardAnim::None, "{style:?}: {c0:?}");
            }
        }
    }

    /// Sem saltos: de um quadro (60 fps) para o outro nada anda demais.
    #[test]
    fn curvas_continuas() {
        for style in ALL.into_iter().filter(|s| *s != CardAnim::None) {
            for phase in [Phase::Open, Phase::Close] {
                let dur = duration(style, phase, AnimSpeed::Fast, Which::Card);
                let n = (dur * 60.0).ceil() as usize;
                let mut last = pose(style, phase, 0.0, Which::Card, CTX);
                for i in 1..=n {
                    let p = pose(style, phase, i as f32 / n as f32, Which::Card, CTX);
                    // Saindo de invisível, o primeiro quadro pode estar em qualquer lugar.
                    if last.alpha < 0.01 {
                        last = p;
                        continue;
                    }
                    // O deslizar da barra anda mais por quadro: até 35% do caminho.
                    let max_dy = if style == CardAnim::Slide { CTX.travel * 0.35 } else { 20.0 };
                    assert!((p.dy - last.dy).abs() <= max_dy, "{style:?} {phase:?} pulou em {i}: {last:?} -> {p:?}");
                    assert!((p.scale - last.scale).abs() <= 0.08, "{style:?} {phase:?} escala pulou em {i}");
                    // Opacidade: o último quadro de fechar pode zerar o que sobrou.
                    assert!((p.alpha - last.alpha).abs() <= 0.65, "{style:?} {phase:?} opacidade pulou em {i}");
                    last = p;
                }
            }
        }
    }

    /// Deslizar: entra pela barra e sai por ela; com a barra no topo, o
    /// contrário. Subir e descer têm sentidos próprios, com a barra onde for.
    #[test]
    fn sentidos() {
        let top = Ctx { bar_below: false, ..CTX };
        let mid = |style, phase, ctx| pose(style, phase, 0.5, Which::Card, ctx).dy;
        assert!(mid(CardAnim::Slide, Phase::Open, CTX) > 0.0 && mid(CardAnim::Slide, Phase::Close, CTX) > 0.0);
        assert!(mid(CardAnim::Slide, Phase::Open, top) < 0.0 && mid(CardAnim::Slide, Phase::Close, top) < 0.0);
        assert_eq!(pose(CardAnim::Slide, Phase::Open, 0.0, Which::Card, CTX).dy, CTX.travel);
        for ctx in [CTX, top] {
            // Sobe: vem de baixo (dy > 0) e sai por cima (dy < 0).
            assert!(mid(CardAnim::Rise, Phase::Open, ctx) > 0.0 && mid(CardAnim::Rise, Phase::Close, ctx) < 0.0);
            assert!(mid(CardAnim::Drop, Phase::Open, ctx) < 0.0 && mid(CardAnim::Drop, Phase::Close, ctx) > 0.0);
        }
    }

    /// Estourar: passa um pouco do tamanho (até ~4%), volta e assenta; o
    /// brilho acende e apaga dentro da abertura.
    #[test]
    fn estourar_passa_e_volta() {
        let scales: Vec<f32> = (0..=100).map(|i| pose(CardAnim::Pop, Phase::Open, i as f32 / 100.0, Which::Card, CTX).scale).collect();
        let max = scales.iter().cloned().fold(0.0, f32::max);
        assert!(max > 1.02 && max < 1.045, "exagero {max}");
        assert!(scales[0] < 0.85 && scales[100] == 1.0);
        let peak = scales.iter().position(|v| *v == max).unwrap();
        assert!((20..45).contains(&peak), "pico em {peak}%");
        assert!(scales[85..].iter().all(|v| (v - 1.0).abs() < 0.004), "não assentou");
        let glow: Vec<f32> = (0..=100).map(|i| pose(CardAnim::Pop, Phase::Open, i as f32 / 100.0, Which::Card, CTX).glow).collect();
        assert!(glow[0] == 0.0 && glow[100] == 0.0 && glow.iter().cloned().fold(0.0, f32::max) > 0.95);
        assert!(spring(0.0, 0.8) < 0.81 && spring(1.0, 0.8) == 1.0);
    }

    /// Inverter no meio não pula: volta pela mesma curva, de onde está.
    #[test]
    fn inverter_no_meio_nao_pula() {
        for style in ALL {
            let mut t = Toggle::default();
            let mut last = t.pose(style, Which::Card, CTX);
            let mut frames = 0;
            // Abre por 0,1 s, manda fechar, abre de novo antes de terminar.
            for (open, n) in [(true, 6), (false, 4), (true, 40), (false, 40)] {
                for _ in 0..n {
                    t.step(open, 1.0 / 60.0, style, AnimSpeed::Normal, Which::Card);
                    let p = t.pose(style, Which::Card, CTX);
                    if style != CardAnim::None && frames > 0 {
                        assert!((p.dy - last.dy).abs() <= CTX.travel * 0.35 && (p.scale - last.scale).abs() <= 0.08, "{style:?} pulou: {last:?} -> {p:?}");
                    }
                    last = p;
                    frames += 1;
                }
            }
            assert_eq!(t.v, 0.0, "{style:?} não fechou");
        }
        // Saindo do repouso depois de muito tempo parado: só um quadro.
        let mut t = Toggle::default();
        t.step(true, 0.1, CardAnim::Slide, AnimSpeed::Normal, Which::Card);
        assert!(t.v > 0.0 && t.v <= 1.0 / 60.0 / 0.34 + 1e-6, "{}", t.v);
        // Parado, a próxima abertura usa a curva de abrir de novo.
        let mut t = Toggle { v: 0.0, phase: Phase::Close };
        t.step(true, 0.01, CardAnim::Pop, AnimSpeed::Normal, Which::Card);
        assert_eq!(t.phase, Phase::Open);
    }

    #[test]
    fn velocidade_e_nenhuma() {
        let d = |sp| duration(CardAnim::Zoom, Phase::Open, sp, Which::Card);
        assert!(d(AnimSpeed::Fast) < d(AnimSpeed::Normal) && d(AnimSpeed::Normal) < d(AnimSpeed::Slow));
        let mut t = Toggle::default();
        t.step(true, 0.001, CardAnim::None, AnimSpeed::Slow, Which::Card);
        assert_eq!(t.v, 1.0);
        t.step(false, 0.001, CardAnim::None, AnimSpeed::Slow, Which::Card);
        assert_eq!(t.v, 0.0);
    }

    /// "A seguir": entra, fica parado e sai dentro do tempo dele.
    #[test]
    fn a_seguir_no_tempo() {
        for style in ALL {
            let total = 5.5;
            assert_eq!(timed_pose(style, AnimSpeed::Slow, Which::UpNext, 2.0, total, CTX), Pose::REST, "{style:?}");
            let end = timed_pose(style, AnimSpeed::Slow, Which::UpNext, total, total, CTX);
            assert!(end.alpha < 0.01 || end.clip, "{style:?}: {end:?}");
        }
    }
}
