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

/// Barras de áudio de exemplo (mini visualizador do card).
const VIZ: &[f32] = &[0.55, 0.9, 0.62, 0.38, 0.74, 0.5];

/// `docs/screenshots` (ou a pasta em `TASKBEAT_SHOTS_DIR`, para conferir sem mexer no README).
fn out_dir() -> PathBuf {
    let d = std::env::var_os("TASKBEAT_SHOTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("docs").join("screenshots"));
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

/// Papel de parede com "janelas" coloridas, para o acrílico ter o que desfocar.
fn busy_wallpaper(cv: &mut Canvas, r: RectF, radius: f32) {
    wallpaper(cv, r, radius);
    let shapes: [(f32, f32, f32, f32, Rgb); 5] = [
        (0.06, 0.20, 0.22, 0.62, [0.98, 0.45, 0.55]),
        (0.36, 0.06, 0.16, 0.42, [1.0, 0.78, 0.35]),
        (0.55, 0.48, 0.20, 0.44, [0.30, 0.80, 0.95]),
        (0.78, 0.10, 0.16, 0.50, [0.62, 0.45, 1.0]),
        (0.26, 0.58, 0.18, 0.34, [0.40, 0.90, 0.55]),
    ];
    for (u, v, w, h, c) in shapes {
        cv.fill_round(RectF::new(r.x + r.w * u, r.y + r.h * v, r.w * w, r.h * h), 12.0 * S, c, 0.7);
    }
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
            let f = CardFrame { cfg: &cfg, colors: &c, media: &media, theme: ui::card_theme(&cfg, &c, &media), it: Interact::default(), pb, s: S, backdrop: None, hint: None, launchers: &[], viz: Some(VIZ) };
            ui::draw_card(cv, &mut tr, &cl, &f);
        });
        let widget = render(wl.width, 40.0 * S, |cv| {
            let f = WidgetFrame { cfg: &cfg, media: &media, theme: ui::widget_theme(&cfg, &c, &media), colors: &c, it: Interact::default(), pb, time: 1.0, viz: None, flyout_open: true, s: S, change: None };
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
            cv.fill_rrect(bar.x, bar.y, bar.right(), bar.y + rad, 0.0, 0.96, bc);
            cv.fill_rrect(bar.x, bar.y, bar.right(), bar.y + S, 0.0, 0.10, [1.0; 3]);
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
    for (name, style, theme) in [("card-artwork.png", CardStyle::Artwork, ThemeMode::Auto), ("card-album-color.png", CardStyle::AlbumColor, ThemeMode::Auto)] {
        let cfg = Config { card_style: style, theme, ..Config::default() };
        let c = colors(&cfg, &media);
        let l = CLayout::build(S, &cfg, true);
        let img = render(l.w, l.h, |cv| {
            let f = CardFrame { cfg: &cfg, colors: &c, media: &media, theme: ui::card_theme(&cfg, &c, &media), it: Interact::default(), pb, s: S, backdrop: None, hint: None, launchers: &[], viz: Some(VIZ) };
            ui::draw_card(cv, &mut tr, &l, &f);
        });
        save_png(name, img.0, img.1, &img.2);
    }
    // Acrílico: mostrado sobre um papel de parede, para dar para ver o desfoque.
    {
        let cfg = Config { card_style: CardStyle::Glass, theme: ThemeMode::Dark, ..Config::default() };
        let c = colors(&cfg, &media);
        let l = CLayout::build(S, &cfg, true);
        let (w, h) = (l.w, l.h);
        let bg = render(w, h, |cv| busy_wallpaper(cv, RectF::new(0.0, 0.0, w, h), 14.0 * S));
        let backdrop = acrylic_backdrop(&bg.2, bg.0, bg.1, l.card, ui::GLASS_BLUR * S);
        let card = render(w, h, |cv| {
            let f = CardFrame { cfg: &cfg, colors: &c, media: &media, theme: ui::card_theme(&cfg, &c, &media), it: Interact::default(), pb, s: S, backdrop: Some(&backdrop), hint: None, launchers: &[], viz: Some(VIZ) };
            ui::draw_card(cv, &mut tr, &l, &f);
        });
        let img = render(w, h, |cv| {
            blit(cv, 0.0, 0.0, bg);
            blit(cv, 0.0, 0.0, card);
        });
        save_png("card-glass.png", img.0, img.1, &img.2);
    }

    // ---- 2b. Widget da barra: tudo dentro do mesmo fundo, em algumas combinações
    {
        let variants = [
            // Só a música; o padrão (com os botões); pílula com aleatório e
            // repetir; reto, neutro, com o play só no ícone e os ícones de contorno.
            Config { taskbar_controls: false, playing_indicator: false, ..Config::default() },
            Config::default(),
            Config { taskbar_shuffle_repeat: true, widget_divider: true, widget_shape: WidgetShape::Pill, ..Config::default() },
            Config { taskbar_play: TaskbarPlay::Icon, widget_shape: WidgetShape::Square, widget_tint: WidgetTint::Neutral, icon_style: IconStyle::Fluent, ..Config::default() },
            // Fundos fortes: gradiente da capa e capa desfocada.
            Config { widget_tint: WidgetTint::Gradient, ..Config::default() },
            Config { widget_tint: WidgetTint::Artwork, widget_shape: WidgetShape::Pill, taskbar_shuffle_repeat: true, ..Config::default() },
        ];
        let bar_h = 48.0 * S;
        let wh = 40.0 * S;
        let mut shuffled = media.clone();
        shuffled.repeat = 2;
        let pb = Playback { repeat: 2, ..playback(&shuffled) };
        let layers: Vec<_> = variants
            .iter()
            .map(|cfg| {
                let c = colors(cfg, &shuffled);
                let l = WLayout::build(S, wh, cfg, true, false);
                render(l.width, wh, |cv| {
                    let f = WidgetFrame { cfg, media: &shuffled, theme: ui::widget_theme(cfg, &c, &shuffled), colors: &c, it: Interact::default(), pb, time: 1.0, viz: None, flyout_open: false, s: S, change: None };
                    ui::draw_widget(cv, &mut tr, &l, &f);
                })
            })
            .collect();
        let iw = layers.iter().map(|l| l.0).max().unwrap() as f32 + 48.0 * S;
        let ih = layers.len() as f32 * (bar_h + 10.0 * S) + 10.0 * S;
        let img = render(iw, ih, |cv| {
            for (k, layer) in layers.into_iter().enumerate() {
                let y = 10.0 * S + k as f32 * (bar_h + 10.0 * S);
                let bar = RectF::new(0.0, y, iw, bar_h);
                cv.fill_round(bar, 10.0 * S, [0.11, 0.11, 0.12], 0.97);
                cv.fill_rrect(bar.x + 10.0 * S, bar.y, bar.right() - 10.0 * S, bar.y + S, 0.0, 0.10, [1.0; 3]);
                blit(cv, iw - 24.0 * S - layer.0 as f32, y + 4.0 * S, layer);
            }
        });
        save_png("taskbar-widgets.png", img.0, img.1, &img.2);
    }

    // ---- 3. "A seguir"
    {
        let c = colors(&cfg, &media);
        let (w, h) = ui::upnext_size(S);
        let img = render(w, h, |cv| {
            let th = ui::card_theme(&cfg, &c, &media);
            ui::draw_upnext(cv, &mut tr, &cfg, &c, &media, &th, S, 2.5, cfg.popup_secs as f32 + 0.5, None);
        });
        save_png("up-next.png", img.0, img.1, &img.2);
    }

    // ---- 4. Aviso de Caps Lock
    {
        let c = colors(&cfg, &MediaInfo::default());
        let (w, h) = ui::osd_size(S, &cfg);
        let img = render(w, h, |cv| {
            let th = ui::osd_theme(&cfg, &c);
            ui::draw_osd(cv, &mut tr, &cfg, &c, &th, S, crate::i18n::tr("Caps Lock"), false, true, 1.0, None);
        });
        save_png("caps-lock.png", img.0, img.1, &img.2);
    }

    // ---- 5. Aviso de teclas: sólido, acrílico e cor de destaque, sobre um papel de parede
    {
        let variants = [
            (OsdMaterial::Solid, OsdIcon::Letter, false, "Caps Lock"),
            (OsdMaterial::Acrylic, OsdIcon::Lock, false, "Caps Lock"),
            (OsdMaterial::Accent, OsdIcon::Keyboard, true, "Num Lock"),
        ];
        let base = Config { osd_theme: ThemeMode::Dark, ..Config::default() };
        let (ow, oh) = ui::osd_size(S, &base);
        let m = ui::SHADOW_MARGIN * S;
        let (gap, cw) = (24.0 * S, ow - 2.0 * m);
        let (iw, ih) = (3.0 * cw + 4.0 * gap, oh - 2.0 * m + 2.0 * gap);
        let bg = render(iw, ih, |cv| busy_wallpaper(cv, RectF::new(0.0, 0.0, iw, ih), 14.0 * S));
        let mut layers = Vec::new();
        for (k, (material, icon, num, key)) in variants.into_iter().enumerate() {
            let cfg = Config { osd_material: material, osd_icon: icon, ..base.clone() };
            let c = colors(&cfg, &MediaInfo::default());
            let x = gap - m + k as f32 * (cw + gap);
            let y = gap - m;
            let (cw, ch) = ui::osd_card(S, &cfg);
            let backdrop = acrylic_backdrop(&bg.2, bg.0, bg.1, RectF::new(x + m, y + m, cw, ch), ui::osd_blur_px(&cfg, S));
            let layer = render(ow, oh, |cv| {
                let th = ui::osd_theme(&cfg, &c);
                ui::draw_osd(cv, &mut tr, &cfg, &c, &th, S, crate::i18n::tr(key), num, true, 1.0, Some(&backdrop));
            });
            layers.push((x, y, layer));
        }
        let img = render(iw, ih, |cv| {
            blit(cv, 0.0, 0.0, bg);
            for (x, y, l) in layers {
                blit(cv, x, y, l);
            }
        });
        save_png("caps-lock-styles.png", img.0, img.1, &img.2);
    }
}

/// Quadros das animações dos ícones do card (folha de contato para conferir
/// o movimento): uma linha por botão, colunas = instantes da animação.
/// `$env:TASKBEAT_SHOTS_DIR="..."; cargo test --release icon_frames -- --ignored`
#[test]
#[ignore]
fn icon_frames() {
    use crate::ui::{Hit, IconFx};
    let cfg = Config::default();
    let media = ui::sample_media();
    let mut tr = TextRenderer::new();
    tr.set_scale(S);
    let c = colors(&cfg, &media);
    let cl = CLayout::build(S, &cfg, true);
    // Só a faixa dos botões do card.
    let (bx, by, bw, bh) = (cl.shuffle.x - 8.0 * S, cl.shuffle.y - 6.0 * S, cl.repeat.right() - cl.shuffle.x + 16.0 * S, cl.shuffle.h + 12.0 * S);
    let cols = 7;
    let rows: [(Hit, &str, bool); 5] = [(Hit::Play, "play", false), (Hit::Prev, "prev", false), (Hit::Next, "next", false), (Hit::Shuffle, "shuffle", true), (Hit::Repeat, "repeat", true)];
    let (iw, ih) = (bw * cols as f32, bh * rows.len() as f32);
    let sw = iw.ceil() as usize;
    let mut sheet = vec![0u32; sw * ih.ceil() as usize];
    {
        for (ri, (h, _, toggled)) in rows.iter().enumerate() {
            for ci in 0..cols {
                let age = IconFx::duration(*h) * ci as f32 / (cols - 1) as f32;
                let mut pb = playback(&media);
                if *toggled {
                    pb.shuffle = true;
                    pb.repeat = 2;
                }
                let card = render(cl.w, cl.h, |cv| {
                    let it = Interact { fx: IconFx::single(*h, age), ..Interact::default() };
                    let f = CardFrame { cfg: &cfg, colors: &c, media: &media, theme: ui::card_theme(&cfg, &c, &media), it, pb, s: S, backdrop: None, hint: None, launchers: &[], viz: Some(VIZ) };
                    ui::draw_card(cv, &mut tr, &cl, &f);
                });
                let (cw, _, px) = card;
                let (x0, y0) = (ci as f32 * bw, ri as f32 * bh);
                for y in 0..bh as usize {
                    for x in 0..bw as usize {
                        let (sx, sy) = (bx as usize + x, by as usize + y);
                        sheet[(y0 as usize + y) * sw + x0 as usize + x] = px[sy * cw + sx];
                    }
                }
            }
        }
    }
    save_png("icon-frames.png", iw.ceil() as usize, ih.ceil() as usize, &sheet);
}

/// Capa de exemplo de uma cor (gradiente diagonal com um "sol"), para conferir
/// os fundos do widget com capas bem diferentes.
fn tinted_art(a: Rgb, b: Rgb) -> std::sync::Arc<Image> {
    const N: usize = 160;
    let mut px = vec![0u32; N * N];
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 / N as f32, y as f32 / N as f32);
            let mut c = lerp3(a, b, (fx * 0.4 + fy * 0.6).clamp(0.0, 1.0));
            let d = ((fx - 0.62).powi(2) + (fy - 0.4).powi(2)).sqrt();
            c = lerp3(c, [1.0, 0.95, 0.85], ((0.16 - d) / 0.02).clamp(0.0, 1.0));
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u32;
            px[y * N + x] = 0xff00_0000 | (q(c[0]) << 16) | (q(c[1]) << 8) | q(c[2]);
        }
    }
    std::sync::Arc::new(Image::new(N, N, px))
}

