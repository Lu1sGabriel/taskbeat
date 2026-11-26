//! Página "Letra" das configurações: a sobreposição de letra (posição,
//! aparência, atalhos, busca no LRCLIB) e a prévia dela.
//!
//! Fica num módulo à parte (filho de `settings.rs`) para mexer pouco na
//! janela de configurações: lá só entram a página, a variante `Key::Lyrics`
//! e uma linha em cada `match`.

use super::*;
use crate::lyrics_view as view;
use crate::lyrics::config::{Align as LAlign, Background, Hotkey, Layout, Position, ZOOM_MAX, ZOOM_MIN};

/// As opções da página (dentro de `Key::Lyrics`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum LK {
    Show,
    Content,
    Fullscreen,
    Place,
    Monitor,
    Zoom,
    Locked,
    ClickThrough,
    Shortcut,
    Bg,
    Opacity,
    TextSize,
    TextAlign,
    ShowTime,
    Volume,
    Mixer,
    AnimatedBg,
    HideIdle,
    HideFullscreen,
    Online,
    Offset,
    Folder,
    ClearCache,
    Privacy,
}

const LAYOUTS: &[&str] = &["Compacto", "Só a letra", "Player e letra", "Só o player", "Legenda"];
const LAYOUT_V: &[Layout] = &[Layout::Compact, Layout::Lyrics, Layout::PlayerLyrics, Layout::Player, Layout::Caption];
const POSITIONS: &[&str] = &[
    "Canto superior esquerdo",
    "Topo, no meio",
    "Canto superior direito",
    "Esquerda, no meio",
    "Centro",
    "Direita, no meio",
    "Canto inferior esquerdo",
    "Embaixo, no meio",
    "Canto inferior direito",
    "Livre",
];
const POSITION_V: &[Position] = &[
    Position::TopLeft,
    Position::Top,
    Position::TopRight,
    Position::Left,
    Position::Center,
    Position::Right,
    Position::BottomLeft,
    Position::Bottom,
    Position::BottomRight,
    Position::Free,
];
const MONITORS: &[&str] = &["Principal", "Segundo monitor", "Terceiro monitor", "Quarto monitor"];
const HOTKEYS: &[&str] = &["Win+Alt+L", "Ctrl+Alt+L", "Ctrl+Shift+L", "Desligado"];
const HOTKEY_V: &[Hotkey] = &[Hotkey::WinAltL, Hotkey::CtrlAltL, Hotkey::CtrlShiftL, Hotkey::Off];
const BACKGROUNDS: &[&str] = &["Capa desfocada", "Cor do álbum", "Escuro", "Sem fundo"];
const BACKGROUND_V: &[Background] = &[Background::Artwork, Background::AlbumColor, Background::Dark, Background::None];
const ALIGNS: &[&str] = &["Automático", "Esquerda", "Centro"];
const ALIGN_V: &[LAlign] = &[LAlign::Auto, LAlign::Left, LAlign::Center];

use LK::*;

