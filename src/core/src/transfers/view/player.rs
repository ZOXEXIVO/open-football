//! What the market can read off a player, and how it projects him.
//!
//! Resolving a player out of the world is a question, not a decision: which
//! club holds him, what his name reads as, what the pipeline's own
//! [`PlayerSummary`] shape says about him. None of it consults a threshold,
//! so none of it belongs on the pipeline processor — the gates live in
//! [`crate::transfers::gate`] and the bands in [`crate::transfers::squad`].
//!
//! [`CountryPlayerLookup`] travels with them because it is the same
//! question asked many times with an index in front of it.

use crate::club::player::events::transfer_social::TransferInterestSignal;
use crate::club::staff::perception::AbilityEstimator;
use crate::{TransferInterestSource, TransferInterestStage};
use chrono::NaiveDate;
use rustc_hash::FxHashMap;
use std::cmp::Reverse;
use std::sync::OnceLock;

use crate::club::player::calculators::{ContractValuation, ValuationContext};
use crate::club::player::language::LanguageProfile;
use crate::club::player::mind::{GoalKind, MindClock, MindSituation};
use crate::club::player::statistics::StuckCareerScan;
use crate::club::player::transfer::{LevelFloor, StandingReading};
use crate::club::team::squad::SquadEvidenceContext;
use crate::transfers::ScoutingRegion;
use crate::transfers::gate::{
    AppraisalConfig, AvailabilityView, EffectivePlayerReputation, PlayerOfferAppraisal,
    PlayerStance, PlayerStanceBuilder, SquadEvidenceSource,
};
use crate::transfers::loan::guard::LoanAssetGuard;
use crate::transfers::loan::home::HomeLoanGates;
use crate::transfers::pipeline::LoanDestinationPreference;
use crate::transfers::pipeline::processor::{PlayerSummary, SellerPlausibilityContext};
use crate::transfers::squad::standing::CareerRecordSnapshot;
use crate::transfers::value::PlayerValuationCalculator;
use crate::transfers::view::club::ClubView;
use crate::{
    Club, Country, Person, Player, PlayerFieldPositionGroup, PlayerSquadStatus, PlayerStatusType,
    PositionCoverage, Team, TeamType,
};

/// Everything a summary reads that is NOT the player: the country's constants
/// and the club's, resolved once per club so every player of the club is
/// projected against the same numbers, whichever pass asks.
pub(crate) struct SummaryClubContext {
    price_level: f32,
    country_reputation: u16,
    country_region: ScoutingRegion,
    seller_league_rep: u16,
    seller_club_rep: u16,
    club_world_rep: i16,
    seller_club_rep_score: f32,
    seller_league_id: Option<u32>,
    seller_in_debt: bool,
    seller_club_matches: u16,
    group_ranks: ClubGroupRanks,
    home_preferences: Vec<u32>,
    stage_parent_loan: bool,
}

impl SummaryClubContext {
    /// `stage_parent_loan` stages each parent's view of lending him, which
    /// only the foreign loan scan reads off the world pool.
    pub(crate) fn of(
        country: &Country,
        club: &Club,
        date: NaiveDate,
        stage_parent_loan: bool,
    ) -> Self {
        let (seller_league_rep, seller_club_rep) =
            PlayerValuationCalculator::seller_context(country, club);
        let main_team = club.teams.main();
        SummaryClubContext {
            price_level: country.settings.pricing.price_level,
            country_reputation: country.reputation,
            country_region: ScoutingRegion::from_country(country.continent_id, &country.code),
            seller_league_rep,
            seller_club_rep,
            club_world_rep: ClubView::club_world_reputation(club),
            seller_club_rep_score: main_team
                .map(|t| t.reputation.overall_score())
                .unwrap_or(0.3),
            seller_league_id: main_team.and_then(|t| t.league_id),
            seller_in_debt: club.finance.balance.balance < 0,
            seller_club_matches: SquadEvidenceContext::current_season_sample(date, club)
                .club_matches_proxy(),
            group_ranks: ClubGroupRanks::build(club),
            // Loan-out candidates the parent has already decided WHERE it
            // would send. That decision is itself a posting, so a candidate
            // identified this tick reaches the world in the same pass.
            home_preferences: club
                .transfer_plan
                .loan_out_candidates
                .iter()
                .filter(|c| c.preferred_destination != LoanDestinationPreference::Any)
                .map(|c| c.player_id)
                .collect(),
            stage_parent_loan,
        }
    }

    pub(in crate::transfers) fn ranks(&self) -> &ClubGroupRanks {
        &self.group_ranks
    }
}

/// The one projection of a player onto the market. The pool scan and a
/// single-player lookup build him here, so both describe the same man.
pub(crate) struct PlayerSummaryBuilder;

