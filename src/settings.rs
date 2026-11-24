//! Tela de configurações no estilo Fluent 2 / WinUI 3: fundo Mica, menu
//! lateral, prévias ao vivo de cada componente, cartões agrupados com toggle,
//! controle segmentado, lista, slider e botão, e rolagem suave.
//!
//! Tudo é desenhado pelo próprio app (mesmo código do widget, em `ui.rs`) e
//! copiado para a janela com alfa: com o quadro estendido para a área cliente,
//! o DWM mostra o Mica por trás das partes transparentes. Sem Mica (Windows 10)
//! o fundo vira uma cor sólida do tema. Mudanças são aplicadas na hora;
//! sliders só gravam no disco ao soltar.
//!
//! Desempenho: a coluna de opções é uma camada desenhada uma vez e só copiada
//! na posição da rolagem; quando algo muda (passar o mouse, clicar, arrastar)
//! só a linha afetada é redesenhada. A prévia também fica em cache e só é
//! refeita quando o quadro dela muda. As animações seguem o vblank do monitor
//! (`vsync.rs`) e param quando não há nada mexendo (0% de CPU).

use crate::config::*;
use crate::i18n::tr;
use crate::menu;
use crate::overlay::Surface;
use crate::render::{self, Canvas, Colors, Image, RectF, Rgb, lerp3};
use crate::shared::{MediaInfo, Shared};
use crate::system;
use crate::text::{Align, Font, TextRenderer};
use crate::ui::{self, Interact, Playback};
use crate::vsync::{Pacer, Target};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;
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
/// Quadro de animação (postado pela thread do vblank).
const WM_APP_FRAME: u32 = WM_APP + 20;
const WIN_W: f32 = 1060.0;
const WIN_H: f32 = 760.0;
const NAV_W: f32 = 268.0;
const MIN_W: f32 = 820.0;
const MIN_H: f32 = 560.0;
/// Timer lento: prévias que mudam pouco (tempo da música) e reserva do vblank.
const SLOW_TIMER: usize = 1;
/// Pré-carrega, com a janela parada, os textos e prévias das outras páginas
/// (a primeira visita a cada página não engasga a animação).
const WARM_TIMER: usize = 2;
/// Espera antes de mostrar a dica de uma opção desativada.
const TIP_DELAY: f32 = 0.45;

