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
use crate::club::player::transfer::{FreeAgentBlockReason, MarketStage};
use crate::simulator::SimulatorData;
use crate::transfers::pipeline::TransferRequestStatus;
use crate::transfers::scouting_region::ScoutingRegion;
use chrono::NaiveDate;
use log::debug;

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
                 reason {}, eligible: {} countries / {} clubs / {} matching requests",
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
    }
}
