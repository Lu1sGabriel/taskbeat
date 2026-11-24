//! Ponte entre os buffers do app (BGRA pré-multiplicado, `0xAARRGGBB`) e o
//! CoreGraphics: quadro → `CGImage` para a camada da janela, e imagens
//! (capa JPEG/PNG, ícone de app) → `Image` do app.

use crate::render::Image;
use objc2_core_foundation::{CFData, CFRetained, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGContext, CGDataProvider, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo, CGInterpolationQuality, kCGColorSpaceSRGB,
};

/// BGRA em little-endian com alfa pré-multiplicado na frente: é o `u32` do app.
const BGRA: u32 = CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::PremultipliedFirst.0;

fn srgb() -> Option<CFRetained<CGColorSpace>> {
    CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
}

/// Copia um quadro para um `CGImage` (o conteúdo da camada da janela).
pub fn to_cgimage(px: &[u32], w: usize, h: usize) -> Option<CFRetained<CGImage>> {
    if w == 0 || h == 0 {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(px.as_ptr().cast::<u8>(), w * h * 4) };
    let data = CFData::from_bytes(bytes);
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    let space = srgb()?;
    unsafe { CGImage::new(w, h, 8, 32, w * 4, Some(&space), CGBitmapInfo(BGRA), Some(&provider), std::ptr::null(), false, CGColorRenderingIntent::RenderingIntentDefault) }
}

/// Desenha `img` cobrindo um quadrado `size`×`size` (corta as sobras, como
/// "aspect fill") e devolve no formato do app.
pub fn cover_square(img: &CGImage, size: usize) -> Option<Image> {
    let (iw, ih) = (CGImage::width(Some(img)) as f64, CGImage::height(Some(img)) as f64);
    if iw <= 0.0 || ih <= 0.0 {
        return None;
    }
    let k = (size as f64 / iw).max(size as f64 / ih);
    let (dw, dh) = (iw * k, ih * k);
    let rect = CGRect::new(CGPoint::new((size as f64 - dw) / 2.0, (size as f64 - dh) / 2.0), CGSize::new(dw, dh));
    draw_into(size, size, |ctx| CGContext::draw_image(Some(ctx), rect, Some(img)))
}

/// Desenha `img` inteiro dentro de `w`×`h` (ícones).
pub fn fit(img: &CGImage, w: usize, h: usize) -> Option<Image> {
    let rect = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as CGFloat, h as CGFloat));
    draw_into(w, h, |ctx| CGContext::draw_image(Some(ctx), rect, Some(img)))
}

fn draw_into(w: usize, h: usize, draw: impl FnOnce(&CGContext)) -> Option<Image> {
    let mut px = vec![0u32; w * h];
    {
        let space = srgb()?;
        let ctx = unsafe { CGBitmapContextCreate(px.as_mut_ptr().cast(), w, h, 8, w * 4, Some(&space), BGRA) }?;
        CGContext::set_interpolation_quality(Some(&ctx), CGInterpolationQuality::High);
        draw(&ctx);
        CGContext::flush(Some(&ctx));
    }
    Some(Image::new(w, h, px))
}

/// Decodifica JPEG ou PNG (capa do álbum) num quadrado `size`×`size`.
pub fn decode_square(bytes: &[u8], size: usize) -> Option<Image> {
    let data = CFData::from_bytes(bytes);
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    let png = bytes.starts_with(b"\x89PNG");
    let img = unsafe {
        if png {
            CGImage::with_png_data_provider(Some(&provider), std::ptr::null(), true, CGColorRenderingIntent::RenderingIntentDefault)
        } else {
            CGImage::with_jpeg_data_provider(Some(&provider), std::ptr::null(), true, CGColorRenderingIntent::RenderingIntentDefault)
        }
    };
    match img {
        Some(img) => cover_square(&img, size),
        // Outros formatos (HEIC, TIFF…): o NSImage sabe abrir.
        None => decode_with_nsimage(bytes, size),
    }
}

fn decode_with_nsimage(bytes: &[u8], size: usize) -> Option<Image> {
    use objc2::AllocAnyThread;
    use objc2_app_kit::NSImage;
    use objc2_foundation::{NSData, NSRect};
    let data = NSData::with_bytes(bytes);
    let image = NSImage::initWithData(NSImage::alloc(), &data)?;
    let mut rect = NSRect::new(CGPoint::new(0.0, 0.0), image.size());
    let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
    cover_square(&cg, size)
}

/// Base64 padrão (o adaptador do "Tocando Agora" manda a capa assim).
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in s.as_bytes() {
        if c == b'=' {
            break;
        }
        if c.is_ascii_whitespace() {
            continue;
        }
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64() {
        assert_eq!(super::base64_decode("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
        assert_eq!(super::base64_decode("YQ==").unwrap(), b"a");
    }
}