pub(super) const PAGE: Page = Page {
    name: "Letras",
    icon: "\u{E7F0}",
    subtitle: "A letra sincronizada e o player numa janela sempre por cima",
    preview: Preview::Lyrics,
    items: &[
        Section("Sobreposição"),
        Row { icon: "\u{E7F0}", title: "Mostrar a letra na tela", desc: "Janela fixa, sempre por cima, que dá para arrastar; o atalho mostra e esconde", key: Key::Lyrics(Show), kind: Kind::Toggle },
        Row { icon: "\u{E8A9}", title: "Conteúdo", desc: "Compacto (mini player e letra), só a letra, player e letra, só o player ou legenda", key: Key::Lyrics(Content), kind: Kind::Choice(LAYOUTS) },
        Row { icon: "\u{E740}", title: "Tela cheia", desc: "A letra grande no monitor da janela, com o fundo da capa em movimento; Esc volta", key: Key::Lyrics(Fullscreen), kind: Kind::Button("Abrir") },
        Row { icon: "\u{E765}", title: "Atalho", desc: "Mostra e esconde; com Shift, trava e destrava a posição", key: Key::Lyrics(Shortcut), kind: Kind::Choice(HOTKEYS) },
        Section("Posição"),
        Row { icon: "\u{E7C4}", title: "Onde fica", desc: "Também arrastando a janela: perto de um dos lugares prontos ela gruda nele", key: Key::Lyrics(Place), kind: Kind::Choice(POSITIONS) },
        Row { icon: "\u{E7F4}", title: "Monitor", desc: "Em qual monitor a janela fica (arrastá-la para outro também muda)", key: Key::Lyrics(Monitor), kind: Kind::Choice(MONITORS) },
        Row { icon: "\u{E740}", title: "Tamanho da janela", desc: "Também com Ctrl + roda do mouse em cima dela", key: Key::Lyrics(Zoom), kind: Kind::Slider { min: ZOOM_MIN, max: ZOOM_MAX, step: 10, unit: "%" } },
        Row { icon: "\u{E72E}", title: "Travar a posição", desc: "Não arrasta mais; os botões continuam funcionando", key: Key::Lyrics(Locked), kind: Kind::Toggle },
        Row { icon: "\u{E7FC}", title: "Travada, o clique passa direto", desc: "Para jogos: a janela não pega o mouse e o clique vai para o que está atrás; destrave pelo atalho ou pelo menu da bandeja", key: Key::Lyrics(ClickThrough), kind: Kind::Toggle },
        Section("Aparência"),
        Row { icon: "\u{E771}", title: "Fundo", desc: "Capa desfocada, cor do álbum, escuro ou sem fundo (só o texto, com sombra)", key: Key::Lyrics(Bg), kind: Kind::Segmented(BACKGROUNDS) },
        Row { icon: "\u{E706}", title: "Opacidade do fundo", desc: "Quanto o fundo cobre o que está atrás", key: Key::Lyrics(Opacity), kind: Kind::Slider { min: 10, max: 100, step: 5, unit: "%" } },
        Row { icon: "\u{E8E9}", title: "Tamanho da letra", desc: "A linha atual e as outras", key: Key::Lyrics(TextSize), kind: Kind::Segmented(SIZES) },
        Row { icon: "\u{E8E4}", title: "Alinhamento", desc: "Automático: no meio só com a letra e na legenda; à esquerda com o player", key: Key::Lyrics(TextAlign), kind: Kind::Segmented(ALIGNS) },
        Row { icon: "\u{E916}", title: "Mostrar o tempo", desc: "Decorrido e restante embaixo da barra de progresso (a barra sempre dá para clicar e arrastar)", key: Key::Lyrics(ShowTime), kind: Kind::Toggle },
        Row { icon: "\u{E767}", title: "Volume", desc: "Slider de volume e mudo no player", key: Key::Lyrics(Volume), kind: Kind::Toggle },
        Row { icon: "\u{E9F5}", title: "Mixer de volume", desc: "Setinha embaixo do player que abre o volume de cada app", key: Key::Lyrics(Mixer), kind: Kind::Toggle },
        Row { icon: "\u{E9A9}", title: "Fundo animado na tela cheia", desc: "As cores da capa girando devagar enquanto a música toca; pausada, fica parado", key: Key::Lyrics(AnimatedBg), kind: Kind::Toggle },
        Section("Quando aparece"),
        Row { icon: "\u{E8D6}", title: "Esconder sem música", desc: "Some quando nenhum player está aberto; pausada, continua", key: Key::Lyrics(HideIdle), kind: Kind::Toggle },
        Row { icon: "\u{E740}", title: "Esconder com app em tela cheia", desc: "Some quando um jogo ou vídeo ocupa o monitor dela. Em jogos de tela cheia exclusiva o Windows não deixa nada por cima; em janela sem borda funciona", key: Key::Lyrics(HideFullscreen), kind: Kind::Toggle },
        Section("De onde vem a letra"),
        Row { icon: "\u{E774}", title: "Buscar letras na internet", desc: "Título, artista, álbum e duração da música vão para o LRCLIB (lrclib.net), um banco de letras aberto e gratuito. Desligado: só as letras já guardadas e a sua pasta", key: Key::Lyrics(Online), kind: Kind::Toggle },
        Row { icon: "\u{E916}", title: "Ajuste de tempo", desc: "Adianta (+) ou atrasa (−) a letra, se ela vier fora do tempo", key: Key::Lyrics(Offset), kind: Kind::Slider { min: -3000, max: 3000, step: 100, unit: "ms" } },
        Row { icon: "\u{E8B7}", title: "Suas letras", desc: "Arquivos \"Artista - Música.lrc\" (ou .txt) nesta pasta valem mais que a busca", key: Key::Lyrics(Folder), kind: Kind::Button("Abrir pasta") },
        Row { icon: "\u{E74D}", title: "Letras guardadas", desc: "Apaga as letras baixadas (voltam quando a música tocar de novo)", key: Key::Lyrics(ClearCache), kind: Kind::Button("Limpar") },
        Row { icon: "\u{E72E}", title: "Privacidade", desc: "Só título, artista, álbum e duração saem do PC, e só com a busca ligada e a janela à vista", key: Key::Lyrics(Privacy), kind: Kind::Info },
    ],
};

