//! Tela de configurações do Mac: janela nativa, no estilo dos Ajustes do
//! Sistema — páginas na barra de ferramentas, opções em grupos com
//! interruptores, controles segmentados, menus e sliders do próprio AppKit
//! (acessíveis e com navegação por teclado), e prévia ao vivo da ilha no
//! topo de cada página. Mudanças valem na hora; sliders gravam ao soltar.

use super::{gfx, shared, system, target};
use crate::config::*;
use crate::i18n::tr;
use crate::notch::{self as nd, Geom};
use crate::text::TextRenderer;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBox, NSBoxType, NSButton, NSColor, NSControl, NSControlStateValueOff, NSControlStateValueOn, NSEventType, NSFont, NSImage, NSImageScaling, NSImageView, NSLineBreakMode,
    NSPopUpButton, NSResponder, NSScrollView, NSSegmentSwitchTracking, NSSegmentedControl, NSSlider, NSSwitch, NSTabViewController, NSTabViewControllerTabStyle, NSTabViewItem, NSTextField,
    NSTitlePosition, NSView, NSViewController, NSWindow,
};
use objc2_core_foundation::{CGPoint, CGSize};
use objc2_foundation::{NSArray, NSObject, NSRect, NSString, NSURL};
use std::cell::RefCell;

/// Largura e altura de cada página (pontos).
const W: f64 = 620.0;
const H: f64 = 640.0;
const PREVIEW_H: f64 = 210.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Key {
    ShowMedia,
    AlwaysShow,
    NotchHover,
    Indicator,
    TrackAnim,
    NotchBg,
    Accent,
    PlayShape,
    CardSeek,
    ShowVolume,
    CardShuffle,
    Popup,
    PopupSecs,
    ShowViz,
    Style,
    Palette,
    Bars,
    Sens,
    Peaks,
    OnlyMedia,
    Autostart,
    MenuIcon,
    Language,
    Fullscreen,
    Reset,
    Permissions,
    Licenses,
    About,
}

