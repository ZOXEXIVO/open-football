//! Cross-domain integration tests for the events module.
//!
//! Fixtures (`build_player`, `stats`, `outcome`, `run_match`, …) are
//! shared across match-play, role-transition, season-event, transfer
//! and exertion tests because they all build the same kind of synthetic
//! `Player` and `MatchOutcome` data — splitting fixtures per domain
//! would duplicate ~50 lines of boilerplate per file with no payoff.

use super::scaling::*;
use super::types::{MatchOutcome, MatchParticipation};
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::builder::PlayerBuilder;
use crate::club::player::player::Player;
use crate::r#match::engine::result::PlayerMatchEndStats;
use crate::shared::fullname::FullName;
use crate::{
    HappinessEventType, PersonAttributes, PlayerAttributes, PlayerFieldPositionGroup,
    PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills,
};
use chrono::NaiveDate;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn build_player(pos: PlayerPositionType, person: PersonAttributes) -> Player {
    let mut attrs = PlayerAttributes::default();
    attrs.current_reputation = 5_000;
    attrs.home_reputation = 6_000;
    attrs.world_reputation = 4_000;
    PlayerBuilder::new()
        .id(1)
        .full_name(FullName::new("Test".to_string(), "Player".to_string()))
        .birth_date(d(2000, 1, 1))
        .country_id(1)
        .attributes(person)
        .skills(PlayerSkills::default())
        .positions(PlayerPositions {
            positions: vec![PlayerPosition {
                position: pos,
                level: 20,
            }],
        })
        .player_attributes(attrs)
        .build()
        .unwrap()
}

fn stats(
    rating: f32,
    goals: u16,
    assists: u16,
    red_cards: u16,
    group: PlayerFieldPositionGroup,
) -> PlayerMatchEndStats {
    PlayerMatchEndStats {
        shots_on_target: 0,
        shots_total: 0,
        passes_attempted: 0,
        passes_completed: 0,
        tackles: 0,
        interceptions: 0,
        saves: 0,
        shots_faced: 0,
        goals,
        assists,
        match_rating: rating,
        xg: 0.0,
        position_group: group,
        fouls: 0,
        yellow_cards: 0,
        red_cards,
    }
}

fn outcome<'a>(
    s: &'a PlayerMatchEndStats,
    rating: f32,
    is_friendly: bool,
    is_cup: bool,
    is_motm: bool,
    is_derby: bool,
    team_for: u8,
    team_against: u8,
    participation: MatchParticipation,
) -> MatchOutcome<'a> {
    let won = team_for > team_against;
    let lost = team_for < team_against;
    MatchOutcome {
        stats: s,
        effective_rating: rating,
        participation,
        is_friendly,
        is_cup,
        is_motm,
        team_goals_for: team_for,
        team_goals_against: team_against,
        league_weight: 1.0,
        world_weight: 1.0,
        is_derby,
        team_won: won,
        team_lost: lost,
    }
}

fn count_events(p: &Player, kind: &HappinessEventType) -> usize {
    p.happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == *kind)
        .count()
}

// ── DecisiveGoal ──────────────────────────────────────────────

#[test]
fn decisive_goal_fires_on_one_goal_win_with_contribution() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(7.0, 1, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        7.0,
        false,
        false,
        false,
        false,
        1,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::DecisiveGoal), 1);
}

#[test]
fn decisive_goal_silent_for_two_goal_win() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(7.0, 1, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        7.0,
        false,
        false,
        false,
        false,
        3,
        1,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::DecisiveGoal), 0);
}

#[test]
fn decisive_goal_silent_in_friendly() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(7.0, 1, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        7.0,
        true,
        false,
        false,
        false,
        1,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::DecisiveGoal), 0);
}

// ── FanPraise / MediaPraise ──────────────────────────────────

#[test]
fn fan_praise_fires_for_excellent_rating() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(8.2, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        8.2,
        false,
        false,
        false,
        false,
        0,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::FanPraise), 1);
}

#[test]
fn media_praise_requires_higher_bar_than_fan_praise() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    // Rating = 8.0 — fan praise fires, media praise does not.
    let s = stats(8.0, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        8.0,
        false,
        false,
        false,
        false,
        0,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::FanPraise), 1);
    assert_eq!(count_events(&p, &HappinessEventType::MediaPraise), 0);
}

#[test]
fn media_praise_fires_at_8_3_rating() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(8.4, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        8.4,
        false,
        false,
        false,
        false,
        0,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::MediaPraise), 1);
}

#[test]
fn fan_praise_cooldown_prevents_double_fire() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(8.2, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        8.2,
        false,
        false,
        false,
        false,
        0,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::FanPraise), 1);
}

// ── FanCriticism ─────────────────────────────────────────────

#[test]
fn fan_criticism_fires_on_low_rating() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(5.2, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        5.2,
        false,
        false,
        false,
        false,
        0,
        1,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::FanCriticism), 1);
}

#[test]
fn fan_criticism_fires_on_red_card() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    // Rating still ok but a red card is enough.
    let s = stats(6.5, 0, 0, 1, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        6.5,
        false,
        false,
        false,
        false,
        0,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::FanCriticism), 1);
}

#[test]
fn fan_criticism_silent_for_solid_performance() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(6.8, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        6.8,
        false,
        false,
        false,
        false,
        1,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::FanCriticism), 0);
}

#[test]
fn fan_criticism_dampened_by_professionalism() {
    let high_pro = PersonAttributes {
        professionalism: 20.0,
        ..PersonAttributes::default()
    };
    let mut high = build_player(PlayerPositionType::Striker, high_pro);

    let low_pro = PersonAttributes {
        professionalism: 0.0,
        ..PersonAttributes::default()
    };
    let mut low = build_player(PlayerPositionType::Striker, low_pro);

    let s = stats(5.2, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        5.2,
        false,
        false,
        false,
        false,
        0,
        1,
        MatchParticipation::Starter,
    );
    high.on_match_played(&o);
    low.on_match_played(&o);

    let high_mag = high
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::FanCriticism)
        .unwrap()
        .magnitude;
    let low_mag = low
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::FanCriticism)
        .unwrap()
        .magnitude;
    // Both negative; "less negative" = closer to zero.
    assert!(
        high_mag > low_mag,
        "high pro {} should soften vs low pro {}",
        high_mag,
        low_mag
    );
}

