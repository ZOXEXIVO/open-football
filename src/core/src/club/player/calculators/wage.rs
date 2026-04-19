use crate::Player;

/// Wage expectations and club willingness-to-pay.
///
/// Drives three decisions:
///   - installed salary on a fresh permanent contract,
///   - the loan wage split (borrower base + match fee) vs parent top-up,
///   - the player's reservation wage during the PersonalTerms phase.
///
/// Output is an **annual** wage in USD — `PlayerClubContract.salary` stores annual.
pub struct WageCalculator;

impl WageCalculator {
    /// The player's expected annual wage at a buying club. Used both as
    /// "what the player asks for" (reservation wage) and "what the club
    /// is willing to pay" — the gap drives personal-terms acceptance.
    ///
    /// Signals:
    ///   - ability (cubic growth, tiny at amateur, big at elite),
    ///   - age curve (peak 25–30, steep fall after 32, discount under 22),
    ///   - league reputation (top-5 league premium ~2× vs mid-tier),
    ///   - club reputation (within a league, the elite club pays more),
    ///   - position (strikers +15%, keepers −15%),
    ///   - player reputation (star premium up to +25%).
    pub fn expected_annual_wage(
        player: &Player,
        age: u8,
        club_reputation_score: f32,
        league_reputation: u16,
    ) -> u32 {
        let ability = player.player_attributes.current_ability as f64;
        let n = ability / 200.0;
        let base = 15_000.0 + 5_000_000.0 * n * n * n;

        let league_norm = (league_reputation as f64 / 10000.0).clamp(0.0, 1.0);
        let league_factor = 0.30 + 1.30 * league_norm;

        let club_factor = 0.70 + (club_reputation_score as f64).clamp(0.0, 1.0) * 0.60;

        let age_factor = match age {
            0..=17 => 0.25,
            18 => 0.35,
            19 => 0.45,
            20 => 0.55,
            21 => 0.70,
            22 => 0.80,
            23 => 0.90,
            24 => 0.97,
            25..=28 => 1.00,
            29 => 0.95,
            30 => 0.88,
            31 => 0.80,
            32 => 0.70,
            33 => 0.58,
            34 => 0.45,
            _ => 0.35,
        };

        let pos = player.position();
        let position_factor = if pos.is_forward() {
            1.15
        } else if pos.is_goalkeeper() {
            0.85
        } else {
            1.0
        };

        let rep = player.player_attributes.current_reputation as f64;
        let rep_factor = if rep > 2000.0 {
            1.25
        } else if rep > 1000.0 {
            1.12
        } else if rep > 500.0 {
            1.03
        } else {
            0.97
        };

        (base * league_factor * club_factor * age_factor * position_factor * rep_factor) as u32
    }

    /// Loan wage split: (borrower_annual_wage, per_match_fee).
    /// Borrower covers the majority; parent keeps paying the rest so the
    /// player isn't worse off financially for going out.
    pub fn loan_wage_split(parent_annual_wage: u32) -> (u32, u32) {
        let parent = parent_annual_wage.max(1_000) as f64;
        let borrower = (parent * 0.55).max(2_400.0) as u32;
        let match_fee = (parent * 0.008).max(150.0) as u32;
        (borrower, match_fee)
    }
}
