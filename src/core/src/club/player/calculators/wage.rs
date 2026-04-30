use crate::PlayerSquadStatus;
use crate::club::player::player::Player;

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
        let pos = player.position();
        Self::expected_annual_wage_raw(
            player.player_attributes.current_ability,
            player.player_attributes.current_reputation,
            pos.is_forward(),
            pos.is_goalkeeper(),
            age,
            club_reputation_score,
            league_reputation,
        )
    }

    /// Same wage formula as [`expected_annual_wage`] but takes raw inputs
    /// instead of a fully-built `Player`. Lets the database hydrator fill in
    /// a sensible salary when an ODB record omits one — at that point the
    /// `Player` doesn't exist yet, but CA / reputation / position are all
    /// readable on the record.
    pub fn expected_annual_wage_raw(
        current_ability: u8,
        current_reputation: i16,
        is_forward: bool,
        is_goalkeeper: bool,
        age: u8,
        club_reputation_score: f32,
        league_reputation: u16,
    ) -> u32 {
        let ability = current_ability as f64;
        let n = ability / 200.0;
        let base = 15_000.0 + 5_000_000.0 * n * n * n;

        let league_norm = (league_reputation as f64 / 10000.0).clamp(0.0, 1.0);
        let league_factor = 0.30 + 1.30 * league_norm;

        let club_factor = 0.70 + (club_reputation_score as f64).clamp(0.0, 1.0) * 0.60;

        let age_factor = age_factor(age);

        let position_factor = if is_forward {
            1.15
        } else if is_goalkeeper {
            0.85
        } else {
            1.0
        };

        let rep = current_reputation as f64;
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

    /// Loan split with awareness of the borrower's appetite. Bigger /
    /// hungrier borrowers cover more of the wage and pay a beefier match
    /// fee; small clubs developing a parent loanee cover less and earn the
    /// top-up. `borrower_score` and `parent_desire_to_develop` are 0..1.
    pub fn loan_wage_split_v2(
        parent_annual_wage: u32,
        borrower_score: f32,
        parent_desire_to_develop: f32,
    ) -> (u32, u32) {
        let parent = parent_annual_wage.max(1_000) as f64;
        // Borrower share scales 35-80% of parent wage.
        let share = 0.35 + (borrower_score.clamp(0.0, 1.0) as f64) * 0.45;
        // Development-focused parents accept absorbing more wage.
        let share =
            (share - (parent_desire_to_develop.clamp(0.0, 1.0) as f64) * 0.10).clamp(0.30, 0.85);
        let borrower = (parent * share).max(2_400.0) as u32;
        // Match fee climbs steeply with borrower size — Premier-League
        // borrowers pay enough that benching the loanee actually hurts.
        let match_fee_pct = 0.005 + (borrower_score.clamp(0.0, 1.0) as f64) * 0.012;
        let match_fee = (parent * match_fee_pct).max(150.0) as u32;
        (borrower, match_fee)
    }
}

/// Age curve used by both the buying-club wage estimate and the unified
/// `ContractValuation`. Peak 25-30, steep fall after 32, discount under 22.
fn age_factor(age: u8) -> f64 {
    match age {
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
    }
}

/// Squad-status premium applied on top of the open-market wage. Real
/// clubs pay key players a premium even at the same ability; backups
/// accept a discount. NotNeeded gets a flat low-end factor — these
/// players keep their existing deal at most.
pub fn squad_status_wage_factor(status: &PlayerSquadStatus) -> f32 {
    match status {
        PlayerSquadStatus::KeyPlayer => 1.45,
        PlayerSquadStatus::FirstTeamRegular => 1.15,
        PlayerSquadStatus::FirstTeamSquadRotation => 0.90,
        PlayerSquadStatus::MainBackupPlayer => 0.70,
        PlayerSquadStatus::HotProspectForTheFuture => 0.65,
        PlayerSquadStatus::DecentYoungster => 0.55,
        PlayerSquadStatus::NotNeeded => 0.55,
        PlayerSquadStatus::Invalid
        | PlayerSquadStatus::NotYetSet
        | PlayerSquadStatus::SquadStatusCount => 1.00,
    }
}

