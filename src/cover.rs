//! Capa em alta resolução baixada da internet. O Windows só recebe 120 px da
//! capa do YouTube Music no navegador; a extensão do Taskbeat manda o
//! endereço da mesma capa em 1200 px (`webext`), e a tela cheia da letra
//! baixa e usa essa.

use crate::render::Image;

/// Servidores de imagem do YouTube e do Google: o endereço vem de uma página,
/// então só esses (e só por HTTPS) são baixados.
const HOSTS: &[&str] = &["googleusercontent.com", "ggpht.com", "ytimg.com"];

/// O endereço é de um servidor de capas aceito?
pub fn allowed(url: &str) -> bool {
    let Ok(u) = crate::http::parse_url(url) else { return false };
    u.secure && u.port == 443 && HOSTS.iter().any(|d| u.host.eq_ignore_ascii_case(d) || u.host.to_ascii_lowercase().ends_with(&format!(".{d}")))
}

/// Baixa a capa e devolve o quadrado do meio, com lado de até `max` px.
pub fn fetch(url: &str, max: u32) -> Option<Image> {
    if !allowed(url) {
        return None;
    }
    let r = crate::http::get(url, &[], 16 << 20).ok().filter(|r| r.ok())?;
    decode_square(&r.body, max)
}

/// Decodifica (WIC) um JPEG/PNG/WebP em memória: o quadrado do meio,
/// reduzido em alta qualidade para no máximo `max` px de lado.
fn decode_square(bytes: &[u8], max: u32) -> Option<Image> {
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    unsafe {
        let f: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let stream = f.CreateStream().ok()?;
        stream.InitializeFromMemory(bytes).ok()?;
        let dec = f.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = dec.GetFrame(0).ok()?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).ok()?;
        if w < 16 || h < 16 || w > 8192 || h > 8192 {
            return None;
        }
        let side = max.min(w.min(h));
        let k = side as f64 / w.min(h) as f64;
        let (sw, sh) = (((w as f64 * k).round() as u32).max(side), ((h as f64 * k).round() as u32).max(side));
        let scaler = f.CreateBitmapScaler().ok()?;
        scaler.Initialize(&frame, sw, sh, WICBitmapInterpolationModeHighQualityCubic).ok()?;
        let conv = f.CreateFormatConverter().ok()?;
        conv.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
        let rect = WICRect { X: ((sw - side) / 2) as i32, Y: ((sh - side) / 2) as i32, Width: side as i32, Height: side as i32 };
        let mut px = vec![0u32; (side * side) as usize];
        let buf = std::slice::from_raw_parts_mut(px.as_mut_ptr().cast::<u8>(), px.len() * 4);
        conv.CopyPixels(&rect, side * 4, buf).ok()?;
        Some(Image::new(side as usize, side as usize, px))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn so_servidores_de_capa() {
        assert!(allowed("https://lh3.googleusercontent.com/abc=w1200-h1200-l90-rj"));
        assert!(allowed("https://yt3.ggpht.com/x=s1200"));
        assert!(allowed("https://i.ytimg.com/vi/abc/maxresdefault.jpg"));
        assert!(!allowed("http://lh3.googleusercontent.com/abc"));
        assert!(!allowed("https://googleusercontent.com.evil.example/abc"));
        assert!(!allowed("https://evilgoogleusercontent.com/abc"));
        assert!(!allowed("https://lh3.googleusercontent.com:8443/abc"));
        assert!(!allowed("file:///C:/x.png"));
        assert!(!allowed(""));
    }

    /// Um PNG em memória vira o quadrado do meio, reduzido.
    #[test]
    fn decodifica_o_quadrado_do_meio() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        // 40×20: metade esquerda vermelha, direita azul.
        let (w, h) = (40usize, 20usize);
        let px: Vec<u32> = (0..w * h).map(|i| if i % w < w / 2 { 0xffff_0000 } else { 0xff00_00ff }).collect();
        let path = std::env::temp_dir().join(format!("taskbeat-capa-teste-{}.png", std::process::id()));
        crate::png::save(&path, w, h, &px);
        let png = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let img = decode_square(&png, 10).expect("decodifica");
        assert_eq!((img.w, img.h), (10, 10));
        // O quadrado do meio pega as duas metades.
        assert_eq!(img.px[10 * 5] & 0x00ff_ffff, 0x00ff_0000);
        assert_eq!(img.px[10 * 5 + 9] & 0x00ff_ffff, 0x0000_00ff);
    }
}
