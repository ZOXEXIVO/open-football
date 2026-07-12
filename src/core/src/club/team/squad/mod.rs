mod asset_protection;
mod contract_renewal;
mod match_squad;
mod move_guard;
mod satisfaction;

pub use asset_protection::{
    SquadAssetClass, SquadAssetContext, SquadAssetProtection, SquadEvidenceContext,
};
pub use contract_renewal::{ContractRenewalManager, WageStructureSnapshot};
pub use satisfaction::SquadSatisfaction;

pub(crate) use move_guard::MainSquadMoveGuard;

use crate::club::staff::perception::{CoachDecisionState, RecentMoveType};
use crate::utils::DateUtils;
use crate::{PlayerStatusType, Team};
use chrono::NaiveDate;

pub struct SquadManager;

pub const MIN_FIRST_TEAM_SQUAD: usize = 25;

impl SquadManager {
    /// Daily mandatory administrative moves. This sweep is deliberately
    /// narrow: it only acts on players the club would realistically pull out
    /// of the senior matchday group *right now* — a player who has agreed a
    /// move elsewhere, or a clearly surplus listed player who still has cover
    /// in his position. Being transfer-listed / unhappy on its own is a market
    /// or morale signal, **not** a reason to stop using a player in matches, so
    /// such players stay on the Main roster and remain match-selectable (the
    /// selection layer's want-away modifier shapes their minutes). All other
    /// squad decisions (recalls, swaps, performance demotions) go through the
    /// monthly AI-driven squad composition.
    pub fn manage_critical_moves(
        teams: &mut [Team],
        coach_state: &mut Option<CoachDecisionState>,
        main_idx: usize,
        reserve_idx: usize,
        date: NaiveDate,
    ) {
        let coach_name = teams[main_idx].staffs.head_coach_name();
        let demotions = Self::identify_administrative_demotions(&teams[main_idx], date);
        let max_age = teams[reserve_idx].team_type.max_age();
        let mut demotions = filter_by_age(demotions, &teams[main_idx], max_age, date);

        // Guard: never let the first team drop below minimum squad size
        let current_size = teams[main_idx].players.players.len();
        if current_size <= MIN_FIRST_TEAM_SQUAD {
            demotions.clear();
        } else if demotions.len() > current_size - MIN_FIRST_TEAM_SQUAD {
            demotions.truncate(current_size - MIN_FIRST_TEAM_SQUAD);
        }

        if !demotions.is_empty() {
            execute_moves(teams, main_idx, reserve_idx, &demotions, date);
            record_player_decisions(
                teams,
                main_idx,
                reserve_idx,
                &demotions,
                date,
                &coach_name,
                "dec_administrative_demotion",
            );
            record_moves(
                coach_state,
                &demotions,
                RecentMoveType::DemotedToReserves,
                date,
            );

            if let Some(state) = coach_state {
                state.trigger_pressure =
                    (state.trigger_pressure + 0.15 * demotions.len() as f32).clamp(0.0, 1.0);
            }
        }
    }

    /// Administrative demotions for the daily sweep. The only proactive trigger
    /// is a *clearly-surplus, transfer-listed* player; even then the move must
    /// clear [`MainSquadMoveGuard`] (real positional cover). Being listed,
    /// transfer-requested, unhappy, or having agreed a move (`Trn`) is **not**
    /// an automatic demotion any more:
    ///   * useful want-away players stay in the senior group and remain
    ///     match-selectable (the selection layer's want-away modifier shapes
    ///     their minutes);
    ///   * an agreed-transfer player stays selectable too — the importance-aware
    ///     injury-protection penalty in selection rests him in routine games but
    ///     keeps him available for the matches that matter.
    fn identify_administrative_demotions(main_team: &Team, date: NaiveDate) -> Vec<u32> {
        let guard = MainSquadMoveGuard::new(main_team, date);
        main_team
            .players
            .players
            .iter()
            .filter(|player| {
                // Proactive daily trigger: only a clearly-surplus listed player.
                player.statuses.has(PlayerStatusType::Lst) && guard.is_surplus(player)
            })
            .filter(|player| {
                guard.allow_demote_from_main(player, "administrative surplus clear-out")
            })
            .map(|player| player.id)
            .collect()
    }
}

