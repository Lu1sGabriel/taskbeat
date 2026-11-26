//! Versão nova no Mac: um alerta do macOS que abre a página da release.
//!
//! Por enquanto o app não se troca sozinho: sem conta de desenvolvedor da
//! Apple, um `.app` trocado por ele mesmo esbarra no Gatekeeper (e as
//! permissões podem ser pedidas de novo). A pessoa baixa o `.dmg` na página.

use crate::i18n::tr;
use crate::update::{Offer, State, Version, log};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSWorkspace};
use objc2_foundation::{NSString, NSURL};

/// Chamado da thread da atualização: o alerta abre na thread principal.
pub fn offer(o: Offer) {
    dispatch2::DispatchQueue::main().exec_async(move || show(&o));
}

/// "Procurar agora" ainda não existe no Mac: só o registro.
pub fn tell(result: Result<(), String>) {
    if let Err(e) = result {
        log(&format!("procura manual: {e}"));
    }
}

fn show(o: &Offer) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&tr("Taskbeat {} disponível").replace("{}", &o.version.to_string())));
    let mut info = tr("Você está na versão {}").replace("{}", &Version::current().to_string());
    if !o.notes.is_empty() {
        info = format!("{}\n\n{info}", o.notes);
    }
    alert.setInformativeText(&NSString::from_str(&info));
    for b in ["Abrir página", "Depois", "Pular esta versão"] {
        alert.addButtonWithTitle(&NSString::from_str(tr(b)));
    }
    super::compat::activate(mtm);
    let r = alert.runModal();
    if r == NSAlertFirstButtonReturn {
        if o.page.starts_with("https://github.com/Lu1sGabriel/taskbeat/") {
            if let Some(u) = NSURL::URLWithString(&NSString::from_str(&o.page)) {
                NSWorkspace::sharedWorkspace().openURL(&u);
            }
        }
        log(&format!("versão {}: página aberta", o.version));
    } else if r == NSAlertSecondButtonReturn {
        log(&format!("versão {}: depois", o.version));
    } else {
        let mut st = State::load();
        st.skipped = o.version.to_string();
        st.save();
        log(&format!("versão {} pulada", o.version));
    }
}
