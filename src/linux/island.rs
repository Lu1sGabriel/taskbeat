//! A ilha no topo da tela (o mesmo desenho do notch do Mac, sem notch):
//! fechada é uma pílula com a capa, o título e as barras; com o mouse em cima
//! (ou um clique) cresce para baixo com o card; numa troca automática de
//! faixa mostra o "A seguir".
//!
//! A lógica (animações, mouse, quadros) é a mesma de `mac/notch.rs`; muda só
//! a janela (`window.rs`) e o laço da thread principal, que fica parado no
//! `poll` quando nada muda.
//!
//! Desempenho: fechada e parada, nada roda; tocando, o quadro é refeito a
//! 30 fps só na área da ilha fechada (capa + barras); as animações seguem o
//! vblank do compositor e param quando terminam.

use super::window::{self, WinEv, Window};
use super::{App, audio, system};
use crate::config::Config;
use crate::dsp::{Analyzer, FFT_SIZE};
use crate::notch::{self as nd, Geom, NLayout, NotchFrame};
use crate::render::{Canvas, Colors, RectF};
use crate::shared::{MediaCmd, MediaInfo, Shared};
use crate::text::TextRenderer;
use crate::ui::{Hit, IconAnims, Interact, Playback};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

const OPEN_DUR: f32 = 0.48;
const CLOSE_DUR: f32 = 0.30;
/// Mouse parado sobre a ilha por este tempo: abre (com "Abrir ao passar o mouse").
const HOVER_DELAY: Duration = Duration::from_millis(140);
/// Mouse fora do card aberto por este tempo: fecha.
const LEAVE_DELAY: Duration = Duration::from_millis(320);
const CHANGE_DUR: f32 = 0.55;
const HINT_DUR: f32 = 2.4;
/// Espera a capa nova antes de mostrar o "A seguir".
const PEEK_ART_WAIT: Duration = Duration::from_millis(1500);
/// Altura da pílula fechada (pontos): a de uma barra de topo.
const BAND: f32 = 30.0;

/// Valor animado entre `from` e `to` com a curva de abrir (com quique) ou fechar.
#[derive(Clone, Copy)]
struct Anim {
    from: f32,
    to: f32,
    start: Instant,
    dur: f32,
}

impl Anim {
    fn new() -> Self {
        Self { from: 0.0, to: 0.0, start: Instant::now(), dur: 0.0 }
    }

    fn u(&self, now: Instant) -> f32 {
        if self.dur <= 0.0 { 1.0 } else { (now.saturating_duration_since(self.start).as_secs_f32() / self.dur).min(1.0) }
    }

    fn value(&self, now: Instant) -> f32 {
        let u = self.u(now);
        let k = if self.to > self.from { nd::grow(u) } else { nd::shrink(u) };
        self.from + (self.to - self.from) * k
    }

    fn done(&self, now: Instant) -> bool {
        self.u(now) >= 1.0
    }

    fn set(&mut self, to: f32, now: Instant) {
        if (to - self.to).abs() < 1e-4 {
            return;
        }
        let v = self.value(now).clamp(0.0, 1.0);
        let dur = if to > v { OPEN_DUR } else { CLOSE_DUR };
        *self = Self { from: v, to, start: now, dur: dur * (to - v).abs().max(0.35) };
    }
}

/// Volume da saída (lido dos avisos do servidor de som).
#[derive(Default)]
struct Volume {
    level: f32,
    muted: bool,
}

impl Volume {
    fn poll(&mut self) {
        if let Some((l, m)) = audio::volume() {
            self.level = l;
            self.muted = m;
        }
    }

    fn set_level(&mut self, v: f32) {
        self.level = v.clamp(0.0, 1.0);
        self.muted = false;
        audio::set_volume(self.level);
    }

    fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        audio::set_muted(self.muted);
    }
}

