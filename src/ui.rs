//! Desenho dos componentes visuais (funções puras, sem estado de janela):
//! widget da barra, card expandido, cartão "A seguir" e aviso de Caps/Num Lock.
//! Usado pelas janelas reais (overlay.rs) e pelas pré-visualizações ao vivo da
//! tela de configurações (settings.rs).
//!
//! Referências de design: Fluent 2 (raios 4/8, elevação com sombra suave,
//! Segoe UI Variable) e cards de "now playing" populares no Dribbble (capa
//! desfocada como fundo, cor extraída da capa, vidro, play circular).

use crate::config::{CardLayout, CardShadow, CardStyle, Config, OsdIcon, OsdMaterial, OsdShape, OsdSize, SecondLine, Shape, ShuffleRepeat, Size3, TaskbarPlay, ThemeMode, WidgetShape, WidgetTint};
use crate::render::*;
use crate::shared::MediaInfo;
use crate::text::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Margem (lógica) em volta dos cards para caber a sombra.
pub const SHADOW_MARGIN: f32 = 26.0;
/// Desfoque (px físicos) do fundo acrílico do card e do "A seguir", conforme
/// a opção "Desfoque do acrílico" (0% = 6 px lógicos, 50% = 30 px, o de
/// sempre, 100% = 54 px).
pub fn glass_blur_px(cfg: &Config, s: f32) -> f32 {
    (6.0 + cfg.glass_blur as f32 * 0.48) * s
}
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
    /// Atalho para abrir um app de música (índice na lista).
    Launch(u8),
    /// Setinha que abre/fecha o mixer de volume do card.
    MixToggle,
    /// Slider e mudo do app `i` no mixer.
    MixVol(u8),
    MixMute(u8),
    /// Saída de áudio no cabeçalho do mixer (abre a lista para trocar).
    MixDevice,
    /// Botão do card que mostra e esconde a letra na tela (Windows).
    Lyrics,
    /// Selo do app de origem na capa do card (a dica diz o nome do app).
    Source,
}

/// Ícone da letra (o mesmo da aba Letras e do menu da bandeja).
const G_LYRICS: &str = "\u{E7F0}";

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
    /// Ícones de controle desenhados como formas preenchidas (Windows, opção
    /// "Estilo dos ícones"); desligado, usa os glifos da fonte (o Mac sempre).
    pub vector: bool,
    /// Botão play com brilho na cor de destaque em vez da sombra preta (Windows).
    pub glow: bool,
    /// Posição x do mouse enquanto um atalho é arrastado (`drag` = `Launch`).
    pub drag_x: f32,
    /// Volume como a barra mostra agora (sobe e desce suave até o valor);
    /// `None`: o próprio volume.
    pub vol_fill: Option<f32>,
    /// Opacidade da dica (tooltip) do botão sob o mouse: aparece depois de
    /// um instante parado em cima, como as dicas do Windows.
    pub tip: f32,
}

impl Interact {
    /// O mesmo estado com o visual do Windows escolhido nas opções.
    pub(crate) fn styled(self, cfg: &Config) -> Interact {
        Interact { vector: cfg.icon_style == crate::config::IconStyle::Filled, glow: true, ..self }
    }
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

/// Ícone com pose (parado = desenho normal, alinhado aos pixels). `vec_em`:
/// tamanho do ícone em vetor (px), quando ele deve ser desenhado como forma.
pub(crate) fn glyph_pose(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, g: &str, font: Font, c: Rgb, a: f32, p: Pose, vec_em: Option<f32>) {
    let (w, h) = (r.w.round().max(1.0), r.h.round().max(1.0));
    let m = match vec_em.zip(vicon_of(g)) {
        Some((em, v)) => vector_mask(v, em, w as i32, h as i32),
        None if p == Pose::REST => return glyph(cv, tr, r, g, font, c, a),
        None => tr.mask(g, font, w as i32, h as i32, Align::Center),
    };
    if p == Pose::REST {
        cv.blit_mask(r.x.round() as i32, r.y.round() as i32, &m, c, a);
        return;
    }
    // O símbolo está centrado na caixa: gira e escala em volta do centro dela.
    cv.blit_mask_xform(r.x.round() + p.dx, r.y.round(), &m, w / 2.0, h / 2.0, p.scale, p.rot, c, a * p.alpha);
}

/// Tamanho (px) do ícone em vetor equivalente a uma fonte de ícones, ou
/// `None` se o quadro usa os glifos da fonte.
fn vec_em(it: &Interact, font: Font, s: f32) -> Option<f32> {
    let px = match font {
        Font::IconSmall => 13.0,
        Font::IconMd => 16.0,
        Font::IconLg => 18.0,
        _ => 15.0,
    };
    it.vector.then_some(px * s)
}

/// Ícone de um botão de controle (pular, play só ícone, volume), animado.
pub(crate) fn ctrl_glyph(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, g: &str, font: Font, c: Rgb, a: f32, it: &Interact, h: Hit, s: f32) {
    glyph_pose(cv, tr, r, g, font, c, a, pose(it, h, s), vec_em(it, font, s));
}

/// Play↔pausa animado: o ícone antigo encolhe e some, o novo cresce com um
/// pequeno salto. Sem animação, desenha só o atual.
fn play_glyph(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, playing: bool, font: Font, c: Rgb, a: f32, it: &Interact, s: f32) {
    let (now_g, old_g) = if playing { (G_PAUSE, G_PLAY) } else { (G_PLAY, G_PAUSE) };
    let base = pose(it, Hit::Play, s);
    let em = vec_em(it, font, s);
    match it.fx.progress(Hit::Play) {
        Some(u) => {
            let out = (u / 0.4).min(1.0);
            if out < 1.0 {
                let po = Pose { scale: base.scale * (1.0 - 0.5 * ease_out_cubic(out)), alpha: 1.0 - out, ..base };
                glyph_pose(cv, tr, r, old_g, font, c, a, po, em);
            }
            let e = ease_out_back(((u - 0.12) / 0.88).clamp(0.0, 1.0));
            let pn = Pose { scale: base.scale * (0.45 + 0.55 * e), alpha: ((u - 0.12) / 0.3).clamp(0.0, 1.0), ..base };
            glyph_pose(cv, tr, r, now_g, font, c, a, pn, em);
        }
        None => glyph_pose(cv, tr, r, now_g, font, c, a, base, em),
    }
}

// ---------------------------------------------------------------- ícones em vetor

/// Ícones de controle desenhados como formas cheias e arredondadas (no lugar
/// dos glifos finos de contorno do Segoe Fluent), no espírito dos símbolos
/// preenchidos do macOS. Cada um vira uma máscara guardada por tamanho.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum VIcon {
    Play,
    Pause,
    Prev,
    Next,
    Shuffle,
    RepeatAll,
    RepeatOne,
    Mute,
    Vol1,
    Vol2,
    Vol3,
}

fn vicon_of(g: &str) -> Option<VIcon> {
    let all = [
        (G_PLAY, VIcon::Play),
        (G_PAUSE, VIcon::Pause),
        (G_PREV, VIcon::Prev),
        (G_NEXT, VIcon::Next),
        (G_SHUFFLE, VIcon::Shuffle),
        (G_REPEAT_ALL, VIcon::RepeatAll),
        (G_REPEAT_ONE, VIcon::RepeatOne),
        (G_MUTE, VIcon::Mute),
        (G_VOL1, VIcon::Vol1),
        (G_VOL2, VIcon::Vol2),
        (G_VOL3, VIcon::Vol3),
    ];
    all.iter().find(|(s, _)| *s == g).map(|(_, v)| *v)
}

type Pt = (f32, f32);

/// Distância de `p` ao segmento `a`–`b`.
fn sd_seg(p: Pt, a: Pt, b: Pt) -> f32 {
    let (pa, ba) = ((p.0 - a.0, p.1 - a.1), (b.0 - a.0, b.1 - a.1));
    let h = ((pa.0 * ba.0 + pa.1 * ba.1) / (ba.0 * ba.0 + ba.1 * ba.1).max(1e-9)).clamp(0.0, 1.0);
    (pa.0 - ba.0 * h).hypot(pa.1 - ba.1 * h)
}

/// Distância com sinal a um triângulo (negativa dentro).
fn sd_tri(p: Pt, a: Pt, b: Pt, c: Pt) -> f32 {
    let e = [(b.0 - a.0, b.1 - a.1), (c.0 - b.0, c.1 - b.1), (a.0 - c.0, a.1 - c.1)];
    let v = [(p.0 - a.0, p.1 - a.1), (p.0 - b.0, p.1 - b.1), (p.0 - c.0, p.1 - c.1)];
    let s = (e[0].0 * e[2].1 - e[0].1 * e[2].0).signum();
    let mut d = (f32::MAX, f32::MAX);
    for k in 0..3 {
        let (ek, vk) = (e[k], v[k]);
        let t = ((vk.0 * ek.0 + vk.1 * ek.1) / (ek.0 * ek.0 + ek.1 * ek.1)).clamp(0.0, 1.0);
        let q = (vk.0 - ek.0 * t, vk.1 - ek.1 * t);
        d.0 = d.0.min(q.0 * q.0 + q.1 * q.1);
        d.1 = d.1.min(s * (vk.0 * ek.1 - vk.1 * ek.0));
    }
    -d.0.sqrt() * d.1.signum()
}

/// Distância com sinal a um retângulo arredondado (centro, meia largura/altura, raio).
fn sd_box(p: Pt, c: Pt, half: Pt, r: f32) -> f32 {
    rrect_sd((p.0 - c.0).abs() - (half.0 - r), (p.1 - c.1).abs() - (half.1 - r), r)
}

/// Distância a uma linha (polilinha) com meia espessura `hw`.
fn sd_path(p: Pt, pts: &[Pt], hw: f32) -> f32 {
    pts.windows(2).map(|w| sd_seg(p, w[0], w[1])).fold(f32::MAX, f32::min) - hw
}

/// Pontos de um arco (centro, raio, ângulos em graus; 0° = direita, sentido horário na tela).
fn arc_pts(c: Pt, r: f32, a0: f32, a1: f32, n: usize) -> Vec<Pt> {
    (0..=n).map(|i| (a0 + (a1 - a0) * i as f32 / n as f32).to_radians()).map(|t| (c.0 + r * t.cos(), c.1 + r * t.sin())).collect()
}

/// Pontos de uma curva de Bézier cúbica.
fn bezier_pts(p0: Pt, p1: Pt, p2: Pt, p3: Pt, n: usize) -> Vec<Pt> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1)
        })
        .collect()
}

/// Forma do ícone: distância com sinal (em "em", com o centro em 0,0 e y para baixo).
struct Shape2 {
    parts: Vec<Box<dyn Fn(Pt) -> f32>>,
}

impl Shape2 {
    fn sd(&self, p: Pt) -> f32 {
        self.parts.iter().map(|f| f(p)).fold(f32::MAX, f32::min)
    }
}

fn vicon_shape(v: VIcon) -> Shape2 {
    let mut parts: Vec<Box<dyn Fn(Pt) -> f32>> = Vec::new();
    let tri = |a: Pt, b: Pt, c: Pt, r: f32| -> Box<dyn Fn(Pt) -> f32> { Box::new(move |p| sd_tri(p, a, b, c) - r) };
    let path = |pts: Vec<Pt>, hw: f32| -> Box<dyn Fn(Pt) -> f32> { Box::new(move |p| sd_path(p, &pts, hw)) };
    // Alto-falante (volume) deslocado para a esquerda; as ondas saem dele.
    let speaker = |parts: &mut Vec<Box<dyn Fn(Pt) -> f32>>, dx: f32| {
        parts.push(Box::new(move |p| sd_box(p, (-0.36 + dx, 0.0), (0.10, 0.15), 0.04)));
        parts.push(tri((-0.31 + dx, -0.13), (-0.08 + dx, -0.37), (-0.08 + dx, 0.37), 0.035));
        parts.push(tri((-0.31 + dx, -0.13), (-0.08 + dx, 0.37), (-0.31 + dx, 0.13), 0.035));
    };
    match v {
        VIcon::Play => parts.push(tri((-0.29, -0.38), (-0.29, 0.38), (0.39, 0.0), 0.07)),
        VIcon::Pause => {
            for cx in [-0.19, 0.19] {
                parts.push(Box::new(move |p| sd_box(p, (cx, 0.0), (0.12, 0.40), 0.07)));
            }
        }
        VIcon::Next | VIcon::Prev => {
            let k = if v == VIcon::Next { 1.0 } else { -1.0 };
            for x0 in [-0.44, 0.0] {
                parts.push(tri((k * x0, -0.31), (k * x0, 0.31), (k * (x0 + 0.42), 0.0), 0.05));
            }
        }
        VIcon::Shuffle => {
            let hw = 0.065;
            for sy in [1.0f32, -1.0] {
                let pts = bezier_pts((-0.46, 0.27 * sy), (-0.06, 0.27 * sy), (0.02, -0.27 * sy), (0.30, -0.27 * sy), 14);
                parts.push(path(pts, hw));
                parts.push(tri((0.27, -0.27 * sy - 0.15), (0.27, -0.27 * sy + 0.15), (0.49, -0.27 * sy), 0.03));
            }
        }
        VIcon::RepeatAll | VIcon::RepeatOne => {
            let hw = 0.07;
            for k in [1.0f32, -1.0] {
                // Topo (k = 1): sobe pela esquerda, faz a curva e segue para a direita com a seta.
                let mut pts = vec![(-0.46 * k, 0.15 * k)];
                pts.extend(arc_pts((-0.26 * k, -0.09 * k), 0.20, if k > 0.0 { 180.0 } else { 0.0 }, if k > 0.0 { 270.0 } else { 90.0 }, 8));
                pts.push((0.30 * k, -0.29 * k));
                parts.push(path(pts, hw));
                parts.push(tri((0.28 * k, -0.29 * k - 0.17), (0.28 * k, -0.29 * k + 0.17), (0.52 * k, -0.29 * k), 0.03));
            }
            if v == VIcon::RepeatOne {
                parts.push(path(vec![(-0.07, -0.06), (0.02, -0.13), (0.02, 0.13)], 0.055));
            }
        }
        VIcon::Mute => {
            speaker(&mut parts, 0.0);
            parts.push(path(vec![(0.10, -0.13), (0.36, 0.13)], 0.055));
            parts.push(path(vec![(0.10, 0.13), (0.36, -0.13)], 0.055));
        }
        VIcon::Vol1 | VIcon::Vol2 | VIcon::Vol3 => {
            let n = match v {
                VIcon::Vol1 => 1,
                VIcon::Vol2 => 2,
                _ => 3,
            };
            let dx = -0.04 * (n - 1) as f32;
            speaker(&mut parts, dx);
            for i in 0..n {
                let r = 0.17 + 0.15 * i as f32;
                parts.push(path(arc_pts((-0.06 + dx, 0.0), r, -48.0, 48.0, 10), 0.05));
            }
        }
    }
    Shape2 { parts }
}

