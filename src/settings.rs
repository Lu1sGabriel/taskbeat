//! Tela de configurações no estilo Fluent 2 / WinUI 3: fundo Mica, menu
//! lateral, prévias ao vivo de cada componente, cartões agrupados com toggle,
//! controle segmentado, lista, slider e botão, e rolagem suave.
//!
//! Tudo é desenhado pelo próprio app (mesmo código do widget, em `ui.rs`) e
//! copiado para a janela com alfa: com o quadro estendido para a área cliente,
//! o DWM mostra o Mica por trás das partes transparentes. Sem Mica (Windows 10)
//! o fundo vira uma cor sólida do tema. Mudanças são aplicadas na hora;
//! sliders só gravam no disco ao soltar.

use crate::config::*;
use crate::i18n::tr;
use crate::overlay::Surface;
use crate::render::{Canvas, Colors, RectF, Rgb, lerp3};
use crate::shared::{MediaInfo, Shared};
use crate::system;
use crate::text::{Align, Font, TextRenderer};
use crate::ui::{self, Interact, Playback};
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, w};

const WM_MOUSELEAVE: u32 = 0x02A3;
const WIN_W: f32 = 1060.0;
const WIN_H: f32 = 760.0;
const NAV_W: f32 = 268.0;
const MIN_W: f32 = 820.0;
const MIN_H: f32 = 560.0;
const ANIM_TIMER: usize = 1;

