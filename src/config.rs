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
    /// O que aparece embaixo do título (com "Mostrar artista" ligado).
    pub second_line: SecondLine,
    /// Tamanho do título e da segunda linha.
    pub text_size: Size3,
    /// Tamanho da capa na barra.
    pub cover_size: Size3,
    /// Cor do fundo do widget.
    pub widget_tint: WidgetTint,
    /// Opacidade do fundo do widget (%).
    pub widget_opacity: i32,
    /// Contorno fino em volta do fundo.
    pub widget_border: bool,
    /// Anima a troca de música no widget (texto desliza, capa esmaece).
    pub track_anim: bool,
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
    pub card_seek: bool,
    /// Mostra de qual app vem a música (nome no card, ícone sobre a capa).
    pub show_source: bool,
    /// Volume no card expandido (slider + mudo).
    pub show_volume: bool,
    /// Quando mostrar aleatório e repetir no card.
    pub card_shuffle_repeat: ShuffleRepeat,
    /// Segundos sem o mouse em cima até o card fechar sozinho.
    pub card_autoclose: i32,

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
    pub show_peaks: bool,

    // ---- notch (Mac)
    /// Abre o card ao passar o mouse no notch (desligado: só com clique).
    pub notch_hover: bool,
    /// Fundo do notch aberto.
    pub notch_bg: NotchBg,
    /// Ícone do Taskbeat na barra de menus (o menu também abre com o botão
    /// direito na ilha).
    pub menu_bar_icon: bool,

    // ---- geral
    pub hide_on_fullscreen: bool,
    pub language: Language,

    /// Versão do esquema (para migrar configs antigas para os novos padrões).
    /// Ausente no arquivo = 0 (config antiga).
    #[serde(default)]
    pub version: u32,
}

const CONFIG_VERSION: u32 = 5;

impl Default for Config {
    fn default() -> Self {
        Self {
            show_media: true,
            always_show_widget: true,
            taskbar_controls: true,
            cover_shape: Shape::Rounded,
            show_artist: true,
            marquee: false,
            playing_indicator: true,
            taskbar_progress: false,
            widget_pill: true,
            widget_shape: WidgetShape::Rounded,
            widget_divider: false,
            taskbar_shuffle_repeat: false,
            taskbar_play: TaskbarPlay::Filled,
            second_line: SecondLine::Artist,
            text_size: Size3::Normal,
            cover_size: Size3::Normal,
            widget_tint: WidgetTint::Album,
            widget_opacity: 6,
            widget_border: true,
            track_anim: true,
            title_width: 120,
            anchor: Anchor::BeforeTray,
            offset: 0,

            card_style: CardStyle::Artwork,
            card_layout: CardLayout::Horizontal,
            theme: ThemeMode::Dark,
            accent_source: AccentSource::Windows,
            corners: Corners::Medium,
            play_shape: Shape::Rounded,
            card_seek: true,
            show_source: false,
            show_volume: true,
            card_shuffle_repeat: ShuffleRepeat::Always,
            card_autoclose: 6,

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

            show_visualizer: true,
            style: Style::Bars,
            palette: Palette::Album,
            width: 52,
            bars: 6,
            sensitivity_db: 0.0,
            only_when_media: false,
            show_peaks: true,

            notch_hover: true,
            notch_bg: NotchBg::Album,
            menu_bar_icon: true,

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
        self.widget_opacity = self.widget_opacity.clamp(0, 60);
    }
}
