//! The `match-stub` result: a scoreline drawn from the two elevens'
//! ability and the home edge, with scorers, substitutions, minutes and
//! ratings that agree with it.

use super::goal_celebration_tests::squad;
use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{MatchResultRaw, MatchSquad};

type Engine = FootballEngine<840, 545>;

fn rated(team_id: u32, base_id: u32, ability: u8) -> MatchSquad {
    let mut side = squad(team_id, base_id);
    for player in &mut side.main_squad {
        player.player_attributes.current_ability = ability;
    }
    side
}

/// A bench of six outfield players and a keeper, each a copy of a starter.
fn with_bench(mut side: MatchSquad, base_id: u32) -> MatchSquad {
    for (offset, index) in (0..=6).enumerate() {
        let mut substitute = side.main_squad[index].clone();
        substitute.id = base_id + offset as u32;
        side.substitutes.push(substitute);
    }
    side
}

fn play(home: &MatchSquad, away: &MatchSquad, seed: u64, knockout: bool) -> MatchResultRaw {
    let config = MatchEngineConfig {
        is_knockout: knockout,
        ..MatchEngineConfig::seeded(seed)
    };
    Engine::play_stub(home.clone(), away.clone(), &config)
}

fn goals(result: &MatchResultRaw) -> (u8, u8) {
    let score = result.score.as_ref().unwrap();
    (score.home_team.get(), score.away_team.get())
}

#[test]
fn every_player_who_takes_the_pitch_is_credited_and_minutes_add_up() {
    let home = with_bench(squad(1, 100), 150);
    let away = with_bench(squad(2, 200), 250);
    for seed in 0..50 {
        let result = play(&home, &away, seed, false);
        for (side, field) in [
            (&home, &result.left_team_players),
            (&away, &result.right_team_players),
        ] {
            let used = field.substitutes_used.len();
            assert!((3..=5).contains(&used), "{used} substitutes used");
            let keeper_sub = side.substitutes[0].id;
            assert!(!field.substitutes_used.contains(&keeper_sub));

            let minutes: u32 = side
                .main_squad
                .iter()
                .chain(&side.substitutes)
                .filter_map(|p| result.player_stats.get(&p.id))
                .map(|s| s.minutes_played as u32)
                .sum();
            assert_eq!(minutes, 11 * 90, "eleven slots of 90 minutes");
            assert_eq!(
                result.player_stats.len() >= 11 + used,
                true,
                "every starter and every used substitute has a stat line"
            );
        }
        assert_eq!(
            result.substitutions.len(),
            result.left_team_players.substitutes_used.len()
                + result.right_team_players.substitutes_used.len()
        );
    }
}

#[test]
fn goal_details_agree_with_the_score_and_the_stat_lines() {
    let home = with_bench(squad(1, 100), 150);
    let away = with_bench(squad(2, 200), 250);
    for seed in 0..200 {
        let result = play(&home, &away, seed, false);
        let (home_goals, away_goals) = goals(&result);
        let score = result.score.as_ref().unwrap();
        let scored_by = |side: &MatchSquad| {
            let ids: Vec<u32> = side
                .main_squad
                .iter()
                .chain(&side.substitutes)
                .map(|p| p.id)
                .collect();
            let details = score
                .detail()
                .iter()
                .filter(|d| d.stat_type == MatchStatisticType::Goal && ids.contains(&d.player_id))
                .count();
            let lines: u16 = ids
                .iter()
                .filter_map(|id| result.player_stats.get(id))
                .map(|s| s.goals)
                .sum();
            let assists: u16 = ids
                .iter()
                .filter_map(|id| result.player_stats.get(id))
                .map(|s| s.assists)
                .sum();
            (details, lines, assists)
        };
        let (home_details, home_lines, home_assists) = scored_by(&home);
        let (away_details, away_lines, away_assists) = scored_by(&away);
        assert_eq!(home_details, home_goals as usize);
        assert_eq!(away_details, away_goals as usize);
        assert_eq!(home_lines, home_goals as u16);
        assert_eq!(away_lines, away_goals as u16);
        assert!(home_assists <= home_lines && away_assists <= away_lines);
    }
}