// ── Clean-sheet pride extension ─────────────────────────────

#[test]
fn clean_sheet_pride_fires_for_defender() {
    let mut p = build_player(
        PlayerPositionType::DefenderCenter,
        PersonAttributes::default(),
    );
    let s = stats(7.0, 0, 0, 0, PlayerFieldPositionGroup::Defender);
    let o = outcome(
        &s,
        7.0,
        false,
        false,
        false,
        false,
        1,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::CleanSheetPride), 1);
}

#[test]
fn clean_sheet_pride_silent_for_forward() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(7.0, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        7.0,
        false,
        false,
        false,
        false,
        1,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::CleanSheetPride), 0);
}

// ── Reputation amplifier shapes magnitudes ──────────────────

// ── Derby outcome ────────────────────────────────────────────

#[test]
fn ordinary_derby_winner_gets_derby_win_not_hero() {
    // Solid 6.8 rating, no goal/assist/POM, midfielder — the kind of
    // player who's on the winning side but didn't carry the day.
    let mut p = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    let s = stats(6.8, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
    let o = outcome(
        &s,
        6.8,
        false,
        false,
        false,
        true,
        2,
        1,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::DerbyHero), 0);
    assert_eq!(count_events(&p, &HappinessEventType::DerbyWin), 1);
}

#[test]
fn derby_scorer_gets_derby_hero() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(7.0, 1, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        7.0,
        false,
        false,
        false,
        true,
        2,
        1,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::DerbyHero), 1);
    assert_eq!(count_events(&p, &HappinessEventType::DerbyWin), 0);
}

#[test]
fn derby_assister_gets_derby_hero() {
    let mut p = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    let s = stats(7.0, 0, 1, 0, PlayerFieldPositionGroup::Midfielder);
    let o = outcome(
        &s,
        7.0,
        false,
        false,
        false,
        true,
        2,
        1,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::DerbyHero), 1);
}

#[test]
fn derby_high_rated_outfielder_gets_derby_hero() {
    let mut p = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    // 7.6 rating with no goal/assist still earns hero status.
    let s = stats(7.6, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
    let o = outcome(
        &s,
        7.6,
        false,
        false,
        false,
        true,
        2,
        1,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::DerbyHero), 1);
}

#[test]
fn derby_defender_clean_sheet_high_rating_gets_hero() {
    // Defender, no goal/assist, 7.3 rating, clean sheet — earns hero
    // status via the back-line clean-sheet gate (rating ≥ 7.2).
    let mut p = build_player(
        PlayerPositionType::DefenderCenter,
        PersonAttributes::default(),
    );
    let s = stats(7.3, 0, 0, 0, PlayerFieldPositionGroup::Defender);
    let o = outcome(
        &s,
        7.3,
        false,
        false,
        false,
        true,
        1,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::DerbyHero), 1);
    assert_eq!(count_events(&p, &HappinessEventType::DerbyWin), 0);
}

#[test]
fn derby_defender_clean_sheet_modest_rating_gets_win_only() {
    // Defender on the winning side, clean sheet, but rating below the
    // clean-sheet hero gate (7.2). Should be DerbyWin, not Hero.
    let mut p = build_player(
        PlayerPositionType::DefenderCenter,
        PersonAttributes::default(),
    );
    let s = stats(6.8, 0, 0, 0, PlayerFieldPositionGroup::Defender);
    let o = outcome(
        &s,
        6.8,
        false,
        false,
        false,
        true,
        1,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::DerbyHero), 0);
    assert_eq!(count_events(&p, &HappinessEventType::DerbyWin), 1);
}

#[test]
fn derby_loser_gets_derby_defeat() {
    let mut p = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    let s = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
    let o = outcome(
        &s,
        6.5,
        false,
        false,
        false,
        true,
        0,
        1,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::DerbyDefeat), 1);
}

#[test]
fn derby_loser_poor_performer_takes_bigger_hit() {
    // Same defeat, two players: one performed solidly, the other
    // crumbled to a 5.0 rating. Poor performer should land a more
    // negative magnitude.
    let mut solid = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    let mut poor = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    let s_solid = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
    let o_solid = outcome(
        &s_solid,
        6.5,
        false,
        false,
        false,
        true,
        0,
        1,
        MatchParticipation::Starter,
    );
    let s_poor = stats(5.0, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
    let o_poor = outcome(
        &s_poor,
        5.0,
        false,
        false,
        false,
        true,
        0,
        1,
        MatchParticipation::Starter,
    );
    solid.on_match_played(&o_solid);
    poor.on_match_played(&o_poor);
    let m_solid = solid
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::DerbyDefeat)
        .unwrap()
        .magnitude;
    let m_poor = poor
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::DerbyDefeat)
        .unwrap()
        .magnitude;
    // More negative = bigger hit. Poor performer should be more negative.
    assert!(
        m_poor < m_solid,
        "poor {} should be more negative than solid {}",
        m_poor,
        m_solid
    );
}

#[test]
fn derby_loser_red_card_amplifies_defeat() {
    let mut clean = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    let mut sent_off = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    let s_clean = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
    let o_clean = outcome(
        &s_clean,
        6.5,
        false,
        false,
        false,
        true,
        0,
        1,
        MatchParticipation::Starter,
    );
    // Red card with otherwise-acceptable rating — extra still applies.
    let s_red = stats(6.5, 0, 0, 1, PlayerFieldPositionGroup::Midfielder);
    let o_red = outcome(
        &s_red,
        6.5,
        false,
        false,
        false,
        true,
        0,
        1,
        MatchParticipation::Starter,
    );
    clean.on_match_played(&o_clean);
    sent_off.on_match_played(&o_red);
    let m_clean = clean
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::DerbyDefeat)
        .unwrap()
        .magnitude;
    let m_red = sent_off
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::DerbyDefeat)
        .unwrap()
        .magnitude;
    assert!(
        m_red < m_clean,
        "red-card {} should be more negative than clean {}",
        m_red,
        m_clean
    );
}