// ---------------------------------------------------------------- modelo

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Key {
    ShowMedia,
    TaskbarControls,
    ShowArtist,
    Indicator,
    TaskbarProgress,
    CoverShape,
    WidgetPill,
    Marquee,
    TitleWidth,
    Anchor,
    Offset,
    CardStyle,
    CardLayout,
    Theme,
    Accent,
    Corners,
    PlayShape,
    CardSeek,
    ShowVolume,
    Autoclose,
    Popup,
    PopupSecs,
    ShowSource,
    CapsOsd,
    NumOsd,
    OsdShowOff,
    OsdPos,
    OsdOffX,
    OsdOffY,
    OsdShape,
    OsdSize,
    OsdTheme,
    OsdDuration,
    OsdSwitch,
    OsdTest,
    ShowViz,
    Style,
    Palette,
    Bars,
    Width,
    Sens,
    OnlyMedia,
    Peaks,
    Autostart,
    Language,
    Fullscreen,
    Reset,
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
    Row { icon: &'static str, title: &'static str, desc: &'static str, key: Key, kind: Kind },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Preview {
    Widget,
    Card,
    UpNext,
    LockKeys,
    Visualizer,
    None,
}

struct Page {
    name: &'static str,
    icon: &'static str,
    subtitle: &'static str,
    preview: Preview,
    items: &'static [Item],
}

use Item::{Row, Section};

const SHAPES: &[&str] = &["Arredondada", "Círculo", "Quadrada"];
const PLAY_SHAPES: &[&str] = &["Círculo", "Arredondado", "Quadrado"];
const ANCHORS: &[&str] = &["Ao lado do relógio", "À esquerda"];
const CARD_STYLES: &[&str] = &["Capa desfocada", "Cor do álbum", "Vidro"];
const CARD_LAYOUTS: &[&str] = &["Horizontal", "Vertical"];
const THEMES: &[&str] = &["Automático", "Escuro", "Claro"];
const ACCENTS: &[&str] = &["Cor da capa", "Cor do Windows"];
const CORNERS: &[&str] = &["Pequenos", "Médios", "Grandes"];
const OSD_POSITIONS: &[&str] = &["Embaixo", "Centro", "Topo", "Acima do player"];
const OSD_SHAPES: &[&str] = &["Iguais ao card", "Pílula", "Retos"];
const OSD_SIZES: &[&str] = &["Compacto", "Normal", "Grande"];
const VIZ_STYLES: &[&str] = &["Barras", "Espelhado", "Onda"];
const PALETTES: &[&str] = &["Cor da capa", "Cor de destaque do Windows", "Arco-íris", "Neon", "Monocromático"];
const LANGS: &[&str] = &["Automático (idioma do Windows)", "Português (Brasil)", "English", "Español", "Deutsch"];
const LANG_V: &[Language] = &[Language::Auto, Language::Pt, Language::En, Language::Es, Language::De];
const BAR_NAMES: &[&str] = &["6", "8", "10", "12", "16", "24", "32"];
const BARS: &[usize] = &[6, 8, 10, 12, 16, 24, 32];

const PAGES: &[Page] = &[
    Page {
        name: "Barra de tarefas",
        icon: "\u{E7F4}",
        subtitle: "O widget compacto que fica ao lado do relógio",
        preview: Preview::Widget,
        items: &[
            Section("Conteúdo"),
            Row { icon: "\u{E8D6}", title: "Mostrar música na barra", desc: "Capa, título e artista; clique para abrir o card", key: Key::ShowMedia, kind: Kind::Toggle },
            Row { icon: "\u{E768}", title: "Botões de controle na barra", desc: "Anterior, play e próximo também na barra; por padrão ficam só no card", key: Key::TaskbarControls, kind: Kind::Toggle },
            Row { icon: "\u{E77B}", title: "Mostrar artista", desc: "Segunda linha com o nome do artista", key: Key::ShowArtist, kind: Kind::Toggle },
            Row { icon: "\u{E9E9}", title: "Indicador \"tocando agora\"", desc: "Mini equalizador animado ao lado do título", key: Key::Indicator, kind: Kind::Toggle },
            Row { icon: "\u{E916}", title: "Barra de progresso", desc: "Linha fina com o andamento da música", key: Key::TaskbarProgress, kind: Kind::Toggle },
            Section("Aparência"),
            Row { icon: "\u{E91B}", title: "Formato da capa", desc: "Como a capa aparece na barra", key: Key::CoverShape, kind: Kind::Segmented(SHAPES) },
            Row { icon: "\u{E7B5}", title: "Fundo em pílula", desc: "Destaque sutil sempre visível atrás do widget", key: Key::WidgetPill, kind: Kind::Toggle },
            Row { icon: "\u{E8C9}", title: "Rolar títulos longos", desc: "Letreiro suave quando o nome não cabe", key: Key::Marquee, kind: Kind::Toggle },
            Row { icon: "\u{E8D2}", title: "Largura do título", desc: "Espaço para o nome da música e do artista", key: Key::TitleWidth, kind: Kind::Slider { min: 100, max: 280, step: 10, unit: "px" } },
            Section("Posição"),
            Row { icon: "\u{E7C4}", title: "Posição na barra", desc: "Onde o widget fica na barra de tarefas", key: Key::Anchor, kind: Kind::Segmented(ANCHORS) },
            Row { icon: "\u{E8AB}", title: "Ajuste horizontal", desc: "Desloca o widget para a esquerda ou para a direita", key: Key::Offset, kind: Kind::Slider { min: -1200, max: 1200, step: 10, unit: "px" } },
        ],
    },
    Page {
        name: "Card expandido",
        icon: "\u{E7C4}",
        subtitle: "Abre ao clicar na música, com todos os controles",
        preview: Preview::Card,
        items: &[
            Section("Estilo"),
            Row { icon: "\u{E771}", title: "Fundo", desc: "Capa desfocada, cor extraída da capa ou vidro neutro", key: Key::CardStyle, kind: Kind::Segmented(CARD_STYLES) },
            Row { icon: "\u{E8A9}", title: "Layout", desc: "Capa ao lado ou capa grande em cima", key: Key::CardLayout, kind: Kind::Segmented(CARD_LAYOUTS) },
            Row { icon: "\u{E793}", title: "Tema", desc: "Segue o Windows ou fixa claro/escuro", key: Key::Theme, kind: Kind::Segmented(THEMES) },
            Row { icon: "\u{E790}", title: "Cor de destaque", desc: "Botões e progresso na cor da capa ou do Windows", key: Key::Accent, kind: Kind::Segmented(ACCENTS) },
            Row { icon: "\u{E7FB}", title: "Cantos", desc: "Raio das bordas do card", key: Key::Corners, kind: Kind::Segmented(CORNERS) },
            Row { icon: "\u{E768}", title: "Botão play", desc: "Formato do botão principal", key: Key::PlayShape, kind: Kind::Segmented(PLAY_SHAPES) },
            Section("Conteúdo"),
            Row { icon: "\u{E916}", title: "Barra de progresso", desc: "Tempo decorrido e restante; arraste para pular", key: Key::CardSeek, kind: Kind::Toggle },
            Row { icon: "\u{E767}", title: "Volume", desc: "Slider de volume e botão de mudo", key: Key::ShowVolume, kind: Kind::Toggle },
            Row { icon: "\u{E7F4}", title: "Mostrar app de origem", desc: "Nome do app (Spotify, Brave…) no card e ícone sobre a capa", key: Key::ShowSource, kind: Kind::Toggle },
            Row { icon: "\u{E916}", title: "Fechar sozinho após", desc: "Tempo sem o mouse em cima do card", key: Key::Autoclose, kind: Kind::Slider { min: 2, max: 20, step: 1, unit: "s" } },
            Row { icon: "\u{E8B1}", title: "Aleatório e repetir", desc: "Aparecem quando o player permite (Spotify, Media Player). Navegadores como Brave e Chrome não permitem", key: Key::About, kind: Kind::Info },
        ],
    },
    Page {
        name: "A seguir",
        icon: "\u{E768}",
        subtitle: "Cartão com a próxima música quando a faixa termina",
        preview: Preview::UpNext,
        items: &[
            Section("Comportamento"),
            Row { icon: "\u{E768}", title: "Mostrar \"A seguir\"", desc: "Só quando o player troca de faixa sozinho; trocas manuais não mostram", key: Key::Popup, kind: Kind::Toggle },
            Row { icon: "\u{E916}", title: "Duração", desc: "Quanto tempo o cartão fica na tela", key: Key::PopupSecs, kind: Kind::Slider { min: 2, max: 12, step: 1, unit: "s" } },
        ],
    },
    Page {
        name: "Caps Lock",
        icon: "\u{E765}",
        subtitle: "Aviso na tela ao ligar ou desligar Caps Lock e Num Lock",
        preview: Preview::LockKeys,
        items: &[
            Section("Avisos"),
            Row { icon: "\u{E765}", title: "Caps Lock", desc: "Mostra quando as maiúsculas forem ativadas ou desativadas", key: Key::CapsOsd, kind: Kind::Toggle },
            Row { icon: "\u{E765}", title: "Num Lock", desc: "Mostra quando o teclado numérico mudar", key: Key::NumOsd, kind: Kind::Toggle },
            Row { icon: "\u{E7E8}", title: "Avisar também ao desligar", desc: "Desligado: só aparece quando a tecla é ativada", key: Key::OsdShowOff, kind: Kind::Toggle },
            Row { icon: "\u{E768}", title: "Testar", desc: "Mostra o aviso agora, na posição escolhida", key: Key::OsdTest, kind: Kind::Button("Mostrar agora") },
            Section("Posição"),
            Row { icon: "\u{E7C4}", title: "Onde aparece", desc: "Predefinições rápidas; ajuste fino logo abaixo", key: Key::OsdPos, kind: Kind::Segmented(OSD_POSITIONS) },
            Row { icon: "\u{E8AB}", title: "Ajuste horizontal", desc: "Move o aviso para a esquerda ou para a direita", key: Key::OsdOffX, kind: Kind::Slider { min: -800, max: 800, step: 10, unit: "px" } },
            Row { icon: "\u{E8CB}", title: "Ajuste vertical", desc: "Move o aviso para cima ou para baixo", key: Key::OsdOffY, kind: Kind::Slider { min: -600, max: 600, step: 10, unit: "px" } },
            Section("Aparência"),
            Row { icon: "\u{E7FB}", title: "Cantos", desc: "Formato do aviso e do ícone", key: Key::OsdShape, kind: Kind::Segmented(OSD_SHAPES) },
            Row { icon: "\u{E8D3}", title: "Tamanho", desc: "Tamanho do aviso e do texto", key: Key::OsdSize, kind: Kind::Segmented(OSD_SIZES) },
            Row { icon: "\u{E793}", title: "Tema", desc: "Segue o Windows ou fixa claro/escuro", key: Key::OsdTheme, kind: Kind::Segmented(THEMES) },
            Row { icon: "\u{E7E8}", title: "Interruptor", desc: "Chave animada no lado direito", key: Key::OsdSwitch, kind: Kind::Toggle },
            Row { icon: "\u{E916}", title: "Duração", desc: "Quanto tempo o aviso fica na tela", key: Key::OsdDuration, kind: Kind::Slider { min: 8, max: 50, step: 1, unit: "s" } },
            Section("Privacidade"),
            Row { icon: "\u{E72E}", title: "Privacidade", desc: "Só essas duas teclas são observadas; nada do que você digita é lido ou guardado", key: Key::About, kind: Kind::Info },
        ],
    },
    Page {
        name: "Visualizador",
        icon: "\u{E9E9}",
        subtitle: "Barras de áudio dinâmicas ao lado da música",
        preview: Preview::Visualizer,
        items: &[
            Section("Barras de áudio"),
            Row { icon: "\u{E9E9}", title: "Mostrar barras dinâmicas", desc: "Desligado por padrão; quando ligado, captura o som do PC", key: Key::ShowViz, kind: Kind::Toggle },
            Row { icon: "\u{E771}", title: "Estilo", desc: "Formato das barras", key: Key::Style, kind: Kind::Segmented(VIZ_STYLES) },
            Row { icon: "\u{E790}", title: "Cores", desc: "Cor da capa, do Windows ou paletas", key: Key::Palette, kind: Kind::Choice(PALETTES) },
            Row { icon: "\u{E8FD}", title: "Quantidade de barras", desc: "Mais barras = mais detalhe", key: Key::Bars, kind: Kind::Choice(BAR_NAMES) },
            Row { icon: "\u{E9A6}", title: "Largura", desc: "Tamanho do visualizador", key: Key::Width, kind: Kind::Slider { min: 20, max: 160, step: 4, unit: "px" } },
            Row { icon: "\u{E9D9}", title: "Sensibilidade", desc: "Ganho extra aplicado ao espectro", key: Key::Sens, kind: Kind::Slider { min: -12, max: 18, step: 1, unit: "dB" } },
            Row { icon: "\u{E70E}", title: "Picos", desc: "Marcadores que caem devagar", key: Key::Peaks, kind: Kind::Toggle },
            Row { icon: "\u{E8B1}", title: "Só quando um player estiver tocando", desc: "Ignora jogos, chamadas e notificações", key: Key::OnlyMedia, kind: Kind::Toggle },
        ],
    },
    Page {
        name: "Geral",
        icon: "\u{E713}",
        subtitle: "Inicialização, tela cheia e informações",
        preview: Preview::None,
        items: &[
            Section("Sistema"),
            Row { icon: "\u{E7E8}", title: "Iniciar com o Windows", desc: "Abre automaticamente quando você entra no Windows", key: Key::Autostart, kind: Kind::Toggle },
            Row { icon: "\u{F2B7}", title: "Idioma", desc: "Automático segue o idioma do Windows (outros idiomas usam inglês)", key: Key::Language, kind: Kind::Choice(LANGS) },
            Row { icon: "\u{E740}", title: "Ocultar em tela cheia", desc: "Some durante jogos, vídeos e apresentações", key: Key::Fullscreen, kind: Kind::Toggle },
            Row { icon: "\u{E72C}", title: "Restaurar padrões", desc: "Volta todas as opções para os valores originais", key: Key::Reset, kind: Kind::Button("Restaurar") },
            Section("Sobre"),
            Row { icon: "\u{E946}", title: "Taskbeat", desc: "Feito em Rust · WASAPI loopback · System Media Transport Controls", key: Key::About, kind: Kind::Info },
        ],
    },
];

fn idx<T: PartialEq>(list: &[T], v: T) -> i32 {
    list.iter().position(|x| *x == v).unwrap_or(0) as i32
}

const SHAPE_V: &[Shape] = &[Shape::Rounded, Shape::Circle, Shape::Square];
const PLAY_V: &[Shape] = &[Shape::Circle, Shape::Rounded, Shape::Square];
const ANCHOR_V: &[Anchor] = &[Anchor::BeforeTray, Anchor::Left];
const STYLE_V: &[CardStyle] = &[CardStyle::Artwork, CardStyle::AlbumColor, CardStyle::Glass];
const LAYOUT_V: &[CardLayout] = &[CardLayout::Horizontal, CardLayout::Vertical];
const THEME_V: &[ThemeMode] = &[ThemeMode::Auto, ThemeMode::Dark, ThemeMode::Light];
const ACCENT_V: &[AccentSource] = &[AccentSource::Album, AccentSource::Windows];
const CORNER_V: &[Corners] = &[Corners::Small, Corners::Medium, Corners::Large];
const VIZ_V: &[Style] = &[Style::Bars, Style::Mirror, Style::Wave];
const OSD_POS_V: &[OsdPosition] = &[OsdPosition::Bottom, OsdPosition::Center, OsdPosition::Top, OsdPosition::AbovePlayer];
const OSD_SHAPE_V: &[OsdShape] = &[OsdShape::Rounded, OsdShape::Pill, OsdShape::Square];
const OSD_SIZE_V: &[OsdSize] = &[OsdSize::Compact, OsdSize::Normal, OsdSize::Large];
const PALETTE_V: &[Palette] = &[Palette::Album, Palette::Accent, Palette::Rainbow, Palette::Neon, Palette::Mono];

fn get(c: &Config, key: Key) -> i32 {
    match key {
        Key::ShowMedia => c.show_media as i32,
        Key::TaskbarControls => c.taskbar_controls as i32,
        Key::ShowArtist => c.show_artist as i32,
        Key::Indicator => c.playing_indicator as i32,
        Key::TaskbarProgress => c.taskbar_progress as i32,
        Key::CoverShape => idx(SHAPE_V, c.cover_shape),
        Key::WidgetPill => c.widget_pill as i32,
        Key::Marquee => c.marquee as i32,
        Key::TitleWidth => c.title_width,
        Key::Anchor => idx(ANCHOR_V, c.anchor),
        Key::Offset => c.offset,
        Key::CardStyle => idx(STYLE_V, c.card_style),
        Key::CardLayout => idx(LAYOUT_V, c.card_layout),
        Key::Theme => idx(THEME_V, c.theme),
        Key::Accent => idx(ACCENT_V, c.accent_source),
        Key::Corners => idx(CORNER_V, c.corners),
        Key::PlayShape => idx(PLAY_V, c.play_shape),
        Key::CardSeek => c.card_seek as i32,
        Key::ShowVolume => c.show_volume as i32,
        Key::Autoclose => c.card_autoclose,
        Key::Popup => c.next_track_popup as i32,
        Key::PopupSecs => c.popup_secs,
        Key::ShowSource => c.show_source as i32,
        Key::CapsOsd => c.caps_lock_osd as i32,
        Key::NumOsd => c.num_lock_osd as i32,
        Key::OsdShowOff => c.osd_show_off as i32,
        Key::OsdPos => idx(OSD_POS_V, c.osd_position),
        Key::OsdOffX => c.osd_offset_x,
        Key::OsdOffY => c.osd_offset_y,
        Key::OsdShape => idx(OSD_SHAPE_V, c.osd_shape),
        Key::OsdSize => idx(OSD_SIZE_V, c.osd_size),
        Key::OsdTheme => idx(THEME_V, c.osd_theme),
        Key::OsdDuration => c.osd_duration,
        Key::OsdSwitch => c.osd_switch as i32,
        Key::ShowViz => c.show_visualizer as i32,
        Key::Style => idx(VIZ_V, c.style),
        Key::Palette => idx(PALETTE_V, c.palette),
        Key::Bars => BARS.iter().position(|b| *b >= c.bars).unwrap_or(BARS.len() - 1) as i32,
        Key::Width => c.width,
        Key::Sens => c.sensitivity_db.round() as i32,
        Key::OnlyMedia => c.only_when_media as i32,
        Key::Peaks => c.show_peaks as i32,
        Key::Autostart => AUTOSTART.with(|a| a.get()) as i32,
        Key::Fullscreen => c.hide_on_fullscreen as i32,
        Key::Language => idx(LANG_V, c.language),
        Key::Reset | Key::About | Key::OsdTest => 0,
    }
}

fn set(c: &mut Config, key: Key, v: i32) {
    let i = v.max(0) as usize;
    fn pick<T: Copy>(l: &[T], i: usize) -> T {
        l[i.min(l.len() - 1)]
    }
    match key {
        Key::ShowMedia => c.show_media = v != 0,
        Key::TaskbarControls => c.taskbar_controls = v != 0,
        Key::ShowArtist => c.show_artist = v != 0,
        Key::Indicator => c.playing_indicator = v != 0,
        Key::TaskbarProgress => c.taskbar_progress = v != 0,
        Key::CoverShape => c.cover_shape = pick(SHAPE_V, i),
        Key::WidgetPill => c.widget_pill = v != 0,
        Key::Marquee => c.marquee = v != 0,
        Key::TitleWidth => c.title_width = v,
        Key::Anchor => c.anchor = pick(ANCHOR_V, i),
        Key::Offset => c.offset = v,
        Key::CardStyle => c.card_style = pick(STYLE_V, i),
        Key::CardLayout => c.card_layout = pick(LAYOUT_V, i),
        Key::Theme => c.theme = pick(THEME_V, i),
        Key::Accent => c.accent_source = pick(ACCENT_V, i),
        Key::Corners => c.corners = pick(CORNER_V, i),
        Key::PlayShape => c.play_shape = pick(PLAY_V, i),
        Key::CardSeek => c.card_seek = v != 0,
        Key::ShowVolume => c.show_volume = v != 0,
        Key::Autoclose => c.card_autoclose = v,
        Key::Popup => c.next_track_popup = v != 0,
        Key::PopupSecs => c.popup_secs = v,
        Key::ShowSource => c.show_source = v != 0,
        Key::CapsOsd => c.caps_lock_osd = v != 0,
        Key::NumOsd => c.num_lock_osd = v != 0,
        Key::OsdShowOff => c.osd_show_off = v != 0,
        Key::OsdPos => c.osd_position = pick(OSD_POS_V, i),
        Key::OsdOffX => c.osd_offset_x = v,
        Key::OsdOffY => c.osd_offset_y = v,
        Key::OsdShape => c.osd_shape = pick(OSD_SHAPE_V, i),
        Key::OsdSize => c.osd_size = pick(OSD_SIZE_V, i),
        Key::OsdTheme => c.osd_theme = pick(THEME_V, i),
        Key::OsdDuration => c.osd_duration = v,
        Key::OsdSwitch => c.osd_switch = v != 0,
        Key::ShowViz => c.show_visualizer = v != 0,
        Key::Style => c.style = pick(VIZ_V, i),
        Key::Palette => c.palette = pick(PALETTE_V, i),
        Key::Bars => c.bars = pick(BARS, i),
        Key::Width => c.width = v,
        Key::Sens => c.sensitivity_db = v as f32,
        Key::OnlyMedia => c.only_when_media = v != 0,
        Key::Peaks => c.show_peaks = v != 0,
        Key::Fullscreen => c.hide_on_fullscreen = v != 0,
        Key::Language => {
            c.language = pick(LANG_V, i);
            crate::i18n::apply(c.language);
        }
        Key::Autostart | Key::Reset | Key::About | Key::OsdTest => {}
    }
}

fn value_label(kind: Kind, key: Key, v: i32) -> String {
    let Kind::Slider { unit, .. } = kind else { return String::new() };
    match key {
        Key::Sens | Key::Offset | Key::OsdOffX | Key::OsdOffY => format!("{v:+} {unit}"),
        Key::OsdDuration => format!("{},{} {unit}", v / 10, v % 10),
        _ => format!("{v} {unit}"),
    }
}

// ---------------------------------------------------------------- estado da janela

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Nav(usize),
    Row(usize),
    Seg(usize, usize),
}