pub struct Island {
    shared: Arc<Shared>,
    win: Box<dyn Window>,
    tr: TextRenderer,
    s: f32,
    layout: NLayout,
    buf: Vec<u32>,
    cfg: Config,
    cfg_version: u64,
    colors: Colors,
    media: MediaInfo,
    media_version: u64,
    open: Anim,
    open_target: bool,
    peek: Anim,
    peek_start: Option<Instant>,
    peek_until: Option<Instant>,
    peek_pending: Option<Instant>,
    change: Option<(MediaInfo, Instant)>,
    hint: Option<(&'static str, Instant)>,
    it: Interact,
    icons: IconAnims,
    seek_preview: Option<f32>,
    hover_since: Option<Instant>,
    leave_since: Option<Instant>,
    inside: bool,
    input: Option<RectF>,
    input_set: bool,
    analyzer: Analyzer,
    samples: Vec<f32>,
    last_written: u64,
    last_audio: Instant,
    volume: Volume,
    last_volume_poll: Instant,
    t0: Instant,
    last_frame: Instant,
    last_sig: Option<u64>,
    last_still: Option<u64>,
    last_pos_sec: u64,
    /// Quadro desenhado mas ainda não mostrado (a janela não estava pronta).
    dirty: bool,
    full: bool,
    hidden: bool,
    /// Quadros por segundo pedidos (0 = parado).
    rate: u32,
    demo: bool,
    /// Diagnóstico (`TASKBEAT_PERF=1`): quadros desenhados e laços desde a última conta.
    perf: Option<(Instant, u32, u32, Duration)>,
}

fn demo(var: &str) -> bool {
    std::env::var_os(var).is_some()
}

impl Island {
    fn new(shared: Arc<Shared>, win: Box<dyn Window>) -> Self {
        let cfg = shared.config();
        let s = win.scale();
        let mut tr = TextRenderer::new();
        tr.set_scale(s);
        let rate = shared.sample_rate.load(Ordering::Relaxed);
        let media = shared.media.lock().unwrap().clone();
        let mut n = Self {
            tr,
            s,
            layout: NLayout::default(),
            buf: Vec::new(),
            analyzer: Analyzer::new(cfg.bars, rate),
            colors: system::colors(cfg.palette),
            cfg_version: shared.config_version.load(Ordering::Relaxed),
            media,
            media_version: shared.media_version.load(Ordering::Relaxed),
            cfg,
            shared,
            win,
            open: Anim::new(),
            open_target: false,
            peek: Anim::new(),
            peek_start: None,
            peek_until: None,
            peek_pending: None,
            change: None,
            hint: None,
            it: Interact::default(),
            icons: IconAnims::default(),
            seek_preview: None,
            hover_since: None,
            leave_since: None,
            inside: false,
            input: None,
            input_set: false,
            samples: vec![0.0; FFT_SIZE],
            last_written: 0,
            last_audio: Instant::now(),
            volume: Volume::default(),
            last_volume_poll: Instant::now(),
            t0: Instant::now(),
            last_frame: Instant::now(),
            last_sig: None,
            last_still: None,
            last_pos_sec: 0,
            dirty: false,
            full: true,
            hidden: true,
            rate: 0,
            demo: demo("TV_DEMO_MEDIA"),
            perf: demo("TASKBEAT_PERF").then(|| (Instant::now(), 0, 0, Duration::ZERO)),
        };
        n.relayout();
        if n.demo {
            n.media = crate::ui::sample_media();
        }
        if demo("TV_DEMO_FLYOUT") {
            n.open_target = true;
            n.open = Anim { from: 1.0, to: 1.0, start: Instant::now(), dur: 0.0 };
            n.volume.level = 0.62;
        }
        if demo("TV_DEMO_POPUP") {
            let now = Instant::now();
            n.peek = Anim { from: 1.0, to: 1.0, start: now, dur: 0.0 };
            n.peek_start = Some(now);
            n.peek_until = Some(now + Duration::from_secs(3600));
        }
        n.volume.poll();
        n.update_visibility();
        n
    }

