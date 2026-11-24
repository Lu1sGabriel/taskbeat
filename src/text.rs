//! Texto e ícones (glifos da fonte de ícones do Windows) rasterizados em
//! máscaras de cobertura e reaproveitados.
//!
//! Caminho principal: **DirectWrite** (via Direct2D em modo software, sem GPU)
//! com anti-aliasing em tons de cinza "natural symmetric": suaviza na
//! horizontal *e* na vertical e usa posicionamento subpixel, como o WinUI e o
//! WPF. O GDI antigo encaixava as letras na grade de pixels e deixava curvas
//! serrilhadas. Se o Direct2D não estiver disponível, cai no GDI (ícones com
//! supersampling 4x).

use crate::render::Mask;
use std::collections::HashMap;
use std::rc::Rc;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::core::{HSTRING, w};

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
    /// Tela de configurações (tipografia Fluent / Segoe UI Variable).
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
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Align {
    Left,
    Center,
}

// Glifos (Segoe Fluent Icons / Segoe MDL2 Assets).
pub const G_PREV: &str = "\u{E892}";
pub const G_NEXT: &str = "\u{E893}";
pub const G_PLAY: &str = "\u{E768}";
pub const G_PAUSE: &str = "\u{E769}";
pub const G_MUTE: &str = "\u{E74F}";
pub const G_VOL1: &str = "\u{E993}";
pub const G_VOL2: &str = "\u{E994}";
pub const G_VOL3: &str = "\u{E995}";
pub const G_MUSIC: &str = "\u{E8D6}";
pub const G_SHUFFLE: &str = "\u{E8B1}";
pub const G_REPEAT_ALL: &str = "\u{E8EE}";
pub const G_REPEAT_ONE: &str = "\u{E8ED}";
pub const G_LOCK: &str = "\u{E72E}";
pub const G_UNLOCK: &str = "\u{E785}";
pub const G_KEYBOARD: &str = "\u{E765}";
pub const G_NUMPAD: &str = "\u{E8EF}";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Face {
    Text,
    Display,
    Icon,
}

/// (tamanho em px a 96 DPI, peso, família)
fn spec(f: Font) -> (f32, i32, Face) {
    match f {
        Font::Title => (12.5, 600, Face::Text),
        Font::Subtitle => (11.5, 400, Face::Text),
        Font::Icon => (15.0, 400, Face::Icon),
        Font::IconSmall => (13.0, 400, Face::Icon),
        Font::Label => (10.0, 700, Face::Text),
        Font::Big => (14.5, 600, Face::Text),
        Font::Heading => (26.0, 600, Face::Display),
        Font::Body => (14.0, 400, Face::Text),
        Font::BodyStrong => (14.0, 600, Face::Text),
        Font::Caption => (12.0, 400, Face::Text),
        Font::IconMd => (16.0, 400, Face::Icon),
        Font::CardTitle => (16.5, 600, Face::Display),
        Font::Small => (11.0, 400, Face::Text),
        Font::Display => (18.0, 600, Face::Display),
        Font::IconLg => (18.0, 400, Face::Icon),
        Font::Hero => (24.0, 600, Face::Display),
        Font::IconXl => (40.0, 400, Face::Icon),
        Font::IconHero => (22.0, 400, Face::Icon),
    }
}

const ALL_FONTS: [Font; 18] = [
    Font::Title,
    Font::Subtitle,
    Font::Icon,
    Font::IconSmall,
    Font::Label,
    Font::Big,
    Font::Heading,
    Font::Body,
    Font::BodyStrong,
    Font::Caption,
    Font::IconMd,
    Font::CardTitle,
    Font::Small,
    Font::Display,
    Font::IconLg,
    Font::Hero,
    Font::IconXl,
    Font::IconHero,
];

type Key = (String, Font, i32, i32, Align);

/// Direct2D (software) + DirectWrite.
struct Dw {
    factory: IDWriteFactory,
    _d2d: ID2D1Factory,
    rt: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
    formats: HashMap<Font, IDWriteTextFormat>,
}

