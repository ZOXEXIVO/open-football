//! The world as the plausibility model needs it.
//!
//! [`super`] is a pure model: it names no `Country`, no `Club` and no
//! `Player`, and decides everything from the numbers it is handed. This file
//! is where those numbers come from — assembling [`TransferPlausibilityInputs`]
//! out of the shapes the pipeline actually carries, whether that is a live
//! roster or a pre-built [`PlayerSummary`] from another country snapshot.
//!
//! Hydration lives apart from the model it feeds — [`stance`] and
//! [`fit`] are the same shape. Put the two in one file and nothing stops
//! a gate reaching for a club, because the club is already in scope.
//!
//! Keeping the builder methods inside a single struct means call sites stay
//! one-liners and the wage policy / importance heuristics never silently
//! drift between callers.
//!
//! [`stance`]: crate::transfers::gate::stance
//! [`fit`]: crate::transfers::gate::fit

use crate::club::mind::organs::memory::{ActorRef, FactClaim};
use crate::club::staff::DossierTuning;
use chrono::NaiveDate;

use super::{
    MoveTerms, TransferMoveAssessment, TransferMovePlausibility, TransferPlausibilityInputs,
};
use crate::club::player::calculators::WageCalculator;
use crate::transfers::market::route::TransferRoutePolicy;
use crate::transfers::pipeline::PlayerSummary;
use crate::transfers::squad::plan::BriefTier;
use crate::transfers::value::WagePower;
use crate::transfers::{
    ClubMarketKnowledge, MarketAffinity, MarketAffinityInputs, MarketMap, MoveKind,
};
use crate::{Club, Country, Player, PlayerFieldPositionGroup};
use rustc_hash::FxHashMap;
use std::cell::RefCell;

/// One account of a buying club, taken once for a pass: the division its
/// first team plays in, its standing, what it could hold for one shirt, the
/// fee it can reach and the markets it knows. Every route and every stage
/// reads the club through it, so no two of them can disagree about it.
pub(crate) struct Suitor<'a> {
    club: &'a Club,
    date: NaiveDate,
    pub country_id: u32,
    pub country_code: &'a str,
    pub league_id: Option<u32>,
    /// The first team's standing, 0..1.
    pub standing: f32,
    pub world_rep: i16,
    /// The division its first team plays in, 0 when it cannot be read.
    pub division: u16,
    pub transfer_budget: f64,
    pub wage_budget: u32,
    pub total_wages: u32,
    /// Annualised trailing income and the best-paid player — what a loan
    /// is priced against. A purchase reads neither.
    pub annual_income: i64,
    pub top_earner: u32,
    benefactor: f32,
    /// The board is in breach of its spending rules, so it spends half.
    ffp_breach: bool,
    /// The two halves of [`WagePower`] that do not depend on who is named.
    shirt_room: f64,
    owner_envelope: f64,
    /// The department's best coverage of each market, by country.
    scout_coverage: FxHashMap<u32, u8>,
    /// Reach memoised by `(nationality, country he plays in)`.
    reach: RefCell<FxHashMap<(u32, u32), f32>>,
}

impl<'a> Suitor<'a> {
    /// The floor under what a club knows of a market it reads: it may look
    /// at one it does not work, never as clearly as one it has people in.
    const KNOWLEDGE_FLOOR: f32 = 0.5;

