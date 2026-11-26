//! Letras das músicas: leitura do LRC, sincronia com a posição da música,
//! busca no LRCLIB (lrclib.net) e cache em disco.
//!
//! Independente de plataforma: o Windows, o Mac e o Linux usam o mesmo código;
//! cada um só entrega o transporte HTTP (`fetch::Transport`; no Windows,
//! `http_win`, com o WinHTTP do sistema) e desenha a letra do seu jeito.

pub mod cache;
pub mod config;
pub mod fetch;
#[cfg(windows)]
pub mod http_win;
pub mod lrclib;

/// Uma linha da letra sincronizada: começa em `t` segundos.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub t: f64,
    /// Texto da linha; vazio = pausa instrumental.
    pub text: String,
}

/// O que se sabe da letra de uma música.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Lyrics {
    /// Ainda procurando.
    #[default]
    Loading,
    /// Letra sincronizada, linha por linha (ordenada pelo tempo).
    Synced(Vec<Line>),
    /// Só o texto, sem tempos.
    Plain(Vec<String>),
    /// A música é instrumental (o LRCLIB marca assim).
    Instrumental,
    /// Não achou letra.
    NotFound,
    /// Não deu para buscar (sem internet, servidor fora); tenta de novo depois.
    Failed,
    /// Busca na internet desligada e nada no cache nem na pasta local.
    Offline,
}

impl Lyrics {
    pub fn is_text(&self) -> bool {
        matches!(self, Lyrics::Synced(_) | Lyrics::Plain(_))
    }
}

/// A música que está tocando, como o player informa.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub artist: String,
    pub title: String,
    pub album: String,
    /// Duração em segundos (0 = desconhecida).
    pub dur: f64,
}

impl Track {
    /// Chave da música no cache e nas trocas (artista, título e álbum,
    /// sem diferença de maiúsculas e espaços).
    pub fn key(&self) -> String {
        format!("{}\n{}\n{}", norm(&self.artist), norm(&self.title), norm(&self.album))
    }
}

/// Minúsculas e espaços simples (para comparar nomes).
pub fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

// ---------------------------------------------------------------- LRC

/// Lê um tempo do LRC: `mm:ss`, `mm:ss.xx`, `mm:ss.xxx`, `mm:ss:xx` ou `h:mm:ss.xx`.
fn parse_time(s: &str) -> Option<f64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    let num = |p: &str| -> Option<f64> {
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return None;
        }
        p.parse::<f64>().ok()
    };
    match parts.as_slice() {
        [m, sec] => Some(num(m)? * 60.0 + num(sec)?),
        // `mm:ss:xx` (centésimos depois de dois-pontos) ou `h:mm:ss.xx`.
        [a, b, c] => {
            if !c.contains('.') && c.len() == 2 && !b.contains('.') && b.len() == 2 && a.len() >= 2 {
                Some(num(a)? * 60.0 + num(b)? + num(c)? / 100.0)
            } else {
                Some(num(a)? * 3600.0 + num(b)? * 60.0 + num(c)?)
            }
        }
        _ => None,
    }
}