/// Fundos do widget (neutro, cor da capa, gradiente, capa desfocada) com capas
/// de cores diferentes, na barra escura e na clara.
/// `$env:TASKBEAT_SHOTS_DIR="..."; cargo test --release widget_tints -- --ignored`
#[test]
#[ignore]
fn widget_tints() {
    let mut tr = TextRenderer::new();
    tr.set_scale(S);
    let arts = [
        ui::sample_media().art.unwrap(),
        tinted_art([0.05, 0.45, 0.50], [0.10, 0.80, 0.70]),
        tinted_art([0.95, 0.80, 0.20], [0.98, 0.55, 0.10]),
        tinted_art([0.80, 0.10, 0.15], [0.35, 0.02, 0.10]),
        tinted_art([0.85, 0.85, 0.86], [0.55, 0.55, 0.58]),
    ];
    let tints = [(WidgetTint::Album, 6), (WidgetTint::Neutral, 60), (WidgetTint::Gradient, 80), (WidgetTint::Artwork, 80), (WidgetTint::Gradient, 100)];
    let wh = 40.0 * S;
    let bar_h = 48.0 * S;
    let mut cols = Vec::new();
    for light in [false, true] {
        for (tint, k) in tints {
            let mut col = Vec::new();
            for art in &arts {
                let mut media = ui::sample_media();
                media.art = Some(art.clone());
                let cfg = Config { widget_tint: tint, widget_opacity: k.min(100), widget_intensity: k, ..Config::default() };
                let mut c = colors(&cfg, &media);
                c.light_taskbar = light;
                let l = WLayout::build(S, wh, &cfg, true, false);
                col.push(render(l.width, wh, |cv| {
                    let f = WidgetFrame { cfg: &cfg, media: &media, theme: ui::widget_theme(&cfg, &c, &media), colors: &c, it: Interact::default(), pb: playback(&media), time: 1.0, viz: None, flyout_open: false, s: S, change: None };
                    ui::draw_widget(cv, &mut tr, &l, &f);
                }));
            }
            cols.push((light, col));
        }
    }
    let cw = cols.iter().flat_map(|(_, c)| c.iter().map(|l| l.0)).max().unwrap() as f32 + 16.0 * S;
    let rows = arts.len();
    let (iw, ih) = (cw * tints.len() as f32, 2.0 * rows as f32 * bar_h);
    let img = render(iw, ih, |cv| {
        for (ci, (light, col)) in cols.into_iter().enumerate() {
            let x = (ci % tints.len()) as f32 * cw;
            let y0 = if light { rows as f32 * bar_h } else { 0.0 };
            let bc: Rgb = if light { [0.93, 0.93, 0.94] } else { [0.11, 0.11, 0.12] };
            cv.fill_rrect(x, y0, x + cw, y0 + rows as f32 * bar_h, 0.0, 1.0, bc);
            for (ri, layer) in col.into_iter().enumerate() {
                blit(cv, x + 8.0 * S, y0 + ri as f32 * bar_h + 4.0 * S, layer);
            }
        }
    });
    save_png("widget-tints.png", img.0, img.1, &img.2);
}

