//! Configurações persistidas em %APPDATA%\Taskbeat\config.toml (Windows) ou
//! ~/Library/Application Support/Taskbeat/config.toml (Mac).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    Bars,
    Mirror,
    Wave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Palette {
    Accent,
    /// Cor extraída da capa do álbum.
    Album,
    Rainbow,
    Neon,
    Mono,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// Logo à esquerda da área de notificação (relógio, ícones).
    BeforeTray,
    /// No canto esquerdo da barra de tarefas.
    Left,
}

/// Fundo do card expandido e do cartão "A seguir".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardStyle {
    /// Capa desfocada ocupando o fundo + gradiente escuro.
    Artwork,
    /// Cor extraída da capa.
    AlbumColor,
    /// Acrílico do Fluent: desfoca o que está atrás, com tinta clara/escura.
    Glass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardLayout {
    /// Capa à esquerda, informações à direita.
    Horizontal,
    /// Capa grande em cima.
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Rounded,
    Circle,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Auto,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccentSource {
    Windows,
    Album,
    /// Uma cor escolhida (`accent_rgb`).
    Custom,
}

/// Sombra do card expandido e do cartão "A seguir".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardShadow {
    /// A de sempre: larga e escura.
    Strong,
    Soft,
    None,
}

/// Pontas das barras de áudio.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BarShape {
    Rounded,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corners {
    Small,
    Medium,
    Large,
}

/// Onde o aviso de Caps/Num Lock aparece.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsdPosition {
    /// Embaixo, centralizado, logo acima da barra de tarefas (padrão).
    Bottom,
    /// Centro da tela.
    Center,
    /// No alto da tela, centralizado.
    Top,
    /// Logo acima do widget da música na barra de tarefas.
    AbovePlayer,
}

/// Cantos do aviso de Caps/Num Lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsdShape {
    /// Mesmos cantos do card expandido (opção "Cantos" do card).
    Rounded,
    Pill,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsdSize {
    Compact,
    Normal,
    Large,
}

impl OsdSize {
    pub fn factor(self) -> f32 {
        match self {
            OsdSize::Compact => 0.85,
            OsdSize::Normal => 1.0,
            OsdSize::Large => 1.2,
        }
    }
}

/// Cantos do fundo do widget da barra.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetShape {
    Rounded,
    /// Totalmente arredondado nas pontas.
    Pill,
    Square,
}

/// Cor dos controles (aleatório, anterior, play, próximo, repetir).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlColor {
    /// Tudo branco (escuro sobre fundos claros), play branco com o ícone escuro.
    White,
    /// Ícones brancos; play e botões ligados na cor de destaque.
    WhiteAccent,
    /// Tudo na cor viva da capa.
    Album,
    /// Tudo na cor de destaque (Windows ou capa, conforme "Cor de destaque").
    Accent,
    /// Tudo numa cor escolhida.
    Custom,
}

/// Cor dos botões laterais (aleatório, anterior, próximo, repetir).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideColor {
    /// A mesma cor do botão do meio (play/pausa).
    SameAsMiddle,
    /// Branco (escuro sobre fundos claros).
    White,
    /// Cor viva da capa.
    Album,
    /// Cor de destaque.
    Accent,
}

/// Cor "#RRGGBB" (ou "#RGB") em 0..1.
pub fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let h = s.trim().trim_start_matches('#');
    let full: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 => h.to_string(),
        _ => return None,
    };
    let v = u32::from_str_radix(&full, 16).ok()?;
    Some([((v >> 16) & 0xff) as f32 / 255.0, ((v >> 8) & 0xff) as f32 / 255.0, (v & 0xff) as f32 / 255.0])
}

/// Cor 0xRRGGBB como "#RRGGBB".
pub fn hex_of(rgb: u32) -> String {
    format!("#{:06X}", rgb & 0xff_ffff)
}

/// "#RRGGBB" como 0xRRGGBB (preto se não der para ler).
pub fn rgb_of(s: &str) -> u32 {
    parse_hex(s).map(|c| c.iter().fold(0u32, |a, v| (a << 8) | (v * 255.0).round() as u32)).unwrap_or(0)
}