    fn relayout(&mut self) {
        let s = self.s;
        let geom = Geom { notch_w: 0.0, notch_h: (BAND * s).round() };
        let mut l = NLayout::build(s, geom, &self.cfg, true);
        // O buffer tem um número inteiro de pontos (a superfície também).
        let (w, h) = self.win.fit(l.w, l.h);
        l.w = w as f32;
        l.h = h as f32;
        // Onde fica cada coisa (o teste de ponta a ponta clica nos botões).
        let c = |r: RectF| format!("{:.0},{:.0}", r.cx(), r.cy());
        system::log::info(&format!(
            "ilha: tamanho {w}x{h} escala {s} fechada {} aleatório {} anterior {} play {} próxima {} repetir {}",
            c(l.closed),
            c(l.shuffle),
            c(l.prev),
            c(l.play),
            c(l.next),
            c(l.repeat)
        ));
        self.layout = l;
        self.buf = vec![0; w * h];
        self.win.set_size(w, h);
        self.last_sig = None;
        self.full = true;
        self.input_set = false;
    }

    fn rescale(&mut self, s: f32) {
        if (s - self.s).abs() < 0.001 {
            return;
        }
        self.s = s;
        self.tr.set_scale(s);
        self.relayout();
    }

    fn pull_media(&mut self) {
        let v = self.shared.media_version.load(Ordering::Relaxed);
        if v == self.media_version || self.demo {
            return;
        }
        self.media_version = v;
        let m = self.shared.media.lock().unwrap().clone();
        let now = Instant::now();
        // "A seguir": o player trocou de faixa sozinho.
        if m.auto_change_seq != self.media.auto_change_seq && self.cfg.next_track_popup && !self.open_target {
            self.peek_pending = Some(now);
        }
        // Troca de música: título desliza e a capa nova aparece.
        let same = m.title == self.media.title && m.artist == self.media.artist;
        if !same && self.media.present() && m.present() && self.cfg.track_anim {
            self.change = Some((self.media.clone(), now));
        }
        self.media = m;
        self.update_visibility();
    }

    fn pull_config(&mut self) {
        let v = self.shared.config_version.load(Ordering::Relaxed);
        if v == self.cfg_version {
            return;
        }
        self.cfg_version = v;
        let old_fs = self.cfg.hide_on_fullscreen;
        self.cfg = self.shared.config();
        self.colors = system::colors(self.cfg.palette);
        let rate = self.shared.sample_rate.load(Ordering::Relaxed);
        self.analyzer.configure(self.cfg.bars, rate);
        if old_fs != self.cfg.hide_on_fullscreen {
            self.win.set_above_fullscreen(!self.cfg.hide_on_fullscreen);
        }
        crate::i18n::apply(self.cfg.language);
        self.relayout();
        self.update_visibility();
    }

    /// Esconde a ilha com "Mostrar música" desligado, ou sem música com
    /// "Sempre visível" desligado. Escondida = quadro transparente e sem
    /// área de clique (a superfície continua lá, sem custo).
    fn update_visibility(&mut self) {
        let want = self.cfg.show_media && (self.media.present() || self.cfg.always_show_widget || self.open_target || self.peek_until.is_some());
        if want == !self.hidden {
            return;
        }
        self.hidden = !want;
        self.last_sig = None;
        self.full = true;
        self.update_input();
    }

    fn set_open(&mut self, open: bool) {
        if open == self.open_target {
            return;
        }
        let now = Instant::now();
        self.open_target = open;
        system::log::info(if open { "ilha: card aberto" } else { "ilha: card fechado" });
        self.open.set(if open { 1.0 } else { 0.0 }, now);
        if open {
            self.peek.set(0.0, now);
            self.peek_until = None;
            self.peek_pending = None;
            audio::refresh_volume();
            self.volume.poll();
            self.last_volume_poll = now;
            self.shared.send_media(MediaCmd::Refresh);
        } else {
            self.it = Interact::default();
            self.seek_preview = None;
        }
        // Barras no card aberto mesmo com o visualizador da ilha desligado.
        self.shared.card_bars.store(open && self.cfg.show_visualizer, Ordering::Relaxed);
        self.hover_since = None;
        self.leave_since = None;
        self.update_visibility();
    }

    fn toggle_open(&mut self) {
        let o = !self.open_target;
        self.set_open(o);
    }

