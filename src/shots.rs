//! Imagens do README, desenhadas pelo próprio código do app com a música de
//! exemplo (nada da máquina de quem gera aparece). Gera em `docs/screenshots`:
//!
//! ```powershell
//! $env:TASKBEAT_LANG="pt"; cargo test --release shots -- --ignored
//! ```

use crate::config::*;
use crate::render::*;
use crate::shared::MediaInfo;
use crate::text::*;
use crate::ui::{self, CLayout, CardFrame, Interact, Playback, WLayout, WidgetFrame};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::GENERIC_WRITE;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::StructuredStorage::IPropertyBag2;
use windows::Win32::System::Com::*;
use windows::core::HSTRING;

/// Escala das imagens (2x = nítidas em telas de alta densidade).
const S: f32 = 2.0;

fn out_dir() -> PathBuf {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs").join("screenshots");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn colors(cfg: &Config, media: &MediaInfo) -> Colors {
    Colors { palette: cfg.palette, accent: [0.93, 0.32, 0.62], accent_light: [0.98, 0.62, 0.80], light_taskbar: false, album: media.art.as_ref().map(|a| a.vibrant) }
}

fn playback(media: &MediaInfo) -> Playback {
    Playback { playing: true, shuffle: media.shuffle, repeat: media.repeat, pos: media.pos, seek_preview: None, volume: 0.62, muted: false }
}

/// Grava o buffer pré-multiplicado como PNG (com transparência) via WIC.
fn save_png(name: &str, w: usize, h: usize, px: &[u32]) {
    let mut buf = Vec::with_capacity(w * h * 4);
    for &p in px {
        let a = (p >> 24) & 0xff;
        let un = |c: u32| if a == 0 { 0 } else { ((c * 255 + a / 2) / a).min(255) as u8 };
        buf.extend_from_slice(&[un(p & 0xff), un((p >> 8) & 0xff), un((p >> 16) & 0xff), a as u8]);
    }
    let path = out_dir().join(name);
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let f: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
        let stream = f.CreateStream().unwrap();
        stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE.0).unwrap();
        let enc = f.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()).unwrap();
        enc.Initialize(&stream, WICBitmapEncoderNoCache).unwrap();
        let mut frame = None;
        enc.CreateNewFrame(&mut frame, std::ptr::null_mut()).unwrap();
        let frame = frame.unwrap();
        frame.Initialize(None::<&IPropertyBag2>).unwrap();
        frame.SetSize(w as u32, h as u32).unwrap();
        let mut fmt = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut fmt).unwrap();
        frame.WritePixels(h as u32, (w * 4) as u32, &buf).unwrap();
        frame.Commit().unwrap();
        enc.Commit().unwrap();
    }
    println!("{} ({w}x{h})", path.display());
}

/// Desenha num buffer transparente `w`×`h` (pixels).
fn render(w: f32, h: f32, f: impl FnOnce(&mut Canvas)) -> (usize, usize, Vec<u32>) {
    let (w, h) = (w.ceil() as usize, h.ceil() as usize);
    let mut px = vec![0u32; w * h];
    f(&mut Canvas::new(w, h, &mut px));
    (w, h, px)
}

fn blit(cv: &mut Canvas, x: f32, y: f32, layer: (usize, usize, Vec<u32>)) {
    let (w, h, px) = layer;
    cv.draw_image(RectF::new(x.round(), y.round(), w as f32, h as f32), &Image::plain(w, h, px), 0.0, 1.0);
}

/// "Papel de parede" escuro, como nas prévias da tela de configurações.
fn wallpaper(cv: &mut Canvas, r: RectF, radius: f32) {
    let (x0, y0) = (r.x, r.y);
    cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), radius, |x, y| {
        let u = ((x - x0) / r.w).clamp(0.0, 1.0);
        let v = ((y - y0) / r.h).clamp(0.0, 1.0);
        let c = lerp3([0.05, 0.09, 0.26], [0.22, 0.12, 0.42], (u * 0.7 + v * 0.3).clamp(0.0, 1.0));
        let glow = (-(((u - 0.30) * 2.0).powi(2) + ((v - 0.30) * 2.4).powi(2))).exp();
        (lerp3(c, [0.30, 0.45, 0.95], glow * 0.5), 1.0)
    });
}