/// Tira os tempos de palavra do LRC "aprimorado" (`<00:12.34>`) do texto.
fn strip_word_times(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        match rest[i..].find('>') {
            Some(j) if parse_time(&rest[i + 1..i + j]).is_some() => {
                out.push_str(&rest[..i]);
                rest = &rest[i + j + 1..];
            }
            _ => {
                out.push_str(&rest[..=i]);
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lê um arquivo LRC. Linhas com vários tempos (`[00:10.00][01:20.00]refrão`)
/// viram várias linhas; `[offset:+250]` adianta a letra 250 ms; marcas sem
/// tempo (`[ar:…]`, `[ti:…]`) são ignoradas. Devolve as linhas em ordem de tempo.
pub fn parse_lrc(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut offset = 0.0f64;
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else { break };
            let tag = &rest[1..end];
            if let Some(t) = parse_time(tag) {
                times.push(t);
            } else if let Some(v) = tag.strip_prefix("offset:") {
                offset = v.trim().trim_start_matches('+').parse::<f64>().unwrap_or(0.0) / 1000.0;
            }
            rest = rest[end + 1..].trim_start();
        }
        let text = strip_word_times(rest);
        for t in times {
            lines.push(Line { t, text: text.clone() });
        }
    }
    // Offset positivo: a letra aparece antes (convenção do LRC).
    for l in &mut lines {
        l.t = (l.t - offset).max(0.0);
    }
    lines.sort_by(|a, b| a.t.total_cmp(&b.t));
    lines
}

/// Texto simples em linhas (tira linhas vazias repetidas e as das pontas).
pub fn parse_plain(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in text.trim_start_matches('\u{feff}').lines() {
        let l = l.trim();
        if l.is_empty() && out.last().is_none_or(|p| p.is_empty()) {
            continue;
        }
        out.push(l.to_string());
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

// ---------------------------------------------------------------- sincronia

/// Introdução a partir de quanto tempo vira "• • •" (mais curta, a primeira
/// linha já aparece como a próxima).
pub const INTRO_MIN: f64 = 4.0;
/// Uma linha comum vale por até tanto; se a próxima demora mais que
/// `LONG_GAP`, o resto é um trecho instrumental sem marca ("• • •").
pub const LINE_HOLD: f64 = 8.0;
pub const LONG_GAP: f64 = 14.0;

/// Linha que está tocando em `t` segundos: a última que já começou.
/// `None` antes da primeira linha.
pub fn line_at(lines: &[Line], t: f64) -> Option<usize> {
    let n = lines.partition_point(|l| l.t <= t);
    n.checked_sub(1)
}

/// Quando a linha em `t` muda (início da próxima), se ainda muda.
pub fn next_change(lines: &[Line], t: f64) -> Option<f64> {
    let n = lines.partition_point(|l| l.t <= t);
    lines.get(n).map(|l| l.t)
}

/// As linhas como aparecem: com uma linha vazia ("• • •") na introdução
/// longa e no meio de trechos instrumentais sem marca, e sem vazias seguidas.
/// Com isso, a linha atual vazia é uma pausa e cada troca é uma linha.
pub fn with_gaps(lines: &[Line]) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::with_capacity(lines.len() + 4);
    let gap = |t: f64| Line { t, text: String::new() };
    if lines.first().is_some_and(|f| f.t >= INTRO_MIN) {
        out.push(gap(0.0));
    }
    for (i, l) in lines.iter().enumerate() {
        if l.text.is_empty() && out.last().is_some_and(|p| p.text.is_empty()) {
            continue;
        }
        out.push(l.clone());
        if let Some(n) = lines.get(i + 1) {
            if !l.text.is_empty() && n.t - l.t > LONG_GAP {
                out.push(gap(l.t + LINE_HOLD));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempos_do_lrc() {
        assert_eq!(parse_time("01:02.50"), Some(62.5));
        assert_eq!(parse_time("01:02"), Some(62.0));
        assert_eq!(parse_time("00:05.123"), Some(5.123));
        assert_eq!(parse_time("01:02:50"), Some(62.5));
        assert_eq!(parse_time("1:02:03.5"), Some(3723.5));
        assert_eq!(parse_time("ar:Artista"), None);
        assert_eq!(parse_time("offset:+100"), None);
        assert_eq!(parse_time(""), None);
    }

    #[test]
    fn le_lrc_com_marcas_offset_e_tempos_repetidos() {
        let lrc = "\u{feff}[ar:Alguém]\r\n[ti:Uma música]\r\n[offset:+500]\r\n[00:10.00][01:10.00]Refrão aqui\r\n[00:05.00]Primeira\r\n[00:20.00]\r\n[00:30.00] Última   linha \r\n";
        let l = parse_lrc(lrc);
        let got: Vec<(f64, &str)> = l.iter().map(|x| (x.t, x.text.as_str())).collect();
        assert_eq!(got, vec![(4.5, "Primeira"), (9.5, "Refrão aqui"), (19.5, ""), (29.5, "Última linha"), (69.5, "Refrão aqui")]);
    }

    #[test]
    fn tira_tempos_de_palavra() {
        let l = parse_lrc("[00:01.00]<00:01.00>Uma <00:01.50>linha <00:02.00>só");
        assert_eq!(l[0].text, "Uma linha só");
        // "<" que não é tempo fica.
        assert_eq!(strip_word_times("a <3 b"), "a <3 b");
    }

    #[test]
    fn offset_negativo_atrasa() {
        let l = parse_lrc("[offset:-250]\n[00:01.00]x");
        assert_eq!(l[0].t, 1.25);
    }

    #[test]
    fn texto_simples() {
        let p = parse_plain("\n\nUm\nDois\n\n\n\nTrês\n\n");
        assert_eq!(p, vec!["Um", "Dois", "", "Três"]);
    }

    fn sample() -> Vec<Line> {
        parse_lrc("[00:08.00]Um\n[00:12.00]Dois\n[00:15.00]\n[00:25.00]Três\n[00:28.00]Quatro\n[01:00.00]Cinco")
    }

    #[test]
    fn linha_atual_e_proxima_troca() {
        let l = sample();
        assert_eq!(line_at(&l, 0.0), None);
        assert_eq!(line_at(&l, 8.0), Some(0));
        assert_eq!(line_at(&l, 11.99), Some(0));
        assert_eq!(line_at(&l, 12.0), Some(1));
        assert_eq!(line_at(&l, 500.0), Some(5));
        assert_eq!(next_change(&l, 0.0), Some(8.0));
        assert_eq!(next_change(&l, 12.0), Some(15.0));
        assert_eq!(next_change(&l, 61.0), None);
        assert_eq!(line_at(&[], 3.0), None);
        assert_eq!(next_change(&[], 3.0), None);
    }

    #[test]
    fn pausas_viram_linhas_vazias() {
        let g = with_gaps(&sample());
        let got: Vec<(f64, &str)> = g.iter().map(|x| (x.t, x.text.as_str())).collect();
        // Introdução de 8 s; "Quatro" (28 s) até "Cinco" (60 s) vira pausa aos 36 s.
        assert_eq!(got, vec![(0.0, ""), (8.0, "Um"), (12.0, "Dois"), (15.0, ""), (25.0, "Três"), (28.0, "Quatro"), (36.0, ""), (60.0, "Cinco")]);
        let at = |t: f64| line_at(&g, t).map(|i| g[i].text.as_str());
        assert_eq!(at(2.0), Some(""));
        assert_eq!(at(13.0), Some("Dois"));
        assert_eq!(at(16.0), Some(""));
        assert_eq!(at(30.0), Some("Quatro"));
        assert_eq!(at(40.0), Some(""));
        assert_eq!(at(60.0), Some("Cinco"));
        assert_eq!(next_change(&g, 29.0), Some(36.0));
        // Introdução curta: sem pausa; vazias seguidas viram uma só.
        let short = with_gaps(&parse_lrc("[00:02.00]Já
[00:03.00]
[00:04.00]
[00:05.00]Fim"));
        assert_eq!(short.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), vec!["Já", "", "Fim"]);
        assert!(with_gaps(&[]).is_empty());
    }

    #[test]
    fn chave_ignora_caixa_e_espacos() {
        let a = Track { artist: "M83".into(), title: "Midnight  City".into(), album: "".into(), dur: 243.0 };
        let b = Track { artist: "m83 ".into(), title: "midnight city".into(), album: "".into(), dur: 241.0 };
        assert_eq!(a.key(), b.key());
    }
}
