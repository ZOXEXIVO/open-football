//! Year-round performance-breakout watch — the one talent watch a club
//! runs, over everything its scouting network can see.
//!
//! The demand-driven scout pipeline and the listed-star sweep only run
//! inside a transfer window, so a player whose *form* is outrunning his
//! level — a 22-year-old striker top-scoring a second division — could sit
//! for months with zero interest and zero scout monitoring simply because
//! the window was shut and his club had merely loan-listed him.
//!
//! This pass closes that hole. Weekly, regardless of the window, it surfaces
//! genuine breakouts to plausible buyers as **scout monitoring** plus a
//! staff recommendation — the club puts the player on its books and its
//! recruitment department argues for him. It never starts a negotiation:
//! those stay window-gated, so the recorded interest flows through the
//! normal recommendation / meeting / shortlist path once the window opens.
//!
//! The watch covers the whole world through one lens. Candidates come from
//! this country's rosters AND from the shared world snapshot, and every one
//! passes the same gates; what differs is only what a scout can observe
//! from where he stands. A home candidate is corroborated — scoring-chart
//! standing, awards, availability state, a read on his character. A
//! foreigner is judged on the bare output and rating his market summary
//! carries, a strictly harder read. And on the buyer's side the club's
//! scouting NETWORK bounds what it sees at all: reach widens continuously
//! with reputation, so a giant's watch covers the world while a minnow's
//! stops at its own border.
//!
//! Realism is identical to the in-window sweep: the same
//! [`evaluate_listed_target`] gates (tier window, affordability, wage
//! headroom, reputation plausibility, squad need / upgrade / resale) plus the
//! staged [`TransferPlausibilityBuilder`] veto run here — only with
//! `form_discovery_mode`, so a not-yet-listed breakout can be discovered on
//! form rather than on a for-sale sign. A flat-track scorer in a weak division
//! is league-rep discounted inside the breakout signal and so never clears the
//! bar; a top club still won't monitor a player who is no upgrade, no resale
//! prospect, and fills no need.
//!
//! Youth squads are covered too. Their matches are friendly-classified, so a
//! youngster's output lives in his `friendly_statistics` and his age-group
//! "league" carries no senior reputation. For a youth team the watch reads the
//! friendly bucket and scores it undiscounted
//! ([`crate::transfers::pipeline::breakout::LeaguePerformanceLookup::breakout_for_youth`]),
//! so an academy standout — an U18 banging in goals — surfaces to plausible
//! clubs as scout monitoring instead of staying invisible behind a
//! zero-reputation age-group league.
//!
//! Per project convention this is a method on [`PipelineProcessor`]; every
//! type is reached through a `use` at the file header.

use std::cmp::Ordering;
use std::collections::HashSet;

use chrono::{Datelike, NaiveDate, Weekday};

use crate::transfers::ScoutingRegion;
use crate::transfers::pipeline::breakout::{
    BreakoutInputs, BreakoutPerformanceSignal, LeaguePerformanceLookup,
};
use crate::transfers::pipeline::circulation::BuyerScan;
use crate::transfers::pipeline::plausibility::{
    BuyerPlausibilityContext, TransferPlausibilityBuilder, TransferPlausibilityVerdict,
};
use crate::transfers::pipeline::processor::{PipelineProcessor, PlayerSummary};
use crate::transfers::pipeline::recommendations::{
    ListedTargetVerdict, ListedTargetView, evaluate_listed_target,
};
use crate::transfers::pipeline::recruitment::{ScoutMonitoringSource, ScoutPlayerMonitoring};
use crate::transfers::pipeline::{
    KnownPlayerMemory, RecommendationSource, RecommendationType, StaffRecommendation,
};
use crate::{Country, Person};

