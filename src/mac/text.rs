//! Texto e ícones no Mac, com a mesma interface de `text.rs` (Windows).
//!
//! Texto: **CoreText** com a fonte do sistema (SF Pro; o macOS troca sozinho
//! entre as versões "Text" e "Display" conforme o tamanho), desenhado numa
//! máscara de 8 bits só de alfa (antialias em tons de cinza, posição subpixel
//! na horizontal e linha de base alinhada ao pixel, como o AppKit).
//!
//! Ícones: **SF Symbols**. As constantes `G_*` são os nomes dos símbolos
//! ("play.fill"…) e as fontes `Font::Icon*` definem o tamanho; o símbolo é
//! rasterizado com o mesmo peso do texto e centralizado na caixa.

use crate::render::Mask;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSFont, NSFontWeight, NSFontWeightBold, NSFontWeightMedium, NSFontWeightRegular, NSFontWeightSemibold, NSGraphicsContext, NSImage, NSImageSymbolConfiguration,
};
use objc2_core_foundation::{CFAttributedString, CFBoolean, CFDictionary, CFRetained, CFString, CFType, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGAffineTransformIdentity, CGBitmapContextCreate, CGContext, CGImageAlphaInfo, CGInterpolationQuality};
use objc2_core_text::{CTFont, CTLine, CTLineTruncationType, kCTFontAttributeName, kCTForegroundColorFromContextAttributeName};
use objc2_foundation::{NSRect, NSString};
use std::collections::HashMap;
use std::rc::Rc;

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

// Ícones: nomes do catálogo SF Symbols.
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

/// (tamanho em pontos, peso, tipo) — os mesmos tamanhos do Windows (px a 96 DPI
/// = pontos do Mac a 1x).
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

/// Os SF Symbols ocupam menos da caixa que os glifos do Segoe Fluent Icons no
/// mesmo tamanho: este fator deixa os botões com o mesmo peso visual.
const SYMBOL_SCALE: f32 = 0.92;

fn weight(w: i32) -> NSFontWeight {
    unsafe {
        match w {
            ..=450 => NSFontWeightRegular,
            451..=550 => NSFontWeightMedium,
            551..=650 => NSFontWeightSemibold,
            _ => NSFontWeightBold,
        }
    }
}

type Key = (String, Font, i32, i32, Align);

pub struct TextRenderer {
    scale: f32,
    fonts: HashMap<Font, Retained<NSFont>>,
    /// Cache com descarte dos menos usados recentemente.
    cache: HashMap<Key, (Rc<Mask>, u64)>,
    tick: u64,
    capacity: usize,
}

impl TextRenderer {
    pub fn new() -> Self {
        Self { scale: 0.0, fonts: HashMap::new(), cache: HashMap::new(), tick: 0, capacity: 160 }
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
        self.fonts.clear();
    }

    fn font(&mut self, f: Font) -> Retained<NSFont> {
        let scale = self.scale.max(1.0);
        self.fonts
            .entry(f)
            .or_insert_with(|| {
                let (pt, w, _) = spec(f);
                NSFont::systemFontOfSize_weight((pt * scale) as CGFloat, weight(w))
            })
            .clone()
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
        let mut m = match spec(font).2 {
            Face::Icon => self.rasterize_symbol(text, font, w, h),
            Face::Text => self.rasterize_text(text, font, w, h, align),
        }
        .unwrap_or_else(|| Mask { w, h, cov: vec![0; w * h] });
        m.crop_right();
        let m = Rc::new(m);
        self.cache.insert(key, (m.clone(), self.tick));
        m
    }