/// Botão play no widget da barra.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskbarPlay {
    /// Círculo na cor de destaque.
    Filled,
    /// Só o ícone, como os outros botões.
    Icon,
}

/// Tamanho de um elemento do widget (texto, capa).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Size3 {
    Small,
    Normal,
    Large,
}

/// O que vai na segunda linha do widget (embaixo do título).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecondLine {
    Artist,
    /// "Artista · Álbum".
    ArtistAlbum,
    Album,
    /// Tempo decorrido e duração ("1:23 / 3:45").
    Time,
}

/// Cor do fundo do widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetTint {
    /// Branco/preto translúcido, como os botões da barra.
    Neutral,
    /// Cor viva da capa.
    Album,
    /// Cor de destaque (Windows ou capa, conforme "Cor de destaque").
    Accent,
    /// Gradiente vivo com a cor da capa: profundo na barra escura, pastel na
    /// clara, com um brilho atrás da capa.
    Gradient,
    /// A própria capa desfocada ocupando o fundo, sob uma película.
    Artwork,
}

impl WidgetTint {
    /// Fundos "fortes" (gradiente e capa desfocada), com intensidade própria.
    pub fn is_rich(self) -> bool {
        matches!(self, WidgetTint::Gradient | WidgetTint::Artwork)
    }
}

/// Desenho dos ícones de controle (anterior, play, próximo, aleatório...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IconStyle {
    /// Formas preenchidas e encorpadas, desenhadas em vetor.
    Filled,
    /// Ícones finos de contorno do Segoe Fluent.
    Fluent,
}

/// Onde ficam os atalhos dos apps de música.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LauncherPlace {
    /// Linha embaixo do card.
    Card,
    /// Botões pequenos dentro do widget da barra.
    Taskbar,
    /// Nos dois lugares.
    Both,
}

impl LauncherPlace {
    pub fn card(self) -> bool {
        matches!(self, LauncherPlace::Card | LauncherPlace::Both)
    }
    pub fn taskbar(self) -> bool {
        matches!(self, LauncherPlace::Taskbar | LauncherPlace::Both)
    }
}

/// Aleatório e repetir: sempre (apagados quando o player não deixa) ou só quando dá.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShuffleRepeat {
    Always,
    WhenSupported,
}

/// Fundo do aviso de Caps/Num Lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsdMaterial {
    /// Cartão sólido claro/escuro.
    Solid,
    /// Vidro fosco: desfoca o que está atrás, com tinta do tema e granulado.
    Acrylic,
    /// Preenchido com a cor de destaque.
    Accent,
}

/// Símbolo do aviso de Caps/Num Lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsdIcon {
    /// "A" para Caps Lock, "1" para Num Lock.
    Letter,
    /// Cadeado fechado (ligado) / aberto (desligado).
    Lock,
    /// Teclado (Caps) / teclado numérico (Num).
    Keyboard,
}

/// Animação de entrada e saída do aviso.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsdAnim {
    /// Desliza alguns pixels enquanto aparece.
    Slide,
    /// Cresce a partir do centro.
    Zoom,
    /// Só esmaece.
    Fade,
}

/// Idioma da interface. `Auto` segue o idioma de exibição do Windows (outros = inglês).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Auto,
    Pt,
    En,
    Es,
    De,
}

/// Fundo do notch aberto (Mac). O topo fica sempre preto, emendado no notch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotchBg {
    /// Preto puro, como o notch.
    Black,
    /// Preto em cima, cor da capa embaixo.
    Album,
    /// Capa desfocada, escurecendo até o preto em cima.
    Artwork,
}

