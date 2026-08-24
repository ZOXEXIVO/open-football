use super::Club;
use crate::Player;
use crate::club::academy::ClubAcademy;
use crate::club::player::calculators::FreeAgentReleaseReason;
use crate::club::player::language::{Language, PlayerLanguage};
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::reason::TransferReason;
use crate::transfers::{CompletedTransfer, TransferType};
use crate::{PlayerStatusType, TeamType};
use chrono::NaiveDate;
use log::debug;

impl Club {
    /// Pre-season reset: restore player conditions and clear lingering statuses.
    /// Called once at season start so teams begin with full healthy squads.
    pub(super) fn process_pre_season_reset(&mut self) {
        for team in &mut self.teams.teams {
            for player in &mut team.players.players {
                // Restore condition to pre-season fitness level (85%)
                if player.player_attributes.condition < 8500 && !player.player_attributes.is_injured
                {
                    player.player_attributes.condition = 8500;
                }

                // Clear stale Int / IntU21 status (should have been released by
                // national team, but safety net in case tournament release was missed)
                player.statuses.remove(PlayerStatusType::Int);
                player.statuses.remove(PlayerStatusType::IntU21);

                // Reset ban flags for new season
                player.player_attributes.is_banned = false;

                // NOTE: Do NOT reset player.statistics here!
                // The season-end snapshot (snapshot_player_season_statistics) takes
                // stats via std::mem::take in on_season_end. If we reset here first,
                // the snapshot captures zeroed stats and the season's history is lost.

                // Reset days since last match (pre-season training counts)
                player.player_attributes.days_since_last_match = 7;
            }
        }
    }

    /// Weekly rescue for a youth squad that cannot put a team on the
    /// pitch.
    ///
    /// The academy → youth pathway used to have exactly one door and it
    /// opened one morning a year, on the country's season-start day.
    /// That is fine for a club whose U18 already exists; it is useless
    /// for the large majority that begin a new world with an empty or
    /// half-empty one, because the source data carries no youth records
    /// for them. Those clubs sat on a full academy — thirty-odd boys,
    /// training every week — while their U18 played short, in some
    /// countries for eleven months, because the season had started three
    /// weeks before the world did.
    ///
    /// So: when a youth squad is under eleven, the club promotes from
    /// its own academy the way a real one does, down to age fourteen,
    /// and only as far as a fielding eleven plus three substitutes.
    /// Everything about it is bounded — one rescue a month, never past
    /// the academy's own [`ClubAcademy::call_up_capacity`], and it stops
    /// the moment the squad can field a team. The seasonal graduation
    /// round is untouched: this is the club not forfeiting, not a second
    /// throughput channel.
    ///
    /// Returns the transfer records so the country layer files them the
    /// same way it files graduation day.
    pub(super) fn process_youth_emergency_callups(
        &mut self,
        date: NaiveDate,
        country_code: &str,
    ) -> Vec<CompletedTransfer> {
        let mut budget = self.academy.emergency_allowance(date);
        if budget == 0 {
            return Vec::new();
        }

        let mut transfers = Vec::new();
        let main_team_name = self
            .teams
            .main()
            .map(|t| t.name.clone())
            .unwrap_or_else(|| self.name.clone());

        // Lowest bracket first: the U18 is the academy's own next step,
        // and a club fielding both a U18 and a U20 wants its oldest
        // prospects in the former before the latter sees any of them.
        for team_type in TeamType::YOUTH_PROGRESSION {
            if budget == 0 {
                break;
            }
            let Some(idx) = self.teams.index_of_type(*team_type) else {
                continue;
            };
            let squad = self.teams.teams[idx].players.len();
            if squad >= ClubAcademy::EMERGENCY_YOUTH_SIZE {
                continue;
            }
            // Continuous in the size of the hole: an empty squad gets
            // fourteen, a squad of nine gets five. No severity ladder —
            // the deficit already is one.
            let wanted = ClubAcademy::EMERGENCY_YOUTH_TARGET
                .saturating_sub(squad)
                .min(budget);
            let called_up = self.academy.emergency_call_up(date, wanted);
            if called_up.is_empty() {
                continue;
            }
            budget = budget.saturating_sub(called_up.len());

            debug!(
                "academy {}: {} emergency call-ups into {:?} (squad was {})",
                self.name,
                called_up.len(),
                team_type,
                squad
            );

            for mut player in called_up {
                if player.languages.is_empty() {
                    player.languages = Language::from_country_code(country_code)
                        .into_iter()
                        .map(PlayerLanguage::native)
                        .collect();
                }

                transfers.push(
                    CompletedTransfer::new(
                        player.id,
                        player.full_name.to_string(),
                        0,
                        0,
                        "Academy".to_string(),
                        self.id,
                        main_team_name.clone(),
                        date,
                        CurrencyValue::new(0.0, Currency::Usd),
                        TransferType::Free,
                    )
                    .with_reason(TransferReason::key(
                        "signing_reason_academy_emergency_callup",
                    )),
                );
                self.teams.teams[idx].players.add(player);
            }
        }

        if !transfers.is_empty() {
            self.academy.record_emergency_call_up(date);
        }
        transfers
    }

