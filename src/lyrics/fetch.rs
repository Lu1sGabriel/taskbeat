//! Busca das letras numa thread própria: pasta do usuário, cache e, com a
//! busca ligada, o LRCLIB. Só a música mais recente importa: pedidos que
//! chegam no meio de uma busca fazem a anterior ser abandonada.
//!
//! O transporte HTTP vem de fora (cada sistema usa o do próprio sistema).

use super::cache::{self, Cache};
use super::{Lyrics, Track, lrclib};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

/// Resposta HTTP: status, corpo e o `Retry-After` (s), se veio.
#[derive(Clone, Debug, Default)]
pub struct Response {
    pub status: u16,
    pub body: String,
    pub retry_after: Option<u64>,
}

/// GET em `https://{host}{path}` com o User-Agent dado.
pub type Transport = fn(host: &str, path: &str, user_agent: &str) -> Result<Response, String>;

/// Pedido de letra: a música e se pode perguntar ao LRCLIB.
#[derive(Clone, Debug)]
pub struct Request {
    pub track: Track,
    pub online: bool,
}

pub struct Fetcher {
    tx: Sender<Request>,
}

impl Fetcher {
    /// Sobe a thread. `done(chave, letra)` é chamado nela quando a letra
    /// chega (ou quando se sabe que não tem).
    pub fn spawn(transport: Transport, done: impl Fn(String, Lyrics) + Send + 'static) -> Option<Fetcher> {
        let (tx, rx) = channel::<Request>();
        std::thread::Builder::new()
            .name("lyrics-fetch".into())
            .spawn(move || {
                let cache = cache::cache_dir().map(Cache::new);
                if let Some(c) = &cache {
                    c.prune();
                }
                run(rx, transport, cache, cache::user_dir(), &done);
            })
            .ok()?;
        Some(Fetcher { tx })
    }

    pub fn request(&self, track: Track, online: bool) {
        let _ = self.tx.send(Request { track, online });
    }
}

/// O mais recente dos pedidos na fila (os outros já não importam). Com o
/// canal fechado, termina o que está em mãos (o laço sai no próximo `recv`).
fn latest(rx: &Receiver<Request>, mut cur: Option<Request>) -> Option<Request> {
    loop {
        match rx.try_recv() {
            Ok(r) => cur = Some(r),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return cur,
        }
    }
}

