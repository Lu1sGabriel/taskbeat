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

use crate::anim::{self, Phase, Pose};
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
/// Miniatura da animação de abrir e fechar do card (px lógicos).
const ANIM_TILE: (f32, f32) = (112.0, 50.0);
/// Uma volta da miniatura: fechado, abre, fica aberto, fecha, fechado (s).
const DEMO_GAP: f32 = 0.25;
const DEMO_HOLD: f32 = 0.7;

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
    IconAnim,
    IconStyle,
    CardViz,
    CoverShape,
    CoverSize,
    TextSize,
    WidgetPill,
    WidgetShape,
    WidgetTint,
    WidgetOpacity,
    WidgetIntensity,
    WidgetGlow,
    WidgetBorder,
    WidgetDivider,
    TaskbarShuffle,
    TaskbarPlay,
    WidgetControls,
    WidgetControlsRgb,
    WidgetSides,
    WidgetSidesAlpha,
    CardShuffle,
    Marquee,
    TitleWidth,
    Anchor,
    Offset,
    WidgetDrag,
    ResetPos,
    CardStyle,
    CardLayout,
    Theme,
    Accent,
    Corners,
    PlayShape,
    CardControls,
    CardControlsRgb,
    CardSides,
    CardSidesAlpha,
    CardSeek,
    ShowVolume,
    CardMixer,
    Autoclose,
    Launchers,
    LauncherPlace,
    LauncherApps,
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
    CardAnim,
    AnimSpeed,
    OsdTest,
    ShowViz,
    Style,
    Palette,
    Bars,
    Width,
    Sens,
    OnlyMedia,
    PlayerOnly,
    Peaks,
    Autostart,
    Language,
    Fullscreen,
    UpdateCheck,
    UpdateAuto,
    UpdateBeta,
    UpdateNow,
    Reset,
    Licenses,
    About,
    /// Cabeçalho "Avançado" (abre e fecha as opções raras da página).
    Advanced,
    AccentRgb,
    CardShadow,
    CardGrain,
    GlassBlur,
    GlassTint,
    ArtworkDim,
    BarShape,
    /// Situação da extensão do navegador (só texto, muda com a conexão).
    BrowserExt,
    /// Página "Letra" (opções em `settings_lyrics.rs`).
    Lyrics(lyrics_page::LK),
}

#[derive(Clone, Copy)]
enum Kind {
    Toggle,
    Segmented(&'static [&'static str]),
    Choice(&'static [&'static str]),
    Slider { min: i32, max: i32, step: i32, unit: &'static str },
    Button(&'static str),
    Info,
    /// Lista dos apps de música achados: interruptor em cada um e arrastar para reordenar.
    AppList,
    /// Seletor de cor: amostras, matiz/saturação, brilho e código hex.
    Color,
    /// Abre e fecha o grupo que vem logo depois (as opções raras da página).
    Expander,
    /// Lista como `Choice`, com uma miniatura do card abrindo e fechando ao
    /// lado (toca ao trocar de estilo e com o mouse em cima).
    Anim(&'static [&'static str]),
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
    /// Card aberto sobre a barra com o widget (opções que valem para os dois).
    Overview,
    Lyrics,
    None,
}

/// Uma aba. Para criar uma aba nova: um `Page` em `PAGES` (na ordem do menu),
/// as linhas dela (com `Key`s novas em `get`/`set`) e, se tiver prévia
/// própria, uma variante em `Preview` desenhada em `draw_preview`.
struct Page {
    name: &'static str,
    icon: &'static str,
    subtitle: &'static str,
    preview: Preview,
    /// Título pequeno no menu, acima desta aba (começa um grupo de abas).
    header: Option<&'static str>,
    items: &'static [Item],
}

use Item::{Row, Section};

/// Página da sobreposição de letra (módulo à parte, para mexer pouco aqui).
#[path = "settings_lyrics.rs"]
mod lyrics_page;

const SHAPES: &[&str] = &["Arredondada", "Círculo", "Quadrada"];
const PLAY_SHAPES: &[&str] = &["Círculo", "Arredondado", "Quadrado"];
const ANCHORS: &[&str] = &["Ao lado do relógio", "À esquerda"];
const CARD_STYLES: &[&str] = &["Capa desfocada", "Cor do álbum", "Acrílico"];
const CARD_LAYOUTS: &[&str] = &["Horizontal", "Vertical"];
const THEMES: &[&str] = &["Automático", "Escuro", "Claro"];
const ACCENTS: &[&str] = &["Cor da capa", "Cor do Windows", "Personalizada"];
const CARD_SHADOWS: &[&str] = &["Forte", "Suave", "Nenhuma"];
const BAR_SHAPES: &[&str] = &["Arredondadas", "Retas"];
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
const CARD_ANIMS: &[&str] = &["Suave", "Deslizar da barra", "Subir", "Descer", "Estourar", "Zoom", "Esmaecer", "Nenhuma"];
const ANIM_SPEEDS: &[&str] = &["Rápida", "Normal", "Lenta"];
const VIZ_STYLES: &[&str] = &["Barras", "Espelhado", "Onda"];
const PALETTES: &[&str] = &["Cor da capa", "Cor de destaque do Windows", "Arco-íris", "Neon", "Monocromático"];
const LANGS: &[&str] = &["Automático (idioma do Windows)", "Português (Brasil)", "English", "Español", "Deutsch"];
const LANG_V: &[Language] = &[Language::Auto, Language::Pt, Language::En, Language::Es, Language::De];
const BAR_NAMES: &[&str] = &["6", "8", "10", "12", "16", "24", "32"];
const BARS: &[usize] = &[6, 8, 10, 12, 16, 24, 32];
const SIZES: &[&str] = &["Pequeno", "Normal", "Grande"];
const COVER_SIZES: &[&str] = &["Pequena", "Normal", "Grande"];
const SECOND_LINES: &[&str] = &["Artista", "Artista · Álbum", "Álbum", "Tempo"];
const SOURCES: &[&str] = &["Só o selo", "Selo e nome", "Desligado"];
const CONTROL_COLORS: &[&str] = &["Branco", "Branco e destaque", "Cor da capa", "Cor de destaque", "Personalizada"];
const SIDE_COLORS: &[&str] = &["Igual ao botão do meio", "Branco", "Cor da capa", "Cor de destaque"];
/// Cores prontas do seletor de cor.
const SWATCHES: [u32; 12] = [0xFFFFFF, 0xC8C8CC, 0xFF6B9A, 0xFF4D4D, 0xFF9F43, 0xFFD93D, 0x4CD964, 0x2ED3B7, 0x4CC9F0, 0x5EB1FF, 0x7C83FD, 0xC77DFF];
const LAUNCHER_PLACES: &[&str] = &["No card", "Na barra", "Nos dois"];
const ICON_STYLES: &[&str] = &["Preenchidos", "Contorno"];
const TINTS: &[&str] = &["Neutro", "Cor da capa", "Cor de destaque", "Gradiente da capa", "Capa desfocada"];

/// Cabeçalho "Avançado": recolhe as linhas que vêm depois dele (até o próximo
/// grupo). O texto embaixo do título é a lista delas.
const ADVANCED: Item = Row { icon: "\u{E712}", title: "Avançado", desc: "", key: Key::Advanced, kind: Kind::Expander };

