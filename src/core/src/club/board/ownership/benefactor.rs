//! The owner who is funding this club out of his own pocket.
//!
//! Some clubs spend money their football cannot earn. Al-Hilal's wage bill
//! is about twice its revenue; Inter Miami's is a multiple of an MLS
//! side's; Zenit and PSG run bills their gates and their shirts do not
//! cover. In every one of those cases the difference is written by an
//! owner, and it is why those clubs can pay a Premier League wage in a
//! league that could never earn one.
//!
//! The simulator used to model none of that. A club's wage budget was
//! `projected_income × ratio`, its transfer ceiling was a country constant,
//! and its bank balance was unreachable — so the richest clubs outside
//! Europe offered a Premier League star *half* his wage and he refused.
//!
//! [`ClubBenefactor`] is the missing signal, and it is deliberately TWO
//! ratios off one balance sheet. No flag, no league list, no country
//! table. The Gulf, China, Russia and MLS all fall out of it, and they all
//! disappear when the ratios normalise — which is exactly what happened to
//! the Chinese Super League after the import tax (memory
//! `feedback_balance_system_not_cases`).
//!
//! **Why not cash ÷ income.** The first cut of this model read "idle cash
//! worth more than a year and a half of revenue". Against the real world's
//! numbers that works; against the SIM's own revenue model it does not.
//! The sim pays its giants three to six times what the design assumed
//! (Al-Ittihad ≈ $110M, Zenit ≈ $268M), so after the wage cover nobody's
//! `idle ÷ income` reached the bar — while a second-division club, whose
//! projected revenue is tiny (`tier_pool_fraction(2) = 0.12`, commercial
//! ∝ rep³) and whose DB balance is not, read 0.5–0.8 and was flipped
//! state-backed. The census printed four Saudi SECOND-division sides as
//! the world's only benefactors and not one of the giants.
//!
//! The real-world tell is not the cash at all — it is **the wage bill the
//! revenue cannot carry**, which is UEFA's own metric, held up against
//! whether the cash is still there to pay it. A club overspending on
//! borrowed money is distressed; a club overspending with the balance to
//! cover it has an owner.

use crate::transfers::pipeline::trace::MarketSwitches;

/// How much of this club's spending its owner, rather than its revenue, is
/// paying for.
pub struct ClubBenefactor;

impl ClubBenefactor {
    /// Years of wage bill a club is assumed to want in the bank before any
    /// of its cash reads as idle. Matches the transfer-ceiling model's
    /// `WAGE_COVER_YEARS` so the two never disagree about what "spare cash"
    /// means.
    pub const WAGE_COVER_YEARS: f64 = 1.5;

    /// Wages worth this share of a year's revenue are what a club can
    /// carry on its own. Everything above is somebody else's money.
    ///
    /// 0.80 is the top of the healthy band the board's own wage mandate
    /// targets (`wage_revenue_target` clamps to 0.30..0.80) — a club at or
    /// under it is spending what it earns, however large the numbers are.
    /// Around 55–70 % of the world's top-flight clubs sit under it; PSG at
    /// ≈ 0.7× and a distressed side at ≈ 1.3× on an overdraft both read 0,
    /// for opposite reasons.
    pub const WAGE_FLOOR: f64 = 0.80;

    /// Wages this far above the floor, as a share of revenue, are a fully
    /// owner-funded club. Al-Hilal's ≈ 2× revenue lands at the top of the
    /// band; Zenit's ≈ 0.9× lands just inside it (≈ 0.12); Inter Miami's
    /// designated-player bill on MLS revenue lands high. Roughly the
    /// twenty to thirty clubs in the world that spend a year's turnover
    /// again on wages — the ≤ 2 % share the design's Part VI band names.
    pub const WAGE_SPAN: f64 = 0.80;

    /// Share of idle cash an owner will convert into wages in a year.
    /// Bounded so a benefactor club cannot strip the elite band in one
    /// window (Part VIII, "the drain").
    pub const SUBSIDY_SHARE: f64 = 0.35;