// ---------------------------------------------------------------- modelo

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Key {
    ShowMedia,
    AlwaysShow,
    TaskbarControls,
    ShowArtist,
    SecondLine,
    Indicator,
    TaskbarProgress,
    TrackAnim,
    CoverShape,
    CoverSize,
    TextSize,
    WidgetPill,
    WidgetShape,
    WidgetTint,
    WidgetOpacity,
    WidgetBorder,
    WidgetDivider,
    TaskbarShuffle,
    TaskbarPlay,
    CardShuffle,
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
    OsdMaterial,
    OsdOpacity,
    OsdBlur,
    OsdIcon,
    OsdAnim,
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
    Row { icon: &'static str, title: &'static str, desc: &'static str, key: Key, kind: Kind },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
const CARD_STYLES: &[&str] = &["Capa desfocada", "Cor do álbum", "Acrílico"];
const CARD_LAYOUTS: &[&str] = &["Horizontal", "Vertical"];
const THEMES: &[&str] = &["Automático", "Escuro", "Claro"];
const ACCENTS: &[&str] = &["Cor da capa", "Cor do Windows"];
const CORNERS: &[&str] = &["Pequenos", "Médios", "Grandes"];
const OSD_POSITIONS: &[&str] = &["Embaixo", "Centro", "Topo", "Acima do player"];
const OSD_SHAPES: &[&str] = &["Iguais ao card", "Pílula", "Retos"];
const OSD_SIZES: &[&str] = &["Compacto", "Normal", "Grande"];
const WIDGET_SHAPES: &[&str] = &["Arredondado", "Pílula", "Reto"];
const TASKBAR_PLAYS: &[&str] = &["Preenchido", "Só o ícone"];
const SHUFFLE_MODES: &[&str] = &["Sempre", "Quando o player permite"];
const OSD_MATERIALS: &[&str] = &["Sólido", "Acrílico", "Cor de destaque"];
const OSD_ICONS: &[&str] = &["Letra", "Cadeado", "Teclado"];
const OSD_ANIMS: &[&str] = &["Deslizar", "Zoom", "Esmaecer"];
const VIZ_STYLES: &[&str] = &["Barras", "Espelhado", "Onda"];
const PALETTES: &[&str] = &["Cor da capa", "Cor de destaque do Windows", "Arco-íris", "Neon", "Monocromático"];
const LANGS: &[&str] = &["Automático (idioma do Windows)", "Português (Brasil)", "English", "Español", "Deutsch"];
const LANG_V: &[Language] = &[Language::Auto, Language::Pt, Language::En, Language::Es, Language::De];
const BAR_NAMES: &[&str] = &["6", "8", "10", "12", "16", "24", "32"];
const BARS: &[usize] = &[6, 8, 10, 12, 16, 24, 32];
const SIZES: &[&str] = &["Pequeno", "Normal", "Grande"];
const COVER_SIZES: &[&str] = &["Pequena", "Normal", "Grande"];
const SECOND_LINES: &[&str] = &["Artista", "Artista · Álbum", "Álbum", "Tempo"];
const TINTS: &[&str] = &["Neutro", "Cor da capa", "Cor de destaque"];

const PAGES: &[Page] = &[
    Page {
        name: "Barra de tarefas",
        icon: "\u{E7F4}",
        subtitle: "O widget compacto que fica ao lado do relógio",
        preview: Preview::Widget,
        items: &[
            Section("Conteúdo"),
            Row { icon: "\u{E8D6}", title: "Mostrar música na barra", desc: "Capa, título e artista; clique para abrir o card", key: Key::ShowMedia, kind: Kind::Toggle },
            Row { icon: "\u{E7B3}", title: "Sempre visível", desc: "Fica na barra mesmo sem nada tocando (\"Nada tocando\")", key: Key::AlwaysShow, kind: Kind::Toggle },
            Row { icon: "\u{E77B}", title: "Mostrar artista", desc: "Segunda linha embaixo do título", key: Key::ShowArtist, kind: Kind::Toggle },
            Row { icon: "\u{E8C4}", title: "Segunda linha", desc: "O que aparece embaixo do título", key: Key::SecondLine, kind: Kind::Segmented(SECOND_LINES) },
            Row { icon: "\u{E9E9}", title: "Indicador \"tocando agora\"", desc: "Mini equalizador animado ao lado do título", key: Key::Indicator, kind: Kind::Toggle },
            Row { icon: "\u{E916}", title: "Barra de progresso", desc: "Linha fina com o andamento da música", key: Key::TaskbarProgress, kind: Kind::Toggle },
            Row { icon: "\u{E895}", title: "Animar troca de música", desc: "O título novo desliza e a capa nova aparece suavemente", key: Key::TrackAnim, kind: Kind::Toggle },
            Section("Botões na barra"),
            Row { icon: "\u{E768}", title: "Botões de controle na barra", desc: "Anterior, play e próximo dentro do widget; desligados, ficam só no card", key: Key::TaskbarControls, kind: Kind::Toggle },
            Row { icon: "\u{E8B1}", title: "Aleatório e repetir na barra", desc: "Junto dos botões; apagados quando o player não deixa (navegadores)", key: Key::TaskbarShuffle, kind: Kind::Toggle },
            Row { icon: "\u{E768}", title: "Botão play", desc: "Círculo na cor de destaque ou só o ícone, como os outros", key: Key::TaskbarPlay, kind: Kind::Segmented(TASKBAR_PLAYS) },
            Row { icon: "\u{E7FB}", title: "Separador", desc: "Linha fina entre a música e os botões", key: Key::WidgetDivider, kind: Kind::Toggle },
            Section("Capa e texto"),
            Row { icon: "\u{E91B}", title: "Formato da capa", desc: "Como a capa aparece na barra", key: Key::CoverShape, kind: Kind::Segmented(SHAPES) },
            Row { icon: "\u{E740}", title: "Tamanho da capa", desc: "Capa menor deixa o widget mais discreto", key: Key::CoverSize, kind: Kind::Segmented(COVER_SIZES) },
            Row { icon: "\u{E8E9}", title: "Tamanho do texto", desc: "Título e segunda linha", key: Key::TextSize, kind: Kind::Segmented(SIZES) },
            Row { icon: "\u{E8C9}", title: "Rolar títulos longos", desc: "Letreiro suave quando o nome não cabe", key: Key::Marquee, kind: Kind::Toggle },
            Row { icon: "\u{E8D2}", title: "Largura do título", desc: "Espaço para o nome da música e do artista", key: Key::TitleWidth, kind: Kind::Slider { min: 100, max: 280, step: 10, unit: "px" } },
            Section("Fundo"),
            Row { icon: "\u{E7B5}", title: "Fundo em pílula", desc: "Fundo sutil sempre visível, com a música e os botões dentro", key: Key::WidgetPill, kind: Kind::Toggle },
            Row { icon: "\u{E7FB}", title: "Formato do fundo", desc: "Cantos do fundo do widget", key: Key::WidgetShape, kind: Kind::Segmented(WIDGET_SHAPES) },
            Row { icon: "\u{E790}", title: "Cor do fundo", desc: "Neutro, tingido com a cor da capa ou com a cor de destaque", key: Key::WidgetTint, kind: Kind::Segmented(TINTS) },
            Row { icon: "\u{E706}", title: "Opacidade do fundo", desc: "Quanto o fundo aparece sobre a barra", key: Key::WidgetOpacity, kind: Kind::Slider { min: 0, max: 60, step: 2, unit: "%" } },
            Row { icon: "\u{E91B}", title: "Contorno", desc: "Borda fina em volta do fundo", key: Key::WidgetBorder, kind: Kind::Toggle },
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
            Row { icon: "\u{E771}", title: "Fundo", desc: "Capa desfocada, cor extraída da capa ou acrílico (desfoca o que está atrás)", key: Key::CardStyle, kind: Kind::Segmented(CARD_STYLES) },
            Row { icon: "\u{E8A9}", title: "Layout", desc: "Capa ao lado ou capa grande em cima", key: Key::CardLayout, kind: Kind::Segmented(CARD_LAYOUTS) },
            Row { icon: "\u{E793}", title: "Tema", desc: "Segue o Windows ou fixa claro/escuro", key: Key::Theme, kind: Kind::Segmented(THEMES) },
            Row { icon: "\u{E790}", title: "Cor de destaque", desc: "Botões e progresso na cor da capa ou do Windows", key: Key::Accent, kind: Kind::Segmented(ACCENTS) },
            Row { icon: "\u{E7FB}", title: "Cantos", desc: "Raio das bordas do card", key: Key::Corners, kind: Kind::Segmented(CORNERS) },
            Row { icon: "\u{E768}", title: "Botão play", desc: "Formato do botão principal", key: Key::PlayShape, kind: Kind::Segmented(PLAY_SHAPES) },
            Section("Conteúdo"),
            Row { icon: "\u{E916}", title: "Barra de progresso", desc: "Tempo decorrido e restante; arraste para pular", key: Key::CardSeek, kind: Kind::Toggle },
            Row { icon: "\u{E767}", title: "Volume", desc: "Slider de volume e botão de mudo", key: Key::ShowVolume, kind: Kind::Toggle },
            Row { icon: "\u{E7F4}", title: "Mostrar app de origem", desc: "Selo com o app (Spotify, Brave…) no card, no \"A seguir\" e sobre a capa na barra", key: Key::ShowSource, kind: Kind::Toggle },
            Row { icon: "\u{E916}", title: "Fechar sozinho após", desc: "Tempo sem o mouse em cima do card", key: Key::Autoclose, kind: Kind::Slider { min: 2, max: 20, step: 1, unit: "s" } },
            Row { icon: "\u{E8B1}", title: "Aleatório e repetir", desc: "Funcionam no Spotify e no Media Player; navegadores (Brave, Chrome) não deixam, então ficam apagados", key: Key::CardShuffle, kind: Kind::Segmented(SHUFFLE_MODES) },
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
            Section("Fundo"),
            Row { icon: "\u{E771}", title: "Material", desc: "Sólido, acrílico (vidro fosco que desfoca o que está atrás) ou na cor de destaque", key: Key::OsdMaterial, kind: Kind::Segmented(OSD_MATERIALS) },
            Row { icon: "\u{E706}", title: "Opacidade", desc: "Quanto o fundo cobre o que está atrás", key: Key::OsdOpacity, kind: Kind::Slider { min: 20, max: 100, step: 2, unit: "%" } },
            Row { icon: "\u{E890}", title: "Desfoque", desc: "Intensidade do desfoque no fundo acrílico", key: Key::OsdBlur, kind: Kind::Slider { min: 0, max: 100, step: 5, unit: "%" } },
            Section("Aparência"),
            Row { icon: "\u{E72E}", title: "Ícone", desc: "Letra (A / 1), cadeado que abre e fecha, ou teclado", key: Key::OsdIcon, kind: Kind::Segmented(OSD_ICONS) },
            Row { icon: "\u{E7FB}", title: "Cantos", desc: "Formato do aviso e do ícone", key: Key::OsdShape, kind: Kind::Segmented(OSD_SHAPES) },
            Row { icon: "\u{E8D3}", title: "Tamanho", desc: "Tamanho do aviso e do texto", key: Key::OsdSize, kind: Kind::Segmented(OSD_SIZES) },
            Row { icon: "\u{E793}", title: "Tema", desc: "Segue o Windows ou fixa claro/escuro", key: Key::OsdTheme, kind: Kind::Segmented(THEMES) },
            Row { icon: "\u{E7E8}", title: "Interruptor", desc: "Chave animada no lado direito", key: Key::OsdSwitch, kind: Kind::Toggle },
            Row { icon: "\u{E768}", title: "Animação", desc: "Como o aviso entra e sai da tela", key: Key::OsdAnim, kind: Kind::Segmented(OSD_ANIMS) },
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
            Row { icon: "\u{E9E9}", title: "Mostrar barras dinâmicas", desc: "Mini visualizador ao lado da música; captura o som do PC", key: Key::ShowViz, kind: Kind::Toggle },
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
            Row { icon: "\u{E8A5}", title: "Licenças", desc: "Licença MIT e bibliotecas de código aberto usadas", key: Key::Licenses, kind: Kind::Button("Ver licenças") },
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
const WIDGET_SHAPE_V: &[WidgetShape] = &[WidgetShape::Rounded, WidgetShape::Pill, WidgetShape::Square];
const TASKBAR_PLAY_V: &[TaskbarPlay] = &[TaskbarPlay::Filled, TaskbarPlay::Icon];
const SHUFFLE_V: &[ShuffleRepeat] = &[ShuffleRepeat::Always, ShuffleRepeat::WhenSupported];
const OSD_MATERIAL_V: &[OsdMaterial] = &[OsdMaterial::Solid, OsdMaterial::Acrylic, OsdMaterial::Accent];
const OSD_ICON_V: &[OsdIcon] = &[OsdIcon::Letter, OsdIcon::Lock, OsdIcon::Keyboard];
const OSD_ANIM_V: &[OsdAnim] = &[OsdAnim::Slide, OsdAnim::Zoom, OsdAnim::Fade];
const PALETTE_V: &[Palette] = &[Palette::Album, Palette::Accent, Palette::Rainbow, Palette::Neon, Palette::Mono];
const SIZE_V: &[Size3] = &[Size3::Small, Size3::Normal, Size3::Large];
const SECOND_V: &[SecondLine] = &[SecondLine::Artist, SecondLine::ArtistAlbum, SecondLine::Album, SecondLine::Time];
const TINT_V: &[WidgetTint] = &[WidgetTint::Neutral, WidgetTint::Album, WidgetTint::Accent];

fn get(c: &Config, key: Key) -> i32 {
    match key {
        Key::ShowMedia => c.show_media as i32,
        Key::AlwaysShow => c.always_show_widget as i32,
        Key::TaskbarControls => c.taskbar_controls as i32,
        Key::ShowArtist => c.show_artist as i32,
        Key::SecondLine => idx(SECOND_V, c.second_line),
        Key::Indicator => c.playing_indicator as i32,
        Key::TaskbarProgress => c.taskbar_progress as i32,
        Key::TrackAnim => c.track_anim as i32,
        Key::CoverShape => idx(SHAPE_V, c.cover_shape),
        Key::CoverSize => idx(SIZE_V, c.cover_size),
        Key::TextSize => idx(SIZE_V, c.text_size),
        Key::WidgetPill => c.widget_pill as i32,
        Key::WidgetShape => idx(WIDGET_SHAPE_V, c.widget_shape),
        Key::WidgetTint => idx(TINT_V, c.widget_tint),
        Key::WidgetOpacity => c.widget_opacity,
        Key::WidgetBorder => c.widget_border as i32,
        Key::WidgetDivider => c.widget_divider as i32,
        Key::TaskbarShuffle => c.taskbar_shuffle_repeat as i32,
        Key::TaskbarPlay => idx(TASKBAR_PLAY_V, c.taskbar_play),
        Key::CardShuffle => idx(SHUFFLE_V, c.card_shuffle_repeat),
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
        Key::OsdMaterial => idx(OSD_MATERIAL_V, c.osd_material),
        Key::OsdOpacity => c.osd_opacity,
        Key::OsdBlur => c.osd_blur,
        Key::OsdIcon => idx(OSD_ICON_V, c.osd_icon),
        Key::OsdAnim => idx(OSD_ANIM_V, c.osd_anim),
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
        Key::Reset | Key::About | Key::OsdTest | Key::Licenses => 0,
    }
}

fn set(c: &mut Config, key: Key, v: i32) {
    let i = v.max(0) as usize;
    fn pick<T: Copy>(l: &[T], i: usize) -> T {
        l[i.min(l.len() - 1)]
    }
    match key {
        Key::ShowMedia => c.show_media = v != 0,
        Key::AlwaysShow => c.always_show_widget = v != 0,
        Key::TaskbarControls => c.taskbar_controls = v != 0,
        Key::ShowArtist => c.show_artist = v != 0,
        Key::SecondLine => c.second_line = pick(SECOND_V, i),
        Key::Indicator => c.playing_indicator = v != 0,
        Key::TaskbarProgress => c.taskbar_progress = v != 0,
        Key::TrackAnim => c.track_anim = v != 0,
        Key::CoverShape => c.cover_shape = pick(SHAPE_V, i),
        Key::CoverSize => c.cover_size = pick(SIZE_V, i),
        Key::TextSize => c.text_size = pick(SIZE_V, i),
        Key::WidgetPill => c.widget_pill = v != 0,
        Key::WidgetShape => c.widget_shape = pick(WIDGET_SHAPE_V, i),
        Key::WidgetTint => c.widget_tint = pick(TINT_V, i),
        Key::WidgetOpacity => c.widget_opacity = v,
        Key::WidgetBorder => c.widget_border = v != 0,
        Key::WidgetDivider => c.widget_divider = v != 0,
        Key::TaskbarShuffle => c.taskbar_shuffle_repeat = v != 0,
        Key::TaskbarPlay => c.taskbar_play = pick(TASKBAR_PLAY_V, i),
        Key::CardShuffle => c.card_shuffle_repeat = pick(SHUFFLE_V, i),
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
        Key::OsdMaterial => c.osd_material = pick(OSD_MATERIAL_V, i),
        Key::OsdOpacity => c.osd_opacity = v,
        Key::OsdBlur => c.osd_blur = v,
        Key::OsdIcon => c.osd_icon = pick(OSD_ICON_V, i),
        Key::OsdAnim => c.osd_anim = pick(OSD_ANIM_V, i),
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
        Key::Autostart | Key::Reset | Key::About | Key::OsdTest | Key::Licenses => {}
    }
}

/// Opções que só fazem sentido com outra ligada: devolve o motivo (texto da
/// dica) quando a opção está desativada. Desativada, ela aparece apagada e,
/// se for um interruptor, desligada.
fn requirement(c: &Config, key: Key) -> Option<&'static str> {
    use Key::*;
    const MEDIA: &str = "Ligue \"Mostrar música na barra\" para usar esta opção";
    const CONTROLS: &str = "Os botões estão desligados: ligue \"Botões de controle na barra\" para usar esta opção";
    const PILL: &str = "Ligue \"Fundo em pílula\" para usar esta opção";
    const ARTIST: &str = "Ligue \"Mostrar artista\" para escolher a segunda linha";
    const POPUP: &str = "Ligue \"Mostrar A seguir\" para usar esta opção";
    const OSD: &str = "Ligue o aviso de Caps Lock ou de Num Lock para usar esta opção";
    const BLUR: &str = "O desfoque só vale para o material Acrílico";
    const VIZ: &str = "Ligue \"Mostrar barras dinâmicas\" para usar esta opção";
    let osd_on = c.caps_lock_osd || c.num_lock_osd;
    match key {
        AlwaysShow | ShowArtist | SecondLine | Indicator | TaskbarProgress | TrackAnim | TaskbarControls | TaskbarShuffle | TaskbarPlay | WidgetDivider | CoverShape | CoverSize | TextSize | Marquee | TitleWidth | WidgetPill | WidgetShape | WidgetTint | WidgetOpacity | WidgetBorder
            if !c.show_media =>
        {
            Some(MEDIA)
        }
        TaskbarShuffle | TaskbarPlay | WidgetDivider if !c.taskbar_controls => Some(CONTROLS),
        WidgetShape | WidgetTint | WidgetOpacity | WidgetBorder if !c.widget_pill => Some(PILL),
        SecondLine if !c.show_artist => Some(ARTIST),
        PopupSecs if !c.next_track_popup => Some(POPUP),
        OsdShowOff | OsdTest | OsdPos | OsdOffX | OsdOffY | OsdShape | OsdSize | OsdTheme | OsdDuration | OsdSwitch | OsdMaterial | OsdOpacity | OsdBlur | OsdIcon | OsdAnim if !osd_on => Some(OSD),
        OsdBlur if c.osd_material != crate::config::OsdMaterial::Acrylic => Some(BLUR),
        Style | Palette | Bars | Width | Sens | OnlyMedia | Peaks if !c.show_visualizer => Some(VIZ),
        _ => None,
    }
}

fn value_label(kind: Kind, key: Key, v: i32) -> String {
    let Kind::Slider { unit, .. } = kind else { return String::new() };
    match key {
        Key::Sens | Key::Offset | Key::OsdOffX | Key::OsdOffY => format!("{v:+} {unit}"),
        _ if unit == "%" => format!("{v}%"),
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
    /// Linha desativada (mostra a dica do motivo).
    Off(usize),
}

/// Camada já desenhada guardada em memória.
struct Cached<K> {
    key: K,
    w: usize,
    h: usize,
    px: Vec<u32>,
}

/// O que obriga a redesenhar a coluna de opções inteira (a rolagem não entra:
/// a camada é copiada na posição da rolagem; mudanças numa linha redesenham só ela).
#[derive(Clone, Copy, PartialEq)]
struct ContentKey {
    page: usize,
    w: i32,
    h: i32,
    scale: u32,
    dark: bool,
    mica: bool,
    accent: [u32; 3],
    lang: u8,
    list_top: i32,
    content_h: i32,
}

/// Estado visível de uma linha: se mudar, só ela é redesenhada.
#[derive(Clone, Copy, PartialEq, Debug)]
struct RowState {
    v: i32,
    enabled: bool,
    hot: bool,
    drag: bool,
    seg_hover: i8,
    hover: u8,
    toggle: u8,
    seg: i16,
}

/// Valores animados de uma linha.
#[derive(Clone, Copy, Default, Debug)]
struct RowAnim {
    /// Interruptor: posição da bolinha (0 = desligado, 1 = ligado).
    toggle: f32,
    /// Segmentado: posição da pílula (índice, fracionário durante a animação).
    seg: f32,
    /// Realce de "mouse em cima" (0..1).
    hover: f32,
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

/// Quadro da prévia: só é desenhado de novo quando isto muda.
#[derive(Clone, Copy, PartialEq)]
struct HeroSig {
    key: HeroKey,
    cfg: u64,
    media: u64,
    frame: u64,
}

/// Retângulo do cartão dentro da prévia, desfoque e fundo usados no acrílico.
type BackdropKey = (i32, i32, i32, i32, i32, HeroKey);

#[derive(Clone, Copy, PartialEq)]
struct LayoutKey {
    page: usize,
    w: i32,
    h: i32,
    scale: u32,
    lang: u8,
    cfg: u64,
}

/// Dica de opção desativada.
struct Tip {
    ri: usize,
    x: f32,
    y: f32,
    since: Instant,
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
    /// Texto da prévia (pode ter escala menor que a da janela).
    preview_text: TextRenderer,
    t0: Instant,
    colors: Option<(Palette, Colors)>,
    layout: Option<(LayoutKey, Rc<Layout>)>,
    content: Option<Cached<ContentKey>>,
    rows_drawn: Vec<RowState>,
    anims: Vec<RowAnim>,
    hero_bg: Option<Cached<HeroKey>>,
    hero_frame: Option<Cached<HeroSig>>,
    /// Onde o componente da prévia foi desenhado no último quadro (coordenadas da prévia).
    hero_comp: Option<RectF>,
    /// Fundo acrílico da prévia (o "papel de parede" desfocado atrás do componente).
    backdrop: Option<(BackdropKey, Arc<Image>)>,
    /// Indicador do menu lateral (posição animada, em índice de página).
    nav_pos: f32,
    /// Troca de página: quando a nova apareceu (entra deslizando e esmaecendo).
    page_anim: Option<Instant>,
    page_anim_pending: bool,
    tip: Option<Tip>,
    pacer: Option<Pacer>,
    slow_timer: bool,
    last_tick: Instant,
    /// Páginas já pré-carregadas (bit por página) e onde o pré-carregamento parou.
    warmed: u32,
    warm_pos: Option<(usize, usize)>,
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
        AUTOSTART.with(|a| a.set(system::autostart_enabled()));
        with_ui(|u| u.schedule());
        unsafe {
            let _ = InvalidateRect(Some(h), None, false);
        }
    }
}

/// O player mudou (música, play/pausa): a prévia acompanha na hora, mesmo com
/// a tela parada (sem timer rodando), e o ritmo dela é recalculado.
pub fn media_changed() {
    if let Some(h) = ui_hwnd() {
        with_ui(|u| u.tick());
        unsafe {
            let _ = UpdateWindow(h);
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

fn perf_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("TASKBEAT_PERF").is_some())
}

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
        let pacer = Pacer::new(Target::Post(hwnd, WM_APP_FRAME));
        if let Some(p) = &pacer {
            p.set_monitor(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST));
        }
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
                preview_text: TextRenderer::new(),
                t0: Instant::now(),
                colors: None,
                layout: None,
                content: None,
                rows_drawn: Vec::new(),
                anims: Vec::new(),
                hero_bg: None,
                hero_frame: None,
                hero_comp: None,
                backdrop: None,
                nav_pos: 0.0,
                page_anim: None,
                page_anim_pending: false,
                tip: None,
                pacer,
                slow_timer: false,
                last_tick: Instant::now(),
                warmed: 1,
                warm_pos: None,
            });
        });
        // Abre maximizada (ao restaurar, volta ao tamanho padrão, centralizada).
        // A 1ª chamada de ShowWindow do processo pode herdar o modo de exibição do
        // atalho/linha de comando (ex.: oculto); a 2ª garante que a janela apareça.
        let _ = ShowWindow(hwnd, SW_SHOWMAXIMIZED);
        let _ = ShowWindow(hwnd, SW_SHOWMAXIMIZED);
        let _ = SetForegroundWindow(hwnd);
        with_ui(|u| u.schedule());
        SetTimer(Some(hwnd), WARM_TIMER, 150, None);
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

#[derive(Clone)]
struct RowL {
    item: usize,
    card: RectF,
    ctl: RectF,
    segs: Vec<RectF>,
}

impl RowL {
    fn shifted(&self, dx: f32, dy: f32) -> RowL {
        RowL { item: self.item, card: self.card.offset(dx, dy), ctl: self.ctl.offset(dx, dy), segs: self.segs.iter().map(|r| r.offset(dx, dy)).collect() }
    }
}

/// Posições na janela com a lista sem rolagem (a rolagem é aplicada ao usar).
struct Layout {
    nav: Vec<RectF>,
    x0: f32,
    x1: f32,
    title_y: f32,
    hero: Option<RectF>,
    /// Topo da área que rola (logo abaixo da prévia fixa).
    list_top: f32,
    sections: Vec<(usize, f32)>,
    rows: Vec<RowL>,
    content_h: f32,
}

fn lang_code() -> u8 {
    crate::i18n::lang() as u8
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

    fn cfg_version(&self) -> u64 {
        self.shared.config_version.load(Ordering::Relaxed)
    }

    fn colors(&mut self, palette: Palette) -> Colors {
        if self.colors.as_ref().map(|c| c.0) != Some(palette) {
            self.colors = Some((palette, system::colors(palette)));
        }
        self.colors.as_ref().map(|c| c.1.clone()).unwrap_or_else(|| system::colors(palette))
    }

    fn preview_media(&self) -> MediaInfo {
        // Capturas de tela/demonstração: sempre a música de exemplo.
        if std::env::var_os("TV_DEMO_MEDIA").is_some() {
            return ui::sample_media();
        }
        let m = self.shared.media.lock().unwrap().clone();
        if m.present() { m } else { ui::sample_media() }
    }

    fn hero_height_at(page: &Page, cfg: &Config, s: f32) -> f32 {
        match page.preview {
            Preview::Widget | Preview::Visualizer => 168.0 * s,
            Preview::Card => ui::CLayout::build(s, cfg, true).h + 8.0 * s,
            Preview::UpNext => ui::upnext_size(s).1 + 24.0 * s,
            Preview::LockKeys => ui::osd_size(s, cfg).1 + 40.0 * s,
            Preview::None => 0.0,
        }
    }

    /// Escala da prévia: a da janela ou menor, quando ela tomaria mais da
    /// metade da altura (a prévia fica fixa no topo; as opções rolam embaixo).
    fn preview_scale(&self, page: &Page, cfg: &Config) -> f32 {
        let s = self.scale;
        let natural = Self::hero_height_at(page, cfg, s);
        let budget = (self.client().1 * 0.5).max(190.0 * s);
        if natural <= budget { s } else { (s * budget / natural).max(0.55 * s) }
    }

    /// Rolagem em pixels inteiros (a camada de conteúdo é copiada sem reamostrar).
    fn scroll_px(&self) -> f32 {
        self.scroll.round()
    }

    /// Layout da página atual (em cache até mudar tamanho, página, idioma ou config).
    fn layout(&mut self) -> Rc<Layout> {
        let (w, h) = self.client();
        let key = LayoutKey { page: self.page, w: w as i32, h: h as i32, scale: self.scale.to_bits(), lang: lang_code(), cfg: self.cfg_version() };
        if let Some((k, l)) = &self.layout {
            if *k == key {
                return l.clone();
            }
        }
        let l = Rc::new(self.build_layout(w));
        self.layout = Some((key, l.clone()));
        l
    }

    fn build_layout(&mut self, w: f32) -> Layout {
        let s = self.scale;
        let cfg = self.cfg();
        let page = &PAGES[self.page];
        let nav = (0..PAGES.len()).map(|i| RectF::new(10.0 * s, (92.0 + i as f32 * 42.0) * s, NAV_W * s - 20.0 * s, 38.0 * s)).collect();
        let left = NAV_W * s + 12.0 * s;
        let right = (w - 32.0 * s).max(left + 200.0 * s);
        let (x0, x1) = (left, right);
        // Título e prévia ficam fixos; só a lista de opções rola, abaixo da prévia.
        let mut y = 92.0 * s;
        let mut hero = None;
        let hh = Self::hero_height_at(page, &cfg, self.preview_scale(page, &cfg));
        if hh > 0.0 {
            hero = Some(RectF::new(x0, y, x1 - x0, hh));
            y += hh + 12.0 * s;
        }
        let list_top = y.round();
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
                    let card = RectF::new(x0, y.round(), x1 - x0, 66.0 * s);
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
                                .map(|o| (self.text.mask(tr(o), Font::BodyStrong, (300.0 * s) as i32, (20.0 * s) as i32, Align::Left).used_width() + 28.0 * s).max(78.0 * s).round())
                                .collect();
                            let total: f32 = widths.iter().sum::<f32>() + 8.0 * s;
                            let r = RectF::new((right - total).round(), (card.cy() - 17.0 * s).round(), total.round(), (34.0 * s).round());
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
        let content_h = y + 24.0 * s;
        Layout { nav, x0, x1, title_y: 26.0 * s, hero, list_top, sections, rows, content_h }
    }

    fn max_scroll(&mut self) -> f32 {
        let (_, h) = self.client();
        (self.layout().content_h - h).max(0.0)
    }

    fn hit(&mut self, x: f32, y: f32) -> Option<Hit> {
        let l = self.layout();
        if let Some(i) = l.nav.iter().position(|r| r.contains(x, y)) {
            return Some(Hit::Nav(i));
        }
        // A lista rola por baixo da prévia fixa: o que está escondido não recebe clique.
        if x < NAV_W * self.scale || y < l.list_top {
            return None;
        }
        let y = y + self.scroll_px();
        let cfg = self.cfg();
        for (ri, r) in l.rows.iter().enumerate() {
            let Row { kind, key, .. } = &PAGES[self.page].items[r.item] else { continue };
            if requirement(&cfg, *key).is_some() {
                if r.card.contains(x, y) {
                    return Some(Hit::Off(ri));
                }
                continue;
            }
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

    fn row_count(&self) -> usize {
        PAGES[self.page].items.iter().filter(|i| matches!(i, Row { .. })).count()
    }

    fn slider_value(&mut self, ri: usize, x: f32) -> Option<i32> {
        let (_, Kind::Slider { min, max, step, .. }) = self.row(ri)? else { return None };
        let ctl = self.layout().rows.get(ri)?.ctl;
        let (x0, x1) = (ctl.x + 10.0 * self.scale, ctl.right() - 10.0 * self.scale);
        let t = ((x - x0) / (x1 - x0)).clamp(0.0, 1.0);
        let raw = min as f32 + t * (max - min) as f32;
        Some(((raw / step as f32).round() as i32 * step).clamp(min, max))
    }

    /// Valor mostrado (opção desativada aparece desligada) e se está ativa.
    fn shown_value(&self, cfg: &Config, key: Key, kind: Kind) -> (i32, bool) {
        let enabled = requirement(cfg, key).is_none();
        let v = get(cfg, key);
        (if !enabled && matches!(kind, Kind::Toggle) { 0 } else { v }, enabled)
    }

    fn anim_target(&self, ri: usize, cfg: &Config) -> RowAnim {
        let Some((key, kind)) = self.row(ri) else { return RowAnim::default() };
        let (v, _) = self.shown_value(cfg, key, kind);
        let hot = matches!(self.hover, Some(Hit::Row(r)) if r == ri) || self.drag == Some(ri);
        RowAnim {
            toggle: if matches!(kind, Kind::Toggle) && v != 0 { 1.0 } else { 0.0 },
            seg: if matches!(kind, Kind::Segmented(_)) { v as f32 } else { 0.0 },
            hover: if hot { 1.0 } else { 0.0 },
        }
    }

    /// Começa a página sem animação nas linhas (tudo já no lugar).
    fn reset_anims(&mut self) {
        let cfg = self.cfg();
        self.anims = (0..self.row_count()).map(|ri| self.anim_target(ri, &cfg)).collect();
    }

    /// Avança as animações das linhas; devolve se alguma ainda está andando.
    fn step_anims(&mut self, dt: f32) -> bool {
        let cfg = self.cfg();
        if self.anims.len() != self.row_count() {
            self.reset_anims();
        }
        let mut moving = false;
        for ri in 0..self.anims.len() {
            let t = self.anim_target(ri, &cfg);
            let a = &mut self.anims[ri];
            let mut go = |v: &mut f32, target: f32, rate: f32, eps: f32| {
                let d = target - *v;
                if d.abs() <= eps {
                    *v = target;
                } else {
                    *v += d * (1.0 - (-dt * rate).exp());
                    moving = true;
                }
            };
            go(&mut a.toggle, t.toggle, 16.0, 0.004);
            go(&mut a.seg, t.seg, 15.0, 0.004);
            go(&mut a.hover, t.hover, 22.0, 0.01);
        }
        moving
    }

    fn anims_moving(&self) -> bool {
        let cfg = self.cfg();
        self.anims.iter().enumerate().any(|(ri, a)| {
            let t = self.anim_target(ri, &cfg);
            (a.toggle - t.toggle).abs() > 0.004 || (a.seg - t.seg).abs() > 0.004 || (a.hover - t.hover).abs() > 0.01
        })
    }

    fn row_state(&self, ri: usize, cfg: &Config) -> RowState {
        let (key, kind) = self.row(ri).unwrap_or((Key::About, Kind::Info));
        let (v, enabled) = self.shown_value(cfg, key, kind);
        let a = self.anims.get(ri).copied().unwrap_or_default();
        let q = |x: f32| (x.clamp(0.0, 1.0) * 96.0).round() as u8;
        RowState {
            v,
            enabled,
            hot: matches!(self.hover, Some(Hit::Row(r)) if r == ri),
            drag: self.drag == Some(ri),
            seg_hover: match self.hover {
                Some(Hit::Seg(r, k)) if r == ri => k as i8,
                _ => -1,
            },
            hover: q(a.hover),
            toggle: q(a.toggle),
            seg: (a.seg * 96.0).round() as i16,
        }
    }

    fn page_anim_t(&self) -> Option<f32> {
        let t = self.page_anim?.elapsed().as_secs_f32() / 0.32;
        (t < 1.0).then_some(t)
    }

    fn nav_moving(&self) -> bool {
        (self.nav_pos - self.page as f32).abs() > 0.002
    }

    fn tip_alpha(&self) -> f32 {
        self.tip.as_ref().map(|t| ((t.since.elapsed().as_secs_f32() - TIP_DELAY) / 0.12).clamp(0.0, 1.0)).unwrap_or(0.0)
    }

    /// A música do player está pausada: na barra, o indicador, as barras e o
    /// tempo ficam parados, e a prévia também (sem redesenhar à toa).
    fn media_paused(&self) -> bool {
        if std::env::var_os("TV_DEMO_MEDIA").is_some() {
            return false;
        }
        let m = self.shared.media.lock().unwrap();
        m.present() && m.pos_at.is_some() && !m.playing
    }

    /// Quadros por segundo que a prévia precisa (0 = parada).
    fn hero_fps(&self, cfg: &Config) -> f32 {
        match PAGES[self.page].preview {
            Preview::UpNext | Preview::LockKeys | Preview::Visualizer => 30.0,
            Preview::Widget if cfg.marquee => 30.0,
            Preview::Widget if self.media_paused() => 0.0,
            Preview::Widget if cfg.playing_indicator || cfg.show_visualizer => 30.0,
            Preview::Widget if cfg.taskbar_progress || cfg.second_line == SecondLine::Time => 2.0,
            Preview::Widget => 0.0,
            Preview::Card if self.media_paused() => 0.0,
            Preview::Card => 2.0,
            Preview::None => 0.0,
        }
    }

    /// Liga o ritmo do monitor só quando algo anima; prévias lentas usam um
    /// timer de 250 ms; parado, nada roda (janela parada = 0% de CPU).
    fn schedule(&mut self) {
        let minimized = unsafe { IsIconic(self.hwnd).as_bool() };
        let cfg = self.cfg();
        let fps = if minimized { 0.0 } else { self.hero_fps(&cfg) };
        let tip_pending = self.tip.is_some() && self.tip_alpha() < 1.0;
        let busy = !minimized
            && ((self.scroll_target - self.scroll).abs() > 0.5
                || self.anims_moving()
                || self.nav_moving()
                || self.page_anim.is_some()
                || self.page_anim_pending
                || tip_pending
                || fps >= 30.0);
        let use_pacer = busy && self.pacer.is_some();
        if let Some(p) = &self.pacer {
            p.set_active(use_pacer);
        }
        // Sem o vblank (falhou ao criar), um timer de 16 ms faz o papel dele.
        let slow = if busy && !use_pacer {
            Some(16)
        } else if fps > 0.0 || use_pacer {
            Some(250)
        } else {
            None
        };
        unsafe {
            match slow {
                Some(ms) => {
                    SetTimer(Some(self.hwnd), SLOW_TIMER, ms, None);
                    self.slow_timer = true;
                }
                None if self.slow_timer => {
                    let _ = KillTimer(Some(self.hwnd), SLOW_TIMER);
                    self.slow_timer = false;
                }
                None => {}
            }
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

    /// Fundo (transparente com Mica; sem Mica, cor sólida do tema) como pixel pré-multiplicado.
    fn base_px(&self) -> u32 {
        if self.mica {
            0
        } else if self.dark {
            0xFF20_2020
        } else {
            0xFFF3_F3F3
        }
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
        let dirty = cv.clip_rect();
        let l = self.layout();
        let ox = (NAV_W * s).round();
        let lt = l.list_top;

        // ---- menu lateral (fixo)
        let nav_area = dirty.intersect(RectF::new(0.0, 0.0, ox, h));
        if !nav_area.is_empty() {
            cv.set_clip(nav_area);
            cv.fill_clip(self.base_px());
            self.draw_nav(cv, &l, h);
        }

        // ---- coluna da direita: camada pronta, copiada na posição da rolagem.
        let right = dirty.intersect(RectF::new(ox, 0.0, w - ox, h));
        if !right.is_empty() {
            self.ensure_content(w, h, ox, &l);
            if self.page_anim_pending {
                // A página nova acabou de ficar pronta: a entrada começa agora.
                self.page_anim_pending = false;
                self.page_anim = Some(Instant::now());
            }
            let (alpha, dy) = match self.page_anim_t() {
                Some(t) => {
                    let e = ui::ease_enter(t);
                    (e, ((1.0 - e) * 28.0 * s).round() as i32)
                }
                None => (1.0, 0),
            };
            let sc = self.scroll_px() as i32;
            if let Some(c) = &self.content {
                let top = right.intersect(RectF::new(ox, 0.0, w - ox, lt));
                let list = right.intersect(RectF::new(ox, lt, w - ox, h - lt));
                if alpha >= 0.999 {
                    cv.set_clip(top);
                    cv.copy_from(ox as i32, 0, &c.px, c.w, c.h, 0);
                    cv.set_clip(list);
                    cv.copy_from(ox as i32, 0, &c.px, c.w, c.h, sc);
                } else {
                    cv.set_clip(right);
                    cv.fill_clip(self.base_px());
                    cv.set_clip(top);
                    cv.composite(ox as i32, dy, &c.px, c.w, c.h, alpha);
                    cv.set_clip(list.intersect(RectF::new(ox, lt + dy as f32, w - ox, h)));
                    cv.composite(ox as i32, dy - sc, &c.px, c.w, c.h, alpha);
                }
            }

            // ---- prévia (fixa no topo, sempre à vista)
            if let Some(hero) = l.hero {
                let hr = RectF::new(hero.x.round(), hero.y.round(), hero.w.round(), hero.h.round());
                let hd = right.intersect(hr.offset(0.0, dy as f32));
                if !hd.is_empty() {
                    let _ = self.ensure_hero(hr);
                    if let Some(f) = &self.hero_frame {
                        cv.set_clip(hd);
                        if alpha >= 0.999 {
                            cv.copy_from(hr.x as i32, hr.y as i32, &f.px, f.w, f.h, 0);
                        } else {
                            cv.composite(hr.x as i32, hr.y as i32 + dy, &f.px, f.w, f.h, alpha);
                        }
                    }
                }
                // Com a lista rolada, uma linha marca onde ela passa por baixo da prévia.
                if self.scroll > 0.5 {
                    cv.set_clip(right);
                    cv.fill_rrect(l.x0, lt - 1.0 * s, l.x1, lt, 0.0, 0.10 * alpha, fg);
                }
            }

            // Barra de rolagem fina (só quando há o que rolar), ao lado da lista.
            let max = (l.content_h - h).max(0.0);
            if max > 1.0 {
                cv.set_clip(right.intersect(RectF::new(ox, lt, w - ox, h - lt)));
                let view = h - lt;
                let track_h = view - 16.0 * s;
                let bar_h = (track_h * view / (l.content_h - lt)).max(30.0 * s);
                let by = lt + 8.0 * s + (track_h - bar_h) * (self.scroll / max).clamp(0.0, 1.0);
                cv.fill_round(RectF::new(w - 8.0 * s, by, 3.0 * s, bar_h), 1.5 * s, fg, 0.28 * alpha);
            }
        }

        cv.set_clip(dirty);
        self.draw_tip(cv, w, h);
    }

    fn draw_nav(&mut self, cv: &mut Canvas, l: &Layout, h: f32) {
        let s = self.scale;
        let fg = self.fg();
        let accent = self.accent;
        draw_logo(cv, RectF::new(22.0 * s, 24.0 * s, 40.0 * s, 40.0 * s));
        self.label(cv, "Taskbeat", Font::Display, 74.0 * s, 22.0 * s, 180.0 * s, 26.0 * s, fg, 1.0);
        self.label(cv, tr("Configurações"), Font::Caption, 75.0 * s, 46.0 * s, 180.0 * s, 18.0 * s, fg, 0.6);
        // Seleção: fundo e indicador deslizam juntos até a página escolhida.
        let p = self.nav_pos.clamp(0.0, (PAGES.len() - 1) as f32);
        let (i0, i1) = (p.floor() as usize, (p.ceil() as usize).min(PAGES.len() - 1));
        let sel = l.nav[i0].lerp(l.nav[i1], p - i0 as f32);
        for (i, r) in l.nav.iter().enumerate() {
            if self.hover == Some(Hit::Nav(i)) && i != self.page {
                cv.fill_round(*r, 6.0 * s, fg, if self.dark { 0.04 } else { 0.03 });
            }
        }
        cv.fill_round(sel, 6.0 * s, fg, if self.dark { 0.065 } else { 0.05 });
        // O indicador estica no meio do caminho (como o NavigationView do WinUI).
        let travel = 1.0 - ((p - p.round()).abs() * 2.0 - 1.0).abs();
        let ih = 16.0 * s * (1.0 + travel * 0.9);
        cv.fill_round(RectF::new(sel.x, sel.cy() - ih / 2.0, 3.0 * s, ih), 1.5 * s, accent, 1.0);
        for (i, page) in PAGES.iter().enumerate() {
            let r = l.nav[i];
            let selected = i == self.page;
            ui::glyph(cv, &mut self.text, RectF::new(r.x + 12.0 * s, r.y, 24.0 * s, r.h), page.icon, Font::IconMd, if selected { accent } else { fg }, 0.95);
            self.label(cv, tr(page.name), Font::Body, r.x + 46.0 * s, r.y, r.w - 50.0 * s, r.h, fg, if selected { 1.0 } else { 0.85 });
        }
        let version = format!("v{} · {}", env!("CARGO_PKG_VERSION"), tr("feito em Rust"));
        self.label(cv, &version, Font::Caption, 26.0 * s, h - 34.0 * s, 200.0 * s, 18.0 * s, fg, 0.45);
    }

    fn content_key(&self, w: f32, h: f32, l: &Layout) -> ContentKey {
        ContentKey {
            page: self.page,
            w: w as i32,
            h: h as i32,
            scale: self.scale.to_bits(),
            dark: self.dark,
            mica: self.mica,
            accent: self.accent.map(f32::to_bits),
            lang: lang_code(),
            list_top: l.list_top as i32,
            content_h: l.content_h as i32,
        }
    }

    /// Camada da coluna de opções (sem rolagem). Redesenha tudo só quando a
    /// página, o tamanho ou o tema mudam; fora isso, só as linhas que mudaram.
    fn ensure_content(&mut self, w: f32, h: f32, ox: f32, l: &Layout) {
        let key = self.content_key(w, h, l);
        let cfg = self.cfg();
        if self.anims.len() != l.rows.len() {
            self.reset_anims();
        }
        let states: Vec<RowState> = (0..l.rows.len()).map(|ri| self.row_state(ri, &cfg)).collect();
        let bg = self.base_px();
        let t0 = Instant::now();
        if self.content.as_ref().is_some_and(|c| c.key == key) {
            let Some(mut c) = self.content.take() else { return };
            let mut n = 0;
            {
                let mut cv = Canvas::new(c.w, c.h, &mut c.px);
                for (ri, st) in states.iter().enumerate() {
                    if self.rows_drawn.get(ri) == Some(st) {
                        continue;
                    }
                    n += 1;
                    let row = l.rows[ri].shifted(-ox, 0.0);
                    cv.set_clip(row.card.inset(-1.0));
                    cv.fill_clip(bg);
                    self.draw_row(&mut cv, ri, &row, st);
                }
            }
            self.content = Some(c);
            self.rows_drawn = states;
            if n > 0 && perf_log() {
                crate::audio::log::info(&format!("PERF   rows {n} {:.2}ms", t0.elapsed().as_secs_f64() * 1e3));
            }
            return;
        }
        let cw = (w - ox).max(1.0) as usize;
        let ch = l.content_h.max(h).ceil().max(1.0) as usize;
        let mut px = self.content.take().map(|c| c.px).unwrap_or_default();
        px.clear();
        px.resize(cw * ch, bg);
        {
            let mut cv = Canvas::new(cw, ch, &mut px);
            self.draw_content(&mut cv, l, ox, &states);
        }
        self.content = Some(Cached { key, w: cw, h: ch, px });
        self.rows_drawn = states;
        if perf_log() {
            crate::audio::log::info(&format!("PERF   content {:.2}ms", t0.elapsed().as_secs_f64() * 1e3));
        }
    }

    fn draw_content(&mut self, cv: &mut Canvas, l: &Layout, ox: f32, states: &[RowState]) {
        let s = self.scale;
        let fg = self.fg();
        let page = &PAGES[self.page];
        let x0 = l.x0 - ox;
        let tw = l.x1 - l.x0;
        self.label(cv, tr(page.name), Font::Heading, x0, l.title_y, tw, 40.0 * s, fg, 1.0);
        self.label(cv, tr(page.subtitle), Font::Body, x0 + 1.0 * s, l.title_y + 40.0 * s, tw, 22.0 * s, fg, 0.62);
        for (i, y) in &l.sections {
            if let Section(name) = page.items[*i] {
                self.label(cv, tr(name), Font::BodyStrong, x0 + 2.0 * s, *y, 400.0 * s, 24.0 * s, fg, 1.0);
            }
        }
        for (ri, row) in l.rows.iter().enumerate() {
            let row = row.shifted(-ox, 0.0);
            self.draw_row(cv, ri, &row, &states[ri]);
        }
    }

    fn draw_row(&mut self, cv: &mut Canvas, ri: usize, r: &RowL, st: &RowState) {
        let s = self.scale;
        let fg = self.fg();
        let Row { icon, title, desc, key, kind } = &PAGES[self.page].items[r.item] else { return };
        let a = self.anims.get(ri).copied().unwrap_or_default();
        let en = if st.enabled { 1.0 } else { 0.42 };
        let card_a = if self.dark { 0.05 } else { 0.70 };
        let border_a = if self.dark { 0.20 } else { 0.06 };
        let card_hover = if matches!(kind, Kind::Toggle) && st.enabled { a.hover } else { 0.0 };
        cv.fill_round(r.card, 7.0 * s, [1.0; 3], card_a + 0.025 * card_hover);
        cv.stroke_round(r.card, 7.0 * s, s, [0.0; 3], border_a);

        ui::glyph(cv, &mut self.text, RectF::new(r.card.x + 18.0 * s, r.card.y, 26.0 * s, r.card.h), icon, Font::IconMd, fg, 0.92 * en);
        let tx = r.card.x + 60.0 * s;
        let tw = if matches!(kind, Kind::Info) { r.card.right() - tx - 20.0 * s } else { r.ctl.x - tx - 24.0 * s };
        self.label(cv, tr(title), Font::Body, tx, r.card.y + 12.0 * s, tw, 21.0 * s, fg, en);
        self.label(cv, tr(desc), Font::Caption, tx, r.card.y + 34.0 * s, tw, 19.0 * s, fg, 0.62 * en);

        let v = st.v;
        match kind {
            Kind::Toggle => self.draw_toggle(cv, r.ctl, v != 0, a.toggle, card_hover, en),
            Kind::Segmented(opts) => self.draw_segmented(cv, r, opts, v as usize, a.seg, st.seg_hover, en),
            Kind::Choice(names) => {
                self.draw_box(cv, r.ctl, a.hover, en);
                let name = tr(names.get(v.max(0) as usize).copied().unwrap_or(""));
                self.label(cv, name, Font::Body, r.ctl.x + 12.0 * s, r.ctl.y, r.ctl.w - 44.0 * s, r.ctl.h, fg, en);
                ui::glyph(cv, &mut self.text, RectF::new(r.ctl.right() - 32.0 * s, r.ctl.y, 24.0 * s, r.ctl.h), "\u{E70D}", Font::IconSmall, fg, 0.8 * en);
            }
            Kind::Slider { min, max, .. } => {
                self.draw_slider(cv, r.ctl, (v - min) as f32 / (max - min) as f32, a.hover, en);
                self.label_right(cv, &value_label(*kind, *key, v), Font::Caption, r.ctl.x - 12.0 * s, r.ctl.cy(), 20.0 * s, fg, 0.75 * en);
            }
            Kind::Button(t) => {
                self.draw_box(cv, r.ctl, a.hover, en);
                ui::text_center(cv, &mut self.text, tr(t), Font::Body, r.ctl, fg, en);
            }
            Kind::Info => {}
        }
    }

    /// Dica (tooltip) sobre uma opção desativada: explica o que ligar antes.
    fn draw_tip(&mut self, cv: &mut Canvas, w: f32, h: f32) {
        let a = self.tip_alpha();
        let (ri, x, y) = match &self.tip {
            Some(t) if a > 0.0 => (t.ri, t.x, t.y),
            _ => return,
        };
        let cfg = self.cfg();
        let Some((key, _)) = self.row(ri) else { return };
        let Some(why) = requirement(&cfg, key) else { return };
        let s = self.scale;
        let fg = self.fg();
        let msg = tr(why);
        let th = 32.0 * s;
        let tw = self.text.mask(msg, Font::Caption, (720.0 * s) as i32, th as i32, Align::Left).used_width();
        let bw = (tw + 44.0 * s).round();
        let mut r = RectF::new((x + 2.0 * s).round(), (y + 24.0 * s).round(), bw, th);
        if r.right() > w - 8.0 * s {
            r.x = (w - 8.0 * s - bw).round();
        }
        if r.bottom() > h - 8.0 * s {
            r.y = (y - 14.0 * s - th).round();
        }
        let r = r.offset(0.0, ((1.0 - ui::ease_enter(a)) * 4.0 * s).round());
        let dark = self.dark;
        cv.drop_shadow(r, 5.0 * s, 12.0 * s, 3.0 * s, (if dark { 0.45 } else { 0.18 }) * a);
        cv.fill_round(r, 5.0 * s, if dark { [0.17, 0.17, 0.18] } else { [0.985, 0.985, 0.99] }, a);
        cv.stroke_round(r, 5.0 * s, s, fg, (if dark { 0.12 } else { 0.10 }) * a);
        let accent = self.accent;
        ui::glyph(cv, &mut self.text, RectF::new(r.x + 9.0 * s, r.y, 20.0 * s, r.h), "\u{E946}", Font::IconSmall, accent, a);
        self.label(cv, msg, Font::Caption, r.x + 33.0 * s, r.y, bw - 36.0 * s, r.h, fg, 0.92 * a);
    }

    /// Fundo fixo da prévia (papel de parede e, no widget, a barra simulada).
    fn hero_static(&mut self, cv: &mut Canvas, r: RectF, preview: Preview, light_tb: bool, s: f32) {
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
        // "Janelas" coloridas no papel de parede das prévias de cartões: dão
        // forma ao que o acrílico desfoca (num degradê liso ele não aparece).
        if matches!(preview, Preview::Card | Preview::UpNext | Preview::LockKeys) {
            // (deslocamento do centro e largura em px lógicos; topo e altura em fração da prévia)
            let shapes: [(f32, f32, f32, f32, Rgb); 4] = [
                (-330.0, 0.20, 250.0, 0.55, [0.98, 0.45, 0.55]),
                (-110.0, 0.06, 190.0, 0.36, [1.0, 0.78, 0.35]),
                (40.0, 0.54, 240.0, 0.40, [0.30, 0.80, 0.95]),
                (220.0, 0.10, 150.0, 0.50, [0.62, 0.45, 1.0]),
            ];
            for (dx, v, w, h, c) in shapes {
                let b = RectF::new(r.cx() + dx * s, r.y + r.h * v, w * s, r.h * h);
                cv.fill_round(b, 10.0 * s, c, if dark { 0.55 } else { 0.65 });
            }
        }
        cv.stroke_round(r, radius, s, [0.0; 3], if dark { 0.3 } else { 0.08 });
        self.label(cv, tr("PRÉ-VISUALIZAÇÃO"), Font::Label, r.x + 16.0 * s, r.y + 12.0 * s, 200.0 * s, 16.0 * s, [1.0; 3], 0.75);

        if matches!(preview, Preview::Widget | Preview::Visualizer) {
            let bar = RectF::new(r.x, r.bottom() - 48.0 * s, r.w, 48.0 * s);
            let bar_c: Rgb = if light_tb { [0.93, 0.93, 0.94] } else { [0.11, 0.11, 0.12] };
            cv.fill_rrect_ca(bar.x, bar.y, bar.right(), bar.bottom(), radius, |_, y| (bar_c, if y < bar.y + radius { 0.0 } else { 0.94 }));
            cv.fill_rrect(bar.x, bar.y, bar.right(), bar.y + radius, 0.0, 0.94, bar_c);
            cv.fill_rrect(bar.x, bar.y, bar.right(), bar.y + s, 0.0, 0.12, if light_tb { [0.0; 3] } else { [1.0; 3] });
            let icon_c: Rgb = if light_tb { [0.2; 3] } else { [0.85; 3] };
            for k in 0..5 {
                let ix = bar.x + 24.0 * s + k as f32 * 40.0 * s;
                cv.fill_round(RectF::new(ix, bar.cy() - 11.0 * s, 22.0 * s, 22.0 * s), 5.0 * s, icon_c, if k == 0 { 0.9 } else { 0.35 });
            }
            self.label(cv, "12:34", Font::Caption, bar.right() - 56.0 * s, bar.y + 6.0 * s, 50.0 * s, 18.0 * s, icon_c, 0.9);
            self.label(cv, "23/11", Font::Caption, bar.right() - 56.0 * s, bar.y + 24.0 * s, 50.0 * s, 18.0 * s, icon_c, 0.7);
        }
    }

    fn hero_sig(&mut self, hr: RectF, cfg: &Config) -> (HeroSig, f32) {
        let page = &PAGES[self.page];
        let s = self.preview_scale(page, cfg);
        let light_tb = self.colors(cfg.palette).light_taskbar;
        let key = HeroKey { preview: page.preview, w: hr.w as i32, h: hr.h as i32, scale: s.to_bits(), dark: self.dark, mica: self.mica, light_tb };
        let fps = self.hero_fps(cfg);
        let t = self.t0.elapsed().as_secs_f64();
        let frame = match page.preview {
            // Aviso de teclas: anima só no começo de cada meio ciclo (liga/desliga a
            // cada 2 s); no resto o quadro é igual e não é refeito.
            Preview::LockKeys if t % 2.0 > 0.55 => 1_000_000 + (t / 2.0) as u64,
            _ if fps > 0.0 => (t * fps as f64) as u64,
            _ => 0,
        };
        let media = self.shared.media_version.load(Ordering::Relaxed);
        (HeroSig { key, cfg: self.cfg_version(), media, frame }, s)
    }

    /// Retângulo do componente dentro da prévia (o mesmo que `draw_preview` usa).
    fn preview_rect(&self, r: RectF, preview: Preview, cfg: &Config, s: f32) -> RectF {
        let rect = match preview {
            Preview::Widget | Preview::Visualizer => {
                let mut c2 = cfg.clone();
                if preview == Preview::Visualizer {
                    c2.show_visualizer = true;
                }
                let bar = RectF::new(r.x, r.bottom() - 48.0 * s, r.w, 48.0 * s);
                let wh = 40.0 * s;
                let l = ui::WLayout::build(s, wh, &c2, c2.show_media, c2.show_visualizer);
                let wx = (bar.right() - 76.0 * s - l.width).max(bar.x + 8.0 * s);
                RectF::new(wx, bar.y + 4.0 * s, l.width, wh)
            }
            Preview::Card => {
                let l = ui::CLayout::build(s, cfg, true);
                RectF::new(r.cx() - l.w / 2.0, r.y + 4.0 * s, l.w, l.h)
            }
            Preview::UpNext => {
                let (uw, uh) = ui::upnext_size(s);
                RectF::new(r.cx() - uw / 2.0, r.y + 12.0 * s, uw, uh)
            }
            Preview::LockKeys => {
                let (ow, oh) = ui::osd_size(s, cfg);
                RectF::new(r.cx() - ow / 2.0, r.y + 22.0 * s, ow, oh)
            }
            Preview::None => RectF::default(),
        };
        RectF::new(rect.x.round(), rect.y.round(), rect.w.ceil(), rect.h.ceil())
    }

    /// Quadro da prévia (fundo em cache + componente animado), refeito só
    /// quando a assinatura muda. Com o fundo igual, só a área do componente
    /// (onde ele estava e onde está agora) é refeita. Devolve essa área
    /// (coordenadas da prévia) ou `None` se nada mudou.
    fn ensure_hero(&mut self, hr: RectF) -> Option<RectF> {
        let cfg = self.cfg();
        let (sig, s) = self.hero_sig(hr, &cfg);
        if self.hero_frame.as_ref().is_some_and(|f| f.key == sig) {
            return None;
        }
        let t0 = Instant::now();
        let preview = sig.key.preview;
        let (hw, hh) = (hr.w.max(1.0) as usize, hr.h.max(1.0) as usize);
        let local = RectF::new(0.0, 0.0, hr.w, hr.h);
        let comp = self.preview_rect(local, preview, &cfg, s);
        // Só o componente mudou: restaura o fundo na área dele e redesenha.
        let same_bg = self.hero_bg.as_ref().is_some_and(|c| c.key == sig.key);
        if same_bg && self.hero_frame.as_ref().is_some_and(|f| f.key.key == sig.key && f.w == hw && f.h == hh) {
            let prev = self.hero_comp.unwrap_or(comp);
            let x0 = prev.x.min(comp.x).max(0.0);
            let y0 = prev.y.min(comp.y).max(0.0);
            let x1 = prev.right().max(comp.right()).min(hw as f32);
            let y1 = prev.bottom().max(comp.bottom()).min(hh as f32);
            let dirty = RectF::new(x0.floor(), y0.floor(), (x1.ceil() - x0.floor()).max(0.0), (y1.ceil() - y0.floor()).max(0.0));
            let (Some(mut f), Some(bg)) = (self.hero_frame.take(), self.hero_bg.as_ref()) else { return Some(local) };
            let (dx0, dx1) = (dirty.x as usize, dirty.right() as usize);
            for y in dirty.y as usize..dirty.bottom() as usize {
                f.px[y * hw + dx0..y * hw + dx1].copy_from_slice(&bg.px[y * hw + dx0..y * hw + dx1]);
            }
            {
                let mut cv = Canvas::new(hw, hh, &mut f.px);
                cv.set_clip(dirty);
                self.draw_preview(&mut cv, local, preview, &cfg, s);
            }
            f.key = sig;
            self.hero_frame = Some(f);
            self.hero_comp = Some(comp);
            if perf_log() {
                crate::audio::log::info(&format!("PERF   hero part {:.0}x{:.0} {:.2}ms", dirty.w, dirty.h, t0.elapsed().as_secs_f64() * 1e3));
            }
            return Some(dirty);
        }
        if !self.hero_bg.as_ref().is_some_and(|c| c.key == sig.key) {
            let mut px = vec![self.base_px(); hw * hh];
            {
                let mut hc = Canvas::new(hw, hh, &mut px);
                self.hero_static(&mut hc, local, preview, sig.key.light_tb, s);
            }
            self.hero_bg = Some(Cached { key: sig.key, w: hw, h: hh, px });
            self.backdrop = None;
        }
        let mut px = self.hero_frame.take().map(|f| f.px).unwrap_or_default();
        px.clear();
        if let Some(bg) = &self.hero_bg {
            px.extend_from_slice(&bg.px);
        }
        px.resize(hw * hh, self.base_px());
        {
            let mut cv = Canvas::new(hw, hh, &mut px);
            self.draw_preview(&mut cv, local, preview, &cfg, s);
        }
        self.hero_frame = Some(Cached { key: sig, w: hw, h: hh, px });
        self.hero_comp = Some(comp);
        if perf_log() {
            crate::audio::log::info(&format!("PERF   hero {:.2}ms", t0.elapsed().as_secs_f64() * 1e3));
        }
        Some(local)
    }

    /// Componente ao vivo da prévia, desenhado sobre o fundo dela (`r` local).
    fn draw_preview(&mut self, cv: &mut Canvas, r: RectF, preview: Preview, cfg: &Config, s: f32) {
        // Texto próprio (escala da prévia), para não descartar o cache da tela.
        self.preview_text.set_scale(s);
        let t = self.t0.elapsed().as_secs_f32();
        let mut colors = self.colors(cfg.palette);
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
                // Pausada de verdade: barras em repouso, como na barra (a página
                // das barras continua animando, para ver o estilo escolhido).
                let rest = preview == Preview::Widget && live && !media.playing;
                let values: Vec<f32> = (0..c2.bars)
                    .map(|i| if rest { 0.0 } else { (0.5 + 0.45 * ((t * 5.0 + i as f32 * 0.9).sin() * (t * 2.3 + i as f32 * 0.4).cos())).clamp(0.05, 1.0) })
                    .collect();
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
                    change: None,
                };
                ui::draw_widget(&mut sub.canvas(), &mut self.preview_text, &l, &f);
                sub.commit(cv);
            }
            Preview::Card => {
                let l = ui::CLayout::build(s, cfg, true);
                let th = ui::card_theme(cfg, &colors, &media);
                let mut m2 = media.clone();
                m2.dur = m2.dur.max(1.0);
                let mut sub = SubCanvas::new(r.cx() - l.w / 2.0, r.y + 4.0 * s, l.w, l.h);
                let backdrop = ui::card_wants_backdrop(cfg).then(|| self.preview_backdrop(l.card.offset(sub.x, sub.y), ui::GLASS_BLUR * s)).flatten();
                let f = ui::CardFrame { cfg, colors: &colors, media: &m2, theme: th, it: Interact::default(), pb, s, backdrop: backdrop.as_deref(), hint: None };
                ui::draw_card(&mut sub.canvas(), &mut self.preview_text, &l, &f);
                sub.commit(cv);
            }
            Preview::UpNext => {
                let (uw, uh) = ui::upnext_size(s);
                let total = cfg.popup_secs as f32 + 0.5;
                let tt = (t % (total + 1.2)).min(total - 0.6);
                let th = ui::card_theme(cfg, &colors, &media);
                let mut sub = SubCanvas::new(r.cx() - uw / 2.0, r.y + 12.0 * s, uw, uh);
                let backdrop = ui::card_wants_backdrop(cfg).then(|| self.preview_backdrop(ui::upnext_card(s).offset(sub.x, sub.y), ui::GLASS_BLUR * s)).flatten();
                ui::draw_upnext(&mut sub.canvas(), &mut self.preview_text, cfg, &colors, &media, &th, s, tt, total, backdrop.as_deref());
                sub.commit(cv);
            }
            Preview::LockKeys => {
                let (ow, oh) = ui::osd_size(s, cfg);
                let cycle = t % 4.0;
                let on = cycle < 2.0 || !cfg.osd_show_off;
                let tt = cycle % 2.0;
                let th = ui::osd_theme(cfg, &colors);
                let mut sub = SubCanvas::new(r.cx() - ow / 2.0, r.y + 22.0 * s, ow, oh);
                let m = ui::SHADOW_MARGIN * s;
                let (cw, ch) = ui::osd_card(s, cfg);
                let card = RectF::new(sub.x + m, sub.y + m, cw, ch);
                let backdrop = ui::osd_wants_backdrop(cfg).then(|| self.preview_backdrop(card, ui::osd_blur_px(cfg, s))).flatten();
                ui::draw_osd(&mut sub.canvas(), &mut self.preview_text, cfg, &colors, &th, s, tr("Caps Lock"), false, on, tt, backdrop.as_deref());
                sub.commit(cv);
            }
            Preview::None => {}
        }
    }

    /// Acrílico da prévia: o "papel de parede" (em cache) atrás de `card`
    /// (coordenadas da prévia), desfocado.
    fn preview_backdrop(&mut self, card: RectF, blur: f32) -> Option<Arc<Image>> {
        let bg = self.hero_bg.as_ref()?;
        let key: BackdropKey = (card.x.round() as i32, card.y.round() as i32, card.w.round() as i32, card.h.round() as i32, blur.round() as i32, bg.key);
        if let Some((k, img)) = &self.backdrop {
            if *k == key {
                return Some(img.clone());
            }
        }
        let img = Arc::new(render::acrylic_backdrop(&bg.px, bg.w, bg.h, card, blur));
        self.backdrop = Some((key, img.clone()));
        Some(img)
    }

    fn draw_box(&mut self, cv: &mut Canvas, r: RectF, hot: f32, en: f32) {
        let s = self.scale;
        let a = if self.dark { 0.061 + 0.023 * hot } else { 0.7 - 0.2 * hot };
        cv.fill_round(r, 5.0 * s, [1.0; 3], a * en.max(0.6));
        cv.stroke_round(r, 5.0 * s, s, if self.dark { [1.0; 3] } else { [0.0; 3] }, if self.dark { 0.07 } else { 0.09 });
    }

    /// Controle segmentado (estilo SelectorBar do WinUI): o item escolhido vira
    /// uma "pílula" elevada que desliza até a nova opção.
    fn draw_segmented(&mut self, cv: &mut Canvas, r: &RowL, opts: &[&'static str], sel: usize, pos: f32, hover: i8, en: f32) {
        let s = self.scale;
        let fg = self.fg();
        let edge: Rgb = if self.dark { [1.0; 3] } else { [0.0; 3] };
        cv.fill_round(r.ctl, 8.0 * s, edge, 0.045);
        cv.stroke_round(r.ctl, 8.0 * s, s, edge, 0.06);
        if r.segs.is_empty() {
            return;
        }
        for (k, seg) in r.segs.iter().enumerate() {
            if hover == k as i8 && k != sel {
                cv.fill_round(*seg, 6.0 * s, fg, 0.05);
            }
        }
        let p = pos.clamp(0.0, (r.segs.len() - 1) as f32);
        let (i0, i1) = (p.floor() as usize, (p.ceil() as usize).min(r.segs.len() - 1));
        let pill = r.segs[i0].lerp(r.segs[i1], p - i0 as f32);
        cv.drop_shadow(pill, 6.0 * s, 4.0 * s, 1.0 * s, (if self.dark { 0.35 } else { 0.12 }) * en);
        cv.fill_round(pill, 6.0 * s, if self.dark { [0.24, 0.24, 0.26] } else { [1.0; 3] }, if en < 1.0 { 0.6 } else { 1.0 });
        cv.stroke_round(pill, 6.0 * s, s, edge, 0.08);
        // Sublinhado na cor de destaque: estica no meio do caminho.
        let travel = 1.0 - ((p - p.round()).abs() * 2.0 - 1.0).abs();
        let uw = 16.0 * s * (1.0 + travel * 1.2);
        cv.fill_round(RectF::new(pill.cx() - uw / 2.0, pill.bottom() - 3.5 * s, uw, 3.0 * s), 1.5 * s, self.accent, en);
        for (k, seg) in r.segs.iter().enumerate() {
            let font = if k == sel { Font::BodyStrong } else { Font::Body };
            ui::text_center(cv, &mut self.text, tr(opts[k]), font, seg.offset(0.0, -1.0 * s), fg, (if k == sel { 1.0 } else { 0.78 }) * en);
        }
    }

    /// Toggle switch do WinUI: trilho 40×20; ligado = cor de destaque. `t`:
    /// posição animada da bolinha; `hot`: realce de mouse em cima (0..1).
    fn draw_toggle(&mut self, cv: &mut Canvas, r: RectF, on: bool, t: f32, hot: f32, en: f32) {
        let s = self.scale;
        let fg = self.fg();
        let e = ui::ease_move(t);
        if hot > 0.0 && e < 1.0 {
            cv.fill_round(r, r.h / 2.0, fg, 0.05 * hot * (1.0 - e));
        }
        cv.stroke_round(r, r.h / 2.0, s, fg, 0.6 * (1.0 - e) * en);
        cv.fill_round(r, r.h / 2.0, self.accent, e * (1.0 - 0.1 * hot) * en.max(0.5));
        let x = r.x + 10.0 * s + (r.w - 20.0 * s) * e;
        let knob = (6.0 + hot) * s;
        // A bolinha estica um pouco enquanto anda (como no WinUI).
        let stretch = (1.0 - (2.0 * e - 1.0).abs()) * 3.0 * s;
        let kc = lerp3(fg, self.on_accent(), e);
        cv.fill_rrect(x - knob - stretch / 2.0, r.cy() - knob, x + knob + stretch / 2.0, r.cy() + knob, knob, (0.8 + 0.2 * e) * en, kc);
        let label = tr(if on { "Ligado" } else { "Desligado" });
        self.label_right(cv, label, Font::Body, r.x - 12.0 * s, r.cy(), 22.0 * s, fg, 0.9 * en);
    }

    fn draw_slider(&mut self, cv: &mut Canvas, r: RectF, t: f32, hot: f32, en: f32) {
        let s = self.scale;
        let fg = self.fg();
        let (x0, x1, cy) = (r.x + 10.0 * s, r.right() - 10.0 * s, r.cy());
        let th = 4.0 * s;
        let xv = x0 + (x1 - x0) * t.clamp(0.0, 1.0);
        cv.fill_rrect(x0, cy - th / 2.0, x1, cy + th / 2.0, th / 2.0, if self.dark { 0.28 } else { 0.30 }, fg);
        if xv > x0 {
            cv.fill_rrect(x0, cy - th / 2.0, xv, cy + th / 2.0, th / 2.0, en.max(0.5), self.accent);
        }
        let ring: Rgb = if self.dark { [0.27; 3] } else { [1.0; 3] };
        cv.drop_shadow(RectF::new(xv - 10.0 * s, cy - 10.0 * s, 20.0 * s, 20.0 * s), 10.0 * s, 4.0 * s, 1.0 * s, 0.25 * en);
        cv.fill_circle(xv, cy, 10.0 * s, ring, 1.0);
        cv.fill_circle(xv, cy, (5.0 + 2.0 * hot) * s, self.accent, en.max(0.5));
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
        cv.fill_rrect(x, base - w * hgt, x + bw, base, bw / 2.0, 1.0, [1.0; 3]);
    }
    // "barra de tarefas" sob as barras
    cv.fill_rrect(r.x + w * 0.2, r.y + w * 0.77, r.right() - w * 0.2, r.y + w * 0.83, w * 0.03, 0.9, [1.0; 3]);
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
        cv.composite(self.x as i32, self.y as i32, &self.px, self.w, self.h, 1.0);
    }
}

// ---------------------------------------------------------------- interação

enum Action {
    None,
    Redraw,
    Menu { ri: usize, names: &'static [&'static str], current: i32, at: RECT },
}

fn activate(u: &mut Ui, hit: Hit) -> Action {
    match hit {
        Hit::Nav(i) => {
            if u.page != i {
                u.page = i;
                u.scroll = 0.0;
                u.scroll_target = 0.0;
                u.hero_bg = None;
                u.hero_frame = None;
                u.backdrop = None;
                u.layout = None;
                u.tip = None;
                u.anims.clear();
                u.page_anim = None;
                u.page_anim_pending = true;
            }
            u.hover = None;
            u.schedule();
            Action::Redraw
        }
        Hit::Seg(ri, k) => {
            if let Some((key, _)) = u.row(ri) {
                u.shared.update_config(|c| set(c, key, k as i32));
                if key == Key::OsdPos {
                    u.shared.osd_test.store(true, Ordering::Relaxed);
                }
            }
            u.schedule();
            Action::Redraw
        }
        Hit::Off(_) => Action::None,
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
                    u.schedule();
                    Action::Redraw
                }
                Kind::Choice(names) => {
                    let sc = u.scroll_px();
                    let ctl = u.layout().rows[ri].ctl.offset(0.0, -sc);
                    let mut tl = POINT { x: ctl.x as i32, y: ctl.y as i32 };
                    let mut br = POINT { x: ctl.right() as i32, y: ctl.bottom() as i32 };
                    unsafe {
                        let _ = ClientToScreen(u.hwnd, &mut tl);
                        let _ = ClientToScreen(u.hwnd, &mut br);
                    }
                    Action::Menu { ri, names, current: get(&cfg, key), at: RECT { left: tl.x, top: tl.y, right: br.x, bottom: br.y } }
                }
                Kind::Button(_) if key == Key::Reset => {
                    u.shared.update_config(|c| *c = Config::default());
                    crate::i18n::apply(Language::Auto);
                    u.schedule();
                    Action::Redraw
                }
                Kind::Button(_) if key == Key::Licenses => {
                    crate::licenses::open();
                    Action::Redraw
                }
                Kind::Button(_) if key == Key::OsdTest => {
                    u.shared.osd_test.store(true, Ordering::Relaxed);
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

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn invalidate_rect(hwnd: HWND, r: RectF) {
    if r.is_empty() {
        return;
    }
    let rc = RECT { left: r.x.floor() as i32 - 1, top: r.y.floor() as i32 - 1, right: r.right().ceil() as i32 + 1, bottom: r.bottom().ceil() as i32 + 1 };
    unsafe {
        let _ = InvalidateRect(Some(hwnd), Some(&rc), false);
    }
}

/// Valor de slider em pré-visualização (sem gravar); move o aviso ao vivo nos ajustes de posição.
fn preview_slider(u: &mut Ui, key: Key, v: i32) {
    u.shared.preview_config(|c| set(c, key, v));
    if matches!(key, Key::OsdOffX | Key::OsdOffY) {
        u.shared.osd_test.store(true, Ordering::Relaxed);
    }
}

impl Ui {
    /// Retângulo (janela) de uma linha visível, recortado à área da lista.
    fn row_window_rect(&mut self, ri: usize) -> RectF {
        let (w, h) = self.client();
        let l = self.layout();
        let Some(r) = l.rows.get(ri) else { return RectF::default() };
        r.card.offset(0.0, -self.scroll_px()).inset(-2.0).intersect(RectF::new(0.0, l.list_top, w, h - l.list_top))
    }

    fn list_rect(&mut self) -> RectF {
        let (w, h) = self.client();
        let s = self.scale;
        let lt = self.layout().list_top;
        let ox = (NAV_W * s).round();
        RectF::new(ox, lt - 1.0 * s, w - ox, h - lt + 1.0 * s)
    }

    /// Um quadro de animação: rolagem, linhas, menu lateral, troca de página,
    /// prévia e dica. Invalida só o que mudou (quem chama pinta na hora, com
    /// `UpdateWindow`, fora do `borrow` do estado).
    fn tick(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_tick).as_secs_f32().clamp(0.0, 0.05);
        self.last_tick = now;
        let (w, h) = self.client();
        let s = self.scale;
        let ox = (NAV_W * s).round();
        let l = self.layout();
        let hwnd = self.hwnd;

        // Rolagem suave independente da taxa de quadros.
        let d = self.scroll_target - self.scroll;
        if d != 0.0 {
            let old = self.scroll_px();
            self.scroll = if d.abs() < 0.5 { self.scroll_target } else { self.scroll + d * (1.0 - (-dt * 14.0).exp()) };
            if self.scroll_px() != old {
                let r = self.list_rect();
                invalidate_rect(hwnd, r);
            }
        }

        // Linhas: só as que mudaram de estado visível.
        let cfg = self.cfg();
        let before: Vec<RowState> = (0..self.anims.len()).map(|ri| self.row_state(ri, &cfg)).collect();
        if self.step_anims(dt) || before.len() != self.anims.len() {
            for ri in 0..self.anims.len() {
                if before.get(ri) != Some(&self.row_state(ri, &cfg)) {
                    let r = self.row_window_rect(ri);
                    invalidate_rect(hwnd, r);
                }
            }
        }

        // Indicador do menu lateral.
        let target = self.page as f32;
        if self.nav_pos != target {
            self.nav_pos += (target - self.nav_pos) * (1.0 - (-dt * 13.0).exp());
            if (self.nav_pos - target).abs() <= 0.002 {
                self.nav_pos = target;
            }
            invalidate_rect(hwnd, RectF::new(0.0, l.nav[0].y - 24.0 * s, ox, l.nav[PAGES.len() - 1].bottom() - l.nav[0].y + 48.0 * s));
        }

        // Troca de página: a coluna da direita entra deslizando.
        if self.page_anim.is_some() || self.page_anim_pending {
            invalidate_rect(hwnd, RectF::new(ox, 0.0, w - ox, h));
            if self.page_anim.is_some() && self.page_anim_t().is_none() {
                self.page_anim = None;
            }
        }

        // Prévia: só quando o quadro dela muda.
        if let Some(hero) = l.hero {
            let hr = RectF::new(hero.x.round(), hero.y.round(), hero.w.round(), hero.h.round());
            // Desenha o quadro novo já aqui para saber a área exata que mudou.
            if let Some(d) = self.ensure_hero(hr) {
                invalidate_rect(hwnd, d.offset(hr.x, hr.y).intersect(hr));
            }
        }

        // Dica: aparece (esmaecendo) depois de um instante parado sobre a opção.
        if self.tip.is_some() {
            let a = self.tip_alpha();
            if a > 0.0 && a <= 1.0 && self.tip.as_ref().is_some_and(|t| t.since.elapsed().as_secs_f32() < TIP_DELAY + 0.3) {
                self.invalidate_tip();
            }
        }

        self.schedule();
    }

    /// Desenha, fora da tela e em pedaços pequenos (alguns ms por vez), as
    /// linhas e a prévia das outras páginas, só para deixar textos e imagens no
    /// cache: a primeira visita a cada página não engasga. `false` = acabou.
    fn warm_step(&mut self) -> bool {
        let (i, r0) = match self.warm_pos {
            Some(p) => p,
            None => match (0..PAGES.len()).find(|i| self.warmed & (1 << i) == 0) {
                Some(i) => (i, 0),
                None => return false,
            },
        };
        let t0 = Instant::now();
        let (w, _) = self.client();
        let saved = (self.page, std::mem::take(&mut self.anims));
        self.page = i;
        let l = self.build_layout(w);
        let cfg = self.cfg();
        const BATCH: usize = 6;
        if r0 < l.rows.len() {
            for ri in r0..(r0 + BATCH).min(l.rows.len()) {
                let row = &l.rows[ri];
                let st = self.row_state(ri, &cfg);
                let (cw, ch) = (row.card.w.ceil().max(1.0) as usize, row.card.h.ceil().max(1.0) as usize);
                let mut px = vec![0u32; cw * ch];
                let local = row.shifted(-row.card.x, -row.card.y);
                self.draw_row(&mut Canvas::new(cw, ch, &mut px), ri, &local, &st);
            }
            self.warm_pos = Some((i, r0 + BATCH));
        } else {
            // Por último a prévia (capa desfocada, acrílico...).
            if let Some(hero) = l.hero {
                let sc = self.preview_scale(&PAGES[i], &cfg);
                let (hw, hh) = (hero.w.max(1.0) as usize, hero.h.max(1.0) as usize);
                let mut hp = vec![0u32; hw * hh];
                self.draw_preview(&mut Canvas::new(hw, hh, &mut hp), RectF::new(0.0, 0.0, hero.w, hero.h), PAGES[i].preview, &cfg, sc);
            }
            self.warmed |= 1 << i;
            self.warm_pos = None;
        }
        (self.page, self.anims) = saved;
        self.backdrop = None;
        if perf_log() {
            crate::audio::log::info(&format!("PERF   warm page {i} row {r0} {:.2}ms", t0.elapsed().as_secs_f64() * 1e3));
        }
        true
    }

    /// Invalida a área onde a dica aparece.
    fn invalidate_tip(&mut self) {
        if let Some(t) = &self.tip {
            let s = self.scale;
            let r = RectF::new(t.x - 760.0 * s, t.y - 70.0 * s, 1540.0 * s, 150.0 * s);
            invalidate_rect(self.hwnd, r);
        }
    }

    fn set_hover(&mut self, h: Option<Hit>, x: f32, y: f32) -> bool {
        if h == self.hover {
            return false;
        }
        let old = self.hover;
        self.hover = h;
        // Dica: começa a contar ao entrar numa linha desativada.
        match h {
            Some(Hit::Off(ri)) => {
                if !matches!(self.tip, Some(Tip { ri: r, .. }) if r == ri) {
                    self.invalidate_tip();
                    self.tip = Some(Tip { ri, x, y, since: Instant::now() });
                }
            }
            _ => {
                if self.tip.is_some() {
                    self.invalidate_tip();
                    self.tip = None;
                }
            }
        }
        // Só as linhas envolvidas (e o menu lateral, se for ele).
        for hit in [old, h].into_iter().flatten() {
            match hit {
                Hit::Nav(_) => {
                    let s = self.scale;
                    invalidate_rect(self.hwnd, RectF::new(0.0, 0.0, NAV_W * s, self.client().1));
                }
                Hit::Row(ri) | Hit::Seg(ri, _) | Hit::Off(ri) => {
                    let r = self.row_window_rect(ri);
                    invalidate_rect(self.hwnd, r);
                }
            }
        }
        self.schedule();
        true
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
                        let t0 = Instant::now();
                        {
                            let mut cv = surf.canvas();
                            cv.set_clip(RectF::new(dx as f32, dy as f32, dw as f32, dh as f32));
                            u.paint(&mut cv, w as f32, h as f32);
                        }
                        let t1 = Instant::now();
                        let _ = BitBlt(hdc, dx, dy, dw, dh, Some(surf.dc), dx, dy, SRCCOPY);
                        if perf_log() {
                            crate::audio::log::info(&format!("PERF paint {dw}x{dh} page={} scroll={:.0} paint={:.2}ms blit={:.2}ms", u.page, u.scroll, (t1 - t0).as_secs_f64() * 1e3, t1.elapsed().as_secs_f64() * 1e3));
                        }
                        u.surface = Some(surf);
                    }
                });
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_APP_FRAME => {
                with_ui(|u| {
                    if let Some(p) = &u.pacer {
                        p.ack();
                    }
                    u.tick();
                });
                let _ = UpdateWindow(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == WARM_TIMER => {
                // Uma página por vez, só com a janela parada (sem animação rodando).
                let more = with_ui(|u| {
                    // Nada de interação em andamento (a prévia pode estar animando).
                    let busy = u.drag.is_some()
                        || (u.scroll_target - u.scroll).abs() > 0.5
                        || u.anims_moving()
                        || u.nav_moving()
                        || u.page_anim.is_some()
                        || u.page_anim_pending
                        || u.surface.is_none();
                    if busy {
                        return true;
                    }
                    // A página atual já está pronta.
                    u.warmed |= 1 << u.page;
                    if u.warm_pos.is_some_and(|p| p.0 == u.page) {
                        u.warm_pos = None;
                    }
                    u.warm_step()
                })
                .unwrap_or(false);
                if !more {
                    let _ = KillTimer(Some(hwnd), WARM_TIMER);
                }
                LRESULT(0)
            }
            WM_TIMER if wp.0 == SLOW_TIMER => {
                // Prévias lentas e reserva: com o vblank mandando quadros, não faz nada a mais.
                with_ui(|u| {
                    if u.last_tick.elapsed().as_millis() > 60 {
                        u.tick();
                    }
                });
                let _ = UpdateWindow(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let mut drag_changed = false;
                with_ui(|u| {
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
                                drag_changed = true;
                                let r = u.row_window_rect(ri);
                                invalidate_rect(hwnd, r);
                                if let Some(hero) = u.layout().hero {
                                    invalidate_rect(hwnd, hero);
                                }
                            }
                        }
                        return;
                    }
                    let h = u.hit(x, y);
                    u.set_hover(h, x, y);
                });
                if drag_changed {
                    let _ = UpdateWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                with_ui(|u| {
                    u.tracking = false;
                    if u.drag.is_none() {
                        u.set_hover(None, 0.0, 0.0);
                    }
                });
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
                            let r = u.row_window_rect(ri);
                            invalidate_rect(hwnd, r);
                            if let Some(hero) = u.layout().hero {
                                invalidate_rect(hwnd, hero);
                            }
                            u.schedule();
                        }
                    }
                });
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                let (x, y) = xy(lp);
                let action = with_ui(|u| {
                    if let Some(ri) = u.drag.take() {
                        u.shared.save_config();
                        u.pressed = None;
                        let r = u.row_window_rect(ri);
                        invalidate_rect(hwnd, r);
                        u.schedule();
                        return Action::None;
                    }
                    let h = u.hit(x, y);
                    let pressed = u.pressed.take();
                    match h {
                        Some(h) if Some(h) == pressed => activate(u, h),
                        _ => Action::None,
                    }
                })
                .unwrap_or(Action::None);
                match action {
                    Action::Redraw => invalidate(hwnd),
                    Action::None => {}
                    // O menu roda um loop modal: nada de `borrow` aberto aqui.
                    Action::Menu { ri, names, current, at } => {
                        let (dark, accent) = with_ui(|u| (u.dark, u.accent)).unwrap_or((true, [0.6, 0.8, 1.0]));
                        let entries: Vec<menu::Entry> = names.iter().enumerate().map(|(k, n)| menu::Entry::radio(k as u32 + 1, tr(n), k as i32 == current)).collect();
                        let opts = menu::Options { dark, accent, min_width: 0.0 };
                        let pick = menu::popup(hwnd, entries, menu::Anchor::Below(at), opts, &mut |_| None);
                        if let Some(id) = pick {
                            with_ui(|u| {
                                if let Some((key, _)) = u.row(ri) {
                                    u.shared.update_config(|c| set(c, key, id as i32 - 1));
                                    if key == Key::Language {
                                        let _ = SetWindowTextW(hwnd, &HSTRING::from(tr("Taskbeat — Configurações")));
                                    }
                                }
                            });
                            invalidate(hwnd);
                        }
                        // O mouse pode ter saído da janela com o menu aberto.
                        with_ui(|u| {
                            u.set_hover(None, 0.0, 0.0);
                            u.tracking = false;
                        });
                    }
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32;
                with_ui(|u| {
                    let max = u.max_scroll();
                    u.scroll_target = (u.scroll_target - delta / 120.0 * 120.0 * u.scale).clamp(0.0, max);
                    // A dica some ao rolar (a linha sai de baixo do mouse).
                    if u.tip.is_some() {
                        u.invalidate_tip();
                        u.tip = None;
                    }
                    u.schedule();
                });
                LRESULT(0)
            }
            WM_SIZE => {
                with_ui(|u| {
                    u.layout = None;
                    let max = u.max_scroll();
                    u.scroll_target = u.scroll_target.clamp(0.0, max);
                    u.scroll = u.scroll.clamp(0.0, max);
                    if let Some(p) = &u.pacer {
                        p.set_monitor(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST));
                    }
                    u.schedule();
                });
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_MOVE => {
                with_ui(|u| {
                    if let Some(p) = &u.pacer {
                        p.set_monitor(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST));
                    }
                });
                DefWindowProcW(hwnd, msg, wp, lp)
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
                    u.hero_frame = None;
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
                    u.layout = None;
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
                let _ = KillTimer(Some(hwnd), SLOW_TIMER);
                let _ = KillTimer(Some(hwnd), WARM_TIMER);
                // Fora do `borrow`: parar o vblank espera a thread dele terminar.
                let ui = UI.with(|u| u.try_borrow_mut().ok().and_then(|mut g| g.take()));
                drop(ui);
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
        let off = Config { show_media: false, show_visualizer: false, caps_lock_osd: false, num_lock_osd: false, next_track_popup: false, ..Config::default() };
        let off2 = Config { taskbar_controls: false, widget_pill: false, show_artist: false, osd_material: OsdMaterial::Solid, ..Config::default() };
        for p in PAGES {
            check(p.name);
            check(p.subtitle);
            for it in p.items {
                match it {
                    Section(n) => check(n),
                    Row { title, desc, kind, key, .. } => {
                        check(title);
                        check(desc);
                        match kind {
                            Kind::Segmented(o) | Kind::Choice(o) => o.iter().for_each(|t| check(t)),
                            Kind::Button(t) => check(t),
                            _ => {}
                        }
                        // As dicas de opção desativada também.
                        for c in [&off, &off2] {
                            if let Some(why) = requirement(c, *key) {
                                check(why);
                            }
                        }
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "sem tradução: {missing:#?}");
    }

    #[test]
    fn requirements_follow_parents() {
        let c = Config { taskbar_controls: false, ..Config::default() };
        assert!(requirement(&c, Key::TaskbarShuffle).is_some());
        assert!(requirement(&c, Key::TaskbarPlay).is_some());
        assert!(requirement(&c, Key::WidgetDivider).is_some());
        let c = Config { taskbar_controls: true, show_media: true, ..Config::default() };
        assert!(requirement(&c, Key::TaskbarShuffle).is_none());
        let c = Config { show_media: false, ..Config::default() };
        assert!(requirement(&c, Key::TaskbarControls).is_some());
        assert!(requirement(&c, Key::Anchor).is_none());
    }
}
