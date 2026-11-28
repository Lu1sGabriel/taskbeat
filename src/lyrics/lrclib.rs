//! LRCLIB (lrclib.net): banco de letras aberto e gratuito, sem chave.
//!
//! Vão para o serviço o título, o artista, o álbum e a duração da música (só
//! isso). O cliente se identifica no User-Agent, como a documentação pede, e
//! respeita o `Retry-After` quando o servidor pede calma (429).

use super::{Lyrics, Track, norm, parse_lrc, parse_plain};
use serde::Deserialize;

pub const HOST: &str = "lrclib.net";

/// "Taskbeat/1.3.0 (https://github.com/Lu1sGabriel/taskbeat)".
pub fn user_agent() -> String {
    format!("Taskbeat/{} ({})", env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_REPOSITORY"))
}

/// Um registro do LRCLIB (resposta de `/api/get` e itens de `/api/search`).
#[derive(Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct Record {
    pub id: i64,
    pub track_name: Option<String>,
    pub artist_name: Option<String>,
    pub album_name: Option<String>,
    pub duration: Option<f64>,
    pub instrumental: bool,
    pub plain_lyrics: Option<String>,
    pub synced_lyrics: Option<String>,
}

impl Record {
    /// A letra deste registro (sincronizada, se tiver).
    pub fn lyrics(&self) -> Lyrics {
        if self.instrumental {
            return Lyrics::Instrumental;
        }
        if let Some(s) = self.synced_lyrics.as_deref() {
            let lines = parse_lrc(s);
            if lines.iter().any(|l| !l.text.is_empty()) {
                return Lyrics::Synced(lines);
            }
        }
        if let Some(p) = self.plain_lyrics.as_deref() {
            let lines = parse_plain(p);
            if lines.iter().any(|l| !l.is_empty()) {
                return Lyrics::Plain(lines);
            }
        }
        Lyrics::NotFound
    }
}

/// Codifica um valor para a URL (UTF-8, só os caracteres seguros ficam).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Palavras que, entre parênteses ou depois de um traço, não são parte do
/// nome da música (vídeos do YouTube, relançamentos).
const NOISE: &[&str] = &[
    "official", "oficial", "video", "vídeo", "clipe", "clip", "audio", "áudio", "lyric", "lyrics", "letra", "legendado", "visualizer", "visualiser",
    "mv", "m/v", "hd", "hq", "4k", "remaster", "remastered", "remasterizado", "feat", "ft", "featuring", "music video",
];

fn is_noise(seg: &str) -> bool {
    let s = seg.to_lowercase();
    NOISE.iter().any(|w| s.split(|c: char| !c.is_alphanumeric() && c != '/').any(|x| x == *w) || (w.contains(' ') && s.contains(w)))
}