impl PlayerSummaryBuilder {
    /// One player, resolved to his club, with no context folded for it.
    pub(crate) fn one(
        country: &Country,
        club: &Club,
        player: &Player,
        date: NaiveDate,
    ) -> Option<PlayerSummary> {
        let team = club
            .teams
            .teams
            .iter()
            .find(|t| t.players.find(player.id).is_some())?;
        let ctx = SummaryClubContext::of(country, club, date, false);
        Some(Self::build(country, club, &ctx, team, player, date))
    }

    pub(crate) fn build(
        country: &Country,
        club: &Club,
        ctx: &SummaryClubContext,
        team: &Team,
        player: &Player,
        date: NaiveDate,
    ) -> PlayerSummary {
        let group = player.position().position_group();
        let value = PlayerValuationCalculator::calculate_value_with_price_level(
            player,
            date,
            ctx.price_level,
            ctx.seller_league_rep,
            ctx.seller_club_rep,
        );
        let (contract_months_remaining, salary) = player
            .contract
            .as_ref()
            .map(|c| {
                let days = (c.expiration - date).num_days().max(0);
                ((days / 30).min(i16::MAX as i64) as i16, c.salary)
            })
            .unwrap_or((0, 0));
        // Not in the first team's depth chart at all reads as ranked BEHIND
        // it, never folded into it at rank 1, which read every reserve as the
        // seller's second choice.
        let seller_rank = match ctx.group_ranks.rank(player.id) {
            u8::MAX => ctx.group_ranks.group_size(group).saturating_add(1),
            r => r,
        };
        // A B side's own "key player" label is standing in that dressing
        // room, not a first-team promise the market should price against.
        let squad_status = player
            .contract
            .as_ref()
            .map(|c| c.squad_status.as_first_team_designation(team.team_type))
            .unwrap_or(PlayerSquadStatus::NotYetSet);
        let mut summary = PlayerSummary {
            player_id: player.id,
            club_id: club.id,
            country_id: country.id,
            continent_id: country.continent_id,
            region: ctx.country_region,
            country_code: country.code.clone(),
            // Where he is FROM, alongside where he plays.
            nationality_country_id: player.country_id,
            nationality_continent_id: player.nationality_continent_id,
            nationality_region: player.home_region(),
            starter_share: player.happiness.starter_ratio,
            tenure_days: StuckCareerScan::club_tenure_days(player, date)
                .unwrap_or(i64::from(u16::MAX))
                .clamp(0, i64::from(u16::MAX)) as u16,
            // The weekly cache: the mind thinks weekly, so a per-player
            // `MindSituation` build here bought nothing.
            return_home_desire: player.home_pull.desire,
            // The parent's own posting of a foreigner whose want to go home
            // has formed. Scoped to loans, so it can never become a
            // permanent-transfer discovery channel.
            home_return_wanted: HomeLoanGates::is_posted(
                player.home_pull.wanted,
                ctx.home_preferences.contains(&player.id),
            ),
            ambition: player.attributes.ambition as u8,
            loyalty: player.attributes.loyalty as u8,
            adaptability: player.attributes.adaptability as u8,
            leave_pressure: player
                .mind
                .pressure_of(GoalKind::GoOutOnLoan)
                .max(player.mind.pressure_of(GoalKind::LeaveThisClub))
                .max(player.mind.pressure_of(GoalKind::PlayFirstTeamFootball))
                .clamp(0.0, 1.0),
            stay_pressure: player
                .mind
                .pressure_of(GoalKind::StayAtThisClub)
                .max(player.mind.pressure_of(GoalKind::BecomeAClubLegend))
                .clamp(0.0, 1.0),
            parent_loan: ctx
                .stage_parent_loan
                .then(|| LoanAssetGuard::parent_view(club, player, date, &ctx.group_ranks)),
            career_plan: player.mind.career.plan_view(MindClock::day(date)),
            anchors: player.anchor_view(date),
            asks_for_a_bigger_stage: player.asks_only_for_a_bigger_stage(),
            parent_subsidy: player.loan_subsidy(),
            player_name: player.full_name.to_string(),
            club_name: club.name.clone(),
            position: player.position(),
            position_group: group,
            coverage: PositionCoverage::of(&player.positions),
            age: player.age(date),
            estimated_value: value.amount,
            is_listed: player.statuses.has(PlayerStatusType::Lst),
            is_loan_listed: player.statuses.has(PlayerStatusType::Loa),
            skill_ability: PlayerView::position_evaluation_ability(player),
            observable_level: AbilityEstimator::observable_level(player),
            pathway_stage: player.pathway_stage(),
            // Regressed, so a small-sample season does not fool the
            // candidate sorter or the recommendation tiers.
            average_rating: player.statistics.average_rating_realistic(group),
            goals: player.statistics.goals,
            assists: player.statistics.assists,
            appearances: player.statistics.total_games(),
            determination: player.skills.mental.determination,
            work_rate: player.skills.mental.work_rate,
            composure: player.skills.mental.composure,
            anticipation: player.skills.mental.anticipation,
            technical_avg: player.skills.technical.average(),
            mental_avg: player.skills.mental.average(),
            physical_avg: player.skills.physical.average(),
            current_reputation: player.player_attributes.current_reputation,
            home_reputation: player.player_attributes.home_reputation,
            world_reputation: player.player_attributes.world_reputation,
            country_reputation: ctx.country_reputation,
            club_world_reputation: ctx.club_world_rep,
            club_best_in_group: ctx.group_ranks.best(group),
            is_injured: player.player_attributes.is_injured,
            contract_months_remaining,
            salary,
            language_profile: LanguageProfile::from_languages(&player.languages),
            international_apps: player.player_attributes.international_apps,
            career_record: CareerRecordSnapshot::read(player, group),
            seller_ctx: SellerPlausibilityContext {
                club_reputation_score: ctx.seller_club_rep_score,
                league_reputation: ctx.seller_league_rep,
                league_id: ctx.seller_league_id,
                position_group_rank: seller_rank,
                squad_status,
                is_transfer_requested: player.statuses.has(PlayerStatusType::Req),
                is_unhappy: player.statuses.has(PlayerStatusType::Unh),
                in_debt: ctx.seller_in_debt,
                days_on_market: player.days_available(date).min(i16::MAX as i64) as i16,
                market_resignation: player.market_resignation(date),
                club_matches_played: SquadEvidenceSource::club_matches(
                    team.team_type,
                    team.league_id.is_some(),
                    ctx.seller_club_matches,
                ),
                big_stage_inclination: player.big_stage_inclination,
                is_marketed: club.transfer_plan.is_marketed(player.id),
            },
            level_floor: LevelFloor::none(),
            asking_wage: 0.0,
            refusal_price: 0.0,
        };
        summary.level_floor = Self::level_floor(&summary);
        let stance = PlayerStance {
            fair_wage_at_current: Self::fair_wage(player, ctx, date) as f64,
            ..PlayerStanceBuilder::from_summary(&summary, 0.0, ctx.country_region)
        };
        let cfg = AppraisalConfig::default();
        summary.asking_wage = PlayerOfferAppraisal::asking_wage(&stance, &cfg);
        summary.refusal_price = PlayerOfferAppraisal::refusal_price(&stance, &cfg);
        summary
    }