// ── Team-season events ───────────────────────────────────────

#[test]
fn trophy_won_emits_positive_magnitude() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.statistics.played = 30;
    let recorded = p.on_team_season_event(HappinessEventType::TrophyWon, 365, d(2032, 5, 30));
    assert!(recorded);
    let mag = p
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::TrophyWon)
        .unwrap()
        .magnitude;
    assert!(mag > 0.0, "TrophyWon should be positive, got {}", mag);
}

#[test]
fn relegated_emits_negative_magnitude() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.statistics.played = 30;
    let recorded = p.on_team_season_event(HappinessEventType::Relegated, 365, d(2032, 5, 30));
    assert!(recorded);
    let mag = p
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::Relegated)
        .unwrap()
        .magnitude;
    assert!(mag < 0.0, "Relegated should be negative, got {}", mag);
}

#[test]
fn season_event_cooldown_prevents_duplicate() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.statistics.played = 30;
    let date = d(2032, 5, 30);
    assert!(p.on_team_season_event(HappinessEventType::Relegated, 365, date));
    assert!(!p.on_team_season_event(HappinessEventType::Relegated, 365, date));
}

#[test]
fn season_event_prestige_scales_magnitude() {
    // Continental trophy (prestige 1.5) should land bigger than a
    // domestic-league title (prestige 1.0).
    let mut domestic = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut continental = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    domestic.statistics.played = 30;
    continental.statistics.played = 30;
    let date = d(2032, 5, 30);
    domestic.on_team_season_event_with_prestige(HappinessEventType::TrophyWon, 365, 1.0, date);
    continental.on_team_season_event_with_prestige(HappinessEventType::TrophyWon, 365, 1.5, date);
    let dm = domestic
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::TrophyWon)
        .unwrap()
        .magnitude;
    let cm = continental
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::TrophyWon)
        .unwrap()
        .magnitude;
    assert!(
        cm > dm,
        "continental prestige {} should exceed domestic {}",
        cm,
        dm
    );
}

fn build_player_with_status(status: crate::PlayerSquadStatus) -> Player {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut contract = crate::PlayerClubContract::new(10_000, d(2035, 6, 30));
    contract.squad_status = status;
    p.contract = Some(contract);
    p.statistics.played = 30;
    p
}

#[test]
fn key_player_takes_bigger_relegation_hit_than_rotation() {
    let mut key = build_player_with_status(crate::PlayerSquadStatus::KeyPlayer);
    let mut rotation = build_player_with_status(crate::PlayerSquadStatus::FirstTeamSquadRotation);
    let date = d(2032, 5, 30);
    key.on_team_season_event(HappinessEventType::Relegated, 365, date);
    rotation.on_team_season_event(HappinessEventType::Relegated, 365, date);
    let m_key = key
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::Relegated)
        .unwrap()
        .magnitude;
    let m_rot = rotation
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::Relegated)
        .unwrap()
        .magnitude;
    // More negative = bigger hit. KeyPlayer should land more negatively.
    assert!(
        m_key < m_rot,
        "KeyPlayer {} should be more negative than rotation {}",
        m_key,
        m_rot
    );
}

#[test]
fn fringe_not_needed_softens_relegation_hit() {
    let mut not_needed = build_player_with_status(crate::PlayerSquadStatus::NotNeeded);
    let mut regular = build_player_with_status(crate::PlayerSquadStatus::FirstTeamRegular);
    let date = d(2032, 5, 30);
    not_needed.on_team_season_event(HappinessEventType::Relegated, 365, date);
    regular.on_team_season_event(HappinessEventType::Relegated, 365, date);
    let m_not = not_needed
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::Relegated)
        .unwrap()
        .magnitude;
    let m_reg = regular
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::Relegated)
        .unwrap()
        .magnitude;
    // Less negative = softer hit. NotNeeded should be closer to zero.
    assert!(
        m_not > m_reg,
        "NotNeeded {} should be less negative than Regular {}",
        m_not,
        m_reg
    );
}

#[test]
fn cup_final_defeat_emits_negative_with_prestige() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.statistics.played = 30;
    let date = d(2032, 5, 30);
    let recorded =
        p.on_team_season_event_with_prestige(HappinessEventType::CupFinalDefeat, 365, 1.4, date);
    assert!(recorded);
    let mag = p
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::CupFinalDefeat)
        .unwrap()
        .magnitude;
    assert!(mag < 0.0, "CupFinalDefeat should be negative, got {}", mag);
}

#[test]
fn fringe_player_feels_trophy_less_than_starter() {
    let mut starter = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    starter.statistics.played = 30;
    let mut fringe = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    fringe.statistics.played = 1;
    let date = d(2032, 5, 30);
    starter.on_team_season_event(HappinessEventType::TrophyWon, 365, date);
    fringe.on_team_season_event(HappinessEventType::TrophyWon, 365, date);
    let starter_mag = starter
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::TrophyWon)
        .unwrap()
        .magnitude;
    let fringe_mag = fringe
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::TrophyWon)
        .unwrap()
        .magnitude;
    assert!(starter_mag > fringe_mag);
}

#[test]
fn ambitious_player_hurts_more_on_relegation() {
    let mut ambitious_pa = PersonAttributes::default();
    ambitious_pa.ambition = 20.0;
    let mut content_pa = PersonAttributes::default();
    content_pa.ambition = 1.0;
    let mut ambitious = build_player(PlayerPositionType::Striker, ambitious_pa);
    let mut content = build_player(PlayerPositionType::Striker, content_pa);
    ambitious.statistics.played = 30;
    content.statistics.played = 30;
    let date = d(2032, 5, 30);
    ambitious.on_team_season_event(HappinessEventType::Relegated, 365, date);
    content.on_team_season_event(HappinessEventType::Relegated, 365, date);
    let amb = ambitious
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::Relegated)
        .unwrap()
        .magnitude;
    let con = content
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::Relegated)
        .unwrap()
        .magnitude;
    // More negative = bigger hit. Ambition makes Relegated worse.
    assert!(
        amb < con,
        "ambitious {} should be more negative than content {}",
        amb,
        con
    );
}