impl Corners {
    /// Raio do card em pixels lógicos.
    pub fn radius(self) -> f32 {
        match self {
            Corners::Small => 8.0,
            Corners::Medium => 14.0,
            Corners::Large => 20.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // ---- barra de tarefas
    /// Capa + título/artista na barra de tarefas.
    pub show_media: bool,
    /// Widget sempre na barra, mesmo sem nada tocando ("Nada tocando").
    pub always_show_widget: bool,
    /// Botões ⏮ ▶ ⏭ também na barra (por padrão só no card).
    pub taskbar_controls: bool,
    pub cover_shape: Shape,
    pub show_artist: bool,
    /// Rola títulos que não cabem.
    pub marquee: bool,
    /// Mini equalizador animado ao lado do título enquanto toca.
    pub playing_indicator: bool,
    /// Linha fina de progresso sob o widget.
    pub taskbar_progress: bool,
    /// Fundo em "pílula" sempre visível atrás do widget (envolve capa, texto e botões).
    pub widget_pill: bool,
    /// Cantos do fundo do widget.
    pub widget_shape: WidgetShape,
    /// Linha fina separando a música dos botões.
    pub widget_divider: bool,
    /// Aleatório e repetir também na barra (com os botões de controle ligados).
    pub taskbar_shuffle_repeat: bool,
    /// Botão play da barra: preenchido com a cor de destaque ou só o ícone.
    pub taskbar_play: TaskbarPlay,
    /// Cor dos controles do widget (padrão: branco).
    pub widget_controls: ControlColor,
    /// Cor dos controles do widget na opção "Personalizada" ("#RRGGBB").
    pub widget_controls_rgb: String,
    /// Cor dos botões laterais do widget (padrão: a do botão do meio).
    pub widget_sides: SideColor,
    /// Intensidade (%) dos botões laterais do widget (100 = tão fortes quanto o do meio).
    pub widget_sides_alpha: i32,
    /// O que aparece embaixo do título (com "Mostrar artista" ligado).
    pub second_line: SecondLine,
    /// Tamanho do título e da segunda linha.
    pub text_size: Size3,
    /// Tamanho da capa na barra.
    pub cover_size: Size3,
    /// Cor do fundo do widget.
    pub widget_tint: WidgetTint,
    /// Opacidade do fundo do widget (%), nas cores neutra, da capa e de destaque.
    pub widget_opacity: i32,
    /// Intensidade (%) dos fundos fortes (gradiente da capa, capa desfocada).
    pub widget_intensity: i32,
    /// Brilho na cor da capa atrás dela, nos fundos fortes.
    pub widget_glow: bool,
    /// Contorno fino em volta do fundo.
    pub widget_border: bool,
    /// Arrastar o widget para mudar o lugar dele na barra (clique simples abre o card).
    pub widget_drag: bool,
    /// Desenho dos ícones de controle (na barra e no card).
    pub icon_style: IconStyle,
    /// Anima a troca de música no widget (texto desliza, capa esmaece).
    pub track_anim: bool,
    /// Anima os ícones dos botões (play/pausa, pular, aleatório, repetir, volume).
    pub icon_anim: bool,
    /// Largura da área do título, em pixels lógicos.
    pub title_width: i32,
    pub anchor: Anchor,
    /// Deslocamento horizontal extra, em pixels lógicos (96 DPI).
    pub offset: i32,

    // ---- card expandido
    pub card_style: CardStyle,
    pub card_layout: CardLayout,
    pub theme: ThemeMode,
    pub accent_source: AccentSource,
    pub corners: Corners,
    pub play_shape: Shape,
    /// Cor dos controles do card (padrão: branco, como no widget).
    pub card_controls: ControlColor,
    /// Cor dos controles do card na opção "Personalizada" ("#RRGGBB").
    pub card_controls_rgb: String,
    /// Cor e intensidade (%) dos botões laterais do card.
    pub card_sides: SideColor,
    pub card_sides_alpha: i32,
    pub card_seek: bool,
    /// Mostra de qual app vem a música (nome no card, ícone sobre a capa).
    pub show_source: bool,
    /// Volume no card expandido (slider + mudo).
    pub show_volume: bool,
    /// Setinha embaixo do card que abre o mixer de volume (volume de cada
    /// app; só no Windows).
    pub card_mixer: bool,
    /// Quando mostrar aleatório e repetir no card.
    pub card_shuffle_repeat: ShuffleRepeat,
    /// Segundos sem o mouse em cima até o card fechar sozinho.
    pub card_autoclose: i32,
    /// Atalhos para abrir os apps de música instalados (linha embaixo do card).
    pub card_launchers: bool,
    /// Onde os atalhos aparecem (card, barra ou os dois).
    pub launcher_place: LauncherPlace,
    /// Ordem escolhida dos atalhos (AUMID de cada app). Apps achados depois
    /// entram no fim, na ordem de sempre.
    pub launcher_order: Vec<String>,
    /// Atalhos escondidos (AUMID).
    pub launcher_hidden: Vec<String>,
    /// Mini barras de áudio no card, ao lado do título (com as barras ligadas).
    pub card_viz: bool,
    /// Cor de destaque na opção "Personalizada" ("#RRGGBB").
    pub accent_rgb: String,
    /// Sombra do card e do "A seguir".
    pub card_shadow: CardShadow,
    /// Granulado fino no fundo do card e do "A seguir".
    pub card_grain: bool,
    /// Desfoque do fundo acrílico do card e do "A seguir" (%; 50 = o de sempre).
    pub glass_blur: i32,
    /// Quanto a película do acrílico do card cobre o desfoque (%).
    pub glass_tint: i32,
    /// Película escura sobre a capa desfocada do card (% da de sempre).
    pub artwork_dim: i32,

    // ---- "A seguir"
    /// Cartão animado com a próxima música quando o player troca sozinho.
    pub next_track_popup: bool,
    pub popup_secs: i32,

    // ---- teclas de bloqueio
    /// Animação ao ligar/desligar o Caps Lock.
    pub caps_lock_osd: bool,
    /// Animação ao ligar/desligar o Num Lock.
    pub num_lock_osd: bool,
    pub osd_position: OsdPosition,
    /// Ajuste fino a partir da predefinição, em pixels lógicos.
    pub osd_offset_x: i32,
    pub osd_offset_y: i32,
    pub osd_shape: OsdShape,
    pub osd_size: OsdSize,
    pub osd_theme: ThemeMode,
    /// Duração do aviso, em décimos de segundo.
    pub osd_duration: i32,
    /// Também avisa quando a tecla é desligada.
    pub osd_show_off: bool,
    /// Interruptor animado no lado direito do aviso.
    pub osd_switch: bool,
    pub osd_material: OsdMaterial,
    /// Quanto o fundo cobre o que está atrás (%): opacidade do cartão sólido /
    /// colorido, ou da tinta sobre o desfoque no acrílico.
    pub osd_opacity: i32,
    /// Intensidade do desfoque do fundo acrílico (%).
    pub osd_blur: i32,
    pub osd_icon: OsdIcon,
    pub osd_anim: OsdAnim,

    // ---- visualizador
    /// Barras/onda reagindo ao som. Desligado por padrão.
    pub show_visualizer: bool,
    pub style: Style,
    pub palette: Palette,
    /// Largura do visualizador, em pixels lógicos.
    pub width: i32,
    pub bars: usize,
    /// Ganho extra em dB (negativo = menos sensível).
    pub sensitivity_db: f32,
    /// Só reage quando algum app de mídia (Spotify, navegador...) está tocando.
    pub only_when_media: bool,
    /// Barras ouvem só o app que está tocando (e não chamadas, jogos, avisos).
    pub viz_player_only: bool,
    pub show_peaks: bool,
    /// Pontas das barras (arredondadas ou retas).
    pub bar_shape: BarShape,

    // ---- notch (Mac)
    /// Abre o card ao passar o mouse no notch (desligado: só com clique).
    pub notch_hover: bool,
    /// Fundo do notch aberto.
    pub notch_bg: NotchBg,
    /// Ícone do Taskbeat na barra de menus (o menu também abre com o botão
    /// direito na ilha).
    pub menu_bar_icon: bool,

    /// O padrão "abrir ao iniciar a sessão" (ligado) já foi aplicado uma vez.
    /// Depois disso vale só a escolha do usuário.
    pub login_default_applied: bool,

    // ---- geral
    pub hide_on_fullscreen: bool,
    pub language: Language,

    /// Versão do esquema (para migrar configs antigas para os novos padrões).
    /// Ausente no arquivo = 0 (config antiga).
    #[serde(default)]
    pub version: u32,
}

const CONFIG_VERSION: u32 = 5;

/// Padrões. No Windows: a configuração que o usuário deixou "certinha"
/// (2026-11-24 e 2026-11-25), sem o que é só da máquina dele (posição do
/// arrasto, a lista de atalhos escolhida, marcas internas). No Mac ficam os de
/// antes (decisão dele: "Mac keeps old ones"); ver `with_mac_defaults`.
impl Default for Config {
    fn default() -> Self {
        let c = Self::windows_defaults();
        if cfg!(target_os = "macos") { c.with_mac_defaults() } else { c }
    }
}

impl Config {
    /// Os padrões do Mac: os de antes da config do usuário virar o padrão do
    /// Windows (f0746a0) em tudo que muda o Mac; o resto é igual.
    pub fn with_mac_defaults(self) -> Self {
        Self {
            playing_indicator: true,
            taskbar_shuffle_repeat: false,
            text_size: Size3::Normal,
            widget_tint: WidgetTint::Album,
            accent_source: AccentSource::Windows,
            show_visualizer: true,
            style: Style::Bars,
            bars: 6,
            ..self
        }
    }

    fn windows_defaults() -> Self {
        Self {
            show_media: true,
            always_show_widget: true,
            taskbar_controls: true,
            cover_shape: Shape::Rounded,
            show_artist: true,
            marquee: false,
            playing_indicator: false,
            taskbar_progress: false,
            widget_pill: true,
            widget_shape: WidgetShape::Rounded,
            widget_divider: false,
            taskbar_shuffle_repeat: true,
            taskbar_play: TaskbarPlay::Filled,
            widget_controls: ControlColor::White,
            widget_controls_rgb: "#5EB1FF".into(),
            widget_sides: SideColor::SameAsMiddle,
            widget_sides_alpha: 100,
            second_line: SecondLine::Artist,
            text_size: Size3::Small,
            cover_size: Size3::Normal,
            widget_tint: WidgetTint::Artwork,
            widget_opacity: 6,
            widget_intensity: 80,
            widget_glow: true,
            widget_border: true,
            widget_drag: true,
            icon_style: IconStyle::Filled,
            track_anim: true,
            icon_anim: true,
            title_width: 120,
            anchor: Anchor::BeforeTray,
            offset: 0,

            card_style: CardStyle::Artwork,
            card_layout: CardLayout::Horizontal,
            theme: ThemeMode::Dark,
            accent_source: AccentSource::Album,
            corners: Corners::Medium,
            play_shape: Shape::Rounded,
            card_controls: ControlColor::White,
            card_controls_rgb: "#5EB1FF".into(),
            card_sides: SideColor::SameAsMiddle,
            card_sides_alpha: 100,
            card_seek: true,
            show_source: false,
            show_volume: true,
            card_mixer: true,
            card_shuffle_repeat: ShuffleRepeat::Always,
            card_autoclose: 6,
            card_launchers: true,
            launcher_place: LauncherPlace::Taskbar,
            launcher_order: Vec::new(),
            launcher_hidden: Vec::new(),
            card_viz: true,
            // Opções de design de 2026-11-25: o padrão é o visual de antes delas.
            accent_rgb: "#5EB1FF".into(),
            card_shadow: CardShadow::Strong,
            card_grain: true,
            glass_blur: 50,
            glass_tint: 70,
            artwork_dim: 100,

            next_track_popup: true,
            popup_secs: 5,

            caps_lock_osd: true,
            num_lock_osd: true,
            osd_position: OsdPosition::Bottom,
            osd_offset_x: 0,
            osd_offset_y: 0,
            osd_shape: OsdShape::Rounded,
            osd_size: OsdSize::Compact,
            osd_theme: ThemeMode::Auto,
            osd_duration: 18,
            osd_show_off: true,
            osd_switch: true,
            osd_material: OsdMaterial::Acrylic,
            osd_opacity: 96,
            osd_blur: 20,
            osd_icon: OsdIcon::Lock,
            osd_anim: OsdAnim::Zoom,

            show_visualizer: false,
            style: Style::Mirror,
            palette: Palette::Album,
            width: 52,
            bars: 32,
            sensitivity_db: 0.0,
            only_when_media: false,
            viz_player_only: true,
            show_peaks: true,
            bar_shape: BarShape::Rounded,

            notch_hover: true,
            notch_bg: NotchBg::Album,
            menu_bar_icon: true,
            login_default_applied: false,

            hide_on_fullscreen: true,
            language: Language::Auto,
            version: CONFIG_VERSION,
        }
    }
}

impl Config {
    #[cfg(windows)]
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(base).join("Taskbeat").join("config.toml"))
    }

