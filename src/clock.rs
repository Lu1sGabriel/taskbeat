//! Contas com `Instant` que nunca dão panic.
//!
//! `Instant::now() - d` dá panic se o resultado cair antes do zero do relógio
//! (no Windows, antes do rustc 1.97, esse zero era o boot), e a própria doc do
//! `Instant` avisa que versões futuras podem voltar a dar panic ao saturar.
//! Por isso "há muito tempo" é `None`, e as contas usam `checked_*`.

use std::time::{Duration, Instant};

/// Tempo desde `t`. `None` ("nunca") conta como há muito tempo.
pub fn elapsed_or_max(t: Option<Instant>) -> Duration {
    since(Instant::now(), t)
}

/// O mesmo que [`elapsed_or_max`], medido a partir de `now`. Um `t` no futuro
/// dá zero.
pub fn since(now: Instant, t: Option<Instant>) -> Duration {
    match t {
        None => Duration::MAX,
        Some(t) => now.checked_duration_since(t).unwrap_or(Duration::ZERO),
    }
}

/// `now - d`, ou `now` se isso cair antes do zero do relógio.
pub fn back(now: Instant, d: Duration) -> Instant {
    now.checked_sub(d).unwrap_or(now)
}

/// Segundos em `f32` para `Duration`, sem o panic do `from_secs_f32` com NaN,
/// negativo, infinito ou grande demais (tudo isso vira zero).
pub fn secs_f32(s: f32) -> Duration {
    Duration::try_from_secs_f32(s).unwrap_or(Duration::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nunca_conta_como_ha_muito_tempo() {
        assert_eq!(elapsed_or_max(None), Duration::MAX);
        assert_eq!(since(Instant::now(), None), Duration::MAX);
    }

    #[test]
    fn agora_e_futuro_dao_zero() {
        let now = Instant::now();
        assert_eq!(since(now, Some(now)), Duration::ZERO);
        assert!(elapsed_or_max(Some(Instant::now())) < Duration::from_secs(1));
        let futuro = now + Duration::from_secs(3600);
        assert_eq!(since(now, Some(futuro)), Duration::ZERO);
        assert_eq!(elapsed_or_max(Some(futuro)), Duration::ZERO);
    }

    #[test]
    fn passado_mede_a_diferenca() {
        let antes = Instant::now();
        let now = antes + Duration::from_secs(4);
        assert_eq!(since(now, Some(antes)), Duration::from_secs(4));
    }

    #[test]
    fn voltar_no_tempo_nao_da_panic() {
        let now = Instant::now();
        assert!(back(now, Duration::from_millis(1)) <= now);
        // Um passo maior que qualquer uptime: fica em `now` em vez de panic.
        assert_eq!(back(now, Duration::MAX), now);
    }

    #[test]
    fn segundos_invalidos_viram_zero() {
        assert_eq!(secs_f32(f32::NAN), Duration::ZERO);
        assert_eq!(secs_f32(f32::INFINITY), Duration::ZERO);
        assert_eq!(secs_f32(-1.0), Duration::ZERO);
        assert_eq!(secs_f32(f32::MAX), Duration::ZERO);
        assert_eq!(secs_f32(1.5), Duration::from_millis(1500));
    }
}
