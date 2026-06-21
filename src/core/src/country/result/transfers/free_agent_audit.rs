//! Audit / debug layer for the free-agent market. Answers the question
//! the matcher itself can't: *why is this long-term free agent still
//! unsigned?* — by combining the player's durable market state (days
//! free, pressure, offers, last block reason) with a live world scan
//! of who could even buy them (eligible countries / clubs / matching
//! requests under the same gates the matcher runs).
//!
//! Read-only: nothing here mutates the market. The monthly
//! `log_long_term` hook gives the simulation log one explanatory line
//! per 12-month-plus free agent when debug logging is enabled.

use super::free_agent_market_calc::FreeAgentMarketCalculator;
use super::types::can_club_accept_player;
use crate::Person;
use crate::Player;
use crate::club::player::transfer::{FreeAgentBlockReason, FreeAgentStatusCategory, MarketStage};
use crate::simulator::{FreeAgentFlowCounters, SimulatorData};
use crate::transfers::pipeline::TransferRequestStatus;
use crate::transfers::scouting_region::ScoutingRegion;
use chrono::NaiveDate;
use log::debug;
use std::collections::HashMap;

/// One player's full market-stall picture. Mirrors the fields the
/// matcher decides on, plus the world-scan eligibility counters that
/// tell apart "nobody wants him" (zero matching requests) from "the
/// gates block him" (zero eligible countries) from "offers keep
/// failing" (block reason from the funnel's far end).
#[derive(Debug, Clone)]
pub struct FreeAgentMarketDiagnosis {
    pub player_id: u32,
    pub player_name: String,
    pub days_free: i64,
    pub market_stage: Option<MarketStage>,
    pub career_pressure: f32,
    pub ability: u8,
    pub age: u8,
    pub reference_reputation: u16,
    pub last_salary: u32,
    pub offers_received_30d: u8,
    pub offers_rejected_total: u16,
    pub last_block_reason: Option<FreeAgentBlockReason>,
    /// Countries whose rep / region / cross-continent gates the player
    /// passes at their current career pressure.
    pub eligible_country_count: usize,
    /// Clubs (within eligible countries) with roster room whose tier
    /// quality band fits the player's CA.
    pub eligible_club_count: usize,
    /// Open transfer requests at eligible clubs matching the player's
    /// position group.
    pub matching_request_count: usize,
}

impl FreeAgentMarketDiagnosis {
    /// Collapse the full world-scan picture into a single category — the
    /// most authoritative structural answer first (data hole → nobody in
    /// reputation range → nobody with room/quality fit → nobody
    /// recruiting his position), then the funnel block reason refined by
    /// offer history for the "clubs want him but terms/dice haven't
    /// landed" cases. This is the richer counterpart to the cheap,
    /// state-only `Player::market_explanation`.
    pub fn category(&self) -> FreeAgentStatusCategory {
        if matches!(
            self.last_block_reason,
            Some(FreeAgentBlockReason::UnknownNationality)
        ) {
            return FreeAgentStatusCategory::DataUnknown;
        }
        if self.eligible_country_count == 0 {
            // No country clears his rep / region gate — he's pricing
            // himself at a level no reachable league can match.
            return FreeAgentStatusCategory::ReputationWait;
        }
        if self.eligible_club_count == 0 {
            // Countries in range exist, but every club is at capacity or
            // outside his quality band.
            return FreeAgentStatusCategory::LowInterest;
        }
        if self.matching_request_count == 0 {
            return FreeAgentStatusCategory::NoPositionNeed;
        }
        // Clubs are recruiting his position — so it's a terms / timing
        // matter. A history of offers means he's turning them down;
        // otherwise interest is building toward a deal.
        let from_reason = FreeAgentStatusCategory::from_block_reason(self.last_block_reason);
        if matches!(from_reason, FreeAgentStatusCategory::WageTooHigh) {
            return from_reason;
        }
        if self.offers_rejected_total >= 1 || self.offers_received_30d >= 1 {
            return FreeAgentStatusCategory::OffersRefused;
        }
        FreeAgentStatusCategory::InterestBuilding
    }

    /// Human-readable, world-scan-aware explanation of why the player is
    /// still unsigned. See [`Self::category`].
    pub fn explanation(&self) -> String {
        self.category().default_message().to_string()
    }
}

/// Read-only auditor over `SimulatorData`. Unit struct per the project
/// convention — callers reach everything through associated functions.
pub struct FreeAgentMarketAuditor;