/// Camada já desenhada guardada em memória (só redesenha quando a chave muda).
struct Cached<K> {
    key: K,
    w: usize,
    h: usize,
    px: Vec<u32>,
}

/// Tudo que muda a coluna de opções (a rolagem não entra: a camada inteira é
/// desenhada uma vez e só copiada na posição da rolagem).
#[derive(Clone, Copy, PartialEq)]
struct ContentKey {
    page: usize,
    w: i32,
    h: i32,
    scale: u32,
    dark: bool,
    mica: bool,
    accent: [u32; 3],
    cfg: u64,
    hover: Option<Hit>,
    drag: Option<usize>,
    autostart: bool,
}

/// Parte fixa da prévia: "papel de parede" e barra de tarefas simulada.
#[derive(Clone, Copy, PartialEq)]
struct HeroKey {
    preview: Preview,
    w: i32,
    h: i32,
    scale: u32,
    dark: bool,
    mica: bool,
    light_tb: bool,
}

struct Ui {
    shared: Arc<Shared>,
    hwnd: HWND,
    scale: f32,
    dark: bool,
    mica: bool,
    accent: Rgb,
    page: usize,
    scroll: f32,
    scroll_target: f32,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    drag: Option<usize>,
    tracking: bool,
    surface: Option<Surface>,
    text: TextRenderer,
    t0: std::time::Instant,
    colors: Option<(Palette, Colors)>,
    content: Option<Cached<ContentKey>>,
    hero_bg: Option<Cached<HeroKey>>,
    /// Período do timer de animação em ms (0 = parado).
    timer_ms: u32,
    last_tick: std::time::Instant,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    /// "Iniciar com o Windows" (lido do registro ao abrir, não a cada quadro).
    static AUTOSTART: Cell<bool> = const { Cell::new(false) };
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|u| u.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn ui_hwnd() -> Option<HWND> {
    with_ui(|u| u.hwnd)
}

pub fn refresh() {
    if let Some(h) = ui_hwnd() {
        unsafe {
            let _ = InvalidateRect(Some(h), None, false);
        }
    }
}

pub fn close() {
    if let Some(h) = ui_hwnd() {
        unsafe {
            let _ = DestroyWindow(h);
        }
    }
}

pub(crate) fn apply_theme(hwnd: HWND, dark: bool) -> bool {
    unsafe {
        let d = windows::core::BOOL::from(dark);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &d as *const _ as _, 4);
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as _, 4);
        let backdrop = DWMSBT_MAINWINDOW;
        let mica = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &backdrop as *const _ as _, 4).is_ok();
        if mica {
            let m = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &m);
        }
        mica
    }
}

const STYLE: WINDOW_STYLE = WS_OVERLAPPEDWINDOW;