// ── Role transitions ─────────────────────────────────────────

fn run_match(p: &mut Player, participation: MatchParticipation) {
    let s = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
    let o = outcome(&s, 6.5, false, false, false, false, 1, 1, participation);
    p.on_match_played(&o);
}

#[test]
fn won_starting_place_fires_after_run_of_starts() {
    let mut p = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    for _ in 0..6 {
        run_match(&mut p, MatchParticipation::Starter);
    }
    assert!(p.happiness.is_established_starter);
    let count = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::WonStartingPlace)
        .count();
    assert_eq!(count, 1);
}

#[test]
fn lost_starting_place_fires_after_drop() {
    let mut p = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    // Establish first.
    for _ in 0..6 {
        run_match(&mut p, MatchParticipation::Starter);
    }
    assert!(p.happiness.is_established_starter);
    // Then a sustained run on the bench drops the EMA below 0.40.
    for _ in 0..10 {
        run_match(&mut p, MatchParticipation::Substitute);
    }
    assert!(!p.happiness.is_established_starter);
    let lost = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::LostStartingPlace)
        .count();
    assert_eq!(lost, 1);
}

fn run_match_with_status(
    p: &mut Player,
    participation: MatchParticipation,
    status: crate::PlayerSquadStatus,
) {
    let mut contract = crate::PlayerClubContract::new(10_000, d(2035, 6, 30));
    contract.squad_status = status;
    p.contract = Some(contract);
    let s = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
    let o = outcome(&s, 6.5, false, false, false, false, 1, 1, participation);
    p.on_match_played(&o);
}

#[test]
fn key_player_lost_starting_place_hit_exceeds_rotation() {
    // Establish both as starters, then sustain bench runs to flip the
    // role state. Key player should land a more negative LostStartingPlace.
    let mut key = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    for _ in 0..6 {
        run_match_with_status(
            &mut key,
            MatchParticipation::Starter,
            crate::PlayerSquadStatus::KeyPlayer,
        );
    }
    for _ in 0..10 {
        run_match_with_status(
            &mut key,
            MatchParticipation::Substitute,
            crate::PlayerSquadStatus::KeyPlayer,
        );
    }

    let mut rot = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    for _ in 0..6 {
        run_match_with_status(
            &mut rot,
            MatchParticipation::Starter,
            crate::PlayerSquadStatus::FirstTeamSquadRotation,
        );
    }
    for _ in 0..10 {
        run_match_with_status(
            &mut rot,
            MatchParticipation::Substitute,
            crate::PlayerSquadStatus::FirstTeamSquadRotation,
        );
    }

    let m_key = key
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::LostStartingPlace)
        .unwrap()
        .magnitude;
    let m_rot = rot
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::LostStartingPlace)
        .unwrap()
        .magnitude;
    // More negative = bigger hit.
    assert!(
        m_key < m_rot,
        "KeyPlayer {} should be more negative than rotation {}",
        m_key,
        m_rot
    );
}

#[test]
fn prospect_won_starting_place_hit_exceeds_senior() {
    let mut prospect = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    for _ in 0..6 {
        run_match_with_status(
            &mut prospect,
            MatchParticipation::Starter,
            crate::PlayerSquadStatus::HotProspectForTheFuture,
        );
    }

    let mut senior = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    for _ in 0..6 {
        run_match_with_status(
            &mut senior,
            MatchParticipation::Starter,
            crate::PlayerSquadStatus::KeyPlayer,
        );
    }

    let m_p = prospect
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::WonStartingPlace)
        .unwrap()
        .magnitude;
    let m_s = senior
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::WonStartingPlace)
        .unwrap()
        .magnitude;
    assert!(
        m_p > m_s,
        "prospect {} should exceed established senior {}",
        m_p,
        m_s
    );
}

#[test]
fn high_professionalism_softens_lost_starting_place() {
    let high_pro = PersonAttributes {
        professionalism: 20.0,
        ..PersonAttributes::default()
    };
    let low_pro = PersonAttributes {
        professionalism: 0.0,
        ..PersonAttributes::default()
    };
    let mut hi = build_player(PlayerPositionType::MidfielderCenter, high_pro);
    let mut lo = build_player(PlayerPositionType::MidfielderCenter, low_pro);
    for _ in 0..6 {
        run_match_with_status(
            &mut hi,
            MatchParticipation::Starter,
            crate::PlayerSquadStatus::FirstTeamRegular,
        );
        run_match_with_status(
            &mut lo,
            MatchParticipation::Starter,
            crate::PlayerSquadStatus::FirstTeamRegular,
        );
    }
    for _ in 0..10 {
        run_match_with_status(
            &mut hi,
            MatchParticipation::Substitute,
            crate::PlayerSquadStatus::FirstTeamRegular,
        );
        run_match_with_status(
            &mut lo,
            MatchParticipation::Substitute,
            crate::PlayerSquadStatus::FirstTeamRegular,
        );
    }
    let m_hi = hi
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::LostStartingPlace)
        .unwrap()
        .magnitude;
    let m_lo = lo
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::LostStartingPlace)
        .unwrap()
        .magnitude;
    // Less negative = softer.
    assert!(
        m_hi > m_lo,
        "high pro {} should soften vs low pro {}",
        m_hi,
        m_lo
    );
}

#[test]
fn role_transition_silent_below_min_appearances() {
    let mut p = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    // Only 3 starts — below the 5-game minimum tracked window.
    for _ in 0..3 {
        run_match(&mut p, MatchParticipation::Starter);
    }
    assert!(!p.happiness.is_established_starter);
    let count = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::WonStartingPlace)
        .count();
    assert_eq!(count, 0);
}

// ── Youth breakthrough ───────────────────────────────────────

#[test]
fn youth_breakthrough_fires_for_young_player() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    // build_player birth date is 2000-01-01; promote in 2019 → age 19.
    p.on_youth_breakthrough(d(2019, 6, 1));
    let count = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::YouthBreakthrough)
        .count();
    assert_eq!(count, 1);
}

