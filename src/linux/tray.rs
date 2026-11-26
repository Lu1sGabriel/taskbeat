//! Ícone na bandeja (StatusNotifierItem) com o menu do Taskbeat (dbusmenu),
//! no mesmo D-Bus que garante uma instância só.
//!
//! Aparece no KDE, XFCE, Cinnamon, MATE, waybar (Sway/Hyprland) e no Ubuntu
//! (a extensão AppIndicator vem ligada). No GNOME puro (Fedora) não há
//! bandeja: abrir o app de novo pelo menu de aplicativos abre as
//! configurações.
//!
//! - Clique: abre/fecha o card da ilha; meio: tocar/pausar; rolar: volume.
//! - Menu: o que está tocando, tocar/pausar, próxima, anterior, abrir ao
//!   passar o mouse, barras, configurações, abrir ao iniciar a sessão, sair.

use super::{App, audio, gfx, system};
use crate::i18n::tr;
use crate::shared::{MediaCmd, Shared};
use dbus::arg::{PropMap, RefArg, Variant};
use dbus::blocking::Connection;
use dbus::channel::{MatchingReceiver, Sender};
use dbus::message::MatchRule;
use dbus::{Message, Path};
use dbus_crossroads::{Crossroads, IfaceBuilder};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender as Tx;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const BUS_NAME: &str = "io.github.lu1sgabriel.Taskbeat";
const APP_PATH: &str = "/io/github/lu1sgabriel/Taskbeat";
const SNI_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

/// Revisão do menu (muda quando os textos ou as marcas mudam).
static REVISION: AtomicU32 = AtomicU32::new(1);

type Var = Variant<Box<dyn RefArg>>;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Item {
    Now = 1,
    Play = 3,
    Next = 4,
    Prev = 5,
    Hover = 7,
    Viz = 8,
    Settings = 10,
    Autostart = 11,
    Quit = 13,
}

/// Itens na ordem do menu (`None` = separador, com o id dado).
const LAYOUT: &[(i32, Option<Item>)] = &[
    (1, Some(Item::Now)),
    (2, None),
    (3, Some(Item::Play)),
    (4, Some(Item::Next)),
    (5, Some(Item::Prev)),
    (6, None),
    (7, Some(Item::Hover)),
    (8, Some(Item::Viz)),
    (9, None),
    (10, Some(Item::Settings)),
    (11, Some(Item::Autostart)),
    (12, None),
    (13, Some(Item::Quit)),
];

#[derive(Clone)]
struct Ctx {
    shared: Arc<Shared>,
    tx: Arc<Mutex<Tx<App>>>,
}

impl Ctx {
    fn post(&self, ev: App) {
        let _ = self.tx.lock().unwrap().send(ev);
        system::wake();
    }
}

/// Resultado da tentativa de ser a única instância.
pub enum Instance {
    /// Esta é a primeira: a conexão guarda o nome.
    First(Connection),
    /// Já havia uma aberta (foi avisada para mostrar as configurações).
    Other,
    /// Sem D-Bus da sessão: segue sem bandeja nem instância única.
    NoBus,
}

pub fn claim() -> Instance {
    let Ok(conn) = Connection::new_session() else { return Instance::NoBus };
    match conn.request_name(BUS_NAME, false, false, true) {
        Ok(dbus::blocking::stdintf::org_freedesktop_dbus::RequestNameReply::PrimaryOwner) => Instance::First(conn),
        Ok(_) => {
            let p = conn.with_proxy(BUS_NAME, APP_PATH, Duration::from_millis(800));
            let _: Result<(), _> = p.method_call(BUS_NAME, "Activate", ());
            Instance::Other
        }
        Err(_) => Instance::NoBus,
    }
}

/// Abre as configurações: por enquanto o arquivo `config.toml` no editor
/// (o app relê ao salvar).
pub fn open_settings(shared: &Shared) {
    let Some(path) = crate::config::Config::path() else { return };
    if !path.exists() {
        shared.config().save();
    }
    system::open_path(&path);
}