#[test]
#[ignore]
fn shots() {
    let cfg = Config::default();
    let media = ui::sample_media();
    let pb = playback(&media);
    let mut tr = TextRenderer::new();
    tr.set_scale(S);

    // ---- 1. Barra de tarefas + card aberto
    {
        let (iw, ih) = (820.0 * S, 400.0 * S);
        let c = colors(&cfg, &media);
        let cl = CLayout::build(S, &cfg, true);
        let wl = WLayout::build(S, 40.0 * S, &cfg, true, false);
        let card = render(cl.w, cl.h, |cv| {
            let f = CardFrame { cfg: &cfg, colors: &c, media: &media, theme: ui::card_theme(&cfg, &c, &media), it: Interact::default(), pb, s: S };
            ui::draw_card(cv, &mut tr, &cl, &f);
        });
        let widget = render(wl.width, 40.0 * S, |cv| {
            let f = WidgetFrame { cfg: &cfg, media: &media, theme: ui::widget_theme(&cfg, &c, &media), colors: &c, it: Interact::default(), pb, time: 1.0, viz: None, flyout_open: true, s: S };
            ui::draw_widget(cv, &mut tr, &wl, &f);
        });
        let img = render(iw, ih, |cv| {
            let r = RectF::new(0.0, 0.0, iw, ih);
            wallpaper(cv, r, 12.0 * S);
            // Barra de tarefas.
            let bar = RectF::new(0.0, ih - 48.0 * S, iw, 48.0 * S);
            let bc: Rgb = [0.11, 0.11, 0.12];
            let rad = 12.0 * S;
            cv.fill_rrect_ca(bar.x, bar.y, bar.right(), bar.bottom(), rad, |_, y| (bc, if y < bar.y + rad { 0.0 } else { 0.96 }));
            cv.fill_rrect(bar.x, bar.y, bar.right(), bar.y + rad, 0.0, 0.96, |_, _| bc);
            cv.fill_rrect(bar.x, bar.y, bar.right(), bar.y + S, 0.0, 0.10, |_, _| [1.0; 3]);
            // Ícones fixados (genéricos) no centro.
            let n = 6;
            let x0 = iw * 0.30 - (n as f32 * 40.0 * S) / 2.0;
            for k in 0..n {
                let ix = x0 + k as f32 * 40.0 * S + 9.0 * S;
                cv.fill_round(RectF::new(ix, bar.cy() - 11.0 * S, 22.0 * S, 22.0 * S), 5.0 * S, [0.85; 3], if k == 0 { 0.9 } else { 0.30 });
            }
            // Área de notificação: ícones + relógio.
            let clock_x = bar.right() - 64.0 * S;
            ui::text(cv, &mut tr, "12:34", Font::Caption, clock_x, bar.y + 6.0 * S, 56.0 * S, 18.0 * S, [0.92; 3], 1.0);
            ui::text(cv, &mut tr, "23/11/2026", Font::Caption, clock_x - 8.0 * S, bar.y + 24.0 * S, 70.0 * S, 18.0 * S, [0.92; 3], 0.8);
            for (k, g) in ["\u{E995}", "\u{E701}", "\u{E70E}"].iter().enumerate() {
                ui::glyph(cv, &mut tr, RectF::new(clock_x - (k as f32 + 1.0) * 26.0 * S - 12.0 * S, bar.y, 24.0 * S, bar.h), g, Font::IconSmall, [0.9; 3], 0.9);
            }
            let wx = clock_x - 104.0 * S - wl.width;
            blit(cv, wx, bar.y + 4.0 * S, widget);
            // Card acima da música, como no app.
            let m = ui::SHADOW_MARGIN * S;
            let cx = (wx + 2.0 * S - m).min(iw - cl.w + m - 16.0 * S);
            blit(cv, cx, bar.y - cl.h + m - 10.0 * S, card);
        });
        save_png("taskbar-card.png", img.0, img.1, &img.2);
    }

    // ---- 2. Estilos do card
    for (name, style, theme) in [("card-artwork.png", CardStyle::Artwork, ThemeMode::Auto), ("card-album-color.png", CardStyle::AlbumColor, ThemeMode::Auto), ("card-glass.png", CardStyle::Glass, ThemeMode::Dark)] {
        let cfg = Config { card_style: style, theme, ..Config::default() };
        let c = colors(&cfg, &media);
        let l = CLayout::build(S, &cfg, true);
        let img = render(l.w, l.h, |cv| {
            let f = CardFrame { cfg: &cfg, colors: &c, media: &media, theme: ui::card_theme(&cfg, &c, &media), it: Interact::default(), pb, s: S };
            ui::draw_card(cv, &mut tr, &l, &f);
        });
        save_png(name, img.0, img.1, &img.2);
    }

    // ---- 3. "A seguir"
    {
        let c = colors(&cfg, &media);
        let (w, h) = ui::upnext_size(S);
        let img = render(w, h, |cv| {
            let th = ui::card_theme(&cfg, &c, &media);
            ui::draw_upnext(cv, &mut tr, &cfg, &c, &media, &th, S, 2.5, cfg.popup_secs as f32 + 0.5);
        });
        save_png("up-next.png", img.0, img.1, &img.2);
    }

    // ---- 4. Aviso de Caps Lock
    {
        let c = colors(&cfg, &MediaInfo::default());
        let (w, h) = ui::osd_size(S, &cfg);
        let img = render(w, h, |cv| {
            let th = ui::osd_theme(&cfg, &c);
            ui::draw_osd(cv, &mut tr, &cfg, &th, S, crate::i18n::tr("Caps Lock"), "A", true, 1.0);
        });
        save_png("caps-lock.png", img.0, img.1, &img.2);
    }
}