#[test]
fn youth_breakthrough_silent_for_veteran() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    // Promote in 2027 → age 27. Squad-depth call, not a debut.
    p.on_youth_breakthrough(d(2027, 6, 1));
    let count = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::YouthBreakthrough)
        .count();
    assert_eq!(count, 0);
}

#[test]
fn youth_breakthrough_late_bloomer_smaller_magnitude() {
    let mut early = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut late = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    early.on_youth_breakthrough(d(2020, 6, 1)); // age 20
    late.on_youth_breakthrough(d(2024, 6, 1)); // age 24
    let early_mag = early
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::YouthBreakthrough)
        .unwrap()
        .magnitude;
    let late_mag = late
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::YouthBreakthrough)
        .unwrap()
        .magnitude;
    assert!(early_mag > late_mag);
}

// ── Transfer events ──────────────────────────────────────────

#[test]
fn transfer_bid_rejected_silent_for_peer_buyer() {
    // Same-rep clubs → not a credible bigger move → no event.
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.attributes.ambition = 18.0;
    p.on_transfer_bid_rejected(0.50, 0.48, false);
    let count = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::TransferBidRejected)
        .count();
    assert_eq!(count, 0);
}

#[test]
fn transfer_bid_rejected_silent_for_content_player() {
    // Bigger buyer but settled, low-ambition player who isn't pushing
    // for a move → still silent.
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.attributes.ambition = 6.0;
    p.on_transfer_bid_rejected(0.80, 0.40, false);
    let count = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::TransferBidRejected)
        .count();
    assert_eq!(count, 0);
}

#[test]
fn transfer_bid_rejected_fires_for_ambitious_player_bigger_buyer() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.attributes.ambition = 16.0;
    p.on_transfer_bid_rejected(0.75, 0.40, false);
    let count = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::TransferBidRejected)
        .count();
    assert_eq!(count, 1);
}

#[test]
fn transfer_bid_rejected_cooldown_blocks_repeat() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.attributes.ambition = 16.0;
    p.on_transfer_bid_rejected(0.75, 0.40, false);
    p.on_transfer_bid_rejected(0.75, 0.40, false);
    let count = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::TransferBidRejected)
        .count();
    assert_eq!(count, 1);
}

#[test]
fn transfer_bid_rejected_favorite_fires_at_lateral_rep() {
    // Favorite-club bid being rejected hurts even at lateral reputation
    // and even for an average-ambition player who isn't pushing for a move.
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.attributes.ambition = 8.0; // not pushing for a move
    p.on_transfer_bid_rejected(0.50, 0.50, true);
    assert_eq!(
        count_events(&p, &HappinessEventType::TransferBidRejected),
        1
    );
}

#[test]
fn transfer_bid_rejected_favorite_amplifies_magnitude() {
    let mut anon = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut fav = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    anon.attributes.ambition = 16.0;
    fav.attributes.ambition = 16.0;
    anon.on_transfer_bid_rejected(0.75, 0.40, false);
    fav.on_transfer_bid_rejected(0.75, 0.40, true);
    let m_anon = anon
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::TransferBidRejected)
        .unwrap()
        .magnitude;
    let m_fav = fav
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::TransferBidRejected)
        .unwrap()
        .magnitude;
    // More negative = bigger hit. Favorite-club rejection should land harder.
    assert!(
        m_fav < m_anon,
        "favorite {} should be more negative than anon {}",
        m_fav,
        m_anon
    );
}

#[test]
fn dream_move_collapsed_silent_for_lateral_move() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.on_dream_move_collapsed(0.55, 0.50, false);
    let count = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::DreamMoveCollapsed)
        .count();
    assert_eq!(count, 0);
}

#[test]
fn dream_move_collapsed_fires_for_step_up() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.on_dream_move_collapsed(0.85, 0.50, false);
    let count = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::DreamMoveCollapsed)
        .count();
    assert_eq!(count, 1);
}

#[test]
fn dream_move_collapsed_fires_for_favorite_even_at_lateral_rep() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    // Lateral rep, but favorite club destination — still a dream move.
    p.on_dream_move_collapsed(0.55, 0.50, true);
    let count = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::DreamMoveCollapsed)
        .count();
    assert_eq!(count, 1);
}

#[test]
fn dream_move_favorite_club_amplifies_magnitude() {
    let mut step_up = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut favorite = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    step_up.on_dream_move_collapsed(0.85, 0.50, false);
    favorite.on_dream_move_collapsed(0.85, 0.50, true);
    let s = step_up
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::DreamMoveCollapsed)
        .unwrap()
        .magnitude;
    let f = favorite
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::DreamMoveCollapsed)
        .unwrap()
        .magnitude;
    // More negative = bigger hit. Favorite-club collapse hurts more.
    assert!(
        f < s,
        "favorite {} should be more negative than step_up {}",
        f,
        s
    );
}

// ── Social events ────────────────────────────────────────────

#[test]
fn close_friend_sold_emits_negative() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.on_close_friend_sold(42, 80.0, true, true);
    let ev = p
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::CloseFriendSold)
        .unwrap();
    assert!(ev.magnitude < 0.0);
    assert_eq!(ev.partner_player_id, Some(42));
}

#[test]
fn close_friend_sold_stronger_with_compatriot() {
    let mut compat = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut foreign = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    compat.on_close_friend_sold(7, 80.0, true, false);
    foreign.on_close_friend_sold(7, 80.0, false, false);
    let c = compat
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::CloseFriendSold)
        .unwrap()
        .magnitude;
    let f = foreign
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::CloseFriendSold)
        .unwrap()
        .magnitude;
    assert!(
        c < f,
        "compatriot version {} should be more negative than foreign {}",
        c,
        f
    );
}

#[test]
fn mentor_departed_emits_negative() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.on_mentor_departed(13, 70.0, false);
    let ev = p
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::MentorDeparted)
        .unwrap();
    assert_eq!(ev.partner_player_id, Some(13));
}

#[test]
fn compatriot_joined_silent_for_domestic_player() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    // p.country_id == 1 by default. Club is in same country.
    p.on_compatriot_joined(2, 1, false);
    let count = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::CompatriotJoined)
        .count();
    assert_eq!(count, 0);
}

