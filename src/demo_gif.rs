//! GIF animado do README: o app em uso (abre o card, pausa, pula a música,
//! "A seguir" e aviso de Caps Lock), desenhado pelo próprio código do app com
//! músicas de exemplo. Inclui um codificador GIF pequeno (paleta por median
//! cut, pontilhado ordenado e só a área que mudou em cada quadro).
//!
//! ```powershell
//! $env:TASKBEAT_LANG="pt"; cargo test --release demo_gif -- --ignored
//! ```

use crate::config::*;
use crate::render::*;
use crate::shared::MediaInfo;
use crate::text::*;
use crate::ui::{self, CLayout, CardFrame, Hit, Interact, Playback, WLayout, WidgetFrame};
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------- codificador GIF

const TRANSPARENT: u8 = 255;

struct Gif {
    w: usize,
    h: usize,
    out: Vec<u8>,
    palette: Vec<[u8; 3]>,
    lut: Vec<u8>,
    lut_ok: Vec<bool>,
    prev: Option<Vec<u8>>,
}

/// Paleta de até `n` cores por median cut.
fn median_cut(mut colors: Vec<[u8; 3]>, n: usize) -> Vec<[u8; 3]> {
    let mut boxes: Vec<Vec<[u8; 3]>> = vec![std::mem::take(&mut colors)];
    while boxes.len() < n {
        // Caixa com a maior faixa de cor (ponderada pela quantidade) é dividida na mediana.
        let range = |b: &Vec<[u8; 3]>| -> (u32, usize) {
            let mut best = (0, 0);
            for ch in 0..3 {
                let (lo, hi) = b.iter().fold((255u8, 0u8), |(lo, hi), c| (lo.min(c[ch]), hi.max(c[ch])));
                let r = (hi - lo) as u32;
                if r > best.0 {
                    best = (r, ch);
                }
            }
            best
        };
        let Some((i, ch)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .map(|(i, b)| {
                let (r, ch) = range(b);
                (i, ch, r as u64 * (b.len() as f64).sqrt() as u64)
            })
            .filter(|x| x.2 > 0)
            .max_by_key(|x| x.2)
            .map(|(i, ch, _)| (i, ch))
        else {
            break;
        };
        let mut b = boxes.swap_remove(i);
        b.sort_unstable_by_key(|c| c[ch]);
        let tail = b.split_off(b.len() / 2);
        boxes.push(b);
        boxes.push(tail);
    }
    boxes
        .iter()
        .filter(|b| !b.is_empty())
        .map(|b| {
            let n = b.len() as u32;
            let s = b.iter().fold([0u32; 3], |a, c| [a[0] + c[0] as u32, a[1] + c[1] as u32, a[2] + c[2] as u32]);
            [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]
        })
        .collect()
}

const BAYER: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

fn rgb(p: u32) -> [u8; 3] {
    [((p >> 16) & 0xff) as u8, ((p >> 8) & 0xff) as u8, (p & 0xff) as u8]
}

impl Gif {
    fn new(w: usize, h: usize, samples: &[&[u32]]) -> Self {
        let mut colors = Vec::new();
        for f in samples {
            for (i, &p) in f.iter().enumerate() {
                if i % 3 == 0 {
                    colors.push(rgb(p));
                }
            }
        }
        let mut palette = median_cut(colors, 255);
        palette.resize(256, [0, 0, 0]);
        let mut out = Vec::new();
        out.extend_from_slice(b"GIF89a");
        out.extend_from_slice(&(w as u16).to_le_bytes());
        out.extend_from_slice(&(h as u16).to_le_bytes());
        out.extend_from_slice(&[0xF7, 0, 0]); // tabela global de 256 cores
        for c in &palette {
            out.extend_from_slice(c);
        }
        // Repetir para sempre (extensão NETSCAPE2.0).
        out.extend_from_slice(&[0x21, 0xFF, 0x0B]);
        out.extend_from_slice(b"NETSCAPE2.0");
        out.extend_from_slice(&[0x03, 0x01, 0x00, 0x00, 0x00]);
        Self { w, h, out, palette, lut: vec![0; 1 << 15], lut_ok: vec![false; 1 << 15], prev: None }
    }

    fn nearest(&mut self, r: i32, g: i32, b: i32) -> u8 {
        let key = (((r >> 3) << 10) | ((g >> 3) << 5) | (b >> 3)) as usize;
        if !self.lut_ok[key] {
            let c = [((r >> 3) << 3) + 4, ((g >> 3) << 3) + 4, ((b >> 3) << 3) + 4];
            let mut best = (i32::MAX, 0usize);
            for (i, p) in self.palette.iter().enumerate().take(255) {
                let d = [p[0] as i32 - c[0], p[1] as i32 - c[1], p[2] as i32 - c[2]];
                let e = 2 * d[0] * d[0] + 4 * d[1] * d[1] + 3 * d[2] * d[2];
                if e < best.0 {
                    best = (e, i);
                }
            }
            self.lut[key] = best.1 as u8;
            self.lut_ok[key] = true;
        }
        self.lut[key]
    }

    fn frame(&mut self, px: &[u32], delay_cs: u16) {
        let (w, h) = (self.w, self.h);
        let mut idx = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let c = rgb(px[y * w + x]);
                let d = (BAYER[y & 3][x & 3] - 8) * 3 / 4; // pontilhado estável entre quadros
                let r = (c[0] as i32 + d).clamp(0, 255);
                let g = (c[1] as i32 + d).clamp(0, 255);
                let b = (c[2] as i32 + d).clamp(0, 255);
                idx[y * w + x] = self.nearest(r, g, b);
            }
        }
        // Só a região que mudou; o resto fica transparente (mantém o quadro anterior).
        let (mut x0, mut y0, mut x1, mut y1) = (0, 0, w, h);
        let mut data = idx.clone();
        if let Some(prev) = &self.prev {
            let (mut a, mut b, mut c, mut d) = (w, h, 0, 0);
            for y in 0..h {
                for x in 0..w {
                    if idx[y * w + x] != prev[y * w + x] {
                        a = a.min(x);
                        b = b.min(y);
                        c = c.max(x + 1);
                        d = d.max(y + 1);
                    }
                }
            }
            if c == 0 {
                (a, b, c, d) = (0, 0, 1, 1);
            }
            (x0, y0, x1, y1) = (a, b, c, d);
            for (i, v) in data.iter_mut().enumerate() {
                if *v == prev[i] {
                    *v = TRANSPARENT;
                }
            }
        }
        let (fw, fh) = (x1 - x0, y1 - y0);
        let mut sub = Vec::with_capacity(fw * fh);
        for y in y0..y1 {
            sub.extend_from_slice(&data[y * w + x0..y * w + x1]);
        }
        let o = &mut self.out;
        // Graphic Control Extension: atraso, não descartar, índice transparente.
        o.extend_from_slice(&[0x21, 0xF9, 0x04, (1 << 2) | 1]);
        o.extend_from_slice(&delay_cs.to_le_bytes());
        o.extend_from_slice(&[TRANSPARENT, 0x00]);
        // Image Descriptor.
        o.push(0x2C);
        for v in [x0, y0, fw, fh] {
            o.extend_from_slice(&(v as u16).to_le_bytes());
        }
        o.push(0x00);
        o.push(8);
        let lzw = lzw_encode(&sub);
        for chunk in lzw.chunks(255) {
            o.push(chunk.len() as u8);
            o.extend_from_slice(chunk);
        }
        o.push(0);
        self.prev = Some(idx);
    }

    fn finish(mut self) -> Vec<u8> {
        self.out.push(0x3B);
        self.out
    }
}