    /// Avança os temporizadores (abrir com o mouse, fechar, "A seguir").
    fn step(&mut self) {
        let now = Instant::now();
        if let Some(t) = self.hover_since {
            if now - t >= HOVER_DELAY && self.inside && self.cfg.notch_hover {
                self.set_open(true);
            }
        }
        if let Some(t) = self.leave_since {
            if now - t >= LEAVE_DELAY && !self.inside && self.it.drag.is_none() {
                self.set_open(false);
            }
        }
        if let Some(t) = self.peek_pending {
            if !self.media.art_pending || now - t >= PEEK_ART_WAIT {
                self.peek_pending = None;
                if !self.open_target && self.media.present() {
                    self.peek.set(1.0, now);
                    self.peek_start = Some(now);
                    self.peek_until = Some(now + Duration::from_secs(self.cfg.popup_secs.max(2) as u64));
                    self.update_visibility();
                }
            }
        }
        if let Some(u) = self.peek_until {
            if now >= u && !(self.inside && !self.open_target) {
                self.peek.set(0.0, now);
                self.peek_until = None;
            }
        }
        if self.change.as_ref().is_some_and(|(_, t)| (now - *t).as_secs_f32() > CHANGE_DUR) {
            self.change = None;
        }
        if self.hint.is_some_and(|(_, t)| (now - t).as_secs_f32() > HINT_DUR) {
            self.hint = None;
        }
        if self.peek.done(now) && self.peek.to == 0.0 && self.peek_until.is_none() && self.peek_start.is_some() {
            self.peek_start = None;
            self.update_visibility();
        }
        if self.open_target && now - self.last_volume_poll > Duration::from_millis(500) {
            self.volume.poll();
            self.last_volume_poll = now;
        }
    }

    fn animating(&self, now: Instant) -> bool {
        !self.open.done(now)
            || !self.peek.done(now)
            || self.change.is_some()
            || self.hint.is_some()
            || self.hover_since.is_some()
            || self.leave_since.is_some()
            || self.peek_pending.is_some()
            || self.it.drag.is_some()
            || (self.cfg.icon_anim && self.icons.running())
    }

    /// Algo muda sozinho com o tempo (barras, indicador, tempo da música)?
    fn continuous(&self) -> bool {
        if self.hidden || !self.media.present() {
            return false;
        }
        let playing = self.media.playing;
        (playing && (self.cfg.show_visualizer || self.cfg.playing_indicator)) || self.peek_until.is_some() || (self.open_target && playing)
    }

    fn schedule(&mut self) {
        let now = Instant::now();
        self.rate = if self.animating(now) {
            120
        } else if self.continuous() {
            30
        } else {
            0
        };
    }

    /// Quanto o `poll` pode esperar (ms; -1 = até algo acontecer).
    fn timeout(&self) -> i32 {
        // Sem superfície (monitor removido): nada de quadros nem do `dirty` de
        // 100 ms; o `global` de uma tela nova, ou o `configure`, acorda o `poll`.
        // Antes isso girava o laço a 100% (o `due` do quadro já tinha passado).
        if self.win.suspended() {
            return -1;
        }
        let mut t: i64 = -1;
        if self.rate > 0 {
            let due = self.last_frame + Duration::from_secs_f32(1.0 / self.rate as f32);
            // Arredonda para cima: o `poll` não acorda antes da hora (e o laço não gira à toa).
            t = due.saturating_duration_since(Instant::now()).as_micros().div_ceil(1000) as i64;
        }
        if self.open_target {
            t = if t < 0 { 500 } else { t.min(500) };
        }
        if self.dirty {
            t = if t < 0 { 100 } else { t.min(100) };
        }
        t.min(i32::MAX as i64) as i32
    }

    // ------------------------------------------------------------ quadro

    /// Parte visível da ilha agora (onde o mouse conta).
    fn hot_rect(&self) -> RectF {
        let now = Instant::now();
        let (shape, _) = self.layout.shape(self.open.value(now).max(0.0), self.peek.value(now).max(0.0));
        shape.inset(-4.0 * self.s)
    }