#[derive(Clone, Copy)]
enum Kind {
    Toggle,
    Segmented(&'static [&'static str]),
    Choice(&'static [&'static str]),
    Slider { min: i32, max: i32, step: i32, unit: &'static str },
    Button(&'static str),
    Info,
}

enum Item {
    Section(&'static str),
    Row { title: &'static str, desc: &'static str, key: Key, kind: Kind },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Preview {
    Open,
    Peek,
    Closed,
    None,
}

struct Page {
    name: &'static str,
    symbol: &'static str,
    preview: Preview,
    items: &'static [Item],
}

use Item::{Row, Section};

const NOTCH_BGS: &[&str] = &["Preto", "Cor do álbum", "Capa desfocada"];
const ACCENTS: &[&str] = &["Cor da capa", "Cor do sistema"];
const PLAY_SHAPES: &[&str] = &["Círculo", "Arredondado", "Quadrado"];
const SHUFFLE_MODES: &[&str] = &["Sempre", "Quando o player permite"];
const VIZ_STYLES: &[&str] = &["Barras", "Espelhado", "Onda"];
const PALETTES: &[&str] = &["Cor da capa", "Cor de destaque do sistema", "Arco-íris", "Neon", "Monocromático"];
const LANGS: &[&str] = &["Automático (idioma do sistema)", "Português (Brasil)", "English", "Español", "Deutsch"];
const BAR_NAMES: &[&str] = &["6", "8", "10", "12", "16", "24", "32"];

const BG_V: &[NotchBg] = &[NotchBg::Black, NotchBg::Album, NotchBg::Artwork];
const ACCENT_V: &[AccentSource] = &[AccentSource::Album, AccentSource::Windows];
const PLAY_V: &[Shape] = &[Shape::Circle, Shape::Rounded, Shape::Square];
const SHUFFLE_V: &[ShuffleRepeat] = &[ShuffleRepeat::Always, ShuffleRepeat::WhenSupported];
const VIZ_V: &[Style] = &[Style::Bars, Style::Mirror, Style::Wave];
const PALETTE_V: &[Palette] = &[Palette::Album, Palette::Accent, Palette::Rainbow, Palette::Neon, Palette::Mono];
const LANG_V: &[Language] = &[Language::Auto, Language::Pt, Language::En, Language::Es, Language::De];
const BARS: &[usize] = &[6, 8, 10, 12, 16, 24, 32];

const PAGES: &[Page] = &[
    Page {
        name: "Notch",
        symbol: "rectangle.topthird.inset.filled",
        preview: Preview::Open,
        items: &[
            Section("Ilha"),
            Row { title: "Mostrar música no notch", desc: "Capa e barras ao lado do notch; passe o mouse ou clique para abrir", key: Key::ShowMedia, kind: Kind::Toggle },
            Row { title: "Sempre visível", desc: "Fica no notch mesmo sem nada tocando", key: Key::AlwaysShow, kind: Kind::Toggle },
            Row { title: "Abrir ao passar o mouse", desc: "Desligado: o card abre só com um clique", key: Key::NotchHover, kind: Kind::Toggle },
            Row { title: "Indicador \"tocando agora\"", desc: "Mini equalizador quando não há barras de áudio", key: Key::Indicator, kind: Kind::Toggle },
            Row { title: "Animar troca de música", desc: "O título novo desliza e a capa nova aparece suavemente", key: Key::TrackAnim, kind: Kind::Toggle },
            Section("Card aberto"),
            Row { title: "Fundo", desc: "Preto como o notch, cor da capa ou capa desfocada", key: Key::NotchBg, kind: Kind::Segmented(NOTCH_BGS) },
            Row { title: "Cor de destaque", desc: "Botão play e progresso na cor da capa ou na do sistema", key: Key::Accent, kind: Kind::Segmented(ACCENTS) },
            Row { title: "Botão play", desc: "Formato do botão principal", key: Key::PlayShape, kind: Kind::Segmented(PLAY_SHAPES) },
            Row { title: "Barra de progresso", desc: "Tempo decorrido e restante; arraste para pular", key: Key::CardSeek, kind: Kind::Toggle },
            Row { title: "Volume", desc: "Slider de volume e botão de mudo; role sobre o card para ajustar", key: Key::ShowVolume, kind: Kind::Toggle },
            Row { title: "Aleatório e repetir", desc: "Apagados quando o player não deixa mudar pelo macOS", key: Key::CardShuffle, kind: Kind::Segmented(SHUFFLE_MODES) },
        ],
    },
    Page {
        name: "A seguir",
        symbol: "forward.end.fill",
        preview: Preview::Peek,
        items: &[
            Section("Comportamento"),
            Row { title: "Mostrar \"A seguir\"", desc: "Só quando o player troca de faixa sozinho; trocas manuais não mostram", key: Key::Popup, kind: Kind::Toggle },
            Row { title: "Duração", desc: "Quanto tempo o notch fica aberto com a próxima música", key: Key::PopupSecs, kind: Kind::Slider { min: 2, max: 12, step: 1, unit: "s" } },
        ],
    },
    Page {
        name: "Visualizador",
        symbol: "waveform",
        preview: Preview::Closed,
        items: &[
            Section("Barras de áudio"),
            Row { title: "Mostrar barras dinâmicas", desc: "Ao lado do notch e no card; usam o som que o Mac está tocando", key: Key::ShowViz, kind: Kind::Toggle },
            Row { title: "Estilo", desc: "Formato das barras", key: Key::Style, kind: Kind::Segmented(VIZ_STYLES) },
            Row { title: "Cores", desc: "Cor da capa, do sistema ou paletas", key: Key::Palette, kind: Kind::Choice(PALETTES) },
            Row { title: "Quantidade de barras", desc: "Mais barras = mais detalhe", key: Key::Bars, kind: Kind::Choice(BAR_NAMES) },
            Row { title: "Sensibilidade", desc: "Ganho extra aplicado ao espectro", key: Key::Sens, kind: Kind::Slider { min: -12, max: 18, step: 1, unit: "dB" } },
            Row { title: "Picos", desc: "Marcadores que caem devagar", key: Key::Peaks, kind: Kind::Toggle },
            Row { title: "Só quando um player estiver tocando", desc: "Ignora chamadas, avisos e sons do sistema", key: Key::OnlyMedia, kind: Kind::Toggle },
        ],
    },
    Page {
        name: "Geral",
        symbol: "gearshape",
        preview: Preview::None,
        items: &[
            Section("Sistema"),
            Row { title: "Abrir ao iniciar a sessão", desc: "O Taskbeat abre sozinho quando você entra no Mac", key: Key::Autostart, kind: Kind::Toggle },
            Row { title: "Ícone na barra de menus", desc: "Desligado: o menu abre com o botão direito no notch", key: Key::MenuIcon, kind: Kind::Toggle },
            Row { title: "Idioma", desc: "Automático segue o idioma do macOS (outros idiomas usam inglês)", key: Key::Language, kind: Kind::Choice(LANGS) },
            Row { title: "Ocultar em tela cheia", desc: "Some durante vídeos, jogos e apresentações", key: Key::Fullscreen, kind: Kind::Toggle },
            Row { title: "Restaurar padrões", desc: "Volta todas as opções para os valores originais", key: Key::Reset, kind: Kind::Button("Restaurar") },
            Section("Permissões"),
            Row { title: "Gravação de áudio do sistema", desc: "Necessária para as barras de áudio (Privacidade e Segurança)", key: Key::Permissions, kind: Kind::Button("Abrir Ajustes") },
            Section("Sobre"),
            Row { title: "Licenças", desc: "Licença MIT e bibliotecas de código aberto usadas", key: Key::Licenses, kind: Kind::Button("Ver licenças") },
            Row { title: "Taskbeat", desc: "Feito em Rust · Core Audio · Tocando Agora do macOS", key: Key::About, kind: Kind::Info },
        ],
    },
];

fn idx<T: PartialEq>(list: &[T], v: T) -> i32 {
    list.iter().position(|x| *x == v).unwrap_or(0) as i32
}

fn pick<T: Copy>(l: &[T], v: i32) -> T {
    l[(v.max(0) as usize).min(l.len() - 1)]
}

fn get(c: &Config, key: Key) -> i32 {
    match key {
        Key::ShowMedia => c.show_media as i32,
        Key::AlwaysShow => c.always_show_widget as i32,
        Key::NotchHover => c.notch_hover as i32,
        Key::Indicator => c.playing_indicator as i32,
        Key::TrackAnim => c.track_anim as i32,
        Key::NotchBg => idx(BG_V, c.notch_bg),
        Key::Accent => idx(ACCENT_V, c.accent_source),
        Key::PlayShape => idx(PLAY_V, c.play_shape),
        Key::CardSeek => c.card_seek as i32,
        Key::ShowVolume => c.show_volume as i32,
        Key::CardShuffle => idx(SHUFFLE_V, c.card_shuffle_repeat),
        Key::Popup => c.next_track_popup as i32,
        Key::PopupSecs => c.popup_secs,
        Key::ShowViz => c.show_visualizer as i32,
        Key::Style => idx(VIZ_V, c.style),
        Key::Palette => idx(PALETTE_V, c.palette),
        Key::Bars => BARS.iter().position(|b| *b >= c.bars).unwrap_or(BARS.len() - 1) as i32,
        Key::Sens => c.sensitivity_db.round() as i32,
        Key::Peaks => c.show_peaks as i32,
        Key::OnlyMedia => c.only_when_media as i32,
        Key::Autostart => system::autostart_enabled() as i32,
        Key::MenuIcon => c.menu_bar_icon as i32,
        Key::Language => idx(LANG_V, c.language),
        Key::Fullscreen => c.hide_on_fullscreen as i32,
        Key::Reset | Key::Permissions | Key::Licenses | Key::About => 0,
    }
}

fn set(c: &mut Config, key: Key, v: i32) {
    match key {
        Key::ShowMedia => c.show_media = v != 0,
        Key::AlwaysShow => c.always_show_widget = v != 0,
        Key::NotchHover => c.notch_hover = v != 0,
        Key::Indicator => c.playing_indicator = v != 0,
        Key::TrackAnim => c.track_anim = v != 0,
        Key::NotchBg => c.notch_bg = pick(BG_V, v),
        Key::Accent => c.accent_source = pick(ACCENT_V, v),
        Key::PlayShape => c.play_shape = pick(PLAY_V, v),
        Key::CardSeek => c.card_seek = v != 0,
        Key::ShowVolume => c.show_volume = v != 0,
        Key::CardShuffle => c.card_shuffle_repeat = pick(SHUFFLE_V, v),
        Key::Popup => c.next_track_popup = v != 0,
        Key::PopupSecs => c.popup_secs = v,
        Key::ShowViz => c.show_visualizer = v != 0,
        Key::Style => c.style = pick(VIZ_V, v),
        Key::Palette => c.palette = pick(PALETTE_V, v),
        Key::Bars => c.bars = pick(BARS, v),
        Key::Sens => c.sensitivity_db = v as f32,
        Key::Peaks => c.show_peaks = v != 0,
        Key::OnlyMedia => c.only_when_media = v != 0,
        Key::MenuIcon => c.menu_bar_icon = v != 0,
        Key::Language => {
            c.language = pick(LANG_V, v);
            crate::i18n::apply(c.language);
        }
        Key::Fullscreen => c.hide_on_fullscreen = v != 0,
        Key::Autostart | Key::Reset | Key::Permissions | Key::Licenses | Key::About => {}
    }
}

/// Opções que dependem de outra: o motivo (dica) quando estão desativadas.
fn requirement(c: &Config, key: Key) -> Option<&'static str> {
    use Key::*;
    match key {
        AlwaysShow | NotchHover | Indicator | TrackAnim if !c.show_media => Some("Ligue \"Mostrar música no notch\" para usar esta opção"),
        PopupSecs if !c.next_track_popup => Some("Ligue \"Mostrar A seguir\" para usar esta opção"),
        Style | Palette | Bars | Sens | Peaks | OnlyMedia if !c.show_visualizer => Some("Ligue \"Mostrar barras dinâmicas\" para usar esta opção"),
        _ => None,
    }
}

fn value_label(kind: Kind, key: Key, v: i32) -> String {
    let Kind::Slider { unit, .. } = kind else { return String::new() };
    match key {
        Key::Sens => format!("{v:+} {unit}"),
        _ => format!("{v} {unit}"),
    }
}

// ---------------------------------------------------------------- janela

struct RowView {
    key: Key,
    kind: Kind,
    control: Option<Retained<NSControl>>,
    value: Option<Retained<NSTextField>>,
    title: Retained<NSTextField>,
}

struct Settings {
    window: Retained<NSWindow>,
    rows: Vec<RowView>,
    previews: Vec<(Preview, Retained<NSImageView>)>,
    tr: TextRenderer,
}

thread_local! {
    static SET: RefCell<Option<Settings>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Settings) -> R) -> Option<R> {
    SET.with(|s| s.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

pub fn open(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    if with(|s| s.window.makeKeyAndOrderFront(None)).is_some() {
        app.activate();
        return;
    }
    let s = build(mtm);
    s.window.center();
    s.window.makeKeyAndOrderFront(None);
    app.activate();
    SET.with(|c| *c.borrow_mut() = Some(s));
    refresh();
}

// Vista com a origem no canto de cima (as linhas descem a partir do topo).
define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    struct Flipped;

    impl Flipped {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl Flipped {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(CGPoint::new(x, y), CGSize::new(w, h))
}

fn label(mtm: MainThreadMarker, text: &str, size: f64, bold: bool, secondary: bool) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&ns(text), mtm);
    let font = if bold { NSFont::boldSystemFontOfSize(size) } else { NSFont::systemFontOfSize(size) };
    l.setFont(Some(&font));
    if secondary {
        l.setTextColor(Some(&NSColor::secondaryLabelColor()));
    }
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l
}

fn build(mtm: MainThreadMarker) -> Settings {
    let tabs = NSTabViewController::new(mtm);
    tabs.setTabStyle(NSTabViewControllerTabStyle::Toolbar);
    let mut rows = Vec::new();
    let mut previews = Vec::new();
    for page in PAGES {
        let view = page_view(mtm, page, &mut rows, &mut previews);
        let vc = NSViewController::new(mtm);
        vc.setView(&view);
        vc.setTitle(Some(&ns(tr(page.name))));
        let item = NSTabViewItem::tabViewItemWithViewController(&vc);
        item.setLabel(&ns(tr(page.name)));
        if let Some(img) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns(page.symbol), Some(&ns(tr(page.name)))) {
            item.setImage(Some(&img));
        }
        tabs.addTabViewItem(&item);
    }
    let window = NSWindow::windowWithContentViewController(&tabs);
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&ns(tr("Taskbeat — Configurações")));
    let mut tr_ = TextRenderer::new();
    tr_.set_scale(2.0);
    Settings { window, rows, previews, tr: tr_ }
}

