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

/// Maior lado copiado do original pixel a pixel (16 MB de pixels). Capas
/// maiores (raras) o CoreGraphics reduz primeiro para este lado.
const DECODE_MAX: usize = 2048;

/// O quadrado do meio de `img` (corta as sobras, como "aspect fill"), com lado
/// de até `max` px, no formato do app.
///
/// O quadrado sai do original pixel a pixel (deslocamento inteiro, sem
/// reamostrar) e, se for maior que `max`, é reduzido uma vez só, por média de
/// área: cada pixel novo é a média exata dos pixels que cobre (a mesma redução
/// do desenho, `Image::resampled`). Antes o CoreGraphics reduzia para 384 px e
/// o desenho reduzia de novo. Nunca amplia: uma capa pequena fica no tamanho
/// dela (o desenho amplia na hora, se precisar).
pub fn cover_square(img: &CGImage, max: usize) -> Option<Image> {
    let (iw, ih) = (CGImage::width(Some(img)), CGImage::height(Some(img)));
    if iw == 0 || ih == 0 || max == 0 {
        return None;
    }
    let full = iw.min(ih);
    let side = full.min(DECODE_MAX);
    let k = side as f64 / full as f64;
    let (dw, dh) = (iw as f64 * k, ih as f64 * k);
    let rect = CGRect::new(CGPoint::new(((side as f64 - dw) / 2.0).floor(), ((side as f64 - dh) / 2.0).floor()), CGSize::new(dw, dh));
    let px = draw_px(side, side, |ctx| CGContext::draw_image(Some(ctx), rect, Some(img)))?;
    Some(square_at_most(side, px, max))
}

/// `px` (`side`×`side`) reduzido por média de área para no máximo `max` de
/// lado; com a análise de cor da capa (`Image::new`).
fn square_at_most(side: usize, px: Vec<u32>, max: usize) -> Image {
    if side <= max {
        return Image::new(side, side, px);
    }
    let big = Image::plain(side, side, px);
    let small = big.resampled(crate::render::RectF::new(0.0, 0.0, side as f32, side as f32), max, max);
    // Solta a cópia guardada na grande para ficar com os pixels sem copiar.
    drop(big);
    let px = std::sync::Arc::try_unwrap(small).map(|i| i.px).unwrap_or_else(|a| a.px.clone());
    Image::new(max, max, px)
}

/// Desenha `img` inteiro dentro de `w`×`h` (ícones).
pub fn fit(img: &CGImage, w: usize, h: usize) -> Option<Image> {
    let rect = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as CGFloat, h as CGFloat));
    draw_px(w, h, |ctx| CGContext::draw_image(Some(ctx), rect, Some(img))).map(|px| Image::new(w, h, px))
}

fn draw_px(w: usize, h: usize, draw: impl FnOnce(&CGContext)) -> Option<Vec<u32>> {
    let mut px = vec![0u32; w * h];
    {
        let space = srgb()?;
        let ctx = unsafe { CGBitmapContextCreate(px.as_mut_ptr().cast(), w, h, 8, w * 4, Some(&space), BGRA) }?;
        CGContext::set_interpolation_quality(Some(&ctx), CGInterpolationQuality::High);
        draw(&ctx);
        CGContext::flush(Some(&ctx));
    }
    Some(px)
}

