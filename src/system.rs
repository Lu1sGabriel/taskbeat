//! Utilidades do sistema: tema/cor de destaque, inicialização com o Windows.

use crate::config::Palette;
use crate::render::{Colors, Rgb};
use windows::UI::ViewManagement::{UIColorType, UISettings};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::System::Memory::{SETPROCESSWORKINGSETSIZEEX_FLAGS, SetProcessWorkingSetSizeEx};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, SetThreadInformation, SetThreadPriority, THREAD_POWER_THROTTLING_CURRENT_VERSION,
    THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE, THREAD_PRIORITY_BELOW_NORMAL, ThreadPowerThrottling,
};
use windows::core::{PCWSTR, w};

/// Marca a thread atual como trabalho de segundo plano: EcoQoS (núcleos e
/// frequências mais econômicos no Windows 11) e prioridade abaixo do normal.
pub fn background_thread() {
    unsafe {
        let st = THREAD_POWER_THROTTLING_STATE {
            Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        };
        let _ = SetThreadInformation(
            GetCurrentThread(),
            ThreadPowerThrottling,
            &st as *const _ as _,
            std::mem::size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        );
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

/// Devolve ao Windows as páginas que não estão em uso (ex.: as que só serviram
/// para iniciar ou para a tela de configurações). Voltam sob demanda.
pub fn trim_memory() {
    unsafe {
        let _ = SetProcessWorkingSetSizeEx(GetCurrentProcess(), usize::MAX, usize::MAX, SETPROCESSWORKINGSETSIZEEX_FLAGS(0));
    }
}

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("Taskbeat");
const LEGACY_RUN_VALUE: PCWSTR = w!("TaskbarVisualizer");

fn reg_dword(key: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let r = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key, value, RRF_RT_REG_DWORD, None, Some(&mut data as *mut u32 as _), Some(&mut size))
    };
    (r == ERROR_SUCCESS).then_some(data)
}

/// Barra de tarefas clara? (Configurações > Personalização > Cores > "modo do Windows").
pub fn taskbar_is_light() -> bool {
    reg_dword(w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), w!("SystemUsesLightTheme"))
        .map(|v| v != 0)
        .unwrap_or(false)
}

fn ui_color(settings: &UISettings, kind: UIColorType) -> Option<Rgb> {
    let c = settings.GetColorValue(kind).ok()?;
    Some([c.R as f32 / 255.0, c.G as f32 / 255.0, c.B as f32 / 255.0])
}

pub fn colors(palette: Palette) -> Colors {
    let light = taskbar_is_light();
    let settings = UISettings::new().ok();
    let pick = |a: UIColorType, fallback: Rgb| settings.as_ref().and_then(|s| ui_color(s, a)).unwrap_or(fallback);
    let (accent, accent_light) = if light {
        let a = ensure_luma(pick(UIColorType::AccentDark2, [0.0, 0.25, 0.55]), false, 0.30);
        (a, ensure_luma(pick(UIColorType::Accent, [0.0, 0.47, 0.84]), false, 0.42))
    } else {
        let a = ensure_luma(pick(UIColorType::AccentLight1, [0.30, 0.65, 1.0]), true, 0.55);
        (a, ensure_luma(pick(UIColorType::AccentLight3, [0.75, 0.88, 1.0]), true, 0.82))
    };
    let (accent, accent_light) = if is_neutral(accent) {
        // Destaque do Windows cinza/neutro: usa a cor da marca para o app não ficar apagado.
        if light { (BRAND_DARK, BRAND) } else { (BRAND, BRAND_LIGHT) }
    } else {
        (accent, accent_light)
    };
    Colors { palette, accent, accent_light, light_taskbar: light, album: None }
}

/// Cores da marca Taskbeat (rosa do logo), usadas quando o destaque do Windows é neutro.
const BRAND: Rgb = [0.93, 0.32, 0.62];
const BRAND_LIGHT: Rgb = [0.98, 0.62, 0.80];
const BRAND_DARK: Rgb = [0.72, 0.13, 0.45];

fn is_neutral(c: Rgb) -> bool {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    max <= 0.0 || (max - min) / max < 0.18
}

/// Apps no modo escuro? (Configurações > Personalização > Cores > "modo do aplicativo").
pub fn apps_use_dark() -> bool {
    reg_dword(w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), w!("AppsUseLightTheme"))
        .map(|v| v == 0)
        .unwrap_or(true)
}

/// Cor de destaque como o WinUI usa: AccentLight2 no escuro, AccentDark1 no claro.
pub fn app_accent(dark: bool) -> Rgb {
    let settings = UISettings::new().ok();
    let kind = if dark { UIColorType::AccentLight2 } else { UIColorType::AccentDark1 };
    let c = settings
        .as_ref()
        .and_then(|s| ui_color(s, kind))
        .unwrap_or(if dark { [0.60, 0.80, 1.0] } else { [0.0, 0.37, 0.72] });
    if is_neutral(c) { if dark { BRAND_LIGHT } else { BRAND_DARK } } else { c }
}

fn luma(c: Rgb) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Garante contraste com a barra: se a cor de destaque for apagada (ex.: cinza
/// em barra cinza), clareia (barra escura) ou escurece (barra clara) até `target`.
fn ensure_luma(c: Rgb, brighten: bool, target: f32) -> Rgb {
    let l = luma(c);
    if brighten && l < target {
        let t = (target - l) / (1.0 - l).max(1e-3);
        [c[0] + (1.0 - c[0]) * t, c[1] + (1.0 - c[1]) * t, c[2] + (1.0 - c[2]) * t]
    } else if !brighten && l > target {
        let k = target / l.max(1e-3);
        [c[0] * k, c[1] * k, c[2] * k]
    } else {
        c
    }
}

pub fn autostart_enabled() -> bool {
    let mut size: u32 = 0;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, Some(&mut size)) };
    r == ERROR_SUCCESS
}

pub fn set_autostart(enable: bool) {
    unsafe {
        if enable {
            let Ok(exe) = std::env::current_exe() else { return };
            let cmd = format!("\"{}\"", exe.display());
            let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                REG_SZ.0,
                Some(wide.as_ptr() as _),
                (wide.len() * 2) as u32,
            );
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
        }
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, LEGACY_RUN_VALUE);
    }
}