/// Máscaras dos ícones em vetor já feitas: (ícone, tamanho, caixa).
type VMaskCache = std::collections::HashMap<(VIcon, u32, i32, i32), std::rc::Rc<Mask>>;

thread_local! {
    static VMASKS: RefCell<VMaskCache> = RefCell::new(std::collections::HashMap::new());
}

/// Máscara do ícone em vetor do glifo `g` (`G_*`), se houver desenho para ele.
/// No Linux não há fonte de ícones no sistema: o `TextRenderer` desenha assim.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn vector_icon(g: &str, em: f32, w: i32, h: i32) -> Option<std::rc::Rc<Mask>> {
    vicon_of(g).map(|v| vector_mask(v, em, w, h))
}

/// Máscara do ícone `v` com `em` px, centrado numa caixa `w`×`h` (guardada).
fn vector_mask(v: VIcon, em: f32, w: i32, h: i32) -> std::rc::Rc<Mask> {
    let key = (v, em.to_bits(), w, h);
    if let Some(m) = VMASKS.with(|c| c.borrow().get(&key).cloned()) {
        return m;
    }
    let shape = vicon_shape(v);
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let mut cov = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            // Cobertura pela distância à borda (em pixels), com 4 amostras por
            // pixel: bordas e pontas finas ficam lisas mesmo nos ícones pequenos.
            let mut acc = 0.0;
            for (ox, oy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let p = ((x as f32 + ox - cx) / em, (y as f32 + oy - cy) / em);
                acc += (0.5 - shape.sd(p) * em * 2.0).clamp(0.0, 1.0);
            }
            cov[y * w + x] = (acc / 4.0 * 255.0 + 0.5) as u8;
        }
    }
    let m = std::rc::Rc::new(Mask { w, h, cov });
    VMASKS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 64 {
            c.clear();
        }
        c.insert(key, m.clone());
    });
    m
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

/// Cor de destaque escolhida ("Cor de destaque": capa, Windows ou uma cor
/// personalizada) para um fundo escuro ou claro. A da capa e a do Windows
/// ganham contraste; a personalizada vale como foi escolhida.
pub fn chosen_accent(cfg: &Config, colors: &Colors, media: &MediaInfo, dark: bool) -> Rgb {
    use crate::config::AccentSource;
    match (cfg.accent_source, &media.art) {
        (AccentSource::Album, Some(a)) => readable(a.vibrant, dark),
        (AccentSource::Custom, _) => crate::config::parse_hex(&cfg.accent_rgb).unwrap_or(colors.accent),
        _ => {
            if dark {
                colors.accent_light
            } else {
                colors.accent
            }
        }
    }
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
    let accent = chosen_accent(cfg, colors, media, dark);
    Theme { fg, accent, on_accent: on_color(accent), dark }
}

/// Cores do widget na barra. O texto segue o fundo de verdade (a barra com o
/// fundo do widget por cima): com um fundo forte e claro, o texto fica escuro
/// mesmo na barra escura. O destaque segue a capa ou o Windows.
pub fn widget_theme(cfg: &Config, colors: &Colors, media: &MediaInfo) -> Theme {
    let dark = luma(widget_bg_mean(cfg, colors, media)) < 0.47;
    let fg = if dark { [1.0; 3] } else { [0.10, 0.10, 0.10] };
    let accent = chosen_accent(cfg, colors, media, dark);
    Theme { fg, accent, on_accent: on_color(accent), dark }
}

/// Cores dos controles (aleatório, anterior, play, próximo, repetir) conforme
/// a opção "Cor dos controles". `fg`: os ícones; `accent`: o fundo do play
/// preenchido e os botões ligados; `on_accent`: o ícone sobre o play.
pub fn controls_theme(choice: crate::config::ControlColor, custom: &str, th: &Theme, m: &MediaInfo) -> Theme {
    use crate::config::ControlColor;
    let one = |c: Rgb| Theme { fg: c, accent: c, on_accent: on_color(c), dark: th.dark };
    match choice {
        // Branco no fundo escuro; no fundo claro (barra clara, gradiente pastel)
        // fica escuro, senão some.
        ControlColor::White => one(th.fg),
        ControlColor::WhiteAccent => *th,
        ControlColor::Album => one(m.art.as_ref().map(|a| readable(a.vibrant, th.dark)).unwrap_or(th.accent)),
        ControlColor::Accent => one(th.accent),
        ControlColor::Custom => one(crate::config::parse_hex(custom).unwrap_or(th.fg)),
    }
}

/// Botões laterais (aleatório, anterior, próximo, repetir): cor e força.
/// Ficam iguais estando ou não disponíveis (pedido do usuário: apagados,
/// pareciam fracos ao lado do botão do meio); aleatório/repetir ligados
/// ganham o pontinho embaixo.
#[derive(Clone, Copy)]
struct Sides {
    c: Rgb,
    a: f32,
    it: Interact,
    s: f32,
}

impl Sides {
    fn of(choice: crate::config::SideColor, alpha: i32, mid: Rgb, th: &Theme, m: &MediaInfo, it: Interact, s: f32) -> Self {
        use crate::config::SideColor;
        let c = match choice {
            // A cor do botão do meio (o fundo do play preenchido ou o ícone).
            SideColor::SameAsMiddle => mid,
            SideColor::White => th.fg,
            SideColor::Album => m.art.as_ref().map(|a| readable(a.vibrant, th.dark)).unwrap_or(th.accent),
            SideColor::Accent => th.accent,
        };
        // Sobre fundo claro, o branco do play fica escuro (como o próprio play).
        let c = if !th.dark && luma(c) > 0.85 { th.fg } else { c };
        Sides { c, a: (alpha as f32 / 100.0).clamp(0.2, 1.0), it, s }
    }

    fn draw(&self, cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, g: &str, font: Font, h: Hit) {
        ctrl_glyph(cv, tr, r, g, font, self.c, self.a, &self.it, h, self.s);
    }

    /// Aleatório/repetir: ligado, ganha o pontinho embaixo.
    fn toggle(&self, cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, g: &str, on: bool, h: Hit) {
        self.draw(cv, tr, r, g, Font::IconSmall, h);
        if on {
            // Numa barra fina o botão é baixo: o pontinho desce até sair de
            // baixo do ícone (sem passar da borda do fundo).
            let s = self.s;
            let y = (r.bottom() - 5.0 * s).max(r.cy() + 8.0 * s).min(r.bottom() - 2.0 * s);
            cv.fill_circle(r.cx(), y, 1.7 * s, self.c, self.a);
        }
    }
}

/// Cor aproximada da barra de tarefas atrás do widget.
fn taskbar_base(colors: &Colors) -> Rgb {
    if colors.light_taskbar { [0.93, 0.93, 0.94] } else { [0.11, 0.11, 0.12] }
}

/// Cor de destaque do Windows legível sobre a barra.
fn taskbar_accent(colors: &Colors) -> Rgb {
    if colors.light_taskbar { colors.accent } else { colors.accent_light }
}

/// Cores do fundo "Gradiente da capa": começo, fim e o brilho atrás da capa.
/// Na barra escura, tons profundos (texto branco legível); na clara, pastéis.
fn rich_gradient(base: Rgb, light: bool) -> (Rgb, Rgb, Rgb) {
    let (h, s, _) = crate::render::rgb_to_hsv(base);
    // Cores quase sem saturação (capa cinza): um gradiente neutro, sem inventar matiz.
    let s = if s < 0.12 { s } else { s.clamp(0.42, 0.85) };
    if light {
        let c0 = lerp3(hsv(h, s, 1.0), [1.0; 3], 0.50);
        let c1 = lerp3(hsv(h + 24.0, s, 0.95), [1.0; 3], 0.66);
        (c0, c1, lerp3(hsv(h, s, 1.0), [1.0; 3], 0.25))
    } else {
        (hsv(h, s, 0.52), hsv(h + 24.0, (s * 0.92).min(0.85), 0.30), hsv(h, (s * 0.8).min(0.75), 0.96))
    }
}

/// Película sobre a capa desfocada (cor e opacidade), para o texto ficar legível.
fn artwork_scrim(light: bool) -> (Rgb, f32) {
    if light { ([1.0; 3], 0.46) } else { ([0.0; 3], 0.38) }
}

/// Cor média do fundo do widget já sobre a barra (decide texto claro ou escuro).
fn widget_bg_mean(cfg: &Config, colors: &Colors, media: &MediaInfo) -> Rgb {
    let base = taskbar_base(colors);
    if !cfg.widget_pill {
        return base;
    }
    let light = colors.light_taskbar;
    let vibrant = media.art.as_ref().map(|a| a.vibrant);
    let op = cfg.widget_opacity as f32 / 100.0;
    let rich = cfg.widget_intensity as f32 / 100.0;
    let (c, a) = match cfg.widget_tint {
        WidgetTint::Neutral | WidgetTint::Album | WidgetTint::Accent => (widget_tint(cfg, colors, media), op),
        WidgetTint::Artwork if media.art.is_some() => {
            let avg = media.art.as_ref().map(|a| a.avg).unwrap_or(base);
            let (sc, sa) = artwork_scrim(light);
            (lerp3(avg, sc, sa), rich)
        }
        WidgetTint::Gradient | WidgetTint::Artwork => {
            let (c0, c1, _) = rich_gradient(vibrant.unwrap_or(taskbar_accent(colors)), light);
            (lerp3(c0, c1, 0.45), rich)
        }
    };
    lerp3(base, c, a.clamp(0.0, 1.0))
}

// ---------------------------------------------------------------- utilidades

/// Onde ficaram as mini barras do último card desenhado e o fundo debaixo
/// delas: só as barras mexendo, o quadro refaz só essa área (`card_bars_update`).
struct BarsSpot {
    rect: RectF,
    bg: Rc<Vec<u32>>,
    w: usize,
    h: usize,
}

thread_local! {
    static CARD_BARS: RefCell<Option<BarsSpot>> = const { RefCell::new(None) };
    /// Capas desfocadas mais recentes (a troca de música mostra a antiga e a nova juntas).
    static BLUR_CACHE: RefCell<Vec<(BlurKey, Arc<Image>)>> = const { RefCell::new(Vec::new()) };
    static LAYERS: RefCell<Vec<Layer>> = const { RefCell::new(Vec::new()) };
}

/// Capa desfocada (em cache por imagem).
pub(crate) fn blurred(img: &Arc<Image>) -> Arc<Image> {
    blurred_cached(img, false)
}

/// Capa bem mais desfocada, quase só as cores dela espalhadas: no widget a
/// faixa é fina e larga, e detalhes da capa (um sol, um rosto) virariam manchas.
fn color_wash(img: &Arc<Image>) -> Arc<Image> {
    blurred_cached(img, true)
}

/// Chave do cache de desfoque: a identidade da capa e se é o "borrão de cor".
type BlurKey = (u64, bool);

fn blurred_cached(img: &Arc<Image>, wash: bool) -> Arc<Image> {
    let key = (img.id, wash);
    BLUR_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(i) = c.iter().position(|(k, _)| *k == key) {
            let hit = c.remove(i);
            let b = hit.1.clone();
            c.push(hit);
            return b;
        }
        // Desfoca uma versão pequena (rápido); o desfoque esconde a ampliação.
        let b = if wash {
            let level = (0..8).find(|&k| img.w >> k <= 24).unwrap_or(8);
            Arc::new(img.mip(level).blurred(4))
        } else {
            let small = img.mip(if img.w >= 128 { 2 } else { 1 });
            Arc::new(small.blurred((small.w / 14).max(2)))
        };
        if c.len() >= 3 {
            c.remove(0);
        }
        c.push((key, b.clone()));
        b
    })
}