pub(super) fn get(c: &Config, k: LK) -> i32 {
    let l = &c.lyrics;
    match k {
        Show => l.show as i32,
        Content => idx(LAYOUT_V, l.layout),
        Place => idx(POSITION_V, l.position),
        Monitor => l.monitor.clamp(0, MONITORS.len() as i32 - 1),
        Zoom => l.zoom,
        Locked => l.locked as i32,
        ClickThrough => l.click_through as i32,
        Shortcut => idx(HOTKEY_V, l.hotkey),
        Bg => idx(BACKGROUND_V, l.background),
        Opacity => l.opacity,
        TextSize => idx(SIZE_V, l.size),
        TextAlign => idx(ALIGN_V, l.align),
        ShowTime => l.show_time as i32,
        Volume => l.volume as i32,
        Mixer => l.mixer as i32,
        AnimatedBg => l.animated_bg as i32,
        HideIdle => l.hide_idle as i32,
        HideFullscreen => l.hide_fullscreen as i32,
        Online => l.online as i32,
        Offset => l.offset_ms,
        Fullscreen | Folder | ClearCache | Privacy => 0,
    }
}

pub(super) fn set(c: &mut Config, k: LK, v: i32) {
    fn pick<T: Copy>(l: &[T], v: i32) -> T {
        l[(v.max(0) as usize).min(l.len() - 1)]
    }
    let l = &mut c.lyrics;
    match k {
        Show => l.show = v != 0,
        Content => l.layout = pick(LAYOUT_V, v),
        Place => l.position = pick(POSITION_V, v),
        Monitor => l.monitor = v.clamp(0, MONITORS.len() as i32 - 1),
        Zoom => l.zoom = v.clamp(ZOOM_MIN, ZOOM_MAX),
        Locked => l.locked = v != 0,
        ClickThrough => l.click_through = v != 0,
        Shortcut => l.hotkey = pick(HOTKEY_V, v),
        Bg => l.background = pick(BACKGROUND_V, v),
        Opacity => l.opacity = v.clamp(10, 100),
        TextSize => l.size = pick(SIZE_V, v),
        TextAlign => l.align = pick(ALIGN_V, v),
        ShowTime => l.show_time = v != 0,
        Volume => l.volume = v != 0,
        Mixer => l.mixer = v != 0,
        AnimatedBg => l.animated_bg = v != 0,
        HideIdle => l.hide_idle = v != 0,
        HideFullscreen => l.hide_fullscreen = v != 0,
        Online => l.online = v != 0,
        Offset => l.offset_ms = v.clamp(-5000, 5000),
        Fullscreen | Folder | ClearCache | Privacy => {}
    }
}

/// Opções que dependem de outra (aparecem apagadas, com o motivo na dica).
pub(super) fn requirement(c: &Config, k: LK) -> Option<&'static str> {
    let l = &c.lyrics;
    let player = matches!(l.layout, Layout::Player | Layout::PlayerLyrics);
    match k {
        ClickThrough if !l.locked => Some("Ligue \"Travar a posição\" para usar esta opção"),
        Opacity if l.background == Background::None => Some("Sem fundo não há o que deixar transparente"),
        Volume | Mixer if !player => Some("Só nos conteúdos com o player (\"Player e letra\" ou \"Só o player\")"),
        TextAlign if l.layout == Layout::Player => Some("\"Só o player\" não mostra a letra"),
        _ => None,
    }
}

/// O valor do slider tem sinal (+/−)?
pub(super) fn signed(k: LK) -> bool {
    k == Offset
}

/// Botões da página.
pub(super) fn press(shared: &Arc<Shared>, k: LK) {
    match k {
        Fullscreen => crate::lyrics_overlay::open_fullscreen(),
        Folder => {
            if let Some(dir) = crate::lyrics::cache::user_dir() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
        }
        ClearCache => {
            if let Some(dir) = crate::lyrics::cache::cache_dir() {
                let n = crate::lyrics::cache::Cache::new(dir).clear();
                crate::audio::log::info(&format!("letra: {n} letras guardadas apagadas"));
            }
            crate::lyrics_overlay::refetch();
        }
        _ => {}
    }
    let _ = shared;
}