    /// The floor under the football he will play, as a borrowing country
    /// can read it: his name as the world sees it, since the summary is
    /// what crosses a border.
    pub(in crate::transfers) fn level_floor(summary: &PlayerSummary) -> LevelFloor {
        let seen = summary.appearances >= MindSituation::TRACKED_APPS as u16;
        LevelFloor::of(&StandingReading {
            level: summary.observable_level,
            group: summary.position_group,
            effective_rep: EffectivePlayerReputation::compute(
                summary.world_reputation,
                summary.current_reputation,
                summary.home_reputation,
                false,
            ),
            league_rep: summary.seller_ctx.league_reputation,
            starter_share: if seen { summary.starter_share } else { 0.0 },
            caps: summary.international_apps,
            age: summary.age,
            resignation: summary.seller_ctx.market_resignation,
            plan_widening: summary.career_plan.renown_widening(),
            held_summit: summary.anchors.held_summit,
            recency: summary.anchors.recency,
        })
    }

    /// What a man of his standing is worth at the club he is at now: the
    /// same valuation the live stance reads, so a man on a token wage is not
    /// read as one whose price is a token.
    fn fair_wage(player: &Player, ctx: &SummaryClubContext, date: NaiveDate) -> u32 {
        let contract = player.contract.as_ref();
        ContractValuation::evaluate(
            player,
            &ValuationContext {
                age: player.age(date),
                club_reputation_score: ctx.seller_club_rep_score,
                league_reputation: ctx.seller_league_rep,
                squad_status: contract
                    .map(|c| c.squad_status.clone())
                    .unwrap_or(PlayerSquadStatus::FirstTeamRegular),
                current_salary: contract.map(|c| c.salary).unwrap_or(0),
                months_remaining: contract
                    .map(|c| ((c.expiration - date).num_days() / 30) as i32)
                    .unwrap_or(0),
                has_market_interest: AvailabilityView::read(player, false, None).available_soft,
            },
        )
        .expected_wage
    }
}

/// Read-only questions the market asks about a player.
pub(crate) struct PlayerView;

