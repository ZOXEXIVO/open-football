//! The owner who is funding this club out of his own pocket.
//!
//! Some clubs hold cash that their revenue cannot explain. Saudi clubs sit
//! on five of the twelve largest balances in the world at a league
//! reputation of 5200; Zenit and Krasnodar hold nine figures at 6500;
//! Inter Miami holds $111M at 6200. In every one of those cases the money
//! did not come from gate receipts, and it is why those clubs can pay a
//! Premier League wage in a league that could never earn one.
//!
//! The simulator used to model none of that. A club's wage budget was
//! `projected_income × ratio`, its transfer ceiling was a country constant,
//! and its bank balance was unreachable — so the richest clubs outside
//! Europe offered a Premier League star *half* his wage and he refused.
//!
//! [`ClubBenefactor`] is the missing signal, and it is deliberately ONE
//! ratio: cash beyond a prudent wage cover, divided by a year's income. No
//! flag, no league list, no country table. The Gulf, China, Russia and MLS
//! all fall out of it, and they all disappear when the ratio normalises —
//! which is exactly what happened to the Chinese Super League after the
//! import tax (memory `feedback_balance_system_not_cases`).

/// How much of this club's spending its owner, rather than its revenue, is
/// paying for.
pub struct ClubBenefactor;

impl ClubBenefactor {
    /// Years of wage bill a club is assumed to want in the bank before any
    /// of its cash reads as idle. Matches the transfer-ceiling model's
    /// `WAGE_COVER_YEARS` so the two never disagree about what "spare cash"
    /// means.
    pub const WAGE_COVER_YEARS: f64 = 1.5;

    /// Idle cash worth this many years of income is where an owner's
    /// funding starts to show. Below it, a healthy club with reserves looks
    /// exactly like a healthy club with reserves.
    pub const SIGNAL_FLOOR: f64 = 1.5;

    /// Idle cash worth this many years of income ABOVE the floor is a fully
    /// owner-funded club. Al-Hilal's ≈ $250M idle against ≈ $40M income
    /// lands near the top of the band; Zenit's ≈ $230M against ≈ $80M lands
    /// near the bottom.
    pub const SIGNAL_SPAN: f64 = 6.0;

    /// Share of idle cash an owner will convert into wages in a year.
    /// Bounded so a benefactor club cannot strip the elite band in one
    /// window (Part VIII, "the drain").
    pub const SUBSIDY_SHARE: f64 = 0.35;

    /// Half-life of the benefactor read, in seasons. A club that spends its
    /// pile keeps its owner's habit for a while; a club whose owner leaves
    /// loses it slowly rather than overnight.
    pub const HALF_LIFE_SEASONS: f32 = 3.0;

    /// Cash beyond wage cover that revenue cannot explain, 0..1.
    ///
    /// `income` is the sim's revenue-derived trailing figure, not the real
    /// club's turnover.
    ///
    /// **Fails closed on no evidence.** The ratio is `idle ÷ income`, so a
    /// missing denominator is not "infinitely owner-funded", it is "we do
    /// not know yet". The original formula clamped income to `max(1)` and
    /// read every cash-positive club in a freshly created world — where
    /// the finance history is empty and the trailing sum is 0 — as a fully
    /// state-backed benefactor, at any reputation, for four seasons of EMA
    /// decay. A Ligue 2 side with $30M in the bank is not Al-Hilal.
    /// Callers with no trailing year feed a PROJECTION instead
    /// ([`crate::club::finance::RevenueModel::projected_annual`]) so the
    /// clubs that genuinely are owner-funded read as such on day one, from
    /// data rather than from a missing divisor.
    pub fn signal(balance: i64, annual_wages: i64, annual_income: i64) -> f32 {
        if annual_income <= 0 {
            return 0.0;
        }
        let idle = Self::idle_cash(balance, annual_wages);
        let income = annual_income as f64;
        (((idle / income) - Self::SIGNAL_FLOOR) / Self::SIGNAL_SPAN).clamp(0.0, 1.0) as f32
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
    pub fn subsidy_per_year(benefactor: f32, injection_appetite: f32, idle_cash: f64) -> f64 {
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
        assert_eq!(ClubBenefactor::signal(250_000_000, 0, 0), 0.0);
    }

    #[test]
    fn cash_that_revenue_cannot_explain_reads_as_an_owner() {
        // Order of magnitude from the P0 census: idle ≈ $250M against
        // income ≈ $40M.
        let gulf = ClubBenefactor::signal(250_000_000, 0, 40_000_000);
        assert!(gulf > 0.7, "{gulf}");
        // A big European club with the SAME pile but four times the income
        // is not being funded by anybody.
        let european = ClubBenefactor::signal(250_000_000, 0, 400_000_000);
        assert!(european < 0.05, "{european}");
    }

    #[test]
    fn the_reading_sits_between_the_two_real_bands() {
        // Zenit: idle ≈ $230M against ≈ $80M income — a Malcom, not a
        // Neymar.
        let zenit = ClubBenefactor::signal(230_000_000, 0, 80_000_000);
        assert!((0.1..0.4).contains(&zenit), "{zenit}");
        // Inter Miami: ≈ $90M against ≈ $30M — one designated-player shirt.
        let miami = ClubBenefactor::signal(90_000_000, 0, 30_000_000);
        assert!((0.1..0.4).contains(&miami), "{miami}");
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