fn run(rx: Receiver<Request>, transport: Transport, cache: Option<Cache>, user: Option<PathBuf>, done: &dyn Fn(String, Lyrics)) {
    // O servidor pediu calma (429) até este instante.
    let mut quiet_until: Option<Instant> = None;
    let mut next: Option<Request> = None;
    loop {
        let req = match next.take() {
            Some(r) => r,
            None => match rx.recv() {
                Ok(r) => r,
                Err(_) => return,
            },
        };
        let Some(req) = latest(&rx, Some(req)) else { continue };
        let key = req.track.key();
        if let Some(l) = user.as_deref().and_then(|d| cache::local(d, &req.track)) {
            done(key, l);
            continue;
        }
        if let Some(l) = cache.as_ref().and_then(|c| c.get(&key)) {
            done(key, l);
            continue;
        }
        if !req.online {
            done(key, Lyrics::Offline);
            continue;
        }
        if quiet_until.is_some_and(|t| Instant::now() < t) {
            done(key, Lyrics::Failed);
            continue;
        }
        let ua = lrclib::user_agent();
        let mut result = Lyrics::NotFound;
        let mut superseded = false;
        for (i, q) in lrclib::queries(&req.track).iter().enumerate() {
            // Um pedido novo chegou: este já não importa.
            if let Some(r) = latest(&rx, None) {
                next = Some(r);
                superseded = true;
                break;
            }
            if i > 0 {
                // Pedidos seguidos com um respiro, como o LRCLIB pede.
                std::thread::sleep(Duration::from_millis(250));
            }
            match transport(lrclib::HOST, &q.path, &ua) {
                Ok(r) if r.status == 200 => {
                    let found = if q.search { lrclib::best(&lrclib::parse_search(&r.body), &req.track) } else { lrclib::parse_record(&r.body) };
                    if let Some(l) = found.map(|rec| rec.lyrics()).filter(|l| *l != Lyrics::NotFound) {
                        result = l;
                        break;
                    }
                }
                Ok(r) if r.status == 404 || r.status == 400 => {}
                Ok(r) if r.status == 429 => {
                    let wait = r.retry_after.unwrap_or(60).clamp(5, 600);
                    quiet_until = Some(Instant::now() + Duration::from_secs(wait));
                    result = Lyrics::Failed;
                    break;
                }
                Ok(_) | Err(_) => {
                    result = Lyrics::Failed;
                    break;
                }
            }
        }
        if superseded {
            continue;
        }
        if let Some(c) = &cache {
            c.put(&key, &result);
        }
        done(key, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::mpsc::RecvTimeoutError;

    static CALLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn fake(host: &str, path: &str, ua: &str) -> Result<Response, String> {
        assert_eq!(host, "lrclib.net");
        assert!(ua.starts_with("Taskbeat/"));
        CALLS.lock().unwrap().push(path.to_string());
        if path.contains("track_name=Sem%20Letra") {
            return Ok(Response { status: 404, ..Default::default() });
        }
        if path.contains("track_name=Fora") {
            return Err("sem rede".into());
        }
        if path.starts_with("/api/search") {
            return Ok(Response { status: 200, body: r#"[{"id":7,"trackName":"Achada Na Busca","artistName":"Fulano","duration":200,"syncedLyrics":"[00:01.00]Oi"}]"#.into(), retry_after: None });
        }
        if path.contains("track_name=Achada%20Na%20Busca") {
            return Ok(Response { status: 404, ..Default::default() });
        }
        Ok(Response { status: 200, body: r#"{"id":1,"trackName":"X","artistName":"Y","plainLyrics":"Linha"}"#.into(), retry_after: None })
    }

    /// Roda o laço com um cache numa pasta temporária e devolve o que `done` recebeu.
    fn ask(reqs: Vec<Request>) -> Vec<(String, Lyrics)> {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("taskbeat-fetch-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (tx, rx) = channel();
        let (dtx, drx) = channel();
        let n = reqs.len();
        for r in reqs {
            tx.send(r).unwrap();
        }
        drop(tx);
        let d2 = dir.clone();
        std::thread::spawn(move || run(rx, fake, Some(Cache::new(d2)), None, &move |k, l| dtx.send((k, l)).unwrap()));
        let mut out = Vec::new();
        for _ in 0..n {
            match drx.recv_timeout(Duration::from_secs(5)) {
                Ok(x) => out.push(x),
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        let _ = std::fs::remove_dir_all(dir);
        out
    }

    fn t(a: &str, title: &str) -> Track {
        Track { artist: a.into(), title: title.into(), album: String::new(), dur: 200.0 }
    }

    #[test]
    fn busca_cai_na_lista_e_guarda() {
        let got = ask(vec![Request { track: t("Fulano", "Achada Na Busca"), online: true }]);
        assert!(matches!(&got[0].1, Lyrics::Synced(l) if l[0].text == "Oi"));
        let got = ask(vec![Request { track: t("Fulano", "Sem Letra"), online: true }]);
        assert_eq!(got[0].1, Lyrics::NotFound);
        let got = ask(vec![Request { track: t("Fulano", "Fora"), online: true }]);
        assert_eq!(got[0].1, Lyrics::Failed);
        let got = ask(vec![Request { track: t("Fulano", "Qualquer"), online: false }]);
        assert_eq!(got[0].1, Lyrics::Offline);
    }

    #[test]
    fn so_o_pedido_mais_recente_importa() {
        // Três pedidos de uma vez: só o último é respondido.
        let got = ask(vec![
            Request { track: t("A", "Primeira"), online: true },
            Request { track: t("A", "Segunda"), online: true },
            Request { track: t("A", "Terceira Valendo"), online: true },
        ]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, t("A", "Terceira Valendo").key());
        assert_eq!(got[0].1, Lyrics::Plain(vec!["Linha".into()]));
    }
}