impl PlayerView {
    pub(in crate::transfers) fn find_player_in_country(
        country: &Country,
        player_id: u32,
    ) -> Option<&Player> {
        for club in &country.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.find(player_id) {
                    return Some(player);
                }
            }
        }
        None
    }

    /// Resolve player full name and selling club name from the country data.
    pub(in crate::transfers) fn resolve_player_and_club_name(
        country: &Country,
        player_id: u32,
        club_id: u32,
    ) -> (String, String) {
        let player_name = country
            .clubs
            .iter()
            .flat_map(|c| c.teams.iter())
            .find_map(|t| t.players.find(player_id))
            .map(|p| p.full_name.to_string())
            .unwrap_or_default();

        let club_name = country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        (player_name, club_name)
    }

    pub(in crate::transfers) fn find_player_in_club(
        club: &Club,
        player_id: u32,
    ) -> Option<&Player> {
        for team in &club.teams.teams {
            if let Some(player) = team.players.find(player_id) {
                return Some(player);
            }
        }
        None
    }

    /// Canonical "evaluate this player against a tier baseline" ability.
    /// Returns the position-weighted ability (1..200 scale) used
    /// throughout the transfer pipeline — squad evaluation, scout
    /// reach, listed-star sweeps. Both `current_ability` and this
    /// helper share the 1..200 scale and are kept in sync by the
    /// training and development paths
    /// (`development/tick.rs:190`, `training/result.rs:88`), which
    /// always recompute CA from `calculate_ability_for_position`.
    /// Naming the helper so call-sites can't accidentally mix it up
    /// with raw skill averages prevents the CA-vs-skill drift the
    /// audit flagged.
    pub(crate) fn position_evaluation_ability(player: &Player) -> u8 {
        player
            .skills
            .calculate_ability_for_position(player.position())
    }

    /// Squads whose players are part of the FIRST TEAM's own depth chart:
    /// the main squad plus the club's age-restricted development sides.
    ///
    /// Senior reserves (B / Second / Reserve) are deliberately excluded —
    /// they are branded senior sides that own their own hierarchy, and
    /// folding their regulars into the first team's ranking would tell the
    /// market a B-team striker is competing for the first team's shirt.
    /// The youth squads are the opposite case: a boy registered there is
    /// registered there BY the first team, and where his ability puts him
    /// in the club's depth chart is exactly the fact every rank-driven
    /// reading needs. Reading the main roster alone is what let a
    /// first-team-calibre teenager score `seller_position_rank = unknown`
    /// (importance 0.62 at best), which is the number the loan gates then
    /// judged him unimportant on.
    pub(in crate::transfers) fn ranks_with_first_team(team_type: TeamType) -> bool {
        matches!(team_type, TeamType::Main) || team_type.is_youth()
    }

    /// 0-indexed position-group rank of a player within their club's
    /// first-team depth chart (main squad plus development squads),
    /// ordered by current ability (descending). Used by the
    /// plausibility layer — first-choice GKs are protected by rank in
    /// a way average + status alone don't capture.
    /// Returns `u8::MAX` when the player is not on any of those squads —
    /// callers should treat that as "unknown" and avoid using the
    /// rank-driven importance bump.
    pub(crate) fn position_group_rank(
        club: &Club,
        player_id: u32,
        group: PlayerFieldPositionGroup,
    ) -> u8 {
        let peers = || {
            club.teams
                .iter()
                .filter(|t| PlayerView::ranks_with_first_team(t.team_type))
                .flat_map(|t| t.players.players.iter())
                .filter(|p| p.position().position_group() == group)
        };
        let Some(own) = peers()
            .find(|p| p.id == player_id)
            .map(|p| p.player_attributes.current_ability)
        else {
            return u8::MAX;
        };
        // His place in the group sorted best first, ties kept in squad
        // order: everyone stronger, and every equal listed before him.
        let mut ahead = 0usize;
        let mut before_him = true;
        for p in peers() {
            if p.id == player_id {
                before_him = false;
                continue;
            }
            let ca = p.player_attributes.current_ability;
            if ca > own || (ca == own && before_him) {
                ahead += 1;
            }
        }
        ahead.min(u8::MAX as usize - 1) as u8
    }

    /// The same, on the scale a loan is actually priced on. Hidden
    /// ability decides development; what a club compares a loanee
    /// against is the player it can see.
    pub(crate) fn best_observable_in_group(club: &Club, group: PlayerFieldPositionGroup) -> u8 {
        club.teams
            .iter()
            .filter(|t| PlayerView::ranks_with_first_team(t.team_type))
            .flat_map(|t| t.players.players.iter())
            .filter(|p| p.position().position_group() == group)
            .map(AbilityEstimator::observable_level)
            .max()
            .unwrap_or(0)
    }

    /// Build the structured interest signal for a DOMESTIC rumour-tier
    /// beat: `buyer_club_id` (this country) is sniffing around a player
    /// at another club in the same country. Pre-bid stages only — the
    /// negotiation resolvers own the concrete stages with their richer
    /// `NegotiationData` context. Returns `None` when either club can't
    /// be resolved or the "target" already plays for the buyer.
    pub(crate) fn local_interest_signal(
        country: &Country,
        buyer_club_id: u32,
        player_id: u32,
        stage: TransferInterestStage,
        source: TransferInterestSource,
    ) -> Option<TransferInterestSignal> {
        let (seller_club, player) = country.clubs.iter().find_map(|c| {
            c.teams
                .teams
                .iter()
                .find_map(|t| t.players.find(player_id))
                .map(|p| (c, p))
        })?;
        if seller_club.id == buyer_club_id {
            return None;
        }
        let buyer_club = country.clubs.iter().find(|c| c.id == buyer_club_id)?;

        let rep01 = |club: &Club| -> f32 {
            club.teams.main().map(|t| t.reputation.world).unwrap_or(0) as f32 / 10_000.0
        };
        let league_rep = |club: &Club| -> u16 {
            club.teams
                .teams
                .first()
                .and_then(|t| t.league_id)
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0)
        };

        Some(TransferInterestSignal {
            interested_club_id: buyer_club_id,
            interested_league_id: buyer_club.teams.teams.first().and_then(|t| t.league_id),
            buyer_rep: rep01(buyer_club),
            seller_rep: rep01(seller_club),
            buyer_league_rep: league_rep(buyer_club),
            seller_league_rep: league_rep(seller_club),
            stage,
            source,
            repeated_attention: false,
            is_rival: seller_club.is_rival(buyer_club_id),
            is_home_country: player.country_id == country.id,
            is_seller_in_home_country: player.country_id == country.id,
            is_former_club: player
                .sold_from
                .as_ref()
                .map(|(cid, _)| *cid == buyer_club_id)
                .unwrap_or(false),
            buyer_country_id: country.id,
            buyer_continent_id: country.continent_id,
            // Rumour-tier beats stay narratively modest — the continental
            // opportunity framing belongs to concrete approaches.
            buyer_has_continental_path: false,
            buyer_competition_path: None,
        })
    }

    pub(in crate::transfers) fn get_player_negotiation_data(
        country: &Country,
        player_id: u32,
        date: NaiveDate,
    ) -> (u8, f32) {
        PlayerView::find_player_in_country(country, player_id)
            .map(|p| (p.age(date), p.attributes.ambition))
            .unwrap_or((25, 0.5))
    }
}