impl Dw {
    fn new() -> Option<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok()?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                // Software: não cria dispositivo de GPU (bem menos memória).
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let rt = d2d.CreateDCRenderTarget(&props).ok()?;
            let brush = rt.CreateSolidColorBrush(&D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, None).ok()?;
            let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            if let Ok(def) = factory.CreateRenderingParams() {
                // Contraste um pouco maior: texto claro sobre fundo escuro com
                // alfa linear tende a parecer fino.
                let contrast = (def.GetEnhancedContrast() + 0.25).min(1.5);
                if let Ok(p) = factory.CreateCustomRenderingParams(def.GetGamma(), contrast, 0.0, DWRITE_PIXEL_GEOMETRY_FLAT, DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC) {
                    rt.SetTextRenderingParams(&p);
                }
            }
            Some(Self { factory, _d2d: d2d, rt, brush, formats: HashMap::new() })
        }
    }

    fn family_exists(&self, name: &str) -> bool {
        unsafe {
            let mut coll: Option<IDWriteFontCollection> = None;
            if self.factory.GetSystemFontCollection(&mut coll, false).is_err() {
                return false;
            }
            let Some(coll) = coll else { return false };
            let (mut idx, mut exists) = (0u32, windows::core::BOOL(0));
            coll.FindFamilyName(&HSTRING::from(name), &mut idx, &mut exists).is_ok() && exists.as_bool()
        }
    }

    fn build_formats(&mut self, scale: f32) {
        self.formats.clear();
        let text = if self.family_exists("Segoe UI Variable Text") { "Segoe UI Variable Text" } else { "Segoe UI" };
        let display = if self.family_exists("Segoe UI Variable Display") { "Segoe UI Variable Display" } else { text };
        let icon = if self.family_exists("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" };
        for f in ALL_FONTS {
            let (px, weight, face) = spec(f);
            let family = match face {
                Face::Text => text,
                Face::Display => display,
                Face::Icon => icon,
            };
            unsafe {
                let Ok(fmt) = self.factory.CreateTextFormat(
                    &HSTRING::from(family),
                    None::<&IDWriteFontCollection>,
                    DWRITE_FONT_WEIGHT(weight),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    px * scale,
                    w!("pt-br"),
                ) else {
                    continue;
                };
                let _ = fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
                let _ = fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                if face != Face::Icon {
                    if let Ok(sign) = self.factory.CreateEllipsisTrimmingSign(&fmt) {
                        let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                        let _ = fmt.SetTrimming(&trim, &sign);
                    }
                }
                self.formats.insert(f, fmt);
            }
        }
    }
}

pub struct TextRenderer {
    dc: HDC,
    scale: f32,
    dw: Option<Dw>,
    /// Fallback GDI.
    fonts: HashMap<Font, HFONT>,
    /// Versões 4x das fontes de ícone (supersampling, só no fallback GDI).
    fonts_ss: HashMap<Font, HFONT>,
    /// Cache com descarte dos menos usados recentemente.
    cache: HashMap<Key, (Rc<Mask>, u64)>,
    tick: u64,
    capacity: usize,
}

impl TextRenderer {
    pub fn new() -> Self {
        let dc = unsafe { CreateCompatibleDC(None) };
        Self { dc, scale: 0.0, dw: Dw::new(), fonts: HashMap::new(), fonts_ss: HashMap::new(), cache: HashMap::new(), tick: 0, capacity: 160 }
    }

    /// Máximo de máscaras guardadas (a tela de configurações usa bem mais textos).
    pub fn set_capacity(&mut self, n: usize) {
        self.capacity = n.max(16);
    }

    fn face_exists(&self, face: &str) -> bool {
        unsafe {
            let f = CreateFontW(-12, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, 0, &HSTRING::from(face));
            let old = SelectObject(self.dc, f.into());
            let mut buf = [0u16; 64];
            let n = GetTextFaceW(self.dc, Some(&mut buf));
            SelectObject(self.dc, old);
            let _ = DeleteObject(f.into());
            String::from_utf16_lossy(&buf[..n.max(1) as usize - 1]).eq_ignore_ascii_case(face)
        }
    }

    pub fn set_scale(&mut self, scale: f32) {
        if (scale - self.scale).abs() < 0.001 {
            return;
        }
        self.scale = scale;
        self.release_fonts();
        self.cache.clear();
        if let Some(dw) = &mut self.dw {
            dw.build_formats(scale);
            if dw.formats.len() == ALL_FONTS.len() {
                return;
            }
            self.dw = None;
        }
        let text_face = if self.face_exists("Segoe UI Variable Text") { "Segoe UI Variable Text" } else { "Segoe UI" };
        let display = if self.face_exists("Segoe UI Variable Display") { "Segoe UI Variable Display" } else { text_face };
        let icon_face = if self.face_exists("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" };
        let mk = |px: f32, weight: i32, face: &str| unsafe {
            CreateFontW(
                -((px * scale).round() as i32),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                0,
                &HSTRING::from(face),
            )
        };
        for f in ALL_FONTS {
            let (px, weight, face) = spec(f);
            let name = match face {
                Face::Text => text_face,
                Face::Display => display,
                Face::Icon => icon_face,
            };
            self.fonts.insert(f, mk(px, weight, name));
            if face == Face::Icon {
                self.fonts_ss.insert(f, mk(px * 4.0, weight, name));
            }
        }
    }