    /// Graduate best academy players to U18 team (3-8 per year).
    /// Move overage youth players to main team.
    /// Aged-out academy players are released onto the global free-agent
    /// pool. Returns completed transfer records and the released
    /// player roster so the country processing layer can route them.
    pub(super) fn process_academy_graduations(
        &mut self,
        date: NaiveDate,
        country_code: &str,
    ) -> (Vec<CompletedTransfer>, Vec<Player>) {
        let mut transfers = Vec::new();
        let mut released_players: Vec<Player> = Vec::new();

        // Clean the youth squads FIRST: promote overage youth up the
        // progression (and into the main team) so room frees up before we
        // graduate. Without this, a nominally-full youth team would stall
        // academy throughput even when plenty of academy players are ready.
        self.rebalance_squads(date);

        // Prefer the lowest youth team to graduate into (U18 → U19 → U20 →
        // U21 → U23). A club with no youth team at all promotes its best
        // academy players straight onto the senior/main team rather than
        // releasing every aged-out 18-year-old for free.
        let graduation_idx = TeamType::YOUTH_PROGRESSION
            .iter()
            .find_map(|tt| self.teams.index_of_type(*tt))
            .or_else(|| self.teams.index_of_type(TeamType::Main));

        // Graduate academy players BEFORE releasing aged-out ones, so 16+
        // year olds get a chance to graduate instead of being deleted.
        //
        // Throughput target (not just "top the squad up"): a healthy
        // academy ships 5-8 graduates a season, up to 12, plus 0-2 elite
        // overshoot, always bounded by the youth soft-max of 30. The
        // academy's `recommended_graduates` / `elite_overshoot_count`
        // helpers own the actual count so there's one place to tune it.
        if let Some(idx) = graduation_idx {
            let youth_count = self.teams.teams[idx].players.len();
            let eligible_count = self.academy.graduation_candidates(date).len();
            let normal = self
                .academy
                .recommended_graduates(youth_count, eligible_count);
            let elite_overshoot = self.academy.elite_overshoot_count(date);
            let to_graduate = self
                .academy
                .graduation_ceiling(youth_count, normal, elite_overshoot);

            // Main team name for contract registration
            let main_team_name = self
                .teams
                .main()
                .map(|t| t.name.clone())
                .unwrap_or_else(|| self.name.clone());

            let youth_team_type = self.teams.teams[idx].team_type;
            let mut graduated = self.academy.graduate_to_youth(date, to_graduate);
            // Loan-ready-age safety net: also pull up prospects within a
            // year of the 18-year-old age-out release who didn't make the
            // readiness-ranked cut — but ONLY into genuine remaining room
            // under the youth soft-max. Uncapped, this net re-absorbed the
            // whole 17-year-old cohort into an already-full youth squad
            // every July, defeating the graduation ceiling entirely. The
            // overdue prospects who don't fit stay in the academy and are
            // released at 18 into the free-agent pool below.
            const LOAN_READY_ACADEMY_AGE: u8 = 17;
            let overdue_room =
                ClubAcademy::SOFT_MAX_YOUTH_SIZE.saturating_sub(youth_count + graduated.len());
            graduated.extend(self.academy.graduate_age_overdue(
                date,
                LOAN_READY_ACADEMY_AGE,
                overdue_room,
            ));
            if !graduated.is_empty() {
                debug!(
                    "academy {}: {} players graduated (contract: {}, assigned: {:?}, was {})",
                    self.name,
                    graduated.len(),
                    main_team_name,
                    youth_team_type,
                    youth_count
                );
                for mut player in graduated {
                    // Assign native languages based on player's nationality
                    if player.languages.is_empty() {
                        player.languages = Language::from_country_code(country_code)
                            .into_iter()
                            .map(|lang| PlayerLanguage::native(lang))
                            .collect();
                    }

                    transfers.push(
                        CompletedTransfer::new(
                            player.id,
                            player.full_name.to_string(),
                            0,
                            0,
                            "Academy".to_string(),
                            self.id,
                            main_team_name.clone(),
                            date,
                            CurrencyValue::new(0.0, Currency::Usd),
                            TransferType::Free,
                        )
                        .with_reason(TransferReason::key("signing_reason_academy_graduation")),
                    );
                    self.teams.teams[idx].players.add(player);
                }
            }
        }

        // Release aged-out academy players (18+) that were NOT graduated.
        // Each release records a free transfer event AND stamps the
        // player as `Frt` with a cleared contract so the global
        // free-agent pipeline picks them up — previously they exited
        // the academy but never reached the senior free-agent pool.
        let released = self.academy.release_aged_out_players(date);
        if !released.is_empty() {
            debug!(
                "academy {}: {} aged-out players released to free agents",
                self.name,
                released.len()
            );
            for player in released {
                // `release_aged_out_players` already cleared the
                // contract and stamped Frt; we only need to record
                // the transfer history line and surface the player.
                transfers.push(
                    CompletedTransfer::new(
                        player.id,
                        player.full_name.to_string(),
                        0,
                        0,
                        "Academy".to_string(),
                        0,
                        "Free Agents".to_string(),
                        date,
                        CurrencyValue::new(0.0, Currency::Usd),
                        TransferType::Free,
                    )
                    .with_reason(TransferReason::key(
                        FreeAgentReleaseReason::AcademyAgedOut.history_reason(),
                    )),
                );
                released_players.push(player);
            }
        }

        // Rebalance: overage moves, talent promotions, backfill
        self.rebalance_squads(date);

        (transfers, released_players)
    }
}

