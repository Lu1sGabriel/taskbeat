//! Texto e ícones no Linux, com a mesma interface de `text.rs` (Windows) e
//! `mac/text.rs`.
//!
//! Texto: a fonte da interface do sistema (o "sans-serif" do fontconfig:
//! Cantarell, Ubuntu Sans, Noto Sans…; `TASKBEAT_FONT` troca), com as fontes
//! de reserva que o próprio fontconfig ordena para japonês, coreano, árabe…
//! O HarfBuzz posiciona os glifos (kerning, ligaduras, escritas complexas) e
//! o FreeType os desenha numa máscara de 8 bits só de alfa: tons de cinza,
//! posição subpixel na horizontal e linha de base no pixel, como no Mac. Em
//! telas 1x usa o hinting "leve" (só vertical), que deixa o texto pequeno
//! mais nítido sem mudar as larguras.
//!
//! Ícones: não há fonte de ícones garantida no Linux; os botões usam os
//! ícones em vetor de `ui.rs` (os mesmos do Windows) e a nota musical é
//! desenhada aqui.

use crate::linux::ft::*;
use crate::render::Mask;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_long, c_uint};
use std::ptr::null_mut;
use std::rc::Rc;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Font {
    Title,
    Subtitle,
    Icon,
    IconSmall,
    /// Rótulo pequeno em negrito ("PRÓXIMA MÚSICA").
    Label,
    /// Título grande do cartão.
    Big,
    /// Tela de configurações.
    Heading,
    Body,
    BodyStrong,
    Caption,
    IconMd,
    /// Título do card expandido.
    CardTitle,
    /// Tempos da barra de progresso, rótulos pequenos.
    Small,
    /// Título grande das prévias / aviso de teclas.
    Display,
    IconLg,
    /// Símbolo grande do aviso de teclas (tamanho "Grande").
    Hero,
    /// Nota musical no lugar da capa grande.
    IconXl,
    /// Ícone do aviso de teclas no tamanho "Grande".
    IconHero,
    /// Título/segunda linha do widget nos tamanhos "Pequeno" e "Grande".
    TitleSm,
    TitleLg,
    SubtitleSm,
    SubtitleLg,
    /// Legenda em semibold (selos, cabeçalho do menu).
    CaptionStrong,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Align {
    Left,
    Center,
}

// Ícones: nomes (os mesmos do Mac); o desenho vem de `ui::vector_icon`.
pub const G_PREV: &str = "backward.fill";
pub const G_NEXT: &str = "forward.fill";
pub const G_PLAY: &str = "play.fill";
pub const G_PAUSE: &str = "pause.fill";
pub const G_MUTE: &str = "speaker.slash.fill";
pub const G_VOL1: &str = "speaker.wave.1.fill";
pub const G_VOL2: &str = "speaker.wave.2.fill";
pub const G_VOL3: &str = "speaker.wave.3.fill";
pub const G_MUSIC: &str = "music.note";
pub const G_SHUFFLE: &str = "shuffle";
pub const G_REPEAT_ALL: &str = "repeat";
pub const G_REPEAT_ONE: &str = "repeat.1";
pub const G_LOCK: &str = "lock.fill";
pub const G_UNLOCK: &str = "lock.open.fill";
pub const G_KEYBOARD: &str = "keyboard";
pub const G_NUMPAD: &str = "number.square";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Face {
    Text,
    Icon,
}

/// (tamanho em px a 1x, peso, tipo) — os mesmos tamanhos do Windows e do Mac.
fn spec(f: Font) -> (f32, i32, Face) {
    match f {
        Font::Title => (12.5, 600, Face::Text),
        Font::Subtitle => (11.5, 400, Face::Text),
        Font::Icon => (15.0, 500, Face::Icon),
        Font::IconSmall => (13.0, 500, Face::Icon),
        Font::Label => (10.0, 700, Face::Text),
        Font::Big => (14.5, 600, Face::Text),
        Font::Heading => (26.0, 700, Face::Text),
        Font::Body => (13.5, 400, Face::Text),
        Font::BodyStrong => (13.5, 600, Face::Text),
        Font::Caption => (11.5, 400, Face::Text),
        Font::IconMd => (16.0, 500, Face::Icon),
        Font::CardTitle => (16.5, 600, Face::Text),
        Font::Small => (11.0, 400, Face::Text),
        Font::Display => (18.0, 600, Face::Text),
        Font::IconLg => (18.0, 500, Face::Icon),
        Font::Hero => (24.0, 600, Face::Text),
        Font::IconXl => (40.0, 400, Face::Icon),
        Font::IconHero => (22.0, 500, Face::Icon),
        Font::TitleSm => (11.5, 600, Face::Text),
        Font::TitleLg => (14.0, 600, Face::Text),
        Font::SubtitleSm => (10.5, 400, Face::Text),
        Font::SubtitleLg => (12.5, 400, Face::Text),
        Font::CaptionStrong => (11.5, 600, Face::Text),
    }
}

/// Peso → posição na lista de fontes (normal, médio, seminegrito, negrito).
fn weight_slot(w: i32) -> usize {
    match w {
        ..=450 => 0,
        451..=550 => 1,
        551..=650 => 2,
        _ => 3,
    }
}

// ---------------------------------------------------------------- fontes do sistema

/// Uma fonte da lista do fontconfig: arquivo, índice (com a instância nomeada
/// das fontes variáveis) e os caracteres que ela tem.
struct FontRef {
    path: CString,
    index: c_long,
    charset: *mut FcCharSet,
}

/// Fontes por peso, da preferida às de reserva. Lida uma vez; só leitura depois.
struct Db {
    weights: [Vec<FontRef>; 4],
}

// Os charsets são cópias nossas e nunca mudam depois de lidos.
unsafe impl Send for Db {}
unsafe impl Sync for Db {}

fn db() -> &'static Db {
    static DB: OnceLock<Db> = OnceLock::new();
    DB.get_or_init(|| {
        let family = std::env::var("TASKBEAT_FONT").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "sans-serif".into());
        unsafe { FcInit() };
        Db { weights: [FC_WEIGHT_REGULAR, FC_WEIGHT_MEDIUM, FC_WEIGHT_DEMIBOLD, FC_WEIGHT_BOLD].map(|w| unsafe { sorted(&family, w) }) }
    })
}