/// One breakout player the watch may surface — his market summary, the few
/// extra signals the listed-target view needs that the summary doesn't carry,
/// and his discovery score.
struct BreakoutCandidate {
    summary: PlayerSummary,
    estimated_potential: u8,
    ambition: f32,
    days_available: i64,
    recent_interest_count: u8,
    failed_scans: u16,
    breakout_score: f32,
}

/// One accepted find the apply pass will file on a buyer's books.
struct WatchAction {
    club_id: u32,
    recommender_staff_id: u32,
    player_id: u32,
    player_club_id: u32,
    player_country_id: u32,
    position: crate::PlayerPositionType,
    position_group: crate::PlayerFieldPositionGroup,
    appearances: u16,
    assessed_ability: u8,
    assessed_potential: u8,
    confidence: f32,
    estimated_value: f64,
}

impl PipelineProcessor {
    /// Per-pass cap on NEW breakout monitors a single club opens, so the
    /// watch builds a club's shortlist gradually rather than in one flood.
    const BREAKOUT_WATCH_PER_PASS: usize = 3;
    /// Soft ceiling on a club's total active monitoring rows before the watch
    /// stops adding more — keeps the books from growing without bound.
    const BREAKOUT_WATCH_MONITOR_CAP: usize = 30;

    /// Big-stage pull a foreign player must carry before his own transfer
    /// request counts as a lead in its own right. Set at the visible-mood
    /// level: he is publicly restless, not merely open to offers.
    const TOUTED_MIN_INCLINATION: f32 = 0.40;
    /// League-reputation gain the watching country must offer over his
    /// current one before that lead means anything. A sideways move
    /// answers nothing he is asking for.
    const TOUTED_MIN_STAGE_GAIN: u16 = 1200;
    /// He still has to be a footballer worth the call. Roughly half the
    /// ordinary breakout bar: enough to exclude a journeyman with an
    /// agent, low enough to surface the good defenders and holding
    /// midfielders whose seasons produce no headline numbers at all.
    const TOUTED_BREAKOUT_FLOOR: f32 = 22.0;

    /// Weekly, year-round breakout watch. Surfaces high-form players to
    /// plausible buyers as scout monitoring plus a staff recommendation
    /// (never a negotiation). See the module docs for the realism model.
    ///
    /// One watch, one world. `foreign_players` is the shared world
    /// snapshot minus this country — the same clubs, gates and caps
    /// evaluate every candidate, and what separates a domestic find from
    /// a foreign one is only what a scout can actually observe from
    /// where he stands: home candidates come corroborated (scoring-chart
    /// standing, awards, availability signals), a foreigner is judged on
    /// the bare output and rating his market summary carries — a
    /// strictly harder read. The buyer-side asymmetry is the scouting
    /// NETWORK: every club watches its own backyard, and reach beyond it
    /// widens continuously with reputation, so a giant's watch covers
    /// the world while a minnow's stops at the border.
    pub fn scan_breakout_form(
        country: &mut Country,
        foreign_players: &[&PlayerSummary],
        date: NaiveDate,
    ) {
        // Weekly cadence, independent of the transfer window.
        if date.weekday() != Weekday::Mon {
            return;
        }

        let performance_lookup = LeaguePerformanceLookup::build(country);

        // ── Collect breakout candidates (immutable read). ──
        let mut candidates: Vec<BreakoutCandidate> = Vec::new();
        for club in &country.clubs {
            let parent_league_reputation = club
                .teams
                .main()
                .and_then(|t| t.league_id)
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);

            for team in &club.teams.teams {
                let is_youth_squad = team.team_type.is_youth();
                for player in &team.players.players {
                    if player.is_on_loan() {
                        continue;
                    }
                    let group = player.position().position_group();
                    let age = player.age(date);
                    // Youth squads play friendly-classified age-group football, so
                    // a youngster's goals and rating live in the FRIENDLY bucket
                    // and his form is judged on the undiscounted signal — a scout
                    // watching the U18s rates the talent on what he sees, not on
                    // the (near-zero) standing of a youth league. Senior squads
                    // keep the official-stats, league-rep-discounted path.
                    let breakout = if is_youth_squad {
                        let appearances = player.friendly_statistics.total_games();
                        let average_rating =
                            player.friendly_statistics.average_rating_realistic(group);
                        performance_lookup.breakout_for_youth(
                            player,
                            appearances,
                            average_rating,
                            age,
                        )
                    } else {
                        let appearances = player.statistics.total_games();
                        let average_rating = player.statistics.average_rating_realistic(group);
                        performance_lookup.breakout_for_player(
                            player,
                            appearances,
                            average_rating,
                            age,
                            parent_league_reputation,
                        )
                    };
                    if !breakout.is_breakout() {
                        continue;
                    }

                    // The candidate walk already holds the (club, player)
                    // pair — build the summary directly instead of
                    // re-finding the player with a country-wide scan.
                    let summary = Self::build_player_summary(country, club, player, date);

                    let skill_ability = Self::position_evaluation_ability(player);
                    let estimated_potential = skill_ability
                        + Self::estimate_growth_potential(
                            age,
                            player.skills.mental.determination,
                            player.skills.mental.work_rate,
                            player.skills.mental.composure,
                            player.skills.mental.anticipation,
                            skill_ability,
                        );

                    candidates.push(BreakoutCandidate {
                        summary,
                        estimated_potential,
                        ambition: player.attributes.ambition,
                        days_available: player.days_available(date),
                        recent_interest_count: player
                            .availability_market_state()
                            .map(|s| s.recent_interest(date))
                            .unwrap_or(0),
                        failed_scans: player
                            .availability_market_state()
                            .map(|s| s.failed_scans)
                            .unwrap_or(0),
                        breakout_score: breakout.score,
                    });
                }
            }
        }

