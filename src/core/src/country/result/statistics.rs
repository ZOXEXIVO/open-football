use super::CountryResult;
use crate::Country;
use crate::TeamInfo;
use crate::league::Season;
use chrono::{Datelike, NaiveDate};
use log::info;
use rayon::prelude::*;
use std::collections::HashMap;

impl CountryResult {
    /// Country-local season snapshot. Operates on `&mut Country` only —
    /// no cross-country reads or writes — so the orchestrator can run it
    /// across every just-ended-season country in parallel (the season is
    /// staggered, but several leagues share an end date, so the country
    /// dimension is worth fanning out; each country's club walk is
    /// already parallel internally).
    ///
    /// Catches up from the per-country watermark
    /// (`Country::last_snapshotted_season_year`): if today's
    /// `ended_season` is N years past the watermark, the snapshot fires
    /// N times, oldest first, so a year that slipped through the
    /// `new_season_started` league gate (regen failure, all leagues
    /// briefly inactive, fixture-skip year, etc.) still yields a row
    /// per season the player existed at the club. After each season is
    /// processed the watermark advances so future ticks don't redo
    /// already-frozen years.
    pub(crate) fn snapshot_country(country: &mut Country, date: NaiveDate) {
        // The just-ended season is identified by CALENDAR year, matching
        // how the schedule defines a season: it regenerates on the
        // league's configured season-start day with `Season::new(date.year())`
        // (see `Schedule::simulate`), and this snapshot fires on that same
        // tick via the `new_season_started` gate. So the season that just
        // ended started in `date.year() - 1`.
        //
        // Deriving the year from `Season::from_date` instead would bake in
        // a fixed Aug–Jul boundary and mislabel by one full year for any
        // league whose season does not start in August: when the start day
        // falls in Jan–Jul (e.g. a spring/summer-start league), the regen
        // tick's `from_date` still resolves to the season that just ended,
        // so `target_ended_year` would point one season too early and the
        // freshly-finished campaign would be frozen under the prior season's
        // label — the user-reported "loan moved to the previous season"
        // bug. For August-start leagues the regen tick is in August, where
        // `date.year()` and `from_date(date).start_year` agree, so this is
        // a no-op for them.
        let target_ended_year = (date.year() as u16).saturating_sub(1);

        // Decide the inclusive range of season-years to catch up on.
        // The very first call (no watermark) snapshots only
        // `target_ended_year`, matching the long-standing behavior the
        // existing tests rely on. Subsequent calls advance from
        // `watermark + 1` so any year whose `new_season_started` gate
        // dropped is recovered in chronological order when the next
        // gate event eventually fires.
        let first_year = match country.last_snapshotted_season_year {
            Some(w) => w.saturating_add(1),
            None => target_ended_year,
        };

        if first_year > target_ended_year {
            return;
        }

        for year in first_year..=target_ended_year {
            // Only the most recent catch-up iteration drains the live
            // stat buckets. Earlier missed years freeze a 0-app
            // placeholder via `on_missed_season_end`: the live buckets
            // have been accumulating across the entire gap, so draining
            // them on the first missed year would attribute the whole
            // span (e.g. two loan seasons of 40 apps each) to one early
            // row and leave the actual target year empty — exactly the
            // user-reported "2027/28 shows 80 apps and 2028/29 is
            // missing" pattern for multi-season loans whose Italian
            // 2027/28 gate dropped.
            let drain_live_stats = year == target_ended_year;
            Self::snapshot_one_season(country, Season::new(year), date, drain_live_stats);
            country.last_snapshotted_season_year = Some(year);
        }
    }