/// LZW do GIF (códigos de 9 a 12 bits, tabela reiniciada ao encher).
fn lzw_encode(data: &[u8]) -> Vec<u8> {
    const CLEAR: u32 = 256;
    const EOI: u32 = 257;
    let mut out = Vec::new();
    let (mut acc, mut nacc) = (0u32, 0u32);
    let mut n_bits = 9u32;
    let mut maxcode = (1u32 << n_bits) - 1;
    let mut free = 258u32;
    let mut clear_flag = false;
    let mut dict: HashMap<u32, u32> = HashMap::new();

    let mut emit = |code: u32, n_bits: &mut u32, maxcode: &mut u32, free: u32, clear_flag: &mut bool, out: &mut Vec<u8>| {
        acc |= code << nacc;
        nacc += *n_bits;
        while nacc >= 8 {
            out.push((acc & 0xff) as u8);
            acc >>= 8;
            nacc -= 8;
        }
        if *clear_flag {
            *n_bits = 9;
            *maxcode = (1 << 9) - 1;
            *clear_flag = false;
        } else if free > *maxcode {
            *n_bits += 1;
            *maxcode = if *n_bits == 12 { 4096 } else { (1 << *n_bits) - 1 };
        }
    };

    emit(CLEAR, &mut n_bits, &mut maxcode, free, &mut clear_flag, &mut out);
    let mut it = data.iter();
    let Some(&first) = it.next() else {
        emit(EOI, &mut n_bits, &mut maxcode, free, &mut clear_flag, &mut out);
        return out;
    };
    let mut ent = first as u32;
    for &k in it {
        let key = (ent << 8) | k as u32;
        if let Some(&code) = dict.get(&key) {
            ent = code;
            continue;
        }
        emit(ent, &mut n_bits, &mut maxcode, free, &mut clear_flag, &mut out);
        ent = k as u32;
        if free < 4096 {
            dict.insert(key, free);
            free += 1;
        } else {
            dict.clear();
            free = 258;
            clear_flag = true;
            emit(CLEAR, &mut n_bits, &mut maxcode, free, &mut clear_flag, &mut out);
        }
    }
    emit(ent, &mut n_bits, &mut maxcode, free, &mut clear_flag, &mut out);
    emit(EOI, &mut n_bits, &mut maxcode, free, &mut clear_flag, &mut out);
    if nacc > 0 {
        out.push((acc & 0xff) as u8);
    }
    out
}