/// Widget em pílula com cada formato e tamanho de capa (conferir que a capa
/// fica dentro da ponta arredondada).
#[test]
#[ignore]
fn pill_covers() {
    let media = ui::sample_media();
    let pb = playback(&media);
    let mut tr = TextRenderer::new();
    tr.set_scale(S);
    let h = 40.0 * S;
    let mut rows = Vec::new();
    for shape in [Shape::Rounded, Shape::Square, Shape::Circle] {
        for size in [Size3::Normal, Size3::Large] {
            let cfg = Config { widget_shape: WidgetShape::Pill, widget_pill: true, cover_shape: shape, cover_size: size, widget_opacity: 30, ..Config::default() };
            let c = colors(&cfg, &media);
            let wl = WLayout::build(S, h, &cfg, true, false);
            rows.push(render(wl.width, h, |cv| {
                let f = WidgetFrame { cfg: &cfg, media: &media, theme: ui::widget_theme(&cfg, &c, &media), colors: &c, it: Interact::default(), pb, time: 1.0, viz: None, flyout_open: false, s: S, change: None };
                ui::draw_widget(cv, &mut tr, &wl, &f);
            }));
        }
    }
    let w = rows.iter().map(|r| r.0).max().unwrap() as f32;
    let img = render(w, h * rows.len() as f32, |cv| {
        cv.fill_rrect(0.0, 0.0, w, h * rows.len() as f32, 0.0, 1.0, [0.12, 0.10, 0.12]);
        for (i, r) in rows.into_iter().enumerate() {
            blit(cv, 0.0, i as f32 * h, r);
        }
    });
    save_png("pill-covers.png", img.0, img.1, &img.2);
}

