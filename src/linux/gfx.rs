//! Imagens no Linux: capa do álbum (JPEG, PNG ou WebP, de um arquivo, de uma
//! URL ou embutida em `data:`) → `Image` do app (BGRA pré-multiplicado).
//! Decodificadores em Rust (zune, image-webp); a URL https (Spotify) é baixada
//! com o `curl` do sistema, se houver.

use crate::render::{Image, RectF};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Maior lado aceito ao decodificar (protege a memória de imagens enormes).
const MAX_SIDE: usize = 8192;
/// Maior arquivo de capa aceito.
const MAX_BYTES: u64 = 16 << 20;

/// RGBA 8 bits (não pré-multiplicado) → `Image` do app.
fn from_rgba(w: usize, h: usize, rgba: &[u8], channels: usize) -> Option<Image> {
    if w == 0 || h == 0 || rgba.len() < w * h * channels {
        return None;
    }
    let mut px = Vec::with_capacity(w * h);
    for p in rgba.chunks_exact(channels).take(w * h) {
        let (r, g, b, a) = match channels {
            1 => (p[0], p[0], p[0], 255),
            2 => (p[0], p[0], p[0], p[1]),
            3 => (p[0], p[1], p[2], 255),
            _ => (p[0], p[1], p[2], p[3]),
        };
        let pm = |c: u8| (c as u32 * a as u32 + 127) / 255;
        px.push(((a as u32) << 24) | (pm(r) << 16) | (pm(g) << 8) | pm(b));
    }
    Some(Image::plain(w, h, px))
}

/// Decodifica JPEG, PNG ou WebP.
pub fn decode(bytes: &[u8]) -> Option<Image> {
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let opts = DecoderOptions::default().set_max_width(MAX_SIDE).set_max_height(MAX_SIDE);
    if bytes.starts_with(b"\x89PNG") {
        let mut d = zune_png::PngDecoder::new_with_options(bytes, opts.png_set_strip_to_8bit(true).png_set_add_alpha_channel(true));
        d.decode_headers().ok()?;
        let (w, h) = d.get_dimensions()?;
        let px = d.decode().ok()?.u8()?;
        let ch = px.len() / (w * h).max(1);
        return from_rgba(w, h, &px, ch);
    }
    if bytes.starts_with(b"\xFF\xD8") {
        let mut d = zune_jpeg::JpegDecoder::new_with_options(bytes, opts.jpeg_set_out_colorspace(ColorSpace::RGB));
        d.decode_headers().ok()?;
        let px = d.decode().ok()?;
        let (w, h) = d.dimensions()?;
        return from_rgba(w, h, &px, 3);
    }
    if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let mut d = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).ok()?;
        let (w, h) = d.dimensions();
        let (w, h) = (w as usize, h as usize);
        if w > MAX_SIDE || h > MAX_SIDE {
            return None;
        }
        let ch = if d.has_alpha() { 4 } else { 3 };
        let mut px = vec![0u8; d.output_buffer_size()?];
        d.read_image(&mut px).ok()?;
        return from_rgba(w, h, &px, ch);
    }
    None
}

/// Capa quadrada `size`×`size` (recorta as sobras, "aspect fill"; nunca
/// estica além do original), com a análise de cor do app.
pub fn decode_square(bytes: &[u8], size: usize) -> Option<Image> {
    let img = decode(bytes)?;
    let side = img.w.min(img.h);
    let size = size.min(side).max(1);
    let src = RectF::new(((img.w - side) / 2) as f32, ((img.h - side) / 2) as f32, side as f32, side as f32);
    let sq = img.resampled(src, size, size);
    Some(Image::new(size, size, sq.px.clone()))
}

/// Base64 padrão ou de URL.
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

/// `%20` → espaço etc. (caminhos em URLs `file://`).
fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Bytes da capa a partir da URL que o player publica (`mpris:artUrl`).
/// Pode demorar (download): chamar fora das threads que não podem esperar.
pub fn fetch(url: &str) -> Option<Vec<u8>> {
    let url = url.trim();
    if let Some(rest) = url.strip_prefix("file://") {
        // file:///caminho ou file://localhost/caminho
        let path = rest.strip_prefix("localhost").unwrap_or(rest);
        use std::os::unix::ffi::OsStrExt;
        let p = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(&percent_decode(path)));
        let f = std::fs::File::open(p).ok()?;
        let mut v = Vec::new();
        f.take(MAX_BYTES).read_to_end(&mut v).ok()?;
        return Some(v);
    }
    if url.starts_with('/') {
        return std::fs::read(url).ok();
    }
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        return if meta.ends_with(";base64") { base64_decode(data) } else { Some(percent_decode(data)) };
    }
    if url.starts_with("https://") || url.starts_with("http://") {
        return curl(url);
    }
    None
}

fn curl(url: &str) -> Option<Vec<u8>> {
    let mut child = Command::new("curl")
        .args(["-sfL", "--max-time", "8", "--max-filesize", &MAX_BYTES.to_string(), "--proto", "=https,http", "--"])
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = Vec::new();
    child.stdout.take()?.take(MAX_BYTES).read_to_end(&mut out).ok()?;
    // Espera o curl terminar (com limite, para não ficar preso).
    let t = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return (st.success() && !out.is_empty()).then_some(out),
            Ok(None) if t.elapsed() < Duration::from_secs(10) => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64() {
        assert_eq!(base64_decode("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
    }

    #[test]
    fn percent() {
        assert_eq!(percent_decode("/tmp/a%20b.png"), b"/tmp/a b.png");
        assert_eq!(percent_decode("100%"), b"100%");
    }

    /// PNG do ícone do app: decodifica e corta em quadrado.
    #[test]
    fn decodes_png() {
        let bytes = include_bytes!("../../assets/taskbeat-256.png");
        let img = decode_square(bytes, 64).expect("png");
        assert_eq!((img.w, img.h), (64, 64));
        assert!(img.px.iter().any(|&p| p >> 24 == 255));
    }
}