// ---------------------------------------------------------------- cena

const W: f32 = 720.0;
const H: f32 = 370.0;
const FPS: f32 = 20.0;
const TOTAL: f32 = 10.8;

fn ease_io(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Ponteiro do mouse (seta) com preenchimento por cobertura (supersampling 4x4).
fn fill_poly(cv: &mut Canvas, pts: &[(f32, f32)], c: Rgb, a: f32) {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let inside = |px: f32, py: f32| {
        let mut inn = false;
        let mut j = pts.len() - 1;
        for i in 0..pts.len() {
            let (xi, yi) = pts[i];
            let (xj, yj) = pts[j];
            if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
                inn = !inn;
            }
            j = i;
        }
        inn
    };
    for py in y0.floor().max(0.0) as usize..(y1.ceil() as usize).min(cv.h) {
        for px in x0.floor().max(0.0) as usize..(x1.ceil() as usize).min(cv.w) {
            let mut n = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    if inside(px as f32 + (sx as f32 + 0.5) / 4.0, py as f32 + (sy as f32 + 0.5) / 4.0) {
                        n += 1;
                    }
                }
            }
            if n > 0 {
                cv.blend(px, py, c, a * n as f32 / 16.0);
            }
        }
    }
}

fn cursor(cv: &mut Canvas, x: f32, y: f32) {
    let outer = [(0.0, 0.0), (0.0, 18.0), (4.4, 14.0), (7.6, 21.0), (10.4, 19.8), (7.3, 13.0), (13.0, 13.0)];
    let inner = [(1.3, 3.0), (1.3, 15.0), (4.8, 11.9), (8.2, 19.2), (9.0, 18.8), (5.6, 11.6), (10.2, 11.6)];
    let shadow: Vec<(f32, f32)> = outer.iter().map(|&(a, b)| (x + a + 1.0, y + b + 1.5)).collect();
    fill_poly(cv, &shadow, [0.0; 3], 0.25);
    fill_poly(cv, &outer.map(|(a, b)| (x + a, y + b)), [0.0; 3], 1.0);
    fill_poly(cv, &inner.map(|(a, b)| (x + a, y + b)), [1.0; 3], 1.0);
}