    #[cfg(not(windows))]
    pub fn path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join("Library").join("Application Support").join("Taskbeat").join("config.toml"))
    }

    /// Pasta do nome antigo do app (lida uma vez para migrar as preferências).
    #[cfg(windows)]
    fn legacy_path() -> Option<PathBuf> {
        let base = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(base).join("TaskbarVisualizer").join("config.toml"))
    }

    #[cfg(not(windows))]
    fn legacy_path() -> Option<PathBuf> {
        None
    }

    /// Já existe configuração salva (desta versão ou do nome antigo do app)?
    pub fn exists() -> bool {
        Self::path().is_some_and(|p| p.exists()) || Self::legacy_path().is_some_and(|p| p.exists())
    }

    /// Abrir ao iniciar a sessão vem **ligado** por padrão. Aplicado uma vez:
    /// na primeira execução (sem configuração salva) ou, com `always`, também
    /// para quem já usava (o Mac, que ainda não tinha esse padrão). Quem
    /// desligar depois continua com a escolha. Devolve `true` quando a
    /// configuração mudou (quem chama grava).
    pub fn apply_login_default(&mut self, first_run: bool, always: bool, enabled: impl FnOnce() -> bool, enable: impl FnOnce()) -> bool {
        if self.login_default_applied {
            return false;
        }
        self.login_default_applied = true;
        if (first_run || always) && !enabled() {
            enable();
        }
        true
    }

    pub fn load() -> Self {
        let Some(path) = Self::path() else { return Self::default() };
        let mut cfg: Config = std::fs::read_to_string(&path)
            .or_else(|_| std::fs::read_to_string(Self::legacy_path().unwrap_or_default()))
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();
        let migrate = cfg.version < CONFIG_VERSION;
        if migrate {
            if cfg.version < 2 {
                // v2: widget compacto; barras dinâmicas desligadas e pequenas.
                cfg.show_visualizer = false;
                cfg.width = 44;
                cfg.bars = 10;
                cfg.show_peaks = false;
            }
            // v3: botões só no card por padrão; novo visual.
            if cfg.version < 3 {
                cfg.taskbar_controls = false;
                cfg.title_width = cfg.title_width.clamp(120, 200);
            }
            // v4: opções novas do aviso de teclas e da origem vêm dos padrões (serde).
            if cfg.version < 5 {
                // v5: aviso embaixo, no meio, com os cantos do card.
                cfg.osd_position = OsdPosition::Bottom;
                cfg.osd_shape = OsdShape::Rounded;
            }
            cfg.version = CONFIG_VERSION;
        }
        if migrate || !path.exists() {
            cfg.save();
        }
        cfg.sanitize();
        cfg
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = toml::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    fn sanitize(&mut self) {
        for c in [&mut self.widget_controls_rgb, &mut self.card_controls_rgb, &mut self.accent_rgb] {
            if parse_hex(c).is_none() {
                *c = "#5EB1FF".into();
            }
        }
        self.width = self.width.clamp(20, 600);
        self.bars = self.bars.clamp(4, 128);
        self.sensitivity_db = self.sensitivity_db.clamp(-24.0, 24.0);
        self.offset = self.offset.clamp(-4000, 4000);
        self.title_width = self.title_width.clamp(60, 400);
        self.card_autoclose = self.card_autoclose.clamp(2, 60);
        self.popup_secs = self.popup_secs.clamp(2, 20);
        self.osd_duration = self.osd_duration.clamp(8, 50);
        self.osd_offset_x = self.osd_offset_x.clamp(-2000, 2000);
        self.osd_offset_y = self.osd_offset_y.clamp(-1200, 1200);
        self.osd_opacity = self.osd_opacity.clamp(20, 100);
        self.osd_blur = self.osd_blur.clamp(0, 100);
        self.widget_opacity = self.widget_opacity.clamp(0, 100);
        self.widget_intensity = self.widget_intensity.clamp(10, 100);
        self.widget_sides_alpha = self.widget_sides_alpha.clamp(20, 100);
        self.card_sides_alpha = self.card_sides_alpha.clamp(20, 100);
        self.glass_blur = self.glass_blur.clamp(0, 100);
        self.glass_tint = self.glass_tint.clamp(20, 100);
        self.artwork_dim = self.artwork_dim.clamp(50, 150);
    }
}