    fn update_input(&mut self) {
        let r = (!self.hidden).then(|| self.hot_rect());
        let same = match (r, self.input) {
            (Some(a), Some(b)) => (a.x - b.x).abs() < 1.0 && (a.y - b.y).abs() < 1.0 && (a.w - b.w).abs() < 1.0 && (a.h - b.h).abs() < 1.0,
            (None, None) => true,
            _ => false,
        };
        if !same || !self.input_set {
            self.input = r;
            self.input_set = true;
            self.win.set_input(r);
        }
    }

    fn render(&mut self, force: bool) {
        if !self.win.ready() {
            self.dirty = true;
            return;
        }
        // No ritmo pedido: 30 quadros/s para as barras, 120 nas animações
        // (cada vblank do compositor acorda o laço; não desenha em todos).
        // Antes de tudo: o espectro avança uma vez por quadro desenhado.
        let now = Instant::now();
        let min_dt = if self.animating(now) { 1.0 / 125.0 } else { 1.0 / 31.0 };
        if !force && !self.full && self.rate > 0 && (now - self.last_frame).as_secs_f32() < min_dt {
            return;
        }
        if self.media.present() {
            let st = Playback { playing: self.media.playing, shuffle: self.media.shuffle, repeat: self.media.repeat, muted: self.volume.muted, ..Default::default() };
            self.icons.observe(&st);
        } else {
            self.icons.reset_seen();
        }
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        let time = (now - self.t0).as_secs_f32();
        let open = self.open.value(now).max(0.0);
        let peek = self.peek.value(now).max(0.0);
        let p = open.max(peek);

        // Áudio -> espectro.
        let viz_on = self.cfg.show_visualizer && self.media.present();
        if viz_on {
            let rate = self.shared.sample_rate.load(Ordering::Relaxed);
            self.analyzer.configure(self.cfg.bars, rate);
            {
                let ring = self.shared.audio.lock().unwrap();
                if ring.written != self.last_written {
                    self.last_written = ring.written;
                    self.last_audio = now;
                    ring.latest(&mut self.samples);
                }
            }
            let media_ok = !self.cfg.only_when_media || self.media.playing;
            if now - self.last_audio > Duration::from_millis(150) || !media_ok {
                self.samples.fill(0.0);
            }
            self.analyzer.update(&self.samples, dt, self.cfg.sensitivity_db);
        }
        let has_sound = viz_on && now - self.last_audio < Duration::from_secs(2) && self.analyzer.energy() > 0.0005;
        self.last_frame = now;

        let l = &self.layout;
        let s = self.s;
        let (sw, sh_) = (l.w as usize, l.h as usize);
        let (shape, _) = l.shape(open, peek);
        let animating = self.animating(now);

        // O que muda sem passar o tempo (mouse, música, opções…).
        let still = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            ((open * 1000.0) as i32, (peek * 1000.0) as i32, self.hidden, sw, sh_).hash(&mut h);
            (self.media_version, self.cfg_version, format!("{:?}", self.it), self.seek_preview.map(|v| (v * 1000.0) as i32)).hash(&mut h);
            (self.volume.level.to_bits(), self.volume.muted, self.hint.map(|h| h.0)).hash(&mut h);
            h.finish()
        };
        let pos_sec = if self.open_target { self.media.position_now() as u64 } else { 0 };
        // Card aberto e parado: só as barras mudam (e o tempo, a cada segundo).
        let quiet_open = !force && !self.full && !self.dirty && !animating && open >= 0.999 && peek <= 0.0005 && self.last_still == Some(still);
        let region = if self.full {
            RectF::new(0.0, 0.0, l.w, l.h)
        } else if quiet_open {
            let mut r = l.viz.inset(-2.0 * s);
            if let (true, Some(sk)) = (pos_sec != self.last_pos_sec, l.seek) {
                let k = sk.inset(-2.0 * s);
                let (x0, y0) = (r.x.min(k.x), r.y.min(k.y));
                r = RectF::new(x0, y0, r.right().max(k.right()) - x0, r.bottom().max(k.bottom()) - y0);
            }
            r
        } else if p <= 0.0005 {
            RectF::new(l.closed.x - 12.0 * s, 0.0, l.closed.w + 24.0 * s, l.band + 2.0 * s)
        } else {
            let m = nd::SHADOW * s + 12.0 * s;
            RectF::new(shape.x - m, 0.0, shape.w + 2.0 * m, shape.bottom() + m)
        };
        let rx0 = (region.x.floor().max(0.0) as usize).min(sw);
        let ry0 = (region.y.floor().max(0.0) as usize).min(sh_);
        let rx1 = (region.right().ceil().max(0.0) as usize).min(sw);
        let ry1 = (region.bottom().ceil().max(0.0) as usize).min(sh_);
        if rx1 <= rx0 || ry1 <= ry0 {
            return;
        }

