use crate::{ContractType, Person, Player, PlayerSquadStatus};
use chrono::NaiveDate;

/// Club-side inputs for the automatic-release gate. Callers assemble it
/// once per decision from whatever context they have: the unresolved-salary
/// fallback reads real league reputation through `LeagueProcessAccess`,
/// while the season-start surplus trim (club scope, no league lookup)
/// substitutes the main team's world reputation when pricing the player.
pub struct ReleaseEligibilityContext {
    pub date: NaiveDate,
    /// Average current ability of the club's main squad — the "team level"
    /// the player is measured against.
    pub squad_avg_ability: u8,
    /// The player's market value as the caller's pricing model sees it
    /// (`Player::value` with the caller's reputation inputs).
    pub market_value: f64,
    /// Total annual wages across all of the club's teams. Scales both the
    /// compensation tolerance and the "worth selling instead" threshold so
    /// big clubs don't tear up sellable assets and tiny clubs can still
    /// move on from players nobody would buy.
    pub annual_wage_bill: u32,
}

/// Why an automatic mutual release was denied. Transfer-list-or-skip is
/// the caller's decision; the variant tells it (and the debug log) which
/// gate fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomaticReleaseBlock {
    /// Loanees belong to the parent club — recalled, never released here.
    OnLoan,
    /// Manager pinned the player into the match-day squad.
    ForceSelected,
    /// No contract to terminate — that player is on the plain-expiry path,
    /// which must keep recording `dec_reason_contract_expired`.
    NoContract,
    /// KeyPlayer / FirstTeamRegular: the squad plan says the club needs him.
    ProtectedRole,
    /// Not clearly below team level — a competitive player is kept,
    /// listed, or sold; never torn up.
    NearTeamLevel,
    /// The market would pay real money — list him instead of walking him.
    ValuableAsset,
    /// Severance exceeds what the club tolerates for a roster-clearing
    /// mutual termination.
    ExpensiveTermination,
}

/// Central gate for every *club-driven* early release that ends in the
/// free-agent sweep stamping `dec_reason_released_free`. Upstream systems
/// (positional surplus trim, unresolved-salary fallback) must pass this
/// before clearing a contract and adding `Frt`; anything blocked goes to
/// the transfer list or stays put. Plain contract expiry never consults
/// this gate — an expired deal costs nothing and isn't a club decision.
pub struct AutomaticReleaseEligibility;

impl AutomaticReleaseEligibility {
    /// A player this far below the main-squad average has no squad role
    /// at this club.
    const QUALITY_GAP: i16 = 25;
    /// Veterans get a softer quality gate: old and clearly declining is
    /// enough, they're not assets to resell.
    const VETERAN_AGE: u8 = 35;
    const VETERAN_QUALITY_GAP: i16 = 15;
    /// Floor on both money gates so semi-pro clubs with near-zero wage
    /// bills can still complete a routine release.
    const TERMINATION_ALLOWANCE_FLOOR: u32 = 5_000;
    /// No club, however rich, auto-pays a seven-figure settlement just to
    /// clear a roster slot — that decision belongs to a sale or a human.
    const FULL_TIME_TERMINATION_CEILING: u32 = 1_000_000;
    const MARKET_VALUE_FLOOR: f64 = 100_000.0;
    /// Above this, the player is a sellable asset at any club size.
    const MARKET_VALUE_CEILING: f64 = 2_000_000.0;

    /// `None` means every hard gate passed and the caller may clear the
    /// contract + stamp `Frt`; `Some(block)` names the first gate that
    /// failed (checked cheapest-first).
    pub fn assess(
        player: &Player,
        ctx: &ReleaseEligibilityContext,
    ) -> Option<AutomaticReleaseBlock> {
        if player.is_on_loan() {
            return Some(AutomaticReleaseBlock::OnLoan);
        }
        if player.is_force_match_selection {
            return Some(AutomaticReleaseBlock::ForceSelected);
        }
        let contract = match player.contract.as_ref() {
            Some(c) => c,
            None => return Some(AutomaticReleaseBlock::NoContract),
        };
        if matches!(
            contract.squad_status,
            PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
        ) {
            return Some(AutomaticReleaseBlock::ProtectedRole);
        }

        let ability = player.player_attributes.current_ability as i16;
        let avg = ctx.squad_avg_ability as i16;
        let age = player.age(ctx.date);
        let clearly_below = ability <= avg - Self::QUALITY_GAP;
        let old_and_declining =
            age >= Self::VETERAN_AGE && ability <= avg - Self::VETERAN_QUALITY_GAP;
        if !clearly_below && !old_and_declining {
            return Some(AutomaticReleaseBlock::NearTeamLevel);
        }

        if ctx.market_value > Self::market_value_cap(ctx.annual_wage_bill) {
            return Some(AutomaticReleaseBlock::ValuableAsset);
        }

        let cost = contract.termination_cost(ctx.date);
        if cost > Self::termination_cost_cap(ctx.annual_wage_bill, &contract.contract_type) {
            return Some(AutomaticReleaseBlock::ExpensiveTermination);
        }

        None
    }

