//! The `match-stub` result: a scoreline drawn from the two elevens and the
//! home edge, and a full stat line per man that agrees with it and that the
//! engine's own rating model reads.

use super::goal_celebration_tests::squad;
use crate::club::player::builder::PlayerBuilder;
use crate::club::player::events::MatchExertionInputs;
use crate::club::player::player::Player;
use crate::club::player::{PlayerPosition, PlayerPositions};
use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::player::injury::InjuryGrade;
use crate::r#match::engine::rating::{MatchRatingPass, RatingContext, RatingVolumes};
use crate::r#match::engine::result::{PlayerMatchEndStats, SubstitutionReason};
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{MatchPlayer, MatchResultRaw, MatchSquad};
use crate::shared::fullname::FullName;
use crate::{
    PersonAttributes, PlayerAttributes, PlayerFieldPositionGroup, PlayerPositionType, PlayerSkills,
};
use chrono::NaiveDate;
use rustc_hash::FxHashMap;

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

/// On a fixed day: a man's season reads the calendar, and a test must not.
fn play(home: &MatchSquad, away: &MatchSquad, seed: u64, knockout: bool) -> MatchResultRaw {
    let config = MatchEngineConfig {
        is_knockout: knockout,
        today: NaiveDate::from_ymd_opt(2030, 10, 1).unwrap(),
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
            let keeper = side.main_squad[0].id;
            let keeper_sub = side.substitutes[0].id;
            assert_eq!(
                field.substitutes_used.contains(&keeper_sub),
                result
                    .substitutions
                    .iter()
                    .any(|s| s.player_out_id == keeper),
                "the bench keeper comes on only for the keeper"
            );

            let lines: Vec<&PlayerMatchEndStats> = side
                .main_squad
                .iter()
                .chain(&side.substitutes)
                .filter_map(|p| result.player_stats.get(&p.id))
                .collect();
            let minutes: u32 = lines.iter().map(|s| s.minutes_played as u32).sum();
            let whole_side_stayed_on = lines.iter().all(|s| s.red_cards == 0) && used < 5;
            if whole_side_stayed_on {
                assert_eq!(minutes, 11 * 90, "eleven slots of 90 minutes");
            } else {
                assert!(minutes <= 11 * 90, "{minutes} minutes in eleven slots");
            }
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
    assert_eq!(first.player_stats.len(), second.player_stats.len());
    for (id, line) in &first.player_stats {
        assert_eq!(
            format!("{line:?}"),
            format!("{:?}", second.player_stats[id])
        );
    }
}

/// A man whose skills sit at the level his ability implies, of ordinary
/// temperament: every volume on his line draws at its table value.
fn make_ordinary(player: &mut MatchPlayer, ability: u8) {
    player.player_attributes.current_ability = ability;
    let level = PlayerSkills::ability_skill_level(ability);
    let t = &mut player.skills.technical;
    t.finishing = level;
    t.passing = level;
    t.crossing = level;
    t.dribbling = level;
    t.tackling = level;
    t.heading = level;
    t.first_touch = level;
    let m = &mut player.skills.mental;
    m.vision = level;
    m.positioning = level;
    m.work_rate = level;
    m.off_the_ball = level;
    m.concentration = 12.0;
    m.composure = 12.0;
    m.aggression = 10.0;
    let g = &mut player.skills.goalkeeping;
    g.reflexes = level;
    g.handling = level;
    g.one_on_ones = level;
    g.passing = level;
    g.command_of_area = level;
    player.attributes.consistency = 12.0;
    player.attributes.professionalism = 12.0;
    player.attributes.dirtiness = 10.0;
    player.attributes.pressure = 12.0;
    player.attributes.important_matches = 12.0;
    player.player_attributes.injury_proneness = 10;
}

fn ordinary(team_id: u32, base_id: u32, ability: u8) -> MatchSquad {
    let mut side = with_bench(squad(team_id, base_id), base_id + 50);
    for player in side.main_squad.iter_mut().chain(&mut side.substitutes) {
        make_ordinary(player, ability);
    }
    side
}

fn play_on(home: &MatchSquad, away: &MatchSquad, seed: u64, today: NaiveDate) -> MatchResultRaw {
    let config = MatchEngineConfig {
        today,
        ..MatchEngineConfig::seeded(seed)
    };
    Engine::play_stub(home.clone(), away.clone(), &config)
}

const ORDINARY_ABILITY: u8 = 120;

/// Mean per-90 of `read` over one position group's lines, across a run.
fn per_90(
    results: &[MatchResultRaw],
    group: PlayerFieldPositionGroup,
    read: impl Fn(&PlayerMatchEndStats) -> f32,
) -> f32 {
    let (mut total, mut minutes) = (0.0, 0.0);
    for line in results
        .iter()
        .flat_map(|r| r.player_stats.values())
        .filter(|s| s.position_group == group)
    {
        total += read(line);
        minutes += line.minutes_played as f32;
    }
    total * 90.0 / minutes
}

/// Over many seasons, so what reads out is the position and not how these
/// twenty-odd men happen to be having this particular year.
fn even_ordinary_matches(matches: u64) -> Vec<MatchResultRaw> {
    let mut home = ordinary(1, 100, ORDINARY_ABILITY);
    let mut away = ordinary(2, 200, ORDINARY_ABILITY);
    (0..matches)
        .map(|seed| {
            let today = season_day(2030 + (seed % 200) as i32, 60);
            for player in home
                .main_squad
                .iter_mut()
                .chain(&mut home.substitutes)
                .chain(&mut away.main_squad)
                .chain(&mut away.substitutes)
            {
                player.birth_date = today - chrono::Duration::days(26 * 365);
            }
            play_on(&home, &away, seed, today)
        })
        .collect()
}

/// When he came on and went off, from what the result records of him.
fn time_on_pitch(result: &MatchResultRaw, player_id: u32) -> (u64, u64) {
    let on = result
        .substitutions
        .iter()
        .find(|s| s.player_in_id == player_id)
        .map_or(0, |s| s.match_time_ms);
    let minutes = result.player_stats[&player_id].minutes_played as u64;
    (on, on + minutes * 60_000)
}

#[test]
fn a_position_groups_line_looks_like_that_position() {
    let results = even_ordinary_matches(1500);
    let tackles = |s: &PlayerMatchEndStats| s.tackles as f32;
    let interceptions = |s: &PlayerMatchEndStats| s.interceptions as f32;
    let shots = |s: &PlayerMatchEndStats| s.shots_total as f32;
    let key_passes = |s: &PlayerMatchEndStats| s.key_passes as f32;
    let progressive_passes = |s: &PlayerMatchEndStats| s.progressive_passes as f32;
    let progressive_carries = |s: &PlayerMatchEndStats| s.progressive_carries as f32;
    let dribbles = |s: &PlayerMatchEndStats| s.attempted_dribbles as f32;
    let measured =
        |group, read: &dyn Fn(&PlayerMatchEndStats) -> f32| per_90(&results, group, read);

    // From `rating/volume.rs` where it states one. Back-line progression,
    // dribbles and shots follow the season fixtures in `season_tests.rs`.
    let defender = PlayerFieldPositionGroup::Defender;
    let midfielder = PlayerFieldPositionGroup::Midfielder;
    let forward = PlayerFieldPositionGroup::Forward;
    let references: [(&str, f32, f32); 21] = [
        ("DEF tackles", measured(defender, &tackles), 1.6),
        ("DEF interceptions", measured(defender, &interceptions), 1.3),
        ("DEF shots", measured(defender, &shots), 0.45),
        ("DEF key passes", measured(defender, &key_passes), 0.45),
        (
            "DEF prog passes",
            measured(defender, &progressive_passes),
            3.0,
        ),
        (
            "DEF prog carries",
            measured(defender, &progressive_carries),
            1.2,
        ),
        ("DEF dribbles", measured(defender, &dribbles), 0.4),
        ("MID tackles", measured(midfielder, &tackles), 1.8),
        (
            "MID interceptions",
            measured(midfielder, &interceptions),
            1.0,
        ),
        ("MID shots", measured(midfielder, &shots), 1.0),
        ("MID key passes", measured(midfielder, &key_passes), 1.0),
        (
            "MID prog passes",
            measured(midfielder, &progressive_passes),
            5.0,
        ),
        (
            "MID prog carries",
            measured(midfielder, &progressive_carries),
            2.0,
        ),
        ("MID dribbles", measured(midfielder, &dribbles), 1.0),
        ("FWD tackles", measured(forward, &tackles), 0.8),
        ("FWD interceptions", measured(forward, &interceptions), 0.6),
        ("FWD shots", measured(forward, &shots), 3.4),
        ("FWD key passes", measured(forward, &key_passes), 1.0),
        (
            "FWD prog passes",
            measured(forward, &progressive_passes),
            2.5,
        ),
        (
            "FWD prog carries",
            measured(forward, &progressive_carries),
            2.5,
        ),
        ("FWD dribbles", measured(forward, &dribbles), 1.2),
    ];
    for (name, mean, reference) in references {
        assert!(
            (mean - reference).abs() <= reference * 0.15,
            "{name}: {mean:.2} per 90 against a reference of {reference}"
        );
    }
    let faced = per_90(&results, PlayerFieldPositionGroup::Goalkeeper, |s| {
        s.shots_faced as f32
    });
    assert!(
        (faced - 4.3).abs() <= 4.3 * 0.15,
        "keepers face {faced:.2} shots on target against a real ~4.3"
    );
}

#[test]
fn a_line_agrees_with_the_score() {
    let home = ordinary(1, 100, ORDINARY_ABILITY);
    let away = ordinary(2, 200, ORDINARY_ABILITY);
    for seed in 0..400 {
        let result = play(&home, &away, seed, false);
        let (home_goals, away_goals) = goals(&result);
        let score = result.score.as_ref().unwrap();

        for detail in score.detail() {
            let (on, off) = time_on_pitch(&result, detail.player_id);
            assert!(
                on <= detail.time && detail.time < off,
                "credited at {} while on the pitch {on}..{off}",
                detail.time
            );
        }

        for line in result.player_stats.values() {
            assert!(line.shots_on_target >= line.goals);
            assert!(line.shots_total >= line.shots_on_target);
            assert!(line.passes_completed <= line.passes_attempted);
            assert!(line.successful_dribbles <= line.attempted_dribbles);
            assert!(line.crosses_completed <= line.crosses_attempted);
            assert!(line.successful_pressures <= line.pressures);
        }

        for (side, conceded) in [(&home, away_goals), (&away, home_goals)] {
            let keepers: Vec<&PlayerMatchEndStats> = side
                .main_squad
                .iter()
                .chain(&side.substitutes)
                .filter_map(|p| result.player_stats.get(&p.id))
                .filter(|s| s.position_group == PlayerFieldPositionGroup::Goalkeeper)
                .collect();
            let beaten: u16 = keepers.iter().map(|k| k.shots_faced - k.saves).sum();
            let goal_was_covered = keepers.iter().map(|k| k.minutes_played).sum::<u16>() >= 90;
            if !goal_was_covered {
                assert!(beaten <= conceded as u16);
            } else {
                assert_eq!(beaten, conceded as u16, "seed {seed}");
            }
        }
    }
}

#[test]
fn a_cameo_has_a_cameos_line() {
    let results = even_ordinary_matches(1500);
    for group in [
        PlayerFieldPositionGroup::Defender,
        PlayerFieldPositionGroup::Midfielder,
    ] {
        let passes_per_minute = |short: bool| {
            let (mut passes, mut minutes) = (0.0, 0.0);
            for line in results
                .iter()
                .flat_map(|r| r.player_stats.values())
                .filter(|s| s.position_group == group)
                .filter(|s| {
                    if short {
                        s.minutes_played <= 20
                    } else {
                        s.minutes_played >= 85
                    }
                })
            {
                passes += line.passes_attempted as f32;
                minutes += line.minutes_played as f32;
            }
            passes / minutes
        };
        let (cameo, full) = (passes_per_minute(true), passes_per_minute(false));
        assert!(
            (cameo - full).abs() < full * 0.20,
            "a cameo passes {cameo:.3} a minute against a full match's {full:.3}"
        );
    }
}

#[test]
fn a_better_player_has_a_better_line() {
    let mut home = ordinary(1, 100, ORDINARY_ABILITY);
    let away = ordinary(2, 200, ORDINARY_ABILITY);
    // Two centre midfielders in the same fixture, forty points either side.
    let (better, worse) = (home.main_squad[6].id, home.main_squad[7].id);
    make_ordinary(&mut home.main_squad[6], ORDINARY_ABILITY + 40);
    make_ordinary(&mut home.main_squad[7], ORDINARY_ABILITY - 40);

    let (mut better_lines, mut worse_lines) = (Vec::new(), Vec::new());
    for seed in 0..1500 {
        let result = play(&home, &away, seed, false);
        better_lines.push(result.player_stats[&better].clone());
        worse_lines.push(result.player_stats[&worse].clone());
    }
    let mean = |lines: &[PlayerMatchEndStats], read: fn(&PlayerMatchEndStats) -> f32| {
        lines.iter().map(read).sum::<f32>() / lines.len() as f32
    };
    let reads: [(&str, fn(&PlayerMatchEndStats) -> f32); 5] = [
        ("key passes", |s| s.key_passes as f32),
        ("progressive passes", |s| s.progressive_passes as f32),
        ("tackles", |s| s.tackles as f32),
        ("completed passes", |s| s.passes_completed as f32),
        ("rating", |s| s.match_rating),
    ];
    for (name, read) in reads {
        let (b, w) = (mean(&better_lines, read), mean(&worse_lines, read));
        assert!(b > w, "{name}: the better man {b:.2}, the worse {w:.2}");
    }
    let rating_gap =
        mean(&better_lines, |s| s.match_rating) - mean(&worse_lines, |s| s.match_rating);
    assert!(
        rating_gap > 0.25,
        "eighty points of ability is worth {rating_gap:.2} of rating"
    );
}

#[test]
fn ratings_land_in_the_models_bands() {
    let results = even_ordinary_matches(1500);
    let bands = [
        (PlayerFieldPositionGroup::Goalkeeper, 6.60, 7.00),
        (PlayerFieldPositionGroup::Defender, 6.55, 6.95),
        (PlayerFieldPositionGroup::Midfielder, 6.60, 7.00),
        (PlayerFieldPositionGroup::Forward, 6.55, 7.15),
    ];
    for (group, low, high) in bands {
        let ratings: Vec<f32> = results
            .iter()
            .flat_map(|r| r.player_stats.values())
            .filter(|s| s.position_group == group && s.minutes_played >= 60)
            .map(|s| s.match_rating)
            .collect();
        let mean = ratings.iter().sum::<f32>() / ratings.len() as f32;
        assert!(
            (low..=high).contains(&mean),
            "{group:?} rate {mean:.3}, outside {low}..{high}"
        );
    }
}

#[test]
fn a_goal_is_worth_what_the_model_says() {
    let home = ordinary(1, 100, ORDINARY_ABILITY);
    let away = ordinary(2, 200, ORDINARY_ABILITY);
    let mut result = play(&home, &away, 11, false);
    let (home_goals, away_goals) = goals(&result);
    let forward = home.main_squad[9].id;

    let line = result.player_stats[&forward].clone();
    assert_eq!(
        line.raw_match_rating,
        RatingContext::new(&line, home_goals, away_goals).calculate(),
        "the stub has no rating of its own"
    );

    let mut with_a_goal = line.clone();
    with_a_goal.goals += 1;
    with_a_goal.shots_on_target += 1;
    with_a_goal.shots_total += 1;
    let model_says = RatingContext::new(&with_a_goal, home_goals, away_goals).calculate();

    result.player_stats.insert(forward, with_a_goal);
    MatchRatingPass::rate(&mut result, RatingVolumes::RealUnits);
    assert_eq!(result.player_stats[&forward].raw_match_rating, model_says);
    assert!(model_says > line.raw_match_rating);
}

#[test]
fn a_bad_day_shows_in_the_line() {
    let away = ordinary(2, 200, ORDINARY_ABILITY);
    let side_with_form = |form: f32| {
        let mut home = ordinary(1, 100, ORDINARY_ABILITY);
        home.main_squad[6].matchday_form = form;
        home
    };
    let subject = side_with_form(1.0).main_squad[6].id;
    let lines = |form: f32| -> Vec<PlayerMatchEndStats> {
        let home = side_with_form(form);
        (0..1500)
            .map(|seed| play(&home, &away, seed, false).player_stats[&subject].clone())
            .collect()
    };
    let mean = |lines: &[PlayerMatchEndStats], read: fn(&PlayerMatchEndStats) -> f32| {
        lines.iter().map(read).sum::<f32>() / lines.len() as f32
    };
    let (bad_day, good_day) = (lines(0.80), lines(1.20));
    let completed: fn(&PlayerMatchEndStats) -> f32 = |s| s.passes_completed as f32;
    let rating: fn(&PlayerMatchEndStats) -> f32 = |s| s.match_rating;
    assert!(mean(&bad_day, completed) < mean(&good_day, completed) * 0.85);
    assert!(mean(&bad_day, rating) < mean(&good_day, rating) - 0.1);
}

#[test]
fn a_dirty_player_collects_more_cards() {
    let mut home = ordinary(1, 100, ORDINARY_ABILITY);
    let away = ordinary(2, 200, ORDINARY_ABILITY);
    home.main_squad[6].skills.mental.aggression = 18.0;
    home.main_squad[6].attributes.dirtiness = 18.0;
    home.main_squad[7].skills.mental.aggression = 4.0;
    home.main_squad[7].attributes.dirtiness = 4.0;
    let (dirty, clean) = (home.main_squad[6].id, home.main_squad[7].id);

    let (mut dirty_fouls, mut clean_fouls, mut dirty_cards, mut clean_cards) = (0, 0, 0, 0);
    for seed in 0..1500 {
        let result = play(&home, &away, seed, false);
        let (d, c) = (&result.player_stats[&dirty], &result.player_stats[&clean]);
        dirty_fouls += d.fouls;
        clean_fouls += c.fouls;
        dirty_cards += d.yellow_cards + d.red_cards;
        clean_cards += c.yellow_cards + c.red_cards;
    }
    assert!(
        dirty_fouls > clean_fouls * 3 / 2,
        "{dirty_fouls} fouls against {clean_fouls}"
    );
    assert!(
        dirty_cards > clean_cards * 2,
        "{dirty_cards} cards against {clean_cards}"
    );
}

#[test]
fn a_red_card_ends_the_stint() {
    let mut home = ordinary(1, 100, ORDINARY_ABILITY);
    for player in &mut home.main_squad {
        player.skills.mental.aggression = 20.0;
        player.attributes.dirtiness = 20.0;
    }
    let away = ordinary(2, 200, ORDINARY_ABILITY);
    let mut sent_off = 0;
    for seed in 0..1000 {
        let result = play(&home, &away, seed, false);
        let score = result.score.as_ref().unwrap();
        for starter in &home.main_squad {
            let line = &result.player_stats[&starter.id];
            if line.red_cards == 0 {
                continue;
            }
            sent_off += 1;
            let dismissed_at = line.minutes_played as u64 * 60_000;
            assert!(
                score
                    .detail()
                    .iter()
                    .filter(|d| d.player_id == starter.id)
                    .all(|d| d.time < dismissed_at),
                "credited after he was sent off at {dismissed_at}"
            );
            assert!(
                !result
                    .substitutions
                    .iter()
                    .any(|s| s.player_out_id == starter.id),
                "nobody replaces a man sent off"
            );
        }
    }
    assert!(sent_off > 20, "only {sent_off} red cards to look at");
}

#[test]
fn an_injury_reaches_the_player() {
    let home = ordinary(1, 100, ORDINARY_ABILITY);
    let away = ordinary(2, 200, ORDINARY_ABILITY);
    let mut carried_off = 0;
    for seed in 0..3000 {
        let result = play(&home, &away, seed, false);
        for snapshot in result.physical_snapshots.values() {
            let id = snapshot.player_id;
            assert_eq!(
                snapshot.minutes_played as u16,
                result.player_stats[&id].minutes_played
            );
            if snapshot.injury != Some(InjuryGrade::Serious) {
                continue;
            }
            carried_off += 1;
            let team = if home
                .main_squad
                .iter()
                .chain(&home.substitutes)
                .any(|p| p.id == id)
            {
                home.team_id
            } else {
                away.team_id
            };
            let changes_made = result
                .substitutions
                .iter()
                .filter(|s| s.team_id == team)
                .count();
            assert!(
                changes_made >= 5
                    || result
                        .substitutions
                        .iter()
                        .any(|s| s.player_out_id == id
                            && s.reason == SubstitutionReason::CriticalInjury),
                "a man carried off is replaced while a change is left"
            );

            let mut player = persisted_player(id);
            player.on_match_exertion(
                MatchExertionInputs {
                    minutes: snapshot.minutes_played,
                    starting_condition: snapshot.starting_condition,
                    final_match_energy: snapshot.final_match_energy,
                    high_intensity_load_hint: snapshot.high_intensity_load_hint,
                    injury: snapshot.injury,
                },
                NaiveDate::from_ymd_opt(2030, 10, 1).unwrap(),
                false,
            );
            assert!(player.player_attributes.is_injured);
        }
    }
    assert!(carried_off > 0, "3000 matches and nobody carried off");
}

fn persisted_player(id: u32) -> Player {
    PlayerBuilder::new()
        .id(id)
        .full_name(FullName::new("T".to_string(), "P".to_string()))
        .birth_date(NaiveDate::from_ymd_opt(2004, 5, 1).unwrap())
        .country_id(1)
        .attributes(PersonAttributes::default())
        .skills(PlayerSkills::default())
        .player_attributes(PlayerAttributes::default())
        .positions(PlayerPositions {
            positions: vec![PlayerPosition {
                position: PlayerPositionType::MidfielderCenter,
                level: 18,
            }],
        })
        .build()
        .unwrap()
}

fn season_day(season_start: i32, into_season: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(season_start, 8, 1).unwrap() + chrono::Duration::days(into_season)
}

/// One man of this age and temperament, in a side of ordinary men.
fn side_around(age: i64, today: NaiveDate, steadiness: f32) -> MatchSquad {
    let mut side = ordinary(1, 100, ORDINARY_ABILITY);
    let subject = &mut side.main_squad[6];
    subject.birth_date = today - chrono::Duration::days(age * 365 + 100);
    subject.attributes.consistency = steadiness;
    subject.attributes.professionalism = steadiness;
    subject.skills.mental.concentration = steadiness;
    subject.skills.mental.composure = steadiness;
    side
}

/// His mean rating over `matches` matches through one season.
fn season_rating(age: i64, steadiness: f32, season_start: i32, matches: u64, salt: u64) -> f32 {
    let away = ordinary(2, 200, ORDINARY_ABILITY);
    let mut ratings = Vec::new();
    for round in 0..matches {
        let today = season_day(season_start, round as i64 * 7);
        let home = side_around(age, today, steadiness);
        let seed = salt + season_start as u64 * 1000 + round;
        ratings.push(
            play_on(&home, &away, seed, today).player_stats[&home.main_squad[6].id].match_rating,
        );
    }
    ratings.iter().sum::<f32>() / ratings.len() as f32
}

fn spread(values: &[f32]) -> (f32, f32) {
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32;
    (mean, variance.sqrt())
}

#[test]
fn a_season_holds_together() {
    let away = ordinary(2, 200, ORDINARY_ABILITY);
    // An erratic teenager, whose season moves furthest from his level.
    let half = |season: i32, first_day: i64, salt: u64| {
        let matches = 300;
        let mut sum = 0.0;
        for m in 0..matches {
            let today = season_day(season, first_day + m % 100);
            let home = side_around(19, today, 3.0);
            let seed = salt + season as u64 * 10_000 + m as u64;
            let result = play_on(&home, &away, seed, today);
            sum += result.player_stats[&home.main_squad[6].id].progressive_passes as f32;
        }
        sum / matches as f32
    };
    let seasons: Vec<(f32, f32)> = (2030..2046)
        .map(|season| (half(season, 0, 1), half(season, 120, 500_000)))
        .collect();
    let whole: Vec<f32> = seasons.iter().map(|(a, b)| (a + b) * 0.5).collect();
    let within = seasons.iter().map(|(a, b)| (a - b).abs()).sum::<f32>() / seasons.len() as f32;
    let (_, between) = spread(&whole);
    assert!(
        between > within * 2.0,
        "his seasons differ by {between:.3}, two halves of one by {within:.3}"
    );
}

#[test]
fn a_metronome_swings_less() {
    let seasons = 2030..2070;
    let steady: Vec<f32> = seasons
        .clone()
        .map(|s| season_rating(28, 18.0, s, 38, 1))
        .collect();
    let erratic: Vec<f32> = seasons
        .map(|s| season_rating(19, 3.0, s, 38, 900_000))
        .collect();
    let (steady_mean, steady_spread) = spread(&steady);
    let (erratic_mean, erratic_spread) = spread(&erratic);
    assert!(
        erratic_spread > steady_spread * 1.3,
        "the teenager's seasons spread {erratic_spread:.3}, the veteran's {steady_spread:.3}"
    );
    assert!(
        (steady_mean - erratic_mean).abs() < 0.15,
        "the means sit {steady_mean:.3} and {erratic_mean:.3} apart"
    );
}

#[test]
fn two_equal_players_have_different_seasons() {
    // Men of one level, position and club, season after season. What their
    // season ratings spread beyond the noise of 38 matches is the season.
    let away = ordinary(2, 200, ORDINARY_ABILITY);
    let mut season_means = Vec::new();
    let mut match_variance = 0.0;
    for season in 2030..2060 {
        let mut by_man: FxHashMap<u32, Vec<f32>> = FxHashMap::default();
        for round in 0..38u64 {
            let today = season_day(season, round as i64 * 7);
            let mut home = ordinary(1, 100, ORDINARY_ABILITY);
            for player in &mut home.main_squad {
                player.birth_date = today - chrono::Duration::days(26 * 365);
            }
            let seed = season as u64 * 1000 + round;
            let result = play_on(&home, &away, seed, today);
            for starter in &home.main_squad[5..9] {
                by_man
                    .entry(starter.id)
                    .or_default()
                    .push(result.player_stats[&starter.id].match_rating);
            }
        }
        for ratings in by_man.values() {
            let (mean, sd) = spread(ratings);
            season_means.push(mean);
            match_variance += sd * sd;
        }
    }
    let noise_alone = (match_variance / season_means.len() as f32 / 38.0).sqrt();
    let (_, season_spread) = spread(&season_means);
    assert!(
        season_spread > noise_alone * 1.25,
        "season ratings spread {season_spread:.3}, match noise alone gives {noise_alone:.3}"
    );
}