/// Letra de exemplo da prévia (inventada).
const SAMPLE_LRC: &str = "[00:02.00]As luzes da cidade acendem devagar\n[00:04.40]E o rádio toca a nossa canção\n[00:06.80]Corre comigo até o sol nascer\n[00:09.20]Ninguém vai dormir esta noite\n[00:11.60]O céu inteiro cabe na janela\n[00:14.00]E a estrada some no retrovisor";

thread_local! {
    static SAMPLE: (crate::lyrics::Lyrics, Vec<crate::lyrics::Line>) = {
        let l = crate::lyrics::parse_lrc(SAMPLE_LRC);
        (crate::lyrics::Lyrics::Synced(l.clone()), l)
    };
}

/// Linha em foco da prévia no tempo `t`: uma linha nova a cada 2,4 s, com a troca animada.
fn preview_focus(t: f32, n: usize) -> f32 {
    let n = n.max(1) as f32;
    let step = 2.4;
    let k = (t / step).floor();
    let u = ((t - k * step) / 0.3).min(1.0);
    let base = (k % n) as f32;
    // Volta ao começo sem animar (a última linha para a primeira).
    if base == 0.0 { 0.0 } else { base - 1.0 + crate::ui::ease_out_cubic(u) }
}

fn preview_geometry(tr: Option<&mut TextRenderer>, cfg: &Config, s: f32, frame: &view::LyricFrame) -> view::Geo {
    let lc = &cfg.lyrics;
    let cc = view::card_config(cfg, lc);
    let s = s * lc.zoom as f32 / 100.0;
    let caption = match (lc.layout, tr) {
        (Layout::Caption, Some(tr)) => view::caption_content(tr, frame, lc, s),
        (Layout::Caption, None) => (380.0 * s, 58.0 * s),
        _ => (0.0, 0.0),
    };
    view::geometry(lc, &cc, s, caption, None)
}

/// Altura da prévia (px).
pub(super) fn preview_height(cfg: &Config, s: f32) -> f32 {
    SAMPLE.with(|(l, lines)| {
        let frame = view::LyricFrame { lyrics: l, lines, focus: 2.0, scroll: 0.0 };
        preview_geometry(None, cfg, s, &frame).h + 8.0 * s
    })
}

/// Retângulo da janela dentro da prévia (o mesmo que `draw_preview` usa).
pub(super) fn preview_rect(r: RectF, cfg: &Config, s: f32) -> RectF {
    SAMPLE.with(|(l, lines)| {
        let frame = view::LyricFrame { lyrics: l, lines, focus: 2.0, scroll: 0.0 };
        let g = preview_geometry(None, cfg, s, &frame);
        RectF::new(r.cx() - g.w / 2.0, r.y + 4.0 * s, g.w, g.h)
    })
}

/// A sobreposição na prévia, com a música de agora (ou a de exemplo) e a letra de exemplo.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_preview(cv: &mut Canvas, tr: &mut TextRenderer, r: RectF, cfg: &Config, colors: &Colors, media: &MediaInfo, pb: Playback, s: f32, t: f32) {
    SAMPLE.with(|(l, lines)| {
        let lines = crate::lyrics::with_gaps(lines);
        let n_text = lines.len();
        let frame = view::LyricFrame { lyrics: l, lines: &lines, focus: preview_focus(t, n_text), scroll: 0.0 };
        let lc = &cfg.lyrics;
        let cc = view::card_config(cfg, lc);
        let zs = s * lc.zoom as f32 / 100.0;
        tr.set_scale(zs);
        let g = preview_geometry(Some(&mut *tr), cfg, s, &frame);
        let mut m2 = media.clone();
        m2.dur = m2.dur.max(1.0);
        let empty_mixer = crate::shared::MixerState::default();
        let v = view::View {
            card_cfg: &cc,
            lc,
            colors,
            media: &m2,
            pb,
            it: Interact::default(),
            hover: None,
            tools: 0.0,
            bg_time: None,
            hires: None,
            change: None,
            mixer: lc.mixer.then_some(&empty_mixer),
            lyr_alpha: 1.0,
            tip: None,
            toast: None,
            frame,
            s: zs,
        };
        let mut sub = SubCanvas::new(r.cx() - g.w / 2.0, r.y + 4.0 * s, g.w, g.h);
        view::draw(&mut sub.canvas(), tr, &g, &v);
        sub.commit(cv);
        tr.set_scale(s);
    });
}
