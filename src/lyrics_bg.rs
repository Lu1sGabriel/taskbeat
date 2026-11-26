//! Fundo da tela cheia da sobreposição de letra: a capa bem desfocada, em
//! três camadas que giram e andam em círculos devagar (as cores da própria
//! capa se misturando), com uma película escura para a letra ler bem.
//!
//! O quadro é montado numa grade 12× menor que a tela (a capa reduzida a
//! 96 px e desfocada, girada e ampliada com bilinear), suavizada, e então
//! ampliada para a tela com contas inteiras (dois canais por vez num u32) e
//! um pontilhado fino (Bayer 8×8) no arredondamento: sem os degraus da grade
//! nem as faixas do escuro, a uns poucos ms por quadro.

use crate::render::{Canvas, Image};
use std::cell::RefCell;

/// Lado da capa reduzida usada nas camadas (maior: as manchas ampliadas
/// não mostram as facetas do bilinear).
const SRC: usize = 96;
/// Desfoque da capa reduzida (px dela).
const SRC_BLUR: usize = 6;
/// Quanto a grade é menor que a tela (o conteúdo é todo desfocado; com 16
/// e sem suavizar, a ampliação mostrava degraus).
const DOWN: usize = 12;

/// Pontilhado ordenado 8×8 (0..255): o arredondamento de cada pixel varia,
/// e os degradês escuros deixam de virar faixas.
const BAYER: [u32; 64] = {
    const M: [u8; 64] = [
        0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46, 6, 38, 60, 28, 52, 20, 62, 30, 54, 22, 3, 35, 11, 43, 1, 33, 9, 41, 51, 19, 59, 27, 49, 17, 57, 25, 15, 47, 7, 39, 13, 45, 5, 37, 63, 31, 55,
        23, 61, 29, 53, 21,
    ];
    let mut out = [0u32; 64];
    let mut i = 0;
    while i < 64 {
        out[i] = M[i] as u32 * 4 + 2;
        i += 1;
    }
    out
};

struct Cache {
    art: Option<u64>,
    /// Capa reduzida e desfocada (opaca), ou um degradê neutro sem capa.
    src: Vec<[f32; 3]>,
    /// A da música anterior (a troca cruza as duas).
    prev: Option<Vec<[f32; 3]>>,
    /// Grade do último quadro: tamanho, tempo e pixels.
    field: (usize, usize, u32, Vec<u32>),
    /// Linhas da grade já ampliadas na horizontal (todas: grade × largura da tela).
    rows: Vec<u32>,
    /// Pontilhado de cada linha da tela (y % 8), um valor por byte do pixel.
    dither: Vec<u16>,
    /// x de origem e peso (0..256) de cada coluna da tela.
    xs: Vec<(u32, u32)>,
}

thread_local! {
    static CACHE: RefCell<Option<Cache>> = const { RefCell::new(None) };
}

pub fn forget() {
    CACHE.with(|c| *c.borrow_mut() = None);
}

fn unpack(p: u32) -> [f32; 3] {
    [((p >> 16) & 0xff) as f32 / 255.0, ((p >> 8) & 0xff) as f32 / 255.0, (p & 0xff) as f32 / 255.0]
}

/// Capa a `SRC` px, desfocada; sem capa, um degradê escuro azulado.
fn source(art: Option<&Image>) -> Vec<[f32; 3]> {
    match art {
        Some(a) => {
            let small = a.resampled(crate::render::RectF::new(0.0, 0.0, a.w as f32, a.h as f32), SRC, SRC);
            let blurred = small.blurred(SRC_BLUR);
            blurred.px.iter().map(|&p| unpack(p)).collect()
        }
        None => (0..SRC * SRC)
            .map(|i| {
                let (x, y) = ((i % SRC) as f32 / SRC as f32, (i / SRC) as f32 / SRC as f32);
                [0.10 + 0.10 * x, 0.08 + 0.06 * y, 0.22 + 0.12 * (1.0 - x)]
            })
            .collect(),
    }
}