/// Fontes para `family` no peso dado, na ordem do fontconfig (só as que
/// acrescentam caracteres). Fontes coloridas (emoji) e de bitmap ficam de
/// fora: a máscara é só de alfa.
unsafe fn sorted(family: &str, weight: c_int) -> Vec<FontRef> {
    let mut out = Vec::new();
    let Ok(name) = CString::new(family) else { return out };
    unsafe {
        let pat = FcNameParse(name.as_ptr().cast());
        if pat.is_null() {
            return out;
        }
        FcPatternAddInteger(pat, c"weight".as_ptr(), weight);
        FcConfigSubstitute(null_mut(), pat, FC_MATCH_PATTERN);
        FcDefaultSubstitute(pat);
        let mut res = 0;
        let set = FcFontSort(null_mut(), pat, 1, null_mut(), &mut res);
        FcPatternDestroy(pat);
        if set.is_null() {
            return out;
        }
        let s = &*set;
        for i in 0..s.nfont.max(0) as usize {
            let p = *s.fonts.add(i);
            let mut file: *mut u8 = null_mut();
            if FcPatternGetString(p, c"file".as_ptr(), 0, &mut file) != FC_RESULT_MATCH || file.is_null() {
                continue;
            }
            let mut flag = 0;
            if FcPatternGetBool(p, c"color".as_ptr(), 0, &mut flag) == FC_RESULT_MATCH && flag != 0 {
                continue;
            }
            if FcPatternGetBool(p, c"scalable".as_ptr(), 0, &mut flag) == FC_RESULT_MATCH && flag == 0 {
                continue;
            }
            let mut cs: *mut FcCharSet = null_mut();
            if FcPatternGetCharSet(p, c"charset".as_ptr(), 0, &mut cs) != FC_RESULT_MATCH || cs.is_null() {
                continue;
            }
            let mut index = 0;
            FcPatternGetInteger(p, c"index".as_ptr(), 0, &mut index);
            out.push(FontRef { path: CStr::from_ptr(file as *const c_char).to_owned(), index: index as c_long, charset: FcCharSetCopy(cs) });
            if out.len() >= 64 {
                break;
            }
        }
        FcFontSetDestroy(set);
    }
    out
}