    fn rasterize_text(&mut self, text: &str, font: Font, w: usize, h: usize, align: Align) -> Option<Mask> {
        let nsfont = self.font(font);
        // NSFont e CTFont são o mesmo objeto (toll-free bridging).
        let ct: &CTFont = unsafe { &*(Retained::as_ptr(&nsfont) as *const CTFont) };
        let attrs = unsafe {
            CFDictionary::<CFString, CFType>::from_slices(
                &[kCTFontAttributeName, kCTForegroundColorFromContextAttributeName],
                &[ct.as_ref(), CFBoolean::new(true).as_ref()],
            )
        };
        let line = |s: &str| -> Option<CFRetained<CTLine>> {
            let cf = CFString::from_str(s);
            let attr = unsafe { CFAttributedString::new(None, Some(&cf), Some(attrs.as_ref())) }?;
            Some(unsafe { CTLine::with_attributed_string(&attr) })
        };
        let full = line(text)?;
        let width = unsafe { full.typographic_bounds(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
        let line = if width > w as f64 {
            let ellipsis = line("…")?;
            unsafe { full.truncated_line(w as f64, CTLineTruncationType::End, Some(&ellipsis)) }.unwrap_or(full)
        } else {
            full
        };
        let width = unsafe { line.typographic_bounds(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
        let (ascent, descent) = unsafe { (ct.ascent(), ct.descent()) };
        let x = match align {
            Align::Left => 0.0,
            Align::Center => ((w as f64 - width) / 2.0).max(0.0),
        };
        // Coordenadas do CoreGraphics: origem embaixo. Linha centralizada na
        // caixa, com a linha de base num pixel inteiro (nitidez).
        let baseline = ((h as f64 - (ascent + descent)) / 2.0 + descent).round();
        alpha_bitmap(w, h, |ctx| unsafe {
            CGContext::set_text_matrix(Some(ctx), CGAffineTransformIdentity);
            CGContext::set_text_position(Some(ctx), x, baseline);
            line.draw(ctx);
        })
    }

    fn rasterize_symbol(&mut self, name: &str, font: Font, w: usize, h: usize) -> Option<Mask> {
        let (pt, wt, _) = spec(font);
        let size = (pt * self.scale.max(1.0) * SYMBOL_SCALE) as CGFloat;
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)?;
        let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(size, weight(wt));
        let image = image.imageWithSymbolConfiguration(&config)?;
        let sz = image.size();
        // Tamanho em pontos = pixels aqui (a escala já está no tamanho do símbolo).
        let (iw, ih) = (sz.width, sz.height);
        let x = ((w as f64 - iw) / 2.0).round();
        let y = ((h as f64 - ih) / 2.0).round();
        let dst = CGRect::new(CGPoint::new(x, y), CGSize::new(iw, ih));
        // O vetor do símbolo vai direto para a máscara, já no tamanho final.
        // (Antes virava imagem e era reduzido: em telas Retina o NSImage a
        // gerava em 2x e a redução serrilhava as bordas dos ícones.)
        let vector = alpha_bitmap(w, h, |ctx| {
            let gc = NSGraphicsContext::graphicsContextWithCGContext_flipped(ctx, false);
            NSGraphicsContext::saveGraphicsState_class();
            NSGraphicsContext::setCurrentContext(Some(&gc));
            image.drawInRect(dst);
            NSGraphicsContext::restoreGraphicsState_class();
        });
        if vector.as_ref().is_some_and(|m| m.cov.iter().any(|&c| c != 0)) {
            return vector;
        }
        // Reserva: imagem do símbolo, reduzida com interpolação de qualidade.
        let mut rect = NSRect::new(CGPoint::new(0.0, 0.0), sz);
        let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
        alpha_bitmap(w, h, |ctx| {
            CGContext::set_interpolation_quality(Some(ctx), CGInterpolationQuality::High);
            CGContext::draw_image(Some(ctx), dst, Some(&cg));
        })
    }
}

/// Contexto de 8 bits só com alfa: o que for desenhado vira a cobertura.
fn alpha_bitmap(w: usize, h: usize, draw: impl FnOnce(&CGContext)) -> Option<Mask> {
    let mut cov = vec![0u8; w * h];
    {
        let ctx = unsafe { CGBitmapContextCreate(cov.as_mut_ptr().cast(), w, h, 8, w, None, CGImageAlphaInfo::Only.0) }?;
        CGContext::set_should_antialias(Some(&ctx), true);
        CGContext::set_allows_font_subpixel_positioning(Some(&ctx), true);
        CGContext::set_should_subpixel_position_fonts(Some(&ctx), true);
        CGContext::set_should_smooth_fonts(Some(&ctx), false);
        CGContext::set_gray_fill_color(Some(&ctx), 1.0, 1.0);
        draw(&ctx);
        CGContext::flush(Some(&ctx));
    }
    // A primeira linha da memória é a de cima da imagem.
    Some(Mask { w, h, cov })
}