/// Per-pass `player_id → club` index for country-wide player lookups, and
/// the summaries those lookups build.
///
/// The scan walks every club/team in the country PER CANDIDATE, which made
/// it the single hottest leaf of the transfer pipeline (shortlist builds,
/// recommendation plausibility re-checks). Passes that resolve many
/// candidates build this once, then each lookup is a hash probe + a
/// verified fetch. The hit is verified against the club's roster (mirrors
/// `resolve_foreign_player_club`'s stale-index guard).
///
/// The passes it serves read the country without changing it and apply
/// what they decide afterwards, so a player's summary is built the first
/// time a pass asks for it and read from then on — his name formatted
/// once, however many clubs weigh him.
pub(in crate::transfers) struct CountryPlayerLookup {
    /// Club index and summary slot of every rostered player.
    slot_by_player: FxHashMap<u32, (u32, u32)>,
    /// Per-club [`SummaryClubContext`], indexed like `Country::clubs` and
    /// folded on first use, so `find_summary` stays `&self` and the lookup
    /// can be shared across parallel per-club scans.
    contexts: Vec<OnceLock<SummaryClubContext>>,
    summaries: Vec<OnceLock<Box<PlayerSummary>>>,
}

impl CountryPlayerLookup {
    pub(in crate::transfers) fn build(country: &Country) -> Self {
        let mut slot_by_player = FxHashMap::default();
        let mut slots = 0u32;
        for (club_idx, club) in country.clubs.iter().enumerate() {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    slot_by_player.insert(player.id, (club_idx as u32, slots));
                    slots += 1;
                }
            }
        }
        CountryPlayerLookup {
            slot_by_player,
            contexts: (0..country.clubs.len()).map(|_| OnceLock::new()).collect(),
            summaries: (0..slots).map(|_| OnceLock::new()).collect(),
        }
    }

    pub(in crate::transfers) fn find_summary(
        &self,
        country: &Country,
        player_id: u32,
        date: NaiveDate,
    ) -> Option<&PlayerSummary> {
        let &(club_idx, slot) = self.slot_by_player.get(&player_id)?;
        let club = country.clubs.get(club_idx as usize)?;
        let team = club
            .teams
            .teams
            .iter()
            .find(|t| t.players.find(player_id).is_some())?;
        let player = team.players.find(player_id)?;
        let ctx = self.contexts[club_idx as usize]
            .get_or_init(|| SummaryClubContext::of(country, club, date, false));
        Some(self.summaries[slot as usize].get_or_init(|| {
            Box::new(PlayerSummaryBuilder::build(
                country, club, ctx, team, player, date,
            ))
        }))
    }

    /// Indexed counterpart of [`PlayerView::find_player_in_country`]: a
    /// verified hit, else the scan.
    pub(in crate::transfers) fn find_player<'a>(
        &self,
        country: &'a Country,
        player_id: u32,
    ) -> Option<&'a Player> {
        if let Some(&(club_idx, _)) = self.slot_by_player.get(&player_id)
            && let Some(club) = country.clubs.get(club_idx as usize)
            && let Some(player) = PlayerView::find_player_in_club(club, player_id)
        {
            return Some(player);
        }
        PlayerView::find_player_in_country(country, player_id)
    }
}