const PAGES: &[Page] = &[
    Page {
        name: "Barra de tarefas",
        icon: "\u{E7F4}",
        subtitle: "O widget compacto que fica ao lado do relógio",
        preview: Preview::Widget,
        header: None,
        items: &[
            Section("Na barra"),
            Row { icon: "\u{E8D6}", title: "Mostrar música na barra", desc: "Capa, título e artista; clique para abrir o card", key: Key::ShowMedia, kind: Kind::Toggle },
            Row { icon: "\u{E7B3}", title: "Sempre visível", desc: "Fica na barra mesmo sem nada tocando (\"Nada tocando\")", key: Key::AlwaysShow, kind: Kind::Toggle },
            Row { icon: "\u{E77B}", title: "Mostrar artista", desc: "Segunda linha embaixo do título", key: Key::ShowArtist, kind: Kind::Toggle },
            Row { icon: "\u{E8C4}", title: "Segunda linha", desc: "O que aparece embaixo do título", key: Key::SecondLine, kind: Kind::Segmented(SECOND_LINES) },
            Row { icon: "\u{E9E9}", title: "Indicador \"tocando agora\"", desc: "Mini equalizador animado ao lado do título", key: Key::Indicator, kind: Kind::Toggle },
            Row { icon: "\u{E916}", title: "Barra de progresso", desc: "Linha fina com o andamento da música", key: Key::TaskbarProgress, kind: Kind::Toggle },
            Row { icon: "\u{E895}", title: "Animar troca de música", desc: "O título novo desliza e a capa nova aparece suavemente", key: Key::TrackAnim, kind: Kind::Toggle },
            Section("Capa e texto"),
            Row { icon: "\u{E91B}", title: "Formato da capa", desc: "Como a capa aparece na barra", key: Key::CoverShape, kind: Kind::Segmented(SHAPES) },
            Row { icon: "\u{E740}", title: "Tamanho da capa", desc: "Capa menor deixa o widget mais discreto", key: Key::CoverSize, kind: Kind::Segmented(COVER_SIZES) },
            Row { icon: "\u{E8E9}", title: "Tamanho do texto", desc: "Título e segunda linha", key: Key::TextSize, kind: Kind::Segmented(SIZES) },
            Row { icon: "\u{E8C9}", title: "Rolar títulos longos", desc: "Letreiro suave quando o nome não cabe", key: Key::Marquee, kind: Kind::Toggle },
            Section("Botões"),
            Row { icon: "\u{E768}", title: "Botões de controle na barra", desc: "Anterior, play e próximo dentro do widget; desligados, ficam só no card", key: Key::TaskbarControls, kind: Kind::Toggle },
            Row { icon: "\u{E8B1}", title: "Aleatório e repetir na barra", desc: "Junto dos botões; somem quando o player não deixa mudar, se \"Aleatório e repetir\" (em Apps de música) estiver em \"Quando o player permite\". No YouTube Music do navegador eles apertam os botões da própria página", key: Key::TaskbarShuffle, kind: Kind::Toggle },
            Row { icon: "\u{E768}", title: "Botão play", desc: "Preenchido, na cor dos controles, ou só o ícone, como os outros", key: Key::TaskbarPlay, kind: Kind::Segmented(TASKBAR_PLAYS) },
            Row { icon: "\u{E790}", title: "Cor dos controles", desc: "Aleatório, anterior, play, próximo e repetir; o branco fica escuro sobre fundos claros", key: Key::WidgetControls, kind: Kind::Choice(CONTROL_COLORS) },
            Row { icon: "\u{E2B1}", title: "Cor personalizada", desc: "Qualquer cor: amostras, matiz e saturação, brilho ou o código hex", key: Key::WidgetControlsRgb, kind: Kind::Color },
            Row { icon: "\u{E892}", title: "Botões laterais", desc: "Cor do aleatório, anterior, próximo e repetir", key: Key::WidgetSides, kind: Kind::Choice(SIDE_COLORS) },
            Section("Fundo"),
            Row { icon: "\u{E7B5}", title: "Fundo em pílula", desc: "Fundo sutil sempre visível, com a música e os botões dentro", key: Key::WidgetPill, kind: Kind::Toggle },
            Row { icon: "\u{E7FB}", title: "Formato do fundo", desc: "Cantos do fundo do widget", key: Key::WidgetShape, kind: Kind::Segmented(WIDGET_SHAPES) },
            Row { icon: "\u{E790}", title: "Cor do fundo", desc: "Neutro, cor da capa ou de destaque; ou fundos fortes: gradiente vivo da capa ou a própria capa desfocada", key: Key::WidgetTint, kind: Kind::Choice(TINTS) },
            Row { icon: "\u{E706}", title: "Opacidade do fundo", desc: "Quanto o fundo aparece sobre a barra", key: Key::WidgetOpacity, kind: Kind::Slider { min: 0, max: 100, step: 2, unit: "%" } },
            Row { icon: "\u{E706}", title: "Intensidade da cor", desc: "Quanto o gradiente ou a capa desfocada cobrem a barra; o texto se ajusta para ficar legível", key: Key::WidgetIntensity, kind: Kind::Slider { min: 10, max: 100, step: 5, unit: "%" } },
            Row { icon: "\u{E9A9}", title: "Brilho da capa", desc: "Luz suave na cor da capa atrás dela", key: Key::WidgetGlow, kind: Kind::Toggle },
            Row { icon: "\u{E91B}", title: "Contorno", desc: "Borda fina em volta do fundo", key: Key::WidgetBorder, kind: Kind::Toggle },
            Section("Posição"),
            Row { icon: "\u{E7C4}", title: "Posição na barra", desc: "Onde o widget fica na barra de tarefas", key: Key::Anchor, kind: Kind::Segmented(ANCHORS) },
            Row { icon: "\u{E7C2}", title: "Arrastar para mover", desc: "Aperte e arraste o widget pela barra; um clique simples continua abrindo o card", key: Key::WidgetDrag, kind: Kind::Toggle },
            Row { icon: "\u{E72C}", title: "Restaurar posição", desc: "Volta o widget para o lado do relógio", key: Key::ResetPos, kind: Kind::Button("Restaurar") },
            ADVANCED,
            Row { icon: "\u{E8AB}", title: "Ajuste de posição", desc: "Desloca o widget ao longo da barra: para os lados ou, com a barra na vertical, para cima e para baixo", key: Key::Offset, kind: Kind::Slider { min: -4000, max: 4000, step: 10, unit: "px" } },
            Row { icon: "\u{E8D2}", title: "Largura do título", desc: "Espaço para o nome da música e do artista", key: Key::TitleWidth, kind: Kind::Slider { min: 100, max: 280, step: 10, unit: "px" } },
            Row { icon: "\u{E706}", title: "Força dos botões laterais", desc: "100% é tão forte quanto o botão do meio; menos deixa os do lado mais suaves", key: Key::WidgetSidesAlpha, kind: Kind::Slider { min: 20, max: 100, step: 5, unit: "%" } },
            Row { icon: "\u{E7FB}", title: "Separador", desc: "Linha fina entre a música e os botões", key: Key::WidgetDivider, kind: Kind::Toggle },
        ],
    },
    Page {
        name: "Card expandido",
        icon: "\u{E7C4}",
        subtitle: "Abre ao clicar na música, com todos os controles",
        preview: Preview::Card,
        header: None,
        items: &[
            Section("Estilo"),
            Row { icon: "\u{E771}", title: "Fundo", desc: "Capa desfocada, cor extraída da capa ou acrílico (desfoca o que está atrás); vale também para o \"A seguir\"", key: Key::CardStyle, kind: Kind::Segmented(CARD_STYLES) },
            Row { icon: "\u{E8A9}", title: "Layout", desc: "Capa ao lado ou capa grande em cima", key: Key::CardLayout, kind: Kind::Segmented(CARD_LAYOUTS) },
            Row { icon: "\u{E793}", title: "Tema", desc: "Segue o Windows ou fixa claro/escuro", key: Key::Theme, kind: Kind::Segmented(THEMES) },
            Row { icon: "\u{E7FB}", title: "Cantos", desc: "Raio das bordas do card e do \"A seguir\" (e do aviso de Caps Lock, com \"Iguais ao card\")", key: Key::Corners, kind: Kind::Segmented(CORNERS) },
            Row { icon: "\u{E768}", title: "Botão play", desc: "Formato do botão principal", key: Key::PlayShape, kind: Kind::Segmented(PLAY_SHAPES) },
            Row { icon: "\u{E81E}", title: "Sombra", desc: "Sombra do card e do \"A seguir\" sobre o que está atrás", key: Key::CardShadow, kind: Kind::Segmented(CARD_SHADOWS) },
            Row { icon: "\u{E8B3}", title: "Granulado", desc: "Textura fina no fundo, que tira o aspecto de plástico dos degradês", key: Key::CardGrain, kind: Kind::Toggle },
            Section("Botões"),
            Row { icon: "\u{E790}", title: "Cor dos controles", desc: "Ícones, play e botões ligados do card; o branco fica escuro sobre fundos claros", key: Key::CardControls, kind: Kind::Choice(CONTROL_COLORS) },
            Row { icon: "\u{E2B1}", title: "Cor personalizada", desc: "Qualquer cor: amostras, matiz e saturação, brilho ou o código hex", key: Key::CardControlsRgb, kind: Kind::Color },
            Row { icon: "\u{E892}", title: "Botões laterais", desc: "Cor do aleatório, anterior, próximo e repetir", key: Key::CardSides, kind: Kind::Choice(SIDE_COLORS) },
            Section("No card"),
            Row { icon: "\u{E916}", title: "Barra de progresso", desc: "Tempo decorrido e restante; arraste para pular", key: Key::CardSeek, kind: Kind::Toggle },
            Row { icon: "\u{E767}", title: "Volume", desc: "Slider de volume e botão de mudo", key: Key::ShowVolume, kind: Kind::Toggle },
            Row { icon: "\u{E9F5}", title: "Mixer de volume", desc: "Setinha embaixo do card que abre o volume de cada app com som", key: Key::CardMixer, kind: Kind::Toggle },
            Row { icon: "\u{E9E9}", title: "Barras no card", desc: "Mini visualizador ao lado do título, reagindo ao som", key: Key::CardViz, kind: Kind::Toggle },
            Section("Animação"),
            Row { icon: "\u{E768}", title: "Abrir e fechar", desc: "Como o card e o \"A seguir\" entram e saem da tela", key: Key::CardAnim, kind: Kind::Anim(CARD_ANIMS) },
            Row { icon: "\u{E916}", title: "Velocidade", desc: "Duração das animações de abrir e fechar", key: Key::AnimSpeed, kind: Kind::Segmented(ANIM_SPEEDS) },
            ADVANCED,
            Row { icon: "\u{E706}", title: "Força dos botões laterais", desc: "100% é tão forte quanto o botão do meio; menos deixa os do lado mais suaves", key: Key::CardSidesAlpha, kind: Kind::Slider { min: 20, max: 100, step: 5, unit: "%" } },
            Row { icon: "\u{E890}", title: "Desfoque do acrílico", desc: "Quanto o fundo Acrílico desfoca o que está atrás do card e do \"A seguir\"", key: Key::GlassBlur, kind: Kind::Slider { min: 0, max: 100, step: 5, unit: "%" } },
            Row { icon: "\u{E706}", title: "Película do acrílico", desc: "Quanto a cor do tema cobre o desfoque; menos deixa o card mais transparente", key: Key::GlassTint, kind: Kind::Slider { min: 20, max: 100, step: 2, unit: "%" } },
            Row { icon: "\u{E706}", title: "Escurecer a capa", desc: "Película escura sobre a capa desfocada; menos deixa a capa mais viva", key: Key::ArtworkDim, kind: Kind::Slider { min: 50, max: 150, step: 5, unit: "%" } },
            Row { icon: "\u{E916}", title: "Fechar sozinho após", desc: "Tempo sem o mouse em cima do card", key: Key::Autoclose, kind: Kind::Slider { min: 2, max: 20, step: 1, unit: "s" } },
        ],
    },
    Page {
        name: "A seguir",
        icon: "\u{E768}",
        subtitle: "Cartão com a próxima música quando a faixa termina",
        preview: Preview::UpNext,
        header: None,
        items: &[
            Section("Comportamento"),
            Row { icon: "\u{E768}", title: "Mostrar \"A seguir\"", desc: "Só quando o player troca de faixa sozinho; trocas manuais não mostram", key: Key::Popup, kind: Kind::Toggle },
            Row { icon: "\u{E916}", title: "Duração", desc: "Quanto tempo o cartão fica na tela", key: Key::PopupSecs, kind: Kind::Slider { min: 2, max: 12, step: 1, unit: "s" } },
            Section("Visual"),
            Row { icon: "\u{E771}", title: "Visual do cartão", desc: "Usa o fundo, o tema, os cantos e a sombra do card expandido", key: Key::About, kind: Kind::Info },
        ],
    },
    Page {
        name: "Visualizador",
        icon: "\u{E9E9}",
        subtitle: "Barras de áudio dinâmicas ao lado da música",
        preview: Preview::Visualizer,
        header: None,
        items: &[
            Section("Barras de áudio"),
            Row { icon: "\u{E9E9}", title: "Mostrar barras dinâmicas", desc: "Mini visualizador ao lado da música; captura o som do PC", key: Key::ShowViz, kind: Kind::Toggle },
            Row { icon: "\u{E771}", title: "Estilo", desc: "Formato das barras", key: Key::Style, kind: Kind::Segmented(VIZ_STYLES) },
            Row { icon: "\u{E7FB}", title: "Pontas das barras", desc: "Arredondadas ou retas", key: Key::BarShape, kind: Kind::Segmented(BAR_SHAPES) },
            Row { icon: "\u{E790}", title: "Cores", desc: "Cor da capa, do Windows ou paletas", key: Key::Palette, kind: Kind::Choice(PALETTES) },
            Row { icon: "\u{E8FD}", title: "Quantidade de barras", desc: "Mais barras = mais detalhe", key: Key::Bars, kind: Kind::Choice(BAR_NAMES) },
            Row { icon: "\u{E9A6}", title: "Largura", desc: "Tamanho do visualizador", key: Key::Width, kind: Kind::Slider { min: 20, max: 160, step: 4, unit: "px" } },
            Row { icon: "\u{E70E}", title: "Picos", desc: "Marcadores que caem devagar", key: Key::Peaks, kind: Kind::Toggle },
            ADVANCED,
            Row { icon: "\u{E9D9}", title: "Sensibilidade", desc: "Ganho extra aplicado ao espectro", key: Key::Sens, kind: Kind::Slider { min: -12, max: 18, step: 1, unit: "dB" } },
            Row { icon: "\u{E8D6}", title: "Só o som do player", desc: "As barras ouvem só o app que está tocando: chamadas, jogos e avisos não mexem nelas", key: Key::PlayerOnly, kind: Kind::Toggle },
            Row { icon: "\u{E8B1}", title: "Só quando um player estiver tocando", desc: "Ignora jogos, chamadas e notificações", key: Key::OnlyMedia, kind: Kind::Toggle },
        ],
    },
    lyrics_page::PAGE,
    Page {
        name: "Caps Lock",
        icon: "\u{E765}",
        subtitle: "Aviso na tela ao ligar ou desligar Caps Lock e Num Lock",
        preview: Preview::LockKeys,
        header: None,
        items: &[
            Section("Avisos"),
            Row { icon: "\u{E765}", title: "Caps Lock", desc: "Mostra quando as maiúsculas forem ativadas ou desativadas", key: Key::CapsOsd, kind: Kind::Toggle },
            Row { icon: "\u{E765}", title: "Num Lock", desc: "Mostra quando o teclado numérico mudar", key: Key::NumOsd, kind: Kind::Toggle },
            Row { icon: "\u{E7E8}", title: "Avisar também ao desligar", desc: "Desligado: só aparece quando a tecla é ativada", key: Key::OsdShowOff, kind: Kind::Toggle },
            Row { icon: "\u{E768}", title: "Testar", desc: "Mostra o aviso agora, na posição escolhida", key: Key::OsdTest, kind: Kind::Button("Mostrar agora") },
            Section("Aparência"),
            Row { icon: "\u{E771}", title: "Material", desc: "Sólido, acrílico (vidro fosco que desfoca o que está atrás) ou na cor de destaque", key: Key::OsdMaterial, kind: Kind::Segmented(OSD_MATERIALS) },
            Row { icon: "\u{E706}", title: "Opacidade", desc: "Quanto o fundo cobre o que está atrás", key: Key::OsdOpacity, kind: Kind::Slider { min: 20, max: 100, step: 2, unit: "%" } },
            Row { icon: "\u{E890}", title: "Desfoque", desc: "Intensidade do desfoque no fundo acrílico", key: Key::OsdBlur, kind: Kind::Slider { min: 0, max: 100, step: 5, unit: "%" } },
            Row { icon: "\u{E72E}", title: "Ícone", desc: "Letra (A / 1), cadeado que abre e fecha, ou teclado", key: Key::OsdIcon, kind: Kind::Segmented(OSD_ICONS) },
            Row { icon: "\u{E7FB}", title: "Cantos", desc: "Formato do aviso e do ícone", key: Key::OsdShape, kind: Kind::Segmented(OSD_SHAPES) },
            Row { icon: "\u{E8D3}", title: "Tamanho", desc: "Tamanho do aviso e do texto", key: Key::OsdSize, kind: Kind::Segmented(OSD_SIZES) },
            Row { icon: "\u{E793}", title: "Tema", desc: "Segue o Windows ou fixa claro/escuro", key: Key::OsdTheme, kind: Kind::Segmented(THEMES) },
            Row { icon: "\u{E7E8}", title: "Interruptor", desc: "Chave animada no lado direito", key: Key::OsdSwitch, kind: Kind::Toggle },
            Section("Na tela"),
            Row { icon: "\u{E7C4}", title: "Onde aparece", desc: "Predefinições rápidas; ajuste fino em \"Avançado\"", key: Key::OsdPos, kind: Kind::Segmented(OSD_POSITIONS) },
            Row { icon: "\u{E768}", title: "Animação", desc: "Como o aviso entra e sai da tela", key: Key::OsdAnim, kind: Kind::Segmented(OSD_ANIMS) },
            Row { icon: "\u{E916}", title: "Duração", desc: "Quanto tempo o aviso fica na tela", key: Key::OsdDuration, kind: Kind::Slider { min: 8, max: 50, step: 1, unit: "s" } },
            Section("Privacidade"),
            Row { icon: "\u{E72E}", title: "Privacidade", desc: "Só essas duas teclas são observadas; nada do que você digita é lido ou guardado", key: Key::About, kind: Kind::Info },
            ADVANCED,
            Row { icon: "\u{E8AB}", title: "Ajuste horizontal", desc: "Move o aviso para a esquerda ou para a direita", key: Key::OsdOffX, kind: Kind::Slider { min: -800, max: 800, step: 10, unit: "px" } },
            Row { icon: "\u{E8CB}", title: "Ajuste vertical", desc: "Move o aviso para cima ou para baixo", key: Key::OsdOffY, kind: Kind::Slider { min: -600, max: 600, step: 10, unit: "px" } },
        ],
    },
    Page {
        name: "Aparência",
        icon: "\u{E771}",
        subtitle: "Cores e ícones da barra, do card e do \"A seguir\"",
        preview: Preview::Overview,
        header: Some("Para todos"),
        items: &[
            Section("Cores"),
            Row { icon: "\u{E790}", title: "Cor de destaque", desc: "Play, progresso e botões ligados na barra, no card e no \"A seguir\"", key: Key::Accent, kind: Kind::Segmented(ACCENTS) },
            Row { icon: "\u{E2B1}", title: "Cor personalizada", desc: "Qualquer cor: amostras, matiz e saturação, brilho ou o código hex", key: Key::AccentRgb, kind: Kind::Color },
            Section("Ícones"),
            Row { icon: "\u{E8D3}", title: "Estilo dos ícones", desc: "Formas preenchidas e encorpadas ou o contorno fino do Fluent (no card e na barra)", key: Key::IconStyle, kind: Kind::Segmented(ICON_STYLES) },
            Row { icon: "\u{E945}", title: "Animar ícones", desc: "Play/pausa, pular, aleatório, repetir e volume se mexem ao mudar (no card e na barra)", key: Key::IconAnim, kind: Kind::Toggle },
        ],
    },
    Page {
        name: "Apps de música",
        icon: "\u{E8F1}",
        subtitle: "Atalhos para abrir os players e o que vem de cada um",
        preview: Preview::Overview,
        header: None,
        items: &[
            Section("Atalhos"),
            Row { icon: "\u{E71D}", title: "Atalhos dos apps de música", desc: "Ícones dos players instalados para abrir com um clique; vêm ligados só os de música (VLC e Media Player ficam desligados até você ligar)", key: Key::Launchers, kind: Kind::Toggle },
            Row { icon: "\u{E7C4}", title: "Onde ficam os atalhos", desc: "Embaixo do card, dentro do widget na barra ou nos dois", key: Key::LauncherPlace, kind: Kind::Segmented(LAUNCHER_PLACES) },
            Row { icon: "\u{E8FD}", title: "Apps nos atalhos", desc: "Ligue só os que você quer; arraste para mudar a ordem (no card também dá para arrastar os ícones)", key: Key::LauncherApps, kind: Kind::AppList },
            Section("Players"),
            Row { icon: "\u{E7F4}", title: "App de origem", desc: "Selo com o ícone do app (Spotify, Brave…) no canto da capa, na barra, no card e no \"A seguir\"; com o nome, ele aparece também na linha do artista", key: Key::ShowSource, kind: Kind::Segmented(SOURCES) },
            Row { icon: "\u{E8B1}", title: "Aleatório e repetir", desc: "No card e na barra: sempre (apagados quando o player não deixa mudar) ou só quando o player permite. No YouTube Music do navegador valem os botões da própria página", key: Key::CardShuffle, kind: Kind::Segmented(SHUFFLE_MODES) },
            Row { icon: "\u{E774}", title: "Extensão do navegador", desc: "", key: Key::BrowserExt, kind: Kind::Button("Instalar extensão") },
        ],
    },
    Page {
        name: "Geral",
        icon: "\u{E713}",
        subtitle: "Inicialização, tela cheia e informações",
        preview: Preview::None,
        header: None,
        items: &[
            Section("Sistema"),
            Row { icon: "\u{E7E8}", title: "Iniciar com o Windows", desc: "Abre automaticamente quando você entra no Windows", key: Key::Autostart, kind: Kind::Toggle },
            Row { icon: "\u{F2B7}", title: "Idioma", desc: "Automático segue o idioma do Windows (outros idiomas usam inglês)", key: Key::Language, kind: Kind::Choice(LANGS) },
            Row { icon: "\u{E740}", title: "Ocultar em tela cheia", desc: "Some durante jogos, vídeos e apresentações", key: Key::Fullscreen, kind: Kind::Toggle },
            Row { icon: "\u{E72C}", title: "Restaurar padrões", desc: "Volta todas as opções para os valores originais", key: Key::Reset, kind: Kind::Button("Restaurar") },
            Section("Atualizações"),
            Row { icon: "\u{E895}", title: "Procurar atualizações", desc: "Ao abrir e uma vez por dia, nas releases do Taskbeat no GitHub", key: Key::UpdateCheck, kind: Kind::Toggle },
            Row { icon: "\u{E896}", title: "Instalar sozinho, sem perguntar", desc: "Baixa, confere a assinatura, instala e reabre o Taskbeat; desligado, pergunta antes", key: Key::UpdateAuto, kind: Kind::Toggle },
            Row { icon: "\u{EBE8}", title: "Receber versões de teste", desc: "Também avisa dos betas, que podem ter defeitos", key: Key::UpdateBeta, kind: Kind::Toggle },
            Row { icon: "\u{E72C}", title: "Procurar agora", desc: "O resultado aparece num aviso perto do relógio", key: Key::UpdateNow, kind: Kind::Button("Procurar") },
            Row { icon: "\u{E72E}", title: "Privacidade", desc: "A consulta mostra o seu IP ao GitHub, como abrir qualquer site; nada mais é enviado", key: Key::About, kind: Kind::Info },
            Section("Sobre"),
            Row { icon: "\u{E8A5}", title: "Licenças", desc: "Licença MIT e bibliotecas de código aberto usadas", key: Key::Licenses, kind: Kind::Button("Ver licenças") },
            Row { icon: "\u{E946}", title: "Taskbeat", desc: "Feito em Rust · WASAPI loopback · System Media Transport Controls", key: Key::About, kind: Kind::Info },
        ],
    },
];

/// A extensão do navegador está conectada agora?
fn ext_connected() -> bool {
    !crate::webext::connected().is_empty()
}

/// Texto da linha "Extensão do navegador": conectada (e em quais) ou não.
fn browser_ext_status() -> String {
    let on = crate::webext::connected();
    if on.is_empty() {
        tr("Não conectada. Opcional: com a extensão do Taskbeat no Chrome, Brave ou Edge, o aleatório e o repetir do YouTube Music vêm da própria página").to_string()
    } else {
        format!("{} ({}): {}", tr("Conectada"), on.join(", "), tr("o aleatório e o repetir do YouTube Music vêm da própria página"))
    }
}

fn idx<T: PartialEq>(list: &[T], v: T) -> i32 {
    list.iter().position(|x| *x == v).unwrap_or(0) as i32
}

