use crate::club::player::calculators::{FreeAgentReleaseReason, WageCalculator};
use crate::club::player::transfer::ReleaseContext;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::deal::reason::TransferReason;
use crate::transfers::pipeline::approach::ApproachPass;
use crate::transfers::{CompletedTransfer, TransferType};
use crate::world::SimulatorData;
use crate::world::bootstrap::ClubIdentity;
use crate::{Person, Player, PlayerSquadStatus, PlayerStatusType};
use rayon::prelude::*;
use std::collections::HashMap;

impl SimulatorData {
    /// Move every team-attached player whose main-club contract is `None`
    /// onto the global `free_agents` pool. Several pipelines (positional
    /// surplus, unresolved-salary "free transfer", contract expiry) clear
    /// the contract in place; without this sweep the player lingers on the
    /// roster as a "free agent on a team," which the player page renders
    /// inconsistently — the header reads the team name while the contract
    /// panel reads "Free Agent."
    ///
    /// Each move is logged as a `CompletedTransfer` (zero fee, `Free`
    /// type) on the losing club's country, so the transfer history page
    /// reflects the departure. Reason is derived from the player's
    /// status: `Frt` set means the club explicitly released early
    /// (mutual / surplus / unresolved-salary path); otherwise the
    /// contract simply expired.
    ///
    /// Once the reason is read, the player's club-transient state is
    /// reset (`reset_on_club_change`): the upstream release pipelines
    /// only clear the contract, so without this the player would sit in
    /// the pool still flagged "Listed / Loan Listed / Wants Free
    /// Transfer / Unhappy" about a club he no longer belongs to.
    ///
    /// Loanees are skipped (their `contract` is the parent-club contract
    /// and stays `Some` during the loan), as are retired players (already
    /// removed from team rosters by the retirement pipeline). Sets
    /// `dirty_player_index` so the next index rebuild picks up the moves.
    pub fn sweep_released_to_free_agents(&mut self) {
        let date = self.date.date();
        let released: Vec<Player> = self
            .continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .flat_map_iter(|country| {
                // League reputation is needed by `on_release` so the
                // player carries an accurate market-state snapshot into
                // the free-agent pool. Pre-collect once per country —
                // immutable read before the mutable club iteration
                // takes the borrow.
                let league_reputations: HashMap<u32, u16> = country
                    .leagues
                    .leagues
                    .iter()
                    .map(|l| (l.id, l.reputation))
                    .collect();
                let country_id = country.id;
                let country_reputation = country.reputation;
                // Per-club main-team identity resolver — the same one the
                // history seeder uses, so a released player's spell is
                // marked departed under the exact slug it was seeded with
                // (youth / Reserve squads alias to the parent Main team).
                let league_lookup = ClubIdentity::league_lookup(country);
                let mut released_in_country: Vec<Player> = Vec::new();
                let mut new_history: Vec<CompletedTransfer> = Vec::new();
                for club in &mut country.clubs {
                    let club_id = club.id;
                    let identity = ClubIdentity::resolve(club, &league_lookup);
                    for team in &mut club.teams.teams {
                        let release_team_info = identity.team_info_for(team);
                        let team_id = team.id;
                        let team_name = team.name.clone();
                        let team_reputation_world = team.reputation.world;
                        let team_league_reputation = team
                            .league_id
                            .and_then(|lid| league_reputations.get(&lid).copied())
                            .unwrap_or(country_reputation);
                        let candidates: Vec<(u32, String, bool)> = team
                            .players
                            .players
                            .iter()
                            .filter(|p| p.contract.is_none() && !p.is_on_loan() && !p.retired)
                            .map(|p| {
                                let was_released_early = p.statuses.has(PlayerStatusType::Frt);
                                (p.id, p.full_name.to_string(), was_released_early)
                            })
                            .collect();
                        for (id, player_name, released_early) in candidates {
                            if let Some(mut p) = team.players.take_player(&id) {
                                // Prefer the explicit reason the release path
                                // recorded; fall back to the legacy Frt-vs-no-Frt
                                // inference so an older save (or a path that
                                // didn't stamp a reason) still reads sensibly:
                                // an Frt marker without a reason is a generic
                                // mutual release, no marker is a plain expiry.
                                let reason = TransferReason::key(
                                    p.release_reason()
                                        .unwrap_or(if released_early {
                                            FreeAgentReleaseReason::MutualTermination
                                        } else {
                                            FreeAgentReleaseReason::ContractExpired
                                        })
                                        .history_reason(),
                                );
                                new_history.push(
                                    CompletedTransfer::new(
                                        id,
                                        player_name,
                                        club_id,
                                        team_id,
                                        team_name.clone(),
                                        0,
                                        "Free Agent".to_string(),
                                        date,
                                        CurrencyValue::new(0.0, Currency::Usd),
                                        TransferType::Free,
                                    )
                                    .with_reason(reason),
                                );
                                // Stamp the player's market-state
                                // snapshot at the moment they enter the
                                // pool. `last_salary` is unrecoverable
                                // here (the contract was already cleared
                                // upstream), so seed from the wage
                                // calculator using the team / league
                                // tiers as a faithful replacement.
                                let last_squad_status = PlayerSquadStatus::FirstTeamSquadRotation;
                                let club_score =
                                    (team_reputation_world as f32 / 10_000.0).clamp(0.0, 1.0);
                                let last_salary = WageCalculator::expected_annual_wage(
                                    &p,
                                    p.age(date),
                                    club_score,
                                    team_league_reputation,
                                );
                                // A complete release fires BOTH halves: the
                                // stats-history side (snapshot in-flight match
                                // stats onto the source-club spell and mark it
                                // departed) and the market-state side below.
                                // Skipping `on_release` leaves the current-club
                                // entry `departed_date: None`, so a later
                                // same-season signing becomes a second "active"
                                // spell the History projection hides as a
                                // phantom — and the un-drained cup / friendly
                                // buckets trip `on_free_agent_signing`.
                                p.on_release(&release_team_info, date);
                                if p.free_agent_state().is_none() {
                                    p.enter_free_agent_market(ReleaseContext {
                                        date,
                                        last_club_id: Some(club_id),
                                        last_country_id: Some(country_id),
                                        last_country_reputation: country_reputation,
                                        last_league_reputation: team_league_reputation,
                                        last_club_reputation_score: club_score,
                                        last_salary,
                                        last_squad_status,
                                    });
                                }
                                // The register's side of the exit: the day a
                                // career spell ends and the player becomes a
                                // free agent used to leave no row at all —
                                // his page jumped from old club to new club
                                // with the months in between unexplained.
                                p.decision_history.add(
                                    date,
                                    format!("{team_name} →"),
                                    "dec_contract_expired_free_agent".to_string(),
                                    String::new(),
                                );
                                // The spell at the old club is over —
                                // strip transfer statuses and the
                                // unhappiness they were attached to,
                                // exactly like a completed transfer
                                // would. `released_early` consumed the
                                // `Frt` marker above, so nothing here
                                // still needs it.
                                p.reset_on_club_change();
                                released_in_country.push(p);
                            }
                        }
                    }
                }
                country.transfer_market.transfer_history.extend(new_history);
                released_in_country
            })
            .collect();
        if !released.is_empty() {
            self.dirty_player_index = true;
            // Scrub the world of stale market state for the departed
            // players: open listings end Cancelled, team transfer lists
            // drop their rows, scouting/shortlist/monitoring interest is
            // cleared and live negotiations rejected — the same
            // chokepoint a completed transfer runs through, in its
            // release flavour. Without this a released player kept an
            // Available country-market listing pointing at a club he no
            // longer plays for.
            let released_ids: Vec<u32> = released.iter().map(|p| p.id).collect();
            ApproachPass::cleanup_player_release_interest_batch(self, &released_ids);
            // Monthly diagnostics flow counter — every swept player is one
            // that leaked out of a roster into the pool this period.
            self.free_agent_flow.released_to_pool = self
                .free_agent_flow
                .released_to_pool
                .saturating_add(released.len() as u32);
            self.free_agents.extend(released);
        }
    }
}