pub fn open(shared: Arc<Shared>, icon: HICON) {
    if let Some(h) = ui_hwnd() {
        unsafe {
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            let _ = SetForegroundWindow(h);
        }
        return;
    }
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = module.into();
        let class = w!("TaskbeatSettings");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            hIcon: icon,
            hIconSm: icon,
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let dpi = GetDpiForSystem().max(96);
        let (ww, wh) = window_size(dpi, WIN_W, WIN_H);
        let sx = (GetSystemMetrics(SM_CXSCREEN) - ww) / 2;
        let sy = ((GetSystemMetrics(SM_CYSCREEN) - wh) / 2).max(0);
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), class, &HSTRING::from(tr("Taskbeat — Configurações")), STYLE, sx, sy, ww, wh, None, None, Some(hinst), None) else {
            return;
        };
        let dpi = GetDpiForWindow(hwnd).max(96);
        let dark = system::apps_use_dark();
        let mica = apply_theme(hwnd, dark);
        let mut text = TextRenderer::new();
        // A tela tem bem mais textos que o widget: cache maior evita
        // rasterizar tudo de novo a cada quadro.
        text.set_capacity(900);
        let scale = dpi as f32 / 96.0;
        text.set_scale(scale);
        AUTOSTART.with(|a| a.set(system::autostart_enabled()));
        UI.with(|u| {
            *u.borrow_mut() = Some(Ui {
                shared,
                hwnd,
                scale,
                dark,
                mica,
                accent: system::app_accent(dark),
                page: 0,
                scroll: 0.0,
                scroll_target: 0.0,
                hover: None,
                pressed: None,
                drag: None,
                tracking: false,
                surface: None,
                text,
                t0: std::time::Instant::now(),
                colors: None,
                content: None,
                hero_bg: None,
                timer_ms: 0,
                last_tick: std::time::Instant::now(),
            });
        });
        // A 1ª chamada de ShowWindow do processo pode herdar o modo de exibição do
        // atalho/linha de comando (ex.: oculto); a 2ª garante que a janela apareça.
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(hwnd);
        with_ui(|u| u.schedule());
    }
}

fn window_size(dpi: u32, w: f32, h: f32) -> (i32, i32) {
    let s = dpi as f32 / 96.0;
    let mut rc = RECT { left: 0, top: 0, right: (w * s) as i32, bottom: (h * s) as i32 };
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut rc, STYLE, false, WINDOW_EX_STYLE(0), dpi);
    }
    (rc.right - rc.left, rc.bottom - rc.top)
}

// ---------------------------------------------------------------- layout

struct RowL {
    item: usize,
    card: RectF,
    ctl: RectF,
    segs: Vec<RectF>,
}

struct Layout {
    nav: Vec<RectF>,
    x0: f32,
    x1: f32,
    title_y: f32,
    hero: Option<RectF>,
    sections: Vec<(usize, f32)>,
    rows: Vec<RowL>,
    content_h: f32,
}

impl Ui {
    fn client(&self) -> (f32, f32) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        (rc.right as f32, rc.bottom as f32)
    }

    fn cfg(&self) -> Config {
        self.shared.config()
    }

    fn colors(&mut self, palette: Palette) -> Colors {
        if self.colors.as_ref().map(|c| c.0) != Some(palette) {
            self.colors = Some((palette, system::colors(palette)));
        }
        self.colors.as_ref().map(|c| c.1.clone()).unwrap_or_else(|| system::colors(palette))
    }

    fn preview_media(&self) -> MediaInfo {
        let m = self.shared.media.lock().unwrap().clone();
        if m.present() { m } else { ui::sample_media() }
    }

    fn hero_height(&self, page: &Page, cfg: &Config) -> f32 {
        let s = self.scale;
        match page.preview {
            Preview::Widget | Preview::Visualizer => 168.0 * s,
            Preview::Card => ui::CLayout::build(s, cfg, true).h + 8.0 * s,
            Preview::UpNext => ui::upnext_size(s).1 + 24.0 * s,
            Preview::LockKeys => ui::osd_size(s, cfg).1 + 40.0 * s,
            Preview::None => 0.0,
        }
    }

    /// Rolagem em pixels inteiros (a camada de conteúdo é copiada sem reamostrar).
    fn scroll_px(&self) -> f32 {
        self.scroll.round()
    }

    fn layout(&mut self, w: f32) -> Layout {
        let sc = self.scroll_px();
        self.layout_at(w, sc, 0.0)
    }

    /// `scroll`: deslocamento vertical; `ox`: origem horizontal (para desenhar
    /// a coluna de opções na sua própria camada).
    fn layout_at(&mut self, w: f32, scroll: f32, ox: f32) -> Layout {
        let s = self.scale;
        let cfg = self.cfg();
        let page = &PAGES[self.page];
        let nav = (0..PAGES.len()).map(|i| RectF::new(10.0 * s, (92.0 + i as f32 * 42.0) * s, NAV_W * s - 20.0 * s, 38.0 * s)).collect();
        let left = NAV_W * s + 12.0 * s;
        let right = (w - 32.0 * s).max(left + 200.0 * s);
        let (x0, x1) = (left - ox, right - ox);
        let mut y = 92.0 * s - scroll;
        let mut hero = None;
        let hh = self.hero_height(page, &cfg);
        if hh > 0.0 {
            hero = Some(RectF::new(x0, y, x1 - x0, hh));
            y += hh + 20.0 * s;
        }
        let mut sections = Vec::new();
        let mut rows = Vec::new();
        for (i, item) in page.items.iter().enumerate() {
            match item {
                Section(_) => {
                    y += 8.0 * s;
                    sections.push((i, y));
                    y += 30.0 * s;
                }
                Row { kind, .. } => {
                    let card = RectF::new(x0, y, x1 - x0, 66.0 * s);
                    let right = card.right() - 18.0 * s;
                    let mut segs = Vec::new();
                    let ctl = match kind {
                        Kind::Toggle => RectF::new(right - 40.0 * s, card.cy() - 10.0 * s, 40.0 * s, 20.0 * s),
                        Kind::Choice(_) => RectF::new(right - 250.0 * s, card.cy() - 16.0 * s, 250.0 * s, 32.0 * s),
                        Kind::Slider { .. } => RectF::new(right - 230.0 * s, card.cy() - 16.0 * s, 230.0 * s, 32.0 * s),
                        Kind::Button(_) => RectF::new(right - 150.0 * s, card.cy() - 16.0 * s, 150.0 * s, 32.0 * s),
                        Kind::Segmented(opts) => {
                            let widths: Vec<f32> = opts
                                .iter()
                                .map(|o| (self.text.mask(tr(o), Font::BodyStrong, (300.0 * s) as i32, (20.0 * s) as i32, Align::Left).used_width() + 28.0 * s).max(78.0 * s))
                                .collect();
                            let total: f32 = widths.iter().sum::<f32>() + 8.0 * s;
                            let r = RectF::new(right - total, card.cy() - 17.0 * s, total, 34.0 * s);
                            let mut x = r.x + 4.0 * s;
                            for wd in widths {
                                segs.push(RectF::new(x, r.y + 4.0 * s, wd, r.h - 8.0 * s));
                                x += wd;
                            }
                            r
                        }
                        Kind::Info => RectF::default(),
                    };
                    rows.push(RowL { item: i, card, ctl, segs });
                    y += 70.0 * s;
                }
            }
        }
        let content_h = y + scroll + 24.0 * s;
        Layout { nav, x0, x1, title_y: 26.0 * s - scroll, hero, sections, rows, content_h }
    }

    fn max_scroll(&mut self) -> f32 {
        let (w, h) = self.client();
        let l = self.layout_at(w, 0.0, 0.0);
        (l.content_h - h).max(0.0)
    }

    fn hit(&mut self, x: f32, y: f32) -> Option<Hit> {
        let (w, _) = self.client();
        let l = self.layout(w);
        if let Some(i) = l.nav.iter().position(|r| r.contains(x, y)) {
            return Some(Hit::Nav(i));
        }
        if x < NAV_W * self.scale {
            return None;
        }
        for (ri, r) in l.rows.iter().enumerate() {
            let Row { kind, .. } = &PAGES[self.page].items[r.item] else { continue };
            match kind {
                Kind::Segmented(_) => {
                    if let Some(k) = r.segs.iter().position(|sr| sr.contains(x, y)) {
                        return Some(Hit::Seg(ri, k));
                    }
                }
                Kind::Toggle => {
                    if r.card.contains(x, y) {
                        return Some(Hit::Row(ri));
                    }
                }
                Kind::Choice(_) | Kind::Slider { .. } | Kind::Button(_) => {
                    if r.ctl.contains(x, y) {
                        return Some(Hit::Row(ri));
                    }
                }
                Kind::Info => {}
            }
        }
        None
    }

    fn row(&self, ri: usize) -> Option<(Key, Kind)> {
        PAGES[self.page].items.iter().filter_map(|i| if let Row { key, kind, .. } = i { Some((*key, *kind)) } else { None }).nth(ri)
    }

    fn slider_value(&mut self, ri: usize, x: f32) -> Option<i32> {
        let (_, Kind::Slider { min, max, step, .. }) = self.row(ri)? else { return None };
        let (w, _) = self.client();
        let ctl = self.layout(w).rows.get(ri)?.ctl;
        let (x0, x1) = (ctl.x + 10.0 * self.scale, ctl.right() - 10.0 * self.scale);
        let t = ((x - x0) / (x1 - x0)).clamp(0.0, 1.0);
        let raw = min as f32 + t * (max - min) as f32;
        Some(((raw / step as f32).round() as i32 * step).clamp(min, max))
    }

    /// A prévia do topo está (parcialmente) na tela?
    fn hero_visible(&self) -> bool {
        let page = &PAGES[self.page];
        if page.preview == Preview::None {
            return false;
        }
        let cfg = self.cfg();
        let top = 92.0 * self.scale - self.scroll;
        top + self.hero_height(page, &cfg) > 0.0
    }

    /// Liga o timer só quando precisa: ~60 fps rolando, ~30 fps com prévia
    /// animada visível, parado no resto (janela parada = 0% de CPU).
    fn schedule(&mut self) {
        let minimized = unsafe { IsIconic(self.hwnd).as_bool() };
        let want = if minimized {
            0
        } else if (self.scroll_target - self.scroll).abs() > 0.5 {
            15
        } else if self.hero_visible() {
            33
        } else {
            0
        };
        if want != self.timer_ms {
            unsafe {
                if want == 0 {
                    let _ = KillTimer(Some(self.hwnd), ANIM_TIMER);
                } else {
                    SetTimer(Some(self.hwnd), ANIM_TIMER, want, None);
                }
            }
            if self.timer_ms == 0 {
                self.last_tick = std::time::Instant::now();
            }
            self.timer_ms = want;
        }
    }
}