fn wallpaper(cv: &mut Canvas) {
    cv.fill_rrect_ca(0.0, 0.0, W, H, 0.0, |x, y| {
        let (u, v) = (x / W, y / H);
        let c = lerp3([0.05, 0.09, 0.26], [0.22, 0.12, 0.42], (u * 0.7 + v * 0.3).clamp(0.0, 1.0));
        let glow = (-(((u - 0.28) * 2.0).powi(2) + ((v - 0.32) * 2.4).powi(2))).exp();
        (lerp3(c, [0.30, 0.45, 0.95], glow * 0.5), 1.0)
    });
}

fn recolor(img: &Image, perm: [usize; 3]) -> Arc<Image> {
    let px = img
        .px
        .iter()
        .map(|&p| {
            let c = [(p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff];
            (p & 0xff00_0000) | (c[perm[0]] << 16) | (c[perm[1]] << 8) | c[perm[2]]
        })
        .collect();
    Arc::new(Image::new(img.w, img.h, px))
}

fn layer(w: f32, h: f32, f: impl FnOnce(&mut Canvas)) -> Image {
    let (w, h) = (w.ceil() as usize, h.ceil() as usize);
    let mut px = vec![0u32; w * h];
    f(&mut Canvas::new(w, h, &mut px));
    Image::plain(w, h, px)
}

#[test]
#[ignore]
fn demo_gif() {
    let s = 1.0;
    let cfg = Config::default();
    let mut tr = TextRenderer::new();
    tr.set_scale(s);

    // Músicas de exemplo (nomes fictícios, capa gerada).
    let a = ui::sample_media();
    let art = a.art.clone().unwrap();
    let song = |title: &str, artist: &str, art: Arc<Image>, dur: f64| MediaInfo { title: title.into(), artist: artist.into(), art: Some(art), dur, pos: 0.0, ..ui::sample_media() };
    let songs = [
        song("Midnight Drive", "Aurora Lane", art.clone(), 231.0),
        song("Neon Skyline", "The Lumens", recolor(&art, [2, 0, 1]), 204.0),
        song("Golden Hour", "Paper Kites Club", recolor(&art, [1, 0, 2]), 247.0),
    ];

    // Fundo fixo: papel de parede + barra de tarefas (sem o widget).
    let bar = RectF::new(0.0, H - 48.0, W, 48.0);
    let clock_x = W - 64.0;
    let bg = layer(W, H, |cv| {
        wallpaper(cv);
        cv.fill_round(bar, 0.0, [0.11, 0.11, 0.12], 0.96);
        cv.fill_rrect(bar.x, bar.y, bar.right(), bar.y + 1.0, 0.0, 0.10, |_, _| [1.0; 3]);
        for k in 0..5 {
            let ix = W * 0.10 + k as f32 * 40.0;
            cv.fill_round(RectF::new(ix, bar.cy() - 11.0, 22.0, 22.0), 5.0, [0.85; 3], if k == 0 { 0.9 } else { 0.30 });
        }
        ui::text(cv, &mut tr, "12:34", Font::Caption, clock_x, bar.y + 6.0, 56.0, 18.0, [0.92; 3], 1.0);
        ui::text(cv, &mut tr, "23/11/2026", Font::Caption, clock_x - 8.0, bar.y + 24.0, 70.0, 18.0, [0.92; 3], 0.8);
        for (k, g) in ["\u{E995}", "\u{E701}", "\u{E70E}"].iter().enumerate() {
            ui::glyph(cv, &mut tr, RectF::new(clock_x - (k as f32 + 1.0) * 26.0 - 12.0, bar.y, 24.0, bar.h), g, Font::IconSmall, [0.9; 3], 0.9);
        }
    });

    let wl = WLayout::build(s, 40.0, &cfg, true, false);
    let wx = clock_x - 104.0 - wl.width;
    let wrect = RectF::new(wx, bar.y + 4.0, wl.width, 40.0);
    let cl = CLayout::build(s, &cfg, true);
    let m = ui::SHADOW_MARGIN;
    let card_x = (wx + 2.0 - m).min(W - cl.w + m - 16.0).round();
    let card_y = (bar.y - (cl.h - m) - 10.0).round();
    let abs = |r: RectF| (card_x + r.cx(), card_y + r.cy());

    // Caminho do mouse: (tempo, x, y).
    let (wcx, wcy) = (wrect.cx() - 30.0, wrect.cy());
    let (px_, py_) = abs(cl.play);
    let (nx, ny) = abs(cl.next);
    let path = [
        (0.0, 250.0, 150.0),
        (0.5, 250.0, 150.0),
        (1.1, wcx, wcy),
        (1.6, wcx, wcy),
        (2.2, px_, py_),
        (3.5, px_, py_),
        (3.9, nx, ny),
        (4.4, nx, ny),
        (5.0, 170.0, 130.0),
        (TOTAL, 170.0, 130.0),
    ];
    let mouse = |t: f32| {
        let i = path.iter().rposition(|p| p.0 <= t).unwrap_or(0).min(path.len() - 2);
        let (t0, x0, y0) = path[i];
        let (t1, x1, y1) = path[i + 1];
        let k = ease_io((t - t0) / (t1 - t0).max(1e-3));
        (x0 + (x1 - x0) * k, y0 + (y1 - y0) * k)
    };
    let pressed = |t: f32, at: f32| (t - at).abs() < 0.07;

    let frames = (TOTAL * FPS) as usize;
    let mut out: Vec<Vec<u32>> = Vec::with_capacity(frames);
    for f in 0..frames {
        let t = f as f32 / FPS;
        let (mx, my) = mouse(t);
        // Estado do player.
        let (si, start, pos0) = if t < 4.0 {
            (0, 0.0, 96.0)
        } else if t < 5.9 {
            (1, 4.0, 0.0)
        } else {
            (2, 5.9, 0.0)
        };
        let paused = (2.5..3.2).contains(&t);
        let mut media = songs[si].clone();
        let played = if si == 0 { (t.min(2.5) - start) + (t - 3.2).max(0.0) } else { t - start };
        media.pos = pos0 + played.max(0.0) as f64;
        media.playing = !paused;
        let colors = Colors { palette: cfg.palette, accent: [0.93, 0.32, 0.62], accent_light: [0.98, 0.62, 0.80], light_taskbar: false, album: media.art.as_ref().map(|a| a.vibrant) };
        let pb = Playback { playing: !paused, shuffle: media.shuffle, repeat: media.repeat, pos: media.pos, seek_preview: None, volume: 0.62, muted: false };

        // Card: abre após o clique (1.3 s) e fecha quando o mouse sai (5.0 s).
        let open = ui::ease_out_cubic(((t - 1.3) / 0.25).clamp(0.0, 1.0)) * (1.0 - ui::ease_out_cubic(((t - 5.0) / 0.22).clamp(0.0, 1.0)));
        let on_widget = wrect.contains(mx, my);
        let mut px = bg.px.clone();
        {
            let mut cv = Canvas::new(W as usize, H as usize, &mut px);
            let widget = layer(wl.width, 40.0, |c| {
                let it = Interact { hover: on_widget.then_some(Hit::Info), pressed: (on_widget && pressed(t, 1.25)).then_some(Hit::Info), drag: None };
                let fr = WidgetFrame { cfg: &cfg, media: &media, theme: ui::widget_theme(&cfg, &colors, &media), colors: &colors, it, pb, time: t, viz: None, flyout_open: open > 0.5, s };
                ui::draw_widget(c, &mut tr, &wl, &fr);
            });
            cv.draw_image(wrect, &widget, 0.0, 1.0);

            if open > 0.01 {
                let card = layer(cl.w, cl.h, |c| {
                    let hover = if cl.play.contains(mx - card_x, my - card_y) {
                        Some(Hit::Play)
                    } else if cl.next.contains(mx - card_x, my - card_y) {
                        Some(Hit::Next)
                    } else {
                        None
                    };
                    let down = pressed(t, 2.5) || pressed(t, 3.2) || pressed(t, 4.0);
                    let it = Interact { hover, pressed: if down { hover } else { None }, drag: None };
                    let fr = CardFrame { cfg: &cfg, colors: &colors, media: &media, theme: ui::card_theme(&cfg, &colors, &media), it, pb, s };
                    ui::draw_card(c, &mut tr, &cl, &fr);
                });
                let slide = ((1.0 - open) * 14.0).round();
                cv.draw_image(RectF::new(card_x, card_y + slide, cl.w.ceil(), cl.h.ceil()), &card, 0.0, open);
            }

            // "A seguir": a música 2 terminou e o player passou sozinho para a 3.
            let tp = t - 5.9;
            let total = cfg.popup_secs as f32 * 0.6;
            if (0.0..total).contains(&tp) {
                let (uw, uh) = ui::upnext_size(s);
                let e = if tp < 0.45 { ui::ease_out_cubic(tp / 0.45) } else if tp > total - 0.4 { 1.0 - ui::ease_out_cubic((tp - (total - 0.4)) / 0.4) } else { 1.0 };
                let pop = layer(uw, uh, |c| {
                    let th = ui::card_theme(&cfg, &colors, &media);
                    ui::draw_upnext(c, &mut tr, &cfg, &colors, &media, &th, s, tp, total);
                });
                let ux = (wx + 2.0 - m).min(W - uw + m - 16.0).round();
                let uy = (bar.y - (uh - m) - 10.0 + (1.0 - e) * 18.0).round();
                cv.draw_image(RectF::new(ux, uy, uw.ceil(), uh.ceil()), &pop, 0.0, e);
            }

            // Aviso de Caps Lock (embaixo, no meio).
            let to = t - 9.0;
            let secs = cfg.osd_duration as f32 / 10.0;
            if (0.0..secs).contains(&to) {
                let (ow, oh) = ui::osd_size(s, &cfg);
                let e = if to < 0.25 { ui::ease_out_cubic(to / 0.25) } else if to > secs - 0.3 { 1.0 - ui::ease_out_cubic((to - (secs - 0.3)) / 0.3) } else { 1.0 };
                let osd = layer(ow, oh, |c| {
                    let th = ui::osd_theme(&cfg, &colors);
                    ui::draw_osd(c, &mut tr, &cfg, &th, s, crate::i18n::tr("Caps Lock"), "A", true, to);
                });
                let ox = (W / 2.0 - ow / 2.0).round();
                let oy = (bar.y - oh + m - 20.0 + (1.0 - e) * 14.0).round();
                cv.draw_image(RectF::new(ox, oy, ow.ceil(), oh.ceil()), &osd, 0.0, e);
            }
            cursor(&mut cv, mx.round(), my.round());
        }
        out.push(px);
    }

    let samples: Vec<&[u32]> = out.iter().step_by(12).map(|v| v.as_slice()).collect();
    let mut gif = Gif::new(W as usize, H as usize, &samples);
    for f in &out {
        gif.frame(f, (100.0 / FPS) as u16);
    }
    let bytes = gif.finish();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs").join("screenshots").join("demo.gif");
    std::fs::write(&path, &bytes).unwrap();
    println!("{} ({} quadros, {:.1} MB)", path.display(), frames, bytes.len() as f64 / 1e6);
}