// ─── Shared helpers ─────────────────────────────────────────────────

pub(crate) fn execute_moves(
    teams: &mut [Team],
    from_idx: usize,
    to_idx: usize,
    player_ids: &[u32],
    date: NaiveDate,
) {
    if from_idx == to_idx {
        return;
    }

    // Capture the two squads' identities once: every moved player closes
    // their spell on `from` and opens one on `to` in career history. Both
    // teams keep their own `history_info` (empty league fields are filled
    // by the web layer at render time, matching `rebalance_squads`).
    let from_info = teams[from_idx].history_info();
    let to_info = teams[to_idx].history_info();
    let from_senior = teams[from_idx].team_type.is_own_team();
    let to_senior = teams[to_idx].team_type.is_own_team();

    for &player_id in player_ids {
        // Force-selected players are pinned to their current team — admin
        // demotion, AI rotation, transfer-listing-driven moves all skip
        // them. Single point of denial for every caller of this helper.
        let locked = teams[from_idx]
            .players
            .players
            .iter()
            .find(|p| p.id == player_id)
            .map(|p| p.is_force_match_selection)
            .unwrap_or(false);
        if locked {
            continue;
        }
        if let Some(mut player) = teams[from_idx].players.take_player(&player_id) {
            // Internal squad move: migrate any team-level transfer-list entry
            // (the asking price) to the destination team rather than dropping
            // it. The player's market status (Lst/Req/Unh) lives on the player
            // and travels with him automatically; market discovery scans every
            // team and keys on that status, so an internal Main ↔ Reserve / B
            // move never removes him from the market. Dropping the entry here
            // used to desync the asking price from the player's location.
            if let Some(item) = teams[from_idx].transfer_list.take(player_id) {
                teams[to_idx].transfer_list.add(item);
            }
            // Close the previous spell and open one on the destination so
            // the player's stats accumulate against the team they actually
            // play for. Without this, a Main → Second demotion left the
            // active history entry pointing at Main, and the Second-team
            // league appearances showed under Main (no Second row at all).
            player.on_intra_club_move(&from_info, &to_info, from_senior, to_senior, date);
            teams[to_idx].players.add(player);
        }
    }
}

fn team_label(team: &Team) -> String {
    team.name.clone()
}

pub(crate) fn record_player_decisions(
    teams: &mut [Team],
    from_idx: usize,
    to_idx: usize,
    player_ids: &[u32],
    date: NaiveDate,
    decided_by: &str,
    reason: &str,
) {
    let from_label = team_label(&teams[from_idx]);
    let to_label = team_label(&teams[to_idx]);
    let movement = format!("{} → {}", from_label, to_label);
    for &pid in player_ids {
        if let Some(player) = teams[to_idx]
            .players
            .players
            .iter_mut()
            .find(|p| p.id == pid)
        {
            player.decision_history.add(
                date,
                movement.clone(),
                reason.to_string(),
                decided_by.to_string(),
            );
        }
    }
}

pub(crate) fn filter_by_age(
    ids: Vec<u32>,
    team: &Team,
    max_age: Option<u8>,
    date: NaiveDate,
) -> Vec<u32> {
    match max_age {
        Some(max) => ids
            .into_iter()
            .filter(|&pid| {
                team.players
                    .players
                    .iter()
                    .find(|p| p.id == pid)
                    .map(|p| DateUtils::age(p.birth_date, date) <= max)
                    .unwrap_or(false)
            })
            .collect(),
        None => ids,
    }
}