fn page_view(mtm: MainThreadMarker, page: &Page, rows: &mut Vec<RowView>, previews: &mut Vec<(Preview, Retained<NSImageView>)>) -> Retained<NSView> {
    let doc = Flipped::new(mtm, rect(0.0, 0.0, W, H));
    let mut y = 20.0;
    if page.preview != Preview::None {
        let iv = NSImageView::new(mtm);
        iv.setFrame(rect(24.0, y, W - 48.0, PREVIEW_H));
        iv.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        doc.addSubview(&iv);
        previews.push((page.preview, iv));
        y += PREVIEW_H + 16.0;
    }
    let target = target();
    let mut i = 0;
    while i < page.items.len() {
        let Section(name) = &page.items[i] else {
            i += 1;
            continue;
        };
        let head = label(mtm, tr(name), 13.0, true, false);
        head.setFrame(rect(30.0, y, W - 60.0, 18.0));
        doc.addSubview(&head);
        y += 24.0;
        // Fundo arredondado do grupo (posto atrás das linhas).
        let group = NSBox::new(mtm);
        group.setBoxType(NSBoxType::Custom);
        group.setTitlePosition(NSTitlePosition::NoTitle);
        group.setCornerRadius(10.0);
        group.setBorderWidth(0.0);
        group.setFillColor(&NSColor::quaternarySystemFillColor());
        doc.addSubview(&group);
        let top = y;
        i += 1;
        let mut first = true;
        while i < page.items.len() {
            let Row { title, desc, key, kind } = page.items[i] else { break };
            i += 1;
            if !first {
                let sep = NSBox::new(mtm);
                sep.setBoxType(NSBoxType::Separator);
                sep.setFrame(rect(40.0, y, W - 80.0, 1.0));
                doc.addSubview(&sep);
            }
            first = false;
            let desc_t = tr(desc);
            let long = desc_t.chars().count() > 62;
            let rh = if long { 60.0 } else { 50.0 };
            let t = label(mtm, tr(title), 13.0, false, false);
            t.setFrame(rect(40.0, y + 8.0, 330.0, 18.0));
            doc.addSubview(&t);
            let d = NSTextField::wrappingLabelWithString(&ns(desc_t), mtm);
            d.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            d.setTextColor(Some(&NSColor::secondaryLabelColor()));
            d.setFrame(rect(40.0, y + 26.0, 330.0, rh - 30.0));
            doc.addSubview(&d);

            let tag = rows.len() as isize;
            let (control, value): (Option<Retained<NSControl>>, Option<Retained<NSTextField>>) = match kind {
                Kind::Toggle => {
                    let sw = NSSwitch::new(mtm);
                    sw.sizeToFit();
                    Some((Retained::into_super(sw), None))
                }
                Kind::Segmented(opts) => {
                    let labels: Vec<Retained<NSString>> = opts.iter().map(|o| ns(tr(o))).collect();
                    let arr = NSArray::from_retained_slice(&labels);
                    let seg = unsafe { NSSegmentedControl::segmentedControlWithLabels_trackingMode_target_action(&arr, NSSegmentSwitchTracking::SelectOne, None, None, mtm) };
                    seg.sizeToFit();
                    Some((Retained::into_super(seg), None))
                }
                Kind::Choice(opts) => {
                    let pop = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), rect(0.0, 0.0, 210.0, 26.0), false);
                    for o in opts {
                        pop.addItemWithTitle(&ns(tr(o)));
                    }
                    Some((Retained::into_super(Retained::into_super(pop)), None))
                }
                Kind::Slider { min, max, step, .. } => {
                    let sl = unsafe { NSSlider::sliderWithValue_minValue_maxValue_target_action(min as f64, min as f64, max as f64, None, None, mtm) };
                    sl.setNumberOfTickMarks(((max - min) / step + 1) as isize);
                    sl.setAllowsTickMarkValuesOnly(true);
                    sl.setFrame(rect(0.0, 0.0, 150.0, 24.0));
                    let v = label(mtm, "", 12.0, false, true);
                    v.setFrame(rect(W - 40.0 - 52.0, y + (rh - 18.0) / 2.0, 52.0, 18.0));
                    doc.addSubview(&v);
                    Some((Retained::into_super(sl), Some(v)))
                }
                Kind::Button(text) => {
                    let b = unsafe { NSButton::buttonWithTitle_target_action(&ns(tr(text)), Some(&target), Some(sel!(button:)), mtm) };
                    Some((Retained::into_super(b), None))
                }
                Kind::Info => {
                    let v = label(mtm, &format!("v{}", env!("CARGO_PKG_VERSION")), 12.0, false, true);
                    v.setFrame(rect(W - 40.0 - 80.0, y + (rh - 18.0) / 2.0, 80.0, 18.0));
                    v.setAlignment(objc2_app_kit::NSTextAlignment::Right);
                    doc.addSubview(&v);
                    None
                }
            }
            .map_or((None, None), |(c, v)| (Some(c), v));
            if let Some(c) = &control {
                c.setTag(tag);
                if !matches!(kind, Kind::Button(_)) {
                    unsafe {
                        c.setTarget(Some(&target));
                        c.setAction(Some(sel!(control:)));
                    }
                }
                let f = c.frame();
                let right = if value.is_some() { W - 40.0 - 58.0 } else { W - 40.0 };
                c.setFrame(rect(right - f.size.width, y + (rh - f.size.height) / 2.0, f.size.width, f.size.height));
                doc.addSubview(c);
            }
            rows.push(RowView { key, kind, control, value, title: t });
            y += rh;
        }
        group.setFrame(rect(24.0, top, W - 48.0, y - top));
        y += 20.0;
    }
    doc.setFrameSize(CGSize::new(W, y.max(H)));
    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(0.0, 0.0, W, H));
    scroll.setHasVerticalScroller(true);
    scroll.setDrawsBackground(false);
    scroll.setDocumentView(Some(&doc));
    Retained::into_super(scroll)
}