/// Camada já desenhada (fundo do widget, do card, do "A seguir") guardada:
/// enquanto nada muda nela, os quadros seguintes só a copiam (sem refazer
/// gradientes, sombras e capa a cada quadro).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayerTag {
    Widget,
    /// Fundo do card (com a capa), no tamanho do card fechado e sem o
    /// granulado da faixa que estica com o mixer.
    Card,
    /// O fundo do card na altura atual: esticado com o mixer aberto e com o
    /// granulado da faixa refeito (esticado, ele virava riscos verticais).
    CardFinal,
    UpNext,
}

struct Layer {
    tag: LayerTag,
    key: u64,
    w: usize,
    h: usize,
    px: Rc<Vec<u32>>,
}

/// Chave de uma camada a partir de tudo que muda o desenho dela.
pub(crate) fn layer_key(parts: impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

/// Bits de um retângulo (para entrar em chaves de cache).
fn rect_bits(r: RectF) -> [u32; 4] {
    [r.x.to_bits(), r.y.to_bits(), r.w.to_bits(), r.h.to_bits()]
}

/// Compõe a camada `key` (do tamanho do canvas) com opacidade `alpha`,
/// desenhando-a antes com `draw` se ainda não estiver guardada (quantas
/// ficam guardadas por tipo: ver `layer`).
fn cached_layer(cv: &mut Canvas, tag: LayerTag, key: u64, alpha: f32, draw: impl FnOnce(&mut Canvas)) {
    let (w, h) = (cv.w, cv.h);
    with_layer(tag, key, w, h, draw, |px, w, h| cv.composite(0, 0, px, w, h, alpha));
}

/// A camada `key` de `w`×`h` (desenhada antes com `draw` se ainda não
/// estiver guardada), entregue a `use_it`.
fn with_layer(tag: LayerTag, key: u64, w: usize, h: usize, draw: impl FnOnce(&mut Canvas), use_it: impl FnOnce(&[u32], usize, usize)) {
    let px = layer(tag, key, w, h, draw);
    use_it(&px, w, h);
}

/// A camada `key` de `w`×`h`, desenhada com `draw` se ainda não estiver guardada.
fn layer(tag: LayerTag, key: u64, w: usize, h: usize, draw: impl FnOnce(&mut Canvas)) -> Rc<Vec<u32>> {
    if let Some(px) = LAYERS.with(|cell| cell.borrow().iter().find(|l| l.tag == tag && l.key == key && l.w == w && l.h == h).map(|l| l.px.clone())) {
        return px;
    }
    // Quantas guardar por tipo: o widget, 2 (a troca de capa cruza a antiga
    // com a nova); o card e o "A seguir", uma só (são desenhados uma vez por
    // quadro, inteiros). A que sai tem a memória reaproveitada se ninguém
    // mais a usa (abrindo o mixer, a camada final é refeita a cada quadro).
    let keep = if tag == LayerTag::Widget { 2 } else { 1 };
    let recycled = LAYERS.with(|cell| {
        let mut layers = cell.borrow_mut();
        let same: Vec<usize> = layers.iter().enumerate().filter(|(_, l)| l.tag == tag).map(|(i, _)| i).collect();
        (same.len() >= keep).then(|| layers.remove(same[0])).and_then(|old| Rc::try_unwrap(old.px).ok())
    });
    let mut px = match recycled {
        Some(mut v) => {
            v.clear();
            v.resize(w * h, 0);
            v
        }
        None => vec![0u32; w * h],
    };
    draw(&mut Canvas::new(w, h, &mut px));
    let px = Rc::new(px);
    LAYERS.with(|cell| cell.borrow_mut().push(Layer { tag, key, w, h, px: px.clone() }));
    px
}

/// Linha da camada (altura `h0`) que vai na linha `y` de um canvas `grow`
/// linhas mais alto: em cima e embaixo iguais, a faixa `a..b` esticada.
fn stretch_row(y: usize, a: usize, b: usize, grow: usize) -> usize {
    if y < a {
        y
    } else if y >= b + grow {
        y - grow
    } else {
        a + (y - a) * (b - a) / (b - a + grow)
    }
}

/// Libera as camadas guardadas de um tipo (ex.: o card fechou; vale também
/// para a camada final dele).
pub fn forget_layers(tag: LayerTag) {
    LAYERS.with(|cell| cell.borrow_mut().retain(|l| l.tag != tag && !(tag == LayerTag::Card && l.tag == LayerTag::CardFinal)));
    if tag == LayerTag::Card {
        CARD_BARS.with(|c| *c.borrow_mut() = None);
    }
}

/// Só as mini barras do card mudaram: refaz a área delas por cima do mesmo
/// quadro (o fundo guardado debaixo delas e as barras novas), sem redesenhar
/// o card inteiro. Devolve a área refeita (para mostrar só ela); `None`: não
/// há card desenhado com barras nesse tamanho (o quadro tem de ser o inteiro).
pub fn card_bars_update(cv: &mut Canvas, values: &[f32], th: &Theme, s: f32) -> Option<RectF> {
    CARD_BARS.with(|c| {
        let c = c.borrow();
        let b = c.as_ref().filter(|b| b.w == cv.w && b.h == cv.h)?;
        let r = RectF::new(b.rect.x.floor() - 1.0, b.rect.y.floor() - 1.0, b.rect.w.ceil() + 2.0, b.rect.h.ceil() + 2.0);
        let clip = cv.clip_rect();
        cv.set_clip(clip.intersect(r));
        cv.copy_from(0, 0, &b.bg, b.w, b.h, 0);
        card_bars(cv, b.rect, values, th, s);
        cv.set_clip(clip);
        Some(r)
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

/// Selo do app no canto da capa do card e do "A seguir": o mesmo da barra,
/// mas com um anel escuro em volta no lugar do recorte (lá o vão mostra a
/// barra; aqui furaria o fundo do card).
fn card_badge(cv: &mut Canvas, b: RectF, media: &MediaInfo, th: &Theme, s: f32) {
    let Some(icon) = &media.icon else { return };
    let (cx, cy, r) = (b.cx(), b.cy(), b.w / 2.0);
    cv.fill_circle(cx, cy, r + 2.0 * s, [0.0; 3], 0.42);
    let base: Rgb = if th.dark { [0.16, 0.16, 0.17] } else { [0.98, 0.98, 0.985] };
    cv.fill_circle(cx, cy, r, base, 1.0);
    cv.stroke_round(b, r, (0.8 * s).max(0.8), th.fg, 0.12);
    let k = b.w * 0.74;
    cv.draw_image(RectF::new(cx - k / 2.0, cy - k / 2.0, k, k), icon, k * 0.22, 1.0);
}

pub fn draw_art_a(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, media: &MediaInfo, th: &Theme, shape: Shape, s: f32, source: Option<f32>, alpha: f32) {
    let badge = source.unwrap_or(0.0);
    let icon = if source.is_some() { &media.icon } else { &None };
    let radius = cover_radius(shape, r.w, s);
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
    if it.glow {
        // Windows: brilho suave na cor do botão por baixo dele e um degradê
        // discreto (mais claro em cima), como um botão com volume.
        if enabled {
            let big = size > 30.0 * s;
            let (blur, dy) = if big { (16.0 * s, 5.0 * s) } else { (9.0 * s, 2.5 * s) };
            cv.glow(b, radius, blur, dy, th.accent, (if th.dark { 0.55 } else { 0.42 }) * if hot { 1.25 } else { 1.0 });
            cv.drop_shadow(b, radius, 4.0 * s, 1.0 * s, if th.dark { 0.25 } else { 0.10 });
        }
        let (top, bot) = (lerp3(th.accent, [1.0; 3], 0.14), lerp3(th.accent, [0.0; 3], 0.06));
        let (y0, h) = (b.y, b.h);
        cv.fill_rrect_ca(b.x, b.y, b.right(), b.bottom(), radius, |_, y| (lerp3(top, bot, ((y - y0) / h).clamp(0.0, 1.0)), en));
        cv.stroke_round(b, radius, s, [1.0; 3], 0.12 * en);
    } else {
        if enabled {
            cv.drop_shadow(b, radius, 8.0 * s, 2.0 * s, if th.dark { 0.35 } else { 0.18 });
        }
        cv.fill_round(b, radius, th.accent, en);
    }
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
    // Engrossa sob o mouse; a parte cheia tem um brilho suave da própria cor.
    let h = if hot { 6.0 * s } else { 4.0 * s };
    cv.fill_rrect(x0, cy - h / 2.0, x1, cy + h / 2.0, h / 2.0, if th.dark { 0.22 } else { 0.16 }, th.fg);
    let xv = x0 + (x1 - x0) * t.clamp(0.0, 1.0);
    if xv > x0 + 0.5 {
        let bar = RectF::new(x0, cy - h / 2.0, xv - x0, h);
        cv.glow(bar, h / 2.0, 7.0 * s, 0.0, th.accent, if th.dark { 0.30 } else { 0.18 });
        let (a, b) = (lerp3(th.accent, th.fg, 0.10), th.accent);
        cv.fill_rrect_ca(bar.x, bar.y, bar.right(), bar.bottom(), h / 2.0, |x, _| (lerp3(a, b, ((x - x0) / (xv - x0).max(1.0)).clamp(0.0, 1.0)), 1.0));
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
    /// Atalhos dos apps de música dentro do widget (opção "Na barra").
    pub launch: Vec<RectF>,
    /// Separador antes dos atalhos: (x, y0, y1).
    pub launch_divider: Option<(f32, f32, f32)>,
    pub viz: Option<RectF>,
}

/// Raio dos cantos da capa em cada formato (o mesmo de `draw_art`).
fn cover_radius(shape: Shape, side: f32, s: f32) -> f32 {
    match shape {
        Shape::Circle => side / 2.0,
        Shape::Rounded => (side * 0.16).max(3.0 * s),
        Shape::Square => 2.0 * s,
    }
}

/// Distância da borda esquerda da pílula (altura `bh`) até a capa (lado `art`,
/// cantos de raio `rc`) para o canto arredondado da capa ficar dentro da ponta
/// da pílula, com um respiro. Nunca mais perto que a capa centralizada no
/// círculo da ponta (o que já serve para a capa redonda).
fn pill_lead(bh: f32, art: f32, rc: f32, s: f32) -> f32 {
    let (r, half) = (bh / 2.0, art / 2.0);
    let centered = r - half;
    let room = r - 1.5 * s - rc;
    let dy = (half - rc).max(0.0);
    if room <= dy {
        return centered;
    }
    // Centro do arco do canto a `room` do centro do círculo da ponta.
    (r - rc - (room * room - dy * dy).sqrt()).max(centered)
}

/// Aleatório e repetir na barra: com a opção ligada, cada um aparece sempre ou
/// só quando o player deixa mudar, conforme a mesma escolha do card ("Sempre" /
/// "Quando o player permite"). Antes a barra mostrava os dois sempre, até
/// num vídeo do YouTube, que não tem nenhum dos dois.
pub fn bar_toggles(cfg: &Config, m: &MediaInfo) -> (bool, bool) {
    if !cfg.taskbar_shuffle_repeat {
        return (false, false);
    }
    let always = cfg.card_shuffle_repeat == ShuffleRepeat::Always;
    (always || m.can_shuffle, always || m.can_repeat)
}

impl WLayout {
    /// Sem atalhos dentro do widget (capturas e testes).
    #[cfg(test)]
    pub fn build(s: f32, h: f32, cfg: &Config, panel: bool, viz: bool) -> Self {
        Self::build_with(s, h, cfg, panel, viz, 0)
    }

    /// Como `build`, com `launchers` atalhos de apps de música dentro do widget
    /// (aleatório e repetir conforme a opção, sem olhar o player: capturas).
    #[cfg(test)]
    pub fn build_with(s: f32, h: f32, cfg: &Config, panel: bool, viz: bool, launchers: usize) -> Self {
        let sr = cfg.taskbar_shuffle_repeat;
        Self::build_for(s, h, cfg, panel, viz, launchers, (sr, sr))
    }

    /// Como `build_with`, com o aleatório e o repetir à vista ou não
    /// (`toggles`, ver `bar_toggles`).
    pub fn build_for(s: f32, h: f32, cfg: &Config, panel: bool, viz: bool, launchers: usize, toggles: (bool, bool)) -> Self {
        let mut l = WLayout::default();
        let (top, bh) = (3.0 * s, h - 6.0 * s);
        l.radius = match cfg.widget_shape {
            WidgetShape::Rounded => 6.0 * s,
            WidgetShape::Pill => bh / 2.0,
            WidgetShape::Square => 2.0 * s,
        };
        let x0 = 2.0 * s;
        let mut x = x0;
        let pill = cfg.widget_shape == WidgetShape::Pill && cfg.widget_pill;
        let mut art = match cfg.cover_size {
            Size3::Small => (22.0 * s).min(h - 10.0 * s),
            Size3::Normal => (28.0 * s).min(h - 10.0 * s),
            Size3::Large => (32.0 * s).min(h - 8.0 * s),
        };
        if pill {
            // Cabe na altura da pílula com um respiro em cima e embaixo.
            art = art.min(bh - 3.5 * s);
        }
        // Na pílula a capa fica dentro da ponta arredondada: os cantos dela não
        // podem passar da curva (com a capa quadrada ou de cantos pequenos,
        // centralizar no círculo da ponta deixava o canto para fora).
        let lead = if pill { pill_lead(bh, art, cover_radius(cfg.cover_shape, art, s), s) } else { 7.0 * s };
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
                if toggles.0 {
                    l.shuffle = slot(btn);
                }
                l.prev = slot(btn);
                l.play = slot(32.0 * s);
                l.next = slot(btn);
                if toggles.1 {
                    l.repeat = slot(btn);
                }
                x += 2.0 * s;
            }
            // Atalhos dos apps de música, pequenos, depois dos botões.
            if launchers > 0 {
                l.launch_divider = Some((x.round() + 0.5, h * 0.32, h * 0.68));
                x += 5.0 * s;
                for _ in 0..launchers {
                    l.launch.push(RectF::new(x, top, 26.0 * s, bh));
                    x += 26.0 * s;
                }
                x += 3.0 * s;
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
        if let Some(i) = self.launch.iter().position(|r| r.contains(x, y)) {
            return Some(Hit::Launch(i as u8));
        }
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
    /// Atalhos dos apps de música à mostra no widget (em `l.launch`).
    pub launchers: &'a [crate::shared::Launcher],
}

/// Fontes do título e da segunda linha do widget, e a altura de cada linha.
fn fonts_of(size: Size3, s: f32) -> (Font, Font, f32) {
    match size {
        Size3::Small => (Font::TitleSm, Font::SubtitleSm, (15.5 * s).round()),
        Size3::Normal => (Font::Title, Font::Subtitle, (17.0 * s).round()),
        Size3::Large => (Font::TitleLg, Font::SubtitleLg, (19.0 * s).round()),
    }
}

/// Letras do widget numa área de texto de altura `h`, e se cabem duas linhas.
/// Primeiro o tamanho escolhido; se as duas linhas não couberem (barra fina,
/// como a do Windows 10), um tamanho menor; se nem assim, só o título numa
/// linha. As linhas têm folga própria: duas cabem até 2 px além da área.
fn widget_fit(cfg: &Config, h: f32, s: f32) -> (Font, Font, f32, bool) {
    let sizes: &[Size3] = match cfg.text_size {
        Size3::Large => &[Size3::Large, Size3::Normal, Size3::Small],
        Size3::Normal => &[Size3::Normal, Size3::Small],
        Size3::Small => &[Size3::Small],
    };
    for &z in sizes {
        let (ft, fs, line) = fonts_of(z, s);
        if 2.0 * line <= h + 2.0 * s {
            return (ft, fs, line, true);
        }
    }
    let (ft, fs, line) = sizes.iter().map(|&z| fonts_of(z, s)).find(|f| f.2 <= h).unwrap_or(fonts_of(Size3::Small, s));
    (ft, fs, line, false)
}

/// Lado do círculo do play na barra, se ele cabe: numa barra fina o círculo
/// ficava menor que o próprio ícone (só se via o ícone escuro, que é feito
/// para ficar sobre a cor do círculo); aí o play vira só o ícone.
fn filled_play(cfg: &Config, r: RectF, s: f32) -> Option<f32> {
    let size = (24.0 * s).min(r.h - 6.0 * s);
    (cfg.taskbar_play == TaskbarPlay::Filled && size >= 18.0 * s).then_some(size)
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
    let (ft, fs, line, two) = widget_fit(cfg, r.h, s);
    let sub = if two { widget_second_line(m, cfg, f.pb.pos) } else { String::new() };
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

/// Cor do fundo translúcido do widget (cores neutra, da capa e de destaque),
/// escolhida pela barra (o texto é que se ajusta ao resultado).
fn widget_tint(cfg: &Config, colors: &Colors, m: &MediaInfo) -> Rgb {
    let album = m.art.as_ref().map(|a| readable(a.vibrant, !colors.light_taskbar));
    match cfg.widget_tint {
        WidgetTint::Neutral => colors.foreground(),
        WidgetTint::Album | WidgetTint::Gradient | WidgetTint::Artwork => album.unwrap_or(taskbar_accent(colors)),
        WidgetTint::Accent => match (cfg.accent_source, album) {
            (crate::config::AccentSource::Album, Some(c)) => c,
            (crate::config::AccentSource::Custom, _) => crate::config::parse_hex(&cfg.accent_rgb).unwrap_or_else(|| taskbar_accent(colors)),
            _ => taskbar_accent(colors),
        },
    }
}

/// Fundo forte do widget (gradiente da capa ou capa desfocada) de uma música,
/// desenhado numa camada guardada: os quadros seguintes só a compõem.
fn rich_widget_bg(cv: &mut Canvas, l: &WLayout, f: &WidgetFrame, m: &MediaInfo, alpha: f32) {
    let (Some(bg), Some(_)) = (l.bg, l.art.or(l.text)) else { return };
    let (s, cfg, colors) = (f.s, f.cfg, f.colors);
    let light = colors.light_taskbar;
    let a = cfg.widget_intensity as f32 / 100.0;
    let art = m.art.clone().filter(|_| cfg.widget_tint == WidgetTint::Artwork);
    let base = m.art.as_ref().map(|a| a.vibrant).unwrap_or(taskbar_accent(colors));
    // Centro do brilho: a capa (ou o começo do texto).
    let glow_at = l.art.map(|r| (r.cx(), r.cy())).unwrap_or((bg.x + bg.h, bg.cy()));
    let key = layer_key((
        cfg.widget_tint as u8,
        art.as_ref().map(|a| a.id),
        base.map(f32::to_bits),
        rect_bits(bg),
        l.radius.to_bits(),
        (glow_at.0.to_bits(), glow_at.1.to_bits()),
        (cfg.widget_intensity, cfg.widget_glow, light),
    ));
    let radius = l.radius;
    cached_layer(cv, LayerTag::Widget, key, alpha, |cv| {
        let (c0, c1, glow) = rich_gradient(base, light);
        let reach = bg.h * 1.5;
        let glow_a = if cfg.widget_glow { if light { 0.55 } else { 0.62 } } else { 0.0 };
        let (gx, gy) = glow_at;
        let shade = move |x: f32, y: f32, c: Rgb| -> Rgb {
            // Brilho suave na cor da capa em volta dela e um reflexo claro no alto.
            let d2 = ((x - gx) / reach).powi(2) + ((y - gy) / (reach * 0.8)).powi(2);
            let g = glow_a * (-2.2 * d2).exp();
            let top = (1.0 - ((y - bg.y) / bg.h).clamp(0.0, 1.0)).powi(2);
            lerp3(lerp3(c, glow, g), [1.0; 3], 0.07 * top)
        };
        match &art {
            Some(img) => {
                // A capa inteira espalhada pela faixa (cores da esquerda à direita).
                let b = color_wash(img);
                cv.draw_image(bg, &b, radius, a);
                let (sc, sa) = artwork_scrim(light);
                // Película um pouco mais forte à direita (onde ficam os botões).
                cv.fill_rrect_ca(bg.x, bg.y, bg.right(), bg.bottom(), radius, |x, _| {
                    let t = ((x - bg.x) / bg.w).clamp(0.0, 1.0);
                    (sc, a * (sa + 0.10 * t))
                });
                if glow_a > 0.0 {
                    let gc = if light { lerp3(img.vibrant, [1.0; 3], 0.4) } else { readable(img.vibrant, true) };
                    cv.fill_rrect_ca(bg.x, bg.y, bg.right(), bg.bottom(), radius, |x, y| {
                        let d2 = ((x - gx) / reach).powi(2) + ((y - gy) / (reach * 0.8)).powi(2);
                        (gc, a * 0.42 * glow_a * (-2.2 * d2).exp())
                    });
                }
            }
            None => {
                cv.fill_rrect_ca(bg.x, bg.y, bg.right(), bg.bottom(), radius, |x, y| {
                    let t = ((x - bg.x) / bg.w).clamp(0.0, 1.0);
                    let t = t * t * (3.0 - 2.0 * t);
                    (shade(x, y, lerp3(c0, c1, t)), a)
                });
            }
        }
        // Brilho fino no topo e contorno (elevação do Fluent).
        cv.fill_rrect_ca(bg.x, bg.y, bg.right(), bg.bottom(), radius, |_, y| ([1.0; 3], if y < bg.y + 1.5 * s { 0.10 * a } else { 0.0 }));
        if cfg.widget_border {
            cv.stroke_round(bg, radius, s, if light { [0.0; 3] } else { [1.0; 3] }, if light { 0.10 } else { 0.12 });
        }
    });
}

pub fn draw_widget(cv: &mut Canvas, tr: &mut TextRenderer, l: &WLayout, f: &WidgetFrame) {
    let (s, th, m, cfg) = (f.s, &f.theme, f.media, f.cfg);
    let it = f.it.styled(cfg);
    // Fundo único com tudo dentro; o card aberto deixa o fundo um pouco mais forte.
    if let Some(bg) = l.bg {
        if cfg.widget_tint.is_rich() {
            // Troca de música: o fundo da capa antiga dá lugar ao da nova.
            match f.change.filter(|c| c.2 < 1.0) {
                Some((old, _, t)) => {
                    rich_widget_bg(cv, l, f, old, 1.0);
                    rich_widget_bg(cv, l, f, m, ease_enter(t));
                }
                None => rich_widget_bg(cv, l, f, m, 1.0),
            }
            if f.flyout_open {
                cv.fill_round(bg, l.radius, th.fg, 0.06);
            }
        } else {
            let a = cfg.widget_opacity as f32 / 100.0 + if f.flyout_open { 0.04 } else { 0.0 };
            cv.fill_round(bg, l.radius, widget_tint(cfg, f.colors, m), a.min(1.0));
            if cfg.widget_border {
                cv.stroke_round(bg, l.radius, s, th.fg, if th.dark { 0.08 } else { 0.10 });
            }
            // Brilho fino no topo (elevação do Fluent).
            cv.fill_rrect_ca(bg.x, bg.y, bg.right(), bg.bottom(), l.radius, |_, y| ([1.0; 3], if y < bg.y + 1.5 * s { 0.05 } else { 0.0 }));
        }
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
        if h == Hit::Play && l.play.is_some_and(|r| filled_play(cfg, r, s).is_some()) {
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
    // Controles na cor escolhida (padrão: branco).
    let ct = controls_theme(cfg.widget_controls, &cfg.widget_controls_rgb, th, m);
    // Botões laterais: por padrão tão fortes quanto o do meio, na mesma cor.
    // "Igual ao botão do meio": a cor do play; em "Branco e destaque" (play na
    // cor de destaque, ícones brancos) os laterais ficam brancos, senão essa
    // opção ficaria igual a "Cor de destaque".
    let white_accent = cfg.widget_controls == crate::config::ControlColor::WhiteAccent;
    let mid = if cfg.taskbar_play == TaskbarPlay::Icon || white_accent { ct.fg } else { ct.accent };
    let side = Sides::of(cfg.widget_sides, cfg.widget_sides_alpha, mid, th, m, it, s);
    if let Some(r) = l.shuffle {
        side.toggle(cv, tr, r, G_SHUFFLE, f.pb.shuffle && m.can_shuffle, Hit::Shuffle);
    }
    if let Some(r) = l.prev {
        side.draw(cv, tr, r, G_PREV, Font::IconSmall, Hit::Prev);
    }
    if let Some(r) = l.play {
        match filled_play(cfg, r, s) {
            Some(size) => play_button(cv, tr, r, size, f.cfg.play_shape, &ct, &it, f.pb.playing, m.can_toggle, s),
            // Só o ícone (opção, ou barra fina sem lugar para o círculo): na
            // barra fina, na cor dos botões do lado, do tamanho deles.
            None if cfg.taskbar_play == TaskbarPlay::Filled => play_glyph(cv, tr, r, f.pb.playing, Font::Icon, side.c, dim(m.can_toggle), &it, s),
            None => play_glyph(cv, tr, r, f.pb.playing, Font::Icon, ct.fg, dim(m.can_toggle), &it, s),
        }
    }
    if let Some(r) = l.next {
        side.draw(cv, tr, r, G_NEXT, Font::IconSmall, Hit::Next);
    }
    if let Some(r) = l.repeat {
        let g = if f.pb.repeat == 1 { G_REPEAT_ONE } else { G_REPEAT_ALL };
        side.toggle(cv, tr, r, g, f.pb.repeat != 0 && m.can_repeat, Hit::Repeat);
    }
    // Atalhos dos apps de música (opção "Na barra").
    if let Some((x, y0, y1)) = l.launch_divider {
        cv.fill_rrect(x - 0.5 * s, y0, x + 0.5 * s, y1, 0.5 * s, 0.14, th.fg);
    }
    for (i, r) in l.launch.iter().enumerate() {
        let Some(app) = f.launchers.get(i) else { break };
        let h = Hit::Launch(i as u8);
        let hot = it.hover == Some(h);
        if hot {
            let k = (r.w.min(r.h) - 3.0 * s).max(1.0);
            hover_bg(cv, RectF::new(r.cx() - k / 2.0, r.cy() - k / 2.0, k, k), k / 2.0, th, it.pressed == Some(h));
        }
        let k = if hot && it.pressed == Some(h) { 15.0 * s } else { 17.0 * s };
        launcher_icon(cv, tr, RectF::new(r.cx() - k / 2.0, r.cy() - k / 2.0, k, k), app, th, 1.0);
    }
    if let (Some(r), Some((values, peaks))) = (l.viz, f.viz) {
        draw(cv, r, &Frame { style: cfg.style, values, peaks, show_peaks: cfg.show_peaks, colors: f.colors, scale: s, square: cfg.bar_shape == crate::config::BarShape::Square });
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
    /// Selo do app de origem no canto da capa ("Mostrar app de origem").
    pub source: Option<RectF>,
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
    /// Atalhos dos apps de música (linha embaixo do card) e o texto ao lado.
    pub launch: Vec<RectF>,
    pub launch_label: RectF,
    /// Mixer de volume: a setinha (sempre, com a opção ligada), a área que
    /// abre (altura animada), o cabeçalho e as linhas (linha, slider, mudo).
    pub mixer_btn: Option<RectF>,
    pub mixer_area: RectF,
    pub mixer_head: RectF,
    pub mixer_rows: Vec<(RectF, RectF, RectF)>,
    /// Quanto o mixer está aberto (0..1, já com a curva da animação).
    pub mixer_open: f32,
    /// Altura (px inteiros) que o mixer aberto acrescenta ao card: o fundo
    /// guardado é o do card fechado, esticado no meio por esta altura.
    pub grown: f32,
    /// Faixa das linhas do mixer à vista (com mais apps, a lista rola).
    pub mixer_list: RectF,
    /// Seletor da saída de áudio no cabeçalho do mixer.
    pub mixer_dev: RectF,
    /// Quanto a lista do mixer rola no máximo (px lógicos) e quanto rolou (px).
    pub mixer_scroll_max: f32,
    pub mixer_scroll: f32,
    /// Tempos (decorrido e restante) embaixo do progresso.
    pub times: bool,
    /// Botão que mostra e esconde a letra na tela (ponta direita da linha do volume).
    pub lyrics: Option<RectF>,
}

impl CLayout {
    pub fn build(s: f32, cfg: &Config, has_seek: bool) -> Self {
        let m = SHADOW_MARGIN * s;
        let pad = 16.0 * s;
        let mut l = CLayout { radius: cfg.corners.radius() * s, vertical: cfg.card_layout == CardLayout::Vertical, times: true, ..Default::default() };
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
            let by = l.art.bottom() - 40.0 * s;
            (l.shuffle, l.prev, l.play, l.next, l.repeat) = row(tx, tx + tw, by, 40.0 * s, 9.0 * s);
            l.play_size = 38.0 * s;
            // Título e artista ficam centralizados entre o topo da capa e os
            // botões; `draw_card` sobe um pouco se houver álbum.
            let top = l.art.y;
            let ty = top + ((by - top) - 43.0 * s) / 2.0;
            l.title = RectF::new(tx, ty.round(), tw, 24.0 * s);
            l.artist = RectF::new(tx, (ty + 24.0 * s).round(), tw, 19.0 * s);
            y = l.art.bottom() + 16.0 * s;
        } else {
            cw = 304.0 * s;
            let art = cw - 2.0 * pad;
            l.art = RectF::new(m + pad, m + pad, art, art);
            l.title = RectF::new(m + pad, l.art.bottom() + 14.0 * s, art, 26.0 * s);
            l.artist = RectF::new(m + pad, l.art.bottom() + 40.0 * s, art, 19.0 * s);
            let by = l.art.bottom() + 66.0 * s;
            (l.shuffle, l.prev, l.play, l.next, l.repeat) = row(m + pad, m + cw - pad, by, 46.0 * s, 20.0 * s);
            l.play_size = 46.0 * s;
            y = by + 46.0 * s + 12.0 * s;
        }
        // Selo do app no canto de baixo da capa, como o da barra (um pouco
        // para fora dela, com um anel em volta).
        if cfg.show_source {
            let k = (if l.vertical { 32.0 } else { 24.0 } * s).round();
            let (cx, cy) = (l.art.right() - k * 0.22, l.art.bottom() - k * 0.22);
            l.source = Some(RectF::new(cx - k / 2.0, cy - k / 2.0, k, k));
        }
        // Progresso e volume com os trilhos nas mesmas bordas (as da capa e do texto).
        if seek {
            l.seek = Some(RectF::new(m + pad, y, cw - 2.0 * pad, 30.0 * s));
            y += 34.0 * s;
        }
        // Botão da letra na ponta direita da linha do volume, espelhando o do
        // volume na esquerda (o slider encolhe); sem o volume, a linha fica só com ele.
        let lyrics = cfg.lyrics.card_button;
        if cfg.show_volume || lyrics {
            if lyrics {
                l.lyrics = Some(RectF::new(m + cw - pad - 24.0 * s, y, 32.0 * s, 32.0 * s));
            }
            if cfg.show_volume {
                let b = RectF::new(m + pad - 8.0 * s, y, 32.0 * s, 32.0 * s);
                l.vol_btn = Some(b);
                let vx = m + pad + 26.0 * s;
                let right = m + cw - pad - if lyrics { 30.0 * s } else { 0.0 };
                l.vol = Some(RectF::new(vx, y, right - vx, 32.0 * s));
            }
            y += 36.0 * s;
        }
        let ch = y + 8.0 * s - m;
        l.card = RectF::new(m, m, cw, ch);
        l.w = cw + 2.0 * m;
        l.h = ch + 2.0 * m;
        l
    }

    /// Acrescenta a linha de atalhos para `n` apps de música embaixo do card.
    pub fn with_launchers(mut self, n: usize, s: f32) -> Self {
        if n == 0 {
            return self;
        }
        let pad = 16.0 * s;
        let size = 32.0 * s;
        let y = self.card.bottom() - 2.0 * s;
        let x0 = self.card.x + pad - 6.0 * s;
        self.launch = (0..n).map(|i| RectF::new(x0 + i as f32 * (size + 2.0 * s), y, size, size)).collect();
        let lx = x0 + n as f32 * (size + 2.0 * s) + 6.0 * s;
        self.launch_label = RectF::new(lx, y, (self.card.right() - pad - lx).max(0.0), size);
        let extra = size + 6.0 * s;
        self.card.h += extra;
        self.h += extra;
        self
    }

    /// Acrescenta o mixer de volume embaixo do card: a setinha e, aberto
    /// (`open` de 0 a 1), as linhas de `rows` apps. `None`: opção desligada.
    /// Mixer com `rows` apps (`None`: sem mixer), aberto `open` (0..1), a lista
    /// rolada `scroll` px lógicos e no máximo `max_rows` linhas à vista.
    pub fn with_mixer(mut self, rows: Option<usize>, open: f32, scroll: f32, max_rows: usize, s: f32) -> Self {
        let Some(n) = rows else { return self };
        let pad = 16.0 * s;
        let top = self.card.bottom() - 4.0 * s;
        let visible = n.clamp(1, max_rows.clamp(1, crate::shared::MIXER_ROWS));
        let full = mixer_height(visible, s);
        let h = (full * open.clamp(0.0, 1.0)).round();
        self.mixer_open = open.clamp(0.0, 1.0);
        self.mixer_area = RectF::new(self.card.x, top, self.card.w, h);
        self.mixer_head = RectF::new(self.card.x + pad, top + 6.0 * s, self.card.w - 2.0 * pad, 22.0 * s);
        let hd = self.mixer_head;
        self.mixer_dev = RectF::new(hd.x + hd.w * 0.42, hd.y - 3.0 * s, hd.w * 0.58 + 6.0 * s, hd.h + 6.0 * s);
        self.mixer_list = RectF::new(self.card.x, top + 30.0 * s, self.card.w, visible as f32 * 36.0 * s + 2.0 * s);
        self.mixer_scroll_max = n.saturating_sub(visible) as f32 * 36.0;
        let off = (scroll.clamp(0.0, self.mixer_scroll_max) * s).round();
        self.mixer_scroll = off;
        self.mixer_rows = (0..n)
            .map(|i| {
                let row = RectF::new(self.card.x + pad, top + 32.0 * s + i as f32 * 36.0 * s - off, self.card.w - 2.0 * pad, 34.0 * s);
                let slider = RectF::new(row.x + 140.0 * s, row.y, row.w - 140.0 * s - 40.0 * s, row.h);
                let mute = RectF::new(row.right() - 34.0 * s, row.y, 34.0 * s, row.h);
                (row, slider, mute)
            })
            .collect();
        self.mixer_btn = Some(RectF::new(self.card.cx() - 24.0 * s, top + h, 48.0 * s, 20.0 * s));
        self.grown = h;
        let extra = h + 18.0 * s;
        self.card.h += extra;
        self.h += extra;
        self
    }

    /// Trilho do slider do app `i` no mixer.
    pub fn mix_track(&self, i: usize, s: f32) -> Option<(f32, f32)> {
        self.mixer_rows.get(i).map(|(_, r, _)| (r.x + 6.0 * s, r.right() - 6.0 * s))
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        if let Some(i) = self.launch.iter().position(|r| r.contains(x, y)) {
            return Some(Hit::Launch(i as u8));
        }
        if self.mixer_btn.is_some_and(|r| r.contains(x, y)) {
            return Some(Hit::MixToggle);
        }
        // Linhas do mixer: só a parte que já está à vista (abrindo, rolando).
        if self.mixer_open > 0.5 && self.mixer_area.contains(x, y) {
            if self.mixer_dev.contains(x, y) {
                return Some(Hit::MixDevice);
            }
            let list = self.mixer_list.contains(x, y);
            for (i, (_, slider, mute)) in self.mixer_rows.iter().enumerate().filter(|_| list) {
                if mute.contains(x, y) {
                    return Some(Hit::MixMute(i as u8));
                }
                if slider.contains(x, y) {
                    return Some(Hit::MixVol(i as u8));
                }
            }
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
            (self.lyrics, Hit::Lyrics),
            (self.source, Hit::Source),
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
    card_bg(cv, &BgSpec { card, radius, cfg, colors, media, th, s, backdrop }, None);
}

/// Opções de design que mudam o fundo do card e do "A seguir" (entram na
/// chave das camadas guardadas; o desfoque já vem no `id` do fundo acrílico).
fn card_look(cfg: &Config) -> (u8, bool, i32, i32) {
    (cfg.card_shadow as u8, cfg.card_grain, cfg.glass_tint, cfg.artwork_dim)
}

/// O que `card_background` desenha (para passar adiante sem uma lista enorme).
struct BgSpec<'a> {
    card: RectF,
    radius: f32,
    cfg: &'a Config,
    colors: &'a Colors,
    media: &'a MediaInfo,
    th: &'a Theme,
    s: f32,
    backdrop: Option<&'a Image>,
}

/// Intensidade do granulado do card (o acrílico tem o dele).
fn card_grain(cfg: &Config, th: &Theme, backdrop: bool) -> f32 {
    if !cfg.card_grain {
        return 0.0;
    }
    match cfg.card_style {
        CardStyle::Glass if backdrop => 0.026,
        CardStyle::Glass => 0.0,
        _ if th.dark => 0.022,
        _ => 0.014,
    }
}

/// Fundo do card; `skip`: faixa de linhas (y0, y1) sem granulado (é a que
/// estica com o mixer: o granulado dela é feito depois, na altura certa).
fn card_bg(cv: &mut Canvas, b: &BgSpec, skip: Option<(usize, usize)>) {
    let BgSpec { card, radius, cfg, colors, media, th, s, backdrop } = *b;
    match cfg.card_shadow {
        CardShadow::Strong => {
            cv.drop_shadow(card, radius, 28.0 * s, 10.0 * s, if th.dark { 0.58 } else { 0.24 });
            cv.drop_shadow(card, radius, 6.0 * s, 1.5 * s, if th.dark { 0.30 } else { 0.10 });
        }
        CardShadow::Soft => {
            cv.drop_shadow(card, radius, 16.0 * s, 5.0 * s, if th.dark { 0.30 } else { 0.12 });
            cv.drop_shadow(card, radius, 4.0 * s, 1.0 * s, if th.dark { 0.18 } else { 0.06 });
        }
        CardShadow::None => {}
    }
    let (x0, y0, w, h) = (card.x, card.y, card.w, card.h);
    // Posição relativa (0..1) e distância ao centro, para as luzes e a vinheta.
    let rel = move |x: f32, y: f32| (((x - x0) / w).clamp(0.0, 1.0), ((y - y0) / h).clamp(0.0, 1.0));
    match (cfg.card_style, &media.art) {
        (CardStyle::Artwork, Some(art)) => {
            let dim = cfg.artwork_dim as f32 / 100.0;
            let b = blurred(art);
            cv.draw_image_cover(card, &b, radius, 1.0);
            // Película escura (mais forte embaixo, onde ficam os controles), um
            // pouco da cor viva da capa no alto à esquerda e vinheta nas bordas.
            let glow = readable(art.vibrant, true);
            cv.fill_rrect_ca(x0, y0, card.right(), card.bottom(), radius, |x, y| {
                let (u, v) = rel(x, y);
                let light = (-(((u - 0.18) * 2.0).powi(2) + ((v - 0.15) * 2.2).powi(2))).exp();
                let edge = ((u - 0.5).powi(2) * 1.6 + (v - 0.5).powi(2) * 2.2).min(1.0);
                let c = lerp3([0.02, 0.02, 0.03], glow, 0.22 * light);
                (c, ((0.34 + 0.30 * v + 0.30 * edge - 0.10 * light) * dim).clamp(0.0, 0.92))
            });
        }
        (CardStyle::AlbumColor, _) | (CardStyle::Artwork, None) => {
            let bg = album_bg(cfg, colors, media);
            let dark = th.dark;
            let vib = media.art.as_ref().map(|a| a.vibrant).unwrap_or(th.accent);
            let glow = if dark { readable(vib, true) } else { lerp3(vib, [1.0; 3], 0.35) };
            let far: Rgb = if dark { [0.0; 3] } else { [1.0; 3] };
            cv.fill_rrect_ca(x0, y0, card.right(), card.bottom(), radius, |x, y| {
                let (u, v) = rel(x, y);
                // Luz da cor da capa no alto à esquerda (onde está a capa), fundo
                // mais profundo na diagonal oposta.
                let light = (-(((u - 0.16) * 1.8).powi(2) + ((v - 0.2) * 2.0).powi(2))).exp();
                let c = lerp3(bg, far, 0.32 * ((u * 0.45 + v * 0.55) - 0.2).clamp(0.0, 1.0));
                (lerp3(c, glow, (if dark { 0.30 } else { 0.22 }) * light), 1.0)
            });
        }
        (CardStyle::Glass, _) => {
            let base: Rgb = if th.dark { [0.12, 0.12, 0.13] } else { [0.96, 0.96, 0.965] };
            let tint = media.art.as_ref().map(|a| a.avg).unwrap_or(th.accent);
            let c = lerp3(base, tint, if th.dark { 0.14 } else { 0.06 });
            match backdrop {
                // Acrílico sem o granulado dele: vem logo abaixo, com o dos outros.
                Some(b) => {
                    cv.draw_image_cover(card, b, radius, 1.0);
                    let tint = cfg.glass_tint as f32 / 100.0;
                    cv.fill_round(card, radius, c, if th.dark { tint } else { (tint - 0.04).max(0.16) });
                }
                None => cv.fill_round(card, radius, c, 0.96),
            }
        }
    }
    // Granulado fino (tira o aspecto de "plástico" dos degradês), reflexo no
    // alto e contorno.
    let amount = card_grain(cfg, th, backdrop.is_some());
    match skip {
        None => cv.grain(card, radius, amount),
        Some((y0, y1)) => {
            let clip = cv.clip_rect();
            let w = cv.w as f32;
            cv.set_clip(clip.intersect(RectF::new(0.0, 0.0, w, y0 as f32)));
            cv.grain(card, radius, amount);
            cv.set_clip(clip.intersect(RectF::new(0.0, y1 as f32, w, cv.h as f32 - y1 as f32)));
            cv.grain(card, radius, amount);
            cv.set_clip(clip);
        }
    }
    cv.fill_rrect_ca(x0, y0, card.right(), card.bottom(), radius, |_, y| ([1.0; 3], if y < y0 + 1.5 * s { 0.10 } else { 0.0 }));
    let line: Rgb = if th.dark || cfg.card_style == CardStyle::Artwork { [1.0; 3] } else { [0.0; 3] };
    cv.stroke_round(card, radius, s, line, if th.dark { 0.12 } else { 0.08 });
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
    /// Apps de música instalados (atalhos embaixo do card, em `l.launch`).
    pub launchers: &'a [crate::shared::Launcher],
    /// Barras de áudio (mini visualizador ao lado do título), se ligadas.
    pub viz: Option<&'a [f32]>,
    /// Mixer de volume (com a opção ligada): dispositivo e apps.
    pub mixer: Option<&'a crate::shared::MixerState>,
}

/// Sombra da capa: brilho na cor dela + sombra curta, como capas "flutuando".
pub(crate) fn art_shadow(cv: &mut Canvas, r: RectF, radius: f32, art: &Image, th: &Theme, s: f32) {
    cv.glow(r, radius, 22.0 * s, 7.0 * s, art.vibrant, if th.dark { 0.42 } else { 0.30 });
    cv.drop_shadow(r, radius, 12.0 * s, 4.0 * s, if th.dark { 0.40 } else { 0.22 });
}

/// Mini barras de áudio do card (até 7, na cor de destaque, mais claras em cima).
fn card_bars(cv: &mut Canvas, r: RectF, values: &[f32], th: &Theme, s: f32) {
    if values.is_empty() {
        return;
    }
    // Até 7 barras; cada uma pega a maior de uma faixa dos valores. As faixas
    // dividem o total por igual (com 10 valores: 1,1,2,1,2,1,2), sem passar
    // do fim (com "div_ceil" por faixa, 8 a 11 valores estouravam).
    let n = values.len().min(7);
    let slot = r.w / n as f32;
    let bw = (slot * 0.58).max(2.0 * s);
    let top = lerp3(th.accent, [1.0; 3], if th.dark { 0.35 } else { 0.0 });
    let (y1, h) = (r.bottom(), r.h);
    for i in 0..n {
        let (a, b) = (i * values.len() / n, (i + 1) * values.len() / n);
        let v = values[a..b.max(a + 1)].iter().copied().fold(0.0f32, f32::max);
        let bh = (3.0 * s + v.clamp(0.0, 1.0) * (h - 3.0 * s)).round();
        let x = r.x + i as f32 * slot + (slot - bw) / 2.0;
        cv.fill_rrect_ca(x, y1 - bh, x + bw, y1, bw / 2.0, |_, y| (lerp3(th.accent, top, ((y1 - y) / h).clamp(0.0, 1.0)), 1.0));
    }
}

pub fn draw_card(cv: &mut Canvas, tr: &mut TextRenderer, l: &CLayout, f: &CardFrame) {
    let (s, th, m, cfg) = (f.s, &f.theme, f.media, f.cfg);
    let it = f.it.styled(cfg);

    // Fundo, sombra e capa não mudam de um quadro para o outro: ficam numa
    // camada guardada e os quadros seguintes só a compõem.
    let art_r = (l.radius * 0.6).max(6.0 * s);
    let art_shape = if cfg.cover_shape == Shape::Circle && !l.vertical { Shape::Circle } else { Shape::Rounded };
    // Com o mixer aberto (ou abrindo) o fundo não é redesenhado na altura
    // nova a cada quadro (capa desfocada, película e granulado custam ~20 ms):
    // usa o do card fechado e estica uma faixa do meio, entre a capa e a
    // borda de baixo (tudo ali são degradês suaves).
    let grow = l.grown.round().max(0.0) as usize;
    let base = RectF { h: l.card.h - grow as f32, ..l.card };
    let band = (((l.art.bottom() + 32.0 * s).ceil().max(0.0) as usize), ((base.bottom() - l.radius.max(8.0 * s) - 4.0 * s).floor().max(0.0) as usize));
    let grow = if band.1 > band.0 + 4 && grow < cv.h { grow } else { 0 };
    let base = RectF { h: l.card.h - grow as f32, ..l.card };
    let key = layer_key((
        (cfg.card_style as u8, art_shape as u8, cfg.show_source, l.vertical, card_look(cfg)),
        (rect_bits(base), rect_bits(l.art), l.radius.to_bits()),
        (m.art.as_ref().map(|a| a.id), m.icon.as_ref().map(|a| a.id)),
        (th.dark, th.accent.map(f32::to_bits), th.fg.map(f32::to_bits), f.colors.accent.map(f32::to_bits)),
        f.backdrop.map(|b| b.id),
    ));
    let draw_bg = |cv: &mut Canvas| {
        card_bg(cv, &BgSpec { card: base, radius: l.radius, cfg, colors: f.colors, media: m, th, s, backdrop: f.backdrop }, Some(band));
        if let Some(a) = &m.art {
            art_shadow(cv, l.art, art_r, a, th, s);
        }
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
        // Sem capa o próprio ícone do app já está no lugar dela.
        if let (Some(b), Some(_)) = (l.source, &m.art) {
            card_badge(cv, b, m, th, s);
        }
    };
    // Fundo na altura atual: o guardado (esticado com o mixer) e o granulado
    // da faixa do meio feito por cima, na altura certa. Fechado ou aberto
    // parado, essa camada é reaproveitada; abrindo, é refeita (é barata).
    let (w, h) = (cv.w, cv.h);
    let h0 = h - grow;
    // A camada final antiga é solta antes (para a memória dela ser reaproveitada).
    CARD_BARS.with(|c| *c.borrow_mut() = None);
    let bg = layer(LayerTag::Card, key, w, h0, draw_bg);
    let amount = card_grain(cfg, th, f.backdrop.is_some());
    let fin = layer(LayerTag::CardFinal, layer_key((key, grow, band)), w, h, |fcv| {
        fcv.composite_rows(&bg, w, h0, |y| stretch_row(y, band.0, band.1, grow));
        fcv.set_clip(RectF::new(0.0, band.0 as f32, w as f32, (band.1 - band.0 + grow) as f32));
        fcv.grain_tile(l.card, l.radius, amount);
    });
    cv.composite(0, 0, &fin, w, h, 1.0);

    // Título, artista.
    let title = if !m.present() {
        crate::i18n::tr("Nada tocando").to_string()
    } else if m.title.is_empty() {
        m.app.clone()
    } else {
        m.title.clone()
    };
    // Mini barras de áudio na ponta direita da linha do título (como no Mac).
    let bars_w = 30.0 * s;
    let bars = f.viz.filter(|_| cfg.card_viz && m.present());
    let title_w = |r: RectF| if bars.is_some() { RectF { w: r.w - bars_w - 10.0 * s, ..r } } else { r };
    let draw_bars = |cv: &mut Canvas, t: RectF| {
        if let Some(v) = bars {
            let rect = RectF::new(t.right() - bars_w, t.y + 3.0 * s, bars_w, t.h - 5.0 * s);
            card_bars(cv, rect, v, th, s);
            CARD_BARS.with(|c| *c.borrow_mut() = Some(BarsSpot { rect, bg: fin.clone(), w: cv.w, h: cv.h }));
        }
    };
    if l.vertical {
        // Coluna larga: "Artista · Álbum" numa linha só.
        let t = title_w(l.title);
        text(cv, tr, &title, Font::CardTitle, t.x, t.y, t.w, t.h, th.fg, 1.0);
        draw_bars(cv, l.title);
        text(cv, tr, &artist_album(m), Font::Subtitle, l.artist.x, l.artist.y, l.artist.w, l.artist.h, th.fg, 0.72);
    } else {
        // Álbum numa terceira linha; o bloco sobe metade dela para seguir centralizado.
        let sub = artist_album(m);
        let album = (sub != m.artist && !m.artist.is_empty()).then(|| m.album.trim());
        let dy = if album.is_some() { -8.0 * s } else { 0.0 };
        let (t, a) = (title_w(l.title.offset(0.0, dy)), l.artist.offset(0.0, dy));
        text(cv, tr, &title, Font::CardTitle, t.x, t.y, t.w, t.h, th.fg, 1.0);
        draw_bars(cv, l.title.offset(0.0, dy));
        text(cv, tr, if m.artist.is_empty() { &sub } else { &m.artist }, Font::Subtitle, a.x, a.y, a.w, a.h, th.fg, 0.72);
        if let Some(album) = album {
            text(cv, tr, album, Font::Small, a.x, a.bottom() - 1.0 * s, a.w, 17.0 * s, th.fg, 0.52);
        }
    }

    // Controles. Aleatório/repetir: sempre (apagados quando o player não deixa
    // mudar pelo Windows, como os navegadores) ou só quando dá, conforme a opção.
    let always = cfg.card_shuffle_repeat == ShuffleRepeat::Always;
    for h in [Hit::Shuffle, Hit::Prev, Hit::Next, Hit::Repeat, Hit::VolBtn, Hit::Lyrics] {
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
                Hit::Lyrics => l.lyrics.unwrap_or_default(),
                _ => l.vol_btn.unwrap_or_default(),
            };
            let k = r.h.min(r.w) - 4.0 * s;
            hover_bg(cv, RectF::new(r.cx() - k / 2.0, r.cy() - k / 2.0, k, k), k / 2.0, th, f.it.pressed == Some(h));
        }
    }
    let ct = controls_theme(cfg.card_controls, &cfg.card_controls_rgb, th, m);
    let mid = if cfg.card_controls == crate::config::ControlColor::WhiteAccent { ct.fg } else { ct.accent };
    let side = Sides::of(cfg.card_sides, cfg.card_sides_alpha, mid, th, m, it, s);
    if m.can_shuffle || always {
        side.toggle(cv, tr, l.shuffle, G_SHUFFLE, f.pb.shuffle && m.can_shuffle, Hit::Shuffle);
    }
    side.draw(cv, tr, l.prev, G_PREV, Font::Icon, Hit::Prev);
    play_button(cv, tr, l.play, l.play_size, cfg.play_shape, &ct, &it, f.pb.playing, m.can_toggle, s);
    side.draw(cv, tr, l.next, G_NEXT, Font::Icon, Hit::Next);
    let g = if f.pb.repeat == 1 { G_REPEAT_ONE } else { G_REPEAT_ALL };
    if m.can_repeat || always {
        side.toggle(cv, tr, l.repeat, g, f.pb.repeat != 0 && m.can_repeat, Hit::Repeat);
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
        if l.times {
            text(cv, tr, &fmt_time(pos), Font::Small, x0, r.y + 13.0 * s, 60.0 * s, 16.0 * s, th.fg, 0.62);
            text_right(cv, tr, &format!("-{}", fmt_time((m.dur - pos).max(0.0))), Font::Small, x1, r.y + 13.0 * s, 16.0 * s, th.fg, 0.62, s);
        }
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
        ctrl_glyph(cv, tr, b, g, Font::IconSmall, th.fg, 0.9, &it, Hit::VolBtn, s);
        let hot = f.it.hover == Some(Hit::Vol) || f.it.drag == Some(Hit::Vol);
        let fill = f.it.vol_fill.unwrap_or(if f.pb.muted { 0.0 } else { v });
        slider(cv, x0, x1, r.cy(), fill, th, hot, true, s);
    }

    // Letra na tela: ícone com o pontinho embaixo quando ela está à vista
    // (como o aleatório e o repetir ligados).
    if let Some(b) = l.lyrics {
        let on = cfg.lyrics.show;
        ctrl_glyph(cv, tr, b, G_LYRICS, Font::IconSmall, th.fg, if on { 1.0 } else { 0.9 }, &it, Hit::Lyrics, s);
        if on {
            let y = (b.bottom() - 5.0 * s).max(b.cy() + 8.0 * s).min(b.bottom() - 2.0 * s);
            cv.fill_circle(b.cx(), y, 1.7 * s, th.fg, 0.95);
        }
    }

    // Mixer de volume (setinha embaixo; aberto, uma linha por app).
    if let Some(mx) = f.mixer.filter(|_| l.mixer_btn.is_some()) {
        mixer_section(cv, tr, l, f, mx);
    }

    // Atalhos para abrir os apps de música.
    if !l.launch.is_empty() {
        launcher_row(cv, tr, l, f);
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

    // Dica da setinha do mixer: por último, por cima de tudo (dos atalhos também).
    if let Some(b) = l.mixer_btn.filter(|_| f.it.hover == Some(Hit::MixToggle) && f.it.tip > 0.0) {
        let msg = crate::i18n::tr(if l.mixer_open > 0.5 { "Fechar mixer de volume" } else { "Abrir mixer de volume" });
        tooltip(cv, tr, msg, b, th, f.it.tip, s);
    }
    if let Some(b) = l.lyrics.filter(|_| f.it.hover == Some(Hit::Lyrics) && f.it.tip > 0.0) {
        tooltip(cv, tr, &lyrics_tip(cfg), b, th, f.it.tip, s);
    }
    if let Some(b) = l.source.filter(|_| f.it.hover == Some(Hit::Source) && f.it.tip > 0.0 && !m.app.is_empty()) {
        tooltip(cv, tr, &crate::i18n::tr("Tocando no {}").replace("{}", &m.app), b, th, f.it.tip, s);
    }
}

/// Dica do botão da letra, com o atalho quando ele funciona.
fn lyrics_tip(cfg: &Config) -> String {
    let what = crate::i18n::tr(if cfg.lyrics.show { "Esconder a letra" } else { "Mostrar a letra na tela" });
    #[cfg(windows)]
    if let Some(k) = crate::lyrics_overlay::working_hotkey(cfg.lyrics.hotkey) {
        return format!("{what} ({k})");
    }
    what.to_string()
}

/// Setinha do mixer (para baixo; vira para cima conforme `open` vai a 1).
pub(crate) fn chevron(cv: &mut Canvas, b: RectF, open: f32, c: Rgb, a: f32, s: f32) {
    // Vira como uma dobradiça: as pontas descem enquanto o bico sobe (fica
    // reta no meio do caminho), em vez de girar de lado.
    let k = (std::f32::consts::PI * open.clamp(0.0, 1.0)).cos();
    let em = 15.0 * s;
    let (w, h) = (b.w.round().max(1.0) as usize, b.h.round().max(1.0) as usize);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0 + 0.02 * em);
    let (dx, dy, hw) = (0.34 * em, 0.16 * em * k, 0.075 * em);
    let pts = [(cx - dx, cy - dy), (cx, cy + dy), (cx + dx, cy - dy)];
    let mut cov = vec![0u8; w * h];
    let (y0, y1) = ((cy - 0.16 * em - hw - 1.0).floor().max(0.0) as usize, ((cy + 0.16 * em + hw + 1.0).ceil() as usize).min(h));
    let (x0, x1) = ((cx - dx - hw - 1.0).floor().max(0.0) as usize, ((cx + dx + hw + 1.0).ceil() as usize).min(w));
    for y in y0..y1 {
        for x in x0..x1 {
            let mut acc = 0.0;
            for (ox, oy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                acc += (0.5 - sd_path((x as f32 + ox, y as f32 + oy), &pts, hw)).clamp(0.0, 1.0);
            }
            cov[y * w + x] = (acc / 4.0 * 255.0 + 0.5) as u8;
        }
    }
    cv.blit_mask(b.x.round() as i32, b.y.round() as i32, &Mask { w, h, cov }, c, a);
}

/// Altura do mixer aberto com `n` linhas à vista.
fn mixer_height(n: usize, s: f32) -> f32 {
    32.0 * s + n.max(1) as f32 * 36.0 * s + 6.0 * s
}

/// Mixer de volume: cabeçalho com o dispositivo de saída, uma linha por app
/// (ícone, nome, slider, mudo) e a setinha que abre e fecha.
fn mixer_section(cv: &mut Canvas, tr: &mut TextRenderer, l: &CLayout, f: &CardFrame, mx: &crate::shared::MixerState) {
    let (s, th) = (f.s, &f.theme);
    let it = f.it.styled(f.cfg);
    if l.mixer_area.h > 0.5 {
        let a = l.mixer_open;
        let clip = cv.clip_rect();
        cv.set_clip(clip.intersect(l.mixer_area));
        let top = l.mixer_area.y;
        cv.fill_rrect(l.card.x + 16.0 * s, top, l.card.right() - 16.0 * s, top + s, 0.5 * s, (if th.dark { 0.10 } else { 0.08 }) * a, th.fg);
        let hd = l.mixer_head;
        text(cv, tr, crate::i18n::tr("VOLUME POR APP"), Font::Label, hd.x, hd.y, hd.w * 0.5, hd.h, th.fg, 0.62 * a);
        if !mx.device.is_empty() {
            // Saída padrão: clica para escolher outra (seta para baixo ao lado).
            let hot = f.it.hover == Some(Hit::MixDevice);
            let max_w = hd.w * 0.55 - 20.0 * s;
            let dev_w = tr.mask(&mx.device, Font::Small, max_w as i32, hd.h as i32, Align::Left).used_width().min(max_w);
            let chip = RectF::new(hd.right() - dev_w - 24.0 * s, hd.y - 2.0 * s, dev_w + 28.0 * s, hd.h + 4.0 * s);
            if hot {
                hover_bg(cv, chip, 6.0 * s, th, f.it.pressed == Some(Hit::MixDevice));
            }
            text(cv, tr, &mx.device, Font::Small, chip.x + 6.0 * s, hd.y, dev_w, hd.h, th.fg, (if hot { 0.85 } else { 0.6 }) * a);
            let arrow = RectF::new(chip.right() - 21.0 * s, hd.y + 1.0 * s, 18.0 * s, hd.h - 2.0 * s);
            chevron(cv, arrow, 0.0, th.fg, (if hot { 0.85 } else { 0.55 }) * a, s * 0.75);
        }
        if mx.apps.is_empty() {
            if let Some((row, _, _)) = l.mixer_rows.first().copied().or(Some((RectF::new(hd.x, hd.bottom() + 4.0 * s, hd.w, 34.0 * s), RectF::default(), RectF::default()))) {
                text(cv, tr, crate::i18n::tr("Nenhum app com som agora"), Font::Caption, row.x, row.y, row.w, row.h, th.fg, 0.55 * a);
            }
        }
        // Linhas: só as que estão na faixa à vista (a lista rola).
        cv.set_clip(clip.intersect(l.mixer_area).intersect(l.mixer_list));
        for (i, (app, (row, slider_r, mute_r))) in mx.apps.iter().zip(&l.mixer_rows).enumerate() {
            if row.bottom() < l.mixer_list.y || row.y > l.mixer_list.bottom() {
                continue;
            }
            // Ícone, nome (apagado se o app não está tocando agora) e slider.
            let k = (18.0 * s).round();
            let ir = RectF::new(row.x.round(), (row.cy() - k / 2.0).round(), k, k);
            match (&app.icon, app.system) {
                (Some(img), _) => cv.draw_image(ir, img, 0.0, a),
                _ => glyph_pose(cv, tr, ir.inset(-2.0 * s), G_VOL3, Font::IconSmall, th.fg, 0.8 * a, Pose::REST, vec_em(&it, Font::IconSmall, s)),
            }
            let name = if app.system { crate::i18n::tr("Sons do sistema") } else { app.name.as_str() };
            text(cv, tr, name, Font::Caption, ir.right() + 10.0 * s, row.y, slider_r.x - ir.right() - 18.0 * s, row.h, th.fg, (if app.active { 0.95 } else { 0.62 }) * a);
            let hot = matches!(f.it.hover, Some(Hit::MixVol(h)) if h as usize == i) || matches!(f.it.drag, Some(Hit::MixVol(h)) if h as usize == i);
            if let Some((x0, x1)) = l.mix_track(i, s) {
                // Mesma cor do slider de volume principal, logo acima.
                let v = mx.fill.get(i).copied().unwrap_or(if app.muted { 0.0 } else { app.volume });
                slider(cv, x0, x1, slider_r.cy(), v, th, hot, hot, s);
            }
            let mh = Hit::MixMute(i as u8);
            if f.it.hover == Some(mh) {
                let kk = mute_r.h - 6.0 * s;
                hover_bg(cv, RectF::new(mute_r.cx() - kk / 2.0, mute_r.cy() - kk / 2.0, kk, kk), kk / 2.0, th, f.it.pressed == Some(mh));
            }
            let g = if app.muted || app.volume <= 0.0 { G_MUTE } else { G_VOL2 };
            ctrl_glyph(cv, tr, *mute_r, g, Font::IconSmall, th.fg, 0.85 * a, &it, mh, s);
        }
        // Barra fina de rolagem, com mais apps do que linhas.
        if l.mixer_scroll_max > 0.0 {
            let list = l.mixer_list;
            let track = RectF::new(l.card.right() - 8.0 * s, list.y + 4.0 * s, 3.0 * s, list.h - 8.0 * s);
            let n = mx.apps.len().max(1) as f32;
            let shown = (list.h / (36.0 * s)).floor().max(1.0);
            let th_h = (track.h * shown / n).max(18.0 * s).min(track.h);
            let t = l.mixer_scroll / (l.mixer_scroll_max * s).max(1.0);
            let y = track.y + (track.h - th_h) * t.clamp(0.0, 1.0);
            cv.fill_rrect(track.x, y, track.right(), y + th_h, 1.5 * s, (if th.dark { 0.30 } else { 0.25 }) * a, th.fg);
        }
        cv.set_clip(clip);
    }
    // Setinha (vira para cima com o mixer aberto).
    if let Some(b) = l.mixer_btn {
        let hot = f.it.hover == Some(Hit::MixToggle);
        if hot {
            hover_bg(cv, b.inset(2.0 * s), b.h / 2.0, th, f.it.pressed == Some(Hit::MixToggle));
        }
        chevron(cv, b, l.mixer_open, th.fg, if hot { 0.9 } else { 0.55 }, s);
    }
}

/// Dica no estilo do Windows 11 acima de `anchor` (dentro do canvas), com
/// opacidade `a`: aparece subindo um pouco.
fn tooltip(cv: &mut Canvas, tr: &mut TextRenderer, msg: &str, anchor: RectF, th: &Theme, a: f32, s: f32) {
    let h = 28.0 * s;
    let cw = cv.w as f32;
    let tw = tr.mask(msg, Font::Caption, (cw - 60.0 * s) as i32, h as i32, Align::Left).used_width();
    let w = (tw + 22.0 * s).round().min(cw - 16.0 * s);
    let x = (anchor.cx() - w / 2.0).clamp(8.0 * s, cw - 8.0 * s - w);
    let y = anchor.y - h - 4.0 * s + (1.0 - ease_enter(a)) * 4.0 * s;
    let r = RectF::new(x.round(), y.round(), w, h);
    let radius = 6.0 * s;
    cv.drop_shadow(r, radius, 12.0 * s, 3.0 * s, (if th.dark { 0.45 } else { 0.18 }) * a);
    cv.fill_round(r, radius, if th.dark { [0.17, 0.17, 0.18] } else { [0.985, 0.985, 0.99] }, a);
    let line: Rgb = if th.dark { [1.0; 3] } else { [0.0; 3] };
    cv.stroke_round(r, radius, s, line, (if th.dark { 0.12 } else { 0.10 }) * a);
    let fg: Rgb = if th.dark { [1.0; 3] } else { [0.1, 0.1, 0.1] };
    text_center(cv, tr, msg, Font::Caption, r, fg, 0.92 * a);
}

/// Linha de atalhos: ícone de cada app de música; o nome do que está sob o
/// mouse ao lado.
fn launcher_row(cv: &mut Canvas, tr: &mut TextRenderer, l: &CLayout, f: &CardFrame) {
    let (s, th) = (f.s, &f.theme);
    // Linha fina separando dos controles.
    let y = l.launch[0].y - 1.0 * s;
    cv.fill_rrect(l.card.x + 16.0 * s, y, l.card.right() - 16.0 * s, y + 1.0 * s, 0.5 * s, if th.dark { 0.10 } else { 0.08 }, th.fg);
    let n = l.launch.len().min(f.launchers.len());
    // Arrastando um atalho: os outros abrem espaço onde ele vai cair.
    let dragging = match f.it.drag {
        Some(Hit::Launch(i)) if (i as usize) < n => Some((i as usize, launch_drop_slot(&l.launch[..n], f.it.drag_x))),
        _ => None,
    };
    let mut label: Option<&str> = None;
    for i in 0..n {
        let app = &f.launchers[i];
        let h = Hit::Launch(i as u8);
        let slot = match dragging {
            Some((from, _)) if i == from => continue,
            Some((from, to)) if from < to && i > from && i <= to => i - 1,
            Some((from, to)) if to < from && i >= to && i < from => i + 1,
            _ => i,
        };
        let r = l.launch[slot];
        let hot = f.it.hover == Some(h) && dragging.is_none();
        if hot {
            label = Some(&app.name);
            hover_bg(cv, r.inset(2.0 * s), 8.0 * s, th, f.it.pressed == Some(h));
        }
        let k = if f.it.pressed == Some(h) && hot { 18.0 * s } else { 20.0 * s };
        launcher_icon(cv, tr, RectF::new(r.cx() - k / 2.0, r.cy() - k / 2.0, k, k), app, th, 1.0);
    }
    // O arrastado vai por cima, um pouco maior e com sombra, seguindo o mouse.
    if let Some((from, _)) = dragging {
        let app = &f.launchers[from];
        label = Some(&app.name);
        let r = l.launch[from];
        let cx = f.it.drag_x.clamp(l.launch[0].cx(), l.launch[n - 1].cx());
        let k = 23.0 * s;
        let ir = RectF::new(cx - k / 2.0, r.cy() - k / 2.0 - 2.0 * s, k, k);
        cv.drop_shadow(ir, 6.0 * s, 8.0 * s, 3.0 * s, 0.45);
        launcher_icon(cv, tr, ir, app, th, 1.0);
    }
    // Parado, só os ícones; o nome aparece com o mouse em cima (ou arrastando).
    if let Some(msg) = label {
        text(cv, tr, msg, Font::Small, l.launch_label.x, l.launch_label.y, l.launch_label.w, l.launch_label.h, th.fg, 0.85);
    }
}

/// Em qual posição da linha de atalhos cai um atalho solto no x `x`.
pub fn launch_drop_slot(slots: &[RectF], x: f32) -> usize {
    slots.iter().enumerate().min_by(|a, b| (a.1.cx() - x).abs().total_cmp(&(b.1.cx() - x).abs())).map(|(i, _)| i).unwrap_or(0)
}

/// Ícone de um app de música (ou a inicial dele num círculo).
fn launcher_icon(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, app: &crate::shared::Launcher, th: &Theme, a: f32) {
    let ir = RectF::new(r.x.round(), r.y.round(), r.w.round(), r.h.round());
    match &app.icon {
        Some(img) => cv.draw_image(ir, img, 0.0, a),
        None => {
            cv.fill_round(ir, ir.w / 2.0, th.accent, 0.9 * a);
            let first: String = app.name.chars().take(1).collect();
            text_center(cv, tr, &first, Font::Caption, ir, th.on_accent, a);
        }
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
    let key = layer_key((
        (cfg.card_style as u8, rect_bits(card), radius.to_bits(), card_look(cfg)),
        media.art.as_ref().map(|a| a.id),
        (th.dark, th.accent.map(f32::to_bits), colors.accent.map(f32::to_bits)),
        backdrop.map(|b| b.id),
    ));
    cached_layer(cv, LayerTag::UpNext, key, 1.0, |cv| card_background(cv, card, radius, cfg, colors, media, th, s, backdrop));

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
            // Selo do app no canto da capa, como no card.
            if cfg.show_source {
                let b = (18.0 * s * k).round();
                let (cx, cy) = (art.right() - b * 0.22, art.bottom() - b * 0.22);
                card_badge(cv, RectF::new(cx - b / 2.0, cy - b / 2.0, b, b), media, th, s);
            }
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
        web_shuffle: false,
        web_repeat: false,
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

    /// Na barra, "Quando o player permite" esconde o aleatório e o repetir de
    /// um player que não deixa mudá-los (um vídeo do YouTube no navegador);
    /// "Sempre" os mantém; com a opção da barra desligada, nunca aparecem.
    #[test]
    fn aleatorio_e_repetir_na_barra_seguem_o_player() {
        let mut m = sample_media();
        (m.can_shuffle, m.can_repeat) = (false, false);
        let mut cfg = Config { taskbar_shuffle_repeat: true, card_shuffle_repeat: ShuffleRepeat::WhenSupported, ..Config::default() };
        assert_eq!(bar_toggles(&cfg, &m), (false, false));
        let l = WLayout::build_for(1.0, 40.0, &cfg, true, false, 0, bar_toggles(&cfg, &m));
        assert!(l.shuffle.is_none() && l.repeat.is_none() && l.prev.is_some());
        m.can_repeat = true;
        assert_eq!(bar_toggles(&cfg, &m), (false, true));
        cfg.card_shuffle_repeat = ShuffleRepeat::Always;
        assert_eq!(bar_toggles(&cfg, &m), (true, true));
        cfg.taskbar_shuffle_repeat = false;
        assert_eq!(bar_toggles(&cfg, &m), (false, false));
    }

    /// Capa nova no mesmo endereço de uma já liberada não herda o desfoque
    /// dela (o cache era pelo endereço).
    #[test]
    fn desfoque_nao_herda_a_capa_liberada() {
        for _ in 0..20 {
            let red = Arc::new(Image::new(64, 64, vec![0xffff_0000; 64 * 64]));
            let _ = blurred(&red);
            drop(red);
            let blue = Arc::new(Image::new(64, 64, vec![0xff00_00ff; 64 * 64]));
            let b = blurred(&blue);
            let p = b.px[b.px.len() / 2];
            assert!(p & 0xff > (p >> 16) & 0xff, "desfoque da capa antiga: {p:08x}");
        }
    }

    /// Só as barras mudando: refazer só a área delas dá exatamente o mesmo
    /// quadro que redesenhar o card inteiro com os valores novos.
    #[test]
    fn barras_sozinhas_igual_ao_card_inteiro() {
        let cfg = Config { card_viz: true, ..Config::default() };
        let media = sample_media();
        let colors = Colors { palette: cfg.palette, accent: [0.3, 0.5, 0.9], accent_light: [0.6, 0.8, 1.0], light_taskbar: false, album: None };
        let mut tr = TextRenderer::new();
        let l = CLayout::build(1.0, &cfg, true);
        let (w, h) = (l.w.ceil() as usize, l.h.ceil() as usize);
        let th = card_theme(&cfg, &colors, &media);
        let draw = |tr: &mut TextRenderer, px: &mut Vec<u32>, viz: &[f32]| {
            let f = CardFrame { cfg: &cfg, colors: &colors, media: &media, theme: th, it: Interact::default(), pb: Playback::default(), s: 1.0, backdrop: None, hint: None, launchers: &[], viz: Some(viz), mixer: None };
            let mut cv = Canvas::new(w, h, px);
            cv.clear();
            draw_card(&mut cv, tr, &l, &f);
        };
        let (a, b): (Vec<f32>, Vec<f32>) = ((0..32).map(|i| (i % 7) as f32 / 7.0).collect(), (0..32).map(|i| 1.0 - (i % 5) as f32 / 5.0).collect());
        let mut whole = vec![0u32; w * h];
        draw(&mut tr, &mut whole, &b);
        let mut quick = vec![0u32; w * h];
        draw(&mut tr, &mut quick, &a);
        assert!(card_bars_update(&mut Canvas::new(w, h, &mut quick), &b, &th, 1.0).is_some());
        assert!(quick == whole, "as barras sozinhas deram um quadro diferente do inteiro");
    }

    /// Mini barras do card com qualquer quantidade de barras do visualizador
    /// (8 a 11, 15, 16... davam pânico ao cortar a faixa).
    #[test]
    fn barras_do_card_com_qualquer_quantidade() {
        let th = Theme { fg: [1.0; 3], accent: [1.0, 0.4, 0.6], on_accent: [0.0; 3], dark: true };
        let mut px = vec![0u32; 60 * 30];
        for n in 1..=128 {
            let values: Vec<f32> = (0..n).map(|i| i as f32 / n as f32).collect();
            card_bars(&mut Canvas::new(60, 30, &mut px), RectF::new(5.0, 5.0, 40.0, 20.0), &values, &th, 1.0);
        }
        // E pelo card inteiro, com cada opção de barras das configurações.
        let cfg = Config { card_viz: true, ..Config::default() };
        let media = sample_media();
        let colors = Colors { palette: cfg.palette, accent: [0.3, 0.5, 0.9], accent_light: [0.6, 0.8, 1.0], light_taskbar: false, album: None };
        let mut tr = TextRenderer::new();
        let l = CLayout::build(1.0, &cfg, true);
        let (w, h) = (l.w.ceil() as usize, l.h.ceil() as usize);
        let mut px = vec![0u32; w * h];
        for n in [6usize, 8, 10, 12, 16, 24, 32] {
            let viz = vec![0.7; n];
            let f = CardFrame { cfg: &cfg, colors: &colors, media: &media, theme: card_theme(&cfg, &colors, &media), it: Interact::default(), pb: Playback::default(), s: 1.0, backdrop: None, hint: None, launchers: &[], viz: Some(&viz), mixer: None };
            draw_card(&mut Canvas::new(w, h, &mut px), &mut tr, &l, &f);
        }
    }
    use crate::config::Config;

    /// Na pílula, o canto da capa fica dentro da ponta arredondada em todos os
    /// formatos e tamanhos de capa, alturas de barra e escalas.
    #[test]
    fn pill_keeps_cover_inside() {
        use crate::config::WidgetShape;
        for s in [1.0f32, 1.25, 1.5, 2.0] {
            for bar in [40.0f32, 48.0, 56.0] {
                for shape in [Shape::Rounded, Shape::Square, Shape::Circle] {
                    for size in [Size3::Small, Size3::Normal, Size3::Large] {
                        let cfg = Config { widget_shape: WidgetShape::Pill, widget_pill: true, cover_shape: shape, cover_size: size, ..Config::default() };
                        let l = WLayout::build(s, bar * s, &cfg, true, false);
                        let (bg, art) = (l.bg.unwrap(), l.art.unwrap());
                        let rc = cover_radius(shape, art.w, s);
                        // Pontos da borda da capa (arcos dos cantos) dentro da pílula.
                        for k in 0..=32 {
                            let t = k as f32 / 32.0 * std::f32::consts::FRAC_PI_2;
                            for (cx, cy, sx, sy) in [(art.x + rc, art.y + rc, -1.0, -1.0), (art.x + rc, art.bottom() - rc, -1.0, 1.0)] {
                                let (px, py) = (cx + sx * rc * t.cos(), cy + sy * rc * t.sin());
                                let q = ((px - bg.cx()).abs() - (bg.w / 2.0 - l.radius), (py - bg.cy()).abs() - (bg.h / 2.0 - l.radius));
                                let d = rrect_sd(q.0, q.1, l.radius);
                                assert!(d <= 0.01, "{shape:?} {size:?} @{s}x barra {bar}: canto {d:.2} px para fora");
                            }
                        }
                    }
                }
            }
        }
    }

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
            let f = CardFrame { cfg: &cfg, colors: &colors, media: &media, theme: card_theme(&cfg, &colors, &media), it: Interact::default(), pb: Playback { volume: 0.5, ..Default::default() }, s: 1.0, backdrop: None, hint: None, launchers: &[], viz: None, mixer: None };
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
                let f = CardFrame { cfg: &ucfg, colors: &colors, media: &m2, theme: card_theme(&ucfg, &colors, &m2), it: Interact::default(), pb: Playback { volume: 0.6, pos, ..Default::default() }, s: 1.0, backdrop: None, hint: None, launchers: &[], viz: None, mixer: None };
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