const SHAPE_V: &[Shape] = &[Shape::Rounded, Shape::Circle, Shape::Square];
const PLAY_V: &[Shape] = &[Shape::Circle, Shape::Rounded, Shape::Square];
const ANCHOR_V: &[Anchor] = &[Anchor::BeforeTray, Anchor::Left];
const STYLE_V: &[CardStyle] = &[CardStyle::Artwork, CardStyle::AlbumColor, CardStyle::Glass];
const LAYOUT_V: &[CardLayout] = &[CardLayout::Horizontal, CardLayout::Vertical];
const THEME_V: &[ThemeMode] = &[ThemeMode::Auto, ThemeMode::Dark, ThemeMode::Light];
const ACCENT_V: &[AccentSource] = &[AccentSource::Album, AccentSource::Windows, AccentSource::Custom];
const CARD_SHADOW_V: &[CardShadow] = &[CardShadow::Strong, CardShadow::Soft, CardShadow::None];
const BAR_SHAPE_V: &[BarShape] = &[BarShape::Rounded, BarShape::Square];
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
const CARD_ANIM_V: &[CardAnim] = &[CardAnim::Smooth, CardAnim::Slide, CardAnim::Rise, CardAnim::Drop, CardAnim::Pop, CardAnim::Zoom, CardAnim::Fade, CardAnim::None];
const ANIM_SPEED_V: &[AnimSpeed] = &[AnimSpeed::Fast, AnimSpeed::Normal, AnimSpeed::Slow];
const PALETTE_V: &[Palette] = &[Palette::Album, Palette::Accent, Palette::Rainbow, Palette::Neon, Palette::Mono];
const SIZE_V: &[Size3] = &[Size3::Small, Size3::Normal, Size3::Large];
const SECOND_V: &[SecondLine] = &[SecondLine::Artist, SecondLine::ArtistAlbum, SecondLine::Album, SecondLine::Time];
const CONTROL_V: &[ControlColor] = &[ControlColor::White, ControlColor::WhiteAccent, ControlColor::Album, ControlColor::Accent, ControlColor::Custom];
const SIDE_V: &[SideColor] = &[SideColor::SameAsMiddle, SideColor::White, SideColor::Album, SideColor::Accent];
const LAUNCHER_PLACE_V: &[LauncherPlace] = &[LauncherPlace::Card, LauncherPlace::Taskbar, LauncherPlace::Both];
const ICON_STYLE_V: &[IconStyle] = &[IconStyle::Filled, IconStyle::Fluent];
const TINT_V: &[WidgetTint] = &[WidgetTint::Neutral, WidgetTint::Album, WidgetTint::Accent, WidgetTint::Gradient, WidgetTint::Artwork];

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
        Key::IconAnim => c.icon_anim as i32,
        Key::IconStyle => idx(ICON_STYLE_V, c.icon_style),
        Key::CardViz => c.card_viz as i32,
        Key::CoverShape => idx(SHAPE_V, c.cover_shape),
        Key::CoverSize => idx(SIZE_V, c.cover_size),
        Key::TextSize => idx(SIZE_V, c.text_size),
        Key::WidgetPill => c.widget_pill as i32,
        Key::WidgetShape => idx(WIDGET_SHAPE_V, c.widget_shape),
        Key::WidgetTint => idx(TINT_V, c.widget_tint),
        Key::WidgetOpacity => c.widget_opacity,
        Key::WidgetIntensity => c.widget_intensity,
        Key::WidgetGlow => c.widget_glow as i32,
        Key::WidgetBorder => c.widget_border as i32,
        Key::WidgetDivider => c.widget_divider as i32,
        Key::TaskbarShuffle => c.taskbar_shuffle_repeat as i32,
        Key::TaskbarPlay => idx(TASKBAR_PLAY_V, c.taskbar_play),
        Key::WidgetControls => idx(CONTROL_V, c.widget_controls),
        Key::WidgetControlsRgb => rgb_of(&c.widget_controls_rgb) as i32,
        Key::WidgetSides => idx(SIDE_V, c.widget_sides),
        Key::WidgetSidesAlpha => c.widget_sides_alpha,
        Key::CardShuffle => idx(SHUFFLE_V, c.card_shuffle_repeat),
        Key::Marquee => c.marquee as i32,
        Key::TitleWidth => c.title_width,
        Key::Anchor => idx(ANCHOR_V, c.anchor),
        Key::Offset => c.offset,
        Key::WidgetDrag => c.widget_drag as i32,
        Key::CardStyle => idx(STYLE_V, c.card_style),
        Key::CardLayout => idx(LAYOUT_V, c.card_layout),
        Key::Theme => idx(THEME_V, c.theme),
        Key::Accent => idx(ACCENT_V, c.accent_source),
        Key::Corners => idx(CORNER_V, c.corners),
        Key::PlayShape => idx(PLAY_V, c.play_shape),
        Key::CardControls => idx(CONTROL_V, c.card_controls),
        Key::CardControlsRgb => rgb_of(&c.card_controls_rgb) as i32,
        Key::CardSides => idx(SIDE_V, c.card_sides),
        Key::CardSidesAlpha => c.card_sides_alpha,
        Key::CardSeek => c.card_seek as i32,
        Key::ShowVolume => c.show_volume as i32,
        Key::CardMixer => c.card_mixer as i32,
        Key::Autoclose => c.card_autoclose,
        Key::Launchers => c.card_launchers as i32,
        Key::LauncherPlace => idx(LAUNCHER_PLACE_V, c.launcher_place),
        Key::Popup => c.next_track_popup as i32,
        Key::PopupSecs => c.popup_secs,
        Key::ShowSource => match (c.show_source, c.source_name) {
            (false, _) => 2,
            (true, true) => 1,
            (true, false) => 0,
        },
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
        Key::CardAnim => idx(CARD_ANIM_V, c.card_anim),
        Key::AnimSpeed => idx(ANIM_SPEED_V, c.anim_speed),
        Key::ShowViz => c.show_visualizer as i32,
        Key::Style => idx(VIZ_V, c.style),
        Key::Palette => idx(PALETTE_V, c.palette),
        Key::Bars => BARS.iter().position(|b| *b >= c.bars).unwrap_or(BARS.len() - 1) as i32,
        Key::Width => c.width,
        Key::Sens => c.sensitivity_db.round() as i32,
        Key::OnlyMedia => c.only_when_media as i32,
        Key::PlayerOnly => c.viz_player_only as i32,
        Key::Peaks => c.show_peaks as i32,
        Key::Autostart => AUTOSTART.with(|a| a.get()) as i32,
        Key::Fullscreen => c.hide_on_fullscreen as i32,
        Key::Language => idx(LANG_V, c.language),
        Key::AccentRgb => rgb_of(&c.accent_rgb) as i32,
        Key::CardShadow => idx(CARD_SHADOW_V, c.card_shadow),
        Key::CardGrain => c.card_grain as i32,
        Key::GlassBlur => c.glass_blur,
        Key::GlassTint => c.glass_tint,
        Key::ArtworkDim => c.artwork_dim,
        Key::BarShape => idx(BAR_SHAPE_V, c.bar_shape),
        Key::Reset | Key::About | Key::OsdTest | Key::Licenses | Key::LauncherApps | Key::ResetPos | Key::Advanced | Key::BrowserExt | Key::UpdateNow => 0,
        Key::Lyrics(k) => lyrics_page::get(c, k),
        Key::UpdateCheck => c.update_check as i32,
        Key::UpdateAuto => c.update_auto as i32,
        Key::UpdateBeta => c.update_beta as i32,
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
        Key::IconAnim => c.icon_anim = v != 0,
        Key::IconStyle => c.icon_style = pick(ICON_STYLE_V, i),
        Key::CardViz => c.card_viz = v != 0,
        Key::CoverShape => c.cover_shape = pick(SHAPE_V, i),
        Key::CoverSize => c.cover_size = pick(SIZE_V, i),
        Key::TextSize => c.text_size = pick(SIZE_V, i),
        Key::WidgetPill => c.widget_pill = v != 0,
        Key::WidgetShape => c.widget_shape = pick(WIDGET_SHAPE_V, i),
        Key::WidgetTint => c.widget_tint = pick(TINT_V, i),
        Key::WidgetOpacity => c.widget_opacity = v,
        Key::WidgetIntensity => c.widget_intensity = v,
        Key::WidgetGlow => c.widget_glow = v != 0,
        Key::WidgetBorder => c.widget_border = v != 0,
        Key::WidgetDivider => c.widget_divider = v != 0,
        Key::TaskbarShuffle => c.taskbar_shuffle_repeat = v != 0,
        Key::TaskbarPlay => c.taskbar_play = pick(TASKBAR_PLAY_V, i),
        Key::WidgetControls => c.widget_controls = pick(CONTROL_V, i),
        Key::WidgetControlsRgb => c.widget_controls_rgb = hex_of(v as u32),
        Key::WidgetSides => c.widget_sides = pick(SIDE_V, i),
        Key::WidgetSidesAlpha => c.widget_sides_alpha = v,
        Key::CardShuffle => c.card_shuffle_repeat = pick(SHUFFLE_V, i),
        Key::Marquee => c.marquee = v != 0,
        Key::TitleWidth => c.title_width = v,
        Key::Anchor => c.anchor = pick(ANCHOR_V, i),
        Key::Offset => c.offset = v,
        Key::WidgetDrag => c.widget_drag = v != 0,
        Key::CardStyle => c.card_style = pick(STYLE_V, i),
        Key::CardLayout => c.card_layout = pick(LAYOUT_V, i),
        Key::Theme => c.theme = pick(THEME_V, i),
        Key::Accent => c.accent_source = pick(ACCENT_V, i),
        Key::Corners => c.corners = pick(CORNER_V, i),
        Key::PlayShape => c.play_shape = pick(PLAY_V, i),
        Key::CardControls => c.card_controls = pick(CONTROL_V, i),
        Key::CardControlsRgb => c.card_controls_rgb = hex_of(v as u32),
        Key::CardSides => c.card_sides = pick(SIDE_V, i),
        Key::CardSidesAlpha => c.card_sides_alpha = v,
        Key::CardSeek => c.card_seek = v != 0,
        Key::ShowVolume => c.show_volume = v != 0,
        Key::CardMixer => c.card_mixer = v != 0,
        Key::Autoclose => c.card_autoclose = v,
        Key::Launchers => c.card_launchers = v != 0,
        Key::LauncherPlace => c.launcher_place = pick(LAUNCHER_PLACE_V, i),
        Key::Popup => c.next_track_popup = v != 0,
        Key::PopupSecs => c.popup_secs = v,
        Key::ShowSource => (c.show_source, c.source_name) = (v != 2, v == 1),
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
        Key::CardAnim => c.card_anim = pick(CARD_ANIM_V, i),
        Key::AnimSpeed => c.anim_speed = pick(ANIM_SPEED_V, i),
        Key::ShowViz => c.show_visualizer = v != 0,
        Key::Style => c.style = pick(VIZ_V, i),
        Key::Palette => c.palette = pick(PALETTE_V, i),
        Key::Bars => c.bars = pick(BARS, i),
        Key::Width => c.width = v,
        Key::Sens => c.sensitivity_db = v as f32,
        Key::OnlyMedia => c.only_when_media = v != 0,
        Key::PlayerOnly => c.viz_player_only = v != 0,
        Key::Peaks => c.show_peaks = v != 0,
        Key::Fullscreen => c.hide_on_fullscreen = v != 0,
        Key::Language => {
            c.language = pick(LANG_V, i);
            crate::i18n::apply(c.language);
        }
        Key::AccentRgb => c.accent_rgb = hex_of(v as u32),
        Key::CardShadow => c.card_shadow = pick(CARD_SHADOW_V, i),
        Key::CardGrain => c.card_grain = v != 0,
        Key::GlassBlur => c.glass_blur = v,
        Key::GlassTint => c.glass_tint = v,
        Key::ArtworkDim => c.artwork_dim = v,
        Key::BarShape => c.bar_shape = pick(BAR_SHAPE_V, i),
        Key::Autostart | Key::Reset | Key::About | Key::OsdTest | Key::Licenses | Key::LauncherApps | Key::ResetPos | Key::Advanced | Key::BrowserExt | Key::UpdateNow => {}
        Key::Lyrics(k) => lyrics_page::set(c, k, v),
        Key::UpdateCheck => c.update_check = v != 0,
        Key::UpdateAuto => c.update_auto = v != 0,
        Key::UpdateBeta => c.update_beta = v != 0,
    }
}

/// Opções que só fazem sentido com outra ligada: devolve o motivo (texto da
/// dica) quando a opção está desativada. Desativada, ela aparece apagada e,
/// se for um interruptor, desligada.
fn requirement(c: &Config, key: Key) -> Option<&'static str> {
    use Key::*;
    const MEDIA: &str = "Ligue \"Mostrar música na barra\" para usar esta opção";
    const CUSTOM: &str = "Escolha \"Personalizada\" em \"Cor dos controles\" para usar esta opção";
    const LAUNCH: &str = "Ligue \"Atalhos dos apps de música\" para usar esta opção";
    const CONTROLS: &str = "Os botões estão desligados: ligue \"Botões de controle na barra\" para usar esta opção";
    const PILL: &str = "Ligue \"Fundo em pílula\" para usar esta opção";
    const RICH: &str = "Escolha \"Gradiente da capa\" ou \"Capa desfocada\" em \"Cor do fundo\" para usar esta opção";
    const CLASSIC: &str = "Com o gradiente ou a capa desfocada, quem manda é \"Intensidade da cor\"";
    const ARTIST: &str = "Ligue \"Mostrar artista\" para escolher a segunda linha";
    const POPUP: &str = "Ligue \"Mostrar A seguir\" para usar esta opção";
    const OSD: &str = "Ligue o aviso de Caps Lock ou de Num Lock para usar esta opção";
    const BLUR: &str = "O desfoque só vale para o material Acrílico";
    const VIZ: &str = "Ligue \"Mostrar barras dinâmicas\" para usar esta opção";
    const ACCENT: &str = "Escolha \"Personalizada\" em \"Cor de destaque\" para usar esta opção";
    const GLASS: &str = "Só vale para o fundo \"Acrílico\" do card";
    const ARTWORK: &str = "Só vale para o fundo \"Capa desfocada\" do card";
    const WAVE: &str = "As pontas não valem para o estilo \"Onda\"";
    const NO_ANIM: &str = "Escolha uma animação em \"Abrir e fechar\" para usar esta opção";
    const UPDATES: &str = "Ligue \"Procurar atualizações\" para usar esta opção";
    let osd_on = c.caps_lock_osd || c.num_lock_osd;
    match key {
        AlwaysShow | ShowArtist | SecondLine | Indicator | TaskbarProgress | TrackAnim | TaskbarControls | TaskbarShuffle | TaskbarPlay | WidgetControls | WidgetControlsRgb | WidgetSides | WidgetSidesAlpha | WidgetDivider | CoverShape | CoverSize | TextSize | Marquee | TitleWidth | WidgetPill | WidgetShape | WidgetTint | WidgetOpacity | WidgetIntensity | WidgetGlow | WidgetBorder
            if !c.show_media =>
        {
            Some(MEDIA)
        }
        TaskbarShuffle | TaskbarPlay | WidgetDivider | WidgetControls | WidgetControlsRgb | WidgetSides | WidgetSidesAlpha if !c.taskbar_controls => Some(CONTROLS),
        WidgetControlsRgb if c.widget_controls != ControlColor::Custom => Some(CUSTOM),
        CardControlsRgb if c.card_controls != ControlColor::Custom => Some(CUSTOM),
        WidgetShape | WidgetTint | WidgetOpacity | WidgetIntensity | WidgetGlow | WidgetBorder if !c.widget_pill => Some(PILL),
        WidgetIntensity | WidgetGlow if !c.widget_tint.is_rich() => Some(RICH),
        WidgetOpacity if c.widget_tint.is_rich() => Some(CLASSIC),
        SecondLine if !c.show_artist => Some(ARTIST),
        PopupSecs if !c.next_track_popup => Some(POPUP),
        LauncherPlace | LauncherApps if !c.card_launchers => Some(LAUNCH),
        OsdShowOff | OsdTest | OsdPos | OsdOffX | OsdOffY | OsdShape | OsdSize | OsdTheme | OsdDuration | OsdSwitch | OsdMaterial | OsdOpacity | OsdBlur | OsdIcon | OsdAnim if !osd_on => Some(OSD),
        OsdBlur if c.osd_material != crate::config::OsdMaterial::Acrylic => Some(BLUR),
        Style | Palette | Bars | Width | Sens | OnlyMedia | Peaks | BarShape if !c.show_visualizer => Some(VIZ),
        BarShape if c.style == crate::config::Style::Wave => Some(WAVE),
        AccentRgb if c.accent_source != AccentSource::Custom => Some(ACCENT),
        GlassBlur | GlassTint if c.card_style != crate::config::CardStyle::Glass => Some(GLASS),
        ArtworkDim if c.card_style != crate::config::CardStyle::Artwork => Some(ARTWORK),
        Lyrics(k) => lyrics_page::requirement(c, k),
        AnimSpeed if c.card_anim == crate::config::CardAnim::None => Some(NO_ANIM),
        UpdateAuto if !c.update_check => Some(UPDATES),
        _ => None,
    }
}

/// Uma volta da miniatura da animação do card (s).
fn demo_len(cfg: &Config) -> f32 {
    let d = |ph| anim::duration(cfg.card_anim, ph, cfg.anim_speed, anim::Which::Card);
    DEMO_GAP + d(Phase::Open) + DEMO_HOLD + d(Phase::Close) + DEMO_GAP
}

/// Pose da miniatura `t` segundos depois de começar a volta.
fn demo_pose(cfg: &Config, t: f32, ctx: anim::Ctx) -> Pose {
    let (style, which) = (cfg.card_anim, anim::Which::Card);
    let d_open = anim::duration(style, Phase::Open, cfg.anim_speed, which);
    let d_close = anim::duration(style, Phase::Close, cfg.anim_speed, which);
    let closed = anim::pose(style, Phase::Close, 1.0, which, ctx);
    let opened = DEMO_GAP + d_open;
    let closing = opened + DEMO_HOLD;
    if t < DEMO_GAP {
        closed
    } else if t < opened {
        anim::pose(style, Phase::Open, (t - DEMO_GAP) / d_open, which, ctx)
    } else if t < closing {
        Pose::REST
    } else if t < closing + d_close {
        anim::pose(style, Phase::Close, (t - closing) / d_close, which, ctx)
    } else {
        closed
    }
}

fn value_label(kind: Kind, key: Key, v: i32) -> String {
    let Kind::Slider { unit, .. } = kind else { return String::new() };
    match key {
        Key::Sens | Key::Offset | Key::OsdOffX | Key::OsdOffY => format!("{v:+} {unit}"),
        Key::Lyrics(k) if lyrics_page::signed(k) => format!("{v:+} {unit}"),
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
    /// App `k` da lista de atalhos da linha `ri` (clique liga/desliga; arrastar reordena).
    App(usize, usize),
    /// Parte `k` do seletor de cor da linha `ri` (ver `PICK_*`).
    Pick(usize, usize),
    /// Botão `k` da prévia (`Layout::chips`).
    Chip(usize),
}

/// Botões no canto da prévia: como ela mostra o widget e o card, sem mexer
/// na configuração.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Chip {
    /// Tocando (0), pausado (1) ou sem música (2).
    Play(u8),
    /// Windows claro (`true`) ou escuro: barra, papel de parede e o tema "Automático".
    Light(bool),
}

fn chip_label(c: Chip) -> &'static str {
    match c {
        Chip::Play(0) => "Tocando",
        Chip::Play(1) => "Pausado",
        Chip::Play(_) => "Sem música",
        Chip::Light(true) => "Claro",
        Chip::Light(false) => "Escuro",
    }
}

/// Quanto dura a demonstração que a prévia faz quando a opção muda (s).
fn demo_secs(key: Key) -> f32 {
    match key {
        Key::AlwaysShow => 2.6,
        Key::TrackAnim => 1.3,
        Key::IconAnim => 1.0,
        Key::CardShuffle => 3.5,
        _ => 0.0,
    }
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
    /// Grupos "Avançado" abertos (bit por página).
    adv: u32,
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
    /// Lista de apps: ordem, escolhas, mouse e arrasto (resumidos).
    extra: u64,
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
    /// Papel de parede escuro (segue o Windows ou o botão "Escuro/Claro" da prévia).
    wall_dark: bool,
}