        // Foreign candidates from the world snapshot — judged on exactly
        // what a scout abroad can see. The summary carries his output,
        // rating and league standing but not the per-country scoring
        // charts, awards or personality reads, so those signals are
        // simply absent: a foreigner clears the same bar on less
        // evidence, which makes his bar effectively higher, never lower.
        // Clubs look for talent DOWN the football ladder, the same
        // convention the scouting pass applies.
        let country_reputation = country.reputation;
        // Strength of the best competition this country can offer. A player
        // agitating for a bigger stage is only a lead for clubs whose stage
        // is actually bigger.
        let best_league_reputation = country
            .leagues
            .leagues
            .iter()
            .filter(|l| !l.friendly)
            .map(|l| l.reputation)
            .max()
            .unwrap_or(0);
        for s in foreign_players
            .iter()
            .copied()
            .filter(|s| s.country_reputation <= country_reputation)
        {
            let breakout = BreakoutPerformanceSignal::compute(&BreakoutInputs {
                position_group: s.position_group,
                goals: s.goals,
                assists: s.assists,
                appearances: s.appearances,
                average_rating: s.average_rating,
                age: s.age,
                league_reputation: s.seller_ctx.league_reputation,
                is_league_top_scorer: false,
                scoring_rank: None,
                recent_award_points: 0.0,
            });
            // A player who has formally asked to leave a league he has
            // outgrown is himself a lead — that is what an agent's phone
            // call IS, and it is how most of these moves actually begin.
            // Output alone will never surface a defender or a holding
            // midfielder from a sub-elite league however obviously ready he
            // is, because the breakout signal can only read goals.
            //
            // It lowers the bar; it does not remove it. He still has to be
            // wanting out, drawn strongly enough to have reached a formal
            // request, and looking at a genuinely bigger stage than the one
            // he is on — this country's best competition, not merely a
            // different one.
            let touted_by_his_own_ambition = s.seller_ctx.is_transfer_requested
                && s.seller_ctx.big_stage_inclination >= Self::TOUTED_MIN_INCLINATION
                && best_league_reputation
                    >= s.seller_ctx
                        .league_reputation
                        .saturating_add(Self::TOUTED_MIN_STAGE_GAIN);
            if !breakout.is_breakout()
                && !(touted_by_his_own_ambition && breakout.score >= Self::TOUTED_BREAKOUT_FLOOR)
            {
                continue;
            }
            let estimated_potential = s.skill_ability
                + Self::estimate_growth_potential(
                    s.age,
                    s.determination,
                    s.work_rate,
                    s.composure,
                    s.anticipation,
                    s.skill_ability,
                );
            candidates.push(BreakoutCandidate {
                summary: s.clone(),
                estimated_potential,
                // Personality is not observable from abroad — neutral.
                // It only shades the exposure soft score, never a gate.
                ambition: 10.0,
                days_available: s.seller_ctx.days_on_market as i64,
                recent_interest_count: 0,
                failed_scans: 0,
                breakout_score: breakout.score,
            });
        }