// ---------------------------------------------------------------- valores

/// Atualiza controles, estados (opções dependentes) e prévias.
pub fn refresh() {
    with(|s| {
        let cfg = shared().config();
        for r in &s.rows {
            let v = get(&cfg, r.key);
            let why = requirement(&cfg, r.key);
            if let Some(c) = &r.control {
                c.setEnabled(why.is_none());
                c.setToolTip(why.map(|w| ns(tr(w))).as_deref());
                match r.kind {
                    Kind::Toggle => {
                        if let Some(sw) = c.downcast_ref::<NSSwitch>() {
                            let on = v != 0 && why.is_none();
                            sw.setState(if on { NSControlStateValueOn } else { NSControlStateValueOff });
                        }
                    }
                    Kind::Segmented(_) => {
                        if let Some(seg) = c.downcast_ref::<NSSegmentedControl>() {
                            seg.setSelectedSegment(v as isize);
                        }
                    }
                    Kind::Choice(_) => {
                        if let Some(p) = c.downcast_ref::<NSPopUpButton>() {
                            p.selectItemAtIndex(v as isize);
                        }
                    }
                    Kind::Slider { .. } => c.setDoubleValue(v as f64),
                    _ => {}
                }
            }
            let color = if why.is_some() { NSColor::disabledControlTextColor() } else { NSColor::labelColor() };
            r.title.setTextColor(Some(&color));
            if let Some(l) = &r.value {
                l.setStringValue(&ns(&value_label(r.kind, r.key, v)));
            }
        }
        render_previews(s, &cfg);
    });
}