// ---------------------------------------------------------------- desenho

impl Ui {
    fn fg(&self) -> Rgb {
        if self.dark { [1.0; 3] } else { [0.0; 3] }
    }

    fn on_accent(&self) -> Rgb {
        if self.dark { [0.0; 3] } else { [1.0; 3] }
    }

    /// Fundo sólido (sem Mica), já como pixel pré-multiplicado opaco.
    fn base_px(&self) -> u32 {
        if self.dark { 0xFF20_2020 } else { 0xFFF3_F3F3 }
    }

    fn label(&mut self, cv: &mut Canvas, t: &str, font: Font, x: f32, y: f32, w: f32, h: f32, c: Rgb, a: f32) {
        ui::text(cv, &mut self.text, t, font, x, y, w, h, c, a);
    }

    fn label_right(&mut self, cv: &mut Canvas, t: &str, font: Font, right: f32, cy: f32, h: f32, c: Rgb, a: f32) {
        ui::text_right(cv, &mut self.text, t, font, right, cy - h / 2.0, h, c, a, self.scale);
    }

    fn paint(&mut self, cv: &mut Canvas, w: f32, h: f32) {
        let s = self.scale;
        let fg = self.fg();
        let accent = self.accent;
        // Só a área suja (ex.: a prévia animada) é redesenhada e copiada para a janela.
        let dirty = cv.clip_rect();
        cv.fill_clip(if self.mica { 0 } else { self.base_px() });
        let cfg = self.cfg();
        let l = self.layout(w);

        // ---- menu lateral (fixo)
        draw_logo(cv, RectF::new(22.0 * s, 24.0 * s, 40.0 * s, 40.0 * s));
        self.label(cv, "Taskbeat", Font::Display, 74.0 * s, 22.0 * s, 180.0 * s, 26.0 * s, fg, 1.0);
        self.label(cv, tr("Configurações"), Font::Caption, 75.0 * s, 46.0 * s, 180.0 * s, 18.0 * s, fg, 0.6);
        for (i, page) in PAGES.iter().enumerate() {
            let r = l.nav[i];
            let selected = i == self.page;
            let hovered = self.hover == Some(Hit::Nav(i));
            let a = if selected { if self.dark { 0.065 } else { 0.05 } } else if hovered { if self.dark { 0.04 } else { 0.03 } } else { 0.0 };
            if a > 0.0 {
                cv.fill_round(r, 6.0 * s, fg, a);
            }
            if selected {
                cv.fill_round(RectF::new(r.x, r.cy() - 8.0 * s, 3.0 * s, 16.0 * s), 1.5 * s, accent, 1.0);
            }
            ui::glyph(cv, &mut self.text, RectF::new(r.x + 12.0 * s, r.y, 24.0 * s, r.h), page.icon, Font::IconMd, if selected { accent } else { fg }, 0.95);
            self.label(cv, tr(page.name), Font::Body, r.x + 46.0 * s, r.y, r.w - 50.0 * s, r.h, fg, if selected { 1.0 } else { 0.85 });
        }
        self.label(cv, tr("v1.0 · feito em Rust"), Font::Caption, 26.0 * s, h - 34.0 * s, 200.0 * s, 18.0 * s, fg, 0.45);

        // ---- conteúdo: camada pronta, só copiada na posição da rolagem
        let ox = (NAV_W * s).round();
        self.ensure_content(w, h, ox);
        if let Some(c) = &self.content {
            cv.copy_from(ox as i32, 0, &c.px, c.w, c.h, self.scroll_px() as i32);
        }

        // ---- prévia animada por cima (recortada à direita do menu)
        cv.set_clip(intersect(dirty, RectF::new(ox, 0.0, w - ox, h)));
        if let Some(hero) = l.hero {
            if hero.bottom() > 0.0 && hero.y < h {
                let preview = PAGES[self.page].preview;
                self.draw_hero(cv, hero, preview, &cfg);
            }
        }

        // Barra de rolagem fina (só quando há o que rolar).
        let max = (l.content_h - h).max(0.0);
        if max > 1.0 {
            let track_h = h - 16.0 * s;
            let bar_h = (track_h * h / l.content_h).max(30.0 * s);
            let by = 8.0 * s + (track_h - bar_h) * (self.scroll / max).clamp(0.0, 1.0);
            cv.fill_round(RectF::new(w - 8.0 * s, by, 3.0 * s, bar_h), 1.5 * s, fg, 0.28);
        }
        cv.set_clip(dirty);
    }

    /// Retângulo da prévia na janela (para redesenhar só ela a cada quadro da animação).
    fn hero_rect(&mut self) -> Option<RECT> {
        let (w, h) = self.client();
        let r = self.layout(w).hero?;
        let top = r.y.max(0.0);
        let bottom = r.bottom().min(h);
        (bottom > top).then(|| RECT { left: r.x.floor() as i32, top: top.floor() as i32, right: r.right().ceil() as i32 + 1, bottom: bottom.ceil() as i32 + 1 })
    }

    fn content_key(&self, w: f32, h: f32) -> ContentKey {
        let hover = match self.hover {
            Some(Hit::Nav(_)) => None,
            h => h,
        };
        ContentKey {
            page: self.page,
            w: w as i32,
            h: h as i32,
            scale: self.scale.to_bits(),
            dark: self.dark,
            mica: self.mica,
            accent: self.accent.map(f32::to_bits),
            cfg: self.shared.config_version.load(std::sync::atomic::Ordering::Relaxed),
            hover,
            drag: self.drag,
            autostart: AUTOSTART.with(|a| a.get()),
        }
    }

    /// Redesenha a coluna de opções inteira (sem rolagem) só quando algo mudou.
    fn ensure_content(&mut self, w: f32, h: f32, ox: f32) {
        let key = self.content_key(w, h);
        if self.content.as_ref().is_some_and(|c| c.key == key) {
            return;
        }
        let l = self.layout_at(w, 0.0, ox);
        let cw = (w - ox).max(1.0) as usize;
        let ch = l.content_h.max(h).ceil().max(1.0) as usize;
        let mut px = self.content.take().map(|c| c.px).unwrap_or_default();
        px.clear();
        px.resize(cw * ch, if self.mica { 0 } else { self.base_px() });
        {
            let mut cv = Canvas::new(cw, ch, &mut px);
            self.draw_content(&mut cv, &l);
        }
        self.content = Some(Cached { key, w: cw, h: ch, px });
    }

