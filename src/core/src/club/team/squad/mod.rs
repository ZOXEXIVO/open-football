mod composition;
mod contract_renewal;
mod match_squad;
mod satisfaction;
mod transfer_listing;

pub use composition::SquadComposition;
pub use contract_renewal::{ContractRenewalManager, WageStructureSnapshot};
pub use satisfaction::compute_squad_satisfaction;
pub use transfer_listing::TransferListManager;

use crate::club::staff::perception::{CoachDecisionState, RecentMoveType};
use crate::utils::DateUtils;
use crate::{PlayerStatusType, Team};
use chrono::NaiveDate;

pub struct SquadManager;

pub const MIN_FIRST_TEAM_SQUAD: usize = 25;

impl SquadManager {
    /// Daily: only mandatory administrative demotions (Lst, Loa).
    /// All other squad decisions (recalls, swaps, performance demotions)
    /// go through the monthly AI-driven squad composition.
    pub fn manage_critical_moves(
        teams: &mut [Team],
        coach_state: &mut Option<CoachDecisionState>,
        main_idx: usize,
        reserve_idx: usize,
        date: NaiveDate,
    ) {
        let coach_name = teams[main_idx].staffs.head_coach_name();
        let demotions = Self::identify_administrative_demotions(&teams[main_idx]);
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

    /// Administrative demotions: transfer-listed (Lst) players move to reserves.
    /// Loa (Leave of Absence) players stay where they are — the coach decides.
    fn identify_administrative_demotions(main_team: &Team) -> Vec<u32> {
        main_team
            .players
            .players
            .iter()
            .filter_map(|player| {
                if player.statuses.get().contains(&PlayerStatusType::Lst) {
                    Some(player.id)
                } else {
                    None
                }
            })
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
            teams[from_idx].transfer_list.remove(player_id);
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

    fn make_team(id: u32, name: &str, slug: &str, tt: TeamType, players: Vec<crate::Player>) -> Team {
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
            make_team(10, "Spartak Moscow", "spartak-moscow", TeamType::Main, vec![player]),
            make_team(11, "Spartak Moscow 2", "spartak-moscow-2", TeamType::Second, vec![]),
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
        let active: Vec<&_> = current.iter().filter(|e| e.departed_date.is_none()).collect();
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
        player.statistics_history = crate::PlayerStatisticsHistory::from_items(vec![
            PlayerStatisticsHistoryItem {
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
            },
        ]);
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
            make_team(10, "Spartak Moscow", "spartak-moscow", TeamType::Main, vec![player]),
            make_team(11, "Spartak Moscow 2", "spartak-moscow-2", TeamType::Second, vec![]),
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
}