/// Nome da fonte principal (diagnóstico).
pub fn primary_font_file() -> Option<String> {
    db().weights[0].first().map(|f| f.path.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------- desenho

struct Loaded {
    ft: FT_Face,
    hb: *mut hb_font_t,
    /// Tamanho atual da face (px); o FreeType guarda um só por face.
    px: f32,
}

#[derive(Clone, Copy)]
struct Glyph {
    face: usize,
    gid: u32,
    x: f32,
    y: f32,
}

type Key = (String, Font, i32, i32, Align);

pub struct TextRenderer {
    scale: f32,
    lib: FT_Library,
    buf: *mut hb_buffer_t,
    faces: Vec<Loaded>,
    /// (peso, posição na lista) → face carregada (`None`: o arquivo não abriu).
    by_ref: HashMap<(usize, usize), Option<usize>>,
    by_file: HashMap<(CString, c_long), usize>,
    /// Cache com descarte dos menos usados recentemente.
    cache: HashMap<Key, (Rc<Mask>, u64)>,
    tick: u64,
    capacity: usize,
}

impl TextRenderer {
    pub fn new() -> Self {
        let mut lib: FT_Library = null_mut();
        unsafe {
            if FT_Init_FreeType(&mut lib) != 0 {
                lib = null_mut();
            }
        }
        let buf = unsafe { hb_buffer_create() };
        Self { scale: 0.0, lib, buf, faces: Vec::new(), by_ref: HashMap::new(), by_file: HashMap::new(), cache: HashMap::new(), tick: 0, capacity: 160 }
    }

    /// Máximo de máscaras guardadas (a tela de configurações usa bem mais textos).
    pub fn set_capacity(&mut self, n: usize) {
        self.capacity = n.max(16);
    }

    pub fn set_scale(&mut self, scale: f32) {
        if (scale - self.scale).abs() < 0.001 {
            return;
        }
        self.scale = scale;
        self.cache.clear();
    }

    /// Máscara do texto em uma caixa `w`×`h` (centralizado na vertical, com "…" se não couber).
    pub fn mask(&mut self, text: &str, font: Font, w: i32, h: i32, align: Align) -> Rc<Mask> {
        self.tick += 1;
        let key: Key = (text.to_string(), font, w, h, align);
        if let Some((m, used)) = self.cache.get_mut(&key) {
            *used = self.tick;
            return m.clone();
        }
        if self.cache.len() >= self.capacity {
            // Descarta a metade usada há mais tempo.
            let mut ticks: Vec<u64> = self.cache.values().map(|(_, t)| *t).collect();
            ticks.sort_unstable();
            let cut = ticks[ticks.len() / 2];
            self.cache.retain(|_, (_, t)| *t > cut);
        }
        let (w, h) = (w.max(1) as usize, h.max(1) as usize);
        let (pt, weight, face) = spec(font);
        let px = pt * self.scale.max(1.0);
        let m = match face {
            Face::Icon => icon(text, px, w, h),
            Face::Text => {
                let mut m = self.rasterize(text, weight_slot(weight), px, w, h, align);
                m.crop_right();
                Rc::new(m)
            }
        };
        self.cache.insert(key, (m.clone(), self.tick));
        m
    }

    /// Carrega (uma vez) a fonte `i` da lista do peso `slot`.
    fn face(&mut self, slot: usize, i: usize) -> Option<usize> {
        if let Some(r) = self.by_ref.get(&(slot, i)) {
            return *r;
        }
        let r = db().weights[slot].get(i).and_then(|f| {
            let k = (f.path.clone(), f.index);
            if let Some(&idx) = self.by_file.get(&k) {
                return Some(idx);
            }
            if self.lib.is_null() {
                return None;
            }
            let mut ft: FT_Face = null_mut();
            unsafe {
                if FT_New_Face(self.lib, f.path.as_ptr(), f.index, &mut ft) != 0 || ft.is_null() {
                    return None;
                }
                let hb = hb_ft_font_create_referenced(ft);
                hb_ft_font_set_load_flags(hb, FT_LOAD_NO_HINTING);
                self.faces.push(Loaded { ft, hb, px: 0.0 });
            }
            let idx = self.faces.len() - 1;
            self.by_file.insert(k, idx);
            Some(idx)
        });
        self.by_ref.insert((slot, i), r);
        r
    }

    fn sized(&mut self, face: usize, px: f32) -> &Loaded {
        let f = &mut self.faces[face];
        if (f.px - px).abs() > 0.001 {
            unsafe {
                FT_Set_Char_Size(f.ft, 0, (px * 64.0).round() as c_long, 72, 72);
                hb_ft_font_changed(f.hb);
            }
            f.px = px;
        }
        &self.faces[face]
    }

    /// Fonte para o caractere `c`: a primeira da lista que o tem.
    fn face_for(&mut self, slot: usize, c: char) -> Option<usize> {
        let n = db().weights[slot].len();
        for i in 0..n {
            let has = unsafe { FcCharSetHasChar(db().weights[slot][i].charset, c as u32) != 0 };
            if has {
                if let Some(f) = self.face(slot, i) {
                    return Some(f);
                }
            }
        }
        self.face(slot, 0)
    }

    /// Glifos posicionados (px, linha de base em y = 0) e a largura total.
    fn shape(&mut self, text: &str, slot: usize, px: f32) -> (Vec<Glyph>, f32) {
        // Trechos com a mesma fonte (acentos combinantes, emendas e espaços
        // ficam com a fonte do trecho anterior).
        let mut runs: Vec<(usize, usize, usize)> = Vec::new();
        for (i, c) in text.char_indices() {
            let sticky = c.is_whitespace() || is_mark(c);
            let f = match runs.last() {
                Some(&(f, _, _)) if sticky => Some(f),
                _ => self.face_for(slot, c),
            };
            let Some(f) = f else { continue };
            match runs.last_mut() {
                Some(r) if r.0 == f && r.2 == i => r.2 = i + c.len_utf8(),
                _ => runs.push((f, i, i + c.len_utf8())),
            }
        }
        let mut out = Vec::new();
        let mut pen = 0.0f32;
        if self.buf.is_null() {
            return (out, 0.0);
        }
        for (f, a, b) in runs {
            let hb = self.sized(f, px).hb;
            unsafe {
                hb_buffer_clear_contents(self.buf);
                hb_buffer_add_utf8(self.buf, text.as_ptr().cast(), text.len() as c_int, a as c_uint, (b - a) as c_int);
                hb_buffer_guess_segment_properties(self.buf);
                hb_shape(hb, self.buf, std::ptr::null(), 0);
                let mut n: c_uint = 0;
                let infos = hb_buffer_get_glyph_infos(self.buf, &mut n);
                let pos = hb_buffer_get_glyph_positions(self.buf, &mut n);
                for k in 0..n as usize {
                    let (gi, gp) = (*infos.add(k), *pos.add(k));
                    out.push(Glyph { face: f, gid: gi.codepoint, x: pen + gp.x_offset as f32 / 64.0, y: -(gp.y_offset as f32) / 64.0 });
                    pen += gp.x_advance as f32 / 64.0;
                }
            }
        }
        (out, pen)
    }

    /// Texto que cabe em `w` px: inteiro ou cortado com "…" no fim.
    fn fit(&mut self, text: &str, slot: usize, px: f32, w: f32) -> (Vec<Glyph>, f32) {
        let full = self.shape(text, slot, px);
        if full.1 <= w {
            return full;
        }
        let ends: Vec<usize> = text.char_indices().map(|(i, _)| i).skip(1).collect();
        let (mut lo, mut hi) = (0usize, ends.len());
        let mut best = self.shape("…", slot, px);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let cut = format!("{}…", text[..ends[mid - 1]].trim_end());
            let r = self.shape(&cut, slot, px);
            if r.1 <= w {
                best = r;
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        best
    }

    fn rasterize(&mut self, text: &str, slot: usize, px: f32, w: usize, h: usize, align: Align) -> Mask {
        let mut cov = vec![0u8; w * h];
        if text.is_empty() || self.lib.is_null() {
            return Mask { w, h, cov };
        }
        let (glyphs, width) = self.fit(text, slot, px, w as f32);
        // Linha centralizada na caixa pela altura da fonte principal, com a
        // linha de base num pixel inteiro (nitidez).
        let (asc, desc) = match self.face(slot, 0) {
            Some(f) => {
                let m = unsafe { &(*(*self.sized(f, px).ft).size).metrics };
                (m.ascender as f32 / 64.0, -(m.descender as f32) / 64.0)
            }
            None => (px * 0.95, px * 0.25),
        };
        let mut baseline = ((h as f32 - (asc + desc)) / 2.0 + asc).round() as i32;
        let x0 = match align {
            Align::Left => 0.0,
            Align::Center => ((w as f32 - width) / 2.0).max(0.0),
        };
        let flags = FT_LOAD_RENDER | FT_LOAD_NO_BITMAP | if self.scale < 1.5 { FT_LOAD_TARGET_LIGHT } else { FT_LOAD_NO_HINTING };
        // Glifos desenhados: (x, topo relativo à linha de base, largura, altura, cobertura).
        let mut drawn: Vec<(i32, i32, usize, usize, Vec<u8>)> = Vec::with_capacity(glyphs.len());
        for g in glyphs {
            let ft = self.sized(g.face, px).ft;
            let fx = x0 + g.x;
            let ix = fx.floor();
            let mut delta = FT_Vector { x: ((fx - ix) * 64.0).round() as c_long, y: 0 };
            unsafe {
                FT_Set_Transform(ft, null_mut(), &mut delta);
                let err = FT_Load_Glyph(ft, g.gid, flags);
                FT_Set_Transform(ft, null_mut(), null_mut());
                if err != 0 {
                    continue;
                }
                let slot = &*(*ft).glyph;
                let bm = &slot.bitmap;
                if bm.pixel_mode != FT_PIXEL_MODE_GRAY || bm.buffer.is_null() || bm.width == 0 || bm.rows == 0 {
                    continue;
                }
                let (bw, bh) = (bm.width as usize, bm.rows as usize);
                let mut data = Vec::with_capacity(bw * bh);
                for row in 0..bh {
                    let src = bm.buffer.offset(row as isize * bm.pitch as isize);
                    data.extend_from_slice(std::slice::from_raw_parts(src, bw));
                }
                drawn.push((ix as i32 + slot.bitmap_left, -slot.bitmap_top + g.y.round() as i32, bw, bh, data));
            }
        }
        // Fontes mais altas que a caixa (a Noto tem descendentes longos): sobe
        // a linha até o "g" e o "y" caberem, se houver espaço em cima.
        if let (Some(top), Some(bottom)) = (drawn.iter().map(|d| d.1).min(), drawn.iter().map(|d| d.1 + d.3 as i32).max()) {
            let over = baseline + bottom - h as i32;
            let room = baseline + top;
            if over > 0 && room > 0 {
                baseline -= over.min(room);
            }
        }
        for (left, rel, bw, bh, data) in drawn {
            let top = baseline + rel;
            for row in 0..bh as i32 {
                let y = top + row;
                if y < 0 || y >= h as i32 {
                    continue;
                }
                for col in 0..bw as i32 {
                    let x = left + col;
                    if x < 0 || x >= w as i32 {
                        continue;
                    }
                    let v = data[row as usize * bw + col as usize] as u32;
                    if v == 0 {
                        continue;
                    }
                    let d = &mut cov[y as usize * w + x as usize];
                    // "Sobre": partes de glifos que se encostam não clareiam.
                    let a = *d as u32;
                    *d = (a + v - a * v / 255).min(255) as u8;
                }
            }
        }
        Mask { w, h, cov }
    }
}

impl Drop for TextRenderer {
    fn drop(&mut self) {
        unsafe {
            for f in self.faces.drain(..) {
                hb_font_destroy(f.hb);
                FT_Done_Face(f.ft);
            }
            if !self.buf.is_null() {
                hb_buffer_destroy(self.buf);
            }
            if !self.lib.is_null() {
                FT_Done_FreeType(self.lib);
            }
        }
    }
}

/// Acentos combinantes, emendas e seletores de variação.
fn is_mark(c: char) -> bool {
    matches!(c as u32, 0x300..=0x36F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F | 0x200C | 0x200D | 0xE0100..=0xE01EF)
}

// ---------------------------------------------------------------- ícones

/// Ícone `g` com `em` px, centrado na caixa.
fn icon(g: &str, em: f32, w: usize, h: usize) -> Rc<Mask> {
    if let Some(m) = crate::ui::vector_icon(g, em, w as i32, h as i32) {
        return m;
    }
    let mut cov = vec![0u8; w * h];
    if g == G_MUSIC {
        sdf_fill(&mut cov, w, h, em, music_note);
    }
    Rc::new(Mask { w, h, cov })
}

/// Preenche a máscara pela distância com sinal `sd` (em "em", centro 0,0, y para baixo), 4 amostras por pixel.
fn sdf_fill(cov: &mut [u8], w: usize, h: usize, em: f32, sd: impl Fn(f32, f32) -> f32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (ox, oy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let (px, py) = ((x as f32 + ox - cx) / em, (y as f32 + oy - cy) / em);
                acc += (0.5 - sd(px, py) * em * 2.0).clamp(0.0, 1.0);
            }
            cov[y * w + x] = (acc / 4.0 * 255.0 + 0.5) as u8;
        }
    }
}

fn seg(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (pax, pay, bax, bay) = (px - a.0, py - a.1, b.0 - a.0, b.1 - a.1);
    let t = ((pax * bax + pay * bay) / (bax * bax + bay * bay).max(1e-9)).clamp(0.0, 1.0);
    (pax - bax * t).hypot(pay - bay * t)
}

/// Colcheia (♪): cabeça oval inclinada, haste e bandeirola.
fn music_note(x: f32, y: f32) -> f32 {
    // Cabeça: elipse girada.
    let (hx, hy) = (x + 0.13, y - 0.25);
    let (c, s) = ((-0.42f32).cos(), (-0.42f32).sin());
    let (u, v) = (hx * c - hy * s, hx * s + hy * c);
    let (a, b) = (0.19, 0.135);
    let k = ((u / a).powi(2) + (v / b).powi(2)).sqrt();
    let head = (k - 1.0) * a.min(b);
    // Haste.
    let stem = seg(x, y, (0.045, 0.22), (0.045, -0.40)) - 0.045;
    // Bandeirola: curva para a direita e para baixo.
    let mut flag = f32::MAX;
    let pts = [(0.045, -0.40), (0.15, -0.30), (0.27, -0.20), (0.30, -0.06), (0.24, 0.06)];
    for w in pts.windows(2) {
        flag = flag.min(seg(x, y, w[0], w[1]) - 0.045);
    }
    head.min(stem).min(flag)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O sistema tem uma fonte e o texto sai: com tinta, antialias e largura
    /// que cresce com o texto.
    #[test]
    fn text_renders() {
        let mut tr = TextRenderer::new();
        tr.set_scale(2.0);
        let short = tr.mask("Midnight", Font::CardTitle, 600, 40, Align::Left);
        let long = tr.mask("Midnight City — M83", Font::CardTitle, 600, 40, Align::Left);
        let inked = short.cov.iter().filter(|&&c| c > 0).count();
        let partial = short.cov.iter().filter(|&&c| c > 0 && c < 250).count();
        assert!(inked > 200, "texto vazio (fonte: {:?})", primary_font_file());
        assert!(partial > 40, "sem antialias");
        assert!(long.used_width() > short.used_width() + 50.0);
        // Não cabe: corta com reticências dentro da caixa.
        let cut = tr.mask("Uma música com um nome comprido demais para caber", Font::Body, 120, 30, Align::Left);
        assert!(cut.used_width() <= 120.0);
    }

    #[test]
    fn icons_render() {
        let mut tr = TextRenderer::new();
        tr.set_scale(2.0);
        for g in [G_PLAY, G_PAUSE, G_NEXT, G_PREV, G_SHUFFLE, G_REPEAT_ALL, G_REPEAT_ONE, G_VOL2, G_MUTE, G_MUSIC] {
            let m = tr.mask(g, Font::Icon, 40, 40, Align::Center);
            let inked = m.cov.iter().filter(|&&c| c > 0).count();
            let solid = m.cov.iter().filter(|&&c| c > 250).count();
            assert!(inked > 40, "{g} vazio");
            assert!(solid * 100 < 40 * 40 * 60, "{g} parece um bloco");
        }
    }
}