impl FreeAgentMarketAuditor {
    /// Diagnose a single pool player. `None` when the id isn't in the
    /// global free-agent pool. Entry point for ad-hoc tooling and
    /// tests; the simulation itself only drives `log_long_term`.
    #[allow(dead_code)]
    pub fn diagnose(
        data: &SimulatorData,
        player_id: u32,
        date: NaiveDate,
    ) -> Option<FreeAgentMarketDiagnosis> {
        let player = data.free_agents.iter().find(|p| p.id == player_id)?;
        Some(Self::diagnose_player(data, player, date))
    }

    /// Diagnose every pool player free for at least `min_days_free`
    /// days. The workhorse behind the monthly debug dump.
    pub fn diagnose_long_term(
        data: &SimulatorData,
        date: NaiveDate,
        min_days_free: i64,
    ) -> Vec<FreeAgentMarketDiagnosis> {
        data.free_agents
            .iter()
            .filter(|p| {
                p.free_agent_state()
                    .map(|s| (date - s.free_since).num_days() >= min_days_free)
                    .unwrap_or(false)
            })
            .map(|p| Self::diagnose_player(data, p, date))
            .collect()
    }

    /// Monthly debug dump: one line per 12-month-plus free agent with
    /// the top reason they're still available. Costs a world scan per
    /// player, so it bails out entirely unless debug logging is on.
    pub fn log_long_term(data: &SimulatorData, date: NaiveDate) {
        if !log::log_enabled!(log::Level::Debug) {
            return;
        }
        for d in Self::diagnose_long_term(data, date, 365) {
            debug!(
                "long-term free agent: {} (id {}) — {} days free, stage {:?}, cp {:.2}, \
                 CA {}, age {}, ref-rep {}, last salary {}, offers30d {}, rejected {}, \
                 reason {}, eligible: {} countries / {} clubs / {} matching requests — {}",
                d.player_name,
                d.player_id,
                d.days_free,
                d.market_stage,
                d.career_pressure,
                d.ability,
                d.age,
                d.reference_reputation,
                d.last_salary,
                d.offers_received_30d,
                d.offers_rejected_total,
                d.last_block_reason.map(|r| r.label()).unwrap_or("none"),
                d.eligible_country_count,
                d.eligible_club_count,
                d.matching_request_count,
                d.explanation(),
            );
        }
    }