fn render_previews(s: &mut Settings, cfg: &Config) {
    let mtm = MainThreadMarker::new().unwrap();
    let sh = shared();
    let real = sh.media.lock().unwrap().clone();
    let media = if real.present() && std::env::var_os("TV_DEMO_MEDIA").is_none() { real } else { crate::ui::sample_media() };
    let colors = system::colors(mtm, cfg.palette);
    let sc = 2.0f32;
    let g = Geom { notch_w: 196.0 * sc, notch_h: 32.0 * sc };
    for (kind, iv) in &s.previews {
        let (open, peek) = match kind {
            Preview::Open => (1.0, 0.0),
            Preview::Peek => (0.0, 1.0),
            _ => (0.0, 0.0),
        };
        let (w, h, px) = nd::preview(&mut s.tr, sc, g, cfg, &colors, &media, open, peek, 1.3);
        // Fechada: recorta em volta da ilha (fica pequena na tela inteira).
        let (w, h, px) = if *kind == Preview::Closed {
            let (x0, x1, y1) = (w / 4, w * 3 / 4, ((g.notch_h * 3.2) as usize).min(h));
            let mut out = Vec::with_capacity((x1 - x0) * y1);
            for y in 0..y1 {
                out.extend_from_slice(&px[y * w + x0..y * w + x1]);
            }
            (x1 - x0, y1, out)
        } else {
            (w, h, px)
        };
        if let Some(cg) = gfx::to_cgimage(&px, w, h) {
            let img = NSImage::initWithCGImage_size(NSImage::alloc(), &cg, CGSize::new(w as f64 / sc as f64, h as f64 / sc as f64));
            iv.setImage(Some(&img));
        }
    }
}