    pub(crate) fn of(country: &'a Country, club: &'a Club, date: NaiveDate) -> Self {
        let first_team = club.teams.main().or_else(|| club.teams.teams.first());
        let league_id = first_team.and_then(|t| t.league_id);
        let division = league_id
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);
        let total_wages: u32 = club.teams.iter().map(|t| t.get_annual_salary()).sum();
        let wage_budget = club
            .finance
            .wage_budget
            .as_ref()
            .map(|b| b.amount.max(0.0) as u32)
            .unwrap_or(total_wages.saturating_mul(11) / 10);
        let transfer_budget = club
            .finance
            .transfer_budget
            .as_ref()
            .map(|b| b.amount)
            .unwrap_or(club.transfer_plan.total_budget);
        let unnamed = WagePower::for_player(club, 0.0, BriefTier::A, 0.0);
        let mut scout_coverage: FxHashMap<u32, u8> = FxHashMap::default();
        for staff in club.teams.iter().flat_map(|t| t.staffs.iter()) {
            for known in &staff.staff_attributes.knowledge.known_countries {
                let slot = scout_coverage.entry(known.country_id).or_insert(0);
                *slot = (*slot).max(known.level);
            }
        }
        Suitor {
            club,
            date,
            country_id: country.id,
            country_code: &country.code,
            league_id,
            standing: first_team
                .map(|t| t.reputation.overall_score())
                .unwrap_or(0.3),
            world_rep: first_team.map(|t| t.reputation.world as i16).unwrap_or(0),
            division,
            transfer_budget,
            wage_budget,
            total_wages,
            annual_income: club.finance.estimated_annual_income(date),
            top_earner: club
                .teams
                .iter()
                .flat_map(|t| t.players.iter())
                .filter_map(|p| p.contract.as_ref().map(|c| c.salary))
                .max()
                .unwrap_or(0),
            benefactor: club.board.ownership.benefactor,
            ffp_breach: club.finance.is_ffp_breach(date),
            shirt_room: unnamed.wage_headroom,
            owner_envelope: unnamed.owner_subsidy,
            scout_coverage,
            reach: RefCell::new(FxHashMap::default()),
        }
    }

    /// What the club may put toward a fee this window — its budget, halved
    /// while it is in breach of its spending rules. Every approach reads it.
    pub(crate) fn spend(&self) -> f64 {
        if self.ffp_breach {
            self.transfer_budget * 0.5
        } else {
            self.transfer_budget
        }
    }

    /// The wage this club's own level implies for him.
    pub(crate) fn level_wage_for(&self, target: &PlayerSummary) -> u32 {
        WageCalculator::expected_annual_wage_raw(
            target.skill_ability,
            target.current_reputation,
            matches!(target.position_group, PlayerFieldPositionGroup::Forward),
            matches!(target.position_group, PlayerFieldPositionGroup::Goalkeeper),
            target.age,
            self.standing,
            self.division,
        )
    }

    /// The most the club could hold for his shirt — [`WagePower`] at the
    /// level wage its standing implies for him.
    pub(crate) fn wage_ceiling_for(&self, target: &PlayerSummary) -> f64 {
        WagePower::of_parts(
            self.level_wage_for(target) as f64,
            self.shirt_room,
            self.owner_envelope,
        )
        .ceiling
    }

    /// Whether the club could hold `wage` for his shirt. The room and the
    /// owner's envelope answer most of these without his level wage.
    pub(crate) fn can_hold(&self, target: &PlayerSummary, wage: f64) -> bool {
        wage <= self.shirt_room.max(self.owner_envelope) || wage <= self.wage_ceiling_for(target)
    }

    /// How well the club is placed to reach him: the corridor into his
    /// market times what the club knows of it, 0..1. `1.0` at home, and in
    /// a world with no geography loaded.
    pub(crate) fn reach_to(&self, target: &PlayerSummary, market_map: &MarketMap) -> f32 {
        if target.country_id == self.country_id || market_map.is_silent() {
            return 1.0;
        }
        let key = (target.nationality_country_id, target.country_id);
        if let Some(cached) = self.reach.borrow().get(&key) {
            return *cached;
        }
        let reach = self.reach_for_pair(market_map, key);
        self.reach.borrow_mut().insert(key, reach);
        reach
    }

    /// The same read, addressed by `(nationality, country he plays in)`.
    fn reach_for_pair(&self, market_map: &MarketMap, key: (u32, u32)) -> f32 {
        let (nationality, plays_in) = key;
        let affinity = MarketAffinity::affinity(
            market_map,
            MarketAffinityInputs {
                buyer_country_id: self.country_id,
                nationality_country_id: nationality,
                current_country_id: plays_in,
                // Nothing here declares a move wage-led: the owner's money
                // does, continuously, inside the affinity.
                kind: MoveKind::Talent,
                benefactor: self.benefactor,
            },
        );
        let scout_level = |country_id: u32| self.scout_coverage.get(&country_id).copied();
        // A club with a Brazil man sees a Brazilian at Porto, and one with a
        // Portugal man sees him through the league he plays in.
        let best_scout = scout_level(plays_in)
            .unwrap_or(0)
            .max(scout_level(nationality).unwrap_or(0));
        let knowledge = [plays_in, nationality]
            .into_iter()
            .map(|source| {
                ClubMarketKnowledge::knowledge(
                    market_map,
                    self.country_id,
                    &self.club.market_ledger,
                    best_scout,
                    source,
                    self.date,
                )
            })
            .fold(0.0_f32, f32::max);
        (affinity * knowledge.max(Self::KNOWLEDGE_FLOOR)).clamp(0.0, 1.0)
    }
}