#[cfg(test)]
mod free_agent_release_reason_tests {
    //! The free-agent sweep must record a *specific* transfer-history
    //! reason per exit: a squad-surplus walk-out, a natural contract
    //! expiry, and a legacy `Frt`-without-reason must read differently —
    //! never all collapsed into "released by mutual agreement".
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes,
        PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, PlayerStatusType, StaffCollection, TeamBuilder, TeamCollection,
        TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::{NaiveDate, NaiveTime};

    struct SweepFx;

    impl SweepFx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap()
        }

        /// A contractless senior already sitting on the roster awaiting the
        /// sweep — the upstream release path has cleared the contract.
        fn player(id: u32) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = 80;
            attrs.potential_ability = 80;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Free".to_string(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(1994, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(None)
                .build()
                .unwrap()
        }

        fn sim(players: Vec<Player>) -> SimulatorData {
            let team = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".to_string())
                .slug("main".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap();
            let club = Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![team]),
                ClubFacilities::default(),
            );
            let league = League::new(
                1,
                "L".to_string(),
                "l".to_string(),
                1,
                500,
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
            let country = Country::builder()
                .id(1)
                .code("EN".to_string())
                .slug("en".to_string())
                .name("England".to_string())
                .continent_id(1)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(vec![club])
                .build()
                .unwrap();
            let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
            SimulatorData::new(
                Self::date().and_hms_opt(12, 0, 0).unwrap(),
                vec![continent],
                GlobalCompetitions::new(Vec::new()),
            )
        }

        fn reason_for(data: &SimulatorData, player_id: u32) -> String {
            data.country(1)
                .unwrap()
                .transfer_market
                .transfer_history
                .iter()
                .find(|t| t.player_id == player_id)
                .map(|t| t.reason.key.clone())
                .unwrap_or_default()
        }
    }

    #[test]
    fn sweep_records_distinct_reasons_per_exit() {
        let date = SweepFx::date();

        // Squad-surplus free release: contract cleared, Frt + explicit reason.
        let mut surplus = SweepFx::player(1);
        surplus.statuses.add(date, PlayerStatusType::Frt);
        surplus.set_release_reason(FreeAgentReleaseReason::SurplusFreeRelease);

        // Natural expiry: the contract simply lapsed — no Frt, no reason.
        let expired = SweepFx::player(2);

        // Legacy / fallback: Frt with no recorded reason (older save or a
        // path that didn't stamp one).
        let mut legacy = SweepFx::player(3);
        legacy.statuses.add(date, PlayerStatusType::Frt);

        let mut data = SweepFx::sim(vec![surplus, expired, legacy]);
        data.sweep_released_to_free_agents();

        assert_eq!(
            SweepFx::reason_for(&data, 1),
            "dec_reason_released_surplus",
            "an explicit surplus release must record its own reason"
        );
        assert_eq!(
            SweepFx::reason_for(&data, 2),
            "dec_reason_contract_expired",
            "a natural expiry must NOT read as a release"
        );
        assert_eq!(
            SweepFx::reason_for(&data, 3),
            "dec_reason_released_free",
            "a legacy Frt with no reason falls back to a generic mutual release"
        );

        // Every cleared-contract player reaches the global pool.
        assert_eq!(data.free_agents.len(), 3, "all three must enter the pool");
    }

    /// A player whose contract was cleared this tick is swept into
    /// `data.free_agents` in Phase C — AFTER the tick's cross-country
    /// matching has already run against the snapshot built before Phase A.
    /// His GLOBAL visibility therefore carries one tick of latency: the
    /// first snapshot that includes him is the one the NEXT tick builds
    /// before its matching phase. This test pins that contract (the
    /// post-sweep snapshot carries him with a seeded market state) and
    /// documents the latency — it does NOT claim same-tick global matching.
    /// (His own country's market released him in Phase A and could sign him
    /// THIS tick; only cross-country clubs wait for the next tick's
    /// snapshot.)
    #[test]
    fn newly_expired_player_is_visible_in_post_sweep_global_snapshot() {
        use crate::country::result::transfers::GlobalFreeAgentPool;

        let date = SweepFx::date();
        // A contractless senior awaiting the sweep (contract already
        // cleared upstream this tick).
        let expired = SweepFx::player(7);
        let mut data = SweepFx::sim(vec![expired]);

        // Before the sweep he is still on his club roster, NOT in the pool,
        // so the global snapshot can't see him yet.
        let pre = GlobalFreeAgentPool::snapshot(&mut data, date);
        assert!(
            !pre.iter().any(|s| s.player_id == 7),
            "an un-swept player must not yet appear in the global snapshot"
        );

        // The deterministic post-sweep pass moves him into the pool…
        data.sweep_released_to_free_agents();
        assert!(data.free_agents.iter().any(|p| p.id == 7));

        // …and the snapshot rebuilt AFTER the sweep carries him. Production
        // rebuilds this snapshot at the START of the next tick (before its
        // matching phase), so cross-country clubs first act on him one tick
        // after the sweep — never the same tick he was swept.
        let post = GlobalFreeAgentPool::snapshot(&mut data, date);
        let row = post
            .iter()
            .find(|s| s.player_id == 7)
            .expect("newly expired player must be visible in the post-sweep global snapshot");
        // And he carries a seeded market state (days_free read from the
        // sweep's `free_since`), so the matcher's gates have real inputs.
        assert!(row.days_free >= 0);
    }
}