/// Card com a linha de atalhos dos apps de música instalados nesta máquina
/// (o segundo com o mouse sobre o primeiro atalho).
#[test]
#[ignore]
fn card_launchers() {
    use crate::ui::Hit;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let launchers = crate::launchers::scan();
    let media = ui::sample_media();
    let pb = playback(&media);
    let mut tr = TextRenderer::new();
    tr.set_scale(S);
    for (name, layout) in [("card-launchers.png", CardLayout::Horizontal), ("card-launchers-vertical.png", CardLayout::Vertical)] {
        let cfg = Config { card_layout: layout, ..Config::default() };
        let c = colors(&cfg, &media);
        let cl = CLayout::build(S, &cfg, true).with_launchers(launchers.len(), S);
        let mut cards = Vec::new();
        for hover in [None, Some(Hit::Launch(0))] {
            cards.push(render(cl.w, cl.h, |cv| {
                let it = Interact { hover, ..Interact::default() };
                let f = CardFrame { cfg: &cfg, colors: &c, media: &media, theme: ui::card_theme(&cfg, &c, &media), it, pb, s: S, backdrop: None, hint: None, launchers: &launchers, viz: Some(VIZ) };
                ui::draw_card(cv, &mut tr, &cl, &f);
            }));
        }
        let img = render(cl.w * 2.0, cl.h, |cv| {
            cv.fill_rrect(0.0, 0.0, cl.w * 2.0, cl.h, 0.0, 1.0, [0.16, 0.18, 0.24]);
            for (i, card) in cards.into_iter().enumerate() {
                blit(cv, i as f32 * cl.w, 0.0, card);
            }
        });
        save_png(name, img.0, img.1, &img.2);
    }
}