#[test]
fn the_stronger_eleven_wins_and_outscores_the_weaker_wherever_it_plays() {
    let strong = rated(1, 100, 170);
    let weak = rated(2, 200, 120);
    let (mut strong_wins, mut weak_wins) = (0, 0);
    let (mut strong_goals, mut weak_goals) = (0u32, 0u32);
    for seed in 0..400 {
        let (home, away) = if seed % 2 == 0 {
            (&strong, &weak)
        } else {
            (&weak, &strong)
        };
        let (h, a) = goals(&play(home, away, seed, false));
        let (s, w) = if seed % 2 == 0 { (h, a) } else { (a, h) };
        strong_goals += s as u32;
        weak_goals += w as u32;
        if s > w {
            strong_wins += 1;
        } else if w > s {
            weak_wins += 1;
        }
    }
    assert!(strong_wins > 260, "strong side won {strong_wins} of 400");
    assert!(weak_wins < 50, "weak side won {weak_wins} of 400");
    assert!(
        strong_goals > 3 * weak_goals,
        "{strong_goals} against {weak_goals}"
    );
}

#[test]
fn equal_elevens_share_a_realistic_goal_count_and_the_home_side_wins_more() {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let (mut total, mut home_wins, mut away_wins, mut draws) = (0u32, 0, 0, 0);
    let matches = 2000;
    for seed in 0..matches {
        let (h, a) = goals(&play(&home, &away, seed, false));
        total += (h + a) as u32;
        match h.cmp(&a) {
            std::cmp::Ordering::Greater => home_wins += 1,
            std::cmp::Ordering::Less => away_wins += 1,
            std::cmp::Ordering::Equal => draws += 1,
        }
    }
    let per_match = total as f32 / matches as f32;
    assert!(
        (2.4..=3.0).contains(&per_match),
        "{per_match} goals a match"
    );
    assert!(home_wins > away_wins, "home {home_wins}, away {away_wins}");
    assert!((400..=650).contains(&draws), "{draws} draws in {matches}");
}

#[test]
fn a_knockout_tie_is_always_settled() {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let mut shootouts = 0;
    for seed in 0..300 {
        let result = play(&home, &away, seed, true);
        let score = result.score.as_ref().unwrap();
        if score.is_tied() {
            shootouts += 1;
            assert_ne!(score.home_shootout, score.away_shootout);
            assert_eq!(result.match_time_ms, 120 * 60 * 1000);
        }
    }
    assert!(shootouts > 0, "300 even ties went without a shootout");
}

#[test]
fn ratings_reward_goals_and_stay_on_the_scale() {
    let home = with_bench(squad(1, 100), 150);
    let away = with_bench(squad(2, 200), 250);
    let (mut scorer_sum, mut scorer_n, mut other_sum, mut other_n) = (0.0f32, 0, 0.0f32, 0);
    for seed in 0..200 {
        for stats in play(&home, &away, seed, false).player_stats.values() {
            assert!((3.0..=10.0).contains(&stats.match_rating));
            if stats.position_group == PlayerFieldPositionGroup::Goalkeeper {
                assert!(stats.shots_faced >= stats.saves);
            }
            if stats.goals > 0 {
                scorer_sum += stats.match_rating;
                scorer_n += 1;
            } else if stats.minutes_played >= 60 {
                other_sum += stats.match_rating;
                other_n += 1;
            }
        }
    }
    let other = other_sum / other_n as f32;
    assert!((6.3..=6.9).contains(&other), "non-scorers average {other}");
    assert!(scorer_sum / scorer_n as f32 > other + 0.6);
}

#[test]
fn the_same_seed_draws_the_same_match() {
    let home = with_bench(squad(1, 100), 150);
    let away = with_bench(squad(2, 200), 250);
    let first = play(&home, &away, 7, false);
    let second = play(&home, &away, 7, false);
    assert_eq!(goals(&first), goals(&second));
    let scorers = |r: &MatchResultRaw| {
        r.score
            .as_ref()
            .unwrap()
            .detail()
            .iter()
            .map(|d| (d.player_id, d.time))
            .collect::<Vec<_>>()
    };
    assert_eq!(scorers(&first), scorers(&second));
}