    fn diagnose_player(
        data: &SimulatorData,
        player: &Player,
        date: NaiveDate,
    ) -> FreeAgentMarketDiagnosis {
        // Two-stage nationality resolve, mirroring
        // `snapshot_global_free_agents` — active country first, the
        // lighter `country_info` map second, fail-closed fallback last.
        let nationality = data
            .country(player.country_id)
            .map(|c| (c.reputation, c.continent_id, c.code.clone()))
            .or_else(|| {
                data.country_info
                    .get(&player.country_id)
                    .map(|c| (c.reputation, c.continent_id, c.code.clone()))
            });
        let nationality_unknown = nationality.is_none();
        let (nat_rep, nat_continent, nat_code) =
            nationality.unwrap_or((u16::MAX, 1, "gb".to_string()));

        let state = player.free_agent_state();
        let days_free = state
            .map(|s| (date - s.free_since).num_days().max(0))
            .unwrap_or(0);
        let age = player.age(date);
        let ca = player.player_attributes.current_ability;
        let career_pressure = player.career_pressure(date);
        let reference_reputation = player.reference_reputation(nat_rep);
        let group = player.position().position_group();
        let player_region_prestige =
            ScoutingRegion::from_country(nat_continent, &nat_code).league_prestige();

        let rep_drop = FreeAgentMarketCalculator::rep_drop_allowed(career_pressure, age, ca);
        let region_drop = FreeAgentMarketCalculator::region_drop_allowed(career_pressure);

        let mut eligible_country_count = 0usize;
        let mut eligible_club_count = 0usize;
        let mut matching_request_count = 0usize;

        for continent in &data.continents {
            for country in &continent.countries {
                // Country-level gates, mirroring the request-driven
                // matcher's filter exactly.
                if (country.reputation as i32 + rep_drop) < reference_reputation as i32 {
                    continue;
                }
                let buyer_region_prestige =
                    ScoutingRegion::from_country(country.continent_id, &country.code)
                        .league_prestige();
                if FreeAgentMarketCalculator::cross_continent_blocked(
                    nat_continent == country.continent_id,
                    player_region_prestige,
                    buyer_region_prestige,
                    career_pressure,
                    0.85,
                ) {
                    continue;
                }
                if player_region_prestige > buyer_region_prestige + region_drop {
                    continue;
                }
                eligible_country_count += 1;

                for club in &country.clubs {
                    if club.teams.teams.is_empty() || !can_club_accept_player(club) {
                        continue;
                    }
                    let club_score = club
                        .teams
                        .main()
                        .or_else(|| club.teams.teams.first())
                        .map(|t| (t.reputation.world as f32 / 10_000.0).clamp(0.0, 1.0))
                        .unwrap_or(0.0);
                    let min_ca = FreeAgentMarketCalculator::min_acceptable_ca(
                        club_score,
                        group,
                        career_pressure,
                    );
                    let max_ca = FreeAgentMarketCalculator::max_acceptable_ca(
                        club_score,
                        group,
                        career_pressure,
                    );
                    if ca < min_ca || ca > max_ca {
                        continue;
                    }
                    eligible_club_count += 1;
                    matching_request_count += club
                        .transfer_plan
                        .transfer_requests
                        .iter()
                        .filter(|r| {
                            r.status != TransferRequestStatus::Fulfilled
                                && r.status != TransferRequestStatus::Abandoned
                                && r.position.position_group() == group
                        })
                        .count();
                }
            }
        }

        // Stored funnel reason wins (it's the most specific); fall back
        // to the structural classifications so a player no matcher has
        // touched yet still gets an answer.
        let stored = state.and_then(|s| s.last_block).map(|(_, reason)| reason);
        let last_block_reason = stored.or_else(|| {
            if nationality_unknown {
                Some(FreeAgentBlockReason::UnknownNationality)
            } else if matching_request_count == 0 {
                Some(FreeAgentBlockReason::NoMatchingRequest)
            } else {
                None
            }
        });

        FreeAgentMarketDiagnosis {
            player_id: player.id,
            player_name: player.full_name.to_string(),
            days_free,
            market_stage: player.market_stage(date),
            career_pressure,
            ability: ca,
            age,
            reference_reputation,
            last_salary: state.map(|s| s.last_salary).unwrap_or(0),
            offers_received_30d: state.map(|s| s.offers_received_30d(date)).unwrap_or(0),
            offers_rejected_total: state.map(|s| s.offers_rejected_total).unwrap_or(0),
            last_block_reason,
            eligible_country_count,
            eligible_club_count,
            matching_request_count,
        }
    }
}

/// One days-free cohort in the monthly pool aggregate. The five buckets
/// mirror the `MarketStage` boundaries (0-30 Fresh, 31-90 Open, 91-180
/// Flexible, 181-365 Desperate, 365+ LastChance) so the log reads against
/// the same timeline the decay model uses.
#[derive(Debug, Clone, Copy)]
pub struct FreeAgentPoolBucketStat {
    pub label: &'static str,
    pub count: usize,
    /// Mean `career_pressure` of the players in this bucket (0 when empty).
    pub avg_career_pressure: f32,
}

/// Aggregate snapshot of the global free-agent pool, emitted monthly so an
/// operator can watch the pool's size, its days-free distribution, and the
/// dominant reasons long-term players stay unsigned — the question the
/// per-player `log_long_term` dump answers one line at a time, rolled up.
#[derive(Debug, Clone)]
pub struct FreeAgentPoolStats {
    pub total: usize,
    /// Indexed cohorts: [0-30, 31-90, 91-180, 181-365, 365+].
    pub buckets: [FreeAgentPoolBucketStat; 5],
    /// Block-reason histogram over every pool player carrying a recorded
    /// `last_block`, highest count first. The matcher's funnel reason —
    /// "why was he passed over last".
    pub block_reason_counts: Vec<(FreeAgentBlockReason, usize)>,
    /// This period's pool in/out flow split by route: signed from the
    /// global pool / off a domestic expiry / on a pre-contract, plus
    /// released into and retired out of the pool. Lets a long run tell
    /// apart players saved by pre-contracts, signed off the open pool, and
    /// still leaking into long-term free agency.
    pub flow: FreeAgentFlowCounters,
}

impl FreeAgentMarketAuditor {
    /// Days-free → bucket index. Boundaries match `MarketStage`.
    fn pool_bucket_index(days_free: i64) -> usize {
        match days_free {
            d if d <= 30 => 0,
            d if d <= 90 => 1,
            d if d <= 180 => 2,
            d if d <= 365 => 3,
            _ => 4,
        }
    }