    fn draw_content(&mut self, cv: &mut Canvas, l: &Layout) {
        let s = self.scale;
        let fg = self.fg();
        let cfg = self.cfg();
        let page = &PAGES[self.page];
        let x0 = l.x0;
        let tw = l.x1 - x0;
        self.label(cv, tr(page.name), Font::Heading, x0, l.title_y, tw, 40.0 * s, fg, 1.0);
        self.label(cv, tr(page.subtitle), Font::Body, x0 + 1.0 * s, l.title_y + 40.0 * s, tw, 22.0 * s, fg, 0.62);

        for (i, y) in &l.sections {
            if let Section(name) = page.items[*i] {
                self.label(cv, tr(name), Font::BodyStrong, x0 + 2.0 * s, *y, 400.0 * s, 24.0 * s, fg, 1.0);
            }
        }

        let card_a = if self.dark { 0.05 } else { 0.70 };
        let border_a = if self.dark { 0.20 } else { 0.06 };
        for (ri, r) in l.rows.iter().enumerate() {
            let Row { icon, title, desc, key, kind } = &page.items[r.item] else { continue };
            let hovered = self.hover == Some(Hit::Row(ri)) && matches!(kind, Kind::Toggle);
            cv.fill_round(r.card, 7.0 * s, [1.0; 3], card_a + if hovered { 0.025 } else { 0.0 });
            cv.stroke_round(r.card, 7.0 * s, s, [0.0; 3], border_a);

            ui::glyph(cv, &mut self.text, RectF::new(r.card.x + 18.0 * s, r.card.y, 26.0 * s, r.card.h), icon, Font::IconMd, fg, 0.92);
            let tx = r.card.x + 60.0 * s;
            let tw = if matches!(kind, Kind::Info) { r.card.right() - tx - 20.0 * s } else { r.ctl.x - tx - 24.0 * s };
            self.label(cv, tr(title), Font::Body, tx, r.card.y + 12.0 * s, tw, 21.0 * s, fg, 1.0);
            self.label(cv, tr(desc), Font::Caption, tx, r.card.y + 34.0 * s, tw, 19.0 * s, fg, 0.62);

            let v = get(&cfg, *key);
            match kind {
                Kind::Toggle => self.draw_toggle(cv, r.ctl, v != 0, hovered),
                Kind::Segmented(opts) => self.draw_segmented(cv, r, ri, opts, v as usize),
                Kind::Choice(names) => {
                    let hot = self.hover == Some(Hit::Row(ri));
                    self.draw_box(cv, r.ctl, hot);
                    let name = tr(names.get(v.max(0) as usize).copied().unwrap_or(""));
                    self.label(cv, name, Font::Body, r.ctl.x + 12.0 * s, r.ctl.y, r.ctl.w - 44.0 * s, r.ctl.h, fg, 1.0);
                    ui::glyph(cv, &mut self.text, RectF::new(r.ctl.right() - 32.0 * s, r.ctl.y, 24.0 * s, r.ctl.h), "\u{E70D}", Font::IconSmall, fg, 0.8);
                }
                Kind::Slider { min, max, .. } => {
                    let hot = self.hover == Some(Hit::Row(ri)) || self.drag == Some(ri);
                    self.draw_slider(cv, r.ctl, (v - min) as f32 / (max - min) as f32, hot);
                    self.label_right(cv, &value_label(*kind, *key, v), Font::Caption, r.ctl.x - 12.0 * s, r.ctl.cy(), 20.0 * s, fg, 0.75);
                }
                Kind::Button(t) => {
                    let hot = self.hover == Some(Hit::Row(ri));
                    self.draw_box(cv, r.ctl, hot);
                    ui::text_center(cv, &mut self.text, tr(t), Font::Body, r.ctl, fg, 1.0);
                }
                Kind::Info => {}
            }
        }
    }