/// Quadro da prévia: só é desenhado de novo quando isto muda.
#[derive(Clone, Copy, PartialEq)]
struct HeroSig {
    key: HeroKey,
    cfg: u64,
    media: u64,
    frame: u64,
    /// Tocando / pausado / sem música e a demonstração em andamento.
    state: (u8, Option<Key>),
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
    adv: u32,
}

/// Como desenhar um app da lista de atalhos.
#[derive(Clone, Copy)]
struct AppRow {
    /// À mostra nos atalhos.
    on: bool,
    /// Mouse em cima.
    hot: bool,
    /// Sendo arrastado (por cima dos outros, com sombra).
    lifted: bool,
}

/// Arrastando um app da lista de atalhos (y da janela).
struct AppDrag {
    ri: usize,
    id: String,
    y0: f32,
    y: f32,
    /// Já andou o suficiente para ser arrasto (senão é clique).
    active: bool,
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
    app_drag: Option<AppDrag>,
    /// Arrastando no quadro de matiz/saturação ou no brilho: (linha, parte).
    color_drag: Option<(usize, usize)>,
    /// Editando o código hex de um seletor: (linha, texto digitado).
    hex_edit: Option<(usize, String)>,
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
    /// Miniatura da animação do card tocando: desde quando (uma volta; com o
    /// mouse em cima, em laço) e a última escolha vista (trocar toca de novo).
    anim_demo: Option<Instant>,
    anim_seen: Option<(CardAnim, AnimSpeed)>,
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
    /// Grupos "Avançado" abertos (bit por página; valem enquanto a janela estiver aberta).
    adv_open: u32,
    /// Demonstração na prévia da opção que acabou de mudar (ver `demo_secs`).
    demo: Option<(Key, Instant)>,
    /// Botões da prévia: tocando/pausado/sem música e Windows claro/escuro
    /// (`None`: segue o player e o Windows).
    pv_play: Option<u8>,
    pv_light: Option<bool>,
    /// Ícones animados da prévia (demonstração de "Animar ícones").
    icons: ui::IconAnims,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    /// "Iniciar com o Windows" (lido do registro ao abrir, não a cada quadro).
    static AUTOSTART: Cell<bool> = const { Cell::new(false) };
    /// Aberta sozinha (`run_alone`): fechar a janela encerra o programa.
    static ALONE: Cell<bool> = const { Cell::new(false) };
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
        // Só as configurações (capturas): sem botão na barra de tarefas e sem pegar o foco.
        let ex = if ALONE.with(|a| a.get()) { WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE } else { WINDOW_EX_STYLE(0) };
        let Ok(hwnd) = CreateWindowExW(ex, class, &HSTRING::from(tr("Taskbeat — Configurações")), STYLE, sx, sy, ww, wh, None, None, Some(hinst), None) else {
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
        // Capturas de tela: abre direto numa aba (`TV_DEMO_SETTINGS_PAGE`, como no
        // Mac) e com o "Avançado" aberto (`TV_DEMO_SETTINGS_ADV`).
        let page = std::env::var("TV_DEMO_SETTINGS_PAGE").ok().and_then(|v| v.parse::<usize>().ok()).filter(|p| *p < PAGES.len()).unwrap_or(0);
        let adv_open = if std::env::var_os("TV_DEMO_SETTINGS_ADV").is_some() { u32::MAX } else { 0 };
        UI.with(|u| {
            *u.borrow_mut() = Some(Ui {
                shared,
                hwnd,
                scale,
                dark,
                mica,
                accent: system::app_accent(dark),
                page,
                scroll: 0.0,
                scroll_target: 0.0,
                hover: None,
                pressed: None,
                drag: None,
                app_drag: None,
                color_drag: None,
                hex_edit: None,
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
                anim_demo: None,
                anim_seen: None,
                backdrop: None,
                nav_pos: page as f32,
                page_anim: None,
                page_anim_pending: false,
                tip: None,
                pacer,
                slow_timer: false,
                last_tick: Instant::now(),
                warmed: 1,
                warm_pos: None,
                adv_open,
                demo: None,
                pv_play: None,
                pv_light: None,
                icons: ui::IconAnims::default(),
            });
        });
        // Abre maximizada (ao restaurar, volta ao tamanho padrão, centralizada).
        // A 1ª chamada de ShowWindow do processo pode herdar o modo de exibição do
        // atalho/linha de comando (ex.: oculto); a 2ª garante que a janela apareça.
        if ALONE.with(|a| a.get()) {
            // Só as configurações (capturas e testes): atrás das outras janelas e
            // sem foco, para não cobrir a tela de quem está usando o PC.
            let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), sx, sy, ww, wh, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOWMAXIMIZED);
            let _ = ShowWindow(hwnd, SW_SHOWMAXIMIZED);
            let _ = SetForegroundWindow(hwnd);
        }
        with_ui(|u| u.schedule());
        SetTimer(Some(hwnd), WARM_TIMER, 150, None);
    }
}

/// Só a tela de configurações, até ela fechar (`TASKBEAT_SETTINGS_ONLY`).
pub fn run_alone(shared: Arc<Shared>) {
    ALONE.with(|a| a.set(true));
    open(shared, HICON::default());
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
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
    /// Quadro do grupo da linha (as linhas de um grupo ficam juntas num quadro só).
    group: RectF,
    first: bool,
    last: bool,
    /// Dentro de um "Avançado" fechado: não aparece nem recebe clique.
    hidden: bool,
}

impl RowL {
    fn shifted(&self, dx: f32, dy: f32) -> RowL {
        RowL { card: self.card.offset(dx, dy), ctl: self.ctl.offset(dx, dy), segs: self.segs.iter().map(|r| r.offset(dx, dy)).collect(), group: self.group.offset(dx, dy), ..self.clone() }
    }

    /// Área que a linha redesenha sozinha: a dela, com a borda do grupo nas
    /// pontas (sem apagar o separador da linha de baixo).
    fn clip(&self, s: f32) -> RectF {
        let top = if self.first { 2.0 * s } else { 0.0 };
        let bottom = if self.last { 2.0 * s } else { 0.0 };
        RectF::new(self.card.x - 2.0 * s, self.card.y - top, self.card.w + 4.0 * s, self.card.h + top + bottom)
    }
}