/// Amostra bilinear da capa reduzida (u, v em 0..1, presos à borda).
fn sample(src: &[[f32; 3]], u: f32, v: f32) -> [f32; 3] {
    let x = (u * SRC as f32 - 0.5).clamp(0.0, (SRC - 1) as f32);
    let y = (v * SRC as f32 - 0.5).clamp(0.0, (SRC - 1) as f32);
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(SRC - 1), (y0 + 1).min(SRC - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let p = |xx: usize, yy: usize| src[yy * SRC + xx];
    let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
    let mut o = [0.0; 3];
    for k in 0..3 {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bot = c[k] + (d[k] - c[k]) * fx;
        o[k] = top + (bot - top) * fy;
    }
    o
}

/// Uma camada: a capa girada `rot`, com lado `size` (px da grade), centrada
/// em (cx, cy), misturada com opacidade `alpha` que some em círculo nas bordas.
#[allow(clippy::too_many_arguments)]
fn layer(field: &mut [[f32; 3]], lw: usize, lh: usize, src: &[[f32; 3]], cx: f32, cy: f32, size: f32, rot: f32, alpha: f32, flip: bool) {
    let (sn, cs) = rot.sin_cos();
    let half = size / 2.0;
    let (x0, x1) = ((cx - half * 1.42).floor().max(0.0) as usize, ((cx + half * 1.42).ceil().max(0.0) as usize).min(lw));
    let (y0, y1) = ((cy - half * 1.42).floor().max(0.0) as usize, ((cy + half * 1.42).ceil().max(0.0) as usize).min(lh));
    for y in y0..y1 {
        for x in x0..x1 {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            // Distância ao centro (0..1 na borda do círculo inscrito): some suave.
            let d = (dx * dx + dy * dy).sqrt() / half;
            let m = ((1.25 - d) / 0.55).clamp(0.0, 1.0);
            if m <= 0.0 {
                continue;
            }
            let m = m * m * (3.0 - 2.0 * m) * alpha;
            let u = (dx * cs + dy * sn) / size + 0.5;
            let v = (-dx * sn + dy * cs) / size + 0.5;
            let c = sample(src, if flip { 1.0 - u } else { u }, v);
            let o = &mut field[y * lw + x];
            for k in 0..3 {
                o[k] += (c[k] - o[k]) * m;
            }
        }
    }
}

/// Monta a grade do quadro no tempo `t` (s).
fn field(src: &[[f32; 3]], lw: usize, lh: usize, t: f32) -> Vec<u32> {
    use std::f32::consts::TAU;
    let (w, h) = (lw as f32, lh as f32);
    let big = w.max(h);
    let mut f = vec![[0.0f32; 3]; lw * lh];
    // Fundo: a capa inteira cobrindo tudo, girando bem devagar.
    let a0 = t * 0.035;
    let diag = (w * w + h * h).sqrt();
    let (sn, cs) = a0.sin_cos();
    let (cx, cy) = (w / 2.0 + w * 0.05 * (t * TAU / 47.0).cos(), h / 2.0 + h * 0.06 * (t * TAU / 47.0).sin());
    for y in 0..lh {
        for x in 0..lw {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let u = (dx * cs + dy * sn) / (diag * 1.05) + 0.5;
            let v = (-dx * sn + dy * cs) / (diag * 1.05) + 0.5;
            f[y * lw + x] = sample(src, u, v);
        }
    }
    // Três manchas da capa andando em círculos (períodos diferentes, um sentido cada).
    let orbit = |period: f32, phase: f32, rx: f32, ry: f32| {
        let a = t * TAU / period + phase;
        (w / 2.0 + rx * a.cos(), h / 2.0 + ry * a.sin())
    };
    let (x1, y1) = orbit(26.0, 0.0, w * 0.30, h * 0.30);
    layer(&mut f, lw, lh, src, x1, y1, big * 0.95, -t * 0.08, 0.85, false);
    let (x2, y2) = orbit(-34.0, 2.1, w * 0.34, h * 0.26);
    layer(&mut f, lw, lh, src, x2, y2, big * 0.80, t * 0.06 + 1.3, 0.75, true);
    let (x3, y3) = orbit(41.0, 4.2, w * 0.22, h * 0.34);
    layer(&mut f, lw, lh, src, x3, y3, big * 0.70, t * 0.11 + 2.6, 0.65, false);
    smooth(&mut f, lw, lh);
    // Película: escurece (mais embaixo e nas bordas) e satura um pouco.
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let mut out = vec![0u32; lw * lh];
    for y in 0..lh {
        let vy = y as f32 / h;
        for x in 0..lw {
            let vx = x as f32 / w - 0.5;
            let c = f[y * lw + x];
            let l = 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
            let k = 0.62 - 0.20 * vy - 0.18 * vx.abs();
            let sat = |v: f32| (l + (v - l) * 1.25) * k;
            out[y * lw + x] = 0xff00_0000 | (q(sat(c[0])) << 16) | (q(sat(c[1])) << 8) | q(sat(c[2]));
        }
    }
    out
}

/// `out = a + (b - a) × f/256`, byte a byte, arredondado pelo pontilhado `d`
/// (um valor 0..255 por byte; o compilador faz isso em blocos de 16 bytes: é
/// a parte mais pesada, uma vez por pixel da tela).
fn blend_rows(out: &mut [u32], a: &[u32], b: &[u32], f: u32, d: &[u16]) {
    // SAFETY: u32 tem alinhamento maior que u8 e os tamanhos são exatos.
    let (o8, a8, b8) = unsafe {
        (
            std::slice::from_raw_parts_mut(out.as_mut_ptr() as *mut u8, out.len() * 4),
            std::slice::from_raw_parts(a.as_ptr() as *const u8, a.len() * 4),
            std::slice::from_raw_parts(b.as_ptr() as *const u8, b.len() * 4),
        )
    };
    let (f, g) = (f as u16, 256 - f as u16);
    // 255 × 256 + 255 cabe em u16: a soma com o pontilhado nunca estoura.
    for (((o, &x), &y), &d) in o8.iter_mut().zip(a8).zip(b8).zip(d) {
        *o = ((x as u16 * g + y as u16 * f + d) >> 8) as u8;
    }
}

/// Suaviza a grade com [1 2 1]/4 nas duas direções (tira as quinas que o
/// bilinear mostra ao ampliar muito).
fn smooth(f: &mut [[f32; 3]], lw: usize, lh: usize) {
    let mut tmp = f.to_vec();
    for y in 0..lh {
        for x in 0..lw {
            let (l, r) = (x.saturating_sub(1), (x + 1).min(lw - 1));
            let row = y * lw;
            tmp[row + x] = std::array::from_fn(|k| (f[row + l][k] + 2.0 * f[row + x][k] + f[row + r][k]) * 0.25);
        }
    }
    for y in 0..lh {
        let (u, d) = (y.saturating_sub(1), (y + 1).min(lh - 1));
        for x in 0..lw {
            f[y * lw + x] = std::array::from_fn(|k| (tmp[u * lw + x][k] + 2.0 * tmp[y * lw + x][k] + tmp[d * lw + x][k]) * 0.25);
        }
    }
}

/// Como `lerp`, arredondando pelo pontilhado `d` (0..255) em vez de truncar.
#[inline]
fn lerp_d(a: u32, b: u32, f: u32, d: u32) -> u32 {
    let g = 256 - f;
    let d2 = d * 0x0001_0001;
    let rb = ((((a & 0x00ff_00ff) * g + (b & 0x00ff_00ff) * f) + d2) >> 8) & 0x00ff_00ff;
    let ag = ((((a >> 8) & 0x00ff_00ff) * g + ((b >> 8) & 0x00ff_00ff) * f) + d2) & 0xff00_ff00;
    rb | ag
}

/// Mistura dois pixels opacos com peso `f` (0..256) para `b`.
#[inline]
fn lerp(a: u32, b: u32, f: u32) -> u32 {
    let g = 256 - f;
    let rb = (((a & 0x00ff_00ff) * g + (b & 0x00ff_00ff) * f) >> 8) & 0x00ff_00ff;
    let ag = (((a >> 8) & 0x00ff_00ff) * g + ((b >> 8) & 0x00ff_00ff) * f) & 0xff00_ff00;
    rb | ag
}

/// Pinta o fundo na tela toda (opaco). `t`: tempo da animação (`None`:
/// parado no começo). `mix` (0..1): troca de música em andamento, da capa
/// anterior para a atual.
pub fn fill(cv: &mut Canvas, art: Option<&Image>, t: Option<f32>, mix: f32) {
    let (w, h) = (cv.w, cv.h);
    if w == 0 || h == 0 {
        return;
    }
    let (lw, lh) = (w.div_ceil(DOWN).max(8), h.div_ceil(DOWN).max(8));
    let t = t.unwrap_or(0.0);
    // Tempo em passos de 1/120 s (quadros iguais não refazem a grade).
    let tk = ((t * 120.0) as u32) ^ (((mix.clamp(0.0, 1.0) * 255.0) as u32) << 24);
    CACHE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let id = art.map(|a| a.id);
        if cell.as_ref().is_none_or(|c| c.art != id) {
            let prev = cell.take().map(|c| c.src);
            *cell = Some(Cache { art: id, src: source(art), prev, field: (0, 0, u32::MAX, Vec::new()), rows: Vec::new(), xs: Vec::new(), dither: Vec::new() });
        }
        let c = cell.as_mut().unwrap();
        if mix >= 1.0 {
            c.prev = None;
        }
        if c.field.0 != lw || c.field.1 != lh || c.field.2 != tk {
            let mut f = field(&c.src, lw, lh, t);
            if let Some(p) = c.prev.as_ref().filter(|_| mix < 1.0) {
                let old = field(p, lw, lh, t);
                let k = (mix.clamp(0.0, 1.0) * 256.0) as u32;
                for (n, o) in f.iter_mut().zip(old) {
                    *n = lerp(o, *n, k);
                }
            }
            c.field = (lw, lh, tk, f);
            c.rows.clear();
        }
        if c.xs.len() != w {
            c.xs = (0..w)
                .map(|x| {
                    let u = ((x as f32 + 0.5) * lw as f32 / w as f32 - 0.5).clamp(0.0, (lw - 1) as f32);
                    (u as u32, ((u - u.floor()) * 256.0) as u32)
                })
                .collect();
            // Pontilhado por linha (y % 8) e por byte: os três canais juntos.
            c.dither = (0..8 * w * 4).map(|i| BAYER[(i / (w * 4)) * 8 + (i / 4) % 8] as u16).collect();
            c.rows.clear();
        }
        let Cache { field, rows, xs, dither, .. } = c;
        // Grade ampliada na horizontal (uma vez por quadro do fundo).
        if rows.len() != lh * w {
            rows.resize(lh * w, 0);
            let grid = &field.3;
            for r in 0..lh {
                let src = &grid[r * lw..(r + 1) * lw];
                let bayer = &BAYER[(r & 7) * 8..(r & 7) * 8 + 8];
                for (x, (o, &(x0, fx))) in rows[r * w..(r + 1) * w].iter_mut().zip(xs.iter()).enumerate() {
                    let x0 = x0 as usize;
                    *o = lerp_d(src[x0], src[(x0 + 1).min(lw - 1)], fx, bayer[x & 7]);
                }
            }
        }
        for y in 0..h {
            let v = ((y as f32 + 0.5) * lh as f32 / h as f32 - 0.5).clamp(0.0, (lh - 1) as f32);
            let (r0, fy) = (v as usize, ((v - v.floor()) * 256.0) as u32);
            let r1 = (r0 + 1).min(lh - 1);
            let (a, b) = (&rows[r0 * w..(r0 + 1) * w], &rows[r1 * w..(r1 + 1) * w]);
            let d = &dither[(y & 7) * w * 4..((y & 7) + 1) * w * 4];
            blend_rows(cv.row_mut(y), &a[..w], &b[..w], if r0 == r1 { 0 } else { fy }, d);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ampliação com fração e pontilhado: um degradê escuro de 1 nível
    /// vira uma mistura dos dois níveis vizinhos (sem faixa dura), e os
    /// extremos ficam exatos.
    #[test]
    fn ampliacao_sem_faixas() {
        let d: Vec<u16> = (0..32).map(|i| BAYER[(i / 4) % 8] as u16).collect();
        let (a, b) = (vec![0xff0a_0a0au32; 8], vec![0xff0b_0b0bu32; 8]);
        let mut out = vec![0u32; 8];
        blend_rows(&mut out, &a, &b, 128, &d);
        let lv: Vec<u32> = out.iter().map(|p| p & 0xff).collect();
        assert!(lv.contains(&10) && lv.contains(&11), "{lv:?}");
        assert!(out.iter().all(|p| p >> 24 == 0xff));
        let white = vec![0xffff_ffffu32; 8];
        blend_rows(&mut out, &white, &white, 0, &d);
        assert!(out.iter().all(|&p| p == 0xffff_ffff));
        let black = vec![0xff00_0000u32; 8];
        blend_rows(&mut out, &black, &black, 0, &d);
        assert!(out.iter().all(|&p| p == 0xff00_0000));
    }

    #[test]
    fn mistura_com_pontilhado() {
        // Sem estourar nos extremos; no meio de 10 e 11, dá 10 ou 11 conforme o pontilhado.
        assert_eq!(lerp_d(0xffff_ffff, 0xffff_ffff, 128, 254), 0xffff_ffff);
        assert_eq!(lerp_d(0xff00_0000, 0xff00_0000, 128, 254), 0xff00_0000);
        let lv: Vec<u32> = (0..8).map(|i| lerp_d(0xff0a_0a0a, 0xff0b_0b0b, 128, BAYER[i]) & 0xff).collect();
        assert!(lv.contains(&10) && lv.contains(&11), "{lv:?}");
    }

    #[test]
    fn mistura_inteira() {
        assert_eq!(lerp(0xff00_0000, 0xffff_ffff, 0), 0xff00_0000);
        assert_eq!(lerp(0xff00_0000, 0xffff_ffff, 256) & 0x00ff_ffff, 0x00ff_ffff);
        let m = lerp(0xff00_0000, 0xffff_ffff, 128);
        assert_eq!(m >> 24, 0xff);
        assert!(((m >> 16) & 0xff).abs_diff(0x80) <= 1);
    }

    /// Mede o custo de um quadro do fundo em 2560×1440 (rode em release):
    /// `cargo test --release fundo_custo -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn fundo_custo() {
        let art = crate::ui::sample_media().art.unwrap();
        let (w, h) = (2560, 1440);
        let mut px = vec![0u32; w * h];
        let mut cv = Canvas::new(w, h, &mut px);
        fill(&mut cv, Some(&art), Some(0.0), 1.0);
        let n = 30;
        let t0 = std::time::Instant::now();
        for i in 0..n {
            fill(&mut cv, Some(&art), Some(i as f32 / 24.0), 1.0);
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("fundo animado 2560x1440: {ms:.2} ms por quadro");
        let src = source(Some(&art));
        let t1 = std::time::Instant::now();
        for i in 0..n {
            std::hint::black_box(field(&src, w.div_ceil(DOWN), h.div_ceil(DOWN), i as f32 / 24.0));
        }
        println!("  só a grade: {:.2} ms", t1.elapsed().as_secs_f64() * 1000.0 / n as f64);
        let t2 = std::time::Instant::now();
        for _ in 0..n {
            fill(&mut cv, Some(&art), Some(5.0), 1.0);
        }
        println!("  só a ampliação: {:.2} ms", t2.elapsed().as_secs_f64() * 1000.0 / n as f64);
    }
}