/// Um interruptor, lista, controle segmentado ou slider mudou.
pub fn control_changed(sender: Option<&AnyObject>) {
    let Some(c) = sender.and_then(|s| s.downcast_ref::<NSControl>()) else { return };
    let tag = c.tag();
    let Some((key, kind)) = with(|s| s.rows.get(tag as usize).map(|r| (r.key, r.kind))).flatten() else { return };
    let v = match kind {
        Kind::Toggle => c.downcast_ref::<NSSwitch>().map(|s| (s.state() == NSControlStateValueOn) as i32).unwrap_or(0),
        Kind::Segmented(_) => c.downcast_ref::<NSSegmentedControl>().map(|s| s.selectedSegment() as i32).unwrap_or(0),
        Kind::Choice(_) => c.downcast_ref::<NSPopUpButton>().map(|p| p.indexOfSelectedItem() as i32).unwrap_or(0),
        Kind::Slider { step, .. } => ((c.doubleValue() / step as f64).round() as i32) * step,
        _ => return,
    };
    let sh = shared();
    match key {
        Key::Autostart => system::set_autostart(v != 0),
        Key::Language => {
            sh.update_config(|cfg| set(cfg, key, v));
            // Refaz a janela no idioma novo.
            let mtm = MainThreadMarker::new().unwrap();
            if let Some(old) = SET.with(|s| s.borrow_mut().take()) {
                old.window.close();
            }
            super::config_changed();
            open(mtm);
            return;
        }
        _ => {
            // Slider: aplica enquanto arrasta e grava ao soltar.
            let mtm = MainThreadMarker::new().unwrap();
            let up = NSApplication::sharedApplication(mtm).currentEvent().is_none_or(|e| e.r#type() != NSEventType::LeftMouseDragged);
            if matches!(kind, Kind::Slider { .. }) && !up {
                sh.preview_config(|cfg| set(cfg, key, v));
            } else {
                sh.update_config(|cfg| set(cfg, key, v));
            }
        }
    }
    super::config_changed();
    refresh();
}

/// Botões: restaurar, abrir os Ajustes, licenças.
pub fn button(sender: Option<&AnyObject>) {
    let Some(c) = sender.and_then(|s| s.downcast_ref::<NSControl>()) else { return };
    let Some(key) = with(|s| s.rows.get(c.tag() as usize).map(|r| r.key)).flatten() else { return };
    match key {
        Key::Reset => {
            let lang = shared().config().language;
            shared().update_config(|cfg| *cfg = Config { language: lang, ..Config::default() });
            super::config_changed();
            refresh();
        }
        Key::Permissions => {
            let url = "x-apple.systempreferences:com.apple.preference.security?Privacy_AudioCapture";
            if let Some(u) = NSURL::URLWithString(&ns(url)) {
                objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(&u);
            }
        }
        Key::Licenses => crate::licenses::open(),
        _ => {}
    }
}

/// Quadro de animação da prévia (reservado; a prévia é estática).
pub fn frame() {}

#[cfg(test)]
mod tests {
    #[test]
    fn every_label_is_translated() {
        let mut missing = Vec::new();
        let mut check = |t: &'static str| {
            if !t.is_empty() && !t.chars().all(|c| c.is_ascii_digit()) && !crate::i18n::has(t) && !["English", "Español", "Deutsch", "Português (Brasil)", "Neon", "Taskbeat", "Notch"].contains(&t) {
                missing.push(t);
            }
        };
        for p in super::PAGES {
            check(p.name);
            for it in p.items {
                match it {
                    super::Item::Section(s) => check(s),
                    super::Item::Row { title, desc, kind, .. } => {
                        check(title);
                        check(desc);
                        match kind {
                            super::Kind::Segmented(o) | super::Kind::Choice(o) => o.iter().for_each(|o| check(o)),
                            super::Kind::Button(b) => check(b),
                            _ => {}
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "sem tradução: {missing:#?}");
    }
}