/// Inputs to the unified contract valuation. Optional fields default to
/// neutral values when the caller doesn't have them — happiness, for
/// example, doesn't care about market interest, while the renewal AI does.
#[derive(Debug, Clone)]
pub struct ValuationContext {
    pub age: u8,
    /// 0..1; team reputation overall_score().
    pub club_reputation_score: f32,
    /// 0..10000; league reputation.
    pub league_reputation: u16,
    pub squad_status: PlayerSquadStatus,
    /// Player's current annual salary (0 if no contract).
    pub current_salary: u32,
    /// Months until contract expiry; <0 already expired.
    pub months_remaining: i32,
    /// True if another club is publicly chasing the player (Wnt/Enq/Bid).
    pub has_market_interest: bool,
}

impl ValuationContext {
    pub fn happiness_default(
        player: &Player,
        age: u8,
        squad_status: PlayerSquadStatus,
        club_reputation_score: f32,
        league_reputation: u16,
        months_remaining: i32,
    ) -> Self {
        let current_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
        Self {
            age,
            club_reputation_score,
            league_reputation,
            squad_status,
            current_salary,
            months_remaining,
            has_market_interest: false,
        }
    }
}

/// Output of the unified valuation. Same model is used by:
///   - renewals (target = expected_wage, club caps at max_acceptable),
///   - personal terms (player rejects below min_acceptable),
///   - salary happiness (happy when current_salary >= expected_wage).
#[derive(Debug, Clone, Copy)]
pub struct ContractValuation {
    /// What the player would expect to earn under fair terms.
    pub expected_wage: u32,
    /// Floor — below this the player walks away.
    pub min_acceptable: u32,
    /// Ceiling — above this the club is overpaying.
    pub max_acceptable: u32,
    /// Player leverage 0..1. Higher = stronger negotiating position
    /// (expiring contract, market interest, irreplaceable status).
    pub leverage: f32,
    /// Status-premium factor applied to the open-market base.
    pub status_premium: f32,
}

impl ContractValuation {
    /// Build the full valuation. Happiness/renewal both go through this so
    /// they can't drift apart on the wage curve.
    pub fn evaluate(player: &Player, ctx: &ValuationContext) -> Self {
        let market = WageCalculator::expected_annual_wage(
            player,
            ctx.age,
            ctx.club_reputation_score,
            ctx.league_reputation,
        ) as f32;

        let status_premium = squad_status_wage_factor(&ctx.squad_status);

        let agent = crate::club::player::agent::PlayerAgent::for_player(player);
        // Greedy agents push the band up by ~10% at the high end; loyal
        // agents accept ~5% lower at the low end.
        let agent_premium = 1.0 + (agent.greed - 0.5) * 0.15 - (agent.loyalty - 0.5) * 0.05;

        let leverage = compute_leverage(player, ctx, &agent);

        let expected = market * status_premium * agent_premium;
        // The acceptable band widens with leverage. A player with no
        // leverage signs anything within ±15%; an out-of-contract star
        // with multiple suitors holds out for a much narrower range.
        let band_low = 0.30 - leverage * 0.10;
        let band_high = 0.20 + leverage * 0.30;
        let min_acceptable = (expected * (1.0 - band_low)).max(1.0) as u32;
        let max_acceptable = (expected * (1.0 + band_high)) as u32;

        Self {
            expected_wage: expected as u32,
            min_acceptable,
            max_acceptable,
            leverage,
            status_premium,
        }
    }

    /// Convenience: the wage the renewal AI should anchor to. Same as
    /// `expected_wage` — kept as a named accessor so call-sites read
    /// intentfully ("renewal target" vs raw "expected").
    pub fn renewal_target(player: &Player, ctx: &ValuationContext) -> u32 {
        Self::evaluate(player, ctx).expected_wage
    }

    /// Convenience: the wage the salary-happiness model treats as fair.
    pub fn happiness_expected(player: &Player, ctx: &ValuationContext) -> u32 {
        Self::evaluate(player, ctx).expected_wage
    }
}