    /// Roll the whole `data.free_agents` pool into a single monthly
    /// aggregate. The in/out flow split (`data.free_agent_flow`) is carried
    /// across the month by the execution / sweep / retirement passes — it
    /// can't be recovered from a point-in-time scan of the surviving pool,
    /// since signed / released / retired players have already moved.
    pub fn aggregate(data: &SimulatorData, date: NaiveDate) -> FreeAgentPoolStats {
        const LABELS: [&str; 5] = ["0-30", "31-90", "91-180", "181-365", "365+"];
        let mut counts = [0usize; 5];
        let mut pressure_sums = [0.0f32; 5];
        let mut reason_counts: HashMap<FreeAgentBlockReason, usize> = HashMap::new();

        for player in &data.free_agents {
            let Some(state) = player.free_agent_state() else {
                continue;
            };
            let days_free = (date - state.free_since).num_days().max(0);
            let idx = Self::pool_bucket_index(days_free);
            counts[idx] += 1;
            pressure_sums[idx] += player.career_pressure(date);
            if let Some((_, reason)) = state.last_block {
                *reason_counts.entry(reason).or_insert(0) += 1;
            }
        }

        let buckets = std::array::from_fn(|i| FreeAgentPoolBucketStat {
            label: LABELS[i],
            count: counts[i],
            avg_career_pressure: if counts[i] > 0 {
                pressure_sums[i] / counts[i] as f32
            } else {
                0.0
            },
        });

        let mut block_reason_counts: Vec<(FreeAgentBlockReason, usize)> =
            reason_counts.into_iter().collect();
        // Highest count first; rank as a stable tiebreak so the order is
        // deterministic across runs (HashMap iteration isn't).
        block_reason_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.rank().cmp(&a.0.rank())));

        FreeAgentPoolStats {
            total: data.free_agents.len(),
            buckets,
            block_reason_counts,
            flow: data.free_agent_flow,
        }
    }

    /// Monthly aggregate log line. Like `log_long_term`, bails before the
    /// scan unless debug logging is on. The month's flow split is read
    /// from `data.free_agent_flow`.
    pub fn log_pool_stats(data: &SimulatorData, date: NaiveDate) {
        if !log::log_enabled!(log::Level::Debug) {
            return;
        }
        let stats = Self::aggregate(data, date);
        let buckets: Vec<String> = stats
            .buckets
            .iter()
            .map(|b| format!("{}: {} (cp {:.2})", b.label, b.count, b.avg_career_pressure))
            .collect();
        let reasons: Vec<String> = stats
            .block_reason_counts
            .iter()
            .take(5)
            .map(|(r, n)| format!("{}={}", r.label(), n))
            .collect();
        let flow = stats.flow;
        debug!(
            "free-agent pool: {} total | days-free [{}] | signed: {} global + {} domestic-expiry \
             + {} pre-contract | released {} / retired {} this month | top block reasons: {}",
            stats.total,
            buckets.join(", "),
            flow.signed_from_global_pool,
            flow.signed_same_country_expired,
            flow.signed_pre_contract,
            flow.released_to_pool,
            flow.retired_from_pool,
            if reasons.is_empty() {
                "none".to_string()
            } else {
                reasons.join(", ")
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::country::result::transfers::snapshot_global_free_agents;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::transfers::pipeline::{TransferNeedPriority, TransferNeedReason, TransferRequest};
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes,
        PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, StaffCollection, Team, TeamCollection, TeamReputation, TeamType,
        TrainingSchedule,
    };
    use chrono::NaiveTime;

    struct AuditFixtures;

    impl AuditFixtures {
        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn pool_player(id: u32, country_id: u32, today: NaiveDate) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = 80;
            attrs.potential_ability = 90;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Pool".to_string(), format!("P{id}")))
                .birth_date(today - chrono::Duration::days(27 * 365))
                .country_id(country_id)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 16,
                    }],
                })
                .player_attributes(attrs)
                .build()
                .unwrap()
        }

        fn team(players: Vec<crate::Player>) -> Team {
            Team::builder()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("FC".to_string())
                .slug("fc".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(2000, 2000, 4000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn club(main: Team) -> Club {
            Club::new(
                100,
                "FC".to_string(),
                Location::new(1),
                ClubFinances::new(1_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![main]),
                ClubFacilities::default(),
            )
        }

        fn country(clubs: Vec<Club>) -> Country {
            Country::builder()
                .id(1)
                .code("en".to_string())
                .slug("england".to_string())
                .name("England".to_string())
                .continent_id(1)
                .reputation(5000)
                .leagues(LeagueCollection::new(vec![League::new(
                    1,
                    "L".to_string(),
                    "english".to_string(),
                    1,
                    5000,
                    LeagueSettings {
                        season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                        season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                        tier: 1,
                        promotion_spots: 0,
                        relegation_spots: 0,
                        league_group: None,
                    },
                    false,
                )]))
                .clubs(clubs)
                .build()
                .unwrap()
        }

        fn simulator(
            today: NaiveDate,
            clubs: Vec<Club>,
            pool: Vec<crate::Player>,
        ) -> SimulatorData {
            let continent = Continent::new(
                1,
                "Europe".to_string(),
                vec![Self::country(clubs)],
                Vec::new(),
            );
            let mut data = SimulatorData::new(
                today.and_hms_opt(12, 0, 0).unwrap(),
                vec![continent],
                GlobalCompetitions::new(Vec::new()),
            );
            data.free_agents = pool;
            data
        }
    }

    #[test]
    fn unknown_nationality_fallback_is_diagnosed_not_silent() {
        // Player whose country id resolves nowhere: the snapshot
        // fail-closed fallback blocks every buyer — the diagnosis must
        // say so explicitly instead of leaving an unexplained sit.
        let today = AuditFixtures::d(2026, 6, 13);
        let player = AuditFixtures::pool_player(700, 99_999, today);
        let mut data = AuditFixtures::simulator(today, Vec::new(), vec![player]);

        let snapshot = snapshot_global_free_agents(&mut data, today);
        assert_eq!(snapshot.len(), 1);

        let diag = FreeAgentMarketAuditor::diagnose(&data, 700, today)
            .expect("pool player must be diagnosable");
        assert_eq!(
            diag.eligible_country_count, 0,
            "u16::MAX reference reputation must block every buyer country"
        );
        assert_eq!(
            diag.last_block_reason,
            Some(FreeAgentBlockReason::UnknownNationality),
            "the data hole must be named, not silent"
        );
    }

    #[test]
    fn player_with_no_open_requests_reports_no_matching_request() {
        // Known nationality, eligible country, but a world with zero
        // clubs: structurally nobody is recruiting his position.
        let today = AuditFixtures::d(2026, 6, 13);
        let player = AuditFixtures::pool_player(701, 1, today);
        let mut data = AuditFixtures::simulator(today, Vec::new(), vec![player]);
        snapshot_global_free_agents(&mut data, today);

        let diag = FreeAgentMarketAuditor::diagnose(&data, 701, today).unwrap();
        assert!(
            diag.eligible_country_count >= 1,
            "home country must pass the gates"
        );
        assert_eq!(diag.eligible_club_count, 0);
        assert_eq!(diag.matching_request_count, 0);
        assert_eq!(
            diag.last_block_reason,
            Some(FreeAgentBlockReason::NoMatchingRequest)
        );
    }

    #[test]
    fn eligibility_counters_see_open_capacity_club_and_matching_request() {
        let today = AuditFixtures::d(2026, 6, 13);
        let player = AuditFixtures::pool_player(702, 1, today);

        let main = AuditFixtures::team(Vec::new());
        let mut club = AuditFixtures::club(main);
        club.transfer_plan.initialized = true;
        club.transfer_plan
            .transfer_requests
            .push(TransferRequest::new(
                1,
                PlayerPositionType::MidfielderCenter,
                TransferNeedPriority::Critical,
                TransferNeedReason::SquadPadding,
                50,
                80,
                0.0,
            ));
        let mut data = AuditFixtures::simulator(today, vec![club], vec![player]);
        snapshot_global_free_agents(&mut data, today);

        let diag = FreeAgentMarketAuditor::diagnose(&data, 702, today).unwrap();
        assert_eq!(diag.eligible_country_count, 1);
        assert_eq!(
            diag.eligible_club_count, 1,
            "open-capacity in-band club must count as eligible"
        );
        assert_eq!(
            diag.matching_request_count, 1,
            "the open midfielder request must be visible to the diagnosis"
        );
        assert_eq!(diag.last_block_reason, None);

        // #8: the diagnosis must turn the raw world-scan into a readable
        // answer. A club is recruiting his position and the gates pass —
        // so the story is "interest is building toward a deal", never a
        // silent unexplained sit.
        assert_eq!(diag.category(), FreeAgentStatusCategory::InterestBuilding);
        assert!(
            !diag.explanation().is_empty(),
            "diagnosis must produce a non-empty human explanation"
        );
    }

    #[test]
    fn unknown_nationality_diagnosis_explains_the_data_hole() {
        // #8 + #9: the unknown-nationality data hole must be named in the
        // human explanation, not surface as a mysterious endless sit.
        let today = AuditFixtures::d(2026, 6, 13);
        let player = AuditFixtures::pool_player(703, 99_999, today);
        let mut data = AuditFixtures::simulator(today, Vec::new(), vec![player]);
        snapshot_global_free_agents(&mut data, today);

        let diag = FreeAgentMarketAuditor::diagnose(&data, 703, today).unwrap();
        assert_eq!(diag.category(), FreeAgentStatusCategory::DataUnknown);
        assert!(!diag.explanation().is_empty());
    }

    /// #5: the monthly aggregate must classify the long-term pool by
    /// days-free cohort AND surface the dominant block reasons, so an
    /// operator can see *why* the tail is stuck without reading one line
    /// per player.
    #[test]
    fn aggregate_buckets_pool_and_ranks_block_reasons() {
        use crate::club::player::transfer::ReleaseContext;

        let today = AuditFixtures::d(2026, 6, 13);

        // Helper: a pool player free since `free_since` carrying `reason`.
        let seed = |id: u32, free_since: NaiveDate, reason: Option<FreeAgentBlockReason>| {
            let mut p = AuditFixtures::pool_player(id, 1, today);
            p.enter_free_agent_market(ReleaseContext {
                date: free_since,
                last_club_id: Some(10),
                last_country_id: Some(1),
                last_country_reputation: 4000,
                last_league_reputation: 4000,
                last_club_reputation_score: 0.4,
                last_salary: 50_000,
                last_squad_status: crate::PlayerSquadStatus::FirstTeamSquadRotation,
            });
            if let Some(r) = reason {
                p.on_market_blocked(today, r);
            }
            p
        };

        let pool = vec![
            // 365+ cohort, no offers ever reach his position.
            seed(
                900,
                today - chrono::Duration::days(400),
                Some(FreeAgentBlockReason::NoMatchingRequest),
            ),
            // 181-365 cohort, two players whose offers were declined.
            seed(
                901,
                today - chrono::Duration::days(200),
                Some(FreeAgentBlockReason::AcceptanceRollFailed),
            ),
            seed(
                902,
                today - chrono::Duration::days(220),
                Some(FreeAgentBlockReason::AcceptanceRollFailed),
            ),
            // Fresh cohort, no recorded reason yet.
            seed(903, today - chrono::Duration::days(10), None),
        ];
        let mut data = AuditFixtures::simulator(today, Vec::new(), pool);
        data.free_agent_flow.signed_from_global_pool = 7;
        data.free_agent_flow.retired_from_pool = 2;

        let stats = FreeAgentMarketAuditor::aggregate(&data, today);
        assert_eq!(stats.total, 4);
        assert_eq!(stats.flow.signed_from_global_pool, 7);
        assert_eq!(stats.flow.retired_from_pool, 2);

        // Days-free cohorts: [0-30, 31-90, 91-180, 181-365, 365+].
        assert_eq!(stats.buckets[0].count, 1, "fresh player in 0-30 bucket");
        assert_eq!(stats.buckets[3].count, 2, "two players in 181-365 bucket");
        assert_eq!(stats.buckets[4].count, 1, "long-term player in 365+ bucket");
        // The 365+ cohort's mean pressure must be materially higher than
        // the fresh cohort's — the bucket carries the decay signal.
        assert!(
            stats.buckets[4].avg_career_pressure > stats.buckets[0].avg_career_pressure,
            "long-term cohort must show higher mean career pressure"
        );

        // Block-reason histogram, highest count first.
        assert_eq!(
            stats.block_reason_counts.first().map(|(r, n)| (*r, *n)),
            Some((FreeAgentBlockReason::AcceptanceRollFailed, 2)),
            "the two declined offers must be the top block reason"
        );
        assert!(
            stats
                .block_reason_counts
                .iter()
                .any(|(r, n)| *r == FreeAgentBlockReason::NoMatchingRequest && *n == 1),
            "the no-matching-request player must also be classified"
        );
    }
}
