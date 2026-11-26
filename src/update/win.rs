//! Windows: baixa e confere a versão nova, pergunta no pop-up (ou instala
//! sozinho) e reinicia.

use super::install;
use super::popup::{self, Choice, View};
use super::{Offer, State, Version, log};
use crate::shared::Shared;
use std::sync::Arc;

fn config(shared: &Shared) -> crate::config::Config {
    shared.config.lock().unwrap().clone()
}

/// Logo depois de abrir: arruma o que a atualização deixou e, se este
/// processo é a versão nova recém-instalada, avisa.
pub fn after_start(shared: &Arc<Shared>) {
    unsafe {
        // Para o ShellExecute (abrir a página da release).
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    install::after_start();
    if let Some(from) = install::updated_from() {
        let me = Version::current();
        log(&format!("atualizado de {from} para {me}"));
        if me > from {
            popup::toast(&config(shared), View::Updated { version: me.to_string() });
        }
    }
}

/// Versão nova: só depois de baixada e com a assinatura conferida ela é
/// oferecida (sem a chave privada, ninguém faz este aviso aparecer).
pub fn offer(shared: &Arc<Shared>, o: Offer, manual: bool) {
    let current = Version::current();
    let verified = match install::download(&o) {
        Ok(v) => v,
        Err(e) => {
            log(&format!("{}: {e}", o.version));
            if manual {
                popup::toast(&config(shared), View::Failed { title: "Não foi possível atualizar", reason: e, page: None });
            }
            return;
        }
    };
    let installed = install::installed_dir().is_some();
    let cfg = config(shared);
    let install_now = || install::apply(&verified).and_then(|exe| install::restart(shared, &exe, &current));
    if cfg.update_auto && installed && !manual {
        match install_now() {
            // O processo fecha em instantes; a versão nova avisa ao abrir.
            Ok(()) => return,
            Err(e) => log(&format!("instalação automática falhou: {e}")),
        }
    }
    let p = popup::open(&cfg, View::Offer { version: o.version.to_string(), current: current.to_string(), notes: o.notes.clone(), install: installed });
    loop {
        match p.wait() {
            Choice::Update if installed => {
                p.set(View::Working { version: o.version.to_string() });
                match install_now() {
                    Ok(()) => return,
                    Err(e) => {
                        log(&format!("atualização falhou: {e}"));
                        p.set(View::Failed { title: "Não foi possível atualizar", reason: e, page: Some(o.page.clone()) });
                    }
                }
            }
            // Cópia solta (não é a instalada): a página da release.
            Choice::Update | Choice::Page => install::open_page(&o.page),
            Choice::Skip => {
                let mut st = State::load();
                st.skipped = o.version.to_string();
                st.save();
                install::forget_download();
                log(&format!("versão {} pulada", o.version));
                return;
            }
            Choice::Later => {
                log(&format!("versão {}: depois", o.version));
                return;
            }
            Choice::Close => return,
        }
    }
}

/// Resultado de "Procurar agora" quando não há versão nova.
pub fn tell(shared: &Arc<Shared>, result: Result<(), String>) {
    let cfg = config(shared);
    match result {
        Ok(()) => popup::toast(&cfg, View::UpToDate { current: Version::current().to_string() }),
        Err(e) => popup::toast(&cfg, View::Failed { title: "Não foi possível procurar atualizações", reason: e, page: None }),
    }
}