    /// Fundo fixo da prévia (papel de parede e, no widget, a barra simulada).
    fn hero_static(&mut self, cv: &mut Canvas, r: RectF, preview: Preview, light_tb: bool) {
        let s = self.scale;
        let radius = 10.0 * s;
        let dark = self.dark;
        let (y0, x0) = (r.y, r.x);
        cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), radius, |x, y| {
            let u = ((x - x0) / r.w).clamp(0.0, 1.0);
            let v = ((y - y0) / r.h).clamp(0.0, 1.0);
            let a = if dark { [0.05, 0.09, 0.26] } else { [0.62, 0.76, 0.98] };
            let b = if dark { [0.22, 0.12, 0.42] } else { [0.85, 0.80, 0.98] };
            let c = lerp3(a, b, (u * 0.7 + v * 0.3).clamp(0.0, 1.0));
            let glow = (-(((u - 0.72) * 2.2).powi(2) + ((v - 0.35) * 2.6).powi(2))).exp();
            (lerp3(c, if dark { [0.30, 0.45, 0.95] } else { [1.0; 3] }, glow * 0.55), 1.0)
        });
        cv.stroke_round(r, radius, s, [0.0; 3], if dark { 0.3 } else { 0.08 });
        self.label(cv, tr("PRÉ-VISUALIZAÇÃO"), Font::Label, r.x + 16.0 * s, r.y + 12.0 * s, 200.0 * s, 16.0 * s, [1.0; 3], 0.75);

        if matches!(preview, Preview::Widget | Preview::Visualizer) {
            let bar = RectF::new(r.x, r.bottom() - 48.0 * s, r.w, 48.0 * s);
            let bar_c: Rgb = if light_tb { [0.93, 0.93, 0.94] } else { [0.11, 0.11, 0.12] };
            cv.fill_rrect_ca(bar.x, bar.y, bar.right(), bar.bottom(), radius, |_, y| (bar_c, if y < bar.y + radius { 0.0 } else { 0.94 }));
            cv.fill_rrect(bar.x, bar.y, bar.right(), bar.y + radius, 0.0, 0.94, |_, _| bar_c);
            cv.fill_rrect(bar.x, bar.y, bar.right(), bar.y + s, 0.0, 0.12, |_, _| if light_tb { [0.0; 3] } else { [1.0; 3] });
            let icon_c: Rgb = if light_tb { [0.2; 3] } else { [0.85; 3] };
            for k in 0..5 {
                let ix = bar.x + 24.0 * s + k as f32 * 40.0 * s;
                cv.fill_round(RectF::new(ix, bar.cy() - 11.0 * s, 22.0 * s, 22.0 * s), 5.0 * s, icon_c, if k == 0 { 0.9 } else { 0.35 });
            }
            self.label(cv, "12:34", Font::Caption, bar.right() - 56.0 * s, bar.y + 6.0 * s, 50.0 * s, 18.0 * s, icon_c, 0.9);
            self.label(cv, "23/11", Font::Caption, bar.right() - 56.0 * s, bar.y + 24.0 * s, 50.0 * s, 18.0 * s, icon_c, 0.7);
        }
    }

    /// Prévia ao vivo: fundo em cache + componente animado por cima.
    fn draw_hero(&mut self, cv: &mut Canvas, r: RectF, preview: Preview, cfg: &Config) {
        let s = self.scale;
        let r = RectF::new(r.x.round(), r.y.round(), r.w.round(), r.h.round());
        let t = self.t0.elapsed().as_secs_f32();
        let mut colors = self.colors(cfg.palette);

        let key = HeroKey { preview, w: r.w as i32, h: r.h as i32, scale: s.to_bits(), dark: self.dark, mica: self.mica, light_tb: colors.light_taskbar };
        if !self.hero_bg.as_ref().is_some_and(|c| c.key == key) {
            let (hw, hh) = (r.w.max(1.0) as usize, r.h.max(1.0) as usize);
            let mut px = vec![if self.mica { 0 } else { self.base_px() }; hw * hh];
            {
                let mut hc = Canvas::new(hw, hh, &mut px);
                self.hero_static(&mut hc, RectF::new(0.0, 0.0, r.w, r.h), preview, colors.light_taskbar);
            }
            self.hero_bg = Some(Cached { key, w: hw, h: hh, px });
        }
        if let Some(c) = &self.hero_bg {
            cv.copy_from(r.x as i32, r.y as i32, &c.px, c.w, c.h, 0);
        }

        let mut media = self.preview_media();
        let live = media.pos_at.is_some();
        if !live {
            media.pos = (110.0 + t as f64) % media.dur.max(1.0);
        }
        colors.album = media.art.as_ref().map(|a| a.vibrant);
        let pb = Playback {
            playing: media.playing || !live,
            shuffle: media.shuffle,
            repeat: media.repeat,
            pos: if live { media.position_now() } else { media.pos },
            seek_preview: None,
            volume: 0.62,
            muted: false,
        };
        match preview {
            Preview::Widget | Preview::Visualizer => {
                let mut c2 = cfg.clone();
                if preview == Preview::Visualizer {
                    c2.show_visualizer = true;
                }
                let bar = RectF::new(r.x, r.bottom() - 48.0 * s, r.w, 48.0 * s);
                let wh = 40.0 * s;
                let l = ui::WLayout::build(s, wh, &c2, c2.show_media, c2.show_visualizer);
                let wx = (bar.right() - 76.0 * s - l.width).max(bar.x + 8.0 * s);
                let wy = bar.y + 4.0 * s;
                let values: Vec<f32> = (0..c2.bars).map(|i| (0.5 + 0.45 * ((t * 5.0 + i as f32 * 0.9).sin() * (t * 2.3 + i as f32 * 0.4).cos())).clamp(0.05, 1.0)).collect();
                let theme = ui::widget_theme(&c2, &colors, &media);
                let mut sub = SubCanvas::new(wx, wy, l.width, wh);
                let f = ui::WidgetFrame {
                    cfg: &c2,
                    media: &media,
                    theme,
                    colors: &colors,
                    it: Interact::default(),
                    pb,
                    time: t,
                    viz: Some((&values, &values)),
                    flyout_open: false,
                    s,
                };
                ui::draw_widget(&mut sub.canvas(), &mut self.text, &l, &f);
                sub.commit(cv);
            }
            Preview::Card => {
                let l = ui::CLayout::build(s, cfg, true);
                let th = ui::card_theme(cfg, &colors, &media);
                let mut m2 = media.clone();
                m2.dur = m2.dur.max(1.0);
                let mut sub = SubCanvas::new(r.cx() - l.w / 2.0, r.y + 4.0 * s, l.w, l.h);
                let f = ui::CardFrame { cfg, colors: &colors, media: &m2, theme: th, it: Interact::default(), pb, s };
                ui::draw_card(&mut sub.canvas(), &mut self.text, &l, &f);
                sub.commit(cv);
            }
            Preview::UpNext => {
                let (uw, uh) = ui::upnext_size(s);
                let total = cfg.popup_secs as f32 + 0.5;
                let tt = (t % (total + 1.2)).min(total - 0.6);
                let th = ui::card_theme(cfg, &colors, &media);
                let mut sub = SubCanvas::new(r.cx() - uw / 2.0, r.y + 12.0 * s, uw, uh);
                ui::draw_upnext(&mut sub.canvas(), &mut self.text, cfg, &colors, &media, &th, s, tt, total);
                sub.commit(cv);
            }
            Preview::LockKeys => {
                let (ow, oh) = ui::osd_size(s, cfg);
                let cycle = t % 4.0;
                let on = cycle < 2.0 || !cfg.osd_show_off;
                let tt = cycle % 2.0;
                let th = ui::osd_theme(cfg, &colors);
                let mut sub = SubCanvas::new(r.cx() - ow / 2.0, r.y + 22.0 * s, ow, oh);
                ui::draw_osd(&mut sub.canvas(), &mut self.text, cfg, &th, s, tr("Caps Lock"), "A", on, tt);
                sub.commit(cv);
            }
            Preview::None => {}
        }
    }

    fn draw_box(&mut self, cv: &mut Canvas, r: RectF, hot: bool) {
        let s = self.scale;
        let a = if self.dark { if hot { 0.084 } else { 0.061 } } else if hot { 0.5 } else { 0.7 };
        cv.fill_round(r, 5.0 * s, [1.0; 3], a);
        cv.stroke_round(r, 5.0 * s, s, if self.dark { [1.0; 3] } else { [0.0; 3] }, if self.dark { 0.07 } else { 0.09 });
    }

    /// Controle segmentado (estilo SelectorBar do WinUI): o item escolhido vira uma "pílula" elevada.
    fn draw_segmented(&mut self, cv: &mut Canvas, r: &RowL, ri: usize, opts: &[&'static str], sel: usize) {
        let s = self.scale;
        let fg = self.fg();
        let edge: Rgb = if self.dark { [1.0; 3] } else { [0.0; 3] };
        cv.fill_round(r.ctl, 8.0 * s, edge, 0.045);
        cv.stroke_round(r.ctl, 8.0 * s, s, edge, 0.06);
        for (k, seg) in r.segs.iter().enumerate() {
            let hovered = self.hover == Some(Hit::Seg(ri, k));
            if k == sel {
                cv.drop_shadow(*seg, 6.0 * s, 4.0 * s, 1.0 * s, if self.dark { 0.35 } else { 0.12 });
                cv.fill_round(*seg, 6.0 * s, if self.dark { [0.24, 0.24, 0.26] } else { [1.0; 3] }, 1.0);
                cv.stroke_round(*seg, 6.0 * s, s, edge, 0.08);
                cv.fill_round(RectF::new(seg.cx() - 8.0 * s, seg.bottom() - 3.5 * s, 16.0 * s, 3.0 * s), 1.5 * s, self.accent, 1.0);
            } else if hovered {
                cv.fill_round(*seg, 6.0 * s, fg, 0.05);
            }
            let font = if k == sel { Font::BodyStrong } else { Font::Body };
            ui::text_center(cv, &mut self.text, tr(opts[k]), font, seg.offset(0.0, -1.0 * s), fg, if k == sel { 1.0 } else { 0.78 });
        }
    }

    /// Toggle switch do WinUI: trilho 40×20; ligado = cor de destaque.
    fn draw_toggle(&mut self, cv: &mut Canvas, r: RectF, on: bool, hot: bool) {
        let s = self.scale;
        let fg = self.fg();
        let knob = if hot { 7.0 * s } else { 6.0 * s };
        if on {
            cv.fill_round(r, r.h / 2.0, self.accent, if hot { 0.9 } else { 1.0 });
            cv.fill_circle(r.right() - 10.0 * s, r.cy(), knob, self.on_accent(), 1.0);
        } else {
            if hot {
                cv.fill_round(r, r.h / 2.0, fg, 0.05);
            }
            cv.stroke_round(r, r.h / 2.0, s, fg, 0.6);
            cv.fill_circle(r.x + 10.0 * s, r.cy(), knob, fg, 0.8);
        }
        let t = tr(if on { "Ligado" } else { "Desligado" });
        self.label_right(cv, t, Font::Body, r.x - 12.0 * s, r.cy(), 22.0 * s, fg, 0.9);
    }

    fn draw_slider(&mut self, cv: &mut Canvas, r: RectF, t: f32, hot: bool) {
        let s = self.scale;
        let fg = self.fg();
        let (x0, x1, cy) = (r.x + 10.0 * s, r.right() - 10.0 * s, r.cy());
        let th = 4.0 * s;
        let xv = x0 + (x1 - x0) * t.clamp(0.0, 1.0);
        cv.fill_rrect(x0, cy - th / 2.0, x1, cy + th / 2.0, th / 2.0, if self.dark { 0.28 } else { 0.30 }, |_, _| fg);
        if xv > x0 {
            cv.fill_rrect(x0, cy - th / 2.0, xv, cy + th / 2.0, th / 2.0, 1.0, |_, _| self.accent);
        }
        let ring: Rgb = if self.dark { [0.27; 3] } else { [1.0; 3] };
        cv.drop_shadow(RectF::new(xv - 10.0 * s, cy - 10.0 * s, 20.0 * s, 20.0 * s), 10.0 * s, 4.0 * s, 1.0 * s, 0.25);
        cv.fill_circle(xv, cy, 10.0 * s, ring, 1.0);
        cv.fill_circle(xv, cy, if hot { 7.0 * s } else { 5.0 * s }, self.accent, 1.0);
    }
}

/// Logo do app: "squircle" em gradiente (roxo → rosa → laranja) com barras brancas.
pub fn draw_logo(cv: &mut Canvas, r: RectF) {
    let (x0, y0, w) = (r.x, r.y, r.w);
    cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), w * 0.26, |x, y| {
        let t = (((x - x0) + (y - y0)) / (2.0 * w)).clamp(0.0, 1.0);
        let c = if t < 0.5 { lerp3([0.42, 0.25, 0.95], [0.93, 0.27, 0.62], t * 2.0) } else { lerp3([0.93, 0.27, 0.62], [1.0, 0.62, 0.28], (t - 0.5) * 2.0) };
        (c, 1.0)
    });
    let bw = w * 0.11;
    let gap = w * 0.06;
    let heights = [0.34f32, 0.58, 0.42, 0.24];
    let total = 4.0 * bw + 3.0 * gap;
    let bx = r.cx() - total / 2.0;
    let base = r.y + w * 0.70;
    for (i, hgt) in heights.iter().enumerate() {
        let x = bx + i as f32 * (bw + gap);
        cv.fill_rrect(x, base - w * hgt, x + bw, base, bw / 2.0, 1.0, |_, _| [1.0; 3]);
    }
    // "barra de tarefas" sob as barras
    cv.fill_rrect(r.x + w * 0.2, r.y + w * 0.77, r.right() - w * 0.2, r.y + w * 0.83, w * 0.03, 0.9, |_, _| [1.0; 3]);
}

/// Canvas temporário para desenhar um componente e compor sobre o principal.
struct SubCanvas {
    x: f32,
    y: f32,
    w: usize,
    h: usize,
    px: Vec<u32>,
}

impl SubCanvas {
    fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        let (w, h) = (w.ceil().max(1.0) as usize, h.ceil().max(1.0) as usize);
        Self { x: x.round(), y: y.round(), w, h, px: vec![0; w * h] }
    }

    fn canvas(&mut self) -> Canvas<'_> {
        Canvas::new(self.w, self.h, &mut self.px)
    }

    fn commit(self, cv: &mut Canvas) {
        let img = crate::render::Image::plain(self.w, self.h, self.px);
        cv.draw_image(RectF::new(self.x, self.y, self.w as f32, self.h as f32), &img, 0.0, 1.0);
    }
}

// ---------------------------------------------------------------- interação

enum Action {
    None,
    Redraw,
    Menu { ri: usize, names: &'static [&'static str], current: i32, at: POINT },
}