    /// Half-life of the benefactor read, in seasons. A club that spends its
    /// pile keeps its owner's habit for a while; a club whose owner leaves
    /// loses it slowly rather than overnight.
    pub const HALF_LIFE_SEASONS: f32 = 3.0;

    /// Wages the revenue cannot carry, backed by cash that is still there,
    /// 0..1.
    ///
    /// Two numbers off one balance sheet:
    ///
    /// * `overspend` — how far the wage bill is past what a year's revenue
    ///   supports ([`Self::WAGE_FLOOR`] / [`Self::WAGE_SPAN`]). This is the
    ///   tell: an owner's money shows up as a wage bill, not as a pile.
    /// * `solvent` — whether the balance covers that bill at all. A club
    ///   overspending on borrowed money is distressed, not bankrolled, and
    ///   multiplying by this is what separates Al-Hilal from a side in
    ///   administration paying the same ratio.
    ///
    /// `income` is the sim's revenue-derived figure, not the real club's
    /// turnover.
    ///
    /// **Fails closed on no evidence.** A missing denominator is not
    /// "infinitely owner-funded", it is "we do not know yet". Callers with
    /// no trailing year feed a PROJECTION instead
    /// ([`crate::club::finance::RevenueModel::projected_annual`]) so the
    /// clubs that genuinely are owner-funded read as such on day one, from
    /// data rather than from a missing divisor.
    pub fn signal(balance: i64, annual_wages: i64, annual_income: i64) -> f32 {
        if annual_income <= 0 || annual_wages <= 0 {
            return 0.0;
        }
        let wages = annual_wages as f64;
        let overspend =
            (((wages / annual_income as f64) - Self::WAGE_FLOOR) / Self::WAGE_SPAN).clamp(0.0, 1.0);
        let solvent = (balance as f64 / wages).clamp(0.0, 1.0);
        (overspend * solvent) as f32
    }

    /// Cash the club is not holding back to cover its wage bill.
    pub fn idle_cash(balance: i64, annual_wages: i64) -> f64 {
        (balance as f64 - annual_wages.max(0) as f64 * Self::WAGE_COVER_YEARS).max(0.0)
    }

    /// Fold this season's reading into the stored one on a three-season
    /// half-life. Yearly cadence, so `alpha = 1 − 2^(−1/3) ≈ 0.206`.
    pub fn blend(stored: f32, signal: f32) -> f32 {
        let alpha = 1.0 - (0.5_f32).powf(1.0 / Self::HALF_LIFE_SEASONS);
        (stored + alpha * (signal - stored)).clamp(0.0, 1.0)
    }

    /// What the owner will put into the wage bill this year.
    ///
    /// Bounded three ways: by how owner-funded the club is at all, by the
    /// owner's own appetite for writing cheques
    /// ([`super::OwnershipModel::injection_appetite`]), and by a share of
    /// the cash actually sitting there.
    ///
    /// The ONE place every owner cheque is sized — the wage subsidy, the
    /// tier envelopes split off it, and the transfer-fee headroom — so
    /// [`MarketSwitches::owner_money_off`] disarms all three together for
    /// the census A/B.
    pub fn subsidy_per_year(benefactor: f32, injection_appetite: f32, idle_cash: f64) -> f64 {
        if MarketSwitches::owner_money_off() {
            return 0.0;
        }
        (benefactor.clamp(0.0, 1.0) * injection_appetite.clamp(0.0, 1.0)) as f64
            * idle_cash.max(0.0)
            * Self::SUBSIDY_SHARE
    }

    /// The bar at which an owner's funding is doing the talking. Above it
    /// the club reads as state-backed at ANY reputation, which is what puts
    /// a 5200-reputation league's richest side on the same footing as a
    /// European giant when it comes calling.
    pub const STATE_BACKED_BAR: f32 = 0.5;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_club_living_on_its_revenue_has_no_benefactor() {
        // Break-even side: a season's wages in the bank, nothing more.
        assert_eq!(
            ClubBenefactor::signal(60_000_000, 40_000_000, 80_000_000),
            0.0
        );
        // …and one running an overdraft against a huge wage bill has none
        // either, however famous it is.
        assert_eq!(
            ClubBenefactor::signal(-20_000_000, 340_000_000, 700_000_000),
            0.0
        );
    }