    /// Convenience wrapper: did every hard gate pass?
    pub fn can_auto_release_on_free(player: &Player, ctx: &ReleaseEligibilityContext) -> bool {
        Self::assess(player, ctx).is_none()
    }

    /// A player the market would pay half a month of the club's total
    /// wage bill for is worth listing, not walking. Floor keeps the gate
    /// meaningful at tiny clubs; ceiling keeps rich clubs from writing
    /// off genuinely sellable players.
    fn market_value_cap(annual_wage_bill: u32) -> f64 {
        (annual_wage_bill as f64 / 24.0).clamp(Self::MARKET_VALUE_FLOOR, Self::MARKET_VALUE_CEILING)
    }

    /// Severance tolerance by contract type. Zero-cost deals (Amateur /
    /// NonContract, and anything expired — `termination_cost` already
    /// returns 0 for those) always pass. Youth / PartTime tear-ups are
    /// tolerated up to half a month of the club's wage bill — the same
    /// comfort threshold the manager-talks mutual-termination path uses.
    /// FullTime deals get a 4× stricter cap plus an absolute ceiling: a
    /// professional contract with real money left on it is a negotiation,
    /// not an automatic write-off.
    fn termination_cost_cap(annual_wage_bill: u32, contract_type: &ContractType) -> u32 {
        match contract_type {
            ContractType::Amateur | ContractType::NonContract | ContractType::Loan => 0,
            ContractType::Youth | ContractType::PartTime => {
                (annual_wage_bill / 24).max(Self::TERMINATION_ALLOWANCE_FLOOR)
            }
            ContractType::FullTime => (annual_wage_bill / 96).clamp(
                Self::TERMINATION_ALLOWANCE_FLOOR,
                Self::FULL_TIME_TERMINATION_CEILING,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };
    use chrono::{Datelike, Duration, NaiveDate};

    /// Fixtures for the eligibility gates. All scenarios share one squad
    /// context: avg ability 100 and a 1.2M annual wage bill, which puts
    /// the FullTime termination cap at 12.5K, the Youth/PartTime cap at
    /// 50K, and the market-value cap at the 100K floor.
    struct Fixture;

    impl Fixture {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 6, 12).unwrap()
        }

        fn ctx(market_value: f64) -> ReleaseEligibilityContext {
            ReleaseEligibilityContext {
                date: Self::date(),
                squad_avg_ability: 100,
                market_value,
                annual_wage_bill: 1_200_000,
            }
        }

        fn contract(
            salary: u32,
            contract_type: ContractType,
            months_remaining: u32,
        ) -> PlayerClubContract {
            let expiration = Self::date() + Duration::days(months_remaining as i64 * 30);
            let mut c = PlayerClubContract::new(salary, expiration);
            c.contract_type = contract_type;
            c.squad_status = PlayerSquadStatus::MainBackupPlayer;
            c
        }

        fn player(ability: u8, age: u8, contract: Option<PlayerClubContract>) -> Player {
            let birth_year = Self::date().year() - age as i32;
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ability;
            attrs.potential_ability = ability;
            PlayerBuilder::new()
                .id(1)
                .full_name(FullName::new("Test".to_string(), "Player".to_string()))
                .birth_date(NaiveDate::from_ymd_opt(birth_year, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 20,
                    }],
                })
                .player_attributes(attrs)
                .contract(contract)
                .build()
                .unwrap()
        }
    }

    #[test]
    fn cheap_fringe_player_is_eligible() {
        // CA 60 vs avg 100, low salary, 3 months left → severance ~1.9K.
        let player = Fixture::player(
            60,
            28,
            Some(Fixture::contract(15_000, ContractType::FullTime, 3)),
        );
        assert_eq!(
            AutomaticReleaseEligibility::assess(&player, &Fixture::ctx(20_000.0)),
            None
        );
        assert!(AutomaticReleaseEligibility::can_auto_release_on_free(
            &player,
            &Fixture::ctx(20_000.0)
        ));
    }

    #[test]
    fn old_declining_veteran_passes_softer_quality_gate() {
        // CA 85 vs avg 100 is only -15 — not enough for a 28-year-old,
        // enough for a 36-year-old.
        let young = Fixture::player(
            85,
            28,
            Some(Fixture::contract(15_000, ContractType::FullTime, 3)),
        );
        assert_eq!(
            AutomaticReleaseEligibility::assess(&young, &Fixture::ctx(20_000.0)),
            Some(AutomaticReleaseBlock::NearTeamLevel)
        );
        let veteran = Fixture::player(
            85,
            36,
            Some(Fixture::contract(15_000, ContractType::FullTime, 3)),
        );
        assert_eq!(
            AutomaticReleaseEligibility::assess(&veteran, &Fixture::ctx(20_000.0)),
            None
        );
    }

    #[test]
    fn loaned_player_is_blocked() {
        let mut player = Fixture::player(
            60,
            28,
            Some(Fixture::contract(15_000, ContractType::FullTime, 3)),
        );
        let mut loan = PlayerClubContract::new(15_000, Fixture::date());
        loan.loan_from_club_id = Some(999);
        player.contract_loan = Some(loan);
        assert_eq!(
            AutomaticReleaseEligibility::assess(&player, &Fixture::ctx(20_000.0)),
            Some(AutomaticReleaseBlock::OnLoan)
        );
    }

    #[test]
    fn force_selected_player_is_blocked() {
        let mut player = Fixture::player(
            60,
            28,
            Some(Fixture::contract(15_000, ContractType::FullTime, 3)),
        );
        player.is_force_match_selection = true;
        assert_eq!(
            AutomaticReleaseEligibility::assess(&player, &Fixture::ctx(20_000.0)),
            Some(AutomaticReleaseBlock::ForceSelected)
        );
    }

    #[test]
    fn contractless_player_is_blocked() {
        // No contract → plain-expiry path, never an automatic mutual release.
        let player = Fixture::player(60, 28, None);
        assert_eq!(
            AutomaticReleaseEligibility::assess(&player, &Fixture::ctx(20_000.0)),
            Some(AutomaticReleaseBlock::NoContract)
        );
    }

    #[test]
    fn protected_squad_role_is_blocked() {
        let mut contract = Fixture::contract(15_000, ContractType::FullTime, 3);
        contract.squad_status = PlayerSquadStatus::KeyPlayer;
        let player = Fixture::player(60, 28, Some(contract));
        assert_eq!(
            AutomaticReleaseEligibility::assess(&player, &Fixture::ctx(20_000.0)),
            Some(AutomaticReleaseBlock::ProtectedRole)
        );
    }

    #[test]
    fn valuable_player_is_blocked() {
        // Quality gates pass but the market would pay above the cap
        // (1.2M bill → 100K floor cap).
        let player = Fixture::player(
            60,
            28,
            Some(Fixture::contract(15_000, ContractType::FullTime, 3)),
        );
        assert_eq!(
            AutomaticReleaseEligibility::assess(&player, &Fixture::ctx(400_000.0)),
            Some(AutomaticReleaseBlock::ValuableAsset)
        );
    }

    #[test]
    fn expensive_full_time_termination_is_blocked() {
        // 600K salary, 18 months left → severance 18 × 50K × 0.5 = 450K,
        // far above the 12.5K FullTime cap at a 1.2M wage bill.
        let player = Fixture::player(
            60,
            28,
            Some(Fixture::contract(600_000, ContractType::FullTime, 18)),
        );
        assert_eq!(
            AutomaticReleaseEligibility::assess(&player, &Fixture::ctx(20_000.0)),
            Some(AutomaticReleaseBlock::ExpensiveTermination)
        );
    }

    #[test]
    fn zero_cost_contract_types_pass_the_cost_gate() {
        // Same salary/length that blocks a FullTime deal sails through on
        // Amateur / NonContract terms — termination_cost is 0 there.
        for contract_type in [ContractType::Amateur, ContractType::NonContract] {
            let player =
                Fixture::player(60, 28, Some(Fixture::contract(600_000, contract_type, 18)));
            assert_eq!(
                AutomaticReleaseEligibility::assess(&player, &Fixture::ctx(20_000.0)),
                None
            );
        }
    }

    #[test]
    fn youth_contract_tolerates_small_settlement_only() {
        // Youth settlement factor is 0.25: 60K salary, 12 months left →
        // 12 × 5K × 0.25 = 15K, under the 50K Youth cap (1.2M / 24).
        let cheap = Fixture::player(
            60,
            17,
            Some(Fixture::contract(60_000, ContractType::Youth, 12)),
        );
        assert_eq!(
            AutomaticReleaseEligibility::assess(&cheap, &Fixture::ctx(20_000.0)),
            None
        );
        // 600K salary youth deal → 150K settlement → blocked.
        let pricey = Fixture::player(
            60,
            17,
            Some(Fixture::contract(600_000, ContractType::Youth, 12)),
        );
        assert_eq!(
            AutomaticReleaseEligibility::assess(&pricey, &Fixture::ctx(20_000.0)),
            Some(AutomaticReleaseBlock::ExpensiveTermination)
        );
    }
}