#[cfg(test)]
mod tests {
    use super::Config;
    use std::cell::Cell;

    fn run(cfg: &mut Config, first_run: bool, always: bool, on: bool) -> (bool, bool) {
        let enabled = Cell::new(false);
        let changed = cfg.apply_login_default(first_run, always, || on, || enabled.set(true));
        (changed, enabled.get())
    }

    /// Config gravada antes das opções novas carrega (com os padrões delas) e
    /// as novas vão e voltam do arquivo.
    #[test]
    fn config_antiga_carrega_e_opcoes_novas_sao_gravadas() {
        use super::{IconStyle, LauncherPlace, WidgetTint};
        let old = "show_media = true\nwidget_tint = \"album\"\nwidget_opacity = 6\nversion = 5\n";
        let c: Config = toml::from_str(old).unwrap();
        // Atalhos: por padrão na barra (o lugar que o usuário escolheu).
        assert_eq!((c.widget_opacity, c.widget_intensity, c.icon_style, c.launcher_place), (6, 80, IconStyle::Filled, LauncherPlace::Taskbar));
        assert!(c.launcher_order.is_empty() && c.card_viz && c.widget_drag);
        let c2 = Config {
            launcher_order: vec!["Brave._crx_x".into(), "Spotify.exe".into()],
            launcher_hidden: vec!["VideoLAN.VLC".into()],
            widget_tint: WidgetTint::Gradient,
            launcher_place: LauncherPlace::Both,
            ..Config::default()
        };
        let text = toml::to_string_pretty(&c2).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), c2);
    }

    /// As opções de design de 2026-11-25 vêm com o visual de antes delas,
    /// também para quem já tinha config (nada muda sem o usuário escolher).
    #[test]
    fn opcoes_de_design_vem_com_o_visual_de_antes() {
        use super::{BarShape, CardShadow};
        let c: Config = toml::from_str("card_style = \"glass\"\nversion = 5\n").unwrap();
        assert_eq!((c.card_shadow, c.card_grain, c.glass_blur, c.glass_tint, c.artwork_dim, c.bar_shape), (CardShadow::Strong, true, 50, 70, 100, BarShape::Rounded));
        assert_eq!(c.accent_rgb, "#5EB1FF");
        // 50% de desfoque = os 30 px de sempre.
        assert_eq!(crate::ui::glass_blur_px(&c, 1.0), 30.0);
        let mut c = Config { glass_blur: 500, glass_tint: 1, artwork_dim: 999, accent_rgb: "zz".into(), ..c };
        c.sanitize();
        assert_eq!((c.glass_blur, c.glass_tint, c.artwork_dim, c.accent_rgb.as_str()), (100, 20, 150, "#5EB1FF"));
    }

    /// Os controles vêm brancos (também para quem já tinha config). Cores em
    /// hex vão e voltam.
    #[test]
    fn cor_dos_controles() {
        use super::{ControlColor, hex_of, parse_hex, rgb_of};
        let c: Config = toml::from_str("widget_tint = \"artwork\"\nversion = 5\n").unwrap();
        assert_eq!((c.widget_controls, c.card_controls), (ControlColor::White, ControlColor::White));
        assert_eq!(parse_hex("#fff"), Some([1.0; 3]));
        assert_eq!(parse_hex("12"), None);
        assert_eq!(rgb_of("#5EB1FF"), 0x5EB1FF);
        assert_eq!(hex_of(0x5eb1ff), "#5EB1FF");
    }

    /// Padrões de antes (f0746a0, `toml::to_string(&Config::default())`),
    /// com que o Mac continua (decisão do usuário: "Mac keeps old ones").
    const DEFAULTS_F0746A0: &str = r#"