/// Título sem os enfeites de vídeo: "(Official Video)", "[Lyrics]", "feat. X"…
pub fn clean_title(t: &str) -> String {
    let mut s = t.trim().to_string();
    // Trechos entre parênteses/colchetes com palavras de "ruído".
    loop {
        let mut cut = None;
        for (open, close) in [('(', ')'), ('[', ']'), ('【', '】')] {
            let mut from = 0;
            while let Some(i) = s[from..].find(open).map(|i| i + from) {
                let Some(j) = s[i..].find(close).map(|j| j + i) else { break };
                if is_noise(&s[i + open.len_utf8()..j]) {
                    cut = Some((i, j + close.len_utf8()));
                    break;
                }
                from = j + close.len_utf8();
            }
            if cut.is_some() {
                break;
            }
        }
        match cut {
            Some((a, b)) => s.replace_range(a..b, " "),
            None => break,
        }
    }
    // "Música - Official Video" / "Música | Lyrics".
    for sep in [" - ", " | ", " – ", " — "] {
        if let Some(i) = s.rfind(sep)
            && is_noise(&s[i + sep.len()..])
        {
            s.truncate(i);
        }
    }
    // "Música feat. Fulano" sem parênteses. Só ASCII: o índice achado em `low`
    // corta `s`, e o `to_lowercase` muda o tamanho de letras como 'İ' e 'K'.
    let low = s.to_ascii_lowercase();
    for w in [" feat. ", " feat ", " ft. ", " featuring "] {
        if let Some(i) = low.find(w) {
            s.truncate(i);
            break;
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Artista sem os enfeites de canal: "ColdplayVEVO", "Coldplay - Topic".
pub fn clean_artist(a: &str) -> String {
    let mut s = a.trim().to_string();
    for suf in [" - Topic", " - Tópico", "VEVO", " Official", " Oficial"] {
        if s.len() > suf.len() && s.ends_with(suf) {
            s.truncate(s.len() - suf.len());
        }
    }
    s.trim().to_string()
}

/// Primeiro artista de "A, B", "A & B", "A feat. B", "A x B".
pub fn first_artist(a: &str) -> String {
    // Só ASCII, pelo mesmo motivo do `clean_title`: o índice corta `a`.
    let low = a.to_ascii_lowercase();
    let mut end = a.len();
    for sep in [", ", " & ", " feat. ", " feat ", " ft. ", " featuring ", " x ", "; ", " / "] {
        if let Some(i) = low.find(sep) {
            end = end.min(i);
        }
    }
    a[..end].trim().to_string()
}

/// O canal parece de um artista (vídeo do YouTube com "Artista - Música" no título)?
fn channel_like(a: &str) -> bool {
    a.is_empty() || a.ends_with("VEVO") || a.ends_with(" - Topic") || a.to_lowercase().contains("official")
}

/// Pedido ao LRCLIB: caminho com a consulta e se a resposta é uma lista.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub path: String,
    pub search: bool,
}

fn get_query(artist: &str, title: &str, album: &str, dur: f64) -> Query {
    let mut p = format!("/api/get?artist_name={}&track_name={}", encode(artist), encode(title));
    if !album.trim().is_empty() {
        p.push_str(&format!("&album_name={}", encode(album.trim())));
    }
    if (1.0..=3600.0).contains(&dur) {
        p.push_str(&format!("&duration={}", dur.round() as i64));
    }
    Query { path: p, search: false }
}

/// Pares (artista, título) a tentar, do mais fiel ao mais solto.
pub fn candidates(t: &Track) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |a: String, b: String| {
        if !b.trim().is_empty() && !out.iter().any(|(x, y)| norm(x) == norm(&a) && norm(y) == norm(&b)) {
            out.push((a, b));
        }
    };
    let artist = clean_artist(&t.artist);
    let title = clean_title(&t.title);
    // "Artista - Música" no título (vídeos): o lado esquerdo é o artista.
    let split = ["  - ", " - ", " – ", " — "].iter().find_map(|sep| title.split_once(sep));
    if let Some((l, r)) = split {
        let (l, r) = (l.trim().to_string(), clean_title(r));
        if channel_like(&t.artist) || norm(&l) == norm(&artist) || norm(&first_artist(&l)) == norm(&first_artist(&artist)) {
            add(l.clone(), r.clone());
            add(first_artist(&l), r);
            return out;
        }
        add(artist.clone(), title.clone());
        add(l, r);
    } else {
        add(artist.clone(), title.clone());
    }
    add(first_artist(&artist), title);
    out
}

/// Pedidos ao LRCLIB para a música, na ordem: exato (com álbum e duração),
/// limpo (sem álbum) e busca. Para assim que um acha a letra.
pub fn queries(t: &Track) -> Vec<Query> {
    let mut q = Vec::new();
    if !t.title.trim().is_empty() && !t.artist.trim().is_empty() {
        q.push(get_query(t.artist.trim(), t.title.trim(), &t.album, t.dur));
    }
    let cands = candidates(t);
    for (a, b) in &cands {
        if !a.is_empty() {
            q.push(get_query(a, b, "", t.dur));
        }
    }
    if let Some((a, b)) = cands.first() {
        let mut p = format!("/api/search?track_name={}", encode(b));
        if !a.is_empty() {
            p.push_str(&format!("&artist_name={}", encode(&first_artist(a))));
        }
        q.push(Query { path: p, search: true });
    }
    let mut seen = Vec::new();
    q.retain(|x| {
        let new = !seen.contains(&x.path);
        seen.push(x.path.clone());
        new
    });
    q.truncate(4);
    q
}

/// Melhor registro de uma busca: mesmo nome, duração perto (até 3 s; mais de
/// 10 s de diferença é outra versão) e, entre os bons, o sincronizado.
pub fn best(results: &[Record], t: &Track) -> Option<Record> {
    let title = norm(&clean_title(&t.title));
    let artist = norm(&first_artist(&clean_artist(&t.artist)));
    let score = |r: &Record| -> Option<i32> {
        let lyr = r.lyrics();
        if !(lyr.is_text() || lyr == Lyrics::Instrumental) {
            return None;
        }
        let mut s = 0;
        if t.dur > 0.0 {
            let d = (r.duration.unwrap_or(0.0) - t.dur).abs();
            if d > 10.0 {
                return None;
            }
            s += if d <= 3.0 { 40 } else { 10 };
        }
        let rt = norm(&clean_title(r.track_name.as_deref().unwrap_or("")));
        if rt == title {
            s += 30;
        } else if !rt.contains(&title) && !title.contains(&rt) {
            return None;
        }
        let ra = norm(r.artist_name.as_deref().unwrap_or(""));
        if !artist.is_empty() && (ra == artist || ra.contains(&artist) || artist.contains(&ra)) {
            s += 20;
        }
        if matches!(lyr, Lyrics::Synced(_)) {
            s += 25;
        }
        Some(s)
    };
    results.iter().filter_map(|r| score(r).map(|s| (s, r))).max_by_key(|(s, _)| *s).map(|(_, r)| r.clone())
}

/// Lê a resposta de `/api/get` (um registro).
pub fn parse_record(body: &str) -> Option<Record> {
    serde_json::from_str(body).ok()
}

/// Lê a resposta de `/api/search` (uma lista).
pub fn parse_search(body: &str) -> Vec<Record> {
    serde_json::from_str(body).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(a: &str, t: &str, al: &str, d: f64) -> Track {
        Track { artist: a.into(), title: t.into(), album: al.into(), dur: d }
    }

    #[test]
    fn codifica_url() {
        assert_eq!(encode("Ação & Reação/2"), "A%C3%A7%C3%A3o%20%26%20Rea%C3%A7%C3%A3o%2F2");
        assert_eq!(encode("abc-_.~"), "abc-_.~");
    }

    #[test]
    fn limpa_titulos_de_video() {
        assert_eq!(clean_title("Yellow (Official Video)"), "Yellow");
        assert_eq!(clean_title("Song [Lyrics] (feat. Someone)"), "Song");
        assert_eq!(clean_title("Música - Clipe Oficial"), "Música");
        assert_eq!(clean_title("Tal Música feat. Fulano"), "Tal Música");
        assert_eq!(clean_title("Evidências (Ao Vivo)"), "Evidências (Ao Vivo)");
        assert_eq!(clean_title("Love Song (Remastered 2011)"), "Love Song");
        assert_eq!(clean_artist("ColdplayVEVO"), "Coldplay");
        assert_eq!(clean_artist("Coldplay - Topic"), "Coldplay");
        assert_eq!(first_artist("Anitta, J Balvin & Outro"), "Anitta");
        assert_eq!(first_artist("Zé Neto e Cristiano"), "Zé Neto e Cristiano");
    }

    #[test]
    fn titulo_com_artista_de_video() {
        let c = candidates(&track("CanalQualquerVEVO", "Coldplay - Yellow (Official Video)", "", 266.0));
        assert_eq!(c[0], ("Coldplay".into(), "Yellow".into()));
        // Artista do player igual ao lado esquerdo: só a música do lado direito.
        let c = candidates(&track("Coldplay", "Coldplay - Yellow", "", 0.0));
        assert_eq!(c[0], ("Coldplay".into(), "Yellow".into()));
        // Título com traço de verdade e artista que não bate: tenta os dois.
        let c = candidates(&track("Banda", "Parte 1 - Parte 2", "", 0.0));
        assert_eq!(c[0], ("Banda".into(), "Parte 1 - Parte 2".into()));
        assert_eq!(c[1], ("Parte 1".into(), "Parte 2".into()));
    }

    #[test]
    fn ordem_dos_pedidos() {
        let q = queries(&track("M83", "Midnight City", "Hurry Up, We're Dreaming", 243.4));
        assert_eq!(q[0].path, "/api/get?artist_name=M83&track_name=Midnight%20City&album_name=Hurry%20Up%2C%20We%27re%20Dreaming&duration=243");
        assert_eq!(q[1].path, "/api/get?artist_name=M83&track_name=Midnight%20City&duration=243");
        assert!(q.last().unwrap().search);
        assert_eq!(q.last().unwrap().path, "/api/search?track_name=Midnight%20City&artist_name=M83");
        assert!(q.len() <= 4);
        // Sem duração nem álbum.
        let q = queries(&track("A", "B", "", 0.0));
        assert_eq!(q[0].path, "/api/get?artist_name=A&track_name=B");
        assert_eq!(q.len(), 2);
        // Sem título: nada a perguntar.
        assert!(queries(&track("A", "", "", 0.0)).is_empty());
    }

    #[test]
    fn le_registro_e_escolhe_o_melhor() {
        let body = r#"{"id":1,"trackName":"Midnight City","artistName":"M83","albumName":null,"duration":243.0,"instrumental":false,"plainLyrics":"Um\nDois","syncedLyrics":"[00:01.00]Um\n[00:02.00]Dois"}"#;
        let r = parse_record(body).unwrap();
        assert!(matches!(r.lyrics(), Lyrics::Synced(ref l) if l.len() == 2));
        let plain = Record { id: 2, track_name: Some("Midnight City".into()), artist_name: Some("M83".into()), duration: Some(244.0), plain_lyrics: Some("Um".into()), ..Default::default() };
        let other = Record { id: 3, track_name: Some("Midnight City".into()), artist_name: Some("M83".into()), duration: Some(300.0), synced_lyrics: Some("[00:01.00]x".into()), ..Default::default() };
        let wrong = Record { id: 4, track_name: Some("Outra".into()), artist_name: Some("M83".into()), duration: Some(243.0), synced_lyrics: Some("[00:01.00]x".into()), ..Default::default() };
        let t = track("M83", "Midnight City", "", 243.0);
        assert_eq!(best(&[plain.clone(), other.clone(), wrong.clone(), r.clone()], &t).unwrap().id, 1);
        assert_eq!(best(&[plain.clone(), other.clone(), wrong.clone()], &t).unwrap().id, 2);
        assert!(best(&[other, wrong], &t).is_none());
        let list = parse_search(&format!("[{body}]"));
        assert_eq!(list.len(), 1);
        assert!(parse_search("{\"message\":\"x\"}").is_empty());
        // Instrumental e vazio.
        assert_eq!(Record { instrumental: true, ..Default::default() }.lyrics(), Lyrics::Instrumental);
        assert_eq!(Record { synced_lyrics: Some("[00:01.00]".into()), ..Default::default() }.lyrics(), Lyrics::NotFound);
    }

    /// Letras que mudam de tamanho em bytes no `to_lowercase` ('İ' 2→3,
    /// 'K' Kelvin 3→1, 'ẞ' 3→2). O índice achado no texto em minúsculas não
    /// pode cortar o original no meio de um caractere.
    #[test]
    fn letras_que_mudam_de_tamanho_nao_dao_panic() {
        assert_eq!(first_artist("BİRİCİK, ŞEBNEM"), "BİRİCİK");
        assert_eq!(first_artist("İİİİ, x"), "İİİİ");
        assert_eq!(first_artist("\u{212A}\u{212A}\u{212A} & Y"), "\u{212A}\u{212A}\u{212A}");
        assert_eq!(first_artist("A FEAT. B"), "A");
        assert_eq!(clean_title("\u{1E9E}\u{1E9E}\u{1E9E} feat. X"), "\u{1E9E}\u{1E9E}\u{1E9E}");
        assert_eq!(clean_title("İİİİİİ feat. X"), "İİİİİİ");
        assert_eq!(clean_title("\u{212A}\u{212A} FT. Z"), "\u{212A}\u{212A}");
        // O ruído com acento continua casando ("VÍDEO" → "vídeo" no `is_noise`).
        assert_eq!(clean_title("Música (VÍDEO OFICIAL)"), "Música");
    }

    /// Propriedade: nenhum texto montado com as letras que mudam de tamanho e
    /// os separadores dá panic. Gerador determinístico, sem crate nova.
    #[test]
    fn propriedade_sem_panic_com_letras_especiais() {
        const PECAS: &[&str] = &[
            "İ", "\u{212A}", "\u{1E9E}", "Ş", "ß", "Σ", "a", "Z", " ", ", ", " & ", " feat. ", " FEAT ", " ft. ", " x ", "; ", " / ", " featuring ", "(", ")", "[",
            "]", "【", "】", " - ", " | ", " – ", "video", "VÍDEO",
        ];
        let mut k: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..20_000 {
            k = k.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let n = (k >> 60) as usize;
            let mut s = String::new();
            for _ in 0..n {
                k = k.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                s.push_str(PECAS[(k >> 33) as usize % PECAS.len()]);
            }
            let _ = first_artist(&s);
            let _ = clean_title(&s);
            let _ = clean_artist(&s);
        }
    }
}