/// Per-club snapshot of the first team's position-group depth chart, in the
/// two orders the market reads: by what it can see of each man, and by
/// current ability — the batched counterpart of
/// [`PlayerView::position_group_rank`] and
/// [`PlayerView::best_observable_in_group`]. The pool build used to call
/// both PER PLAYER, and each call re-collected and re-sorted the whole
/// group (`O(squad²·log)` per club per tick). One build serves every player
/// of the club. Same values by construction: identical comparator over the
/// identical roster sequence (roster order breaks ties),
/// `u8::MAX` for anyone not in the depth chart, `0` best for a missing group
/// or missing main team.
pub(in crate::transfers) struct ClubGroupRanks {
    rank_by_player: FxHashMap<u32, u8>,
    ability_rank_by_player: FxHashMap<u32, u8>,
    best_by_group: [u8; PlayerFieldPositionGroup::COUNT],
    size_by_group: [u8; PlayerFieldPositionGroup::COUNT],
}

impl ClubGroupRanks {
    pub(in crate::transfers) fn build(club: &Club) -> Self {
        let mut rank_by_player = FxHashMap::default();
        let mut ability_rank_by_player = FxHashMap::default();
        let mut best_by_group = [0u8; PlayerFieldPositionGroup::COUNT];
        let mut size_by_group = [0u8; PlayerFieldPositionGroup::COUNT];
        // The FIRST TEAM's own depth chart: the main squad plus the club's
        // age-restricted development sides, which the first team registers
        // its own youngsters in. Senior reserves stay out — see
        // [`PlayerView::ranks_with_first_team`], whose contract this
        // mirrors so the pool builder and the live lookup can never
        // disagree about a player's rank.
        let squads: Vec<&Team> = club
            .teams
            .teams
            .iter()
            .filter(|t| PlayerView::ranks_with_first_team(t.team_type))
            .collect();
        if !squads.is_empty() {
            let mut peers_by_group: [Vec<(usize, u32, u8, u8)>; PlayerFieldPositionGroup::COUNT] =
                Default::default();
            for p in squads.iter().flat_map(|t| t.players.players.iter()) {
                let peers = &mut peers_by_group[p.position().position_group().index()];
                peers.push((
                    peers.len(),
                    p.id,
                    AbilityEstimator::observable_level(p),
                    p.player_attributes.current_ability,
                ));
            }
            for (group_idx, peers) in peers_by_group.iter_mut().enumerate() {
                size_by_group[group_idx] = peers.len().min(u8::MAX as usize) as u8;
                peers.sort_unstable_by_key(|&(order, _, _, ability)| (Reverse(ability), order));
                for (rank, &(_, pid, _, _)) in peers.iter().enumerate() {
                    ability_rank_by_player.insert(pid, rank.min(u8::MAX as usize - 1) as u8);
                }
                peers.sort_unstable_by_key(|&(order, _, level, _)| (Reverse(level), order));
                best_by_group[group_idx] =
                    peers.first().map(|&(_, _, level, _)| level).unwrap_or(0);
                for (rank, &(_, pid, _, _)) in peers.iter().enumerate() {
                    rank_by_player.insert(pid, rank.min(u8::MAX as usize - 1) as u8);
                }
            }
        }
        ClubGroupRanks {
            rank_by_player,
            ability_rank_by_player,
            best_by_group,
            size_by_group,
        }
    }

    /// Rank of the player inside his position group by what the market can
    /// see of him (0 = best), `u8::MAX` when he isn't in the depth chart.
    pub(in crate::transfers) fn rank(&self, player_id: u32) -> u8 {
        self.rank_by_player
            .get(&player_id)
            .copied()
            .unwrap_or(u8::MAX)
    }

    /// The same by current ability — [`PlayerView::position_group_rank`].
    pub(in crate::transfers) fn ability_rank(&self, player_id: u32) -> u8 {
        self.ability_rank_by_player
            .get(&player_id)
            .copied()
            .unwrap_or(u8::MAX)
    }

    /// Best observable level in the group —
    /// [`PlayerView::best_observable_in_group`].
    pub(in crate::transfers) fn best(&self, group: PlayerFieldPositionGroup) -> u8 {
        self.best_by_group[group.index()]
    }

    /// How many first-team players occupy this position group. Lets a
    /// caller place someone who isn't in the main-team depth chart at all
    /// BEHIND it, rather than folding him into it at an invented rank.
    pub(in crate::transfers) fn group_size(&self, group: PlayerFieldPositionGroup) -> u8 {
        self.size_by_group[group.index()]
    }
}