/// Posições na janela com a lista sem rolagem (a rolagem é aplicada ao usar).
struct Layout {
    nav: Vec<RectF>,
    /// Títulos dos grupos de abas no menu (texto e topo).
    nav_heads: Vec<(&'static str, f32)>,
    /// Botões no canto da prévia.
    chips: Vec<(Chip, RectF)>,
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

    /// Card da prévia, com os atalhos dos apps de música desta máquina.
    fn preview_card(&self, s: f32, cfg: &Config) -> ui::CLayout {
        let n = if cfg.card_launchers && cfg.launcher_place.card() { self.shown_launchers(cfg).len() } else { 0 };
        ui::CLayout::build(s, cfg, true).with_launchers(n, s).with_mixer(cfg.card_mixer.then_some(0), 0.0, 0.0, crate::shared::MIXER_ROWS, s)
    }

    /// Atalhos à mostra, na ordem escolhida.
    fn shown_launchers(&self, cfg: &Config) -> Vec<crate::shared::Launcher> {
        let found = self.shared.launchers.lock().unwrap().clone();
        crate::launchers::shown(&found, cfg)
    }

    /// Quantos atalhos vão dentro do widget da barra.
    fn bar_launchers(&self, cfg: &Config) -> usize {
        if cfg.card_launchers && cfg.launcher_place.taskbar() { self.shown_launchers(cfg).len() } else { 0 }
    }

    fn hero_height_at(&self, page: &Page, cfg: &Config, s: f32) -> f32 {
        match page.preview {
            Preview::Widget | Preview::Visualizer => 168.0 * s,
            Preview::Card => self.preview_card(s, cfg).h + 18.0 * s,
            Preview::UpNext => ui::upnext_size(s).1 + 24.0 * s,
            // Aviso com a "telinha" de onde ele aparece ao lado.
            Preview::LockKeys => (ui::osd_size(s, cfg).1 + 40.0 * s).max(166.0 * s),
            // Card aberto em cima da barra com o widget, como na tela.
            Preview::Overview => self.preview_card(s, cfg).h - ui::SHADOW_MARGIN * s + 72.0 * s,
            Preview::Lyrics => lyrics_page::preview_height(cfg, s),
            Preview::None => 0.0,
        }
    }

    /// Escala da prévia: a da janela ou menor, quando ela tomaria mais da
    /// metade da altura (a prévia fica fixa no topo; as opções rolam embaixo).
    fn preview_scale(&self, page: &Page, cfg: &Config) -> f32 {
        let s = self.scale;
        let natural = self.hero_height_at(page, cfg, s);
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
        let key = LayoutKey { page: self.page, w: w as i32, h: h as i32, scale: self.scale.to_bits(), lang: lang_code(), cfg: self.cfg_version(), adv: self.adv_open };
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
        // Menu: as abas de "Para todos" (e as que vierem) ganham um título acima.
        let mut nav = Vec::new();
        let mut nav_heads = Vec::new();
        let mut ny = 92.0 * s;
        for p in PAGES {
            if let Some(h) = p.header {
                ny += 14.0 * s;
                nav_heads.push((h, ny));
                ny += 24.0 * s;
            }
            nav.push(RectF::new(10.0 * s, ny, NAV_W * s - 20.0 * s, 38.0 * s));
            ny += 42.0 * s;
        }
        let left = NAV_W * s + 12.0 * s;
        let right = (w - 32.0 * s).max(left + 200.0 * s);
        let (x0, x1) = (left, right);
        // Título e prévia ficam fixos; só a lista de opções rola, abaixo da prévia.
        let mut y = 92.0 * s;
        let mut hero = None;
        let hh = self.hero_height_at(page, &cfg, self.preview_scale(page, &cfg));
        if hh > 0.0 {
            hero = Some(RectF::new(x0, y, x1 - x0, hh));
            y += hh + 12.0 * s;
        }
        let chips = hero.map(|h| self.chip_rects(h, page.preview)).unwrap_or_default();
        let list_top = y.round();
        let open = self.adv_open & (1 << self.page) != 0;
        let mut sections = Vec::new();
        let mut rows: Vec<RowL> = Vec::new();
        // Linhas do grupo em montagem (índices em `rows`) e se estamos num "Avançado".
        let mut group: Vec<usize> = Vec::new();
        let mut in_adv = false;
        fn close(rows: &mut [RowL], group: &mut Vec<usize>) {
            let (Some(&a), Some(&b)) = (group.first(), group.last()) else { return };
            let g = RectF::new(rows[a].card.x, rows[a].card.y, rows[a].card.w, rows[b].card.bottom() - rows[a].card.y);
            for &k in group.iter() {
                rows[k].group = g;
                rows[k].first = k == a;
                rows[k].last = k == b;
            }
            group.clear();
        }
        for (i, item) in page.items.iter().enumerate() {
            match item {
                Section(_) => {
                    close(&mut rows, &mut group);
                    in_adv = false;
                    // Respiro entre os grupos (menos no primeiro).
                    y += if sections.is_empty() && rows.is_empty() { 4.0 * s } else { 22.0 * s };
                    sections.push((i, y));
                    y += 30.0 * s;
                }
                Row { kind, key, .. } => {
                    if matches!(kind, Kind::Expander) {
                        // "Avançado": um grupo próprio, sem título, com as linhas raras embaixo.
                        close(&mut rows, &mut group);
                        in_adv = true;
                        y += 22.0 * s;
                    }
                    let hidden = in_adv && !open && !matches!(kind, Kind::Expander);
                    if hidden {
                        let card = RectF::new(x0, y.round(), x1 - x0, 0.0);
                        rows.push(RowL { item: i, card, ctl: RectF::default(), segs: Vec::new(), group: card, first: false, last: false, hidden: true });
                        continue;
                    }
                    let apps = if matches!(kind, Kind::AppList) { self.shared.launchers.lock().unwrap().len().max(1) } else { 0 };
                    // O seletor de cor só abre quando a opção vale ("Personalizada").
                    let picker = matches!(kind, Kind::Color) && requirement(&cfg, *key).is_none();
                    let card_h = if apps > 0 {
                        ROW_H * s + apps as f32 * 44.0 * s + 10.0 * s
                    } else if picker {
                        PICKER_H * s
                    } else {
                        ROW_H * s
                    };
                    let card = RectF::new(x0, y.round(), x1 - x0, card_h.round());
                    let right = card.right() - 18.0 * s;
                    // Controles centrados na faixa de cima (a de uma linha comum).
                    let cy = card.y + ROW_H * s / 2.0;
                    let mut segs = Vec::new();
                    let ctl = match kind {
                        Kind::Toggle => RectF::new(right - 40.0 * s, cy - 10.0 * s, 40.0 * s, 20.0 * s),
                        Kind::Choice(_) => RectF::new(right - 250.0 * s, cy - 16.0 * s, 250.0 * s, 32.0 * s),
                        Kind::Anim(_) => {
                            // Miniatura à esquerda da lista.
                            let ctl = RectF::new(right - 250.0 * s, cy - 16.0 * s, 250.0 * s, 32.0 * s);
                            segs.push(RectF::new((ctl.x - 14.0 * s - ANIM_TILE.0 * s).round(), (cy - ANIM_TILE.1 * s / 2.0).round(), (ANIM_TILE.0 * s).round(), (ANIM_TILE.1 * s).round()));
                            ctl
                        }
                        Kind::Slider { .. } => RectF::new(right - 230.0 * s, cy - 16.0 * s, 230.0 * s, 32.0 * s),
                        Kind::Button(_) => RectF::new(right - 150.0 * s, cy - 16.0 * s, 150.0 * s, 32.0 * s),
                        Kind::Expander => RectF::new(right - 32.0 * s, cy - 16.0 * s, 32.0 * s, 32.0 * s),
                        Kind::Segmented(opts) => {
                            let widths: Vec<f32> = opts
                                .iter()
                                .map(|o| (self.text.mask(tr(o), Font::BodyStrong, (300.0 * s) as i32, (20.0 * s) as i32, Align::Left).used_width() + 28.0 * s).max(78.0 * s).round())
                                .collect();
                            let total: f32 = widths.iter().sum::<f32>() + 8.0 * s;
                            let r = RectF::new((right - total).round(), (cy - 17.0 * s).round(), total.round(), (34.0 * s).round());
                            let mut x = r.x + 4.0 * s;
                            for wd in widths {
                                segs.push(RectF::new(x, r.y + 4.0 * s, wd, r.h - 8.0 * s));
                                x += wd;
                            }
                            r
                        }
                        Kind::Info => RectF::default(),
                        Kind::AppList => {
                            for k in 0..apps {
                                segs.push(RectF::new(card.x + 48.0 * s, card.y + 60.0 * s + k as f32 * 44.0 * s, card.w - 66.0 * s, 40.0 * s));
                            }
                            RectF::default()
                        }
                        Kind::Color => {
                            if picker {
                                segs = picker_parts(card, s);
                            }
                            // Amostra da cor atual (à direita do cabeçalho).
                            RectF::new(right - 32.0 * s, cy - 16.0 * s, 32.0 * s, 32.0 * s)
                        }
                    };
                    group.push(rows.len());
                    rows.push(RowL { item: i, card, ctl, segs, group: card, first: true, last: true, hidden: false });
                    y += card.h;
                }
            }
        }
        close(&mut rows, &mut group);
        let content_h = y + 32.0 * s;
        Layout { nav, nav_heads, chips, x0, x1, title_y: 26.0 * s, hero, list_top, sections, rows, content_h }
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
        if let Some(k) = l.chips.iter().position(|(_, r)| r.contains(x, y)) {
            return Some(Hit::Chip(k));
        }
        // A lista rola por baixo da prévia fixa: o que está escondido não recebe clique.
        if x < NAV_W * self.scale || y < l.list_top {
            return None;
        }
        let y = y + self.scroll_px();
        let cfg = self.cfg();
        for (ri, r) in l.rows.iter().enumerate() {
            let Row { kind, key, .. } = &PAGES[self.page].items[r.item] else { continue };
            if r.hidden {
                continue;
            }
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
                Kind::Toggle | Kind::Expander => {
                    if r.card.contains(x, y) {
                        return Some(Hit::Row(ri));
                    }
                }
                Kind::Choice(_) | Kind::Slider { .. } | Kind::Button(_) => {
                    if r.ctl.contains(x, y) && !(*key == Key::BrowserExt && ext_connected()) {
                        return Some(Hit::Row(ri));
                    }
                }
                Kind::Anim(_) => {
                    if r.ctl.contains(x, y) || r.segs.iter().any(|t| t.contains(x, y)) {
                        return Some(Hit::Row(ri));
                    }
                }
                Kind::Color => {
                    if let Some(k) = r.segs.iter().position(|sr| sr.contains(x, y)) {
                        return Some(Hit::Pick(ri, k));
                    }
                }
                Kind::AppList => {
                    if let Some(k) = r.segs.iter().position(|sr| sr.contains(x, y))
                        && k < self.shared.launchers.lock().unwrap().len()
                    {
                        return Some(Hit::App(ri, k));
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
        let v = if matches!(kind, Kind::Expander) { self.adv_is_open() as i32 } else { get(cfg, key) };
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
            extra: match kind {
                Kind::AppList => self.app_list_sig(ri, cfg),
                Kind::Color => self.picker_sig(ri, cfg, key),
                // Redesenha quando a extensão conecta ou desconecta.
                Kind::Button(_) if key == Key::BrowserExt => crate::webext::connected().join(",").bytes().fold(1u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64)),
                // Quadro da miniatura (0 = parada).
                Kind::Anim(_) => self.anim_demo.map_or(0, |t| (t.elapsed().as_secs_f64() * 60.0) as u64 + 1),
                _ => 0,
            },
        }
    }

    /// O "Avançado" da página atual está aberto?
    fn adv_is_open(&self) -> bool {
        self.adv_open & (1 << self.page) != 0
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

    /// Demonstração em andamento na prévia (a opção que acabou de mudar).
    fn demo_key(&self) -> Option<Key> {
        self.demo.filter(|(k, t)| t.elapsed().as_secs_f32() < demo_secs(*k)).map(|(k, _)| k)
    }

    /// Começa a demonstração de uma opção cujo efeito só aparece num momento
    /// (troca de música, ícones animados, sem música, player sem aleatório).
    fn start_demo(&mut self, key: Key) {
        let cfg = self.cfg();
        let on = match key {
            Key::AlwaysShow | Key::CardShuffle => true,
            Key::TrackAnim => cfg.track_anim,
            Key::IconAnim => cfg.icon_anim,
            _ => false,
        };
        if !on {
            return;
        }
        self.demo = Some((key, Instant::now()));
        if key == Key::IconAnim {
            for h in [ui::Hit::Shuffle, ui::Hit::Prev, ui::Hit::Play, ui::Hit::Next, ui::Hit::Repeat, ui::Hit::VolBtn] {
                self.icons.kick(h);
            }
        }
    }

    /// O que a prévia mostra: tocando (0), pausado (1) ou sem música (2). Segue
    /// o player até um clique nos botões da prévia; "Sempre visível" acabado de
    /// mudar mostra um instante sem música. Card e "A seguir" não têm "sem música".
    fn play_state_for(&self, preview: Preview) -> u8 {
        let st = if self.demo_key() == Some(Key::AlwaysShow) {
            2
        } else {
            match self.pv_play {
                Some(p) => p,
                None if self.media_paused() => 1,
                None => 0,
            }
        };
        if st == 2 && !matches!(preview, Preview::Widget | Preview::Visualizer) { 0 } else { st }
    }

    /// Windows claro na prévia (botão "Claro/Escuro" dela ou o do sistema).
    fn preview_light(&mut self, cfg: &Config) -> bool {
        match self.pv_light {
            Some(l) => l,
            None => self.colors(cfg.palette).light_taskbar,
        }
    }

    /// Papel de parede escuro na prévia.
    fn wall_dark(&self) -> bool {
        self.pv_light.map(|l| !l).unwrap_or(self.dark)
    }

    /// Quadros por segundo que a prévia precisa (0 = parada).
    fn hero_fps(&self, cfg: &Config) -> f32 {
        let preview = PAGES[self.page].preview;
        if self.demo_key().is_some() || self.icons.running() {
            return 30.0;
        }
        let st = self.play_state_for(preview);
        let widget = if st != 0 {
            0.0
        } else if cfg.marquee || cfg.playing_indicator || cfg.show_visualizer {
            30.0
        } else if cfg.taskbar_progress || cfg.second_line == SecondLine::Time {
            2.0
        } else {
            0.0
        };
        match preview {
            Preview::UpNext if cfg.next_track_popup => 30.0,
            Preview::LockKeys if cfg.caps_lock_osd || cfg.num_lock_osd => 30.0,
            Preview::UpNext | Preview::LockKeys => 0.0,
            Preview::Widget | Preview::Visualizer => widget,
            Preview::Card if st == 0 => 2.0,
            Preview::Overview if st == 0 => widget.max(2.0),
            Preview::Lyrics => 30.0,
            Preview::Card | Preview::Overview | Preview::None => 0.0,
        }
    }

    /// Liga o ritmo do monitor só quando algo anima; prévias lentas usam um
    /// timer de 250 ms; parado, nada roda (janela parada = 0% de CPU).
    fn schedule(&mut self) {
        let minimized = unsafe { IsIconic(self.hwnd).as_bool() };
        let cfg = self.cfg();
        let fps = if minimized { 0.0 } else { self.hero_fps(&cfg) };
        let tip_pending = self.tip.is_some() && self.tip_alpha() < 1.0;
        let anim_tile = self.anim_demo.is_some() || self.anim_hovered() || self.anim_seen.is_some_and(|v| v != (cfg.card_anim, cfg.anim_speed));
        let busy = !minimized
                || anim_tile
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
                    cv.set_clip(hd);
                    self.draw_chips(cv, &l, alpha, dy as f32);
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

    /// Retângulos (janela) dos botões no canto da prévia: tocando / pausado /
    /// sem música onde o widget ou o card aparecem, e Windows escuro / claro.
    fn chip_rects(&mut self, hero: RectF, preview: Preview) -> Vec<(Chip, RectF)> {
        let s = self.scale;
        let plays: &[u8] = match preview {
            Preview::Widget | Preview::Visualizer => &[0, 1, 2],
            Preview::Card | Preview::Overview => &[0, 1],
            _ => &[],
        };
        let (y, h) = ((hero.y + 8.0 * s).round(), (26.0 * s).round());
        let mut x = hero.right() - 10.0 * s;
        let mut out = Vec::new();
        let mut push = |chip: Chip, x: &mut f32, ui: &mut Ui| {
            let w = (ui.text.mask(tr(chip_label(chip)), Font::Caption, (300.0 * s) as i32, (20.0 * s) as i32, Align::Left).used_width() + 24.0 * s).round();
            *x -= w;
            out.push((chip, RectF::new(x.round(), y, w, h)));
        };
        for v in [true, false] {
            push(Chip::Light(v), &mut x, self);
        }
        x -= 10.0 * s;
        for &p in plays.iter().rev() {
            push(Chip::Play(p), &mut x, self);
        }
        out.reverse();
        out
    }

    /// Botões no canto da prévia (por cima do quadro dela, fora do cache).
    fn draw_chips(&mut self, cv: &mut Canvas, l: &Layout, alpha: f32, dy: f32) {
        if l.chips.is_empty() {
            return;
        }
        let s = self.scale;
        let cfg = self.cfg();
        let preview = PAGES[self.page].preview;
        let (play, light, dark) = (self.play_state_for(preview), self.preview_light(&cfg), self.wall_dark());
        let ink: Rgb = if dark { [1.0; 3] } else { [0.05, 0.06, 0.12] };
        for light_group in [false, true] {
            let g = l.chips.iter().filter(|(c, _)| matches!(c, Chip::Light(_)) == light_group).fold(RectF::default(), |g, (_, r)| union(g, *r));
            if g.is_empty() {
                continue;
            }
            let g = g.offset(0.0, dy);
            cv.fill_round(g, g.h / 2.0, if dark { [0.0; 3] } else { [1.0; 3] }, (if dark { 0.34 } else { 0.55 }) * alpha);
            cv.stroke_round(g, g.h / 2.0, s, ink, 0.12 * alpha);
        }
        for (k, (chip, r)) in l.chips.iter().enumerate() {
            let r = r.offset(0.0, dy);
            let sel = match chip {
                Chip::Play(p) => *p == play,
                Chip::Light(v) => *v == light,
            };
            let pill = r.inset(2.0 * s);
            if sel {
                cv.fill_round(pill, pill.h / 2.0, if dark { [1.0; 3] } else { [0.08, 0.09, 0.14] }, 0.92 * alpha);
            } else if self.hover == Some(Hit::Chip(k)) {
                cv.fill_round(pill, pill.h / 2.0, ink, 0.12 * alpha);
            }
            let tc: Rgb = if sel { if dark { [0.08, 0.09, 0.14] } else { [1.0; 3] } } else { ink };
            ui::text_center(cv, &mut self.text, tr(chip_label(*chip)), Font::Caption, r.offset(0.0, -0.5 * s), tc, (if sel { 1.0 } else { 0.85 }) * alpha);
        }
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
        for (name, y) in &l.nav_heads {
            self.label(cv, tr(name), Font::Caption, 24.0 * s, *y, NAV_W * s - 40.0 * s, 18.0 * s, fg, 0.55);
        }
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
            adv: self.adv_open,
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
                    if self.rows_drawn.get(ri) == Some(st) || l.rows[ri].hidden {
                        continue;
                    }
                    n += 1;
                    let row = l.rows[ri].shifted(-ox, 0.0);
                    cv.set_clip(row.clip(self.scale));
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
            if row.hidden {
                continue;
            }
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
        let card_hover = if matches!(kind, Kind::Toggle | Kind::Expander) && st.enabled { a.hover } else { 0.0 };
        self.draw_group_part(cv, r, card_hover);

        let rh = ROW_H * s;
        ui::glyph(cv, &mut self.text, RectF::new(r.card.x + 18.0 * s, r.card.y, 26.0 * s, rh), icon, Font::IconMd, fg, 0.92 * en);
        let tx = r.card.x + 60.0 * s;
        let tw = match kind {
            Kind::Info | Kind::AppList => r.card.right() - tx - 20.0 * s,
            Kind::Anim(_) => r.segs.first().map_or(r.ctl.x, |t| t.x) - tx - 16.0 * s,
            _ => r.ctl.x - tx - 24.0 * s,
        };
        self.label(cv, tr(title), Font::Body, tx, r.card.y + 11.0 * s, tw, 21.0 * s, fg, en);
        let summary;
        let desc = if matches!(kind, Kind::Expander) {
            summary = adv_summary(self.page, r.item);
            summary.as_str()
        } else if *key == Key::BrowserExt {
            summary = browser_ext_status();
            summary.as_str()
        } else {
            tr(desc)
        };
        self.label(cv, desc, Font::Caption, tx, r.card.y + 33.0 * s, tw, 19.0 * s, fg, 0.62 * en);

        let v = st.v;
        match kind {
            Kind::Toggle => self.draw_toggle(cv, r.ctl, v != 0, a.toggle, card_hover, en),
            Kind::Segmented(opts) => self.draw_segmented(cv, r, opts, v as usize, a.seg, st.seg_hover, en),
            Kind::Choice(names) | Kind::Anim(names) => {
                self.draw_box(cv, r.ctl, a.hover, en);
                let name = tr(names.get(v.max(0) as usize).copied().unwrap_or(""));
                self.label(cv, name, Font::Body, r.ctl.x + 12.0 * s, r.ctl.y, r.ctl.w - 44.0 * s, r.ctl.h, fg, en);
                ui::glyph(cv, &mut self.text, RectF::new(r.ctl.right() - 32.0 * s, r.ctl.y, 24.0 * s, r.ctl.h), "\u{E70D}", Font::IconSmall, fg, 0.8 * en);
                if let (Kind::Anim(_), Some(&tile)) = (kind, r.segs.first()) {
                    self.draw_anim_tile(cv, tile, en);
                }
            }
            Kind::Slider { min, max, .. } => {
                self.draw_slider(cv, r.ctl, (v - min) as f32 / (max - min) as f32, a.hover, en);
                self.label_right(cv, &value_label(*kind, *key, v), Font::Caption, r.ctl.x - 12.0 * s, r.ctl.cy(), 20.0 * s, fg, 0.75 * en);
            }
            // Com a extensão conectada, não há o que instalar: só o texto.
            Kind::Button(_) if *key == Key::BrowserExt && ext_connected() => {}
            Kind::Button(t) => {
                self.draw_box(cv, r.ctl, a.hover, en);
                ui::text_center(cv, &mut self.text, tr(t), Font::Body, r.ctl, fg, en);
            }
            Kind::Info => {}
            Kind::AppList => self.draw_app_list(cv, ri, r, en),
            Kind::Color => self.draw_picker(cv, ri, r, *key, en),
            // Setinha: para baixo fechado, para cima aberto (como o Expander do WinUI).
            Kind::Expander => ui::glyph(cv, &mut self.text, r.ctl, if v != 0 { "\u{E70E}" } else { "\u{E70D}" }, Font::IconSmall, fg, 0.85 * en),
        }
    }

    /// Parte do quadro do grupo atrás da linha `r`: o quadro inteiro recortado
    /// à linha (cantos arredondados só nas pontas), o realce de mouse em cima e
    /// o separador da linha de cima.
    fn draw_group_part(&mut self, cv: &mut Canvas, r: &RowL, hover: f32) {
        let s = self.scale;
        let radius = 8.0 * s;
        let saved = cv.clip_rect();
        cv.set_clip(saved.intersect(r.clip(s)));
        cv.fill_round(r.group, radius, [1.0; 3], if self.dark { 0.05 } else { 0.70 });
        if hover > 0.0 {
            cv.set_clip(saved.intersect(r.card));
            cv.fill_round(r.group, radius, [1.0; 3], (if self.dark { 0.03 } else { 0.25 }) * hover);
            cv.set_clip(saved.intersect(r.clip(s)));
        }
        cv.stroke_round(r.group, radius, s, [0.0; 3], if self.dark { 0.20 } else { 0.06 });
        if !r.first {
            let fg = self.fg();
            let y = r.card.y.round();
            cv.fill_rrect(r.card.x + 16.0 * s, y, r.card.right() - 16.0 * s, y + s.round().max(1.0), 0.0, 0.07, fg);
        }
        cv.set_clip(saved);
    }

    /// Resumo do que o seletor de cor mostra (para só redesenhar quando muda).
    fn picker_sig(&self, ri: usize, cfg: &Config, key: Key) -> u64 {
        let hover = match self.hover {
            Some(Hit::Pick(r, k)) if r == ri => k as i32,
            _ => -1,
        };
        let edit = self.hex_edit.as_ref().filter(|(r, _)| *r == ri).map(|(_, t)| t.clone());
        ui::layer_key((get(cfg, key), hover, self.color_drag.filter(|(r, _)| *r == ri), edit))
    }

    /// Seletor de cor: amostra e código no cabeçalho; aberto, as cores prontas,
    /// o quadro de matiz (horizontal) e saturação (vertical), o brilho e o hex.
    fn draw_picker(&mut self, cv: &mut Canvas, ri: usize, r: &RowL, key: Key, en: f32) {
        let s = self.scale;
        let fg = self.fg();
        let cfg = self.cfg();
        let rgb = get(&cfg, key) as u32;
        let cur = rgb_f(rgb);
        // Cabeçalho: amostra arredondada e o código.
        let sw = r.ctl;
        cv.fill_round(sw, 8.0 * s, cur, en.max(0.5));
        cv.stroke_round(sw, 8.0 * s, s, fg, 0.25);
        self.label_right(cv, &hex_of(rgb), Font::Body, sw.x - 12.0 * s, sw.cy(), 22.0 * s, fg, 0.8 * en);
        if r.segs.len() <= PICK_HEX {
            return;
        }
        let hover = match self.hover {
            Some(Hit::Pick(h, k)) if h == ri => Some(k),
            _ => None,
        };
        // Cores prontas (anel na escolhida e sob o mouse).
        for (k, &c) in SWATCHES.iter().enumerate() {
            let b = r.segs[k];
            let chosen = c == rgb;
            if chosen || hover == Some(k) {
                cv.stroke_round(b.inset(-3.0 * s), b.w / 2.0 + 3.0 * s, (if chosen { 2.0 } else { 1.0 }) * s, if chosen { self.accent } else { fg }, if chosen { 1.0 } else { 0.4 });
            }
            cv.fill_circle(b.cx(), b.cy(), b.w / 2.0, rgb_f(c), 1.0);
            cv.stroke_round(b, b.w / 2.0, s, fg, 0.18);
        }
        let (h, sat, v) = render::rgb_to_hsv(cur);
        // Matiz × saturação (no brilho máximo).
        let bx = r.segs[PICK_BOX];
        let (x0, y0) = (bx.x, bx.y);
        cv.fill_rrect_ca(bx.x, bx.y, bx.right(), bx.bottom(), 8.0 * s, |x, y| {
            let hh = ((x - x0) / bx.w).clamp(0.0, 1.0) * 360.0;
            let ss = 1.0 - ((y - y0) / bx.h).clamp(0.0, 1.0);
            (render::hsv(hh, ss, 1.0), 1.0)
        });
        cv.stroke_round(bx, 8.0 * s, s, fg, 0.18);
        let (mx, my) = (bx.x + h / 360.0 * bx.w, bx.y + (1.0 - sat) * bx.h);
        cv.fill_circle(mx, my, 8.0 * s, [0.0; 3], 0.35);
        cv.fill_circle(mx, my, 7.0 * s, [1.0; 3], 1.0);
        cv.fill_circle(mx, my, 5.0 * s, render::hsv(h, sat, 1.0), 1.0);
        // Brilho: do preto à cor no máximo.
        let br = r.segs[PICK_BRIGHT];
        let full = render::hsv(h, sat, 1.0);
        let track = RectF::new(br.x, br.cy() - 6.0 * s, br.w, 12.0 * s);
        let tx0 = track.x;
        cv.fill_rrect_ca(track.x, track.y, track.right(), track.bottom(), 6.0 * s, |x, _| (lerp3([0.0; 3], full, ((x - tx0) / track.w).clamp(0.0, 1.0)), 1.0));
        cv.stroke_round(track, 6.0 * s, s, fg, 0.18);
        let kx = track.x + v * track.w;
        cv.fill_circle(kx, track.cy(), 9.0 * s, [0.0; 3], 0.3);
        cv.fill_circle(kx, track.cy(), 8.0 * s, [1.0; 3], 1.0);
        cv.fill_circle(kx, track.cy(), 5.5 * s, cur, 1.0);
        // Código hex (clique para digitar).
        let hx = r.segs[PICK_HEX];
        let editing = self.hex_edit.as_ref().filter(|(r, _)| *r == ri).map(|(_, t)| t.clone());
        self.draw_box(cv, hx, if hover == Some(PICK_HEX) { 1.0 } else { 0.0 }, en);
        if editing.is_some() {
            cv.stroke_round(hx, 5.0 * s, 2.0 * s, self.accent, 1.0);
        }
        let shown = match &editing {
            Some(t) => format!("#{t}"),
            None => hex_of(rgb),
        };
        self.label(cv, &shown, Font::Body, hx.x + 12.0 * s, hx.y, hx.w - 20.0 * s, hx.h, fg, en);
        if editing.is_some() {
            let tw = self.text.mask(&shown, Font::Body, (hx.w - 20.0 * s) as i32, hx.h as i32, Align::Left).used_width();
            let cx = hx.x + 13.0 * s + tw;
            cv.fill_rrect(cx, hx.y + 8.0 * s, cx + 1.5 * s, hx.bottom() - 8.0 * s, 0.5 * s, 1.0, fg);
        }
        self.label(cv, tr("Clique e digite o código (ex.: FF6B9A)"), Font::Caption, hx.x, hx.bottom() + 4.0 * s, 300.0 * s, 18.0 * s, fg, 0.5 * en);
    }

    /// Cor sob o mouse num seletor (quadro ou brilho), sem gravar.
    fn pick_at(&mut self, ri: usize, part: usize, x: f32, y: f32) {
        let Some((key, _)) = self.row(ri) else { return };
        let y = y + self.scroll_px();
        let l = self.layout();
        let Some(r) = l.rows.get(ri) else { return };
        let Some(b) = r.segs.get(part).copied() else { return };
        let cfg = self.cfg();
        let (h, sat, v) = render::rgb_to_hsv(rgb_f(get(&cfg, key) as u32));
        let c = if part == PICK_BOX {
            let hh = ((x - b.x) / b.w).clamp(0.0, 0.999) * 360.0;
            let ss = 1.0 - ((y - b.y) / b.h).clamp(0.0, 1.0);
            // Vindo do preto, o quadro mostra a cor no brilho máximo.
            render::hsv(hh, ss, if v < 0.05 { 1.0 } else { v })
        } else {
            render::hsv(h, sat, ((x - b.x) / b.w).clamp(0.0, 1.0))
        };
        let v = c.iter().fold(0u32, |a, ch| (a << 8) | (ch.clamp(0.0, 1.0) * 255.0).round() as u32);
        if get(&cfg, key) != v as i32 {
            self.shared.preview_config(|cf| set(cf, key, v as i32));
        }
    }

    /// Termina a edição do código hex: grava se for válido (`apply`).
    fn end_hex_edit(&mut self, apply: bool) {
        let Some((ri, t)) = self.hex_edit.take() else { return };
        if let (true, Some((key, _)), Some(_)) = (apply, self.row(ri), parse_hex(&t)) {
            let v = rgb_of(&t) as i32;
            self.shared.update_config(|c| set(c, key, v));
        }
        let r = self.row_window_rect(ri);
        invalidate_rect(self.hwnd, r);
    }

    /// Todos os apps de música achados, na ordem escolhida, com "à mostra?".
    fn app_list(&self, cfg: &Config) -> Vec<(crate::shared::Launcher, bool)> {
        let found = self.shared.launchers.lock().unwrap().clone();
        crate::launchers::arranged(&found, cfg)
    }

    /// Resumo do que a lista de apps mostra (para só redesenhar quando muda).
    fn app_list_sig(&self, ri: usize, cfg: &Config) -> u64 {
        let apps: Vec<(String, bool)> = self.app_list(cfg).into_iter().map(|(l, on)| (l.id, on)).collect();
        let hover = match self.hover {
            Some(Hit::App(r, k)) if r == ri => k as i32,
            _ => -1,
        };
        let drag = self.app_drag.as_ref().filter(|d| d.ri == ri && d.active).map(|d| (d.id.clone(), (d.y + self.scroll_px()).round() as i32));
        ui::layer_key((apps, hover, drag))
    }

    /// Lista dos apps nos atalhos: alça para arrastar, ícone, nome e interruptor.
    fn draw_app_list(&mut self, cv: &mut Canvas, ri: usize, r: &RowL, en: f32) {
        let s = self.scale;
        let fg = self.fg();
        let cfg = self.cfg();
        let apps = self.app_list(&cfg);
        if apps.is_empty() {
            if let Some(row) = r.segs.first() {
                self.label(cv, tr("Nenhum app de música encontrado"), Font::Caption, row.x + 12.0 * s, row.y, row.w, row.h, fg, 0.6 * en);
            }
            return;
        }
        // Coordenadas da camada (sem rolagem) = janela + rolagem.
        let drag = self.app_drag.as_ref().filter(|d| d.ri == ri && d.active).map(|d| (d.id.clone(), d.y + self.scroll_px()));
        let hover = match self.hover {
            Some(Hit::App(h, k)) if h == ri => Some(k),
            _ => None,
        };
        for (k, (app, on)) in apps.iter().enumerate() {
            let Some(row) = r.segs.get(k).copied() else { break };
            if drag.as_ref().is_some_and(|(id, _)| *id == app.id) {
                // Lugar onde o app vai cair.
                cv.stroke_round(row, 7.0 * s, s, fg, 0.18);
                continue;
            }
            self.draw_app_row(cv, row, app, AppRow { on: *on, hot: hover == Some(k), lifted: false }, en);
        }
        if let Some((id, y)) = drag
            && let (Some((app, on)), Some(first), Some(last)) = (apps.iter().find(|(a, _)| a.id == id), r.segs.first(), r.segs.get(apps.len() - 1))
        {
            let ty = (y - first.h / 2.0).clamp(first.y, last.y);
            let row = RectF::new(first.x, ty, first.w, first.h);
            self.draw_app_row(cv, row, app, AppRow { on: *on, hot: true, lifted: true }, en);
        }
    }

    fn draw_app_row(&mut self, cv: &mut Canvas, row: RectF, app: &crate::shared::Launcher, st: AppRow, en: f32) {
        let AppRow { on, hot, lifted } = st;
        let s = self.scale;
        let fg = self.fg();
        if lifted {
            cv.drop_shadow(row, 7.0 * s, 12.0 * s, 4.0 * s, if self.dark { 0.5 } else { 0.18 });
            cv.fill_round(row, 7.0 * s, if self.dark { [0.20, 0.20, 0.22] } else { [1.0; 3] }, 1.0);
            cv.stroke_round(row, 7.0 * s, s, self.accent, 0.6);
        } else {
            cv.fill_round(row, 7.0 * s, fg, (if hot { 0.065 } else { 0.03 }) * en);
        }
        // Alça (seis pontinhos) para arrastar.
        for (dx, dy) in [(0.0, -5.0), (0.0, 0.0), (0.0, 5.0), (5.0, -5.0), (5.0, 0.0), (5.0, 5.0)] {
            cv.fill_circle(row.x + 13.0 * s + dx * s, row.cy() + dy * s, 1.4 * s, fg, (if hot || lifted { 0.7 } else { 0.4 }) * en);
        }
        let k = 22.0 * s;
        let ir = RectF::new((row.x + 34.0 * s).round(), (row.cy() - k / 2.0).round(), k.round(), k.round());
        match &app.icon {
            Some(img) => cv.draw_image(ir, img, 0.0, if on { en } else { 0.5 * en }),
            None => {
                cv.fill_round(ir, k / 2.0, self.accent, 0.9 * en);
                let first: String = app.name.chars().take(1).collect();
                let oa = self.on_accent();
                ui::text_center(cv, &mut self.text, &first, Font::Caption, ir, oa, en);
            }
        }
        let sw = RectF::new(row.right() - 52.0 * s, row.cy() - 10.0 * s, 40.0 * s, 20.0 * s);
        self.label(cv, &app.name, Font::Body, ir.right() + 12.0 * s, row.y, sw.x - ir.right() - 110.0 * s, row.h, fg, if on { en } else { 0.55 * en });
        self.draw_toggle(cv, sw, on, if on { 1.0 } else { 0.0 }, if hot { 1.0 } else { 0.0 }, en);
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
    fn hero_static(&mut self, cv: &mut Canvas, r: RectF, preview: Preview, light_tb: bool, dark: bool, s: f32) {
        let radius = 10.0 * s;
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
        if matches!(preview, Preview::Card | Preview::UpNext | Preview::LockKeys | Preview::Overview | Preview::Lyrics) {
            // (deslocamento do centro e largura em px lógicos; topo e altura em fração da prévia)
            let shapes: [(f32, f32, f32, f32, Rgb); 4] = [
                (-330.0, 0.20, 250.0, 0.55, [0.98, 0.45, 0.55]),
                (-110.0, 0.06, 190.0, 0.36, [1.0, 0.78, 0.35]),
                (40.0, 0.54, 240.0, 0.40, [0.30, 0.80, 0.95]),
                (220.0, 0.10, 150.0, 0.50, [0.62, 0.45, 1.0]),
            ];
            let saved = cv.clip_rect();
            if preview == Preview::Overview {
                cv.set_clip(saved.intersect(RectF::new(r.x, r.y, r.w, r.h - 48.0 * s)));
            }
            for (dx, v, w, h, c) in shapes {
                let b = RectF::new(r.cx() + dx * s, r.y + r.h * v, w * s, r.h * h);
                cv.fill_round(b, 10.0 * s, c, if dark { 0.55 } else { 0.65 });
            }
            cv.set_clip(saved);
        }
        cv.stroke_round(r, radius, s, [0.0; 3], if dark { 0.3 } else { 0.08 });
        let label: Rgb = if dark { [1.0; 3] } else { [0.05, 0.06, 0.12] };
        self.label(cv, tr("PRÉ-VISUALIZAÇÃO"), Font::Label, r.x + 16.0 * s, r.y + 12.0 * s, 200.0 * s, 16.0 * s, label, 0.75);

        if matches!(preview, Preview::Widget | Preview::Visualizer | Preview::Overview) {
            let bar = RectF::new(r.x, r.bottom() - 48.0 * s, r.w, 48.0 * s);
            let bar_c: Rgb = if light_tb { [0.93, 0.93, 0.94] } else { [0.11, 0.11, 0.12] };
            cv.fill_rrect_ca(bar.x, bar.y, bar.right(), bar.bottom(), radius, |_, y| (bar_c, if y < bar.y + radius { 0.0 } else { 0.94 }));
            cv.fill_rrect(bar.x, bar.y, bar.right(), bar.y + radius, 0.0, 0.94, bar_c);
            cv.fill_rrect(bar.x, bar.y, bar.right(), bar.y + s, 0.0, 0.12, if light_tb { [0.0; 3] } else { [1.0; 3] });
            // Ícones no meio, como na barra do Windows 11 (o widget fica ao lado
            // do relógio ou no canto esquerdo).
            let icon_c: Rgb = if light_tb { [0.2; 3] } else { [0.85; 3] };
            let ix0 = bar.cx() - (4.0 * 40.0 + 22.0) * s / 2.0;
            for k in 0..5 {
                let ix = ix0 + k as f32 * 40.0 * s;
                cv.fill_round(RectF::new(ix, bar.cy() - 11.0 * s, 22.0 * s, 22.0 * s), 5.0 * s, icon_c, if k == 0 { 0.9 } else { 0.35 });
            }
            self.label(cv, "12:34", Font::Caption, bar.right() - 56.0 * s, bar.y + 6.0 * s, 50.0 * s, 18.0 * s, icon_c, 0.9);
            self.label(cv, "23/11", Font::Caption, bar.right() - 56.0 * s, bar.y + 24.0 * s, 50.0 * s, 18.0 * s, icon_c, 0.7);
        }
    }

    fn hero_sig(&mut self, hr: RectF, cfg: &Config) -> (HeroSig, f32) {
        let page = &PAGES[self.page];
        let s = self.preview_scale(page, cfg);
        let light_tb = self.preview_light(cfg);
        let key = HeroKey { preview: page.preview, w: hr.w as i32, h: hr.h as i32, scale: s.to_bits(), dark: self.dark, mica: self.mica, light_tb, wall_dark: self.wall_dark() };
        let fps = self.hero_fps(cfg);
        let t = self.t0.elapsed().as_secs_f64();
        let frame = match page.preview {
            // Aviso de teclas: anima ao entrar e ao sair; parado no meio e no
            // intervalo, o quadro é igual e não é refeito.
            Preview::LockKeys if fps > 0.0 && self.demo_key().is_none() => {
                let secs = cfg.osd_duration as f64 / 10.0;
                let period = secs + OSD_GAP as f64;
                let (k, tt) = ((t / period) as u64, t % period);
                if tt >= secs {
                    2_000_000 + k
                } else if tt > 0.65 && tt < secs - 0.25 {
                    1_000_000 + k
                } else {
                    (t * fps as f64) as u64
                }
            }
            _ if fps > 0.0 => (t * fps as f64) as u64,
            _ => 0,
        };
        let media = self.shared.media_version.load(Ordering::Relaxed);
        let state = (self.play_state_for(page.preview), self.demo_key());
        (HeroSig { key, cfg: self.cfg_version(), media, frame, state }, s)
    }

    /// Música, reprodução e cores que a prévia mostra (com os botões dela e a
    /// demonstração em andamento).
    fn scene(&mut self, cfg: &Config, preview: Preview) -> Scene {
        let t = self.t0.elapsed().as_secs_f32();
        let mut colors = self.colors(cfg.palette);
        colors.light_taskbar = self.preview_light(cfg);
        let state = self.play_state_for(preview);
        let mut media = if state == 2 { MediaInfo::default() } else { self.preview_media() };
        let live = media.pos_at.is_some();
        let pos = if live && self.pv_play.is_none() {
            media.position_now()
        } else if state == 0 {
            (110.0 + t as f64) % media.dur.max(1.0)
        } else if live {
            media.position_now()
        } else {
            110.0f64.min(media.dur.max(1.0))
        };
        media.pos = pos;
        media.playing = state == 0;
        if self.demo_key() == Some(Key::CardShuffle) {
            // Um player que não deixa mudar o aleatório e o repetir (um vídeo no navegador).
            (media.can_shuffle, media.can_repeat) = (false, false);
        }
        colors.album = media.art.as_ref().map(|a| a.vibrant);
        let pb = Playback { playing: state == 0, shuffle: media.shuffle, repeat: media.repeat, pos, seek_preview: None, volume: 0.62, muted: false };
        Scene { media, pb, colors, t }
    }

    /// Onde o widget fica na barra simulada: ao lado do relógio ou no canto
    /// esquerdo, com o ajuste de posição (que também guarda o arrasto).
    fn widget_geo(&self, r: RectF, cfg: &Config, media: &MediaInfo, s: f32) -> (ui::WLayout, RectF) {
        let bar = RectF::new(r.x, r.bottom() - 48.0 * s, r.w, 48.0 * s);
        let wh = 40.0 * s;
        let panel = cfg.show_media && (media.present() || cfg.always_show_widget);
        let l = ui::WLayout::build_for(s, wh, cfg, panel, cfg.show_visualizer, self.bar_launchers(cfg), ui::bar_toggles(cfg, media));
        let lo = bar.x + 8.0 * s;
        let hi = (bar.right() - 76.0 * s - l.width).max(lo);
        let base = match cfg.anchor {
            Anchor::BeforeTray => hi,
            Anchor::Left => lo,
        };
        let wx = (base + cfg.offset as f32 * s).clamp(lo, hi);
        let rect = RectF::new(wx.round(), (bar.y + 4.0 * s).round(), l.width.ceil(), wh);
        (l, rect)
    }

    /// Onde o card fica na prévia de "Aparência"/"Apps": em cima do widget,
    /// alinhado à capa dele, como abre na tela.
    fn overview_card_pos(&self, r: RectF, l: &ui::CLayout, wl: &ui::WLayout, wr: RectF, s: f32) -> (f32, f32) {
        let m = ui::SHADOW_MARGIN * s;
        let anchor = wr.x + wl.art.map(|a| a.x).unwrap_or(0.0) - m - 6.0 * s;
        let lo = r.x + 8.0 * s - m;
        let x = anchor.clamp(lo, (r.right() - l.w + m - 8.0 * s).max(lo));
        let y = r.bottom() - 48.0 * s - 8.0 * s - (l.h - m);
        (x.round(), y.round())
    }

    /// Aviso de teclas na prévia e a "telinha" de onde ele aparece (só com espaço).
    fn osd_geo(&self, r: RectF, cfg: &Config, s: f32) -> (RectF, Option<RectF>) {
        let (ow, oh) = ui::osd_size(s, cfg);
        let map = (r.w > 620.0 * s).then(|| {
            let (mw, mh) = (176.0 * s, 99.0 * s);
            RectF::new((r.right() - 28.0 * s - mw).round(), (r.y + 42.0 * s).round(), mw.round(), mh.round())
        });
        let cx = match map {
            Some(m) => (r.x + m.x) / 2.0,
            None => r.cx(),
        };
        (RectF::new((cx - ow / 2.0).round(), (r.y + 22.0 * s).round(), ow.ceil(), oh.ceil()), map)
    }

    /// Área que o componente da prévia ocupa (o que é refeito a cada quadro).
    fn preview_rect(&mut self, r: RectF, preview: Preview, cfg: &Config, s: f32) -> RectF {
        let bar = RectF::new(r.x, r.bottom() - 48.0 * s, r.w, 48.0 * s);
        let caption = RectF::new(r.x + 12.0 * s, r.y + 28.0 * s, (r.w * 0.6).max(200.0 * s), 28.0 * s);
        let rect = match preview {
            Preview::Widget | Preview::Visualizer => bar,
            Preview::Card => {
                let l = self.preview_card(s, cfg);
                RectF::new(r.cx() - l.w / 2.0, r.y + 14.0 * s, l.w, l.h)
            }
            Preview::Overview => {
                let media = self.preview_media();
                let (wl, wr) = self.widget_geo(r, cfg, &media, s);
                let l = self.preview_card(s, cfg);
                let (x, y) = self.overview_card_pos(r, &l, &wl, wr, s);
                union(union(bar, RectF::new(x, y, l.w, l.h)), caption)
            }
            Preview::UpNext => {
                let (uw, uh) = ui::upnext_size(s);
                RectF::new(r.cx() - uw / 2.0, r.y + 12.0 * s, uw, uh)
            }
            Preview::LockKeys => {
                let (osd, map) = self.osd_geo(r, cfg, s);
                // O "Deslizar" desce até 18 px.
                let osd = RectF { h: osd.h + 20.0 * s, ..osd };
                match map {
                    Some(m) => union(osd, RectF::new(m.x - 4.0 * s, m.y - 4.0 * s, m.w + 8.0 * s, m.h + 30.0 * s)),
                    None => osd,
                }
            }
            Preview::Lyrics => lyrics_page::preview_rect(r, cfg, s),
            Preview::None => RectF::default(),
        };
        let rect = rect.intersect(r);
        RectF::new(rect.x.floor(), rect.y.floor(), rect.w.ceil() + 1.0, rect.h.ceil() + 1.0).intersect(r)
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
            let (dx0, dx1) = (dirty.x as usize, (dirty.right() as usize).min(hw));
            for y in dirty.y as usize..(dirty.bottom() as usize).min(hh) {
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
                self.hero_static(&mut hc, local, preview, sig.key.light_tb, sig.key.wall_dark, s);
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
        let sc = self.scene(cfg, preview);
        match preview {
            Preview::Widget | Preview::Visualizer => self.draw_bar_widget(cv, r, cfg, &sc, s),
            Preview::Card => {
                let l = self.preview_card(s, cfg);
                self.draw_card_at(cv, &l, (r.cx() - l.w / 2.0, r.y + 14.0 * s), cfg, &sc, s);
            }
            Preview::Overview => {
                let (wl, wr) = self.widget_geo(r, cfg, &sc.media, s);
                self.draw_bar_widget(cv, r, cfg, &sc, s);
                let l = self.preview_card(s, cfg);
                let (x, y) = self.overview_card_pos(r, &l, &wl, wr, s);
                self.draw_card_at(cv, &l, (x, y), cfg, &sc, s);
                if self.demo_key() == Some(Key::CardShuffle) {
                    self.preview_caption(cv, r.x + 16.0 * s, r.y + 44.0 * s, 0.0, tr("Exemplo: um player que não deixa mudar o aleatório e o repetir"), s);
                }
            }
            Preview::UpNext => {
                let (uw, uh) = ui::upnext_size(s);
                let total = cfg.popup_secs as f32 + 0.5;
                let on = cfg.next_track_popup;
                let tt = if on { (sc.t % (total + 1.2)).min(total - 0.6) } else { total * 0.4 };
                let th = ui::card_theme(cfg, &sc.colors, &sc.media);
                let mut sub = SubCanvas::new(r.cx() - uw / 2.0, r.y + 12.0 * s, uw, uh);
                let backdrop = ui::card_wants_backdrop(cfg).then(|| self.preview_backdrop(ui::upnext_card(s).offset(sub.x, sub.y), ui::glass_blur_px(cfg, s))).flatten();
                ui::draw_upnext(&mut sub.canvas(), &mut self.preview_text, cfg, &sc.colors, &sc.media, &th, s, tt, total, backdrop.as_deref());
                if on {
                    sub.commit(cv);
                } else {
                    // Desligado: o cartão apagado e o aviso no meio.
                    sub.commit_alpha(cv, 0.32);
                    self.preview_caption(cv, r.cx(), r.y + 12.0 * s + uh / 2.0, 0.5, tr("Desligado"), s);
                }
            }
            Preview::LockKeys => self.draw_osd_preview(cv, r, cfg, &sc, s),
            Preview::Lyrics => lyrics_page::draw_preview(cv, &mut self.preview_text, r, cfg, &sc.colors, &sc.media, sc.pb, s, sc.t),
            Preview::None => {}
        }
    }

    /// Widget na barra simulada: posição, barras com a sensibilidade e os
    /// picos, troca de música e ícones animados quando a opção acabou de mudar;
    /// sem nada à mostra, um aviso no lugar dele.
    fn draw_bar_widget(&mut self, cv: &mut Canvas, r: RectF, cfg: &Config, sc: &Scene, s: f32) {
        let (l, wr) = self.widget_geo(r, cfg, &sc.media, s);
        if l.width < 1.0 {
            let why = if !cfg.show_media { "O widget está desligado" } else { "Sem música, o widget some da barra" };
            let bar = RectF::new(r.x, r.bottom() - 48.0 * s, r.w, 48.0 * s);
            // No lugar do widget: à esquerda do relógio (ou no canto, com "À esquerda").
            match cfg.anchor {
                Anchor::BeforeTray => self.preview_caption(cv, bar.right() - 80.0 * s, bar.cy(), 1.0, tr(why), s),
                Anchor::Left => self.preview_caption(cv, bar.x + 12.0 * s, bar.cy(), 0.0, tr(why), s),
            }
            return;
        }
        let t = sc.t;
        let playing = sc.pb.playing;
        let gain = 10f32.powf(cfg.sensitivity_db / 20.0);
        let values: Vec<f32> = (0..cfg.bars)
            .map(|i| {
                let i = i as f32;
                if playing { ((0.42 + 0.36 * ((t * 5.0 + i * 0.9).sin() * (t * 2.3 + i * 0.4).cos())) * gain).clamp(0.04, 1.0) } else { 0.0 }
            })
            .collect();
        let peaks: Vec<f32> = values.iter().enumerate().map(|(i, v)| if playing { (v + 0.10 + 0.06 * (t * 1.7 + i as f32).sin()).min(1.0) } else { 0.0 }).collect();
        let theme = ui::widget_theme(cfg, &sc.colors, &sc.media);
        let bar_apps = self.shown_launchers(cfg);
        // "Animar troca de música" acabado de ligar: a música anterior dá lugar à atual.
        let old = (self.demo_key() == Some(Key::TrackAnim) && cfg.track_anim).then(|| MediaInfo { title: "Neon Skyline".into(), artist: "Taskbeat".into(), art: None, ..sc.media.clone() });
        let p = self.demo.map(|(_, t0)| ((t0.elapsed().as_secs_f32() - 0.2) / 0.42).clamp(0.0, 1.0)).unwrap_or(1.0);
        let mut sub = SubCanvas::new(wr.x, wr.y, wr.w, wr.h);
        let f = ui::WidgetFrame {
            cfg,
            media: &sc.media,
            theme,
            colors: &sc.colors,
            it: Interact { fx: self.icons.fx(cfg.icon_anim), ..Default::default() },
            pb: sc.pb,
            time: t,
            viz: Some((&values, &peaks)),
            flyout_open: false,
            s,
            change: old.as_ref().map(|o| (o, p, p)),
            launchers: &bar_apps,
        };
        ui::draw_widget(&mut sub.canvas(), &mut self.preview_text, &l, &f);
        sub.commit(cv);
    }

    /// Card da prévia em (x, y) (coordenadas da prévia, com a margem da sombra).
    fn draw_card_at(&mut self, cv: &mut Canvas, l: &ui::CLayout, (x, y): (f32, f32), cfg: &Config, sc: &Scene, s: f32) {
        let launchers = self.shown_launchers(cfg);
        let th = ui::card_theme(cfg, &sc.colors, &sc.media);
        let mut m2 = sc.media.clone();
        m2.dur = m2.dur.max(1.0);
        let mut sub = SubCanvas::new(x, y, l.w, l.h);
        let backdrop = ui::card_wants_backdrop(cfg).then(|| self.preview_backdrop(l.card.offset(sub.x, sub.y), ui::glass_blur_px(cfg, s))).flatten();
        // Mini barras paradas num desenho de exemplo (a prévia do card não anima a 60 fps).
        const BARS: &[f32] = &[0.55, 0.9, 0.62, 0.38, 0.74, 0.5];
        let viz = cfg.card_viz.then_some(if sc.pb.playing { BARS } else { &[0.0; 6][..] });
        let empty_mixer = crate::shared::MixerState::default();
        let it = Interact { fx: self.icons.fx(cfg.icon_anim), ..Default::default() };
        let f = ui::CardFrame { cfg, colors: &sc.colors, media: &m2, theme: th, it, pb: sc.pb, s, backdrop: backdrop.as_deref(), hint: None, launchers: &launchers, viz, mixer: Some(&empty_mixer) };
        ui::draw_card(&mut sub.canvas(), &mut self.preview_text, l, &f);
        sub.commit(cv);
    }

    /// Aviso de teclas na prévia: as teclas ligadas se revezam, com a animação
    /// e a duração escolhidas; ao lado, a telinha com o lugar dele na tela.
    fn draw_osd_preview(&mut self, cv: &mut Canvas, r: RectF, cfg: &Config, sc: &Scene, s: f32) {
        let (osd, map) = self.osd_geo(r, cfg, s);
        let mut names: Vec<(&'static str, bool)> = Vec::new();
        if cfg.caps_lock_osd {
            names.push((tr("Caps Lock"), false));
        }
        if cfg.num_lock_osd {
            names.push((tr("Num Lock"), true));
        }
        let th = ui::osd_theme(cfg, &sc.colors);
        let m = ui::SHADOW_MARGIN * s;
        let (cw, ch) = ui::osd_card(s, cfg);
        let card = RectF::new(osd.x + m, osd.y + m, cw, ch);
        let backdrop = ui::osd_wants_backdrop(cfg).then(|| self.preview_backdrop(card, ui::osd_blur_px(cfg, s))).flatten();
        let (w, h) = (osd.w as usize, osd.h as usize);
        let mut px = vec![0u32; w * h];
        match osd_phase(cfg, sc.t, names.len()) {
            None => {
                ui::draw_osd(&mut Canvas::new(w, h, &mut px), &mut self.preview_text, cfg, &sc.colors, &th, s, tr("Caps Lock"), false, true, 1.0, backdrop.as_deref());
                cv.composite(osd.x as i32, osd.y as i32, &px, w, h, 0.32);
                self.preview_caption(cv, osd.cx(), card.cy(), 0.5, tr("Desligado"), s);
            }
            Some((k, on, tt, e)) => {
                let (name, num) = names[k];
                if e > 0.0 {
                    ui::draw_osd(&mut Canvas::new(w, h, &mut px), &mut self.preview_text, cfg, &sc.colors, &th, s, name, num, on, tt, backdrop.as_deref());
                    match cfg.osd_anim {
                        OsdAnim::Slide => cv.composite(osd.x as i32, (osd.y + (1.0 - e) * 18.0 * s).round() as i32, &px, w, h, e),
                        OsdAnim::Fade => cv.composite(osd.x as i32, osd.y as i32, &px, w, h, e),
                        OsdAnim::Zoom => {
                            let img = Image::plain(w, h, px);
                            let k = 0.88 + 0.12 * e;
                            let (zw, zh) = (w as f32 * k, h as f32 * k);
                            cv.draw_image(RectF::new(osd.x + (w as f32 - zw) / 2.0, osd.y + (h as f32 - zh) / 2.0, zw, zh), &img, 0.0, e);
                        }
                    }
                }
            }
        }
        if let Some(map) = map {
            self.draw_osd_map(cv, map, cfg, s);
        }
    }

    /// Telinha com o lugar do aviso (posição e ajustes finos, numa tela de 1920×1080).
    fn draw_osd_map(&mut self, cv: &mut Canvas, map: RectF, cfg: &Config, s: f32) {
        let dark = self.wall_dark();
        let line: Rgb = if dark { [1.0; 3] } else { [0.05, 0.06, 0.12] };
        cv.fill_round(map, 7.0 * s, [0.0; 3], if dark { 0.30 } else { 0.10 });
        cv.stroke_round(map, 7.0 * s, 1.5 * s, line, 0.45);
        let k = map.w / 1920.0;
        let tb_h = (48.0 * k).max(5.0 * s);
        cv.fill_rrect(map.x + 2.0 * s, map.bottom() - tb_h - 1.0 * s, map.right() - 2.0 * s, map.bottom() - 2.0 * s, 4.0 * s, 0.35, line);
        let (cw, ch) = ui::osd_card(1.0, cfg);
        let (sw, sh) = (1920.0f32, 1080.0f32);
        let (x, y) = match cfg.osd_position {
            OsdPosition::Bottom => ((sw - cw) / 2.0, sh - 48.0 - ch - 20.0),
            OsdPosition::Center => ((sw - cw) / 2.0, (sh - ch) / 2.0),
            OsdPosition::Top => ((sw - cw) / 2.0, 40.0),
            OsdPosition::AbovePlayer => (sw - 76.0 - 150.0 - cw / 2.0, sh - 48.0 - ch - 12.0),
        };
        let x = (x + cfg.osd_offset_x as f32).clamp(0.0, sw - cw);
        let y = (y + cfg.osd_offset_y as f32).clamp(0.0, sh - ch);
        let mk = RectF::new(map.x + x * k, map.y + y * k, (cw * k).max(10.0 * s), (ch * k).max(4.0 * s));
        cv.fill_round(mk, (mk.h / 2.0).min(3.0 * s), self.accent, 1.0);
        let label = tr("Onde aparece");
        ui::text_center(cv, &mut self.preview_text, label, Font::Caption, RectF::new(map.x - 20.0 * s, map.bottom() + 4.0 * s, map.w + 40.0 * s, 18.0 * s), line, 0.8);
    }

    /// Recado curto sobre a prévia (pílula com texto), centrado na vertical em
    /// `cy` e preso em `x` pela esquerda (`anchor` 0), pelo meio (0,5) ou pela direita (1).
    fn preview_caption(&mut self, cv: &mut Canvas, x: f32, cy: f32, anchor: f32, text: &str, s: f32) {
        let dark = self.wall_dark();
        let tw = self.preview_text.mask(text, Font::Caption, (900.0 * s) as i32, (20.0 * s) as i32, Align::Left).used_width();
        let (w, h) = ((tw + 24.0 * s).round(), (24.0 * s).round());
        let r = RectF::new((x - w * anchor).round(), (cy - h / 2.0).round(), w, h);
        cv.fill_round(r, h / 2.0, if dark { [0.0; 3] } else { [1.0; 3] }, if dark { 0.55 } else { 0.75 });
        cv.stroke_round(r, h / 2.0, s, if dark { [1.0; 3] } else { [0.0; 3] }, 0.12);
        ui::text_center(cv, &mut self.preview_text, text, Font::Caption, r, if dark { [1.0; 3] } else { [0.05, 0.06, 0.12] }, 0.92);
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

    /// Linha da miniatura da animação do card nesta página.
    fn anim_row(&self) -> Option<usize> {
        (0..self.row_count()).find(|&ri| matches!(self.row(ri), Some((_, Kind::Anim(_)))))
    }

    fn anim_hovered(&self) -> bool {
        matches!(self.hover, Some(Hit::Row(r)) if Some(r) == self.anim_row())
    }

    /// Avança a miniatura: toca uma volta quando a escolha muda e em laço
    /// com o mouse em cima. Parada, não pede quadros.
    fn step_anim_demo(&mut self) {
        let Some(ri) = self.anim_row() else {
            self.anim_demo = None;
            return;
        };
        let cfg = self.cfg();
        let sel = (cfg.card_anim, cfg.anim_speed);
        let was = self.anim_demo.is_some();
        if self.anim_seen != Some(sel) {
            if self.anim_seen.is_some() {
                self.anim_demo = Some(Instant::now());
            }
            self.anim_seen = Some(sel);
        }
        let hovered = self.anim_hovered();
        match self.anim_demo {
            Some(t0) if t0.elapsed().as_secs_f32() >= demo_len(&cfg) => self.anim_demo = hovered.then(Instant::now),
            None if hovered => self.anim_demo = Some(Instant::now()),
            _ => {}
        }
        if was || self.anim_demo.is_some() {
            let r = self.row_window_rect(ri);
            invalidate_rect(self.hwnd, r);
        }
    }

    /// Miniatura do card abrindo e fechando no estilo escolhido, sobre uma
    /// barra de tarefas de brinquedo (parada: o card aberto). Só formas
    /// simples: cada quadro custa quase nada.
    fn draw_anim_tile(&mut self, cv: &mut Canvas, r: RectF, en: f32) {
        let s = self.scale;
        let cfg = self.cfg();
        let rad = 6.0 * s;
        let (x0, y0) = (r.x, r.y);
        cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), rad, |x, y| {
            let u = ((x - x0) / r.w).clamp(0.0, 1.0);
            let v = ((y - y0) / r.h).clamp(0.0, 1.0);
            (lerp3([0.05, 0.09, 0.26], [0.22, 0.12, 0.42], (u * 0.7 + v * 0.3).clamp(0.0, 1.0)), en)
        });
        let bar = RectF::new(r.x, r.bottom() - 9.0 * s, r.w, 9.0 * s);
        cv.fill_rrect_ca(r.x, r.y, r.right(), r.bottom(), rad, |_, y| ([0.11, 0.11, 0.12], if y < bar.y { 0.0 } else { 0.96 * en }));
        // Widget na barra (de onde o card sai).
        let wid = RectF::new(r.right() - 46.0 * s, bar.y + 2.0 * s, 32.0 * s, bar.h - 4.0 * s);
        cv.fill_round(wid, 2.5 * s, [1.0; 3], 0.14 * en);
        let dot = RectF::new(wid.x + 1.5 * s, wid.y + 1.0 * s, wid.h - 2.0 * s, wid.h - 2.0 * s);
        cv.fill_round(dot, 1.2 * s, [0.98, 0.62, 0.42], en);

        // Card parado: acima do widget, alinhado à capa dele (como no app).
        let (cw, ch) = (66.0 * s, 27.0 * s);
        let body = RectF::new((dot.x - 5.0 * s).clamp(r.x + 5.0 * s, r.right() - 5.0 * s - cw), bar.y - 4.0 * s - ch, cw, ch);
        // As distâncias da animação são de um card de ~150 px: em miniatura.
        let k = ch / (150.0 * s);
        let ctx = anim::Ctx { bar_below: true, travel: (bar.y - body.y) / s / k };
        let pose = match self.anim_demo {
            None => Pose::REST,
            Some(t0) => demo_pose(&cfg, t0.elapsed().as_secs_f32(), ctx),
        };
        if pose.alpha <= 0.003 {
            return;
        }
        let old = cv.clip_rect();
        let mut clip = r.inset(1.0).intersect(old);
        if pose.clip {
            clip = clip.intersect(RectF::new(r.x, r.y, r.w, bar.y - r.y));
        }
        cv.set_clip(clip);
        let (ox, oy) = match pose.origin {
            anim::Origin::Center => (body.cx(), body.cy()),
            anim::Origin::Anchor => (dot.cx().clamp(body.x, body.right()), body.bottom()),
        };
        let sc = pose.scale;
        let b = RectF::new(ox + (body.x - ox) * sc, oy + (body.y - oy) * sc + pose.dy * k * s, body.w * sc, body.h * sc);
        let a = pose.alpha * en;
        let u = sc * s;
        anim::draw_burst(cv, b, 4.0 * u, pose.glow * a, pose.ring, self.accent, 0.35 * s);
        cv.drop_shadow(b, 4.0 * u, 5.0 * u, 1.5 * u, 0.4 * a);
        let (bx, by) = (b.x, b.y);
        cv.fill_rrect_ca(b.x, b.y, b.right(), b.bottom(), 4.0 * u, |x, y| {
            let t = (((x - bx) / b.w) * 0.4 + ((y - by) / b.h) * 0.6).clamp(0.0, 1.0);
            (lerp3([0.36, 0.20, 0.42], [0.16, 0.09, 0.22], t), a)
        });
        let art = RectF::new(b.x + 3.5 * u, b.y + 3.5 * u, b.h - 7.0 * u, b.h - 7.0 * u);
        let (ax, ay) = (art.x, art.y);
        cv.fill_rrect_ca(art.x, art.y, art.right(), art.bottom(), 2.0 * u, |x, y| {
            let t = (((y - ay) / art.h) * 0.8 + ((x - ax) / art.w) * 0.2).clamp(0.0, 1.0);
            (lerp3([0.98, 0.70, 0.45], [0.55, 0.25, 0.55], t), a)
        });
        let tx = art.right() + 4.0 * u;
        cv.fill_round(RectF::new(tx, b.y + 6.0 * u, 26.0 * u, 3.0 * u), 1.5 * u, [1.0; 3], 0.9 * a);
        cv.fill_round(RectF::new(tx, b.y + 11.5 * u, 16.0 * u, 2.5 * u), 1.2 * u, [1.0; 3], 0.5 * a);
        cv.fill_circle(b.right() - 9.0 * u, b.bottom() - 7.5 * u, 3.4 * u, [1.0; 3], 0.92 * a);
        cv.set_clip(old);
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

/// Pausa entre um aviso e o próximo na prévia do Caps Lock (s).
const OSD_GAP: f32 = 0.9;

/// Momento do aviso na prévia (as teclas ligadas se revezam, cada uma ligando
/// e, com "Avisar também ao desligar", desligando): (qual tecla, ligada,
/// tempo desde que apareceu, opacidade/entrada 0..1). `None`: nenhuma ligada.
fn osd_phase(cfg: &Config, t: f32, n: usize) -> Option<(usize, bool, f32, f32)> {
    if n == 0 {
        return None;
    }
    let secs = cfg.osd_duration as f32 / 10.0;
    let period = secs + OSD_GAP;
    let k = (t / period) as usize;
    let tt = t % period;
    let on = !cfg.osd_show_off || k.is_multiple_of(2);
    let per = if cfg.osd_show_off { 2 } else { 1 };
    let e = if tt >= secs {
        0.0
    } else if tt < 0.3 {
        ui::ease_enter(tt / 0.3)
    } else if tt > secs - 0.22 {
        1.0 - ui::ease_exit((tt - (secs - 0.22)) / 0.22)
    } else {
        1.0
    };
    Some(((k / per) % n, on, tt, e))
}

/// Menor retângulo com `a` e `b` (um vazio não conta).
fn union(a: RectF, b: RectF) -> RectF {
    if a.is_empty() {
        return b;
    }
    if b.is_empty() {
        return a;
    }
    let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
    RectF::new(x0, y0, a.right().max(b.right()) - x0, a.bottom().max(b.bottom()) - y0)
}

/// Texto do "Avançado": os títulos das linhas que ele guarda.
fn adv_summary(page: usize, item: usize) -> String {
    let items = &PAGES[page].items[item + 1..];
    let titles: Vec<&str> = items.iter().take_while(|i| matches!(i, Row { .. })).filter_map(|i| if let Row { title, .. } = i { Some(tr(title)) } else { None }).collect();
    titles.join(", ")
}

/// O que a prévia mostra num quadro.
struct Scene {
    media: MediaInfo,
    pb: Playback,
    colors: Colors,
    t: f32,
}

/// Altura (px lógicos) de uma linha comum.
const ROW_H: f32 = 64.0;
/// Altura (px lógicos) da linha do seletor de cor aberto.
const PICKER_H: f32 = 268.0;
/// Partes do seletor de cor: 0..12 = cores prontas; depois o quadro, o brilho e o hex.
const PICK_BOX: usize = SWATCHES.len();
const PICK_BRIGHT: usize = PICK_BOX + 1;
const PICK_HEX: usize = PICK_BOX + 2;

/// Retângulos das partes do seletor de cor aberto numa linha `card`.
fn picker_parts(card: RectF, s: f32) -> Vec<RectF> {
    let x0 = card.x + 60.0 * s;
    let y0 = card.y + 64.0 * s;
    let mut parts: Vec<RectF> = (0..SWATCHES.len()).map(|k| RectF::new(x0 + k as f32 * 36.0 * s, y0, 26.0 * s, 26.0 * s)).collect();
    let bw = (card.w - 60.0 * s - 260.0 * s).clamp(160.0 * s, 440.0 * s);
    let bx = RectF::new(x0, y0 + 42.0 * s, bw, 110.0 * s);
    parts.push(bx);
    parts.push(RectF::new(x0, bx.bottom() + 12.0 * s, bw, 26.0 * s));
    parts.push(RectF::new(bx.right() + 28.0 * s, bx.y, 150.0 * s, 34.0 * s));
    parts
}

/// 0xRRGGBB em 0..1.
fn rgb_f(v: u32) -> Rgb {
    [((v >> 16) & 0xff) as f32 / 255.0, ((v >> 8) & 0xff) as f32 / 255.0, (v & 0xff) as f32 / 255.0]
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
        self.commit_alpha(cv, 1.0);
    }

    fn commit_alpha(self, cv: &mut Canvas, alpha: f32) {
        cv.composite(self.x as i32, self.y as i32, &self.px, self.w, self.h, alpha);
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
                u.start_demo(key);
            }
            u.schedule();
            Action::Redraw
        }
        Hit::Chip(k) => {
            let chip = u.layout().chips.get(k).map(|c| c.0);
            match chip {
                Some(Chip::Play(p)) => u.pv_play = Some(p),
                Some(Chip::Light(v)) => u.pv_light = Some(v),
                None => {}
            }
            u.schedule();
            Action::Redraw
        }
        Hit::Off(_) => Action::None,
        Hit::Pick(ri, k) => {
            let Some((key, _)) = u.row(ri) else { return Action::None };
            if k < PICK_BOX {
                u.shared.update_config(|c| set(c, key, SWATCHES[k] as i32));
            } else if k == PICK_HEX {
                let cur = hex_of(get(&u.cfg(), key) as u32);
                u.hex_edit = Some((ri, cur.trim_start_matches('#').to_string()));
            }
            u.schedule();
            Action::Redraw
        }
        Hit::App(_, k) => {
            let cfg = u.cfg();
            let mut all = u.app_list(&cfg);
            if let Some((_, on)) = all.get_mut(k) {
                *on = !*on;
                // Ordem e escondidos juntos: vale também para a primeira mudança
                // de quem estava com o atalho escolhido sozinho.
                let (order, hidden) = crate::launchers::lists_of(&all);
                u.shared.update_config(|c| {
                    c.launcher_order = order;
                    c.launcher_hidden = hidden;
                });
            }
            u.schedule();
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
                        u.start_demo(key);
                    }
                    u.schedule();
                    Action::Redraw
                }
                Kind::Expander => {
                    u.toggle_advanced(ri);
                    u.schedule();
                    Action::Redraw
                }
                Kind::Choice(names) | Kind::Anim(names) => {
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
                    u.shared.update_config(|c| *c = Config { login_default_applied: true, ..Config::default() });
                    crate::i18n::apply(Language::Auto);
                    // O padrão também é iniciar com o Windows.
                    system::set_autostart(true);
                    AUTOSTART.with(|a| a.set(system::autostart_enabled()));
                    u.schedule();
                    Action::Redraw
                }
                Kind::Button(_) if key == Key::ResetPos => {
                    u.shared.update_config(|c| {
                        c.anchor = Anchor::BeforeTray;
                        c.offset = 0;
                    });
                    u.schedule();
                    Action::Redraw
                }
                Kind::Button(_) if key == Key::Licenses => {
                    crate::licenses::open();
                    Action::Redraw
                }
                Kind::Button(_) if key == Key::BrowserExt => {
                    if !ext_connected() {
                        crate::webext::open_install_page();
                    }
                    Action::Redraw
                }
                Kind::Button(_) if key == Key::OsdTest => {
                    u.shared.osd_test.store(true, Ordering::Relaxed);
                    Action::Redraw
                }
                Kind::Button(_) if matches!(key, Key::Lyrics(_)) => {
                    if let Key::Lyrics(k) = key {
                        lyrics_page::press(&u.shared, k);
                    }
                    Action::Redraw
                }
                Kind::Button(_) if key == Key::UpdateNow => {
                    crate::update::check_now();
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
    /// Abre ou fecha o "Avançado" da página; aberto, rola até as linhas dele
    /// aparecerem (sem esconder o cabeçalho).
    fn toggle_advanced(&mut self, ri: usize) {
        self.adv_open ^= 1 << self.page;
        self.layout = None;
        self.content = None;
        if !self.adv_is_open() {
            let max = self.max_scroll();
            self.scroll_target = self.scroll_target.min(max);
            return;
        }
        let (_, h) = self.client();
        let l = self.layout();
        let Some(head) = l.rows.get(ri).map(|r| r.card) else { return };
        let bottom = l.rows.get(ri).map(|r| r.group.bottom()).unwrap_or(head.bottom());
        let view = h - l.list_top;
        let want = (bottom + 16.0 * self.scale - l.list_top - view).min(head.y - l.list_top - 12.0 * self.scale);
        let max = (l.content_h - h).max(0.0);
        if want > self.scroll_target {
            self.scroll_target = want.clamp(0.0, max);
        }
    }

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

        self.step_anim_demo();

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
                if row.hidden {
                    continue;
                }
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

    /// Arrastando um app da lista: põe ele na posição sob o mouse (sem gravar).
    fn drag_app_to(&mut self, y: f32) {
        let Some(d) = &self.app_drag else { return };
        let (ri, id) = (d.ri, d.id.clone());
        let cy = y + self.scroll_px();
        let l = self.layout();
        let Some(row) = l.rows.get(ri) else { return };
        let cfg = self.cfg();
        let apps = self.app_list(&cfg);
        let n = apps.len().min(row.segs.len());
        let Some(to) = (0..n).min_by(|a, b| (row.segs[*a].cy() - cy).abs().total_cmp(&(row.segs[*b].cy() - cy).abs())) else { return };
        if apps.iter().position(|(a, _)| a.id == id) != Some(to) {
            let order = crate::launchers::moved(&apps, &id, to);
            let (_, hidden) = crate::launchers::lists_of(&apps);
            self.shared.preview_config(|c| {
                c.launcher_order = order;
                c.launcher_hidden = hidden;
            });
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
                Hit::Row(ri) | Hit::Seg(ri, _) | Hit::Off(ri) | Hit::App(ri, _) | Hit::Pick(ri, _) => {
                    let r = self.row_window_rect(ri);
                    invalidate_rect(self.hwnd, r);
                }
                Hit::Chip(k) => {
                    if let Some((_, r)) = self.layout().chips.get(k) {
                        invalidate_rect(self.hwnd, *r);
                    }
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
                    if let Some((ri, part)) = u.color_drag {
                        u.pick_at(ri, part, x, y);
                        let r = u.row_window_rect(ri);
                        invalidate_rect(hwnd, r);
                        if let Some(hero) = u.layout().hero {
                            invalidate_rect(hwnd, hero);
                        }
                        drag_changed = true;
                        return;
                    }
                    if let Some(d) = u.app_drag.as_mut() {
                        d.y = y;
                        if !d.active && (y - d.y0).abs() > 4.0 * u.scale {
                            d.active = true;
                        }
                        if d.active {
                            let ri = d.ri;
                            u.drag_app_to(y);
                            let r = u.row_window_rect(ri);
                            invalidate_rect(hwnd, r);
                            if let Some(hero) = u.layout().hero {
                                invalidate_rect(hwnd, hero);
                            }
                            drag_changed = true;
                            return;
                        }
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
                    // Clicou fora do campo hex: termina a edição (grava se válido).
                    if u.hex_edit.is_some() && !matches!(h, Some(Hit::Pick(_, PICK_HEX))) {
                        u.end_hex_edit(true);
                    }
                    if let Some(Hit::Pick(ri, part @ (PICK_BOX | PICK_BRIGHT))) = h {
                        u.color_drag = Some((ri, part));
                        u.pick_at(ri, part, x, y);
                        let r = u.row_window_rect(ri);
                        invalidate_rect(hwnd, r);
                        if let Some(hero) = u.layout().hero {
                            invalidate_rect(hwnd, hero);
                        }
                    }
                    if let Some(Hit::App(ri, k)) = h {
                        let cfg = u.cfg();
                        if let Some((app, _)) = u.app_list(&cfg).get(k) {
                            u.app_drag = Some(AppDrag { ri, id: app.id.clone(), y0: y, y, active: false });
                        }
                    }
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
                    if let Some((ri, _)) = u.color_drag.take() {
                        u.shared.save_config();
                        u.pressed = None;
                        let r = u.row_window_rect(ri);
                        invalidate_rect(hwnd, r);
                        return Action::None;
                    }
                    if let Some(d) = u.app_drag.take()
                        && d.active
                    {
                        u.shared.save_config();
                        u.pressed = None;
                        let r = u.row_window_rect(d.ri);
                        invalidate_rect(hwnd, r);
                        u.schedule();
                        return Action::None;
                    }
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
                            // Escolha nova (a miniatura da animação toca).
                            u.schedule();
                        });
                    }
                }
                LRESULT(0)
            }
            // Digitando o código hex de uma cor.
            WM_CHAR => {
                let ch = wp.0 as u32;
                let used = with_ui(|u| {
                    let Some((ri, t)) = u.hex_edit.as_mut() else { return false };
                    let ri = *ri;
                    match ch {
                        8 => {
                            t.pop();
                        }
                        13 => {
                            u.end_hex_edit(true);
                            return true;
                        }
                        27 => {
                            u.end_hex_edit(false);
                            return true;
                        }
                        _ => match char::from_u32(ch).filter(|c| c.is_ascii_hexdigit()) {
                            Some(c) if t.len() < 6 => t.push(c.to_ascii_uppercase()),
                            _ => return true,
                        },
                    }
                    let r = u.row_window_rect(ri);
                    invalidate_rect(hwnd, r);
                    true
                })
                .unwrap_or(false);
                if used { LRESULT(0) } else { DefWindowProcW(hwnd, msg, wp, lp) }
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
                if ALONE.with(|a| a.get()) {
                    PostQuitMessage(0);
                }
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
        let off3 = Config { widget_tint: WidgetTint::Gradient, style: Style::Wave, card_style: CardStyle::AlbumColor, ..Config::default() };
        for p in PAGES {
            check(p.name);
            check(p.subtitle);
            if let Some(h) = p.header {
                check(h);
            }
            for it in p.items {
                match it {
                    Section(n) => check(n),
                    Row { title, desc, kind, key, .. } => {
                        check(title);
                        check(desc);
                        match kind {
                            Kind::Segmented(o) | Kind::Choice(o) | Kind::Anim(o) => o.iter().for_each(|t| check(t)),
                            Kind::Button(t) => check(t),
                            _ => {}
                        }
                        // As dicas de opção desativada também.
                        for c in [&off, &off2, &off3] {
                            if let Some(why) = requirement(c, *key) {
                                check(why);
                            }
                        }
                    }
                }
            }
        }
        check("Nenhum app de música encontrado");
        check("Clique e digite o código (ex.: FF6B9A)");
        for c in [Chip::Play(0), Chip::Play(1), Chip::Play(2), Chip::Light(true), Chip::Light(false)] {
            check(chip_label(c));
        }
        for t in ["O widget está desligado", "Sem música, o widget some da barra", "Exemplo: um player que não deixa mudar o aleatório e o repetir", "Desligado", "Onde aparece", "PRÉ-VISUALIZAÇÃO"] {
            check(t);
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