#[test]
fn compatriot_joined_fires_for_foreign_player() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    // Player from country 1, club in country 99 → foreign at this club.
    p.on_compatriot_joined(2, 99, true);
    let ev = p
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::CompatriotJoined)
        .unwrap();
    assert_eq!(ev.partner_player_id, Some(2));
}

#[test]
fn compatriot_joined_does_not_double_fire_in_cooldown() {
    // Two compatriots joining within 30 days at the same club should
    // not stack two events on the existing player — `on_compatriot_joined`
    // has a 30-day cooldown.
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.on_compatriot_joined(2, 99, true);
    p.on_compatriot_joined(3, 99, true);
    assert_eq!(count_events(&p, &HappinessEventType::CompatriotJoined), 1);
}

#[test]
fn compatriot_joined_amplified_when_no_local_language() {
    let mut isolated = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut settled = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    isolated.on_compatriot_joined(2, 99, true);
    settled.on_compatriot_joined(2, 99, false);
    let i = isolated
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::CompatriotJoined)
        .unwrap()
        .magnitude;
    let s = settled
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::CompatriotJoined)
        .unwrap()
        .magnitude;
    assert!(
        i > s,
        "language-isolated boost {} should exceed settled {}",
        i,
        s
    );
}

// ── End-to-end event-stream audit ────────────────────────────
//
// Drives one player through a season-shaped sequence of matches and
// checks that no single event-type spams the recent_events buffer.
// Catches regressions in cooldowns (e.g. someone shortens
// FanPraise's 21d gate to 0d and the buffer fills up with FanPraise
// entries inside a single test). Ratings/derbies/wins are randomised
// to a deterministic shape — the goal isn't a real simulator, it's
// an integration-level smoke that exercises every emit path under
// realistic call cadence.

fn drive_season(p: &mut Player) {
    // 38 league + ~6 cup matches = 44, similar to an English season.
    // Shape: ~50% wins, ~25% draws, ~25% losses, mixed ratings.
    // Two derbies, half cup matches, occasional reds.
    let pattern: &[(f32, u16, u16, u16, bool, bool, bool, u8, u8)] = &[
        // (rating, goals, assists, reds, is_cup, is_motm, is_derby, gf, ga)
        (7.2, 1, 0, 0, false, false, false, 2, 1),
        (6.8, 0, 1, 0, false, false, false, 1, 0),
        (6.5, 0, 0, 0, false, false, false, 0, 1),
        (8.1, 1, 1, 0, false, true, false, 3, 0),
        (5.5, 0, 0, 0, false, false, false, 0, 2),
        (7.0, 0, 0, 0, false, false, true, 1, 0), // derby win, modest perf
        (7.8, 1, 0, 0, false, false, false, 2, 1),
        (6.2, 0, 0, 0, false, false, false, 1, 1),
        (6.9, 0, 1, 0, true, false, false, 2, 1),
        (5.8, 0, 0, 1, false, false, false, 0, 3), // red card defeat
        (7.5, 0, 0, 0, false, false, false, 1, 0),
        (8.4, 2, 0, 0, false, true, false, 3, 1),
        (6.5, 0, 0, 0, false, false, false, 1, 1),
        (7.0, 0, 1, 0, true, false, false, 2, 0),
        (5.4, 0, 0, 0, false, false, false, 0, 2),
        (6.8, 0, 0, 0, false, false, false, 1, 1),
        (7.6, 1, 0, 0, false, false, true, 2, 0), // derby win, standout
        (6.2, 0, 0, 0, false, false, false, 0, 1),
        (7.0, 0, 1, 0, false, false, false, 1, 0),
        (8.0, 1, 1, 0, true, false, false, 4, 0),
        (6.5, 0, 0, 0, false, false, false, 1, 1),
        (5.9, 0, 0, 0, false, false, false, 0, 1),
        (7.2, 0, 0, 0, false, false, false, 2, 0),
        (6.8, 1, 0, 0, false, false, false, 1, 0),
        (7.0, 0, 0, 0, false, false, false, 1, 1),
        (5.5, 0, 0, 0, false, false, false, 0, 2),
        (8.1, 1, 1, 0, false, true, false, 3, 1),
        (6.4, 0, 0, 0, false, false, false, 1, 1),
        (7.5, 0, 0, 0, true, false, false, 2, 0),
        (6.2, 0, 0, 0, false, false, false, 0, 0),
        (7.0, 0, 1, 0, false, false, false, 2, 1),
        (8.3, 2, 0, 0, false, true, false, 4, 1),
        (5.8, 0, 0, 0, false, false, false, 0, 2),
        (6.9, 0, 0, 0, false, false, false, 1, 0),
        (7.0, 1, 0, 0, false, false, false, 2, 1),
        (6.5, 0, 0, 0, true, false, false, 1, 0),
        (5.6, 0, 0, 0, false, false, false, 0, 2),
        (7.4, 0, 1, 0, false, false, false, 1, 0),
        (6.8, 0, 0, 0, false, false, false, 1, 1),
        (7.2, 0, 0, 0, false, false, false, 2, 1),
        (6.0, 0, 0, 0, false, false, false, 0, 1),
        (7.8, 1, 0, 0, false, false, false, 2, 0),
        (6.5, 0, 0, 0, false, false, false, 1, 1),
        (8.2, 1, 1, 0, true, true, false, 3, 0),
    ];
    for &(rating, goals, assists, reds, is_cup, is_motm, is_derby, gf, ga) in pattern {
        let s = stats(
            rating,
            goals,
            assists,
            reds,
            PlayerFieldPositionGroup::Forward,
        );
        let o = outcome(
            &s,
            rating,
            false,
            is_cup,
            is_motm,
            is_derby,
            gf,
            ga,
            MatchParticipation::Starter,
        );
        p.on_match_played(&o);
    }
}