        if candidates.is_empty() {
            return;
        }

        // ── Per-buyer evaluation (immutable read). ──
        let mut actions: Vec<WatchAction> = Vec::new();
        for club in &country.clubs {
            if club.teams.teams.is_empty() {
                continue;
            }
            let plan = &club.transfer_plan;
            if !plan.initialized || plan.scout_monitoring.len() >= Self::BREAKOUT_WATCH_MONITOR_CAP
            {
                continue;
            }
            let Some(scan) = BuyerScan::build(country, club, date) else {
                continue;
            };

            let team = &club.teams.teams[0];
            let resolved = team.staffs.resolve_for_transfers();
            let recommender_id = resolved
                .director_of_football
                .map(|s| s.id)
                .or_else(|| resolved.scouts.first().map(|s| s.id))
                .unwrap_or(team.staffs.head_coach().id);
            let buyer_plaus_ctx = BuyerPlausibilityContext::build(country, club);

            // The club's scouting NETWORK — which regions of the world its
            // watch actually covers. Always includes the home backyard, so
            // domestic candidates pass by construction; reach beyond it
            // widens continuously with reputation, the same curve the
            // demand-driven scouting pass uses. This one gate is what
            // keeps "form travels" meaning "as far as your scouts do".
            let home_region = ScoutingRegion::from_country(country.continent_id, &country.code);
            let club_overall_score = club
                .teams
                .main()
                .or_else(|| club.teams.teams.first())
                .map(|t| t.reputation.overall_score())
                .unwrap_or(0.0);
            let reach: HashSet<ScoutingRegion> =
                Self::reputation_scout_regions(home_region, club_overall_score)
                    .into_iter()
                    .collect();

            let mut scored: Vec<(&BreakoutCandidate, f32)> = candidates
                .iter()
                .filter_map(|c| {
                    let s = &c.summary;
                    // Identity gates — own player, rival, or one this club is
                    // already tracking.
                    if s.club_id == club.id || club.is_rival(s.club_id) {
                        return None;
                    }
                    if !reach.contains(&s.region) {
                        return None;
                    }
                    if !plan.monitorings_for_player(s.player_id).is_empty() {
                        return None;
                    }
                    // Meeting rejections blocklist the player for 6 months.
                    // The Rejected monitoring row fails is_active_interest,
                    // so the dedup above misses him and the watch used to
                    // re-seed a meeting-ready row the very next Monday —
                    // an agenda churn loop the blocklist exists to stop.
                    if plan.is_rejected(s.player_id, date) {
                        return None;
                    }
                    // Staged plausibility veto — importance / country route /
                    // step-down realism. Unsolicited: we're scouting on form.
                    if matches!(
                        TransferPlausibilityBuilder::evaluate_summary(
                            &buyer_plaus_ctx,
                            s,
                            false,
                            true,
                            date,
                        ),
                        Some(TransferPlausibilityVerdict::HardReject(_))
                    ) {
                        return None;
                    }

                    let view = ListedTargetView {
                        ability: s.skill_ability,
                        estimated_potential: c.estimated_potential,
                        age: s.age,
                        estimated_value: s.estimated_value,
                        position_group: s.position_group,
                        is_listed: s.is_listed,
                        is_transfer_requested: s.seller_ctx.is_transfer_requested,
                        is_unhappy: s.seller_ctx.is_unhappy,
                        is_loan_listed: s.is_loan_listed,
                        breakout_score: c.breakout_score,
                        world_reputation: s.world_reputation,
                        current_reputation: s.current_reputation,
                        ambition: c.ambition,
                        parent_club_score: s.seller_ctx.club_reputation_score,
                        parent_club_in_debt: s.seller_ctx.in_debt,
                        days_available: c.days_available,
                        contract_months_remaining: s.contract_months_remaining,
                        low_usage: s.appearances < 8,
                        recent_interest_count: c.recent_interest_count,
                        failed_scans: c.failed_scans,
                        last_block: None,
                    };
                    // Form-discovery mode: a not-yet-listed breakout is
                    // admitted, but the affordability / tier / reputation /
                    // squad-need gates are unchanged.
                    let ctx = scan.buyer_context(s.position_group, true);
                    match evaluate_listed_target(&view, &ctx) {
                        ListedTargetVerdict::Accept(score) => Some((c, score)),
                        ListedTargetVerdict::Reject(_) => None,
                    }
                })
                .collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));

            for (cand, _score) in scored.iter().take(Self::BREAKOUT_WATCH_PER_PASS) {
                let s = &cand.summary;
                actions.push(WatchAction {
                    club_id: club.id,
                    recommender_staff_id: recommender_id,
                    player_id: s.player_id,
                    player_club_id: s.club_id,
                    player_country_id: s.country_id,
                    position: s.position,
                    position_group: s.position_group,
                    appearances: s.appearances,
                    assessed_ability: s.skill_ability,
                    assessed_potential: cand.estimated_potential,
                    // Confidence scales with breakout strength so a clear
                    // breakout lands meeting-ready — the interest can flow
                    // straight into the shortlist when the window opens.
                    confidence: (0.5 + (cand.breakout_score / 100.0) * 0.35).min(0.85),
                    estimated_value: s.estimated_value,
                });
            }
        }

        if actions.is_empty() {
            return;
        }

        // ── Apply (mutable): file each find on the buyer's books. ──
        for action in actions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                let rec_cap = club
                    .teams
                    .teams
                    .first()
                    .map(|t| Self::staff_recommendation_cap(t.reputation.level()))
                    .unwrap_or(0);
                let plan = &mut club.transfer_plan;
                if plan
                    .find_monitoring_mut(action.recommender_staff_id, action.player_id)
                    .is_some()
                {
                    continue;
                }
                let id = plan.next_monitoring_id();
                let mut row = ScoutPlayerMonitoring::new(
                    id,
                    action.recommender_staff_id,
                    action.player_id,
                    ScoutMonitoringSource::StaffRecommendation,
                    date,
                );
                row.record_observation(
                    action.assessed_ability,
                    action.assessed_potential,
                    action.confidence,
                    1.0,
                    action.estimated_value,
                    Vec::new(),
                    date,
                    false,
                );
                plan.scout_monitoring.push(row);

                // What the club now knows about him. For a foreigner this
                // is the only durable record of who and where he is — he
                // exists on no roster the recommendation processor can
                // walk — and for a domestic find it is simply the same
                // institutional memory every scouting pass keeps.
                plan.remember_known_player(KnownPlayerMemory {
                    player_id: action.player_id,
                    last_known_club_id: action.player_club_id,
                    last_known_country_id: action.player_country_id,
                    position: action.position,
                    position_group: action.position_group,
                    assessed_ability: action.assessed_ability,
                    assessed_potential: action.assessed_potential,
                    confidence: action.confidence,
                    estimated_fee: action.estimated_value,
                    last_seen: date,
                    official_appearances_seen: action.appearances,
                    friendly_appearances_seen: 0,
                });

                // A meeting-ready file still dies without a positional
                // request to attach to — the meeting drops promotions
                // with no live request, so a monitored star at a
                // position with no vacancy used to sit on the books
                // forever. A staff recommendation is what the weekly
                // processor can turn into a request of its own: the
                // marquee path, where the player's form creates the
                // need. Queue bound: the same reputation-scaled cap
                // every other recommendation source respects.
                let already_recommended = plan
                    .staff_recommendations
                    .iter()
                    .any(|r| r.player_id == action.player_id);
                if !already_recommended && plan.staff_recommendations.len() < rec_cap {
                    plan.staff_recommendations.push(StaffRecommendation {
                        player_id: action.player_id,
                        recommender_staff_id: action.recommender_staff_id,
                        source: RecommendationSource::ScoutNetwork,
                        recommendation_type: RecommendationType::PerformanceBreakout,
                        assessed_ability: action.assessed_ability,
                        assessed_potential: action.assessed_potential,
                        confidence: action.confidence,
                        estimated_fee: action.estimated_value,
                        date_recommended: date,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod breakout_watch_tests {
    use super::*;
    use crate::club::academy::ClubAcademy;
    use crate::club::player::LanguageProfile;
    use crate::club::player::builder::PlayerBuilder;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::fullname::FullName;
    use crate::shared::{Currency, CurrencyValue, Location};
    use crate::transfers::pipeline::{
        SellerPlausibilityContext, TransferNeedReason, TransferRequestStatus,
    };
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
        PlayerAttributes, PlayerCollection, PlayerFieldPositionGroup, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, PlayerSquadStatus, PositionCoverage,
        StaffClubContract, StaffCollection, StaffPosition, StaffStatus, StaffStub, Team,
        TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::NaiveTime;

    /// Fixtures for the cross-border breakout watch. The buying country
    /// holds one well-run club; the candidates are hand-built foreign
    /// market summaries — exactly what the shared world snapshot carries.
    struct Fx;

    impl Fx {
        /// A Monday inside the Italian summer window, so both the weekly
        /// cadence gate and the recommendation processor's calendar pass.
        fn monday() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 7, 6).unwrap()
        }

        fn player(id: u32, ability: u8) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ability;
            attrs.potential_ability = ability;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Home".into(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(1999, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ability))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .build()
                .unwrap()
        }

        fn scout(id: u32) -> crate::Staff {
            let mut s = StaffStub::default();
            s.id = id;
            s.staff_attributes.knowledge.judging_player_ability = 15;
            s.staff_attributes.knowledge.judging_player_potential = 15;
            s.contract = Some(StaffClubContract::new(
                100_000,
                NaiveDate::from_ymd_opt(2030, 6, 30).unwrap(),
                StaffPosition::Scout,
                StaffStatus::Active,
            ));
            s
        }

        /// The buying club: strong reputation (global scouting reach), a
        /// real recruitment department, money to spend, an initialized
        /// plan — and a modest squad whose average doesn't reject a
        /// quality incomer.
        fn buyer_club(reputation: u16) -> Club {
            let squad: Vec<Player> = (1..=4).map(|i| Self::player(800 + i, 130)).collect();
            let team = Team::builder()
                .id(11)
                .league_id(Some(10))
                .club_id(1)
                .name("Buyer".into())
                .slug("buyer".into())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(squad))
                .staffs(StaffCollection::new(vec![Self::scout(501)]))
                .reputation(TeamReputation::new(reputation, reputation, reputation))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap();
            let mut club = Club::new(
                1,
                "Buyer".into(),
                Location::new(1),
                ClubFinances::new(100_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![team]),
                ClubFacilities::default(),
            );
            club.finance.transfer_budget = Some(CurrencyValue {
                amount: 60_000_000.0,
                currency: Currency::Usd,
            });
            club.finance.wage_budget = Some(CurrencyValue {
                amount: 30_000_000.0,
                currency: Currency::Usd,
            });
            club.transfer_plan.initialized = true;
            club.transfer_plan.total_budget = 60_000_000.0;
            club
        }

        fn country(buyer_reputation: u16) -> Country {
            let league = League::new(
                10,
                "Serie A".into(),
                "serie-a".into(),
                1,
                8000,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                    split_season: false,
                },
                false,
            );
            let mut country = Country::builder()
                .id(1)
                .code("IT".into())
                .slug("italy".into())
                .name("Italy".into())
                .continent_id(1)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(vec![Self::buyer_club(buyer_reputation)])
                .build()
                .unwrap();
            country.reputation = 8_500;
            country
        }

        /// The reported case: a right-sided midfielder at a big club in a
        /// smaller football country, 23 goals in 23 games at an 8.4
        /// average — contented, unlisted, no market flag of any kind.
        fn massalyga(ability: u8) -> PlayerSummary {
            PlayerSummary {
                player_id: 9001,
                club_id: 300,
                country_id: 2,
                continent_id: 1,
                region: ScoutingRegion::from_country(1, "IT"),
                country_code: "RU".into(),
                player_name: "Foreign Star".into(),
                club_name: "Big Foreign Club".into(),
                position: PlayerPositionType::MidfielderRight,
                position_group: PlayerFieldPositionGroup::Midfielder,
                coverage: PositionCoverage::single(PlayerPositionType::MidfielderRight),
                age: 27,
                estimated_value: 20_000_000.0,
                is_listed: false,
                is_loan_listed: false,
                skill_ability: ability,
                average_rating: 8.4,
                goals: 23,
                assists: 3,
                appearances: 23,
                determination: 14.0,
                work_rate: 14.0,
                composure: 16.0,
                anticipation: 15.0,
                technical_avg: 16.0,
                mental_avg: 15.0,
                physical_avg: 13.0,
                current_reputation: 5_500,
                home_reputation: 6_000,
                world_reputation: 3_000,
                country_reputation: 5_000,
                club_world_reputation: 6_000,
                club_best_in_group: ability,
                is_injured: false,
                contract_months_remaining: 30,
                salary: 3_000_000,
                seller_ctx: SellerPlausibilityContext {
                    club_reputation_score: 0.60,
                    league_reputation: 6_000,
                    league_id: None,
                    position_group_rank: 0,
                    squad_status: PlayerSquadStatus::KeyPlayer,
                    is_transfer_requested: false,
                    is_unhappy: false,
                    in_debt: false,
                    days_on_market: 0,
                    market_resignation: 0.0,
                    club_matches_played: 0,
                    big_stage_inclination: 0.0,
                },
                language_profile: LanguageProfile::default(),
            }
        }

        /// Same player, ordinary output — the control.
        fn ordinary_foreigner(ability: u8) -> PlayerSummary {
            let mut s = Self::massalyga(ability);
            s.player_id = 9002;
            s.goals = 3;
            s.assists = 1;
            s.average_rating = 6.6;
            s
        }
    }

    /// The Massalyga case: sustained elite output at a big club in a
    /// smaller country must surface to a foreign buyer with global
    /// scouting reach — monitoring, durable memory AND a staff
    /// recommendation, even though he is contented and unlisted.
    #[test]
    fn foreign_breakout_star_is_discovered_without_being_listed() {
        let mut country = Fx::country(8_200);
        let rep_score = country.clubs[0].teams.teams[0].reputation.overall_score();
        let ability = PipelineProcessor::tier_starter_ca_score(
            rep_score,
            PlayerFieldPositionGroup::Midfielder,
        );
        let star = Fx::massalyga(ability);
        let pool: Vec<&PlayerSummary> = vec![&star];

        PipelineProcessor::scan_breakout_form(&mut country, &pool, Fx::monday());

        let plan = &country.clubs[0].transfer_plan;
        assert!(
            !plan.monitorings_for_player(star.player_id).is_empty(),
            "the scouting department must open a file on a 23-goal 8.4-rated foreigner"
        );
        assert!(
            plan.known_player(star.player_id).is_some(),
            "the club must remember who and where he is — he exists on no domestic roster"
        );
        assert!(
            plan.staff_recommendations.iter().any(|r| {
                r.player_id == star.player_id
                    && r.recommendation_type == RecommendationType::PerformanceBreakout
            }),
            "the find must be filed as a recommendation so it can become a pursuit"
        );
    }

    /// …and the recommendation becomes a REQUEST — the marquee path.
    /// With no vacancy anywhere in the buyer's plan, the player's form
    /// itself opens the need, the shortlist, and so the road to a bid.
    #[test]
    fn the_recommendation_opens_a_marquee_request() {
        let mut country = Fx::country(8_200);
        let rep_score = country.clubs[0].teams.teams[0].reputation.overall_score();
        let ability = PipelineProcessor::tier_starter_ca_score(
            rep_score,
            PlayerFieldPositionGroup::Midfielder,
        );
        let star = Fx::massalyga(ability);
        let pool: Vec<&PlayerSummary> = vec![&star];

        PipelineProcessor::scan_breakout_form(&mut country, &pool, Fx::monday());
        assert!(country.clubs[0].transfer_plan.transfer_requests.is_empty());

        PipelineProcessor::process_staff_recommendations(&mut country, Fx::monday());

        let plan = &country.clubs[0].transfer_plan;
        let request = plan
            .transfer_requests
            .iter()
            .find(|r| r.reason == TransferNeedReason::StaffRecommendation);
        assert!(
            request.is_some(),
            "a high-confidence breakout find must open its own transfer request"
        );
        let request = request.unwrap();
        assert_ne!(request.status, TransferRequestStatus::Abandoned);
        assert!(
            plan.shortlists.iter().any(|s| {
                s.transfer_request_id == request.id
                    && s.candidates.iter().any(|c| c.player_id == star.player_id)
            }),
            "the player himself must be the shortlisted candidate on that request"
        );
    }

    #[test]
    fn ordinary_foreign_output_is_not_discovered() {
        let mut country = Fx::country(8_200);
        let rep_score = country.clubs[0].teams.teams[0].reputation.overall_score();
        let ability = PipelineProcessor::tier_starter_ca_score(
            rep_score,
            PlayerFieldPositionGroup::Midfielder,
        );
        let plain = Fx::ordinary_foreigner(ability);
        let pool: Vec<&PlayerSummary> = vec![&plain];

        PipelineProcessor::scan_breakout_form(&mut country, &pool, Fx::monday());

        assert!(
            country.clubs[0]
                .transfer_plan
                .monitorings_for_player(plain.player_id)
                .is_empty(),
            "3 goals at 6.6 is not a breakout - no file is opened"
        );
    }

    /// A modest club has no scouting network abroad — the same star
    /// stays invisible to it. Form travels only as far as a club's
    /// actual reach.
    #[test]
    fn a_small_club_has_no_network_to_see_him() {
        let mut country = Fx::country(2_000);
        let star = {
            let mut s = Fx::massalyga(90);
            // Put him in a region a backyard-only network can't cover.
            s.region = ScoutingRegion::from_country(3, "BR");
            s
        };
        let pool: Vec<&PlayerSummary> = vec![&star];

        PipelineProcessor::scan_breakout_form(&mut country, &pool, Fx::monday());

        assert!(
            country.clubs[0]
                .transfer_plan
                .monitorings_for_player(star.player_id)
                .is_empty(),
            "a low-reputation club's scouting reach stops at its own backyard"
        );
    }
}