fn icon_pixmaps() -> Vec<(i32, i32, Vec<u8>)> {
    let png = include_bytes!("../../assets/taskbeat-256.png");
    let mut out = Vec::new();
    for size in [22usize, 32, 48, 64] {
        let Some(img) = gfx::decode_square(png, size) else { continue };
        let mut bytes = Vec::with_capacity(size * size * 4);
        for &p in &img.px {
            let a = (p >> 24) & 0xff;
            let un = |c: u32| if a == 0 { 0 } else { ((c * 255 + a / 2) / a).min(255) as u8 };
            // ARGB em ordem de rede (big-endian).
            bytes.extend_from_slice(&[a as u8, un((p >> 16) & 0xff), un((p >> 8) & 0xff), un(p & 0xff)]);
        }
        out.push((size as i32, size as i32, bytes));
    }
    out
}

fn now_playing(shared: &Shared) -> String {
    let m = shared.media.lock().unwrap();
    if !m.present() {
        return tr("Nada tocando").to_string();
    }
    let t = if m.title.is_empty() { m.app.clone() } else { m.title.clone() };
    let s = if m.artist.is_empty() { t } else { format!("{t} — {}", m.artist) };
    // Menus cortam mal textos longos.
    if s.chars().count() > 60 { format!("{}…", s.chars().take(59).collect::<String>().trim_end()) } else { s }
}

fn props_of(ctx: &Ctx, id: i32) -> PropMap {
    let mut p: PropMap = HashMap::new();
    let mut put = |k: &str, v: Box<dyn RefArg>| {
        p.insert(k.to_string(), Variant(v));
    };
    let item = LAYOUT.iter().find(|(i, _)| *i == id).and_then(|(_, it)| *it);
    let Some(item) = item else {
        if id == 0 {
            put("children-display", Box::new("submenu".to_string()));
        } else {
            put("type", Box::new("separator".to_string()));
        }
        return p;
    };
    let cfg = ctx.shared.config();
    let label = match item {
        Item::Now => now_playing(&ctx.shared),
        Item::Play => tr("Tocar / pausar").into(),
        Item::Next => tr("Próxima").into(),
        Item::Prev => tr("Anterior").into(),
        Item::Hover => tr("Abrir ao passar o mouse").into(),
        Item::Viz => tr("Barras de áudio dinâmicas").into(),
        Item::Settings => tr("Configurações…").into(),
        Item::Autostart => tr("Abrir ao iniciar a sessão").into(),
        Item::Quit => tr("Sair do Taskbeat").into(),
    };
    // "_" vira atalho de teclado nos menus: escapa.
    put("label", Box::new(label.replace('_', "__")));
    if item == Item::Now {
        put("enabled", Box::new(false));
    }
    let check = match item {
        Item::Hover => Some(cfg.notch_hover),
        Item::Viz => Some(cfg.show_visualizer),
        Item::Autostart => Some(system::autostart_enabled()),
        _ => None,
    };
    if let Some(on) = check {
        put("toggle-type", Box::new("checkmark".to_string()));
        put("toggle-state", Box::new(on as i32));
    }
    p
}

fn clicked(ctx: &Ctx, id: i32) {
    let Some(item) = LAYOUT.iter().find(|(i, _)| *i == id).and_then(|(_, it)| *it) else { return };
    let sh = &ctx.shared;
    match item {
        Item::Play => sh.send_media(MediaCmd::PlayPause),
        Item::Next => sh.send_media(MediaCmd::Next),
        Item::Prev => sh.send_media(MediaCmd::Previous),
        Item::Hover => sh.update_config(|c| c.notch_hover = !c.notch_hover),
        Item::Viz => sh.update_config(|c| c.show_visualizer = !c.show_visualizer),
        Item::Settings => open_settings(sh),
        Item::Autostart => system::set_autostart(!system::autostart_enabled()),
        Item::Quit => ctx.post(App::Quit),
        Item::Now => {}
    }
    REVISION.fetch_add(1, Ordering::Relaxed);
    system::wake();
}

fn layout(ctx: &Ctx, parent: i32) -> (i32, PropMap, Vec<Var>) {
    let children = if parent == 0 {
        LAYOUT.iter().map(|(id, _)| Variant(Box::new((*id, props_of(ctx, *id), Vec::<Var>::new())) as Box<dyn RefArg>)).collect()
    } else {
        Vec::new()
    };
    (parent, props_of(ctx, parent), children)
}