impl TransferMovePlausibility {
    /// How far this club's pursuit of this player can go. The one way any
    /// route asks: the club read through its [`Suitor`], the player through
    /// his summary, and what only the caller knows through the terms.
    pub(crate) fn read(
        suitor: &Suitor<'_>,
        target: &PlayerSummary,
        terms: MoveTerms,
        market_map: &MarketMap,
    ) -> TransferMoveAssessment {
        Self::read_at(suitor, target, terms, suitor.reach_to(target, market_map))
    }

    /// The same read, for a route that prices the market itself — the loan
    /// scan reads the loan corridor rather than the talent one — and hands
    /// its reach in.
    pub(crate) fn read_at(
        suitor: &Suitor<'_>,
        target: &PlayerSummary,
        terms: MoveTerms,
        market_reach: f32,
    ) -> TransferMoveAssessment {
        Self::assess(&MoveInputs::of(suitor, target, terms, market_reach))
    }
}

impl MoveTerms {
    /// The terms of a move whose caller holds the live player: whether his
    /// release clause is met, and what he makes of the man who would pick
    /// him.
    pub(crate) fn for_player(
        player: &Player,
        buying_club: &Club,
        date: NaiveDate,
        is_loan: bool,
        unsolicited: bool,
        fee: f64,
    ) -> Self {
        MoveTerms {
            is_loan,
            unsolicited,
            fee: Some(fee),
            release_clause_triggered: player
                .contract
                .as_ref()
                .is_some_and(|c| c.release_clause_triggered(0.0, false).is_some()),
            manager_affinity: PlayerManagerAffinity::of(player, buying_club, date),
        }
    }
}

/// The model's numbers, assembled in one place out of one account of the
/// club, one reading of the player and the caller's terms.
struct MoveInputs;