        // Pula quadros iguais (sem animação e sem nada que mude sozinho).
        let sig = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (still, rx0, ry0, rx1, ry1, pos_sec).hash(&mut h);
            if self.continuous() || animating {
                ((time * 60.0) as u64).hash(&mut h);
            }
            h.finish()
        };
        if !force && !self.full && !self.dirty && self.last_sig == Some(sig) {
            return;
        }

        // Só o desenho no canvas, como no Mac e no Windows (o espectro e as
        // assinaturas acima ficam de fora).
        let t_draw = crate::stats::enabled().then(Instant::now);
        {
            let mut cv = Canvas::new(sw, sh_, &mut self.buf);
            cv.set_clip(RectF::new(rx0 as f32, ry0 as f32, (rx1 - rx0) as f32, (ry1 - ry0) as f32));
            cv.fill_clip(0);
            if !self.hidden {
                let theme = nd::theme(&self.cfg, &self.colors, &self.media);
                let colors = Colors { album: self.media.art.as_ref().map(|a| a.vibrant), ..self.colors };
                let peek_t = self.peek_start.map(|t| (now - t).as_secs_f32()).unwrap_or(0.0);
                let hint = self.hint.map(|(m, t)| {
                    let e = (now - t).as_secs_f32();
                    (m, (e / 0.2).min(1.0) * ((HINT_DUR - e) / 0.4).clamp(0.0, 1.0))
                });
                let change = self.change.as_ref().map(|(m, t)| (m, ((now - *t).as_secs_f32() / CHANGE_DUR).min(1.0)));
                let f = NotchFrame {
                    cfg: &self.cfg,
                    colors: &colors,
                    media: &self.media,
                    theme,
                    // Ícones em vetor (não há fonte de ícones no Linux).
                    it: Interact { fx: self.icons.fx(self.cfg.icon_anim), vector: true, ..self.it },
                    pb: Playback {
                        playing: self.media.playing,
                        shuffle: self.media.shuffle,
                        repeat: self.media.repeat,
                        pos: self.media.position_now(),
                        seek_preview: self.seek_preview,
                        volume: self.volume.level,
                        muted: self.volume.muted,
                    },
                    time,
                    viz: has_sound.then_some((&self.analyzer.values[..], &self.analyzer.peaks[..])),
                    open,
                    peek,
                    peek_t,
                    peek_total: self.cfg.popup_secs.max(2) as f32,
                    hint,
                    change,
                    launchers: &[],
                    wing_app: None,
                };
                nd::draw(&mut cv, &mut self.tr, l, &f);
            }
        }
        if let Some(p) = &mut self.perf {
            p.1 += 1;
            p.3 += now.elapsed();
        }
        let t_present = t_draw.map(|_| Instant::now());
        let shown = self.win.present(&self.buf, sw, sh_, (rx0, ry0, rx1, ry1));
        if let (Some(a), Some(b)) = (t_draw, t_present) {
            crate::stats::frame("ilha", b - a, b.elapsed());
        }
        if shown {
            self.last_sig = Some(sig);
            self.last_still = Some(still);
            self.last_pos_sec = pos_sec;
            self.dirty = false;
            self.full = false;
        } else {
            self.dirty = true;
        }
        // A forma mudou (abrindo/fechando): a área de clique acompanha.
        self.update_input();
    }

    // ------------------------------------------------------------ mouse

    fn hit(&self, x: f32, y: f32, open: bool) -> Option<Hit> {
        if !open && self.hot_rect().contains(x, y) {
            return Some(Hit::Info);
        }
        self.layout.hit(x, y, open)
    }

    fn mouse_move(&mut self, x: f32, y: f32) {
        let inside = !self.hidden && self.hot_rect().contains(x, y);
        let now = Instant::now();
        if inside && !self.inside {
            self.leave_since = None;
            if !self.open_target && self.cfg.notch_hover {
                self.hover_since = Some(now);
            }
        } else if !inside && self.inside {
            self.hover_since = None;
            if self.open_target {
                self.leave_since = Some(now);
            }
        }
        self.inside = inside;
        let open = self.open_target && self.open.value(now) > 0.85;
        let hover = if inside { self.hit(x, y, open) } else { None };
        if self.it.hover != hover {
            self.it.hover = hover;
        }
        if self.it.drag.is_some() {
            self.drag_to(x);
        }
    }

    fn mouse_down(&mut self, x: f32, y: f32) {
        let open = self.open_target && self.open.value(Instant::now()) > 0.85;
        let hit = self.hit(x, y, open);
        self.it.pressed = hit;
        if matches!(hit, Some(Hit::Seek) if self.media.can_seek) || hit == Some(Hit::Vol) {
            self.it.drag = hit;
            self.drag_to(x);
        }
    }

    fn drag_to(&mut self, x: f32) {
        match self.it.drag {
            Some(Hit::Seek) => {
                if let Some((x0, x1)) = self.layout.seek_track() {
                    self.seek_preview = Some(((x - x0) / (x1 - x0)).clamp(0.0, 1.0));
                }
            }
            Some(Hit::Vol) => {
                if let Some((x0, x1)) = self.layout.vol_track() {
                    self.volume.set_level(((x - x0) / (x1 - x0)).clamp(0.0, 1.0));
                }
            }
            _ => {}
        }
    }

    fn mouse_up(&mut self, x: f32, y: f32) {
        let open = self.open_target && self.open.value(Instant::now()) > 0.85;
        let hit = self.hit(x, y, open);
        if self.it.drag == Some(Hit::Seek) {
            if let Some(p) = self.seek_preview.take() {
                if self.media.dur > 0.0 {
                    let pos = p as f64 * self.media.dur;
                    self.shared.send_media(MediaCmd::Seek(pos));
                    self.media.pos = pos;
                    self.media.pos_at = Some(Instant::now());
                }
            }
        } else if self.it.pressed == hit && self.it.drag.is_none() {
            self.activate(hit);
        }
        self.it.pressed = None;
        self.it.drag = None;
        // Soltou fora da ilha depois de arrastar: começa a contar para fechar.
        if !self.inside && self.open_target && self.leave_since.is_none() {
            self.leave_since = Some(Instant::now());
        }
    }

    fn activate(&mut self, hit: Option<Hit>) {
        let sh = self.shared.clone();
        let m = self.media.clone();
        let not_here = "Este player não deixa mudar isso";
        match hit {
            Some(Hit::Info) if !self.open_target => self.set_open(true),
            Some(Hit::Play) if m.can_toggle => {
                sh.send_media(MediaCmd::PlayPause);
                self.media.playing = !self.media.playing;
            }
            Some(Hit::Prev) if m.can_prev => {
                self.icons.kick(Hit::Prev);
                sh.send_media(MediaCmd::Previous);
            }
            Some(Hit::Next) if m.can_next => {
                self.icons.kick(Hit::Next);
                sh.send_media(MediaCmd::Next);
            }
            Some(Hit::Shuffle) => {
                if m.can_shuffle {
                    sh.send_media(MediaCmd::Shuffle(!m.shuffle));
                    self.media.shuffle = !m.shuffle;
                } else {
                    self.hint = Some((crate::i18n::tr(not_here), Instant::now()));
                }
            }
            Some(Hit::Repeat) => {
                if m.can_repeat {
                    let r = (m.repeat + 1) % 3;
                    sh.send_media(MediaCmd::Repeat(r));
                    self.media.repeat = r;
                } else {
                    self.hint = Some((crate::i18n::tr(not_here), Instant::now()));
                }
            }
            Some(Hit::VolBtn) => self.volume.toggle_mute(),
            _ => {}
        }
    }

    fn scroll(&mut self, dy: f32) {
        if self.open_target && self.cfg.show_volume {
            self.volume.set_level(self.volume.level + dy * 0.01);
        }
    }

    fn window_event(&mut self, ev: WinEv) -> bool {
        match ev {
            WinEv::Motion(x, y) => self.mouse_move(x, y),
            WinEv::Leave => self.mouse_move(-1.0, -1.0),
            WinEv::Button { down: true, button: 1, x, y } => self.mouse_down(x, y),
            WinEv::Button { down: false, button: 1, x, y } => self.mouse_up(x, y),
            // Meio: tocar/pausar; direito: abre/fecha o card.
            WinEv::Button { down: true, button: 2, .. } => self.shared.send_media(MediaCmd::PlayPause),
            WinEv::Button { down: true, button: 3, .. } => self.toggle_open(),
            WinEv::Button { .. } => {}
            WinEv::Scroll(dy) => self.scroll(dy),
            WinEv::Scale(s) => self.rescale(s),
            WinEv::Expose => {
                self.full = true;
                self.input_set = false;
            }
            WinEv::Closed => return false,
        }
        true
    }

    fn app_event(&mut self, ev: App) {
        match ev {
            App::ToggleCard => self.toggle_open(),
            App::Quit => self.shared.running.store(false, Ordering::Relaxed),
        }
    }
}