#[cfg(test)]
mod rank_table_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::shared::Location;
    use crate::utils::random::engine::RandomEngine;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PeopleNameGeneratorData,
        PlayerCollection, PlayerGenerator, PlayerPositionType, StaffCollection, TeamCollection,
        TeamReputation, TrainingSchedule,
    };
    use chrono::NaiveTime;

    struct RankFx;

    impl RankFx {
        const POSITIONS: [PlayerPositionType; 4] = [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderCenter,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::Striker,
        ];
        /// Few distinct abilities, so every group carries ties.
        const ABILITIES: [u8; 5] = [96, 110, 110, 124, 124];

        fn team(id: u32, team_type: TeamType, first_id: u32, size: u32, date: NaiveDate) -> Team {
            let names = PeopleNameGeneratorData {
                first_names: vec!["Rank".to_string()],
                last_names: vec!["Table".to_string()],
                nicknames: vec![],
            };
            let players = (0..size as usize)
                .map(|n| {
                    let position = Self::POSITIONS[(n * 7 + id as usize) % Self::POSITIONS.len()];
                    let mut player = PlayerGenerator::generate(1, date, position, 10, &names);
                    player.id = first_id + n as u32;
                    player.player_attributes.current_ability =
                        Self::ABILITIES[(n * 3 + id as usize) % Self::ABILITIES.len()];
                    player
                })
                .collect();
            Team::builder()
                .id(id)
                .league_id(Some(1))
                .club_id(1)
                .name(format!("Team {id}"))
                .slug(format!("team-{id}"))
                .team_type(team_type)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(5000, 5000, 5000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn club(teams: Vec<Team>) -> Club {
            Club::new(
                1,
                "Rank FC".to_string(),
                Location::new(1),
                ClubFinances::new(1_000_000, Vec::new()),
                ClubAcademy::new(10),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(teams),
                ClubFacilities::default(),
            )
        }
    }

    #[test]
    fn rank_table_matches_the_per_player_lookups() {
        let _seed = RandomEngine::hold_seed_for_test(0x5EED_0054);
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let clubs = [
            RankFx::club(vec![
                RankFx::team(1, TeamType::Main, 1, 26, date),
                RankFx::team(2, TeamType::U19, 101, 18, date),
                RankFx::team(3, TeamType::B, 201, 20, date),
            ]),
            RankFx::club(vec![
                RankFx::team(4, TeamType::B, 301, 20, date),
                RankFx::team(5, TeamType::Main, 401, 23, date),
            ]),
            RankFx::club(vec![RankFx::team(6, TeamType::Reserve, 501, 22, date)]),
        ];

        let mut tied = 0;
        for club in &clubs {
            let ranks = ClubGroupRanks::build(club);
            for group in PlayerFieldPositionGroup::ALL {
                assert_eq!(
                    ranks.best(group),
                    PlayerView::best_observable_in_group(club, group)
                );
            }
            let ranked: Vec<&Player> = club
                .teams
                .iter()
                .filter(|t| PlayerView::ranks_with_first_team(t.team_type))
                .flat_map(|t| t.players.players.iter())
                .collect();
            for player in club.teams.iter().flat_map(|t| t.players.players.iter()) {
                assert_eq!(
                    ranks.ability_rank(player.id),
                    PlayerView::position_group_rank(
                        club,
                        player.id,
                        player.position().position_group()
                    ),
                    "player {}",
                    player.id
                );
            }
            for player in &ranked {
                tied += ranked
                    .iter()
                    .filter(|peer| {
                        peer.id < player.id
                            && peer.position().position_group()
                                == player.position().position_group()
                            && peer.player_attributes.current_ability
                                == player.player_attributes.current_ability
                    })
                    .count();
            }
        }
        assert!(tied > 0, "the fixture must rank tied abilities");
        assert_eq!(ClubGroupRanks::build(&clubs[0]).ability_rank(201), u8::MAX);
    }
}

#[cfg(test)]
mod summary_builder_tests {
    use super::*;
    use crate::PlayerPositionType;
    use crate::transfers::gate::build::Suitor;
    use crate::transfers::scouting::ScoutingPass;
    use crate::transfers::tests::kit::{TestClub, TestCountry, TestDate, TestPlayer, TestTeam};

    struct Fx;

    impl Fx {
        /// A club with a first team and a B side, and a reserve centre-half
        /// registered with the B side.
        fn country() -> Country {
            let today = TestDate::today();
            let first_team = TestTeam::new(10)
                .club_id(1)
                .team_type(TeamType::Main)
                .reputation(6_500)
                .players(vec![
                    TestPlayer::new(1)
                        .position(PlayerPositionType::DefenderCenter)
                        .ability(140)
                        .skills_match_ability()
                        .contract_until(2_000_000, TestDate::on(2029, 6, 30))
                        .build(),
                    TestPlayer::new(2)
                        .position(PlayerPositionType::DefenderCenter)
                        .ability(130)
                        .skills_match_ability()
                        .contract_until(1_500_000, TestDate::on(2029, 6, 30))
                        .build(),
                ])
                .build();
            let reserves = TestTeam::new(11)
                .club_id(1)
                .team_type(TeamType::B)
                .league_id(None)
                .reputation(2_000)
                .players(vec![
                    TestPlayer::new(3)
                        .position(PlayerPositionType::DefenderCenter)
                        .ability(95)
                        .age(20)
                        .skills_match_ability()
                        .contract_until(150_000, TestDate::on(2028, 6, 30))
                        .on(today)
                        .build(),
                ])
                .build();
            let buyer = TestClub::new(2).reputation(4_000).build();
            TestCountry::new(1)
                .league_reputation(6_000)
                .clubs(vec![
                    TestClub::new(1).teams(vec![first_team, reserves]).build(),
                    buyer,
                ])
                .build()
        }
    }

    /// The pool scan, the country lookup and the single-player build are one
    /// projection: a reserve player reads the same on all three.
    #[test]
    fn a_reserve_reads_the_same_from_the_pool_and_a_single_lookup() {
        let country = Fx::country();
        let today = TestDate::today();
        let pool = ScoutingPass::collect_player_pool(&country, today, false);
        let from_pool = pool.iter().find(|s| s.player_id == 3).expect("on the pool");
        let club = &country.clubs[0];
        let player = PlayerView::find_player_in_club(club, 3).unwrap();
        let single = PlayerSummaryBuilder::one(&country, club, player, today).unwrap();
        let lookup = CountryPlayerLookup::build(&country);
        let looked_up = lookup.find_summary(&country, 3, today).unwrap();

        let suitor = Suitor::of(&country, &country.clubs[1], today);
        for other in [&single, looked_up] {
            assert_eq!(other.skill_ability, from_pool.skill_ability);
            assert_eq!(
                other.seller_ctx.position_group_rank,
                from_pool.seller_ctx.position_group_rank
            );
            assert_eq!(
                other.seller_ctx.club_matches_played,
                from_pool.seller_ctx.club_matches_played
            );
            assert_eq!(other.estimated_value, from_pool.estimated_value);
            assert_eq!(
                suitor.level_wage_for(other),
                suitor.level_wage_for(from_pool)
            );
            assert_eq!(other.asking_wage, from_pool.asking_wage);
        }
        assert!(
            from_pool.seller_ctx.position_group_rank >= 2,
            "a B-side player ranks behind the first team's depth chart, not inside it"
        );
    }

    /// The floor travels as it was read: the stored figure is the one the
    /// summary's own fields give.
    #[test]
    fn the_stored_floor_is_the_one_its_fields_give() {
        let country = Fx::country();
        let today = TestDate::today();
        for summary in ScoutingPass::collect_player_pool(&country, today, false) {
            assert_eq!(
                summary.level_floor,
                PlayerSummaryBuilder::level_floor(&summary)
            );
            assert!(summary.asking_wage >= 500.0);
            assert!(summary.refusal_price >= 500.0);
        }
    }
}

#[cfg(test)]
mod token_wage_tests {
    use super::*;
    use crate::PlayerPositionType;
    use crate::transfers::tests::kit::{TestClub, TestCountry, TestDate, TestPlayer};

    /// A first-team regular at a top-flight club on a token wage. Read off
    /// his contract alone, his refusal price is a token too, and a club in a
    /// division of reputation 1200 could pay it; read at what a man of his
    /// standing is worth there, it cannot.
    #[test]
    fn a_token_wage_does_not_make_a_refused_division_payable() {
        let today = TestDate::today();
        let player = TestPlayer::new(1)
            .position(PlayerPositionType::MidfielderCenter)
            .ability(150)
            .age(25)
            .skills_match_ability()
            .contract_until(444, TestDate::on(2029, 6, 30))
            .build();
        let country = TestCountry::new(1)
            .league_reputation(8_000)
            .clubs(vec![
                TestClub::new(1)
                    .reputation(8_000)
                    .players(vec![player])
                    .build(),
            ])
            .build();
        let club = &country.clubs[0];
        let player = PlayerView::find_player_in_club(club, 1).unwrap();
        let summary = PlayerSummaryBuilder::one(&country, club, player, today).unwrap();
        assert!(
            summary.level_floor.below(1_200.0) >= 1.0,
            "a division of reputation 1200 is one he refuses outright"
        );

        let on_contract = PlayerOfferAppraisal::refusal_price(
            &PlayerStance::on_contract(summary.age, summary.contract_months_remaining, 444),
            &AppraisalConfig::default(),
        );
        // What a small club could hold for one shirt: well above the token
        // price, well under his own.
        let shirt = (on_contract * summary.refusal_price).sqrt();
        assert!(summary.refusal_price > 10.0 * on_contract);
        assert!(shirt >= on_contract, "the contract-only read would pay it");
        assert!(
            shirt < summary.refusal_price,
            "his price {:.0} against the contract-only {:.0}",
            summary.refusal_price,
            on_contract
        );
    }
}