impl MoveInputs {
    fn of(
        suitor: &Suitor<'_>,
        target: &PlayerSummary,
        terms: MoveTerms,
        market_reach: f32,
    ) -> TransferPlausibilityInputs {
        let seller = &target.seller_ctx;
        let same_country = target.country_id == suitor.country_id;
        let same_league_or_division = same_country
            && match (suitor.league_id, seller.league_id) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            };
        TransferPlausibilityInputs {
            manager_affinity: terms.manager_affinity,
            buyer_rep: suitor.standing,
            seller_rep: seller.club_reputation_score,
            buyer_league_rep: suitor.division,
            seller_league_rep: seller.league_reputation,
            buyer_world_rep: suitor.world_rep,
            seller_world_rep: target.club_world_reputation,
            player_world_rep: target.world_reputation,
            player_current_rep: target.current_reputation,
            player_home_rep: target.home_reputation,
            player_age: target.age,
            position_group: target.position_group,
            is_listed: target.is_listed,
            is_loan_listed: target.is_loan_listed,
            is_transfer_requested: seller.is_transfer_requested,
            is_unhappy: seller.is_unhappy,
            squad_status: seller.squad_status.clone(),
            contract_months_remaining: target.contract_months_remaining,
            current_salary: target.salary,
            estimated_value: terms.fee.unwrap_or(target.estimated_value),
            player_appearances: target.appearances,
            seller_club_matches: seller.club_matches_played,
            seller_position_rank: seller.position_group_rank,
            player_ca: target.skill_ability,
            best_group_ca_at_seller: target.club_best_in_group.max(target.skill_ability),
            is_loan: terms.is_loan,
            is_unsolicited: terms.unsolicited,
            seller_in_debt: seller.in_debt,
            release_clause_triggered: terms.release_clause_triggered,
            listing_resignation: seller.market_resignation,
            player_plan: target.career_plan,
            anchors: target.anchors,
            international_apps: target.international_apps,
            same_country,
            same_league_or_division,
            country_pair_blocked: TransferRoutePolicy::is_blocked(
                &target.country_code,
                suitor.country_code,
                suitor.date,
            ),
            buyer_transfer_budget: suitor.transfer_budget,
            buyer_wage_budget: suitor.wage_budget,
            buyer_total_wages: suitor.total_wages,
            expected_annual_wage: suitor.level_wage_for(target),
            player_stage_inclination: seller.big_stage_inclination,
            // The reach is already the product of the corridor and what the
            // club knows of it, so it rides the affinity term whole.
            market_affinity: market_reach,
            buyer_market_knowledge: 1.0,
            seller_marketed: seller.is_marketed,
            buyer_annual_income: suitor.annual_income,
            buyer_top_earner: suitor.top_earner,
            parent_subsidy: target.parent_subsidy,
            pathway_stage: target.pathway_stage,
            level_floor: target.level_floor,
            asking_wage: target.asking_wage,
            refusal_price: target.refusal_price,
            wage_ceiling: suitor.wage_ceiling_for(target),
        }
    }
}

/// How a player feels about the man who would be picking him.
///
/// Read from the *player's* side — his standing with the coach and the
/// convictions he has formed about him — rather than from the coach's
/// dossier. The two are allowed to disagree and frequently should: a
/// manager who remembers moving a squad player on remembers it as routine,
/// and the player remembers being moved on.
pub(in crate::transfers) struct PlayerManagerAffinity;