pub fn spawn(conn: Connection, shared: Arc<Shared>, tx: Tx<App>) {
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || {
            system::background_thread();
            run(conn, Ctx { shared, tx: Arc::new(Mutex::new(tx)) });
        })
        .expect("thread da bandeja");
}

fn register(conn: &Connection, name: &str) -> bool {
    let p = conn.with_proxy(WATCHER, "/StatusNotifierWatcher", Duration::from_millis(1500));
    let r: Result<(), _> = p.method_call(WATCHER, "RegisterStatusNotifierItem", (name,));
    match &r {
        Ok(()) => system::log::info("bandeja: ícone registrado"),
        Err(e) => system::log::info(&format!("bandeja: sem StatusNotifierWatcher ({e})")),
    }
    r.is_ok()
}

fn run(conn: Connection, ctx: Ctx) {
    let mut cr = Crossroads::new();

    // Outra instância pediu para aparecer: abre as configurações.
    let app = cr.register(BUS_NAME, |b: &mut IfaceBuilder<Ctx>| {
        b.method("Activate", (), (), |_, ctx, ()| {
            open_settings(&ctx.shared);
            Ok(())
        });
    });
    cr.insert(APP_PATH, &[app], ctx.clone());

    let pixmaps = icon_pixmaps();
    let sni = cr.register("org.kde.StatusNotifierItem", move |b: &mut IfaceBuilder<Ctx>| {
        b.property("Category").get(|_, _| Ok("ApplicationStatus".to_string()));
        b.property("Id").get(|_, _| Ok("taskbeat".to_string()));
        b.property("Title").get(|_, _| Ok("Taskbeat".to_string()));
        b.property("Status").get(|_, _| Ok("Active".to_string()));
        b.property("WindowId").get(|_, _| Ok(0i32));
        b.property("IconName").get(|_, _| Ok(String::new()));
        let px = pixmaps.clone();
        b.property("IconPixmap").get(move |_, _| Ok(px.clone()));
        b.property("OverlayIconName").get(|_, _| Ok(String::new()));
        b.property("AttentionIconName").get(|_, _| Ok(String::new()));
        b.property("ItemIsMenu").get(|_, _| Ok(false));
        b.property("Menu").get(|_, _| Ok(Path::from(MENU_PATH)));
        b.property("ToolTip").get(|_, ctx: &mut Ctx| Ok((String::new(), Vec::<(i32, i32, Vec<u8>)>::new(), "Taskbeat".to_string(), now_playing(&ctx.shared))));
        b.method("Activate", ("x", "y"), (), |_, ctx, (_x, _y): (i32, i32)| {
            ctx.post(App::ToggleCard);
            Ok(())
        });
        b.method("SecondaryActivate", ("x", "y"), (), |_, ctx, (_x, _y): (i32, i32)| {
            ctx.shared.send_media(MediaCmd::PlayPause);
            Ok(())
        });
        b.method("ContextMenu", ("x", "y"), (), |_, _, (_x, _y): (i32, i32)| Ok(()));
        b.method("Scroll", ("delta", "orientation"), (), |_, _, (delta, orient): (i32, String)| {
            if orient.eq_ignore_ascii_case("vertical") {
                if let Some((v, _)) = audio::volume() {
                    // 120 = um degrau da roda.
                    audio::set_volume(v + delta as f32 / 120.0 * 0.04);
                }
            }
            Ok(())
        });
        b.signal::<(), _>("NewToolTip", ());
        b.signal::<(), _>("NewIcon", ());
        b.signal::<(String,), _>("NewStatus", ("status",));
    });
    cr.insert(SNI_PATH, &[sni], ctx.clone());

    let menu = cr.register("com.canonical.dbusmenu", |b: &mut IfaceBuilder<Ctx>| {
        b.property("Version").get(|_, _| Ok(3u32));
        b.property("TextDirection").get(|_, _| Ok("ltr".to_string()));
        b.property("Status").get(|_, _| Ok("normal".to_string()));
        b.property("IconThemePath").get(|_, _| Ok(Vec::<String>::new()));
        b.method("GetLayout", ("parentId", "recursionDepth", "propertyNames"), ("revision", "layout"), |_, ctx, (parent, _depth, _names): (i32, i32, Vec<String>)| {
            Ok((REVISION.load(Ordering::Relaxed), layout(ctx, parent)))
        });
        b.method("GetGroupProperties", ("ids", "propertyNames"), ("properties",), |_, ctx, (ids, _names): (Vec<i32>, Vec<String>)| {
            let ids = if ids.is_empty() { LAYOUT.iter().map(|(i, _)| *i).collect() } else { ids };
            Ok((ids.into_iter().map(|id| (id, props_of(ctx, id))).collect::<Vec<_>>(),))
        });
        b.method("GetProperty", ("id", "name"), ("value",), |_, ctx, (id, name): (i32, String)| {
            let mut p = props_of(ctx, id);
            Ok((p.remove(&name).unwrap_or_else(|| Variant(Box::new(String::new()))),))
        });
        b.method("Event", ("id", "eventId", "data", "timestamp"), (), |_, ctx, (id, ev, _data, _ts): (i32, String, Var, u32)| {
            if ev == "clicked" {
                clicked(ctx, id);
            }
            Ok(())
        });
        b.method("EventGroup", ("events",), ("idErrors",), |_, ctx, (events,): (Vec<(i32, String, Var, u32)>,)| {
            for (id, ev, _, _) in events {
                if ev == "clicked" {
                    clicked(ctx, id);
                }
            }
            Ok((Vec::<i32>::new(),))
        });
        b.method("AboutToShow", ("id",), ("needUpdate",), |_, _, (_id,): (i32,)| {
            // Os textos são lidos na hora em GetLayout: sempre atuais.
            Ok((false,))
        });
        b.method("AboutToShowGroup", ("ids",), ("updatesNeeded", "idErrors"), |_, _, (_ids,): (Vec<i32>,)| Ok((Vec::<i32>::new(), Vec::<i32>::new())));
        b.signal::<(u32, i32), _>("LayoutUpdated", ("revision", "parent"));
    });
    cr.insert(MENU_PATH, &[menu], ctx.clone());

    let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
    let _ = conn.request_name(name.as_str(), false, false, true);

    // O "vigia" da bandeja pode aparecer depois (painel reiniciou).
    let watcher_up = Arc::new(AtomicBool::new(false));
    {
        let up = watcher_up.clone();
        let rule = MatchRule::new_signal("org.freedesktop.DBus", "NameOwnerChanged").with_sender("org.freedesktop.DBus");
        let _ = conn.add_match(rule, move |(n, _old, new): (String, String, String), _, _| {
            if n == WATCHER && !new.is_empty() {
                up.store(true, Ordering::Relaxed);
            }
            true
        });
    }

    conn.start_receive(
        MatchRule::new_method_call(),
        Box::new(move |msg, conn| {
            let _ = cr.handle_message(msg, conn);
            true
        }),
    );
    register(&conn, &name);

    let mut seen = (u64::MAX, u64::MAX, REVISION.load(Ordering::Relaxed));
    while ctx.shared.is_running() {
        let _ = conn.process(Duration::from_millis(2000));
        if watcher_up.swap(false, Ordering::Relaxed) {
            register(&conn, &name);
        }
        let mv = ctx.shared.media_version.load(Ordering::Relaxed);
        let cv = ctx.shared.config_version.load(Ordering::Relaxed);
        if (mv, cv) != (seen.0, seen.1) {
            REVISION.fetch_add(1, Ordering::Relaxed);
        }
        let rev = REVISION.load(Ordering::Relaxed);
        if rev != seen.2 {
            let _ = conn.send(Message::new_signal(MENU_PATH, "com.canonical.dbusmenu", "LayoutUpdated").unwrap().append2(rev, 0i32));
            if mv != seen.0 {
                let _ = conn.send(Message::new_signal(SNI_PATH, "org.kde.StatusNotifierItem", "NewToolTip").unwrap());
            }
        }
        seen = (mv, cv, rev);
    }
}