    #[test]
    fn no_revenue_evidence_reads_as_no_owner() {
        // A freshly created world has no finance history at all, so the
        // trailing sum is 0 for EVERY club. Read as a divisor that would
        // be "1", a Ligue 2 side with $30M in the bank and a $10M wage
        // bill is a fully state-backed benefactor; read honestly, nobody
        // knows anything about it yet.
        assert_eq!(
            ClubBenefactor::signal(30_000_000, 10_000_000, 0),
            0.0,
            "a missing denominator is not a benefactor"
        );
        // …and a club with a pile and no wage bill at all says nothing
        // either: an owner shows up in what is SPENT, not in what is held.
        assert_eq!(ClubBenefactor::signal(250_000_000, 0, 40_000_000), 0.0);
    }

    #[test]
    fn a_wage_bill_the_revenue_cannot_carry_reads_as_an_owner() {
        // Al-Hilal: wages ≈ 2x revenue, and the balance covers the bill
        // twice over. The clearest benefactor in world football.
        let gulf = ClubBenefactor::signal(160_000_000, 80_000_000, 40_000_000);
        assert!(gulf > 0.9, "{gulf}");
        // A big European club with the SAME pile whose wages sit inside
        // its revenue is not being funded by anybody, however large the
        // numbers are.
        let european = ClubBenefactor::signal(500_000_000, 280_000_000, 400_000_000);
        assert_eq!(european, 0.0);
    }

    #[test]
    fn overspending_on_borrowed_money_is_distress_and_not_an_owner() {
        // Wages 1.3x revenue is a real overspend — but with the balance
        // gone it is a club in trouble, not a club with a backer.
        let distressed = ClubBenefactor::signal(-40_000_000, 91_000_000, 70_000_000);
        assert_eq!(distressed, 0.0);
        // The same bill with the cash to pay it reads as the owner it is.
        let backed = ClubBenefactor::signal(140_000_000, 91_000_000, 70_000_000);
        assert!(backed > 0.5, "{backed}");
    }

    #[test]
    fn the_reading_sits_between_the_two_real_bands() {
        // Zenit: wages ≈ 0.9x revenue, cash still there — a Malcom, not a
        // Neymar.
        let zenit = ClubBenefactor::signal(300_000_000, 72_000_000, 80_000_000);
        assert!((0.05..0.3).contains(&zenit), "{zenit}");
        // Inter Miami: a designated-player bill on MLS revenue, cash
        // positive.
        let miami = ClubBenefactor::signal(60_000_000, 45_000_000, 30_000_000);
        assert!(miami > 0.7, "{miami}");
    }

    #[test]
    fn the_habit_fades_rather_than_switching_off() {
        let mut b = 0.8_f32;
        for _ in 0..3 {
            b = ClubBenefactor::blend(b, 0.0);
        }
        assert!(
            (0.35..0.45).contains(&b),
            "half the habit after one half-life: {b}"
        );
        for _ in 0..9 {
            b = ClubBenefactor::blend(b, 0.0);
        }
        assert!(b < 0.06, "and gone after four: {b}");
    }

    #[test]
    fn the_subsidy_is_bounded_by_the_cash_that_is_actually_there() {
        let idle = ClubBenefactor::idle_cash(250_000_000, 60_000_000);
        let subsidy = ClubBenefactor::subsidy_per_year(0.8, 0.8, idle);
        // ≈ 0.8 x 0.8 x 160M x 0.35
        assert!((30_000_000.0..50_000_000.0).contains(&subsidy), "{subsidy}");
        assert_eq!(ClubBenefactor::subsidy_per_year(0.0, 1.0, idle), 0.0);
    }
}