/// Laço da thread principal: espera a janela, o pipe de "acordar" ou a hora
/// do próximo quadro.
pub fn run(shared: Arc<Shared>, rx: Receiver<App>) {
    let above = !shared.config().hide_on_fullscreen;
    let Some(win) = window::open(above) else {
        system::log::warn("sem janela: nem Wayland com layer-shell nem X11");
        // Sem onde mostrar a ilha: fica só a bandeja até o app ser fechado.
        while shared.is_running() {
            if let Ok(App::Quit) = rx.recv_timeout(Duration::from_secs(1)) {
                break;
            }
        }
        return;
    };
    system::log::info(&format!("janela: {} escala {}", win.name(), win.scale()));
    let mut n = Island::new(shared.clone(), win);
    n.render(true);
    n.schedule();
    let wake = system::wake_fd();
    while shared.is_running() {
        crate::stats::wake("ilha");
        n.win.before_poll();
        let mut fds = [libc::pollfd { fd: n.win.fd(), events: libc::POLLIN, revents: 0 }, libc::pollfd { fd: wake, events: libc::POLLIN, revents: 0 }];
        let timeout = n.timeout();
        unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout) };
        n.win.after_poll(fds[0].revents & libc::POLLIN != 0);
        if fds[1].revents != 0 {
            system::drain_wake();
        }
        if fds[0].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            system::log::warn("janela: conexão com o servidor gráfico caiu");
            break;
        }
        for ev in n.win.take_events() {
            if !n.window_event(ev) {
                system::log::warn("janela: superfície fechada pelo compositor");
                shared.running.store(false, Ordering::Relaxed);
            }
        }
        while let Ok(ev) = rx.try_recv() {
            n.app_event(ev);
        }
        n.pull_media();
        n.pull_config();
        n.step();
        n.schedule();
        n.render(false);
        if let Some(p) = &mut n.perf {
            p.2 += 1;
            if p.0.elapsed() >= Duration::from_secs(5) {
                let t = p.0.elapsed().as_secs_f32();
                let us = p.3.as_micros() as f32 / p.1.max(1) as f32;
                system::log::info(&format!("perf: {:.1} quadros/s, {:.1} laços/s, ritmo {}, desenho {us:.0} µs/quadro", p.1 as f32 / t, p.2 as f32 / t, n.rate));
                *p = (Instant::now(), 0, 0, Duration::ZERO);
            }
        }
    }
    shared.running.store(false, Ordering::Relaxed);
}