    fn release_fonts(&mut self) {
        for (_, f) in self.fonts.drain().chain(self.fonts_ss.drain()) {
            unsafe {
                let _ = DeleteObject(f.into());
            }
        }
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
        let m = Rc::new(self.rasterize(text, font, w.max(1), h.max(1), align));
        self.cache.insert(key, (m.clone(), self.tick));
        m
    }

    fn rasterize(&self, text: &str, font: Font, w: i32, h: i32, align: Align) -> Mask {
        if let Some(dw) = &self.dw {
            if let Some(m) = self.rasterize_dw(dw, text, font, w, h, align) {
                return m;
            }
        }
        // Ícones no GDI: desenha em 4x e reduz com média -> bordas suaves.
        if let Some(&big) = self.fonts_ss.get(&font) {
            const K: i32 = 4;
            let hi = self.rasterize_gdi(text, big, w * K, h * K, align);
            let mut out = Mask { w: w as usize, h: h as usize, cov: vec![0; (w * h) as usize] };
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let mut acc = 0u32;
                    for dy in 0..K as usize {
                        let row = (y * K as usize + dy) * hi.w;
                        for dx in 0..K as usize {
                            acc += hi.cov[row + x * K as usize + dx] as u32;
                        }
                    }
                    out.cov[y * w as usize + x] = (acc / (K * K) as u32) as u8;
                }
            }
            return out;
        }
        match self.fonts.get(&font).copied() {
            Some(f) => self.rasterize_gdi(text, f, w, h, align),
            None => Mask { w: w as usize, h: h as usize, cov: vec![0; (w * h) as usize] },
        }
    }

    /// Cria um DIB 32bpp `w`×`h` selecionado no DC, chama `draw` e converte os pixels.
    fn with_dib(&self, w: i32, h: i32, draw: impl FnOnce(), read: impl Fn(u32) -> u8) -> Option<Mask> {
        unsafe {
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(self.dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            let old_bmp = SelectObject(self.dc, bmp.into());
            draw();
            let _ = GdiFlush();
            let px = std::slice::from_raw_parts(bits as *const u32, (w * h) as usize);
            let cov = px.iter().map(|&p| read(p)).collect();
            SelectObject(self.dc, old_bmp);
            let _ = DeleteObject(bmp.into());
            Some(Mask { w: w as usize, h: h as usize, cov })
        }
    }

    fn rasterize_dw(&self, dw: &Dw, text: &str, font: Font, w: i32, h: i32, align: Align) -> Option<Mask> {
        let fmt = dw.formats.get(&font)?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut ok = false;
        let mask = self.with_dib(
            w,
            h,
            || unsafe {
                let rc = RECT { left: 0, top: 0, right: w, bottom: h };
                if dw.rt.BindDC(self.dc, &rc).is_err() {
                    return;
                }
                let _ = fmt.SetTextAlignment(if align == Align::Center { DWRITE_TEXT_ALIGNMENT_CENTER } else { DWRITE_TEXT_ALIGNMENT_LEADING });
                dw.rt.BeginDraw();
                dw.rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
                let rect = D2D_RECT_F { left: 0.0, top: 0.0, right: w as f32, bottom: h as f32 };
                dw.rt.DrawText(&wide, fmt, &rect, &dw.brush, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
                ok = dw.rt.EndDraw(None, None).is_ok();
            },
            // Texto branco pré-multiplicado: o alfa é a cobertura.
            |p| (p >> 24) as u8,
        )?;
        ok.then_some(mask)
    }

    fn rasterize_gdi(&self, text: &str, hfont: HFONT, w: i32, h: i32, align: Align) -> Mask {
        let m = self.with_dib(
            w,
            h,
            || unsafe {
                let old_font = SelectObject(self.dc, hfont.into());
                SetTextColor(self.dc, COLORREF(0x00FF_FFFF));
                SetBkMode(self.dc, TRANSPARENT);
                let mut rc = RECT { left: 0, top: 0, right: w, bottom: h };
                let mut wide: Vec<u16> = text.encode_utf16().collect();
                let mut fmt = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS;
                fmt |= if align == Align::Center { DT_CENTER } else { DT_LEFT };
                DrawTextW(self.dc, &mut wide, &mut rc, fmt);
                SelectObject(self.dc, old_font);
            },
            |p| (((p >> 16) & 0xff).max((p >> 8) & 0xff).max(p & 0xff)) as u8,
        );
        m.unwrap_or(Mask { w: w as usize, h: h as usize, cov: vec![0; (w * h) as usize] })
    }
}

impl Drop for TextRenderer {
    fn drop(&mut self) {
        self.release_fonts();
        unsafe {
            let _ = DeleteDC(self.dc);
        }
    }
}