/// Decodifica JPEG ou PNG (capa do álbum): o quadrado do meio, com lado de até `size` px.
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
    // Pede o tamanho em pixels da maior versão guardada (o `size` é em
    // pontos: numa imagem de 144 dpi daria a metade dos pixels).
    let px = image.representations().iter().map(|r| (r.pixelsWide(), r.pixelsHigh())).filter(|&(w, h)| w > 0 && h > 0).max_by_key(|&(w, h)| w * h);
    let sz = px.map(|(w, h)| CGSize::new(w as f64, h as f64)).unwrap_or(image.size());
    let mut rect = NSRect::new(CGPoint::new(0.0, 0.0), sz);
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
    use super::*;

    #[test]
    fn base64() {
        assert_eq!(super::base64_decode("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
        assert_eq!(super::base64_decode("YQ==").unwrap(), b"a");
    }

    fn cg(w: usize, h: usize, f: impl Fn(usize, usize) -> u32) -> CFRetained<CGImage> {
        let px: Vec<u32> = (0..w * h).map(|i| f(i % w, i / w)).collect();
        to_cgimage(&px, w, h).expect("CGImage")
    }

    fn rgb(p: u32) -> [i32; 3] {
        [((p >> 16) & 0xff) as i32, ((p >> 8) & 0xff) as i32, (p & 0xff) as i32]
    }

    /// Capa menor que o limite: o quadrado do meio sai pixel a pixel, sem
    /// ampliar nem reamostrar.
    #[test]
    fn capa_pequena_fica_igual() {
        let f = |x: usize, y: usize| 0xff00_0000 | (((x * 7) & 0xff) << 16) as u32 | (((y * 13) & 0xff) << 8) as u32 | ((x ^ y) & 0xff) as u32;
        let img = cover_square(&cg(120, 90, f), 768).unwrap();
        assert_eq!((img.w, img.h), (90, 90), "não amplia");
        for y in 0..90 {
            for x in 0..90 {
                let (a, b) = (rgb(img.px[y * 90 + x]), rgb(f(x + 15, y)));
                assert!((0..3).all(|k| (a[k] - b[k]).abs() <= 1), "({x},{y}): {a:?} ≠ {b:?}");
            }
        }
    }

    /// Capa grande: corta o meio e reduz por média de área. Um xadrez de 1 px
    /// reduzido à metade vira cinza liso (uma amostragem bilinear serrilharia).
    #[test]
    fn capa_grande_reduz_por_area() {
        let img = cover_square(&cg(1600, 1200, |x, y| if (x + y) % 2 == 0 { 0xffff_ffff } else { 0xff00_0000 }), 600).unwrap();
        assert_eq!((img.w, img.h), (600, 600));
        for (i, &p) in img.px.iter().enumerate() {
            let c = rgb(p);
            assert!(c.iter().all(|&v| (124..=132).contains(&v)), "pixel {i}: {c:?}");
        }
        // Metade vermelha à esquerda, azul à direita: o recorte pega o meio.
        let img = cover_square(&cg(1000, 500, |x, _| if x < 500 { 0xffff_0000 } else { 0xff00_00ff }), 250).unwrap();
        assert_eq!((img.w, img.h), (250, 250));
        let row = &img.px[125 * 250..126 * 250];
        assert_eq!(rgb(row[0]), [255, 0, 0]);
        assert_eq!(rgb(row[249]), [0, 0, 255]);
    }

    /// O caminho de antes: o CoreGraphics reduzia direto para 384 px.
    fn cover_square_384(img: &CGImage) -> Image {
        let (iw, ih) = (CGImage::width(Some(img)) as f64, CGImage::height(Some(img)) as f64);
        let size = 384usize.min(iw.min(ih) as usize).max(64);
        let k = (size as f64 / iw).max(size as f64 / ih);
        let rect = CGRect::new(CGPoint::new((size as f64 - iw * k) / 2.0, (size as f64 - ih * k) / 2.0), CGSize::new(iw * k, ih * k));
        Image::new(size, size, draw_px(size, size, |ctx| CGContext::draw_image(Some(ctx), rect, Some(img))).unwrap())
    }

    /// Capa de antes e de agora no tamanho do card aberto em Retina (72 pt =
    /// 144 px), a partir de uma capa de teste de 1200 px com anéis cada vez
    /// mais finos (placa de zonas): no miolo os anéis cabem nos 144 px (devem
    /// ficar com contraste), na borda não cabem (devem virar cinza liso, sem
    /// moiré). Salva as duas, ampliadas 4×, em `TASKBEAT_SHOTS_DIR`.
    /// `cargo test --release cover_shots -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn cover_shots() {
        use crate::render::{Canvas, RectF};
        const N: usize = 1200;
        const D: usize = 144;
        let k = 0.25 * std::f64::consts::PI / (N as f64 / 2.0);
        let src = cg(N, N, |x, y| {
            let (dx, dy) = (x as f64 + 0.5 - N as f64 / 2.0, y as f64 + 0.5 - N as f64 / 2.0);
            let v = (127.5 + 127.5 * (k * (dx * dx + dy * dy)).cos()).round() as u32;
            0xff00_0000 | (v << 16) | (v << 8) | v
        });
        let dir = std::env::var_os("TASKBEAT_SHOTS_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("taskbeat-shots"));
        for (name, img) in [("antes-384", cover_square_384(&src)), ("agora", cover_square(&src, 768).unwrap())] {
            let t0 = std::time::Instant::now();
            let mut px = vec![0xff00_0000u32; D * D];
            Canvas::new(D, D, &mut px).draw_image_cover(RectF::new(0.0, 0.0, D as f32, D as f32), &img, 0.0, 1.0);
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            // Desvio padrão do cinza em dois anéis do resultado (raio em px de 144).
            let sd = |r0: f64, r1: f64| {
                let v: Vec<f64> = (0..D * D)
                    .filter(|i| {
                        let r = ((*i % D) as f64 + 0.5 - 72.0).hypot((*i / D) as f64 + 0.5 - 72.0);
                        r >= r0 && r < r1
                    })
                    .map(|i| (px[i] & 0xff) as f64)
                    .collect();
                let m = v.iter().sum::<f64>() / v.len() as f64;
                (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
            };
            println!("capa {name} ({}x{} guardada): anéis que cabem (contraste, maior é melhor) {:.1}; anéis finos demais (moiré, menor é melhor) {:.1}; desenho {ms:.1} ms", img.w, img.h, sd(0.0, 10.0), sd(34.0, 70.0));
            let zoom: Vec<u32> = (0..D * 4 * D * 4).map(|i| px[(i / (D * 4)) / 4 * D + (i % (D * 4)) / 4]).collect();
            crate::png::save(&dir.join(format!("cover-{name}.png")), D * 4, D * 4, &zoom);
        }
    }
}