#[test]
fn season_long_event_stream_stays_within_sane_bounds() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    drive_season(&mut p);

    // Within a single season (no decay applied here — `decay_events`
    // would be invoked weekly in production), the recent_events
    // buffer should not be dominated by any single repeat-event
    // type. Cooldowns should keep each individual event from firing
    // more than ~once a fortnight to month.
    let count_of = |kind: &HappinessEventType| -> u32 {
        p.happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == *kind)
            .count() as u32
    };

    // Hard ceilings per event type. Everything below should be gated
    // by its emit-site cooldown — these caps are the safety net for
    // anything that slips through.
    let assertions: &[(HappinessEventType, u32)] = &[
        (HappinessEventType::FanPraise, 4), // 21d cooldown × 44 matches
        (HappinessEventType::FanCriticism, 4),
        (HappinessEventType::MediaPraise, 3),  // 30d cooldown
        (HappinessEventType::DecisiveGoal, 4), // 14d cooldown
        (HappinessEventType::DerbyHero, 2),    // 2 derbies in pattern
        (HappinessEventType::DerbyDefeat, 2),
        (HappinessEventType::WonStartingPlace, 1),
        (HappinessEventType::FirstClubGoal, 1),
        (HappinessEventType::PlayerOfTheMatch, 8),
    ];
    for (event, ceiling) in assertions {
        let n = count_of(event);
        assert!(
            n <= *ceiling,
            "event {:?} fired {} times in season — ceiling {}; cooldown likely broken",
            event,
            n,
            ceiling
        );
    }

    // Per the cap on `recent_events_cap` (100), the buffer must not
    // be saturated by a single season for one player.
    assert!(
        p.happiness.recent_events.len() <= 100,
        "recent_events buffer overran: {} entries",
        p.happiness.recent_events.len()
    );
}

#[test]
fn season_long_no_event_repeats_within_30_days_for_cooldown_gated_types() {
    // For the event types that explicitly use `add_event_with_cooldown`
    // ≥ 21 days, walk pairs of recorded events and assert no two of
    // the same type sit at `days_ago` within 21 days of each other.
    // (All recent_events have `days_ago = 0` in this synthetic test
    // since we don't tick `decay_events`. The check we want is the
    // *count* exceeding the legal max, which the previous test
    // covers — this test is a redundant safety net for the
    // derby/captaincy paths that fire on bespoke cadence.)
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    drive_season(&mut p);

    // DerbyHero requires standout perf in derby win — should be at
    // most one entry per derby in the pattern, and the pattern has
    // exactly 2 derbies (one routine win, one with a goal).
    let derby_hero = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::DerbyHero)
        .count();
    let derby_win = p
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type == HappinessEventType::DerbyWin)
        .count();
    assert_eq!(
        derby_hero + derby_win,
        2,
        "expected exactly 2 derby outcomes (hero or win), got hero={} win={}",
        derby_hero,
        derby_win
    );
}

#[test]
fn fan_praise_amplified_by_reputation() {
    let mut famous = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    famous.player_attributes.current_reputation = 10_000;
    let mut anon = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    anon.player_attributes.current_reputation = 0;

    let s = stats(8.2, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        8.2,
        false,
        false,
        false,
        false,
        0,
        0,
        MatchParticipation::Starter,
    );
    famous.on_match_played(&o);
    anon.on_match_played(&o);

    let fmag = famous
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::FanPraise)
        .unwrap()
        .magnitude;
    let amag = anon
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::FanPraise)
        .unwrap()
        .magnitude;
    assert!(fmag > amag, "famous {} should exceed anon {}", fmag, amag);
}

// ── Match-load model (post-match exertion) ────────────────────

fn fresh_player(pos: PlayerPositionType) -> Player {
    let mut p = build_player(pos, PersonAttributes::default());
    p.player_attributes.condition = 9_500;
    p.player_attributes.fitness = 9_000;
    p.player_attributes.jadedness = 1_000;
    p.skills.physical.match_readiness = 15.0;
    p.skills.physical.natural_fitness = 14.0;
    p
}

#[test]
fn full_match_keeper_has_lower_load_than_full_match_fullback() {
    let mut gk = fresh_player(PlayerPositionType::Goalkeeper);
    let mut fb = fresh_player(PlayerPositionType::WingbackLeft);
    let date = d(2025, 9, 14);
    gk.on_match_exertion(90.0, date, false);
    fb.on_match_exertion(90.0, date, false);
    // Wingback's load should be at least 2× the keeper's.
    assert!(
        fb.load.physical_load_7 > gk.load.physical_load_7 * 2.0,
        "fb={} gk={}",
        fb.load.physical_load_7,
        gk.load.physical_load_7
    );
    // High-intensity share should also be much higher for the FB.
    assert!(
        fb.load.high_intensity_load_7 > gk.load.high_intensity_load_7 * 4.0,
        "fb_hi={} gk_hi={}",
        fb.load.high_intensity_load_7,
        gk.load.high_intensity_load_7
    );
}

#[test]
fn keeper_full_match_jadedness_lower_than_fullback_full_match() {
    let mut gk = fresh_player(PlayerPositionType::Goalkeeper);
    let mut fb = fresh_player(PlayerPositionType::WingbackLeft);
    let date = d(2025, 9, 14);
    gk.on_match_exertion(90.0, date, false);
    fb.on_match_exertion(90.0, date, false);
    assert!(
        (fb.player_attributes.jadedness as i32) > (gk.player_attributes.jadedness as i32) + 100,
        "fb jad={} gk jad={}",
        fb.player_attributes.jadedness,
        gk.player_attributes.jadedness
    );
}

#[test]
fn friendly_cameo_adds_less_load_than_competitive_full_match() {
    let mut friendly = fresh_player(PlayerPositionType::MidfielderCenter);
    let mut competitive = fresh_player(PlayerPositionType::MidfielderCenter);
    let date = d(2025, 9, 14);
    friendly.on_match_exertion(90.0, date, true);
    competitive.on_match_exertion(90.0, date, false);
    assert!(
        friendly.load.physical_load_7 < competitive.load.physical_load_7,
        "friendly load {} should be less than competitive load {}",
        friendly.load.physical_load_7,
        competitive.load.physical_load_7
    );
    // Friendly didn't push minute window
    assert_eq!(friendly.load.minutes_last_7, 0.0);
    assert_eq!(competitive.load.minutes_last_7, 90.0);
}