show_media = true
always_show_widget = true
taskbar_controls = true
cover_shape = "rounded"
show_artist = true
marquee = false
playing_indicator = true
taskbar_progress = false
widget_pill = true
widget_shape = "rounded"
widget_divider = false
taskbar_shuffle_repeat = false
taskbar_play = "filled"
second_line = "artist"
text_size = "normal"
cover_size = "normal"
widget_tint = "album"
widget_opacity = 6
widget_border = true
track_anim = true
icon_anim = true
title_width = 120
anchor = "before_tray"
offset = 0
card_style = "artwork"
card_layout = "horizontal"
theme = "dark"
accent_source = "windows"
corners = "medium"
play_shape = "rounded"
card_seek = true
show_source = false
show_volume = true
card_shuffle_repeat = "always"
card_autoclose = 6
card_launchers = true
next_track_popup = true
popup_secs = 5
caps_lock_osd = true
num_lock_osd = true
osd_position = "bottom"
osd_offset_x = 0
osd_offset_y = 0
osd_shape = "rounded"
osd_size = "compact"
osd_theme = "auto"
osd_duration = 18
osd_show_off = true
osd_switch = true
osd_material = "acrylic"
osd_opacity = 96
osd_blur = 20
osd_icon = "lock"
osd_anim = "zoom"
show_visualizer = true
style = "bars"
palette = "album"
width = 52
bars = 6
sensitivity_db = 0.0
only_when_media = false
viz_player_only = true
show_peaks = true
notch_hover = true
notch_bg = "album"
menu_bar_icon = true
login_default_applied = false
hide_on_fullscreen = true
language = "auto"
version = 5
"#;

    /// No Mac os padrões são os de f0746a0 em todo campo que já existia; no
    /// Windows só mudam os oito que vieram da config do usuário.
    #[test]
    fn padroes_do_mac_sao_os_de_antes() {
        let old: toml::Table = toml::from_str(DEFAULTS_F0746A0).unwrap();
        let table = |c: &Config| -> toml::Table { toml::from_str(&toml::to_string(c).unwrap()).unwrap() };
        let mac = table(&Config::windows_defaults().with_mac_defaults());
        for (k, v) in &old {
            assert_eq!(mac.get(k), Some(v), "padrão do Mac diferente do de antes em `{k}`");
        }
        let win = table(&Config::windows_defaults());
        let mut changed: Vec<&str> = old.iter().filter(|(k, v)| win.get(*k) != Some(*v)).map(|(k, _)| k.as_str()).collect();
        changed.sort();
        assert_eq!(changed, ["accent_source", "bars", "playing_indicator", "show_visualizer", "style", "taskbar_shuffle_repeat", "text_size", "widget_tint"]);
        let here = if cfg!(target_os = "macos") { mac } else { win };
        assert_eq!(table(&Config::default()), here);
    }

    /// Botões laterais: na cor do botão do meio e tão fortes quanto ele (também
    /// para quem já tinha config); a intensidade fica entre 20 e 100%.
    #[test]
    fn botoes_laterais() {
        use super::SideColor;
        let mut c: Config = toml::from_str("widget_controls = \"white\"\ncard_sides_alpha = 5\nversion = 5\n").unwrap();
        assert_eq!((c.widget_sides, c.card_sides, c.widget_sides_alpha), (SideColor::SameAsMiddle, SideColor::SameAsMiddle, 100));
        c.sanitize();
        assert_eq!(c.card_sides_alpha, 20);
        let c: Config = toml::from_str("widget_sides = \"same_as_middle\"\ncard_sides = \"album\"\n").unwrap();
        assert_eq!((c.widget_sides, c.card_sides), (SideColor::SameAsMiddle, SideColor::Album));
    }

    #[test]
    fn iniciar_com_a_sessao_vem_ligado() {
        // Primeira execução: liga.
        assert_eq!(run(&mut Config::default(), true, false, false), (true, true));
        // Já estava ligado: só marca.
        assert_eq!(run(&mut Config::default(), true, false, true), (true, false));
        // Windows, quem já usava (pode ter desligado no instalador): respeita.
        assert_eq!(run(&mut Config::default(), false, false, false), (true, false));
        // Mac, quem já usava o beta: liga uma vez.
        assert_eq!(run(&mut Config::default(), false, true, false), (true, true));
        // Depois de aplicado, vale a escolha do usuário.
        let mut c = Config::default();
        run(&mut c, true, true, false);
        assert_eq!(run(&mut c, true, true, false), (false, false));
    }
}