fn compute_leverage(
    player: &Player,
    ctx: &ValuationContext,
    agent: &crate::club::player::agent::PlayerAgent,
) -> f32 {
    let mut score = 0.0_f32;

    // Months remaining — out-of-contract is maximum leverage; >24mo is
    // almost none.
    score += if ctx.months_remaining <= 0 {
        0.55
    } else if ctx.months_remaining <= 6 {
        0.45
    } else if ctx.months_remaining <= 12 {
        0.30
    } else if ctx.months_remaining <= 18 {
        0.15
    } else {
        0.05
    };

    if ctx.has_market_interest {
        score += 0.20;
    }

    // Status that can't easily be replaced
    score += match ctx.squad_status {
        PlayerSquadStatus::KeyPlayer => 0.20,
        PlayerSquadStatus::FirstTeamRegular => 0.10,
        PlayerSquadStatus::HotProspectForTheFuture => 0.10,
        _ => 0.0,
    };

    // Reputation premium
    let rep = player.player_attributes.current_reputation as f32;
    if rep > 5000.0 {
        score += 0.15;
    } else if rep > 2000.0 {
        score += 0.08;
    } else if rep > 1000.0 {
        score += 0.04;
    }

    // Greedy agent / ambitious player — they think they have leverage
    score += (agent.greed - 0.5) * 0.10;

    score.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::player::Player;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, PlayerSquadStatus,
    };
    use chrono::NaiveDate;

    fn ctx(
        age: u8,
        status: PlayerSquadStatus,
        club_rep: f32,
        league_rep: u16,
        months: i32,
        interest: bool,
    ) -> ValuationContext {
        ValuationContext {
            age,
            club_reputation_score: club_rep,
            league_reputation: league_rep,
            squad_status: status,
            current_salary: 0,
            months_remaining: months,
            has_market_interest: interest,
        }
    }

    fn person(ambition: f32, loyalty: f32) -> PersonAttributes {
        PersonAttributes {
            adaptability: 10.0,
            ambition,
            controversy: 10.0,
            loyalty,
            pressure: 10.0,
            professionalism: 10.0,
            sportsmanship: 10.0,
            temperament: 10.0,
            consistency: 10.0,
            important_matches: 10.0,
            dirtiness: 10.0,
        }
    }

    fn player_with(ability: u8, age: u8, ambition: f32, loyalty: f32) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ability;
        attrs.potential_ability = ability;
        attrs.current_reputation = (ability as i16) * 30;
        attrs.world_reputation = (ability as i16) * 25;
        attrs.home_reputation = (ability as i16) * 35;

        // Birth date 2026-04-26 minus age years (rough — leap days don't matter).
        let today = NaiveDate::from_ymd_opt(2026, 4, 26).unwrap();
        let birth = today
            .checked_sub_signed(chrono::Duration::days(age as i64 * 365))
            .unwrap();

        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".to_string(), "Player".to_string()))
            .birth_date(birth)
            .country_id(1)
            .attributes(person(ambition, loyalty))
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 20,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    #[test]
    fn key_player_premium_dominates_backup_at_same_ability() {
        let p = player_with(120, 26, 12.0, 12.0);
        let key = ContractValuation::evaluate(
            &p,
            &ctx(26, PlayerSquadStatus::KeyPlayer, 0.6, 6000, 24, false),
        );
        let backup = ContractValuation::evaluate(
            &p,
            &ctx(
                26,
                PlayerSquadStatus::MainBackupPlayer,
                0.6,
                6000,
                24,
                false,
            ),
        );
        assert!(
            key.expected_wage > backup.expected_wage * 2,
            "key {} should be at least 2× backup {}",
            key.expected_wage,
            backup.expected_wage
        );
    }

    #[test]
    fn expiring_contract_increases_leverage() {
        let p = player_with(120, 26, 12.0, 12.0);
        let two_year = ContractValuation::evaluate(
            &p,
            &ctx(
                26,
                PlayerSquadStatus::FirstTeamRegular,
                0.6,
                6000,
                24,
                false,
            ),
        );
        let final_six = ContractValuation::evaluate(
            &p,
            &ctx(26, PlayerSquadStatus::FirstTeamRegular, 0.6, 6000, 6, false),
        );
        assert!(final_six.leverage > two_year.leverage);
    }

    #[test]
    fn market_interest_increases_leverage() {
        let p = player_with(120, 26, 12.0, 12.0);
        let no_interest = ContractValuation::evaluate(
            &p,
            &ctx(
                26,
                PlayerSquadStatus::FirstTeamRegular,
                0.6,
                6000,
                12,
                false,
            ),
        );
        let with_interest = ContractValuation::evaluate(
            &p,
            &ctx(26, PlayerSquadStatus::FirstTeamRegular, 0.6, 6000, 12, true),
        );
        assert!(with_interest.leverage > no_interest.leverage);
    }

    #[test]
    fn higher_ability_pays_more() {
        let weak = player_with(60, 26, 10.0, 10.0);
        let strong = player_with(150, 26, 10.0, 10.0);
        let c = ctx(
            26,
            PlayerSquadStatus::FirstTeamRegular,
            0.6,
            6000,
            24,
            false,
        );
        let weak_wage = ContractValuation::evaluate(&weak, &c).expected_wage;
        let strong_wage = ContractValuation::evaluate(&strong, &c).expected_wage;
        assert!(strong_wage > weak_wage * 5);
    }

    #[test]
    fn elite_league_pays_more_than_minor_league() {
        let p = player_with(120, 26, 12.0, 12.0);
        let elite = ContractValuation::evaluate(
            &p,
            &ctx(
                26,
                PlayerSquadStatus::FirstTeamRegular,
                0.7,
                9000,
                24,
                false,
            ),
        );
        let minor = ContractValuation::evaluate(
            &p,
            &ctx(
                26,
                PlayerSquadStatus::FirstTeamRegular,
                0.4,
                2000,
                24,
                false,
            ),
        );
        assert!(elite.expected_wage > minor.expected_wage * 2);
    }

    #[test]
    fn loan_v2_split_scales_with_borrower_score() {
        let parent = 200_000u32;
        let (small_borrower_wage, _) = WageCalculator::loan_wage_split_v2(parent, 0.2, 0.0);
        let (big_borrower_wage, _) = WageCalculator::loan_wage_split_v2(parent, 0.9, 0.0);
        assert!(
            big_borrower_wage > small_borrower_wage,
            "big={big_borrower_wage} small={small_borrower_wage}"
        );
    }

    #[test]
    fn loan_v2_match_fee_climbs_with_borrower_size() {
        let parent = 200_000u32;
        let (_, small_fee) = WageCalculator::loan_wage_split_v2(parent, 0.2, 0.5);
        let (_, big_fee) = WageCalculator::loan_wage_split_v2(parent, 0.9, 0.5);
        assert!(big_fee > small_fee);
    }

    #[test]
    fn loan_v2_dev_focus_lowers_borrower_share() {
        let parent = 200_000u32;
        let (no_dev, _) = WageCalculator::loan_wage_split_v2(parent, 0.6, 0.0);
        let (dev_focused, _) = WageCalculator::loan_wage_split_v2(parent, 0.6, 1.0);
        assert!(
            no_dev >= dev_focused,
            "no_dev={no_dev} dev_focused={dev_focused}"
        );
    }

    #[test]
    fn old_player_earns_less_than_peak_player_same_status() {
        let young = player_with(120, 26, 10.0, 10.0);
        let old = player_with(120, 34, 10.0, 10.0);
        let c = ctx(
            26,
            PlayerSquadStatus::FirstTeamRegular,
            0.6,
            6000,
            24,
            false,
        );
        let c_old = ctx(
            34,
            PlayerSquadStatus::FirstTeamRegular,
            0.6,
            6000,
            24,
            false,
        );
        let young_wage = ContractValuation::evaluate(&young, &c).expected_wage;
        let old_wage = ContractValuation::evaluate(&old, &c_old).expected_wage;
        assert!(young_wage > old_wage);
    }
}