/// Graduation salary: ability sets the tier, club reputation scales it.
/// A youth graduate at Man City earns 50x what the same ability player earns in Chad.
pub(super) fn graduation_salary(current_ability: u8, club_reputation: u16) -> u32 {
    let base = match current_ability {
        0..=60 => 2_000,
        61..=80 => 5_000,
        81..=100 => 12_000,
        101..=120 => 30_000,
        121..=150 => 80_000,
        _ => 200_000,
    };

    // Club reputation multiplier: cubic curve
    let norm = (club_reputation as f64 / 10000.0).clamp(0.0, 1.0);
    let multiplier = 0.10 + 2.90 * norm * norm * norm;

    (base as f64 * multiplier).max(500.0) as u32
}

#[cfg(test)]
mod emergency_callup_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::shared::Location;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PeopleNameGeneratorData,
        PlayerCollection, PlayerGenerator, PlayerPositionType, StaffCollection, TeamBuilder,
        TeamCollection, TeamReputation, TrainingSchedule,
    };
    use chrono::Datelike;

    struct Fixture;

    impl Fixture {
        fn date() -> NaiveDate {
            // A Monday well away from any season boundary, so nothing but
            // the weekly emergency pass could be moving players.
            NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()
        }

        fn names() -> PeopleNameGeneratorData {
            PeopleNameGeneratorData {
                first_names: vec!["Test".into()],
                last_names: vec!["Prospect".into()],
                nicknames: vec![],
            }
        }

        fn prospect(age: u8) -> Player {
            let date = Self::date();
            let mut player = PlayerGenerator::generate(
                1,
                date,
                PlayerPositionType::MidfielderCenter,
                10,
                &Self::names(),
            );
            player.birth_date = NaiveDate::from_ymd_opt(date.year() - age as i32, 1, 1).unwrap();
            player.player_attributes.condition = 8500;
            player.player_attributes.jadedness = 0;
            player.player_attributes.is_injured = false;
            player
        }

        fn training_schedule() -> TrainingSchedule {
            use chrono::NaiveTime;
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        fn team(id: u32, name: &str, team_type: TeamType, players: Vec<Player>) -> crate::Team {
            TeamBuilder::new()
                .id(id)
                .league_id(Some(1))
                .club_id(100)
                .name(name.to_string())
                .slug(name.to_lowercase())
                .team_type(team_type)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(Self::training_schedule())
                .build()
                .unwrap()
        }

        /// Main + U18 + U20 — the commonest shape in the shipped world,
        /// and the one the annual round only ever feeds at one end.
        fn club(u18: usize, u20: usize, academy_ages: &[u8]) -> Club {
            let mut academy = ClubAcademy::new(8);
            for age in academy_ages {
                academy.players.add(Self::prospect(*age));
            }
            let teams = vec![
                Self::team(10, "Main", TeamType::Main, Vec::new()),
                Self::team(
                    11,
                    "U18",
                    TeamType::U18,
                    (0..u18).map(|_| Self::prospect(17)).collect(),
                ),
                Self::team(
                    12,
                    "U20",
                    TeamType::U20,
                    (0..u20).map(|_| Self::prospect(19)).collect(),
                ),
            ];
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                academy,
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(teams),
                ClubFacilities::default(),
            )
        }

        fn squad(club: &Club, team_type: TeamType) -> usize {
            club.teams
                .teams
                .iter()
                .find(|t| t.team_type == team_type)
                .map(|t| t.players.len())
                .unwrap_or(0)
        }
    }

    #[test]
    fn empty_youth_squad_is_filled_from_the_academy_the_same_week() {
        // The bug this exists for: a new world whose U18 has no players
        // and whose country's season started three weeks before the
        // simulation did, leaving a full academy and an empty team sheet
        // for eleven months.
        let date = Fixture::date();
        let mut club = Fixture::club(0, 12, &[15; 40]);

        let transfers = club.process_youth_emergency_callups(date, "en");

        assert_eq!(
            Fixture::squad(&club, TeamType::U18),
            ClubAcademy::EMERGENCY_YOUTH_TARGET,
            "an empty U18 is topped up to a fielding eleven plus subs"
        );
        assert_eq!(transfers.len(), ClubAcademy::EMERGENCY_YOUTH_TARGET);
        assert!(
            transfers
                .iter()
                .all(|t| t.reason.key == "signing_reason_academy_emergency_callup"),
            "the history line must read as an emergency call-up, not graduation day"
        );
        assert_eq!(
            Fixture::squad(&club, TeamType::U20),
            12,
            "a U20 that can already field a team is left alone"
        );
    }

    #[test]
    fn a_healthy_youth_squad_is_left_alone() {
        let date = Fixture::date();
        let mut club = Fixture::club(11, 14, &[15; 40]);
        let before = club.academy.players.players.len();

        let transfers = club.process_youth_emergency_callups(date, "en");

        assert!(transfers.is_empty(), "eleven players can field a team");
        assert_eq!(club.academy.players.players.len(), before);
    }

    #[test]
    fn both_short_squads_are_fed_lowest_bracket_first() {
        let date = Fixture::date();
        let mut club = Fixture::club(0, 0, &[15; 60]);

        club.process_youth_emergency_callups(date, "en");

        assert_eq!(
            Fixture::squad(&club, TeamType::U18),
            ClubAcademy::EMERGENCY_YOUTH_TARGET
        );
        assert_eq!(
            Fixture::squad(&club, TeamType::U20),
            ClubAcademy::EMERGENCY_YOUTH_TARGET,
            "the U20 gets the rest of the month's budget"
        );
    }

    #[test]
    fn the_rescue_runs_once_a_month_however_often_it_is_asked() {
        let date = Fixture::date();
        let mut club = Fixture::club(0, 12, &[15; 40]);

        assert!(!club.process_youth_emergency_callups(date, "en").is_empty());
        let after_first = club.academy.players.players.len();

        // Next Monday, and the one after: even a squad pushed straight
        // back under the line waits for the new month rather than
        // draining the academy every week.
        club.teams.teams[1].players.players.clear();
        for next in [
            NaiveDate::from_ymd_opt(2026, 9, 14).unwrap(),
            NaiveDate::from_ymd_opt(2026, 9, 21).unwrap(),
        ] {
            assert!(
                club.process_youth_emergency_callups(next, "en").is_empty(),
                "the academy answers one emergency a month"
            );
            assert_eq!(club.academy.players.players.len(), after_first);
        }

        let next_month = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        assert!(
            !club
                .process_youth_emergency_callups(next_month, "en")
                .is_empty(),
            "the budget reopens the following month"
        );
    }

    #[test]
    fn a_club_with_nothing_left_to_give_does_not_invent_players() {
        // Academy already at its bootstrap line: the rescue must decline
        // rather than drain it and let the backfill mint a year group.
        let date = Fixture::date();
        let mut club = Fixture::club(0, 0, &[15; 8]);
        let before = club.academy.players.players.len();

        let transfers = club.process_youth_emergency_callups(date, "en");

        assert!(transfers.is_empty());
        assert_eq!(club.academy.players.players.len(), before);
        assert_eq!(Fixture::squad(&club, TeamType::U18), 0);
    }
}