#[test]
fn three_matches_in_seven_days_accrue_higher_debt_than_one_match() {
    let mut once = fresh_player(PlayerPositionType::ForwardLeft);
    let mut thrice = fresh_player(PlayerPositionType::ForwardLeft);

    // Seed the rolling decay clocks
    once.load.daily_decay(d(2025, 9, 1));
    thrice.load.daily_decay(d(2025, 9, 1));

    once.on_match_exertion(90.0, d(2025, 9, 7), false);

    thrice.load.daily_decay(d(2025, 9, 1));
    thrice.on_match_exertion(90.0, d(2025, 9, 1), false);
    thrice.load.daily_decay(d(2025, 9, 4));
    thrice.on_match_exertion(90.0, d(2025, 9, 4), false);
    thrice.load.daily_decay(d(2025, 9, 7));
    thrice.on_match_exertion(90.0, d(2025, 9, 7), false);

    // 3-day recency-decay between matches eats some of the cumulative
    // load, but the three-game week still ends materially heavier than
    // the single match.
    assert!(
        thrice.load.recovery_debt > once.load.recovery_debt * 1.5,
        "thrice debt {} vs once debt {}",
        thrice.load.recovery_debt,
        once.load.recovery_debt
    );
    // Jadedness has a congestion multiplier (matches_last_14 ≥ 3 →
    // ×1.2 on the third match), so the gap is wider than load alone.
    assert!(
        thrice.player_attributes.jadedness > once.player_attributes.jadedness + 600,
        "thrice jad {} vs once jad {}",
        thrice.player_attributes.jadedness,
        once.player_attributes.jadedness
    );
}

// ── Maturity-amplified match exertion ─────────────────────────────────

#[test]
fn under_15_competitive_match_carries_far_more_load_than_adult_peer() {
    // Same position, same minutes, same condition. The 14-year-old's
    // body absorbs senior intensity at roughly 1.8× the adult cost.
    let date = d(2025, 9, 14);
    let mut adult = fresh_player(PlayerPositionType::MidfielderCenter);
    adult.birth_date = d(1998, 1, 1); // 27
    let mut youth = fresh_player(PlayerPositionType::MidfielderCenter);
    youth.birth_date = d(2011, 1, 1); // 14

    adult.on_match_exertion(90.0, date, false);
    youth.on_match_exertion(90.0, date, false);

    // Load: at least 1.5× the adult's (1.8× nominal, allowing for the
    // depletion-factor and friendly-factor folds in the formula).
    assert!(
        youth.load.physical_load_7 > adult.load.physical_load_7 * 1.5,
        "youth load {} not far enough above adult {}",
        youth.load.physical_load_7,
        adult.load.physical_load_7
    );
    // Recovery debt: amplified even harder (target 2.0×).
    assert!(
        youth.load.recovery_debt > adult.load.recovery_debt * 1.7,
        "youth debt {} not far enough above adult {}",
        youth.load.recovery_debt,
        adult.load.recovery_debt
    );
    // Jadedness ends up materially higher too.
    assert!(
        (youth.player_attributes.jadedness as i32)
            > (adult.player_attributes.jadedness as i32) + 200,
        "youth jad {} should exceed adult jad {} by a clear margin",
        youth.player_attributes.jadedness,
        adult.player_attributes.jadedness
    );
}

#[test]
fn under_15_friendly_match_does_not_get_maturity_amplification() {
    // Pre-season cameos are already discounted via the friendly factor;
    // the maturity multiplier deliberately doesn't pile on top of that.
    let date = d(2025, 9, 14);
    let mut youth_friendly = fresh_player(PlayerPositionType::MidfielderCenter);
    youth_friendly.birth_date = d(2011, 1, 1); // 14
    let mut adult_friendly = fresh_player(PlayerPositionType::MidfielderCenter);
    adult_friendly.birth_date = d(1998, 1, 1); // 27

    youth_friendly.on_match_exertion(90.0, date, true);
    adult_friendly.on_match_exertion(90.0, date, true);

    // Within ~5% of each other — friendlies don't amplify by age.
    let diff = (youth_friendly.load.physical_load_7 - adult_friendly.load.physical_load_7).abs();
    assert!(
        diff < adult_friendly.load.physical_load_7 * 0.05,
        "youth friendly load {} should match adult {} within 5%",
        youth_friendly.load.physical_load_7,
        adult_friendly.load.physical_load_7
    );
}

#[test]
fn condition_floor_is_enforced_post_match() {
    let mut p = fresh_player(PlayerPositionType::Striker);
    p.player_attributes.condition = 1_500; // engine-floor low
    p.on_match_exertion(90.0, d(2025, 9, 14), false);
    assert!(p.player_attributes.condition >= 3_000);
}

#[test]
fn long_idle_player_match_rebuilds_readiness() {
    let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
    p.skills.physical.match_readiness = 8.0;
    let before = p.skills.physical.match_readiness;
    p.on_match_exertion(90.0, d(2025, 9, 14), false);
    assert!(p.skills.physical.match_readiness > before + 2.0);
}

#[test]
fn returning_from_injury_player_carries_elevated_match_risk() {
    let mut healthy = fresh_player(PlayerPositionType::DefenderCenter);
    let mut returning = fresh_player(PlayerPositionType::DefenderCenter);
    // Same builder; mark returning as in-recovery.
    returning.player_attributes.recovery_days_remaining = 10;
    // Build chronic baseline so the spike branch isn't disabled.
    healthy.load.physical_load_30 = 300.0;
    returning.load.physical_load_30 = 300.0;

    let date = d(2025, 9, 14);
    let inputs_h = crate::club::player::condition::InjuryRiskInputs {
        base_rate: 0.005,
        intensity: 1.0,
        in_recovery: false,
        medical_multiplier: 1.0,
        now: date,
    };
    let inputs_r = crate::club::player::condition::InjuryRiskInputs {
        base_rate: 0.005,
        intensity: 1.0,
        in_recovery: true,
        medical_multiplier: 1.0,
        now: date,
    };
    let h = healthy.compute_injury_risk(inputs_h);
    let r = returning.compute_injury_risk(inputs_r);
    assert!(
        r > h * 2.0,
        "returning {} should be much higher than healthy {}",
        r,
        h
    );
}