pub(crate) fn record_moves(
    coach_state: &mut Option<CoachDecisionState>,
    ids: &[u32],
    move_type: RecentMoveType,
    date: NaiveDate,
) {
    if let Some(state) = coach_state {
        for &id in ids {
            state.record_move(id, move_type, date);
        }
    }
}

#[cfg(test)]
mod execute_moves_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, StaffCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::NaiveTime;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_player(id: u32) -> crate::Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".to_string(), format!("Player{id}")))
            .birth_date(d(2002, 1, 1))
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn make_team(
        id: u32,
        name: &str,
        slug: &str,
        tt: TeamType,
        players: Vec<crate::Player>,
    ) -> Team {
        Team::builder()
            .id(id)
            .league_id(Some(1))
            .club_id(100)
            .name(name.to_string())
            .slug(slug.to_string())
            .team_type(tt)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    #[test]
    fn main_to_second_demotion_opens_second_history_spell() {
        // Repro for the user report: a player demoted Main → Second via
        // the shared `execute_moves` helper (administrative demotion, AI
        // squad composition, …) must have the move reflected in career
        // history. Before the fix the move bypassed `on_intra_club_move`,
        // leaving the active history entry on Main, so the Second-team
        // appearances rendered under Main and no Second row ever appeared.
        let mut player = make_player(1);
        let main_info = crate::TeamInfo {
            name: "Spartak Moscow".to_string(),
            slug: "spartak-moscow".to_string(),
            reputation: 200,
            league_name: String::new(),
            league_slug: String::new(),
        };
        player
            .statistics_history
            .seed_initial_team(&main_info, d(2025, 8, 1), false);

        let mut teams = vec![
            make_team(
                10,
                "Spartak Moscow",
                "spartak-moscow",
                TeamType::Main,
                vec![player],
            ),
            make_team(
                11,
                "Spartak Moscow 2",
                "spartak-moscow-2",
                TeamType::Second,
                vec![],
            ),
        ];

        execute_moves(&mut teams, 0, 1, &[1], d(2025, 9, 15));

        // Player physically moved to the Second roster.
        assert!(teams[0].players.players.is_empty());
        assert_eq!(teams[1].players.players.len(), 1);

        let moved = &teams[1].players.players[0];
        let current = &moved.statistics_history.current;

        // Active spell is now the Second team. The Main spell had 0 games,
        // so it's a pass-through stop and is dropped entirely rather than
        // left as a phantom 0-game row.
        let active: Vec<&_> = current
            .iter()
            .filter(|e| e.departed_date.is_none())
            .collect();
        assert_eq!(active.len(), 1, "exactly one active spell expected");
        assert_eq!(
            active[0].team_slug, "spartak-moscow-2",
            "active history spell must follow the player to the Second team"
        );
        assert!(
            !current.iter().any(|e| e.team_slug == "spartak-moscow"),
            "the 0-game Main pass-through spell must be removed, got: {:?}",
            current.iter().map(|e| &e.team_slug).collect::<Vec<_>>()
        );

        // The rendered history must surface the Second team even before the
        // player logs a single Second-team game — the active spell is always
        // shown, 0-game skip only applies to *departed* rows.
        let view = moved
            .statistics_history
            .view_items(Some(&moved.statistics), d(2025, 10, 1));
        assert!(
            view.iter().any(|i| i.team_slug == "spartak-moscow-2"),
            "Second team must appear in the history view with 0 games for Main, got: {:?}",
            view.iter().map(|i| &i.team_slug).collect::<Vec<_>>()
        );
    }

    #[test]
    fn second_team_row_survives_zero_game_main_spell_with_prior_history() {
        // Realistic case: an established player (prior frozen seasons) who
        // hasn't featured for Main this season is demoted to the Second
        // team and then plays there. The empty Main spell is correctly
        // hidden (departed, 0 games, not the career-first record), but the
        // Second row must show the games the player actually logs there.
        use crate::PlayerStatistics;
        use crate::club::player::statistics::PlayerStatisticsHistoryItem;
        use crate::league::Season;

        let mut player = make_player(1);
        // Pre-load a prior season so this is NOT the player's first record.
        let mut prior = PlayerStatistics::default();
        prior.played = 30;
        player.statistics_history =
            crate::PlayerStatisticsHistory::from_items(vec![PlayerStatisticsHistoryItem {
                season: Season::new(2024),
                team_name: "Spartak Moscow".to_string(),
                team_slug: "spartak-moscow".to_string(),
                team_reputation: 200,
                league_name: String::new(),
                league_slug: String::new(),
                is_loan: false,
                transfer_fee: None,
                statistics: prior,
                seq_id: 0,
            }]);
        let main_info = crate::TeamInfo {
            name: "Spartak Moscow".to_string(),
            slug: "spartak-moscow".to_string(),
            reputation: 200,
            league_name: String::new(),
            league_slug: String::new(),
        };
        player
            .statistics_history
            .seed_initial_team(&main_info, d(2025, 8, 1), false);

        let mut teams = vec![
            make_team(
                10,
                "Spartak Moscow",
                "spartak-moscow",
                TeamType::Main,
                vec![player],
            ),
            make_team(
                11,
                "Spartak Moscow 2",
                "spartak-moscow-2",
                TeamType::Second,
                vec![],
            ),
        ];

        // 0 games for Main, then demoted to the Second team.
        execute_moves(&mut teams, 0, 1, &[1], d(2025, 9, 15));

        // Player logs 7 league games for the Second team.
        teams[1].players.players[0].statistics.played = 7;

        let moved = &teams[1].players.players[0];
        let view = moved
            .statistics_history
            .view_items(Some(&moved.statistics), d(2025, 10, 1));

        let second_row = view
            .iter()
            .find(|i| i.season.start_year == 2025 && i.team_slug == "spartak-moscow-2")
            .expect("Second team row must appear for the current season");
        assert_eq!(second_row.statistics.played, 7);

        // The empty Main spell this season is hidden (departed, 0 games,
        // not the career-first record).
        assert!(
            !view
                .iter()
                .any(|i| i.season.start_year == 2025 && i.team_slug == "spartak-moscow"),
            "the 0-game Main spell must not show a row this season"
        );
    }

    // ── Administrative-demotion decision + transfer-list migration ──

    use crate::shared::{Currency, CurrencyValue};
    use crate::{PlayerClubContract, PlayerSquadStatus, TransferItem};

    /// A contracted, single-position player with an explicit squad status —
    /// the inputs the demotion guard reads. Match-fit (high condition) and a
    /// mid current ability so a teammate built the same way counts as credible,
    /// usable cover for [`MainSquadMoveGuard`].
    fn make_contracted(
        id: u32,
        position: PlayerPositionType,
        status: PlayerSquadStatus,
    ) -> crate::Player {
        let mut p = make_player(id);
        p.positions = PlayerPositions {
            positions: vec![PlayerPosition {
                position,
                level: 16,
            }],
        };
        p.player_attributes.current_ability = 130;
        p.player_attributes.condition = 9000;
        p.player_attributes.fitness = 9000;
        let mut contract = PlayerClubContract::new(50_000, d(2030, 6, 30));
        contract.squad_status = status;
        p.contract = Some(contract);
        p
    }

    fn list_for_transfer(p: &mut crate::Player, date: NaiveDate) {
        p.statuses.add(date, PlayerStatusType::Lst);
        if let Some(c) = p.contract.as_mut() {
            c.is_transfer_listed = true;
        }
    }

    #[test]
    fn listed_first_team_regular_is_not_administratively_demoted() {
        // A transfer-listed first-team regular stays a Main-team asset —
        // listing is a market action, not a "stop playing him" instruction.
        let date = d(2026, 6, 15);
        let mut regular = make_contracted(
            1,
            PlayerPositionType::MidfielderCenter,
            PlayerSquadStatus::FirstTeamRegular,
        );
        list_for_transfer(&mut regular, date);
        let cover = make_contracted(
            2,
            PlayerPositionType::MidfielderCenter,
            PlayerSquadStatus::FirstTeamRegular,
        );
        let team = make_team(10, "Main", "main", TeamType::Main, vec![regular, cover]);

        let demotions = SquadManager::identify_administrative_demotions(&team, date);
        assert!(
            !demotions.contains(&1),
            "a listed FirstTeamRegular must not be administratively demoted"
        );
    }

    #[test]
    fn listed_key_player_is_not_administratively_demoted() {
        let date = d(2026, 6, 15);
        let mut key = make_contracted(1, PlayerPositionType::Striker, PlayerSquadStatus::KeyPlayer);
        list_for_transfer(&mut key, date);
        let cover = make_contracted(
            2,
            PlayerPositionType::Striker,
            PlayerSquadStatus::FirstTeamRegular,
        );
        let team = make_team(10, "Main", "main", TeamType::Main, vec![key, cover]);

        let demotions = SquadManager::identify_administrative_demotions(&team, date);
        assert!(
            !demotions.contains(&1),
            "a listed KeyPlayer must stay in the Main squad"
        );
    }

    #[test]
    fn listed_surplus_player_is_demoted_when_cover_exists() {
        // A listed NotNeeded player can still drop to the reserves when there
        // is realistic cover in his position.
        let date = d(2026, 6, 15);
        let mut surplus = make_contracted(
            1,
            PlayerPositionType::MidfielderCenter,
            PlayerSquadStatus::NotNeeded,
        );
        list_for_transfer(&mut surplus, date);
        let cover = make_contracted(
            2,
            PlayerPositionType::MidfielderCenter,
            PlayerSquadStatus::FirstTeamRegular,
        );
        let team = make_team(10, "Main", "main", TeamType::Main, vec![surplus, cover]);

        let demotions = SquadManager::identify_administrative_demotions(&team, date);
        assert!(
            demotions.contains(&1),
            "a listed, surplus, well-covered player should be demoted"
        );
    }

    #[test]
    fn listed_surplus_player_kept_when_last_in_his_unit() {
        // No replacement in the position group → keep him even if surplus, so a
        // listing never strips the senior squad of its last specialist.
        let date = d(2026, 6, 15);
        let mut surplus = make_contracted(
            1,
            PlayerPositionType::Goalkeeper,
            PlayerSquadStatus::NotNeeded,
        );
        list_for_transfer(&mut surplus, date);
        let mid = make_contracted(
            2,
            PlayerPositionType::MidfielderCenter,
            PlayerSquadStatus::FirstTeamRegular,
        );
        let team = make_team(10, "Main", "main", TeamType::Main, vec![surplus, mid]);

        let demotions = SquadManager::identify_administrative_demotions(&team, date);
        assert!(
            !demotions.contains(&1),
            "the only keeper must not be demoted just for being listed"
        );
    }

    #[test]
    fn agreed_transfer_player_is_not_auto_demoted() {
        // An agreed move (Trn) no longer triggers an automatic administrative
        // demotion: the player stays in the Main squad and is handled by the
        // selection layer's importance-aware injury-protection (rested in
        // routine games, selectable when the match matters).
        let date = d(2026, 6, 15);
        let mut leaving = make_contracted(
            1,
            PlayerPositionType::MidfielderCenter,
            PlayerSquadStatus::FirstTeamRegular,
        );
        leaving.statuses.add(date, PlayerStatusType::Trn);
        let cover = make_contracted(
            2,
            PlayerPositionType::MidfielderCenter,
            PlayerSquadStatus::FirstTeamRegular,
        );
        let team = make_team(10, "Main", "main", TeamType::Main, vec![leaving, cover]);

        let demotions = SquadManager::identify_administrative_demotions(&team, date);
        assert!(
            !demotions.contains(&1),
            "an agreed-transfer player must not be auto-demoted by the daily sweep"
        );
    }

    #[test]
    fn guard_allows_surplus_demote_only_with_usable_cover() {
        // Cover must be a real, usable replacement. An injured-only "cover"
        // teammate does not count, so a surplus player with no usable peer stays.
        let date = d(2026, 6, 15);
        let surplus = make_contracted(
            1,
            PlayerPositionType::MidfielderCenter,
            PlayerSquadStatus::NotNeeded,
        );
        let mut injured_cover = make_contracted(
            2,
            PlayerPositionType::MidfielderCenter,
            PlayerSquadStatus::FirstTeamRegular,
        );
        injured_cover.player_attributes.is_injured = true;
        let team = make_team(
            10,
            "Main",
            "main",
            TeamType::Main,
            vec![surplus, injured_cover],
        );

        let guard = MainSquadMoveGuard::new(&team, date);
        assert!(
            !guard.allow_demote_from_main(&team.players.players[0], "surplus"),
            "an injured teammate is not usable cover — surplus player should stay"
        );
    }

    #[test]
    fn manage_critical_moves_keeps_listed_regular_demotes_surplus() {
        // End-to-end through the daily entry point with a squad above the
        // minimum so demotions are allowed.
        let date = d(2026, 6, 15);
        let mut players = Vec::new();
        let mut regular = make_contracted(
            1,
            PlayerPositionType::MidfielderCenter,
            PlayerSquadStatus::FirstTeamRegular,
        );
        list_for_transfer(&mut regular, date);
        players.push(regular);
        let mut surplus = make_contracted(
            2,
            PlayerPositionType::MidfielderCenter,
            PlayerSquadStatus::NotNeeded,
        );
        list_for_transfer(&mut surplus, date);
        players.push(surplus);
        // Fillers (same group → surplus has cover) to clear MIN_FIRST_TEAM_SQUAD.
        for id in 3..=27u32 {
            players.push(make_contracted(
                id,
                PlayerPositionType::MidfielderCenter,
                PlayerSquadStatus::FirstTeamSquadRotation,
            ));
        }
        let mut teams = vec![
            make_team(10, "Main", "main", TeamType::Main, players),
            make_team(11, "Reserves", "reserves", TeamType::Second, vec![]),
        ];

        let mut coach_state = None;
        SquadManager::manage_critical_moves(&mut teams, &mut coach_state, 0, 1, date);

        assert!(
            teams[0].players.contains(1),
            "listed regular must remain on the Main roster"
        );
        assert!(
            !teams[0].players.contains(2),
            "listed surplus player must be demoted off Main"
        );
        assert!(
            teams[1].players.contains(2),
            "listed surplus player must land in the reserves"
        );
    }

    #[test]
    fn internal_move_migrates_transfer_list_entry() {
        // Internal squad moves must not destroy market visibility. The player's
        // Lst status travels with him; the asking-price entry is migrated to the
        // destination team rather than dropped.
        let date = d(2026, 6, 15);
        let mut listed = make_contracted(
            1,
            PlayerPositionType::MidfielderCenter,
            PlayerSquadStatus::NotNeeded,
        );
        list_for_transfer(&mut listed, date);
        let mut teams = vec![
            make_team(10, "Main", "main", TeamType::Main, vec![listed]),
            make_team(11, "Reserves", "reserves", TeamType::Second, vec![]),
        ];
        teams[0].transfer_list.add(TransferItem::new(
            1,
            CurrencyValue::new(1_000_000.0, Currency::Usd),
        ));

        execute_moves(&mut teams, 0, 1, &[1], date);

        let moved = &teams[1].players.players[0];
        assert!(
            moved.statuses.has(PlayerStatusType::Lst),
            "Lst status must travel with the player"
        );
        assert!(
            !teams[0].transfer_list.contains(1),
            "stale asking-price entry must not remain on the source team"
        );
        assert!(
            teams[1].transfer_list.contains(1),
            "asking-price entry must follow the player to the destination team"
        );
    }

    #[test]
    fn cover_ignores_injured_banned_intl_and_far_weaker_players() {
        // A listed first-team regular whose only same-position peers are
        // injured / banned / on international duty / far weaker has no usable,
        // credible cover, so the guard keeps him on Main.
        let date = d(2026, 6, 15);
        let mut star = make_contracted(
            1,
            PlayerPositionType::Striker,
            PlayerSquadStatus::FirstTeamRegular,
        );
        star.player_attributes.current_ability = 150;
        list_for_transfer(&mut star, date);

        let mut injured = make_contracted(
            2,
            PlayerPositionType::Striker,
            PlayerSquadStatus::FirstTeamRegular,
        );
        injured.player_attributes.is_injured = true;
        let mut banned = make_contracted(
            3,
            PlayerPositionType::Striker,
            PlayerSquadStatus::FirstTeamRegular,
        );
        banned.player_attributes.is_banned = true;
        let mut on_duty = make_contracted(
            4,
            PlayerPositionType::Striker,
            PlayerSquadStatus::FirstTeamRegular,
        );
        on_duty.statuses.add(date, PlayerStatusType::Int);
        let mut weak = make_contracted(
            5,
            PlayerPositionType::Striker,
            PlayerSquadStatus::MainBackupPlayer,
        );
        weak.player_attributes.current_ability = 100; // > 25 CA below the star

        let team = make_team(
            10,
            "Main",
            "main",
            TeamType::Main,
            vec![star, injured, banned, on_duty, weak],
        );
        let guard = MainSquadMoveGuard::new(&team, date);
        assert!(
            !guard.allow_demote_from_main(&team.players.players[0], "llm wants him gone"),
            "no usable, credible same-position cover → listed regular must stay"
        );
    }

    #[test]
    fn listed_keeper_protected_unless_two_backups_remain() {
        let date = d(2026, 6, 15);

        // One backup → demoting the listed keeper leaves a single keeper: blocked.
        let mut keeper = make_contracted(
            1,
            PlayerPositionType::Goalkeeper,
            PlayerSquadStatus::FirstTeamRegular,
        );
        list_for_transfer(&mut keeper, date);
        let backup = make_contracted(
            2,
            PlayerPositionType::Goalkeeper,
            PlayerSquadStatus::MainBackupPlayer,
        );
        let team_one = make_team(10, "Main", "main", TeamType::Main, vec![keeper, backup]);
        let guard_one = MainSquadMoveGuard::new(&team_one, date);
        assert!(
            !guard_one.allow_demote_from_main(&team_one.players.players[0], "sell keeper"),
            "a listed keeper must not be demoted when it leaves only one usable keeper"
        );

        // Two backups → enough cover, demotion permitted.
        let mut keeper2 = make_contracted(
            1,
            PlayerPositionType::Goalkeeper,
            PlayerSquadStatus::FirstTeamRegular,
        );
        list_for_transfer(&mut keeper2, date);
        let b1 = make_contracted(
            2,
            PlayerPositionType::Goalkeeper,
            PlayerSquadStatus::MainBackupPlayer,
        );
        let b2 = make_contracted(
            3,
            PlayerPositionType::Goalkeeper,
            PlayerSquadStatus::MainBackupPlayer,
        );
        let team_two = make_team(11, "Main", "main", TeamType::Main, vec![keeper2, b1, b2]);
        let guard_two = MainSquadMoveGuard::new(&team_two, date);
        assert!(
            guard_two.allow_demote_from_main(&team_two.players.players[0], "sell keeper"),
            "a listed keeper with two usable backups can be demoted"
        );
    }
}