impl PlayerManagerAffinity {
    /// −1..=1, and exactly zero for the overwhelming majority of moves,
    /// where the two of them have never met.
    pub(in crate::transfers) fn of(player: &Player, buying_club: &Club, date: NaiveDate) -> f32 {
        let Some(coach) = buying_club
            .teams
            .main()
            .map(|team| team.staffs.head_coach())
        else {
            return 0.0;
        };
        if coach.id == 0 {
            return 0.0;
        }
        let manager = ActorRef::staff(coach.id);
        let ctx = player.mind_context(date, buying_club.id.into());
        let standing = player.mind.standing_with(manager, &ctx);
        if standing == 0.0
            && player.relations.get_staff(coach.id).is_none()
            && player.rapport.score(coach.id) == 0
        {
            return 0.0;
        }

        (standing
            + player.mind.believes(FactClaim::MadeMeAPlayer, manager)
                * DossierTuning::PLAYER_AFFINITY_W_MADE
            + player.mind.believes(FactClaim::HeBackedMe, manager)
                * DossierTuning::PLAYER_AFFINITY_W_BACKED
            + player.mind.believes(FactClaim::WeClashed, manager)
                * DossierTuning::PLAYER_AFFINITY_W_CLASHED
            + player.mind.believes(FactClaim::NeverTrustedMe, manager)
                * DossierTuning::PLAYER_AFFINITY_W_NEVER_TRUSTED
            + player.mind.believes(FactClaim::HisWordIsWorthless, manager)
                * DossierTuning::PLAYER_AFFINITY_W_WORD)
            .clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfers::ScoutingRegion;
    use crate::transfers::market::map::{
        CorridorWeight, CountryTransferProfile, MarketCountryFacts,
    };
    use crate::transfers::tests::kit::{TestClub, TestCountry, TestDate, TestSummary, TestTeam};
    use crate::{PlayerSquadStatus, TeamType};

    struct Fx;

    impl Fx {
        const BR: u32 = 1;
        const PT: u32 = 2;
        const ES: u32 = 3;

        fn day() -> NaiveDate {
            TestDate::on(2026, 8, 1)
        }

        /// A Spanish club with one main team in a 6 200 division.
        fn spain(clubs: Vec<Club>) -> Country {
            TestCountry::new(Self::ES)
                .code("es")
                .league_reputation(6_200)
                .clubs(clubs)
                .build()
        }

        fn facts(id: u32, code: &str, continent: u32, top: u16) -> MarketCountryFacts {
            MarketCountryFacts {
                id,
                code: code.to_string(),
                continent_id: continent,
                region: ScoutingRegion::from_country(continent, code),
                reputation: top,
                top_flight_reputation: top,
                median_top_flight_wage: 500_000,
            }
        }

        fn weight(country_id: u32, weight: f32) -> CorridorWeight {
            CorridorWeight {
                country_id,
                weight,
                money: false,
            }
        }

        /// Portugal imports Brazilians heavily; Spain buys Brazilians and
        /// hardly any Portuguese nationals.
        fn world() -> MarketMap {
            let mut facts = FxHashMap::default();
            for f in [
                Self::facts(Self::BR, "br", 3, 7800),
                Self::facts(Self::PT, "pt", 1, 7500),
                Self::facts(Self::ES, "es", 1, 9200),
            ] {
                facts.insert(f.id, f);
            }
            let mut profiles = FxHashMap::default();
            profiles.insert(
                Self::ES,
                CountryTransferProfile {
                    import: vec![Self::weight(Self::BR, 1.0), Self::weight(Self::PT, 0.05)],
                    ..Default::default()
                },
            );
            profiles.insert(
                Self::BR,
                CountryTransferProfile {
                    export: vec![Self::weight(Self::ES, 1.0), Self::weight(Self::PT, 1.0)],
                    ..Default::default()
                },
            );
            profiles.insert(
                Self::PT,
                CountryTransferProfile {
                    import: vec![Self::weight(Self::BR, 1.0)],
                    export: vec![Self::weight(Self::ES, 0.05)],
                    ..Default::default()
                },
            );
            MarketMap::new(profiles, facts)
        }
    }

    #[test]
    fn a_club_is_read_by_its_main_squad_not_the_first_one_listed() {
        let reserves = TestTeam::new(11)
            .club_id(1)
            .team_type(TeamType::B)
            .league_id(None)
            .reputation(1_500)
            .build();
        let main = TestTeam::new(10)
            .club_id(1)
            .team_type(TeamType::Main)
            .reputation(7_000)
            .build();
        let country = Fx::spain(vec![TestClub::new(1).teams(vec![reserves, main]).build()]);
        let club = &country.clubs[0];
        let suitor = Suitor::of(&country, club, Fx::day());
        let first_team = club.teams.main().unwrap();
        assert_eq!(suitor.standing, first_team.reputation.overall_score());
        assert_eq!(suitor.world_rep, first_team.reputation.world as i16);
        assert_eq!(suitor.division, 6_200);
        assert_eq!(suitor.league_id, Some(1));
    }

    #[test]
    fn a_club_with_no_budget_on_its_books_reads_its_plan() {
        let mut club = TestClub::new(1).build();
        club.finance.transfer_budget = None;
        club.transfer_plan.total_budget = 3_000_000.0;
        let country = Fx::spain(vec![club]);
        let suitor = Suitor::of(&country, &country.clubs[0], Fx::day());
        assert_eq!(suitor.transfer_budget, 3_000_000.0);
        assert_eq!(
            suitor.spend(),
            3_000_000.0,
            "every approach reads the same budget"
        );
    }

    #[test]
    fn the_shirt_ceiling_is_wage_power_at_the_level_wage() {
        let country = Fx::spain(vec![TestClub::new(1).reputation(5_000).build()]);
        let club = &country.clubs[0];
        let suitor = Suitor::of(&country, club, Fx::day());
        for target in [
            TestSummary::new(7).ability(80).build(),
            TestSummary::new(8).ability(160).reputation(7_000).build(),
        ] {
            let level_wage = suitor.level_wage_for(&target) as f64;
            assert_eq!(
                suitor.wage_ceiling_for(&target),
                WagePower::for_player(club, level_wage, BriefTier::A, 0.0).ceiling
            );
        }
    }

    /// A Brazilian at Porto and a Portuguese at Porto are two different
    /// market questions, and the order they are asked in changes neither.
    #[test]
    fn two_passports_at_one_club_are_two_different_markets() {
        let map = Fx::world();
        let country = Fx::spain(vec![TestClub::new(1).build()]);
        let club = &country.clubs[0];
        assert!(club.market_ledger.entries().is_empty());
        let at_porto = |id: u32, passport: u32| {
            TestSummary::new(id)
                .country(Fx::PT, "pt")
                .nationality(passport)
                .build()
        };
        let brazilian = at_porto(1, Fx::BR);
        let portuguese = at_porto(2, Fx::PT);

        let suitor = Suitor::of(&country, club, Fx::day());
        let b = suitor.reach_to(&brazilian, &map);
        let p = suitor.reach_to(&portuguese, &map);
        assert!(
            b > p * 1.5,
            "a Brazilian at Porto ({b}) must read differently from a Portuguese at Porto ({p})"
        );

        let fresh = Suitor::of(&country, club, Fx::day());
        assert_eq!(fresh.reach_to(&portuguese, &map), p);
        assert_eq!(fresh.reach_to(&brazilian, &map), b);
    }

    #[test]
    fn a_world_with_no_geography_reads_every_market_as_open() {
        let country = Fx::spain(vec![TestClub::new(1).build()]);
        let suitor = Suitor::of(&country, &country.clubs[0], Fx::day());
        let abroad = TestSummary::new(1)
            .country(Fx::PT, "pt")
            .nationality(Fx::BR)
            .build();
        assert_eq!(suitor.reach_to(&abroad, &MarketMap::default()), 1.0);
    }

    /// One entry, one reading: whichever route asks, the same club and the
    /// same player give the same stage and the same reason.
    #[test]
    fn the_same_pair_reads_the_same_on_every_route() {
        let map = Fx::world();
        let country = Fx::spain(vec![TestClub::new(1).reputation(3_000).build()]);
        let target = TestSummary::new(9)
            .country(Fx::PT, "pt")
            .nationality(Fx::PT)
            .ability(150)
            .reputation(7_000)
            .league(7_500, 0.8)
            .squad_status(PlayerSquadStatus::KeyPlayer)
            .rank(0)
            .build();
        let readings: Vec<TransferMoveAssessment> = (0..3)
            .map(|_| {
                let suitor = Suitor::of(&country, &country.clubs[0], Fx::day());
                TransferMovePlausibility::read(&suitor, &target, MoveTerms::purchase(true), &map)
            })
            .collect();
        for reading in &readings[1..] {
            assert_eq!(reading.stage, readings[0].stage);
            assert_eq!(reading.blocking_reason, readings[0].blocking_reason);
        }
    }
}