fn activate(u: &mut Ui, hit: Hit) -> Action {
    match hit {
        Hit::Nav(i) => {
            if u.page != i {
                u.page = i;
                u.scroll = 0.0;
                u.scroll_target = 0.0;
                u.hero_bg = None;
            }
            u.hover = None;
            u.schedule();
            Action::Redraw
        }
        Hit::Seg(ri, k) => {
            if let Some((key, _)) = u.row(ri) {
                u.shared.update_config(|c| set(c, key, k as i32));
                if key == Key::OsdPos {
                    u.shared.osd_test.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            Action::Redraw
        }
        Hit::Row(ri) => {
            let Some((key, kind)) = u.row(ri) else { return Action::None };
            let cfg = u.cfg();
            match kind {
                Kind::Toggle => {
                    let v = get(&cfg, key);
                    if key == Key::Autostart {
                        system::set_autostart(v == 0);
                        AUTOSTART.with(|a| a.set(system::autostart_enabled()));
                    } else {
                        u.shared.update_config(|c| set(c, key, 1 - v));
                    }
                    Action::Redraw
                }
                Kind::Choice(names) => {
                    let (w, _) = u.client();
                    let ctl = u.layout(w).rows[ri].ctl;
                    let mut pt = POINT { x: ctl.x as i32, y: (ctl.bottom() + 2.0 * u.scale) as i32 };
                    unsafe {
                        let _ = ClientToScreen(u.hwnd, &mut pt);
                    }
                    Action::Menu { ri, names, current: get(&cfg, key), at: pt }
                }
                Kind::Button(_) if key == Key::Reset => {
                    u.shared.update_config(|c| *c = Config::default());
                    crate::i18n::apply(Language::Auto);
                    Action::Redraw
                }
                Kind::Button(_) if key == Key::OsdTest => {
                    u.shared.osd_test.store(true, std::sync::atomic::Ordering::Relaxed);
                    Action::Redraw
                }
                _ => Action::None,
            }
        }
    }
}

fn xy(lp: LPARAM) -> (f32, f32) {
    ((lp.0 & 0xffff) as i16 as f32, ((lp.0 >> 16) & 0xffff) as i16 as f32)
}

fn intersect(a: RectF, b: RectF) -> RectF {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = a.right().min(b.right());
    let y1 = a.bottom().min(b.bottom());
    RectF::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// Valor de slider em pré-visualização (sem gravar); move o aviso ao vivo nos ajustes de posição.
fn preview_slider(u: &mut Ui, key: Key, v: i32) {
    u.shared.preview_config(|c| set(c, key, v));
    if matches!(key, Key::OsdOffX | Key::OsdOffY) {
        u.shared.osd_test.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                with_ui(|u| {
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let (w, h) = (rc.right.max(1), rc.bottom.max(1));
                    Surface::ensure(&mut u.surface, w, h);
                    if let Some(mut surf) = u.surface.take() {
                        let d = ps.rcPaint;
                        let (dx, dy) = (d.left.clamp(0, w), d.top.clamp(0, h));
                        let (dw, dh) = (d.right.clamp(0, w) - dx, d.bottom.clamp(0, h) - dy);
                        {
                            let mut cv = surf.canvas();
                            cv.set_clip(RectF::new(dx as f32, dy as f32, dw as f32, dh as f32));
                            u.paint(&mut cv, w as f32, h as f32);
                        }
                        let _ = BitBlt(hdc, dx, dy, dw, dh, Some(surf.dc), dx, dy, SRCCOPY);
                        u.surface = Some(surf);
                    }
                });
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_TIMER if wp.0 == ANIM_TIMER => {
                with_ui(|u| {
                    let now = std::time::Instant::now();
                    let dt = (now - u.last_tick).as_secs_f32().clamp(0.0, 0.05);
                    u.last_tick = now;
                    // Rolagem suave independente da taxa de quadros.
                    let d = u.scroll_target - u.scroll;
                    let scrolling = d.abs() >= 0.5;
                    u.scroll = if !scrolling { u.scroll_target } else { u.scroll + d * (1.0 - (-dt * 16.0).exp()) };
                    u.schedule();
                    // Rolando: janela toda. Parado: só a prévia animada.
                    match (scrolling, u.hero_rect()) {
                        (false, Some(rc)) => {
                            let _ = InvalidateRect(Some(hwnd), Some(&rc), false);
                        }
                        _ => invalidate(hwnd),
                    }
                });
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let changed = with_ui(|u| {
                    if !u.tracking {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut tme);
                        u.tracking = true;
                    }
                    if let Some(ri) = u.drag {
                        if let (Some(v), Some((key, _))) = (u.slider_value(ri, x), u.row(ri)) {
                            if get(&u.cfg(), key) != v {
                                preview_slider(u, key, v);
                                return true;
                            }
                        }
                        return false;
                    }
                    let h = u.hit(x, y);
                    let changed = h != u.hover;
                    u.hover = h;
                    changed
                })
                .unwrap_or(false);
                if changed {
                    invalidate(hwnd);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                with_ui(|u| {
                    u.tracking = false;
                    if u.drag.is_none() {
                        u.hover = None;
                    }
                });
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                SetCapture(hwnd);
                with_ui(|u| {
                    let h = u.hit(x, y);
                    u.pressed = h;
                    if let Some(Hit::Row(ri)) = h {
                        if let Some((key, Kind::Slider { .. })) = u.row(ri) {
                            u.drag = Some(ri);
                            if let Some(v) = u.slider_value(ri, x) {
                                preview_slider(u, key, v);
                            }
                        }
                    }
                });
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                let (x, y) = xy(lp);
                let action = with_ui(|u| {
                    if u.drag.take().is_some() {
                        u.shared.save_config();
                        u.pressed = None;
                        return Action::Redraw;
                    }
                    let h = u.hit(x, y);
                    let pressed = u.pressed.take();
                    match h {
                        Some(h) if Some(h) == pressed => activate(u, h),
                        _ => Action::Redraw,
                    }
                })
                .unwrap_or(Action::None);
                // O menu roda um loop modal: nada de `borrow` aberto aqui.
                if let Action::Menu { ri, names, current, at } = action {
                    if let Ok(menu) = CreatePopupMenu() {
                        for (k, name) in names.iter().enumerate() {
                            let flags = MF_STRING | if k as i32 == current { MF_CHECKED } else { MF_UNCHECKED };
                            let _ = AppendMenuW(menu, flags, k + 1, &HSTRING::from(tr(name)));
                        }
                        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, at.x, at.y, None, hwnd, None);
                        let _ = DestroyMenu(menu);
                        if cmd.0 > 0 {
                            with_ui(|u| {
                                if let Some((key, _)) = u.row(ri) {
                                    u.shared.update_config(|c| set(c, key, cmd.0 - 1));
                                    if key == Key::Language {
                                        let _ = SetWindowTextW(hwnd, &HSTRING::from(tr("Taskbeat — Configurações")));
                                    }
                                }
                            });
                        }
                    }
                }
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32;
                with_ui(|u| {
                    let max = u.max_scroll();
                    u.scroll_target = (u.scroll_target - delta / 120.0 * 110.0 * u.scale).clamp(0.0, max);
                    u.schedule();
                });
                LRESULT(0)
            }
            WM_SIZE => {
                with_ui(|u| {
                    let max = u.max_scroll();
                    u.scroll_target = u.scroll_target.clamp(0.0, max);
                    u.scroll = u.scroll.clamp(0.0, max);
                    u.schedule();
                });
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let dpi = GetDpiForWindow(hwnd).max(96);
                let (mw, mh) = window_size(dpi, MIN_W, MIN_H);
                let mmi = &mut *(lp.0 as *mut MINMAXINFO);
                mmi.ptMinTrackSize = POINT { x: mw, y: mh };
                LRESULT(0)
            }
            WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DWMCOLORIZATIONCOLORCHANGED => {
                with_ui(|u| {
                    u.dark = system::apps_use_dark();
                    u.accent = system::app_accent(u.dark);
                    u.mica = apply_theme(hwnd, u.dark);
                    u.colors = None;
                    u.hero_bg = None;
                    u.content = None;
                });
                invalidate(hwnd);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_DPICHANGED => {
                let dpi = ((wp.0 >> 16) & 0xffff) as u32;
                with_ui(|u| {
                    u.scale = dpi as f32 / 96.0;
                    u.text.set_scale(u.scale);
                });
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), ANIM_TIMER);
                UI.with(|u| {
                    if let Ok(mut g) = u.try_borrow_mut() {
                        g.take();
                    }
                });
                system::trim_memory();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_label_is_translated() {
        let mut missing = Vec::new();
        let mut check = |t: &'static str| {
            if !t.is_empty() && !t.chars().all(|c| c.is_ascii_digit()) && !crate::i18n::has(t) {
                missing.push(t);
            }
        };
        for p in PAGES {
            check(p.name);
            check(p.subtitle);
            for it in p.items {
                match it {
                    Section(n) => check(n),
                    Row { title, desc, kind, .. } => {
                        check(title);
                        check(desc);
                        match kind {
                            Kind::Segmented(o) | Kind::Choice(o) => o.iter().for_each(|t| check(t)),
                            Kind::Button(t) => check(t),
                            _ => {}
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "sem tradução: {missing:#?}");
    }
}