    /// Process every team in the country for a single ended season.
    /// Used by the catch-up loop above so one tick can advance the
    /// watermark across multiple years when the league gate failed for
    /// a previous year.
    ///
    /// `drain_live_stats` is `true` for the target year (the most recent
    /// season in the catch-up window) and `false` for any older missed
    /// year — see the comment at the loop site for why splitting the
    /// drain matters.
    fn snapshot_one_season(
        country: &mut Country,
        ended_season: Season,
        date: NaiveDate,
        drain_live_stats: bool,
    ) {
        info!(
            "📋 Season snapshot: saving player statistics for season {} (country {})",
            ended_season.start_year, country.id
        );

        // Build league lookup so we can resolve team.league_id -> (name, slug)
        let league_lookup: HashMap<u32, (String, String)> = country
            .leagues
            .leagues
            .iter()
            .map(|l| (l.id, (l.name.clone(), l.slug.clone())))
            .collect();

        country.clubs.par_iter_mut().for_each(|club| {
            // Resolve the main-team identity once per club so non-senior
            // squads (Reserve, U18..U23) can alias under it. The player
            // always carries a Main-team row even when they only ever
            // played for a youth squad — the user's rule is that
            // non-owning teams never appear under their own slug, but
            // the player still belongs to the parent club's main team.
            let main_team_info: Option<TeamInfo> = club.teams.main().map(|t| {
                let (league_name, league_slug) = t
                    .league_id
                    .and_then(|lid| league_lookup.get(&lid))
                    .cloned()
                    .unwrap_or_default();
                TeamInfo {
                    name: t.name.clone(),
                    slug: t.slug.clone(),
                    reputation: t.reputation.world,
                    league_name,
                    league_slug,
                }
            });

            for team in &mut club.teams.teams {
                let keeps_own_identity = team.team_type.is_own_team();

                if !keeps_own_identity {
                    // Non-senior squad: alias to the parent club's main
                    // team and call the youth-specific season-end path.
                    // It discards the accumulated youth-team match stats
                    // (U21 games don't count toward career statistics)
                    // but still drains any departed senior spells from
                    // `current` and ensures a Main-team row exists for
                    // the season.
                    let alias = match main_team_info.as_ref() {
                        Some(info) => info.clone(),
                        None => continue,
                    };
                    let (youth_league_name, youth_league_slug) = team
                        .league_id
                        .and_then(|lid| league_lookup.get(&lid))
                        .cloned()
                        .unwrap_or_default();
                    let youth_team_info = TeamInfo {
                        name: team.name.clone(),
                        slug: team.slug.clone(),
                        reputation: team.reputation.world,
                        league_name: youth_league_name,
                        league_slug: youth_league_slug,
                    };
                    for player in &mut team.players.players {
                        if drain_live_stats {
                            player.on_non_senior_season_end(
                                ended_season.clone(),
                                &alias,
                                &youth_team_info,
                                date,
                            );
                        } else {
                            player.on_missed_season_end(ended_season.clone(), &alias, date);
                        }
                        player.evaluate_favorite_club(club.id, &alias.slug, date);
                    }
                    continue;
                }

                let (league_name, league_slug) = team
                    .league_id
                    .and_then(|lid| league_lookup.get(&lid))
                    .cloned()
                    .unwrap_or_default();

                let team_info = TeamInfo {
                    name: team.name.clone(),
                    slug: team.slug.clone(),
                    reputation: team.reputation.world,
                    league_name,
                    league_slug,
                };

                for player in &mut team.players.players {
                    if drain_live_stats {
                        player.on_season_end(ended_season.clone(), &team_info, date);
                    } else {
                        player.on_missed_season_end(ended_season.clone(), &team_info, date);
                    }
                    player.evaluate_favorite_club(club.id, &team_info.slug, date);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::competitions::global::GlobalCompetitions;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::{Currency, CurrencyValue, Location};
    use crate::transfers::{TransferListing, TransferListingStatus, TransferListingType};
    use crate::{
        Club, ClubColors, ClubFinances, ClubStatus, PersonAttributes, PlayerAttributes,
        PlayerCollection, PlayerHappiness, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, PlayerStatusType, StaffCollection, TeamBuilder, TeamCollection,
        TeamReputation, TeamType, TrainingSchedule, TransferItem,
    };
    use chrono::NaiveDate;

    fn make_date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn make_player(id: u32, played: u16, goals: u16) -> crate::Player {
        let mut player = PlayerBuilder::new()
            .id(id)
            .full_name(crate::shared::fullname::FullName::new(
                "Test".to_string(),
                format!("Player{}", id),
            ))
            .birth_date(make_date(2000, 1, 1))
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            // The post-snapshot processing now reads `position()`, so
            // the test fixture must declare at least one role. A
            // central midfielder is the most neutral choice for the
            // squad-aggregation tests in this module.
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap();
        player.statistics.played = played;
        player.statistics.goals = goals;
        player
    }

    fn make_training_schedule() -> TrainingSchedule {
        use chrono::NaiveTime;
        TrainingSchedule::new(
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        )
    }

    fn make_team(
        id: u32,
        club_id: u32,
        name: &str,
        slug: &str,
        team_type: TeamType,
        league_id: Option<u32>,
        players: Vec<crate::Player>,
    ) -> crate::Team {
        TeamBuilder::new()
            .id(id)
            .league_id(league_id)
            .club_id(club_id)
            .name(name.to_string())
            .slug(slug.to_string())
            .team_type(team_type)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(make_training_schedule())
            .build()
            .unwrap()
    }

    fn make_club(id: u32, name: &str, teams: Vec<crate::Team>) -> Club {
        Club::new(
            id,
            name.to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(teams),
            crate::ClubFacilities::default(),
        )
    }

    fn make_league(id: u32, name: &str, slug: &str, friendly: bool) -> League {
        League::new(
            id,
            name.to_string(),
            slug.to_string(),
            1,
            500,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier: 1,
                promotion_spots: 0,
                relegation_spots: 0,
                league_group: None,
            },
            friendly,
        )
    }

    fn make_country(clubs: Vec<Club>, leagues: Vec<League>) -> crate::Country {
        crate::Country::builder()
            .id(1)
            .code("IT".to_string())
            .slug("italy".to_string())
            .name("Italy".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(leagues))
            .clubs(clubs)
            .build()
            .unwrap()
    }

    fn make_simulator_data(date: NaiveDate, country: crate::Country) -> SimulatorData {
        let continent =
            crate::continent::Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
        SimulatorData::new(
            date.and_hms_opt(12, 0, 0).unwrap(),
            vec![continent],
            GlobalCompetitions::new(Vec::new()),
        )
    }

    #[test]
    fn release_to_free_agents_marks_source_spell_departed() {
        // Regression (user repro: Vladislav Tereshkin, Fakel → Stumbras):
        // `sweep_released_to_free_agents` used to fire only the
        // market-state half of a release and skip the stats-history half
        // (`on_release`). That left the player's current-club spell with
        // `departed_date: None`, so a later same-season signing became a
        // second "active" spell the History projection dropped as a
        // phantom — the player's real new club vanished from History.
        let mut player = make_player(1, 0, 0);
        // Contract expired → eligible for the release sweep.
        player.contract = None;
        let main_team = make_team(
            10,
            100,
            "Fakel",
            "fakel",
            TeamType::Main,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Fakel", vec![main_team]);
        let league = make_league(1, "First Division", "first-division", false);
        let country = make_country(vec![club], vec![league]);

        // `SimulatorData::new` seeds the player's current-season spell at
        // Fakel (departed_date None).
        let mut data = make_simulator_data(make_date(2027, 7, 3), country);

        data.sweep_released_to_free_agents();

        let released = data
            .free_agents
            .iter()
            .find(|p| p.id == 1)
            .expect("released player should land in the global free-agent pool");
        let fakel_entry = released
            .statistics_history
            .current
            .iter()
            .find(|e| e.team_slug == "fakel")
            .expect("Fakel current-season spell missing");
        assert!(
            fakel_entry.departed_date.is_some(),
            "a complete release must mark the source-club spell departed"
        );
    }

    #[test]
    fn expired_contract_without_frt_records_contract_expired_reason() {
        // A contract that simply ran out (no `Frt` stamped by any release
        // pipeline) must keep recording the plain-expiry reason — only
        // club-driven early releases become "released by mutual
        // agreement".
        let date = make_date(2027, 7, 3);
        let mut player = make_player(1, 0, 0);
        player.contract = None;

        let main_team = make_team(
            10,
            100,
            "Fakel",
            "fakel",
            TeamType::Main,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Fakel", vec![main_team]);
        let league = make_league(1, "First Division", "first-division", false);
        let country = make_country(vec![club], vec![league]);
        let mut data = make_simulator_data(date, country);

        data.sweep_released_to_free_agents();

        assert!(
            data.free_agents.iter().any(|p| p.id == 1),
            "expired player must land in the global free-agent pool"
        );
        let history = &data
            .country(1)
            .expect("country 1 must exist")
            .transfer_market
            .transfer_history;
        assert_eq!(history.len(), 1, "the sweep logs exactly one departure");
        assert_eq!(
            history[0].reason, "dec_reason_contract_expired",
            "plain expiry must not be recorded as a mutual-agreement release"
        );
    }

    #[test]
    fn release_sweep_clears_transfer_statuses_and_unhappiness() {
        // Regression (user repro: Vladislav Panteleev, Juventus → pool):
        // the early-release pipelines (mutual termination, positional
        // surplus, unresolved salary) and the expiry sweep only clear the
        // contract, so the player used to enter the free-agent pool still
        // flagged Lst / Loa / Frt / Unh — rendering a clubless player as
        // "Listed · Loan Listed · Wants Free Transfer" and unhappy. The
        // sweep must reset that state, while still consuming `Frt` for
        // the released-early history reason.
        let date = make_date(2027, 7, 3);
        let mut player = make_player(1, 0, 0);
        player.contract = None;
        player.statuses.add(date, PlayerStatusType::Lst);
        player.statuses.add(date, PlayerStatusType::Loa);
        player.statuses.add(date, PlayerStatusType::Unh);
        player.statuses.add(date, PlayerStatusType::Frt);
        player.happiness.adjust_morale(-30.0);

        let main_team = make_team(
            10,
            100,
            "Fakel",
            "fakel",
            TeamType::Main,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Fakel", vec![main_team]);
        let league = make_league(1, "First Division", "first-division", false);
        let country = make_country(vec![club], vec![league]);
        let mut data = make_simulator_data(date, country);

        data.sweep_released_to_free_agents();

        let released = data
            .free_agents
            .iter()
            .find(|p| p.id == 1)
            .expect("released player should land in the global free-agent pool");
        let statuses = released.statuses.get();
        for status in [
            PlayerStatusType::Lst,
            PlayerStatusType::Loa,
            PlayerStatusType::Frt,
            PlayerStatusType::Unh,
        ] {
            assert!(
                !statuses.contains(&status),
                "pool free agent must not carry the {:?} status of his old club",
                status
            );
        }
        assert_eq!(
            released.happiness.morale,
            PlayerHappiness::new().morale,
            "unhappiness must not follow the player into the pool"
        );
        let history = &data
            .country(1)
            .expect("country 1 must exist")
            .transfer_market
            .transfer_history;
        assert_eq!(history.len(), 1, "the sweep logs exactly one departure");
        assert_eq!(
            history[0].reason, "dec_reason_released_free",
            "the Frt released-early marker must drive the history reason before the reset consumes it"
        );
    }

    #[test]
    fn release_sweep_scrubs_stale_market_state() {
        // A released player must vanish from every market surface, not
        // just the roster: his open country-market listing ends Cancelled
        // (nothing was sold), the team's transfer list drops its row, and
        // the market-state snapshot is seeded for the free-agent pool.
        let date = make_date(2027, 7, 3);
        let mut player = make_player(1, 0, 0);
        player.contract = None;
        player.statuses.add(date, PlayerStatusType::Frt);

        let main_team = make_team(
            10,
            100,
            "Fakel",
            "fakel",
            TeamType::Main,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Fakel", vec![main_team]);
        let league = make_league(1, "First Division", "first-division", false);
        let mut country = make_country(vec![club], vec![league]);

        // Stale market state left over from an earlier listing pass.
        country.transfer_market.add_listing(TransferListing::new(
            1,
            100,
            10,
            CurrencyValue::new(250_000.0, Currency::Usd),
            make_date(2027, 6, 1),
            TransferListingType::Transfer,
        ));
        country.clubs[0].teams.teams[0]
            .transfer_list
            .add(TransferItem::new(
                1,
                CurrencyValue::new(250_000.0, Currency::Usd),
            ));

        let mut data = make_simulator_data(date, country);
        data.sweep_released_to_free_agents();

        let country = data.country(1).expect("country 1 must exist");
        let listing = country
            .transfer_market
            .listings
            .iter()
            .find(|l| l.player_id == 1)
            .expect("the listing row stays on record");
        assert_eq!(
            listing.status,
            TransferListingStatus::Cancelled,
            "a release must cancel the open listing — not leave it Available or fake a sale"
        );
        assert!(
            country.clubs[0].teams.teams[0]
                .transfer_list
                .listed_player_ids()
                .is_empty(),
            "the team transfer list must drop the released player"
        );
        let released = data
            .free_agents
            .iter()
            .find(|p| p.id == 1)
            .expect("released player must land in the global free-agent pool");
        assert!(
            released.free_agent_state().is_some(),
            "the sweep must seed the free-agent market state"
        );
    }

    #[test]
    fn snapshot_resets_player_stats_and_creates_history() {
        let player = make_player(1, 20, 5);
        let main_team = make_team(
            10,
            100,
            "Inter",
            "inter",
            TeamType::Main,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Inter", vec![main_team]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let player = &country.clubs[0].teams.teams[0].players.players[0];

        assert_eq!(player.statistics.played, 0);
        assert_eq!(player.statistics.goals, 0);
        let entry = player
            .statistics_history
            .items
            .iter()
            .find(|i| i.season.start_year == 2031)
            .expect("Frozen 2031 row missing");
        assert_eq!(entry.team_slug, "inter");
        assert_eq!(entry.league_slug, "serie-a");
        assert_eq!(entry.statistics.played, 20);
    }

    #[test]
    fn reserve_players_snapshot_under_main_team_alias() {
        // Reserve / U18..U23 players never appear under their own slug
        // in career history — the user's rule is that non-owning teams
        // are not tracked. Instead the snapshot writes a row under the
        // parent club's main team so the player always has a "career
        // home" entry for the season. Reserve-league appearances live
        // in `friendly_statistics` (youth/reserve leagues are friendly)
        // and are discarded — the Main row shows 0 senior games.
        let mut player = make_player(1, 0, 0);
        player.friendly_statistics.played = 10;
        player.friendly_statistics.goals = 2;
        let main_team = make_team(
            10,
            100,
            "Juventus",
            "juventus",
            TeamType::Main,
            Some(1),
            vec![],
        );
        let reserve_team = make_team(
            11,
            100,
            "Juventus B",
            "juventus-b",
            TeamType::Reserve,
            Some(2),
            vec![player],
        );
        let club = make_club(100, "Juventus", vec![main_team, reserve_team]);
        let league_main = make_league(1, "Serie A", "serie-a", false);
        let league_reserve = make_league(2, "Serie B", "serie-b", true);
        let country = make_country(vec![club], vec![league_main, league_reserve]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let reserve_player = &country.clubs[0].teams.teams[1].players.players[0];
        assert_eq!(reserve_player.statistics.played, 0);
        let entry = reserve_player
            .statistics_history
            .items
            .iter()
            .find(|i| i.season.start_year == 2031)
            .expect("reserve player should have a Main alias row");
        assert_eq!(entry.team_slug, "juventus");
        // Reserve games are discarded — Main row carries 0 senior apps.
        assert_eq!(entry.statistics.played, 0);
    }

    #[test]
    fn snapshot_processes_all_players_in_all_teams() {
        let p1 = make_player(1, 30, 10);
        let p2 = make_player(2, 15, 3);
        let p3 = make_player(3, 5, 0);

        let main_team = make_team(
            10,
            100,
            "Roma",
            "roma",
            TeamType::Main,
            Some(1),
            vec![p1, p2],
        );
        let reserve_team = make_team(
            11,
            100,
            "Roma B",
            "roma-b",
            TeamType::Reserve,
            None,
            vec![p3],
        );
        let club = make_club(100, "Roma", vec![main_team, reserve_team]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();

        for player in &country.clubs[0].teams.teams[0].players.players {
            assert_eq!(player.statistics.played, 0);
            assert_eq!(player.statistics_history.items.len(), 1);
        }
        assert_eq!(
            country.clubs[0].teams.teams[0].players.players[0]
                .statistics_history
                .items[0]
                .statistics
                .played,
            30
        );
        assert_eq!(
            country.clubs[0].teams.teams[0].players.players[1]
                .statistics_history
                .items[0]
                .statistics
                .played,
            15
        );

        let reserve_player = &country.clubs[0].teams.teams[1].players.players[0];
        assert_eq!(reserve_player.statistics.played, 0);
        // Reserve squads alias to Main: the player gets a Roma row
        // under the parent club's main slug. The 5 games on
        // `player.statistics` represent senior callup appearances
        // (reserve-league games would land in friendly_statistics) so
        // they survive into the Main row.
        let reserve_entry = &reserve_player.statistics_history.items[0];
        assert_eq!(reserve_entry.team_slug, "roma");
        assert_eq!(reserve_entry.statistics.played, 5);
    }

    #[test]
    fn snapshot_with_invalid_country_id_does_nothing() {
        let club = make_club(100, "Inter", vec![]);
        let country = make_country(vec![club], vec![]);
        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 999);
    }

    #[test]
    fn snapshot_ended_season_uses_calendar_year_for_sub_august_start() {
        // A league whose season starts before August regenerates its
        // schedule (and flips `new_season_started`) on a Jan–Jul date.
        // The just-ended season is `date.year() - 1` by the calendar-year
        // model the schedule uses. Deriving it from `Season::from_date`
        // would hardcode an Aug–Jul split and freeze the finished season
        // one year too early — the user-reported "loan moved to the
        // previous season" bug. Here the snapshot fires on 1 July 2027,
        // so the ended season must be 2026/27, not 2025/26.
        let player = make_player(1, 5, 0);
        let main_team = make_team(
            10,
            100,
            "Zenit",
            "zenit",
            TeamType::Main,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Zenit", vec![main_team]);
        let league = make_league(1, "Premier League", "rpl", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2027, 7, 1), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country(1).unwrap();
        assert_eq!(country.last_snapshotted_season_year, Some(2026));
        let entry = &country.clubs[0].teams.teams[0].players.players[0]
            .statistics_history
            .items[0];
        assert_eq!(
            entry.season.start_year, 2026,
            "season starting before August must be frozen under its calendar \
             start year, not one season earlier"
        );
    }

    #[test]
    fn snapshot_calculates_correct_ended_season() {
        let player = make_player(1, 10, 1);
        let main_team = make_team(
            10,
            100,
            "Milan",
            "milan",
            TeamType::Main,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Milan", vec![main_team]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2033, 9, 1), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let entry = &country.clubs[0].teams.teams[0].players.players[0]
            .statistics_history
            .items[0];
        assert_eq!(entry.season.start_year, 2032);
    }

    #[test]
    fn snapshot_processes_multiple_clubs() {
        let p1 = make_player(1, 20, 5);
        let p2 = make_player(2, 25, 8);

        let team1 = make_team(10, 100, "Inter", "inter", TeamType::Main, Some(1), vec![p1]);
        let team2 = make_team(20, 200, "Milan", "milan", TeamType::Main, Some(1), vec![p2]);
        let club1 = make_club(100, "Inter", vec![team1]);
        let club2 = make_club(200, "Milan", vec![team2]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club1, club2], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        assert_eq!(
            country.clubs[0].teams.teams[0].players.players[0]
                .statistics_history
                .items[0]
                .team_slug,
            "inter"
        );
        assert_eq!(
            country.clubs[1].teams.teams[0].players.players[0]
                .statistics_history
                .items[0]
                .team_slug,
            "milan"
        );
    }

    #[test]
    fn b_team_keeps_own_history_identity() {
        // B is one of the senior types ("Main, B, Second") that *do*
        // contribute to player history under their own slug.
        let player = make_player(1, 8, 1);
        let b_team = make_team(
            10,
            100,
            "Club B",
            "club-b",
            TeamType::B,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Club", vec![b_team]);
        let league = make_league(1, "Serie B", "serie-b", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let entry = &country.clubs[0].teams.teams[0].players.players[0]
            .statistics_history
            .items[0];
        assert_eq!(entry.team_slug, "club-b");
        assert_eq!(entry.statistics.played, 8);
    }

    #[test]
    fn youth_squad_player_snapshots_under_main_team_alias() {
        // U21 (and other youth squads) never appear under their own
        // slug. A player who spent the season entirely at U21 still
        // gets a Main-team row in career history (the parent club's
        // main team) — the user's rule is that non-owning team players
        // always show a Main row each season, even with 0 games.
        // U21 league games go to `friendly_statistics` (youth leagues
        // are friendly leagues) so we set them there to model the U21
        // appearances that must be discarded.
        let mut player = make_player(1, 0, 0);
        player.friendly_statistics.played = 12;
        player.friendly_statistics.goals = 3;
        let main_team = make_team(10, 100, "Napoli", "napoli", TeamType::Main, Some(1), vec![]);
        let u21 = make_team(
            11,
            100,
            "Napoli U21",
            "napoli-u21",
            TeamType::U21,
            None,
            vec![player],
        );
        let club = make_club(100, "Napoli", vec![main_team, u21]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let u21_player = &country.clubs[0].teams.teams[1].players.players[0];
        assert_eq!(u21_player.statistics.played, 0);
        // Friendly bucket reset too so U21 games don't bleed forward.
        assert_eq!(u21_player.friendly_statistics.played, 0);
        let entry = u21_player
            .statistics_history
            .items
            .iter()
            .find(|i| i.season.start_year == 2031)
            .expect("U21-only player must still have a Main alias row");
        assert_eq!(entry.team_slug, "napoli");
        // U21 (friendly-bucket) games are discarded — Main row 0 apps.
        assert_eq!(entry.statistics.played, 0);
    }

    #[test]
    fn youth_squad_player_main_team_callups_count_toward_main_row() {
        // Bug repro: a player rostered on U21 who got called up to
        // Main for some matches saw their senior-callup games erased
        // at season end. Match stats from senior matches land in
        // `player.statistics` (not `friendly_statistics`), so the
        // non-senior season-end path must preserve them and feed them
        // into the Main-team row.
        let mut player = make_player(1, 0, 0);
        // Senior callups: 5 apps for the Main team during the season.
        player.statistics.played = 5;
        player.statistics.goals = 1;
        // Plus the regular U21 league output, which must be discarded.
        player.friendly_statistics.played = 18;
        player.friendly_statistics.goals = 4;

        let main_team = make_team(10, 100, "Napoli", "napoli", TeamType::Main, Some(1), vec![]);
        let u21 = make_team(
            11,
            100,
            "Napoli U21",
            "napoli-u21",
            TeamType::U21,
            None,
            vec![player],
        );
        let club = make_club(100, "Napoli", vec![main_team, u21]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let u21_player = &country.clubs[0].teams.teams[1].players.players[0];
        assert_eq!(
            u21_player.statistics.played, 0,
            "stats reset for new season"
        );
        let entry = u21_player
            .statistics_history
            .items
            .iter()
            .find(|i| i.season.start_year == 2031)
            .expect("U21 player with senior callups must have a Main row");
        assert_eq!(entry.team_slug, "napoli");
        assert_eq!(
            entry.statistics.played, 5,
            "senior callup apps must survive the youth-squad season-end"
        );
        assert_eq!(entry.statistics.goals, 1, "callup goals must survive too");
    }

    #[test]
    fn youth_player_promoted_mid_season_keeps_main_row_only() {
        // The bug the user reported: a player who started the season at
        // Main, was demoted to U21 mid-season, and ended the season on
        // U21 should NOT see a duplicate or aliased Main row written by
        // the snapshot. Their pre-demotion Main spell is already frozen
        // into history by the intra-club move; the snapshot must not
        // double-write under the U21 → Main alias.
        use crate::PlayerStatistics;
        use crate::club::player::statistics::CurrentSeasonEntry;

        let mut player = make_player(1, 0, 0);
        // Simulate the state after a mid-season Main → U21 demotion: a
        // departed Main entry sits in `current` carrying the player's
        // pre-demotion stats.
        let mut main_stats = PlayerStatistics::default();
        main_stats.played = 12;
        main_stats.goals = 3;
        player.statistics_history.current.push(CurrentSeasonEntry {
            team_name: "Napoli".to_string(),
            team_slug: "napoli".to_string(),
            team_reputation: 100,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: main_stats,
            joined_date: make_date(2031, 8, 1),
            departed_date: Some(make_date(2032, 1, 15)),
            seq_id: 0,
        });
        // Player is now on U21 and accumulated 5 youth-team apps that
        // must NOT bleed into history. Youth-league apps live in
        // `friendly_statistics`, not `statistics`.
        player.friendly_statistics.played = 5;
        player.friendly_statistics.goals = 1;

        let main_team = make_team(10, 100, "Napoli", "napoli", TeamType::Main, Some(1), vec![]);
        let u21 = make_team(
            11,
            100,
            "Napoli U21",
            "napoli-u21",
            TeamType::U21,
            None,
            vec![player],
        );
        let club = make_club(100, "Napoli", vec![main_team, u21]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let player = &country.clubs[0].teams.teams[1].players.players[0];
        assert_eq!(player.statistics.played, 0, "match stats must be reset");

        // Exactly one career row: the Main spell with 12 apps. No U21
        // alias, no phantom row from the youth-team snapshot.
        assert_eq!(
            player.statistics_history.items.len(),
            1,
            "expected exactly one career row from the Main spell"
        );
        let entry = &player.statistics_history.items[0];
        assert_eq!(entry.team_slug, "napoli");
        assert_eq!(entry.statistics.played, 12);
        assert_eq!(entry.statistics.goals, 3);
    }

    // Snapshot watermark catches up missed seasons. Simulates the
    // user-reported "missing 2026/27" pattern: the first snapshot
    // fires for 2031/32 (date Aug 15, 2032), then the next snapshot
    // call happens after a one-year gap (date Aug 15, 2034 — the
    // 2032/33 snapshot was dropped because the league gate failed for
    // that year). The catch-up loop must process both 2032/33 and
    // 2033/34 in order so the player has one row per season.
    #[test]
    fn snapshot_catches_up_missed_year_via_watermark() {
        let player = make_player(1, 5, 1);
        let main_team = make_team(
            10,
            100,
            "Milan",
            "milan",
            TeamType::Main,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Milan", vec![main_team]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        // After the first call: watermark is 2031, one frozen row exists.
        let country = data.country(1).unwrap();
        assert_eq!(country.last_snapshotted_season_year, Some(2031));
        let items_after_first = country.clubs[0].teams.teams[0].players.players[0]
            .statistics_history
            .items
            .len();
        assert_eq!(items_after_first, 1);

        // Advance the date by two years and play another partial season.
        // The 2032/33 snapshot was never fired (gate dropped). The next
        // call's catch-up loop should produce TWO rows: 2032 and 2033.
        let player = &mut data.continents[0].countries[0].clubs[0].teams.teams[0]
            .players
            .players[0];
        player.statistics.played = 8;
        player.statistics.goals = 2;
        data.date = make_date(2034, 8, 15).and_hms_opt(12, 0, 0).unwrap();

        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country(1).unwrap();
        assert_eq!(country.last_snapshotted_season_year, Some(2033));

        let years: Vec<u16> = country.clubs[0].teams.teams[0].players.players[0]
            .statistics_history
            .items
            .iter()
            .map(|i| i.season.start_year)
            .collect();
        assert!(
            years.contains(&2031),
            "first snapshot row missing: {:?}",
            years
        );
        assert!(
            years.contains(&2032),
            "missed-year catch-up row missing: {:?}",
            years
        );
        assert!(
            years.contains(&2033),
            "current ended-season row missing: {:?}",
            years
        );

        // Live stats were drained for the most recent ended season
        // (2033/34) — not the older missed year. The buckets had been
        // accumulating across both years so the catch-up has no per-
        // season split, but attributing the lot to the target year
        // keeps the row a user sees most prominently honest.
        let items = &country.clubs[0].teams.teams[0].players.players[0]
            .statistics_history
            .items;
        let item_2032 = items
            .iter()
            .find(|i| i.season.start_year == 2032)
            .expect("missed-year 2032 row missing");
        assert_eq!(
            item_2032.statistics.played, 0,
            "missed-year row must be an empty placeholder — live stats belong to target year"
        );
        let item_2033 = items
            .iter()
            .find(|i| i.season.start_year == 2033)
            .expect("target-year 2033 row missing");
        assert_eq!(
            item_2033.statistics.played, 8,
            "target-year row must carry the drained live stats"
        );
        assert_eq!(item_2033.statistics.goals, 2);
    }

    // Multi-season loan whose middle-year snapshot gate dropped: the
    // catch-up that re-engages the next year must not collapse the two
    // seasons' worth of live stats into the older missed row (which
    // then leaves the target-year row empty enough for the
    // `stale_loan_seed` filter in `record_season_end` to drop it
    // entirely — the user-reported "2027/28 shows 80 apps, 2028/29 is
    // missing" pattern).
    #[test]
    fn snapshot_loan_player_catchup_attributes_stats_to_target_year() {
        let mut player = make_player(1, 0, 0);
        // Mark the player as on loan BEFORE constructing the simulator
        // so the auto-seed pass in `SimulatorData::new` seeds the spell
        // with `is_loan=true` — otherwise the construct-time seed
        // writes a non-loan entry that later coexists with the loan
        // row and muddies the test signal.
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            999,
            make_date(2029, 7, 31),
            100,
            0,
            100,
        ));
        let main_team = make_team(
            10,
            100,
            "Juventus",
            "juventus",
            TeamType::Main,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Juventus", vec![main_team]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2026, 8, 1), country);
        {
            let player = &mut data.continents[0].countries[0].clubs[0].teams.teams[0]
                .players
                .players[0];
            player.statistics.played = 30;
        }
        data.date = make_date(2027, 8, 15).and_hms_opt(12, 0, 0).unwrap();

        // First snapshot: 2026/27 ends normally — drains 30 apps as the
        // 2026/27 Juventus loan row.
        CountryResult::snapshot_player_season_statistics(&mut data, 1);
        assert_eq!(
            data.country(1).unwrap().last_snapshotted_season_year,
            Some(2026)
        );

        // 2027/28 snapshot gate drops (no call here). During 2027/28 and
        // 2028/29 the player keeps playing on loan — the buckets carry
        // 40 + 40 = 80 cumulative apps by the time the next snapshot
        // fires in Aug 2029.
        {
            let player = &mut data.continents[0].countries[0].clubs[0].teams.teams[0]
                .players
                .players[0];
            player.statistics.played = 80;
            player.statistics.goals = 4;
        }
        data.date = make_date(2029, 8, 15).and_hms_opt(12, 0, 0).unwrap();

        // Second snapshot: watermark=2026, target=2028. Catch-up should
        // iterate [2027, 2028] but only attribute the 80 apps to 2028.
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let items = &data.continents[0].countries[0].clubs[0].teams.teams[0]
            .players
            .players[0]
            .statistics_history
            .items;

        // 2026/27: original 30-app loan row preserved.
        let item_2026 = items
            .iter()
            .find(|i| i.season.start_year == 2026 && i.team_slug == "juventus")
            .expect("2026 Juventus loan row missing");
        assert_eq!(item_2026.statistics.played, 30);
        assert!(item_2026.is_loan);

        // 2028/29: target-year row carries the full 80 apps.
        let item_2028 = items
            .iter()
            .find(|i| i.season.start_year == 2028 && i.team_slug == "juventus")
            .expect("target-year 2028 Juventus row missing");
        assert_eq!(
            item_2028.statistics.played, 80,
            "drained live stats must land on the target year, not the older missed year"
        );
        assert_eq!(item_2028.statistics.goals, 4);
        assert!(item_2028.is_loan);

        // 2027/28: no inflated row. A 0-app loan placeholder is filtered
        // out by `stale_loan_seed`, which is fine — the missed-year row
        // for a loaned player has no faithful representation anyway.
        let item_2027 = items
            .iter()
            .find(|i| i.season.start_year == 2027 && i.team_slug == "juventus");
        if let Some(item) = item_2027 {
            assert_eq!(
                item.statistics.played, 0,
                "missed-year placeholder must not absorb target-year stats"
            );
        }
    }
}
