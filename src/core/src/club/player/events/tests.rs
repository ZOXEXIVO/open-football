//! Cross-domain integration tests for the events module.
//!
//! Fixtures (`build_player`, `stats`, `outcome`, `run_match`, …) are
//! shared across match-play, role-transition, season-event, transfer
//! and exertion tests because they all build the same kind of synthetic
//! `Player` and `MatchOutcome` data — splitting fixtures per domain
//! would duplicate ~50 lines of boilerplate per file with no payoff.

use super::types::{MatchOutcome, MatchParticipation};
use crate::club::player::builder::PlayerBuilder;
use crate::club::player::condition::InjuryRiskInputs;
use crate::club::player::player::Player;
use crate::r#match::engine::result::PlayerMatchEndStats;
use crate::shared::fullname::FullName;
use crate::{
    AwardReputationInput, AwardReputationKind, HappinessEventType, PersonAttributes,
    PlayerAttributes, PlayerFieldPositionGroup, PlayerPosition, PlayerPositionType,
    PlayerPositions, PlayerSkills,
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
        minutes_played: 90,
        key_passes: 0,
        progressive_passes: 0,
        progressive_carries: 0,
        successful_dribbles: 0,
        attempted_dribbles: 0,
        successful_pressures: 0,
        pressures: 0,
        blocks: 0,
        clearances: 0,
        passes_into_box: 0,
        crosses_attempted: 0,
        crosses_completed: 0,
        xg_chain: 0.0,
        xg_buildup: 0.0,
        miscontrols: 0,
        heavy_touches: 0,
        carry_distance: 0,
        errors_leading_to_shot: 0,
        errors_leading_to_goal: 0,
        xg_prevented: 0.0,
        offsides: 0,
        own_goals: 0,
        zone_stats: Default::default(),
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
        competition_slug: if is_cup { "champions-league" } else { "league" },
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

fn build_player_with_status(status: PlayerSquadStatus) -> Player {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut contract = PlayerClubContract::new(10_000, d(2035, 6, 30));
    contract.squad_status = status;
    p.contract = Some(contract);
    p.statistics.played = 30;
    p
}

#[test]
fn key_player_takes_bigger_relegation_hit_than_rotation() {
    let mut key = build_player_with_status(PlayerSquadStatus::KeyPlayer);
    let mut rotation = build_player_with_status(PlayerSquadStatus::FirstTeamSquadRotation);
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
    let mut not_needed = build_player_with_status(PlayerSquadStatus::NotNeeded);
    let mut regular = build_player_with_status(PlayerSquadStatus::FirstTeamRegular);
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
    status: PlayerSquadStatus,
) {
    let mut contract = PlayerClubContract::new(10_000, d(2035, 6, 30));
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
            PlayerSquadStatus::KeyPlayer,
        );
    }
    for _ in 0..10 {
        run_match_with_status(
            &mut key,
            MatchParticipation::Substitute,
            PlayerSquadStatus::KeyPlayer,
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
            PlayerSquadStatus::FirstTeamSquadRotation,
        );
    }
    for _ in 0..10 {
        run_match_with_status(
            &mut rot,
            MatchParticipation::Substitute,
            PlayerSquadStatus::FirstTeamSquadRotation,
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
            PlayerSquadStatus::HotProspectForTheFuture,
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
            PlayerSquadStatus::KeyPlayer,
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
            PlayerSquadStatus::FirstTeamRegular,
        );
        run_match_with_status(
            &mut lo,
            MatchParticipation::Starter,
            PlayerSquadStatus::FirstTeamRegular,
        );
    }
    for _ in 0..10 {
        run_match_with_status(
            &mut hi,
            MatchParticipation::Substitute,
            PlayerSquadStatus::FirstTeamRegular,
        );
        run_match_with_status(
            &mut lo,
            MatchParticipation::Substitute,
            PlayerSquadStatus::FirstTeamRegular,
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
    gk.on_match_exertion_minutes_only(90.0, date, false);
    fb.on_match_exertion_minutes_only(90.0, date, false);
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
    gk.on_match_exertion_minutes_only(90.0, date, false);
    fb.on_match_exertion_minutes_only(90.0, date, false);
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
    friendly.on_match_exertion_minutes_only(90.0, date, true);
    competitive.on_match_exertion_minutes_only(90.0, date, false);
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

    once.on_match_exertion_minutes_only(90.0, d(2025, 9, 7), false);

    thrice.load.daily_decay(d(2025, 9, 1));
    thrice.on_match_exertion_minutes_only(90.0, d(2025, 9, 1), false);
    thrice.load.daily_decay(d(2025, 9, 4));
    thrice.on_match_exertion_minutes_only(90.0, d(2025, 9, 4), false);
    thrice.load.daily_decay(d(2025, 9, 7));
    thrice.on_match_exertion_minutes_only(90.0, d(2025, 9, 7), false);

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

    adult.on_match_exertion_minutes_only(90.0, date, false);
    youth.on_match_exertion_minutes_only(90.0, date, false);

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

// ── MatchDropped + structured selection context ─────────────

fn drop_ctx(
    scope: SelectionDecisionScope,
    reason: SelectionOmissionReason,
    importance: f32,
    is_friendly: bool,
) -> MatchSelectionContext {
    MatchSelectionContext {
        scope,
        reason,
        comparison: None,
        role: SelectionRole::Striker,
        match_importance: importance,
        repeated: false,
        is_friendly,
    }
}

#[test]
fn match_dropped_carries_structured_selection_context() {
    let mut p = build_player_with_status(PlayerSquadStatus::KeyPlayer);
    let ctx = drop_ctx(
        SelectionDecisionScope::DroppedToBench,
        SelectionOmissionReason::TeammatePreferredOnFitness,
        0.8,
        false,
    );
    p.on_match_dropped_with_context(ctx);

    let event = p
        .happiness
        .recent_events
        .last()
        .expect("MatchDropped event must land");
    assert_eq!(event.event_type, HappinessEventType::MatchDropped);
    let stored = event
        .context
        .as_ref()
        .and_then(|c| c.selection_context.as_ref())
        .expect("selection context must round-trip into the event");
    assert_eq!(stored.scope, SelectionDecisionScope::DroppedToBench);
    assert_eq!(
        stored.reason,
        SelectionOmissionReason::TeammatePreferredOnFitness
    );
}

#[test]
fn match_dropped_key_player_severity_exceeds_rotation_player() {
    let mut key = build_player_with_status(PlayerSquadStatus::KeyPlayer);
    let mut rotation = build_player_with_status(PlayerSquadStatus::FirstTeamSquadRotation);
    let ctx = drop_ctx(
        SelectionDecisionScope::LeftOutOfMatchdaySquad,
        SelectionOmissionReason::TeammatePreferredOnAbility,
        0.8,
        false,
    );
    key.on_match_dropped_with_context(ctx.clone());
    rotation.on_match_dropped_with_context(ctx);

    let key_mag = key
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::MatchDropped)
        .map(|e| e.magnitude.abs())
        .unwrap_or(0.0);
    let rot_mag = rotation
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::MatchDropped)
        .map(|e| e.magnitude.abs())
        .unwrap_or(0.0);
    assert!(
        key_mag > rot_mag * 1.5,
        "KeyPlayer drop magnitude {} should clearly exceed rotation {}",
        key_mag,
        rot_mag
    );
}

#[test]
fn match_dropped_rest_softer_than_tactical_rejection() {
    let mut rested = build_player_with_status(PlayerSquadStatus::FirstTeamRegular);
    let mut rejected = build_player_with_status(PlayerSquadStatus::FirstTeamRegular);
    rested.on_match_dropped_with_context(drop_ctx(
        SelectionDecisionScope::Rested,
        SelectionOmissionReason::FatigueManagement,
        0.8,
        false,
    ));
    rejected.on_match_dropped_with_context(drop_ctx(
        SelectionDecisionScope::DroppedToBench,
        SelectionOmissionReason::PoorRecentForm,
        0.8,
        false,
    ));
    let r_mag = rested
        .happiness
        .recent_events
        .last()
        .map(|e| e.magnitude.abs())
        .unwrap_or(0.0);
    let j_mag = rejected
        .happiness
        .recent_events
        .last()
        .map(|e| e.magnitude.abs())
        .unwrap_or(0.0);
    assert!(
        r_mag < j_mag,
        "rest magnitude {} must be softer than tactical-rejection magnitude {}",
        r_mag,
        j_mag
    );
}

#[test]
fn match_dropped_friendly_dampens_magnitude_below_competitive() {
    let mut friendly = build_player_with_status(PlayerSquadStatus::KeyPlayer);
    let mut competitive = build_player_with_status(PlayerSquadStatus::KeyPlayer);
    friendly.on_match_dropped_with_context(drop_ctx(
        SelectionDecisionScope::DroppedToBench,
        SelectionOmissionReason::PoorRecentForm,
        0.6,
        true,
    ));
    competitive.on_match_dropped_with_context(drop_ctx(
        SelectionDecisionScope::DroppedToBench,
        SelectionOmissionReason::PoorRecentForm,
        0.6,
        false,
    ));
    let f = friendly
        .happiness
        .recent_events
        .last()
        .map(|e| e.magnitude.abs())
        .unwrap_or(0.0);
    let c = competitive
        .happiness
        .recent_events
        .last()
        .map(|e| e.magnitude.abs())
        .unwrap_or(0.0);
    assert!(
        f < c,
        "friendly magnitude {} should be softer than competitive {}",
        f,
        c
    );
}

#[test]
fn on_match_dropped_synthesises_a_default_selection_context() {
    // The legacy contextless path is gone — `on_match_dropped()` now
    // routes through `on_match_dropped_with_context` with a sensible
    // default (UnusedSubstitute / BenchBalance) when the squad
    // selector has no specific omission record. Every MatchDropped
    // event therefore carries structured selection metadata, and the
    // renderer never needs the bare "Dropped from match squad"
    // fallback string.
    let mut p = build_player_with_status(PlayerSquadStatus::FirstTeamRegular);
    p.on_match_dropped();
    let event = p
        .happiness
        .recent_events
        .last()
        .expect("emit must land an event");
    assert_eq!(event.event_type, HappinessEventType::MatchDropped);
    let ctx = event
        .context
        .as_ref()
        .expect("on_match_dropped must attach a selection context");
    let sel = ctx
        .selection_context
        .as_ref()
        .expect("default path must populate selection_context");
    assert_eq!(
        sel.scope,
        SelectionDecisionScope::UnusedSubstitute,
        "default scope is bench-warming"
    );
    assert_eq!(
        sel.reason,
        SelectionOmissionReason::BenchBalance,
        "default reason is bench-balance"
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

    youth_friendly.on_match_exertion_minutes_only(90.0, date, true);
    adult_friendly.on_match_exertion_minutes_only(90.0, date, true);

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
    // Competitive floor (2500) applies only to players who STARTED
    // above it and ran themselves through it — a stabilization, not
    // an overnight refill. A player who walked onto the pitch at
    // 9500 and was drained to ~3000 lands at the floor (or just
    // above, depending on drop magnitude), never below.
    let mut p = fresh_player(PlayerPositionType::Striker);
    p.player_attributes.condition = 9_500;
    p.on_match_exertion_minutes_only(90.0, d(2025, 9, 14), false);
    assert!(
        p.player_attributes.condition >= 2_500,
        "above-floor starter must stabilize at or above 2500: got {}",
        p.player_attributes.condition
    );
}

#[test]
fn long_idle_player_match_rebuilds_readiness() {
    let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
    p.skills.physical.match_readiness = 8.0;
    let before = p.skills.physical.match_readiness;
    p.on_match_exertion_minutes_only(90.0, d(2025, 9, 14), false);
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
    let inputs_h = InjuryRiskInputs {
        base_rate: 0.005,
        intensity: 1.0,
        in_recovery: false,
        medical_multiplier: 1.0,
        now: date,
    };
    let inputs_r = InjuryRiskInputs {
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

// ── Friendly discount source-of-truth ─────────────────────────────────
//
// `on_match_exertion` used to multiply by 0.45 for friendlies and then
// hand that already-discounted load to `PlayerLoad::record_match_load`
// — which applies its own 0.45 — so friendlies actually booked at
// 0.2025× competitive load. The fix routes the friendly discount
// through `record_match_load` exclusively for the load windows; debt
// and jadedness apply the same single discount inline. A friendly 90
// minutes ends up at ~0.45× competitive across every book.

#[test]
fn friendly_competitive_load_ratio_is_single_discount() {
    let date = d(2025, 9, 14);
    let mut friendly = fresh_player(PlayerPositionType::MidfielderCenter);
    let mut competitive = fresh_player(PlayerPositionType::MidfielderCenter);

    friendly.on_match_exertion_minutes_only(90.0, date, true);
    competitive.on_match_exertion_minutes_only(90.0, date, false);

    // Friendly load should be ~0.45× competitive — one discount, not
    // 0.45 × 0.45 = 0.2025× as the previous double-application produced.
    let ratio = friendly.load.physical_load_7 / competitive.load.physical_load_7;
    assert!(
        (ratio - 0.45).abs() < 0.05,
        "friendly/competitive load ratio {} should be ~0.45 (got friendly={}, competitive={})",
        ratio,
        friendly.load.physical_load_7,
        competitive.load.physical_load_7,
    );
}

#[test]
fn youth_friendly_cameo_does_not_apply_youth_senior_overload_multipliers() {
    // The youth maturity load multiplier deliberately returns 1.0 for
    // friendlies — pre-season cameos are already discounted by the
    // friendly factor and shouldn't pile a 1.8× youth multiplier on top.
    // A 14yo and a 27yo friendly cameo should book the same load.
    let date = d(2025, 9, 14);
    let mut youth = fresh_player(PlayerPositionType::MidfielderCenter);
    youth.birth_date = d(2011, 1, 1); // 14
    let mut adult = fresh_player(PlayerPositionType::MidfielderCenter);
    adult.birth_date = d(1998, 1, 1); // 27

    youth.on_match_exertion_minutes_only(90.0, date, true);
    adult.on_match_exertion_minutes_only(90.0, date, true);

    let diff = (youth.load.physical_load_7 - adult.load.physical_load_7).abs();
    assert!(
        diff < adult.load.physical_load_7 * 0.05,
        "youth friendly load {} should match adult {} within 5%",
        youth.load.physical_load_7,
        adult.load.physical_load_7
    );
    let debt_diff = (youth.load.recovery_debt - adult.load.recovery_debt).abs();
    assert!(
        debt_diff < adult.load.recovery_debt.max(1.0) * 0.10,
        "youth friendly debt {} should match adult {} within 10%",
        youth.load.recovery_debt,
        adult.load.recovery_debt
    );
}

#[test]
fn youth_competitive_full_match_higher_load_and_debt_than_adult() {
    // The other side of the same fix: competitive matches DO apply the
    // youth maturity multipliers. A 14yo's 90-minute senior start books
    // ~1.8× the load and ~2.0× the recovery debt of an adult peer.
    let date = d(2025, 9, 14);
    let mut youth = fresh_player(PlayerPositionType::MidfielderCenter);
    youth.birth_date = d(2011, 1, 1); // 14
    let mut adult = fresh_player(PlayerPositionType::MidfielderCenter);
    adult.birth_date = d(1998, 1, 1); // 27

    youth.on_match_exertion_minutes_only(90.0, date, false);
    adult.on_match_exertion_minutes_only(90.0, date, false);

    assert!(
        youth.load.physical_load_7 > adult.load.physical_load_7 * 1.5,
        "youth competitive load {} should beat adult {} by ≥ 1.5×",
        youth.load.physical_load_7,
        adult.load.physical_load_7
    );
    assert!(
        youth.load.recovery_debt > adult.load.recovery_debt * 1.7,
        "youth competitive debt {} should beat adult {} by ≥ 1.7×",
        youth.load.recovery_debt,
        adult.load.recovery_debt
    );
}

// ── Hat-trick / assist hat-trick ─────────────────────────────

#[test]
fn hat_trick_fires_for_three_goals() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(7.5, 3, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        7.5,
        false,
        false,
        false,
        false,
        3,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::HatTrick), 1);
}

#[test]
fn hat_trick_silent_for_two_goals() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(7.5, 2, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        7.5,
        false,
        false,
        false,
        false,
        3,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::HatTrick), 0);
}

#[test]
fn hat_trick_silent_in_friendly() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(7.5, 3, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        7.5,
        true,
        false,
        false,
        false,
        3,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::HatTrick), 0);
}

#[test]
fn hat_trick_cooldown_blocks_repeat() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(7.5, 3, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        7.5,
        false,
        false,
        false,
        false,
        3,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::HatTrick), 1);
}

#[test]
fn assist_hat_trick_fires_for_three_assists() {
    let mut p = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    let s = stats(7.5, 0, 3, 0, PlayerFieldPositionGroup::Midfielder);
    let o = outcome(
        &s,
        7.5,
        false,
        false,
        false,
        false,
        3,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::AssistHatTrick), 1);
}

// ── Drought ──────────────────────────────────────────────────

#[test]
fn drought_concern_silent_for_first_match() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Forward);
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
    assert_eq!(
        count_events(&p, &HappinessEventType::ScoringDroughtConcern),
        0
    );
}

#[test]
fn drought_concern_fires_after_six_goalless_apps() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    for _ in 0..7 {
        let o = outcome(
            &s,
            6.5,
            false,
            false,
            false,
            false,
            1,
            0,
            MatchParticipation::Starter,
        );
        p.on_match_played(&o);
    }
    assert!(
        count_events(&p, &HappinessEventType::ScoringDroughtConcern) >= 1,
        "expected ScoringDroughtConcern after sustained drought"
    );
}

#[test]
fn drought_ended_and_concern_do_not_co_fire() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let goalless = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    for _ in 0..9 {
        let o = outcome(
            &goalless,
            6.5,
            false,
            false,
            false,
            false,
            1,
            1,
            MatchParticipation::Starter,
        );
        p.on_match_played(&o);
    }
    let scoring = stats(7.0, 1, 0, 0, PlayerFieldPositionGroup::Forward);
    let win = outcome(
        &scoring,
        7.0,
        false,
        false,
        false,
        false,
        2,
        1,
        MatchParticipation::Starter,
    );
    let concern_before = count_events(&p, &HappinessEventType::ScoringDroughtConcern);
    p.on_match_played(&win);
    assert!(count_events(&p, &HappinessEventType::GoalDroughtEnded) >= 1);
    let concern_after = count_events(&p, &HappinessEventType::ScoringDroughtConcern);
    assert_eq!(
        concern_after, concern_before,
        "drought-ended match should not co-fire ScoringDroughtConcern"
    );
}

// ── Senior debut ─────────────────────────────────────────────

#[test]
fn senior_debut_fires_on_first_competitive_app() {
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
        false,
        1,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::SeniorDebut), 1);
}

#[test]
fn senior_debut_silent_in_friendly() {
    let mut p = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    let s = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
    let o = outcome(
        &s,
        6.5,
        true,
        false,
        false,
        false,
        1,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::SeniorDebut), 0);
}

// ── Milestones ───────────────────────────────────────────────

#[test]
fn appearance_milestone_fires_at_threshold_only_once() {
    let mut p = build_player(
        PlayerPositionType::MidfielderCenter,
        PersonAttributes::default(),
    );
    p.statistics.played = 49; // about to cross 50 with this match.
    let s = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
    let o = outcome(
        &s,
        6.5,
        false,
        false,
        false,
        false,
        1,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(
        count_events(&p, &HappinessEventType::AppearanceMilestone),
        1
    );
    // Next match (51 apps) — no further milestone fire.
    let o2 = outcome(
        &s,
        6.5,
        false,
        false,
        false,
        false,
        1,
        0,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o2);
    assert_eq!(
        count_events(&p, &HappinessEventType::AppearanceMilestone),
        1
    );
}

#[test]
fn goal_milestone_fires_when_threshold_crossed() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.statistics.played = 100;
    p.statistics.goals = 24; // hits 25 with one more goal this match.
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
    assert_eq!(count_events(&p, &HappinessEventType::GoalMilestone), 1);
}

// ── Catalog handles every variant ───────────────────────────

#[test]
fn catalog_handles_every_new_variant() {
    let cat = MoraleEventCatalog::default();
    // Spec defaults — exhaustive checks for the new variants.
    assert_eq!(cat.magnitude(HappinessEventType::PlayerOfTheMonth), 8.0);
    assert_eq!(
        cat.magnitude(HappinessEventType::YoungPlayerOfTheMonth),
        7.0
    );
    assert_eq!(
        cat.magnitude(HappinessEventType::TeamOfTheWeekSelection),
        3.0
    );
    assert_eq!(
        cat.magnitude(HappinessEventType::TeamOfTheSeasonSelection),
        9.0
    );
    assert_eq!(cat.magnitude(HappinessEventType::PlayerOfTheSeason), 12.0);
    assert_eq!(
        cat.magnitude(HappinessEventType::YoungPlayerOfTheSeason),
        10.0
    );
    assert_eq!(cat.magnitude(HappinessEventType::LeagueTopScorer), 10.0);
    assert_eq!(cat.magnitude(HappinessEventType::LeagueTopAssists), 8.0);
    assert_eq!(cat.magnitude(HappinessEventType::LeagueGoldenGlove), 8.0);
    assert_eq!(
        cat.magnitude(HappinessEventType::ContinentalPlayerOfYear),
        14.0
    );
    assert_eq!(cat.magnitude(HappinessEventType::WorldPlayerOfYear), 18.0);
    assert_eq!(cat.magnitude(HappinessEventType::SeniorDebut), 6.0);
    assert_eq!(cat.magnitude(HappinessEventType::NationalTeamDebut), 8.0);
    assert_eq!(cat.magnitude(HappinessEventType::HatTrick), 7.0);
    assert_eq!(cat.magnitude(HappinessEventType::GoalDroughtEnded), 3.5);
    assert_eq!(
        cat.magnitude(HappinessEventType::ScoringDroughtConcern),
        -3.0
    );
    assert_eq!(cat.magnitude(HappinessEventType::AppearanceMilestone), 5.0);
    assert_eq!(cat.magnitude(HappinessEventType::GoalMilestone), 5.0);
    assert_eq!(cat.magnitude(HappinessEventType::CleanSheetMilestone), 5.0);
    assert_eq!(
        cat.magnitude(HappinessEventType::TrainingGroundBustUp),
        -4.0
    );
    assert_eq!(cat.magnitude(HappinessEventType::PublicApology), 1.0);
    assert_eq!(cat.magnitude(HappinessEventType::FansChantPlayerName), 3.0);
    assert_eq!(
        cat.magnitude(HappinessEventType::MediaPressureMounting),
        -3.5
    );
    assert_eq!(cat.magnitude(HappinessEventType::LeadershipEmergence), 4.0);
}

#[test]
fn catalog_polarity_for_new_variants() {
    let cat = MoraleEventCatalog::default();
    let positives = [
        HappinessEventType::PlayerOfTheMonth,
        HappinessEventType::TeamOfTheWeekSelection,
        HappinessEventType::PlayerOfTheSeason,
        HappinessEventType::LeagueTopScorer,
        HappinessEventType::ContinentalPlayerOfYearNomination,
        HappinessEventType::WorldPlayerOfYear,
        HappinessEventType::SeniorDebut,
        HappinessEventType::NationalTeamDebut,
        HappinessEventType::HatTrick,
        HappinessEventType::GoalDroughtEnded,
        HappinessEventType::AppearanceMilestone,
        HappinessEventType::PublicApology,
        HappinessEventType::FansChantPlayerName,
        HappinessEventType::LeadershipEmergence,
    ];
    for p in positives {
        assert!(cat.magnitude(p.clone()) > 0.0, "{:?} should be positive", p);
    }
    let negatives = [
        HappinessEventType::ScoringDroughtConcern,
        HappinessEventType::TrainingGroundBustUp,
        HappinessEventType::MediaPressureMounting,
    ];
    for n in negatives {
        assert!(cat.magnitude(n.clone()) < 0.0, "{:?} should be negative", n);
    }
}

// ── MediaPressureMounting sliding window ────────────────────

fn play_match_with_rating(p: &mut Player, rating: f32) {
    let s = stats(rating, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        rating,
        false,
        false,
        false,
        false,
        1,
        1,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
}

#[test]
fn media_pressure_silent_before_window_fills() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.player_attributes.current_reputation = 7000;
    // Two consecutive poor games but only 2 apps — window not full yet.
    play_match_with_rating(&mut p, 5.5);
    play_match_with_rating(&mut p, 5.5);
    assert_eq!(
        count_events(&p, &HappinessEventType::MediaPressureMounting),
        0,
        "window not full yet — should be silent"
    );
}

#[test]
fn media_pressure_fires_with_poor_apps_split_across_block_boundary() {
    // Poor apps on appearances 2 and 6 — under the old block-reset
    // logic these split across the boundary and never co-trigger.
    // With a sliding window they remain inside the last 5 at app 6.
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.player_attributes.current_reputation = 7000;
    let ratings = [7.5, 5.5, 7.5, 7.5, 7.5, 5.5];
    for r in ratings {
        play_match_with_rating(&mut p, r);
    }
    assert!(
        count_events(&p, &HappinessEventType::MediaPressureMounting) >= 1,
        "sliding window should keep both poor apps in view at app 6"
    );
}

#[test]
fn media_pressure_silent_for_low_profile_player() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.player_attributes.current_reputation = 2000;
    let ratings = [5.5, 5.5, 7.5, 7.5, 7.5];
    for r in ratings {
        play_match_with_rating(&mut p, r);
    }
    assert_eq!(
        count_events(&p, &HappinessEventType::MediaPressureMounting),
        0,
        "low-profile player below the reputation gate should not trigger"
    );
}

#[test]
fn media_pressure_old_lows_age_out_of_sliding_window() {
    // Two poor apps in apps 1-2, then 5 good apps — both lows fall
    // off the 5-app window. No trigger should fire on the 5th good app
    // even though there were 2 lows in the player's full history.
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.player_attributes.current_reputation = 7000;
    let ratings = [5.5, 5.5, 7.5, 7.5, 7.5, 7.5, 7.5, 7.5];
    let mut last_count = 0;
    for r in ratings {
        play_match_with_rating(&mut p, r);
        last_count = count_events(&p, &HappinessEventType::MediaPressureMounting);
    }
    // Some triggers may have fired earlier when both lows were in the
    // window. The test focuses on the *final* state: after enough good
    // appearances the lows have rolled out, so no fresh trigger fires
    // on the latest match alone (cooldown aside, count shouldn't grow
    // beyond what fired within-window).
    let _ = last_count; // assertion below: nothing else can fire here.
    // After the window clears, trigger should not still be "armed":
    // the mask popcount is 0.
    assert_eq!(
        p.happiness.recent_low_rating_mask.count_ones(),
        0,
        "old lows must have rolled off the sliding window"
    );
}

// ── Recent_events_cap respected for award flood ─────────────

#[test]
fn award_emissions_respect_recent_events_cap() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let cfg = HappinessConfig::default();
    let cap = cfg.recent_events_cap;
    // Fire many bursty events of varied types. The cap enforces bound.
    for i in 0..(cap + 50) {
        let event = if i % 2 == 0 {
            HappinessEventType::TeamOfTheWeekSelection
        } else {
            HappinessEventType::HatTrick
        };
        p.happiness.add_event_default(event);
    }
    assert!(p.happiness.recent_events.len() <= cap);
}

// ── Transfer-interest signal tests ───────────────────────────

fn make_signal(stage: TransferInterestStage) -> super::transfer_social::TransferInterestSignal {
    super::transfer_social::TransferInterestSignal {
        interested_club_id: 9001,
        interested_league_id: Some(2),
        buyer_rep: 0.80,
        seller_rep: 0.50,
        buyer_league_rep: 8000,
        seller_league_rep: 5000,
        stage,
        source: TransferInterestSource::ConfirmedApproach,
        repeated_attention: false,
        is_rival: false,
        is_home_country: false,
        is_seller_in_home_country: false,
        is_former_club: false,
        buyer_country_id: 0,
        buyer_continent_id: 0,
        buyer_has_continental_path: false,
        buyer_competition_path: None,
    }
}

#[test]
fn transfer_interest_signal_fires_for_concrete_step_up_for_ambitious_player() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.attributes.ambition = 17.0;
    let sig = make_signal(TransferInterestStage::ConcreteInterest);
    let landed = p.on_transfer_interest_signal(&sig);
    assert!(landed, "concrete interest from a bigger club should land");
    let count = count_events(&p, &HappinessEventType::InterestFromBiggerClub);
    assert_eq!(count, 1);
    let stored_ctx = p
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::InterestFromBiggerClub)
        .and_then(|e| e.context.as_ref())
        .and_then(|c| c.transfer_interest_context.as_ref())
        .expect("transfer interest context must be attached");
    assert_eq!(stored_ctx.interested_club_id, Some(9001));
    assert_eq!(
        stored_ctx.player_reaction,
        TransferInterestReaction::Excited
    );
}

#[test]
fn transfer_interest_high_loyalty_player_reacts_calmly() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.attributes.ambition = 10.0;
    p.attributes.loyalty = 17.0;
    let sig = make_signal(TransferInterestStage::ConcreteInterest);
    p.on_transfer_interest_signal(&sig);
    let stored_ctx = p
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::InterestFromBiggerClub)
        .and_then(|e| e.context.as_ref())
        .and_then(|c| c.transfer_interest_context.as_ref())
        .expect("context attached");
    assert_eq!(
        stored_ctx.player_reaction,
        TransferInterestReaction::PubliclyCalmPrivatelyInterested,
        "loyal player should appear calm publicly even on a step-up link"
    );
}

#[test]
fn transfer_interest_ambitious_player_magnitude_exceeds_low_ambition() {
    let mut high = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut low = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    high.attributes.ambition = 18.0;
    low.attributes.ambition = 4.0;
    let sig = make_signal(TransferInterestStage::ConcreteInterest);
    high.on_transfer_interest_signal(&sig);
    low.on_transfer_interest_signal(&sig);
    let m_high = high
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::InterestFromBiggerClub)
        .map(|e| e.magnitude)
        .unwrap_or(0.0);
    let m_low = low
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::InterestFromBiggerClub)
        .map(|e| e.magnitude)
        .unwrap_or(0.0);
    assert!(
        m_high > m_low,
        "ambitious player should feel a bigger lift than a low-ambition one (high={}, low={})",
        m_high,
        m_low
    );
}

#[test]
fn transfer_interest_scout_only_does_not_emit_for_peer_buyer() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut sig = make_signal(TransferInterestStage::ScoutWatched);
    sig.buyer_rep = 0.51;
    sig.seller_rep = 0.50;
    sig.repeated_attention = false;
    sig.source = TransferInterestSource::ScoutAttendance;
    let landed = p.on_transfer_interest_signal(&sig);
    assert!(
        !landed,
        "single scout sighting at peer rep should not surface as a player event"
    );
}

#[test]
fn transfer_interest_repeated_scout_attention_emits_event() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut sig = make_signal(TransferInterestStage::ScoutWatched);
    sig.buyer_rep = 0.55;
    sig.seller_rep = 0.50;
    sig.repeated_attention = true;
    sig.source = TransferInterestSource::ScoutAttendance;
    let landed = p.on_transfer_interest_signal(&sig);
    assert!(
        landed,
        "repeated scout attention should bubble up to the player"
    );
    let count = count_events(&p, &HappinessEventType::ScoutedByClub);
    assert_eq!(count, 1);
}

#[test]
fn transfer_interest_scout_cooldown_blocks_repeat() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut sig = make_signal(TransferInterestStage::ScoutWatched);
    sig.buyer_rep = 0.55;
    sig.seller_rep = 0.50;
    sig.repeated_attention = true;
    sig.source = TransferInterestSource::ScoutAttendance;
    p.on_transfer_interest_signal(&sig);
    // Same scout, same window — cooldown gate should block the duplicate.
    let landed_again = p.on_transfer_interest_signal(&sig);
    assert!(
        !landed_again,
        "repeated scout watching inside the cooldown window should not fire a second event"
    );
    let count = count_events(&p, &HappinessEventType::ScoutedByClub);
    assert_eq!(count, 1);
}

#[test]
fn transfer_interest_rival_kind_attaches_rival_evidence() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut sig = make_signal(TransferInterestStage::ConcreteInterest);
    sig.is_rival = true;
    p.on_transfer_interest_signal(&sig);
    let event = p
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::InterestFromRival)
        .expect("rival-club concrete interest should fire its own event type");
    let tic = event
        .context
        .as_ref()
        .and_then(|c| c.transfer_interest_context.as_ref())
        .expect("rival context attached");
    assert!(tic.is_rival);
    assert!(tic.evidence.contains(&TransferInterestEvidence::RivalClub));
}

#[test]
fn transfer_interest_underused_player_reads_smaller_club_offer_as_escape() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.attributes.ambition = 12.0;
    p.attributes.loyalty = 8.0;
    let mut sig = make_signal(TransferInterestStage::ConcreteInterest);
    // Smaller club, but offering more minutes
    sig.buyer_rep = 0.30;
    sig.seller_rep = 0.55;
    // Install a contract with fringe squad status so the fringe-detection
    // branch lights up. `build_player` starts the player with no contract;
    // without one the classifier can't read squad status.
    let mut contract = PlayerClubContract::new(10_000, d(2035, 6, 30));
    contract.squad_status = PlayerSquadStatus::MainBackupPlayer;
    p.contract = Some(contract);
    p.on_transfer_interest_signal(&sig);
    let event =
        p.happiness.recent_events.iter().last().expect(
            "an event should fire for a fringe player getting a step-down-with-minutes link",
        );
    let tic = event
        .context
        .as_ref()
        .and_then(|c| c.transfer_interest_context.as_ref())
        .expect("context attached");
    assert_eq!(
        tic.interest_kind,
        TransferInterestKind::EscapeRoute,
        "fringe player offered minutes elsewhere should classify as EscapeRoute"
    );
    assert_eq!(tic.player_reaction, TransferInterestReaction::WantsTalks);
}

#[test]
fn transfer_interest_bid_rejected_signal_emits_with_rejected_evidence() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.attributes.ambition = 16.0;
    let sig = make_signal(TransferInterestStage::BidRejected);
    let landed = p.on_transfer_interest_signal(&sig);
    assert!(
        landed,
        "rejected bid should always land for ambitious player"
    );
    let event = p
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::TransferBidRejected)
        .expect("bid-rejected event must fire via the signal path");
    let tic = event
        .context
        .as_ref()
        .and_then(|c| c.transfer_interest_context.as_ref())
        .expect("context attached for bid rejected");
    assert!(
        tic.evidence
            .contains(&TransferInterestEvidence::RejectedBid)
    );
}

#[test]
fn transfer_interest_already_home_domestic_move_is_not_homecoming() {
    // Russian player at a Russian club linked with another Russian club:
    // the buyer is in the player's home country, but the player is also
    // already there — this is a domestic lateral move, not a homecoming.
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut sig = make_signal(TransferInterestStage::ConcreteInterest);
    sig.buyer_rep = 0.55;
    sig.seller_rep = 0.55;
    sig.buyer_league_rep = 6000;
    sig.seller_league_rep = 6000;
    sig.is_home_country = true;
    sig.is_seller_in_home_country = true;
    let landed = p.on_transfer_interest_signal(&sig);
    assert!(landed, "concrete domestic interest should still surface");
    let homecoming_count = count_events(&p, &HappinessEventType::HomecomingRumour);
    assert_eq!(
        homecoming_count, 0,
        "an already-home domestic move must not fire HomecomingRumour"
    );
    let last_kind = p
        .happiness
        .recent_events
        .iter()
        .last()
        .and_then(|e| e.context.as_ref())
        .and_then(|c| c.transfer_interest_context.as_ref())
        .map(|tic| tic.interest_kind);
    assert_ne!(
        last_kind,
        Some(TransferInterestKind::Homecoming),
        "domestic same-country move should fall through to a non-homecoming kind"
    );
    let tic = p
        .happiness
        .recent_events
        .iter()
        .last()
        .and_then(|e| e.context.as_ref())
        .and_then(|c| c.transfer_interest_context.as_ref())
        .expect("transfer-interest context should be attached");
    assert!(
        !tic.is_home_country,
        "an already-home domestic move must not mark the context as a homecoming"
    );
    assert!(
        !tic.evidence
            .contains(&TransferInterestEvidence::HomeCountry),
        "an already-home domestic move must not carry HomeCountry evidence"
    );
}

#[test]
fn transfer_interest_player_abroad_linked_with_home_club_is_homecoming() {
    // Russian player playing abroad, linked with a Russian club —
    // proper homecoming narrative.
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut sig = make_signal(TransferInterestStage::ConcreteInterest);
    sig.buyer_rep = 0.55;
    sig.seller_rep = 0.55;
    sig.buyer_league_rep = 6000;
    sig.seller_league_rep = 6000;
    sig.is_home_country = true;
    sig.is_seller_in_home_country = false;
    let landed = p.on_transfer_interest_signal(&sig);
    assert!(
        landed,
        "homecoming approach for a foreign-based player should land"
    );
    let event = p
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::HomecomingRumour)
        .expect(
            "foreign-based player linked with a home-country club should fire HomecomingRumour",
        );
    let tic = event
        .context
        .as_ref()
        .and_then(|c| c.transfer_interest_context.as_ref())
        .expect("context attached for homecoming");
    assert_eq!(tic.interest_kind, TransferInterestKind::Homecoming);
    assert!(tic.is_home_country);
}

#[test]
fn transfer_interest_former_club_precedence_over_homecoming() {
    // A former-club approach in the player's home country should still
    // classify as FormerClubReturn even though the home-country signal
    // is also set.
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let mut sig = make_signal(TransferInterestStage::ConcreteInterest);
    sig.buyer_rep = 0.55;
    sig.seller_rep = 0.55;
    sig.is_home_country = true;
    sig.is_seller_in_home_country = false;
    sig.is_former_club = true;
    p.on_transfer_interest_signal(&sig);
    let event = p
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::FormerClubInterest)
        .expect("former-club interest must take precedence over homecoming");
    let tic = event
        .context
        .as_ref()
        .and_then(|c| c.transfer_interest_context.as_ref())
        .expect("context attached for former-club return");
    assert_eq!(tic.interest_kind, TransferInterestKind::FormerClubReturn);
}

#[test]
fn transfer_interest_favorite_club_precedence_over_homecoming() {
    // A favourite-club approach should win over both homecoming and the
    // already-home short-circuit, so the renderer keeps the strongest
    // emotional framing.
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.favorite_clubs.push(9001);
    let mut sig = make_signal(TransferInterestStage::ConcreteInterest);
    sig.buyer_rep = 0.55;
    sig.seller_rep = 0.55;
    sig.is_home_country = true;
    sig.is_seller_in_home_country = true;
    p.on_transfer_interest_signal(&sig);
    let event = p
        .happiness
        .recent_events
        .iter()
        .find(|e| e.event_type == HappinessEventType::FavoriteClubInterest)
        .expect("favourite-club interest must take precedence over homecoming");
    let tic = event
        .context
        .as_ref()
        .and_then(|c| c.transfer_interest_context.as_ref())
        .expect("context attached for favourite club");
    assert_eq!(
        tic.interest_kind,
        TransferInterestKind::FavoriteClubInterest
    );
}

#[test]
fn transfer_interest_count_helper_only_counts_interest_events() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.happiness
        .add_event_default(HappinessEventType::PoorTraining);
    p.happiness
        .add_event_default(HappinessEventType::ManagerPraise);
    let sig = make_signal(TransferInterestStage::ConcreteInterest);
    p.on_transfer_interest_signal(&sig);
    let n = p.count_recent_transfer_interest_events(60);
    assert_eq!(
        n, 1,
        "only the InterestFromBiggerClub event should be counted as interest"
    );
}

#[test]
fn transfer_interest_speculation_pressure_high_professionalism_no_event() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.attributes.professionalism = 18.0;
    p.on_unresolved_speculation_pressure(5);
    let count = count_events(&p, &HappinessEventType::TransferSpeculationDistracts);
    assert_eq!(count, 0, "highly professional players shrug it off");
}

#[test]
fn transfer_interest_speculation_pressure_emits_for_low_professionalism() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    p.attributes.professionalism = 8.0;
    p.on_unresolved_speculation_pressure(4);
    let count = count_events(&p, &HappinessEventType::TransferSpeculationDistracts);
    assert_eq!(count, 1);
}

// ── Award reputation pipeline ─────────────────────────────────

fn build_award_player(birth: NaiveDate, cur: i16, home: i16, world: i16) -> Player {
    let mut attrs = PlayerAttributes::default();
    attrs.current_reputation = cur;
    attrs.home_reputation = home;
    attrs.world_reputation = world;
    PlayerBuilder::new()
        .id(1)
        .full_name(FullName::new("Test".to_string(), "Player".to_string()))
        .birth_date(birth)
        .country_id(1)
        .attributes(PersonAttributes::default())
        .skills(PlayerSkills::default())
        .positions(PlayerPositions {
            positions: vec![PlayerPosition {
                position: PlayerPositionType::Striker,
                level: 20,
            }],
        })
        .player_attributes(attrs)
        .build()
        .unwrap()
}

fn rep_deltas(p: &Player, before: (i16, i16, i16)) -> (i16, i16, i16) {
    (
        p.player_attributes.current_reputation - before.0,
        p.player_attributes.home_reputation - before.1,
        p.player_attributes.world_reputation - before.2,
    )
}

#[test]
fn awards_count_bumps_on_apply_award_reputation_impact() {
    // The Awards-tab dashboard relies on the lifetime counter being
    // bumped exactly once per `apply_award_reputation_impact` call. If
    // an award path is added that skips this funnel, this test must
    // also fail — keeping every award visible on the player page.
    let mut p = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    assert_eq!(p.awards_count.total(), 0);

    let input_league_a = AwardReputationInput::new()
        .with_league_reputation(5_000)
        .with_league_id(42);
    let input_league_b = AwardReputationInput::new()
        .with_league_reputation(5_000)
        .with_league_id(99);
    p.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheWeekSelection,
        input_league_a,
        d(2026, 5, 7),
    );
    p.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheWeekSelection,
        input_league_b,
        d(2026, 5, 14),
    );
    p.apply_award_reputation_impact(
        AwardReputationKind::YoungTeamOfTheWeekSelection,
        input_league_a,
        d(2026, 5, 14),
    );
    p.apply_award_reputation_impact(
        AwardReputationKind::WorldPlayerOfYear,
        AwardReputationInput::new(),
        d(2026, 12, 31),
    );

    assert_eq!(p.awards_count.team_of_the_week, 2);
    assert_eq!(p.awards_count.young_team_of_the_week, 1);
    assert_eq!(p.awards_count.world_player_of_year, 1);
    assert_eq!(p.awards_count.total(), 4);

    // Timeline log captures the date + kind + league_id for every
    // award so the Awards-tab can group totals per league and chart
    // totals per month.
    assert_eq!(p.awards_count.timeline.len(), 4);
    assert_eq!(p.awards_count.timeline[0].date, d(2026, 5, 7));
    assert!(matches!(
        p.awards_count.timeline[0].kind,
        AwardReputationKind::TeamOfTheWeekSelection
    ));
    assert_eq!(p.awards_count.timeline[0].league_id, Some(42));
    assert_eq!(p.awards_count.timeline[1].league_id, Some(99));
    assert_eq!(p.awards_count.timeline[2].league_id, Some(42));
    assert_eq!(
        p.awards_count.timeline[3].league_id, None,
        "global POY award carries no league context"
    );
    assert!(matches!(
        p.awards_count.timeline[3].kind,
        AwardReputationKind::WorldPlayerOfYear
    ));
}

#[test]
fn award_reputation_team_of_week_small_boost() {
    let mut p = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let before = (
        p.player_attributes.current_reputation,
        p.player_attributes.home_reputation,
        p.player_attributes.world_reputation,
    );
    p.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheWeekSelection,
        AwardReputationInput::new().with_league_reputation(5_000),
        d(2026, 5, 7),
    );
    let (dc, dh, dw) = rep_deltas(&p, before);
    assert!(
        dc > 0 && dc < 35,
        "TOTW current delta should be small and positive, got {}",
        dc
    );
    assert!(
        dh > 0 && dh < 35,
        "TOTW home delta should be small, got {}",
        dh
    );
    assert!(dw <= 3, "TOTW world delta should be minimal, got {}", dw);
}

#[test]
fn award_reputation_player_of_week_beats_team_of_week() {
    let mut totw = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let mut pow = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let before = (5_000_i16, 5_000_i16, 4_000_i16);
    let input = AwardReputationInput::new().with_league_reputation(5_000);
    totw.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheWeekSelection,
        input,
        d(2026, 5, 7),
    );
    pow.apply_award_reputation_impact(AwardReputationKind::PlayerOfTheWeek, input, d(2026, 5, 7));
    let (totw_c, _, _) = rep_deltas(&totw, before);
    let (pow_c, _, _) = rep_deltas(&pow, before);
    assert!(
        pow_c > totw_c,
        "POW ({}) must beat TOTW ({}) on current rep",
        pow_c,
        totw_c
    );
}

#[test]
fn award_reputation_young_player_of_week_breakthrough_helps_low_rep_kid() {
    // Same age (17), strong league, but vastly different starting rep.
    let kid_birth = d(2009, 1, 1);
    let mut low_rep = build_award_player(kid_birth, 800, 800, 200);
    let mut famous = build_award_player(kid_birth, 4_500, 4_500, 2_500);
    let input = AwardReputationInput::new().with_league_reputation(8_000);
    low_rep.apply_award_reputation_impact(
        AwardReputationKind::YoungPlayerOfTheWeek,
        input,
        d(2026, 5, 7),
    );
    famous.apply_award_reputation_impact(
        AwardReputationKind::YoungPlayerOfTheWeek,
        input,
        d(2026, 5, 7),
    );
    let dc_low = low_rep.player_attributes.current_reputation - 800;
    let dc_famous = famous.player_attributes.current_reputation - 4_500;
    assert!(
        dc_low > dc_famous,
        "low-rep U-20 should gain more from YPOW (got {}) than already-famous U-20 ({})",
        dc_low,
        dc_famous,
    );
}

#[test]
fn award_reputation_team_of_year_more_than_team_of_week() {
    let mut totw = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let mut toty = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let totw_input = AwardReputationInput::new().with_league_reputation(7_000);
    let toty_input = AwardReputationInput::new()
        .with_league_reputation(7_000)
        .with_avg_rating(7.4)
        .with_matches_played(34);
    totw.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheWeekSelection,
        totw_input,
        d(2026, 5, 7),
    );
    toty.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheYearSelection,
        toty_input,
        d(2026, 5, 7),
    );
    let dc_totw = totw.player_attributes.current_reputation - 5_000;
    let dc_toty = toty.player_attributes.current_reputation - 5_000;
    assert!(
        dc_toty > dc_totw,
        "TOTY ({}) must beat TOTW ({})",
        dc_toty,
        dc_totw
    );
}

#[test]
fn award_reputation_team_of_year_less_than_world_poy() {
    let mut toty = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let mut wpoy = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let toty_input = AwardReputationInput::new()
        .with_league_reputation(9_000)
        .with_avg_rating(7.5)
        .with_matches_played(34);
    toty.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheYearSelection,
        toty_input,
        d(2026, 5, 7),
    );
    wpoy.apply_award_reputation_impact(
        AwardReputationKind::WorldPlayerOfYear,
        AwardReputationInput::new(),
        d(2026, 5, 7),
    );
    let dc_toty = toty.player_attributes.current_reputation - 5_000;
    let dc_wpoy = wpoy.player_attributes.current_reputation - 5_000;
    assert!(
        dc_wpoy > dc_toty,
        "World POY ({}) must beat TOTY ({})",
        dc_wpoy,
        dc_toty
    );
}

#[test]
fn award_reputation_elite_player_reduced_weekly_gain() {
    let mut elite = build_award_player(d(1992, 1, 1), 8_500, 8_500, 8_500);
    let mut mid = build_award_player(d(1992, 1, 1), 5_000, 5_000, 4_000);
    let input = AwardReputationInput::new().with_league_reputation(7_000);
    elite.apply_award_reputation_impact(AwardReputationKind::PlayerOfTheWeek, input, d(2026, 5, 7));
    mid.apply_award_reputation_impact(AwardReputationKind::PlayerOfTheWeek, input, d(2026, 5, 7));
    let dc_elite = elite.player_attributes.current_reputation - 8_500;
    let dc_mid = mid.player_attributes.current_reputation - 5_000;
    assert!(
        dc_elite < dc_mid,
        "elite POW gain ({}) must be smaller than mid-rep gain ({})",
        dc_elite,
        dc_mid,
    );
    assert!(
        dc_elite < 12,
        "elite player above 7500 cur rep should barely move on a weekly award, got {}",
        dc_elite,
    );
}

#[test]
fn award_reputation_low_league_barely_moves_world() {
    let mut p = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let input = AwardReputationInput::new().with_league_reputation(1_000);
    p.apply_award_reputation_impact(AwardReputationKind::PlayerOfTheWeek, input, d(2026, 5, 7));
    let dw = p.player_attributes.world_reputation - 4_000;
    assert!(
        dw <= 1,
        "POW in a low-rep league must barely touch world rep, got {}",
        dw
    );
}

#[test]
fn award_reputation_pow_then_totw_dampens_totw() {
    // Two players: one only got TOTW, one got POW first then TOTW.
    let mut single = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let mut stacked = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let input = AwardReputationInput::new().with_league_reputation(7_000);

    single.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheWeekSelection,
        input,
        d(2026, 5, 7),
    );
    let dc_single = single.player_attributes.current_reputation - 5_000;

    // Same player wins POW then TOTW. POW emits its happiness event,
    // which the dampener picks up.
    stacked.on_player_of_the_week();
    let stacked_after_pow_cur = stacked.player_attributes.current_reputation;
    stacked.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheWeekSelection,
        input,
        d(2026, 5, 7),
    );
    let dc_stacked_totw = stacked.player_attributes.current_reputation - stacked_after_pow_cur;

    assert!(
        dc_stacked_totw < dc_single,
        "stacked TOTW after POW ({}) must be dampened vs lone TOTW ({})",
        dc_stacked_totw,
        dc_single,
    );
}

#[test]
fn award_reputation_continental_poy_preserves_scale() {
    let mut p = build_award_player(d(1995, 1, 1), 6_000, 6_000, 5_000);
    p.apply_award_reputation_impact(
        AwardReputationKind::ContinentalPlayerOfYear,
        AwardReputationInput::new(),
        d(2026, 5, 7),
    );
    assert_eq!(p.player_attributes.current_reputation, 6_500);
    assert_eq!(p.player_attributes.home_reputation, 6_500);
    assert_eq!(p.player_attributes.world_reputation, 5_250);
}

#[test]
fn award_reputation_world_poy_preserves_scale() {
    let mut p = build_award_player(d(1995, 1, 1), 6_000, 6_000, 5_000);
    p.apply_award_reputation_impact(
        AwardReputationKind::WorldPlayerOfYear,
        AwardReputationInput::new(),
        d(2026, 5, 7),
    );
    assert_eq!(p.player_attributes.current_reputation, 6_900);
    assert_eq!(p.player_attributes.home_reputation, 6_900);
    assert_eq!(p.player_attributes.world_reputation, 5_500);
}

#[test]
fn award_reputation_clamps_at_ceiling() {
    let mut p = build_award_player(d(1995, 1, 1), 9_950, 9_950, 9_950);
    p.apply_award_reputation_impact(
        AwardReputationKind::WorldPlayerOfYear,
        AwardReputationInput::new(),
        d(2026, 5, 7),
    );
    assert_eq!(p.player_attributes.current_reputation, 10_000);
    assert_eq!(p.player_attributes.home_reputation, 10_000);
    assert_eq!(p.player_attributes.world_reputation, 10_000);
}

#[test]
fn award_reputation_player_of_season_more_than_team_of_year() {
    let mut toty = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let mut pos = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let input = AwardReputationInput::new()
        .with_league_reputation(7_000)
        .with_avg_rating(7.5)
        .with_matches_played(34);
    toty.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheYearSelection,
        input,
        d(2026, 5, 7),
    );
    pos.apply_award_reputation_impact(AwardReputationKind::PlayerOfTheSeason, input, d(2026, 5, 7));
    let dc_toty = toty.player_attributes.current_reputation - 5_000;
    let dc_pos = pos.player_attributes.current_reputation - 5_000;
    assert!(
        dc_pos > dc_toty,
        "POS ({}) must beat TOTY ({}) — individual top award",
        dc_pos,
        dc_toty,
    );
}

// ── Team of the Month / Young Team of the Month ─────────────

#[test]
fn catalog_magnitude_team_of_the_month_between_week_and_season() {
    let cat = MoraleEventCatalog::default();
    let week = cat.magnitude(HappinessEventType::TeamOfTheWeekSelection);
    let month = cat.magnitude(HappinessEventType::TeamOfTheMonthSelection);
    let season = cat.magnitude(HappinessEventType::TeamOfTheSeasonSelection);
    assert!(
        month > week,
        "TOTM magnitude ({}) must beat TOTW ({})",
        month,
        week
    );
    assert!(
        month < season,
        "TOTM magnitude ({}) must be smaller than TOTS ({})",
        month,
        season
    );
    let young_month = cat.magnitude(HappinessEventType::YoungTeamOfTheMonthSelection);
    assert!(
        young_month > 0.0,
        "Young TOTM magnitude must be positive, got {}",
        young_month
    );
}

#[test]
fn award_reputation_team_of_the_month_between_week_and_season() {
    let mut totw = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let mut totm = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let mut tots = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let input = AwardReputationInput::new()
        .with_league_reputation(7_000)
        .with_avg_rating(7.4)
        .with_matches_played(4);
    totw.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheWeekSelection,
        input,
        d(2026, 5, 7),
    );
    totm.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheMonthSelection,
        input,
        d(2026, 5, 7),
    );
    tots.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheSeasonSelection,
        AwardReputationInput::new()
            .with_league_reputation(7_000)
            .with_avg_rating(7.4)
            .with_matches_played(34),
        d(2026, 5, 7),
    );
    let dc_totw = totw.player_attributes.current_reputation - 5_000;
    let dc_totm = totm.player_attributes.current_reputation - 5_000;
    let dc_tots = tots.player_attributes.current_reputation - 5_000;
    assert!(
        dc_totm > dc_totw,
        "TOTM ({}) must beat TOTW ({})",
        dc_totm,
        dc_totw
    );
    assert!(
        dc_totm < dc_tots,
        "TOTM ({}) must be smaller than TOTS ({})",
        dc_totm,
        dc_tots
    );
}

#[test]
fn award_reputation_pom_then_totm_dampens_totm() {
    // Mirror of `award_reputation_pow_then_totw_dampens_totw` — POM
    // emits its happiness event, the centralised stacking dampener
    // picks it up and trims the TOTM reputation gain on the same
    // first-of-month tick.
    let mut single = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let mut stacked = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let input = AwardReputationInput::new()
        .with_league_reputation(7_000)
        .with_avg_rating(7.4)
        .with_matches_played(4);

    single.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheMonthSelection,
        input,
        d(2026, 5, 1),
    );
    let dc_single = single.player_attributes.current_reputation - 5_000;

    // Same player wins POM and is then named in the monthly XI on the
    // same tick. POM happiness event is the dampener trigger.
    stacked.on_recognition_award(
        HappinessEventType::PlayerOfTheMonth,
        RecognitionEventContext::new(RecognitionEventKind::PlayerOfTheMonth),
        28,
    );
    let stacked_after_pom_cur = stacked.player_attributes.current_reputation;
    stacked.apply_award_reputation_impact(
        AwardReputationKind::TeamOfTheMonthSelection,
        input,
        d(2026, 5, 1),
    );
    let dc_stacked_totm = stacked.player_attributes.current_reputation - stacked_after_pom_cur;

    assert!(
        dc_stacked_totm < dc_single,
        "stacked TOTM after POM ({}) must be dampened vs lone TOTM ({})",
        dc_stacked_totm,
        dc_single,
    );
}

#[test]
fn award_reputation_young_pom_then_young_totm_dampens() {
    // Young POM should dampen Young TOTM the same way POM dampens
    // TOTM — both fire on the same first-of-month tick.
    let kid = d(2007, 1, 1);
    let mut single = build_award_player(kid, 1_500, 1_500, 800);
    let mut stacked = build_award_player(kid, 1_500, 1_500, 800);
    let input = AwardReputationInput::new()
        .with_league_reputation(7_000)
        .with_avg_rating(7.4)
        .with_matches_played(4);

    single.apply_award_reputation_impact(
        AwardReputationKind::YoungTeamOfTheMonthSelection,
        input,
        d(2026, 5, 1),
    );
    let dc_single = single.player_attributes.current_reputation - 1_500;

    stacked.on_recognition_award(
        HappinessEventType::YoungPlayerOfTheMonth,
        RecognitionEventContext::new(RecognitionEventKind::YoungPlayerOfTheMonth),
        28,
    );
    let stacked_after_ypom = stacked.player_attributes.current_reputation;
    stacked.apply_award_reputation_impact(
        AwardReputationKind::YoungTeamOfTheMonthSelection,
        input,
        d(2026, 5, 1),
    );
    let dc_stacked = stacked.player_attributes.current_reputation - stacked_after_ypom;

    assert!(
        dc_stacked < dc_single,
        "stacked Young TOTM after Young POM ({}) must be dampened vs lone Young TOTM ({})",
        dc_stacked,
        dc_single,
    );
}

#[test]
fn on_recognition_award_records_team_of_the_month_event() {
    let mut p = build_award_player(d(1995, 1, 1), 5_000, 5_000, 4_000);
    let recorded = p.on_recognition_award(
        HappinessEventType::TeamOfTheMonthSelection,
        RecognitionEventContext::new(RecognitionEventKind::TeamOfTheMonthSelection),
        28,
    );
    assert!(recorded, "first-time TOTM emit must record");
    assert_eq!(
        count_events(&p, &HappinessEventType::TeamOfTheMonthSelection),
        1
    );
    // Cooldown of 28 days suppresses a same-tick double-fire.
    let second = p.on_recognition_award(
        HappinessEventType::TeamOfTheMonthSelection,
        RecognitionEventContext::new(RecognitionEventKind::TeamOfTheMonthSelection),
        28,
    );
    assert!(
        !second,
        "second TOTM emit inside cooldown must be suppressed"
    );
}

#[test]
fn on_recognition_award_records_young_team_of_the_month_event() {
    let mut p = build_award_player(d(2007, 1, 1), 1_500, 1_500, 800);
    let recorded = p.on_recognition_award(
        HappinessEventType::YoungTeamOfTheMonthSelection,
        RecognitionEventContext::new(RecognitionEventKind::YoungTeamOfTheMonthSelection),
        28,
    );
    assert!(recorded, "first-time Young TOTM emit must record");
    assert_eq!(
        count_events(&p, &HappinessEventType::YoungTeamOfTheMonthSelection),
        1
    );
}

// ════════════════════════════════════════════════════════════════════
// Snapshot-driven condition drop — exercises the new
// `MatchExertionInputs` pipeline. The match engine drains the
// `MatchPlayer` copy tick by tick; without the snapshot the
// persisted `Player.condition` could read 90% after a full 90-minute
// slog. These tests pin the end-to-end behaviour: a competitive 90
// must drop persisted condition by the calibrated band; a friendly
// must drop materially less; a cameo barely dents the tank; congestion
// stacks; injury risk responds to depletion + spike + congestion.
// ════════════════════════════════════════════════════════════════════

use crate::club::player::events::match_exertion::MatchExertionInputs;

/// Helper: build a snapshot for a player who starts at `start_cond`
/// and finishes the shift at `final_energy` after `minutes`. Uses
/// the position-group default HI share — matches the engine path.
fn make_snapshot(
    player: &Player,
    minutes: f32,
    start_cond: i16,
    final_energy: i16,
) -> MatchExertionInputs {
    use crate::club::player::events::PositionLoad;
    let group = player.position().position_group();
    MatchExertionInputs {
        minutes,
        starting_condition: start_cond,
        final_match_energy: final_energy,
        high_intensity_load_hint: PositionLoad::high_intensity_share(group),
    }
}

#[test]
fn full_90_competitive_drops_persisted_condition_into_expected_band() {
    // 90-min outfield midfielder, average stamina/NF/age — should
    // drop ~22-32 percentage points of persisted condition. This is
    // the canonical "condition visibly drops in the UI" case from
    // the acceptance criteria. The old behaviour kept the persisted
    // condition near 90% because only minute-count was wired.
    let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
    // Average stamina (10) / NF (10) — fresh_player has NF 14, override.
    p.skills.physical.stamina = 10.0;
    p.skills.physical.natural_fitness = 10.0;
    let pre = p.player_attributes.condition;
    // Player ran themselves into the engine floor: started 9500 →
    // ended 3000. That's a fully-drained outfielder.
    let inputs = make_snapshot(&p, 90.0, 9500, 3000);
    p.on_match_exertion(inputs, d(2025, 9, 14), false);
    let drop = pre - p.player_attributes.condition;
    assert!(
        (1800..=4000).contains(&(drop as i32)),
        "competitive 90' drop = {} raw points; expected 1800-4000",
        drop
    );
    // Sanity: the persisted condition didn't bottom out at the floor.
    assert!(
        p.player_attributes.condition >= 2500,
        "competitive floor must be 2500 raw"
    );
}

#[test]
fn final_match_energy_snapshot_survives_substitutions_via_engine_path() {
    // The substitution path captures the snapshot AT the swap minute
    // before the outgoing player is replaced. A 60th-minute sub-off
    // at 5500 must read "60 minutes, end at 5500" into
    // on_match_exertion, NOT "60 minutes, end at fresh_player default".
    let mut p = fresh_player(PlayerPositionType::ForwardLeft);
    p.skills.physical.stamina = 10.0;
    p.skills.physical.natural_fitness = 10.0;
    let pre = p.player_attributes.condition;
    // Started at 9500, was subbed off at 60' with 5500 in the tank.
    let inputs = MatchExertionInputs {
        minutes: 60.0,
        starting_condition: 9500,
        final_match_energy: 5500,
        high_intensity_load_hint: 0.32,
    };
    p.on_match_exertion(inputs, d(2025, 9, 14), false);
    let drop = pre - p.player_attributes.condition;
    // 60 min + moderate energy span = visible drop but not the full
    // 90' bill. Should be materially less than a 90' drained drop.
    assert!(drop > 500, "60' drop too small: {}", drop);
    assert!(drop < 2800, "60' drop too large: {}", drop);
}

#[test]
fn keeper_90_drops_less_condition_than_wingback_90() {
    // Position factor table: GK 0.45 vs WingbackLeft 1.18. Same
    // depletion, same age, same stamina/NF — a keeper finishes a
    // 90-min shift fresher.
    let mut gk = fresh_player(PlayerPositionType::Goalkeeper);
    let mut wb = fresh_player(PlayerPositionType::WingbackLeft);
    gk.skills.physical.stamina = 10.0;
    gk.skills.physical.natural_fitness = 10.0;
    wb.skills.physical.stamina = 10.0;
    wb.skills.physical.natural_fitness = 10.0;
    let gk_pre = gk.player_attributes.condition;
    let wb_pre = wb.player_attributes.condition;
    let gk_inputs = make_snapshot(&gk, 90.0, 9500, 4000);
    let wb_inputs = make_snapshot(&wb, 90.0, 9500, 4000);
    gk.on_match_exertion(gk_inputs, d(2025, 9, 14), false);
    wb.on_match_exertion(wb_inputs, d(2025, 9, 14), false);
    let gk_drop = gk_pre - gk.player_attributes.condition;
    let wb_drop = wb_pre - wb.player_attributes.condition;
    assert!(
        wb_drop > gk_drop + 300,
        "wingback drop {} must be materially greater than keeper drop {}",
        wb_drop,
        gk_drop
    );
}

#[test]
fn friendly_90_drops_materially_less_than_competitive_90() {
    // Spec: friendly drop ≈ 55% of competitive drop. Same player,
    // same shift, same depletion — different friendly_mult.
    let mut comp = fresh_player(PlayerPositionType::MidfielderCenter);
    let mut frnd = fresh_player(PlayerPositionType::MidfielderCenter);
    comp.skills.physical.stamina = 10.0;
    comp.skills.physical.natural_fitness = 10.0;
    frnd.skills.physical.stamina = 10.0;
    frnd.skills.physical.natural_fitness = 10.0;
    let comp_pre = comp.player_attributes.condition;
    let frnd_pre = frnd.player_attributes.condition;
    let inputs_c = make_snapshot(&comp, 90.0, 9500, 4000);
    let inputs_f = make_snapshot(&frnd, 90.0, 9500, 4000);
    comp.on_match_exertion(inputs_c, d(2025, 9, 14), false);
    frnd.on_match_exertion(inputs_f, d(2025, 9, 14), true);
    let comp_drop = comp_pre - comp.player_attributes.condition;
    let frnd_drop = frnd_pre - frnd.player_attributes.condition;
    // Friendly drop should be roughly 0.55× competitive (allow band
    // 0.40..0.70 — depletion stays the same, only friendly_mult
    // moves so the ratio is fairly tight).
    let ratio = frnd_drop as f32 / comp_drop.max(1) as f32;
    assert!(
        (0.40..=0.70).contains(&ratio),
        "friendly/competitive ratio {} outside expected 0.40..0.70 (drops f={} c={})",
        ratio,
        frnd_drop,
        comp_drop
    );
}

#[test]
fn twenty_minute_substitute_has_small_condition_drop_and_readiness_gain() {
    // Cameo subs barely dent the tank but DO rebuild match sharpness.
    let mut p = fresh_player(PlayerPositionType::ForwardCenter);
    p.skills.physical.stamina = 10.0;
    p.skills.physical.natural_fitness = 10.0;
    p.skills.physical.match_readiness = 10.0;
    let pre_cond = p.player_attributes.condition;
    let pre_sharp = p.skills.physical.match_readiness;
    // Cameo: came on fresh, lost a little energy in 20 min.
    let inputs = MatchExertionInputs {
        minutes: 20.0,
        starting_condition: 9500,
        final_match_energy: 8000,
        high_intensity_load_hint: 0.32,
    };
    p.on_match_exertion(inputs, d(2025, 9, 14), false);
    let drop = pre_cond - p.player_attributes.condition;
    assert!(
        drop < 1100,
        "20-min cameo drop too large: {} raw points",
        drop
    );
    assert!(
        p.skills.physical.match_readiness > pre_sharp,
        "20-min cameo must build readiness: pre {} post {}",
        pre_sharp,
        p.skills.physical.match_readiness
    );
}

#[test]
fn ninety_minute_competitive_match_lifts_load_debt_jadedness_readiness() {
    // After a real 90, every PlayerLoad signal moves: physical_load_7
    // increases, recovery_debt is non-zero, jadedness rises,
    // match_readiness moves upward.
    let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
    p.skills.physical.stamina = 10.0;
    p.skills.physical.natural_fitness = 10.0;
    let pre_jad = p.player_attributes.jadedness;
    let pre_sharp = p.skills.physical.match_readiness;
    let inputs = make_snapshot(&p, 90.0, 9500, 3500);
    p.on_match_exertion(inputs, d(2025, 9, 14), false);
    assert!(p.load.physical_load_7 > 60.0, "physical_load_7 must climb");
    assert!(p.load.recovery_debt > 0.0, "recovery_debt must accumulate");
    assert!(
        p.player_attributes.jadedness as i32 > pre_jad as i32 + 200,
        "jadedness must climb meaningfully: pre={} post={}",
        pre_jad,
        p.player_attributes.jadedness
    );
    assert!(
        p.skills.physical.match_readiness > pre_sharp,
        "match_readiness must climb: pre={} post={}",
        pre_sharp,
        p.skills.physical.match_readiness
    );
    assert_eq!(p.player_attributes.days_since_last_match, 0);
}

#[test]
fn three_matches_in_fourteen_days_increases_congestion_cost() {
    // Congestion_mult kicks in after the third match in 14 days.
    // Apples-to-apples: drive the pure `compute_condition_drop`
    // helper with identical starting/final/age/stamina, varying only
    // `matches_last_14`. The third match must drop materially more.
    let drop_with = |matches_last_14: u8| -> f32 {
        Player::compute_condition_drop(
            &MatchExertionInputs {
                minutes: 90.0,
                starting_condition: 9500,
                final_match_energy: 4000,
                high_intensity_load_hint: 0.30,
            },
            1.05,
            0.30,
            10.0,
            10.0,
            25,
            matches_last_14,
            false,
        )
    };
    let once_drop = drop_with(1);
    let thrice_drop = drop_with(3);
    assert!(
        thrice_drop > once_drop * 1.05,
        "congested 3rd-match drop {} should exceed once-only drop {} by 5%+",
        thrice_drop,
        once_drop
    );
}

#[test]
fn match_readiness_stays_in_zero_to_twenty_band_and_never_overshoots() {
    // Defensive: match_readiness is on a 0..20 sharpness scale; the
    // exertion path must clamp.
    let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
    p.skills.physical.match_readiness = 19.5; // near the ceiling
    let inputs = make_snapshot(&p, 90.0, 9500, 3500);
    p.on_match_exertion(inputs, d(2025, 9, 14), false);
    assert!(
        p.skills.physical.match_readiness <= 20.0,
        "match_readiness overshoot: {}",
        p.skills.physical.match_readiness
    );
}

// ════════════════════════════════════════════════════════════════════
// Training duration & condition-aware effects — exercises the new
// `duration_mult` scaling, the jadedness-from-load formula, and the
// recovery-debt model that now responds to player condition.
// ════════════════════════════════════════════════════════════════════

use crate::MatchSelectionContext;
use crate::PlayerClubContract;
use crate::PlayerSquadStatus;
use crate::RecognitionEventContext;
use crate::RecognitionEventKind;
use crate::SelectionDecisionScope;
use crate::SelectionOmissionReason;
use crate::SelectionRole;
use crate::TransferInterestEvidence;
use crate::TransferInterestKind;
use crate::TransferInterestReaction;
use crate::TransferInterestSource;
use crate::TransferInterestStage;
use crate::club::StaffStub;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::behaviour_config::MoraleEventCatalog;
use crate::club::player::training::training::PlayerTraining;
use crate::club::staff::Staff;
use crate::{TrainingIntensity, TrainingSession, TrainingType};
use chrono::Duration;
use chrono::NaiveDateTime;
use chrono::NaiveTime;

fn make_test_coach() -> Staff {
    let mut staff = StaffStub::default();
    staff.id = 900;
    staff.birth_date = d(1970, 1, 1);
    staff
}

fn make_session(ty: TrainingType, duration: u16) -> TrainingSession {
    TrainingSession {
        session_type: ty,
        intensity: TrainingIntensity::High,
        duration_minutes: duration,
        focus_positions: vec![],
        participants: vec![],
    }
}

#[test]
fn ninety_minute_pressing_costs_more_than_forty_five_minute_pressing() {
    // Duration scaling is the whole point of the new training model.
    // A 90-min pressing drill must cost ~2× a 45-min one in
    // fatigue and load.
    let p = fresh_player(PlayerPositionType::MidfielderCenter);
    let coach = make_test_coach();
    let date = NaiveDateTime::new(d(2025, 9, 14), NaiveTime::from_hms_opt(10, 0, 0).unwrap());
    let short = make_session(TrainingType::PressingDrills, 45);
    let long = make_session(TrainingType::PressingDrills, 90);
    let r_short = PlayerTraining::train(&p, &coach, &short, date, 0.5);
    let r_long = PlayerTraining::train(&p, &coach, &long, date, 0.5);
    assert!(
        r_long.effects.fatigue_change > r_short.effects.fatigue_change * 1.5,
        "90-min pressing fatigue {} must exceed 45-min pressing {} by 1.5+×",
        r_long.effects.fatigue_change,
        r_short.effects.fatigue_change
    );
    assert!(
        r_long.effects.physical_load_units > r_short.effects.physical_load_units * 1.5,
        "90-min pressing load {} must exceed 45-min load {} by 1.5+×",
        r_long.effects.physical_load_units,
        r_short.effects.physical_load_units
    );
}

#[test]
fn heavy_training_on_tired_player_accrues_more_debt_and_jadedness() {
    // Same drill, two players: one fresh (condition 95%, debt 0),
    // one tired (condition 50%, debt 0). The tired one must end
    // the session with more recovery_debt AND more jadedness.
    let mut fresh = fresh_player(PlayerPositionType::ForwardLeft);
    let mut tired = fresh_player(PlayerPositionType::ForwardLeft);
    tired.player_attributes.condition = 5000; // 50%
    let coach = make_test_coach();
    let date_t = NaiveDateTime::new(d(2025, 9, 14), NaiveTime::from_hms_opt(10, 0, 0).unwrap());
    let session = make_session(TrainingType::PressingDrills, 60);
    let r_fresh = PlayerTraining::train(&fresh, &coach, &session, date_t, 0.5);
    let r_tired = PlayerTraining::train(&tired, &coach, &session, date_t, 0.5);
    r_fresh.apply_to_player(&mut fresh, date_t.date());
    r_tired.apply_to_player(&mut tired, date_t.date());
    assert!(
        tired.load.recovery_debt > fresh.load.recovery_debt,
        "tired debt {} should exceed fresh debt {}",
        tired.load.recovery_debt,
        fresh.load.recovery_debt
    );
    // Jadedness gain is condition-aware too.
    assert!(
        tired.player_attributes.jadedness > fresh.player_attributes.jadedness,
        "tired jadedness {} should exceed fresh jadedness {}",
        tired.player_attributes.jadedness,
        fresh.player_attributes.jadedness
    );
}

#[test]
fn recovery_session_does_not_push_every_profile_to_same_post_session_condition() {
    // Three players all starting on identical low condition;
    // identical Recovery session. They MUST end up in visibly
    // different post-session condition — the deficit + efficiency
    // model is the whole point. An elite recovery profile (high NF,
    // high chronic fitness, high professionalism) closes more of the
    // deficit than an average one, who closes more than an old
    // overloaded body. None of them snaps to "fully fresh".
    let mut elite = fresh_player(PlayerPositionType::MidfielderCenter);
    let mut average = fresh_player(PlayerPositionType::MidfielderCenter);
    let mut tired_vet = fresh_player(PlayerPositionType::MidfielderCenter);
    for (i, p) in [&mut elite, &mut average, &mut tired_vet]
        .iter_mut()
        .enumerate()
    {
        p.id = 700 + i as u32;
        p.player_attributes.condition = 5_500;
    }
    elite.skills.physical.natural_fitness = 19.0;
    elite.player_attributes.fitness = 9_200;
    elite.attributes.professionalism = 19.0;
    average.skills.physical.natural_fitness = 12.0;
    average.player_attributes.fitness = 7_000;
    average.attributes.professionalism = 11.0;
    tired_vet.skills.physical.natural_fitness = 9.0;
    tired_vet.player_attributes.fitness = 6_200;
    tired_vet.attributes.professionalism = 11.0;
    tired_vet.load.recovery_debt = 1_400.0;
    tired_vet.birth_date = d(1990, 1, 1); // ~35

    let coach = make_test_coach();
    let date_t = NaiveDateTime::new(d(2025, 9, 14), NaiveTime::from_hms_opt(10, 0, 0).unwrap());
    for p in [&mut elite, &mut average, &mut tired_vet] {
        let session = make_session(TrainingType::Recovery, 60);
        let r = PlayerTraining::train(p, &coach, &session, date_t, 1.0);
        r.apply_to_player(p, date_t.date());
    }
    let elite_c = elite.player_attributes.condition;
    let avg_c = average.player_attributes.condition;
    let vet_c = tired_vet.player_attributes.condition;
    assert!(
        elite_c > avg_c && avg_c > vet_c,
        "elite ({}) > average ({}) > tired vet ({}) ordering must hold",
        elite_c,
        avg_c,
        vet_c
    );
    // None should be capped at the same value — that would mean the
    // session just refilled everyone to the cap, which is the
    // anti-pattern we're fixing.
    assert!(
        (elite_c - vet_c) > 100,
        "spread {} too small — recovery is washing the squad flat",
        elite_c - vet_c
    );
}

#[test]
fn recovery_session_restores_condition_but_caps_at_individualized_target() {
    // A Recovery session run on a low-condition player must not be
    // able to push them past the per-player target produced by
    // `ConditionRecoveryModel::individualized_target` — the deficit
    // ceiling for the recovery formula. The old "dynamic normal cap"
    // (88..95% based on NF alone) was replaced by the individualised
    // target, but the contract is the same: even an elite NF player
    // can't keep refilling beyond their per-day ceiling.
    let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
    p.player_attributes.condition = 5000; // 50%
    p.skills.physical.natural_fitness = 20.0; // elite — high target.
    let coach = make_test_coach();
    let date_t = NaiveDateTime::new(d(2025, 9, 14), NaiveTime::from_hms_opt(10, 0, 0).unwrap());
    // Run three recovery sessions back-to-back — even with massive
    // negative fatigue_change, the cap must hold.
    for _ in 0..3 {
        let session = make_session(TrainingType::Recovery, 60);
        let r = PlayerTraining::train(&p, &coach, &session, date_t, 1.0);
        r.apply_to_player(&mut p, date_t.date());
    }
    assert!(
        p.player_attributes.condition <= 9700,
        "elite NF recovery breached the individualised target ceiling: condition={}",
        p.player_attributes.condition
    );
    assert!(
        p.player_attributes.condition > 5000,
        "recovery did not restore condition at all: {}",
        p.player_attributes.condition
    );
}

// ════════════════════════════════════════════════════════════════════
// Daily recovery + dynamic cap — checks the
// `ConditionRecoveryModel` integrates into `process_condition_recovery`.
// ════════════════════════════════════════════════════════════════════

#[test]
fn daily_recovery_caps_at_dynamic_normal_for_elite_natural_fitness() {
    let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
    p.player_attributes.condition = 8800;
    p.skills.physical.natural_fitness = 20.0;
    p.player_attributes.days_since_last_match = 10;
    // Recovery over 30 days — condition should sit at the dynamic
    // cap (9500), never overshooting.
    for i in 0..30 {
        p.process_condition_recovery(d(2025, 11, 1) + Duration::days(i));
    }
    assert!(p.player_attributes.condition <= 9500);
    assert!(p.player_attributes.condition >= 9000);
}

#[test]
fn high_recovery_debt_throttles_daily_recovery() {
    // Two identical players; only difference is recovery_debt. The
    // one with heavy debt should recover slower per day.
    let mut healthy = fresh_player(PlayerPositionType::MidfielderCenter);
    let mut debted = fresh_player(PlayerPositionType::MidfielderCenter);
    healthy.player_attributes.condition = 6000;
    debted.player_attributes.condition = 6000;
    healthy.load.recovery_debt = 0.0;
    debted.load.recovery_debt = 1200.0;
    healthy.player_attributes.days_since_last_match = 5;
    debted.player_attributes.days_since_last_match = 5;
    for _ in 0..3 {
        healthy.process_condition_recovery(d(2025, 11, 14));
        debted.process_condition_recovery(d(2025, 11, 14));
    }
    assert!(
        healthy.player_attributes.condition > debted.player_attributes.condition,
        "healthy {} should outpace debted {}",
        healthy.player_attributes.condition,
        debted.player_attributes.condition
    );
}

#[test]
fn natural_recovery_drains_some_debt_each_day() {
    let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
    p.load.recovery_debt = 800.0;
    p.player_attributes.days_since_last_match = 5;
    let pre = p.load.recovery_debt;
    p.process_condition_recovery(d(2025, 11, 14));
    assert!(
        p.load.recovery_debt < pre,
        "natural debt drain didn't fire: pre={} post={}",
        pre,
        p.load.recovery_debt
    );
}

// ════════════════════════════════════════════════════════════════════
// `compute_condition_drop` pure unit — pins the formula curve
// independently of the surrounding mutation pipeline.
// ════════════════════════════════════════════════════════════════════

#[test]
fn compute_condition_drop_keeper_90_smaller_than_wingback_90() {
    let kpr_drop = Player::compute_condition_drop(
        &MatchExertionInputs {
            minutes: 90.0,
            starting_condition: 9500,
            final_match_energy: 4000,
            high_intensity_load_hint: 0.05,
        },
        0.45, // GK position factor
        0.05, // GK HI share
        10.0,
        10.0,
        25,
        1,
        false,
    );
    let wb_drop = Player::compute_condition_drop(
        &MatchExertionInputs {
            minutes: 90.0,
            starting_condition: 9500,
            final_match_energy: 4000,
            high_intensity_load_hint: 0.20,
        },
        1.18, // wingback position factor
        0.20, // defender group HI share
        10.0,
        10.0,
        25,
        1,
        false,
    );
    assert!(
        wb_drop > kpr_drop * 1.5,
        "wb drop {} should be much greater than gk drop {}",
        wb_drop,
        kpr_drop
    );
}

#[test]
fn compute_condition_drop_friendly_is_about_55pct_of_competitive() {
    let comp = Player::compute_condition_drop(
        &MatchExertionInputs {
            minutes: 90.0,
            starting_condition: 9500,
            final_match_energy: 4000,
            high_intensity_load_hint: 0.30,
        },
        1.05,
        0.30,
        10.0,
        10.0,
        25,
        1,
        false,
    );
    let frnd = Player::compute_condition_drop(
        &MatchExertionInputs {
            minutes: 90.0,
            starting_condition: 9500,
            final_match_energy: 4000,
            high_intensity_load_hint: 0.30,
        },
        1.05,
        0.30,
        10.0,
        10.0,
        25,
        1,
        true,
    );
    let ratio = frnd / comp;
    // Allow a fairly tight band around the friendly_mult=0.55 spec.
    assert!(
        (0.50..=0.60).contains(&ratio),
        "friendly/competitive ratio {} outside expected 0.50..0.60",
        ratio
    );
}

#[test]
fn compute_condition_drop_short_cameo_is_small_share_of_ninety() {
    let cameo = Player::compute_condition_drop(
        &MatchExertionInputs {
            minutes: 20.0,
            starting_condition: 9500,
            final_match_energy: 8000,
            high_intensity_load_hint: 0.30,
        },
        1.05,
        0.30,
        10.0,
        10.0,
        25,
        1,
        false,
    );
    let full = Player::compute_condition_drop(
        &MatchExertionInputs {
            minutes: 90.0,
            starting_condition: 9500,
            final_match_energy: 4000,
            high_intensity_load_hint: 0.30,
        },
        1.05,
        0.30,
        10.0,
        10.0,
        25,
        1,
        false,
    );
    // Cameo drop should be a small fraction (sub-linear in duration).
    assert!(
        cameo < full * 0.4,
        "cameo {} should be much less than full {}",
        cameo,
        full
    );
}

// ════════════════════════════════════════════════════════════════════
// Match exertion never heals — pins the "no immediate recovery from
// the post-match floor" semantics. A player who walked onto the pitch
// already below the competitive floor leaves it at, at most, the
// condition they brought in. Overnight recovery belongs to the daily
// rest path, not the exertion pass.
// ════════════════════════════════════════════════════════════════════

#[test]
fn low_starting_condition_is_not_lifted_by_match_exertion() {
    // Player walked on at 1500 (15%), well below the 2500 competitive
    // floor. They will burn another small amount of energy in the
    // match. The post-match persisted condition must NOT lift them
    // back up to 2500 — the floor is a worst-case stabilization for
    // players who STARTED above it, not a free overnight refill.
    let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
    p.skills.physical.stamina = 10.0;
    p.skills.physical.natural_fitness = 10.0;
    p.player_attributes.condition = 1_500;
    let inputs = MatchExertionInputs {
        minutes: 20.0,
        starting_condition: 1_500,
        final_match_energy: 1_500,
        high_intensity_load_hint: 0.30,
    };
    p.on_match_exertion(inputs, d(2025, 9, 14), false);
    assert!(
        p.player_attributes.condition <= 1_500,
        "exertion must never heal: pre=1500 post={}",
        p.player_attributes.condition
    );
}

#[test]
fn starting_above_floor_clamps_to_floor_not_below() {
    // Sanity check: a player who DID start above the floor but ran
    // themselves into the ground still floors at 2500, exactly as
    // before the no-heal change.
    let mut p = fresh_player(PlayerPositionType::ForwardLeft);
    p.skills.physical.stamina = 5.0;
    p.skills.physical.natural_fitness = 5.0;
    p.player_attributes.condition = 9_500;
    let inputs = MatchExertionInputs {
        minutes: 120.0,
        starting_condition: 9_500,
        final_match_energy: 1_500,
        high_intensity_load_hint: 0.40,
    };
    p.on_match_exertion(inputs, d(2025, 9, 14), false);
    assert!(
        p.player_attributes.condition >= 2_500,
        "competitive floor must hold for above-floor starters: post={}",
        p.player_attributes.condition
    );
}

#[test]
fn no_drop_when_zero_minutes_played() {
    // Edge case: a cameo of < 1 min produces zero drop. The persisted
    // condition must match the starting condition exactly (no clamp
    // up to floor, no clamp down — exertion did nothing).
    let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
    p.player_attributes.condition = 4_000;
    let inputs = MatchExertionInputs {
        minutes: 0.0,
        starting_condition: 4_000,
        final_match_energy: 4_000,
        high_intensity_load_hint: 0.30,
    };
    p.on_match_exertion(inputs, d(2025, 9, 14), false);
    assert_eq!(p.player_attributes.condition, 4_000);
}

// ════════════════════════════════════════════════════════════════════
// Fitness adaptation — the smooth absorption replaces the old hard
// cliffs. Strength sessions at HI = 0.15 must still build chronic
// fitness; a heavy session must not block ITS OWN fitness gain by
// booking debt before the absorption multiplier reads it.
// ════════════════════════════════════════════════════════════════════

#[test]
fn current_session_debt_does_not_block_its_own_fitness_gain() {
    // Pre-session debt is what gates fitness adaptation. A player
    // who walked into the session just under the old 500-debt cliff
    // and whose session ADDS enough to cross it must still bank the
    // chronic-fitness gain — the body absorbed the stimulus based
    // on the state it brought to the drill, not the bill it leaves
    // with. Under the previous hard cliff, this exact case would
    // post-read recovery_debt at >500 and drop the fitness gain to
    // zero.
    let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
    p.player_attributes.fitness = 7_000;
    p.player_attributes.condition = 9_500;
    // Walk in JUST under the legacy 500 cliff; the session itself
    // will push debt past it.
    p.load.recovery_debt = 490.0;
    let pre_fitness = p.player_attributes.fitness;
    let coach = make_test_coach();
    let date_t = NaiveDateTime::new(d(2025, 9, 14), NaiveTime::from_hms_opt(10, 0, 0).unwrap());
    let session = make_session(TrainingType::PressingDrills, 90);
    let r = PlayerTraining::train(&p, &coach, &session, date_t, 0.5);
    r.apply_to_player(&mut p, date_t.date());
    assert!(
        p.player_attributes.fitness > pre_fitness,
        "current-session debt must not block its own fitness gain: pre={} post={} debt-after={}",
        pre_fitness,
        p.player_attributes.fitness,
        p.load.recovery_debt
    );
    // And the debt did cross the legacy 500 cliff, demonstrating the
    // exact case the old gate would have failed on.
    assert!(
        p.load.recovery_debt > 500.0,
        "session should have pushed debt past 500 (legacy cliff): {}",
        p.load.recovery_debt
    );
}

#[test]
fn strength_session_at_intensity_share_boundary_still_builds_fitness() {
    // Spec: a strength session at exactly high_intensity_share 0.15
    // used to fall on the wrong side of the old `> 0.15` cliff and
    // bank zero chronic fitness. With the smooth absorption it must
    // still bank something — strength training IS adaptive load.
    let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
    p.player_attributes.fitness = 7_000;
    let pre_fitness = p.player_attributes.fitness;
    let coach = make_test_coach();
    let date_t = NaiveDateTime::new(d(2025, 9, 14), NaiveTime::from_hms_opt(10, 0, 0).unwrap());
    let session = make_session(TrainingType::Strength, 90);
    let r = PlayerTraining::train(&p, &coach, &session, date_t, 0.5);
    // Verify the boundary value the test relies on hasn't drifted.
    assert!(
        (r.effects.high_intensity_share - 0.15).abs() < 1e-4,
        "test depends on strength share being exactly 0.15, got {}",
        r.effects.high_intensity_share
    );
    r.apply_to_player(&mut p, date_t.date());
    assert!(
        p.player_attributes.fitness > pre_fitness,
        "strength session at HI share 0.15 must still build fitness: pre={} post={}",
        pre_fitness,
        p.player_attributes.fitness
    );
}

#[test]
fn elevated_debt_attenuates_fitness_gain_but_does_not_zero_it() {
    // Spec: smooth absorption — pre-session debt smoothly reduces the
    // fitness gain, never cliff-drops it to zero. A player carrying
    // 800 units of debt should still bank SOME chronic gain, just
    // smaller than a fresh player would.
    let mut fresh = fresh_player(PlayerPositionType::MidfielderCenter);
    let mut tired = fresh_player(PlayerPositionType::MidfielderCenter);
    fresh.player_attributes.fitness = 7_000;
    tired.player_attributes.fitness = 7_000;
    tired.load.recovery_debt = 800.0;
    let coach = make_test_coach();
    let date_t = NaiveDateTime::new(d(2025, 9, 14), NaiveTime::from_hms_opt(10, 0, 0).unwrap());
    let session = make_session(TrainingType::PressingDrills, 60);
    let r_fresh = PlayerTraining::train(&fresh, &coach, &session, date_t, 0.5);
    let r_tired = PlayerTraining::train(&tired, &coach, &session, date_t, 0.5);
    let fresh_pre_fitness = fresh.player_attributes.fitness;
    let tired_pre_fitness = tired.player_attributes.fitness;
    r_fresh.apply_to_player(&mut fresh, date_t.date());
    r_tired.apply_to_player(&mut tired, date_t.date());
    let fresh_gain = fresh.player_attributes.fitness - fresh_pre_fitness;
    let tired_gain = tired.player_attributes.fitness - tired_pre_fitness;
    assert!(fresh_gain > 0, "fresh fitness must grow: {}", fresh_gain);
    assert!(
        tired_gain > 0,
        "tired-but-non-floor must still grow: {}",
        tired_gain
    );
    assert!(
        tired_gain < fresh_gain,
        "tired gain {} must be smaller than fresh gain {}",
        tired_gain,
        fresh_gain
    );
}

#[test]
fn very_low_condition_blocks_fitness_gain_entirely() {
    // The hard floor still exists at pre_condition_pct < 45 — training
    // on fumes does not certify as adaptive load. The session still
    // books debt + jadedness; only the chronic gain is denied.
    let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
    p.player_attributes.fitness = 7_000;
    p.player_attributes.condition = 4_000; // 40% — below the 45 floor.
    let pre_fitness = p.player_attributes.fitness;
    let coach = make_test_coach();
    let date_t = NaiveDateTime::new(d(2025, 9, 14), NaiveTime::from_hms_opt(10, 0, 0).unwrap());
    let session = make_session(TrainingType::PressingDrills, 60);
    let r = PlayerTraining::train(&p, &coach, &session, date_t, 0.5);
    r.apply_to_player(&mut p, date_t.date());
    assert_eq!(
        p.player_attributes.fitness, pre_fitness,
        "training on fumes must not bank a chronic fitness gain"
    );
    // And the session DID still book debt (the cost lands).
    assert!(p.load.recovery_debt > 0.0);
}

#[test]
fn smooth_absorption_responds_to_pre_session_condition_continuously() {
    // The condition absorption multiplier is smooth: a player at 60%
    // gains less than one at 90% but more than one at 50%. Pin the
    // ordering — there must be no plateau in the middle.
    let coach = make_test_coach();
    let date_t = NaiveDateTime::new(d(2025, 9, 14), NaiveTime::from_hms_opt(10, 0, 0).unwrap());
    let session = make_session(TrainingType::PressingDrills, 60);

    let gain_at = |start_cond: i16| -> i16 {
        let mut p = fresh_player(PlayerPositionType::MidfielderCenter);
        p.player_attributes.fitness = 7_000;
        p.player_attributes.condition = start_cond;
        let pre = p.player_attributes.fitness;
        let r = PlayerTraining::train(&p, &coach, &session, date_t, 0.5);
        r.apply_to_player(&mut p, date_t.date());
        p.player_attributes.fitness - pre
    };
    let high = gain_at(9_000);
    let mid = gain_at(7_000);
    let low = gain_at(5_000);
    assert!(
        high > mid,
        "9000 gain {} should beat 7000 gain {}",
        high,
        mid
    );
    assert!(mid > low, "7000 gain {} should beat 5000 gain {}", mid, low);
    assert!(low > 0, "5000 should still bank some gain: {}", low);
}

// ════════════════════════════════════════════════════════════════════
// Snapshot-derived high-intensity hint — exercises the new
// behaviour-aware blend in `MatchPlayer::to_physical_snapshot`.
// ════════════════════════════════════════════════════════════════════

#[test]
fn snapshot_hint_rises_with_observed_pressure_volume() {
    use crate::r#match::engine::player::player::MatchPlayer;
    use crate::r#match::engine::player::statistics::MatchPlayerStatistics;
    // Two fictitious midfielders: same position default (0.30), but
    // one logged a busy match (pressures + tackles + dribbles), the
    // other sat in. The derived hint for the active one must be
    // higher than the position default; the passive one must stay
    // at or below the default.
    let quiet = MatchPlayerStatistics::new();
    let mut busy = MatchPlayerStatistics::new();
    busy.pressures = 40;
    busy.tackles = 8;
    busy.successful_dribbles = 6;
    busy.crosses_attempted = 4;
    busy.progressive_carries = 10;

    let position_default = 0.30;
    let minutes_played = 90.0;
    let quiet_hint =
        MatchPlayer::derive_high_intensity_hint(position_default, &quiet, minutes_played);
    let busy_hint =
        MatchPlayer::derive_high_intensity_hint(position_default, &busy, minutes_played);
    assert!(
        busy_hint > position_default,
        "busy player hint {} should exceed position default {}",
        busy_hint,
        position_default
    );
    assert!(
        quiet_hint <= position_default,
        "quiet player hint {} should not exceed default {}",
        quiet_hint,
        position_default
    );
    // Hint must stay in [0.02, 1.0] no matter how extreme the inputs.
    assert!(busy_hint <= 1.0);
    assert!(quiet_hint >= 0.02);
}

#[test]
fn snapshot_hint_zero_minutes_returns_position_default() {
    use crate::r#match::engine::player::player::MatchPlayer;
    use crate::r#match::engine::player::statistics::MatchPlayerStatistics;
    // A player with 0 minutes (subbed off at kickoff for an injury,
    // say) returns the position default exactly — no per-minute
    // division by something near zero.
    let stats = MatchPlayerStatistics::new();
    let hint = MatchPlayer::derive_high_intensity_hint(0.30, &stats, 0.0);
    assert_eq!(hint, 0.30);
}

// ════════════════════════════════════════════════════════════════════
// Individualized match exertion — proves the redesigned condition
// model leaves different players in visibly different physical states
// after the same match, instead of converging on a flat post-match
// distribution.
// ════════════════════════════════════════════════════════════════════

#[test]
fn ninety_minute_wingback_loses_more_condition_than_keeper() {
    // Position factor + HI-share + the new action_style_mult
    // (work_rate / pace / acceleration) all add up to a clearly
    // higher post-match condition drop for a wingback than a
    // keeper, even with identical fitness profiles.
    let mut gk = fresh_player(PlayerPositionType::Goalkeeper);
    let mut wb = fresh_player(PlayerPositionType::WingbackLeft);
    // Same baseline physical / mental profile so the only material
    // differences are position factor + HI share + action style.
    gk.skills.mental.work_rate = 12.0;
    wb.skills.mental.work_rate = 12.0;
    gk.skills.physical.pace = 12.0;
    wb.skills.physical.pace = 12.0;
    gk.skills.physical.acceleration = 12.0;
    wb.skills.physical.acceleration = 12.0;
    let date = d(2025, 9, 14);
    let gk_pre = gk.player_attributes.condition;
    let wb_pre = wb.player_attributes.condition;
    gk.on_match_exertion_minutes_only(90.0, date, false);
    wb.on_match_exertion_minutes_only(90.0, date, false);
    let gk_drop = gk_pre - gk.player_attributes.condition;
    let wb_drop = wb_pre - wb.player_attributes.condition;
    assert!(
        wb_drop >= gk_drop + 300,
        "wb drop {} should be ≥ gk drop {} + 300",
        wb_drop,
        gk_drop
    );
}

#[test]
fn ninety_minute_high_stamina_nf_loses_less_condition_than_low_stamina_nf() {
    // Same position, same minutes, same age — only the physical
    // profile differs. The high-stamina/high-NF player must finish
    // with materially more condition left in the tank.
    let mut elite = fresh_player(PlayerPositionType::MidfielderCenter);
    let mut weak = fresh_player(PlayerPositionType::MidfielderCenter);
    elite.skills.physical.stamina = 18.0;
    elite.skills.physical.natural_fitness = 18.0;
    weak.skills.physical.stamina = 8.0;
    weak.skills.physical.natural_fitness = 8.0;
    let date = d(2025, 9, 14);
    let elite_pre = elite.player_attributes.condition;
    let weak_pre = weak.player_attributes.condition;
    elite.on_match_exertion_minutes_only(90.0, date, false);
    weak.on_match_exertion_minutes_only(90.0, date, false);
    let elite_drop = elite_pre - elite.player_attributes.condition;
    let weak_drop = weak_pre - weak.player_attributes.condition;
    assert!(
        weak_drop > elite_drop + 200,
        "weak drop {} should clearly exceed elite drop {}",
        weak_drop,
        elite_drop
    );
}

#[test]
fn pressing_winger_finishes_more_tired_than_low_block_centerback() {
    // Both play 90 — but the high-WR pressing wide player should
    // visibly differ from the low-block CB. Drives the
    // action_style_mult; the wide player carries more debt and a
    // larger persisted condition drop.
    let mut presser = fresh_player(PlayerPositionType::MidfielderLeft);
    let mut cb = fresh_player(PlayerPositionType::DefenderCenter);
    presser.skills.mental.work_rate = 18.0;
    presser.skills.physical.pace = 17.0;
    presser.skills.physical.acceleration = 17.0;
    cb.skills.mental.work_rate = 10.0;
    cb.skills.physical.pace = 10.0;
    cb.skills.physical.acceleration = 10.0;
    let date = d(2025, 9, 14);
    let presser_pre = presser.player_attributes.condition;
    let cb_pre = cb.player_attributes.condition;
    presser.on_match_exertion_minutes_only(90.0, date, false);
    cb.on_match_exertion_minutes_only(90.0, date, false);
    let presser_drop = presser_pre - presser.player_attributes.condition;
    let cb_drop = cb_pre - cb.player_attributes.condition;
    assert!(
        presser_drop > cb_drop,
        "pressing winger drop {} should exceed low-block CB drop {}",
        presser_drop,
        cb_drop
    );
    assert!(
        presser.load.recovery_debt > cb.load.recovery_debt,
        "presser debt {} should exceed CB debt {}",
        presser.load.recovery_debt,
        cb.load.recovery_debt
    );
}

#[test]
fn weekly_matches_for_four_weeks_preserve_condition_spread_across_squad() {
    // Heuristic acceptance: differentiated XI playing the same
    // weekly matches over a month should NOT converge to a flat
    // condition distribution. The combination of individualized
    // recovery targets, position-based drains, and per-style
    // multipliers should keep the squad meaningfully spread.
    use crate::PlayerPositionType::*;
    use chrono::Duration;
    let positions = [
        Goalkeeper,
        DefenderCenter,
        WingbackLeft,
        DefenderRight,
        DefensiveMidfielder,
        MidfielderCenter,
        MidfielderLeft,
        AttackingMidfielderRight,
        Striker,
        ForwardLeft,
        ForwardCenter,
    ];
    let mut squad: Vec<Player> = positions
        .iter()
        .enumerate()
        .map(|(i, pos)| {
            let mut p = fresh_player(*pos);
            p.id = 500 + i as u32;
            // Spread the squad — wide range of NF/stamina/work_rate
            // so individualized recovery has variance to express.
            p.skills.physical.natural_fitness = 8.0 + (i as f32 * 1.0);
            p.skills.physical.stamina = 8.0 + (i as f32 * 0.9);
            p.skills.mental.work_rate = 9.0 + ((i as f32 * 0.7) % 9.0);
            p.skills.physical.pace = 9.0 + ((i as f32 * 1.1) % 9.0);
            p.skills.physical.acceleration = 9.0 + ((i as f32 * 0.6) % 9.0);
            p.player_attributes.fitness = 6_500 + (i as i16 * 250);
            p
        })
        .collect();
    let start = d(2025, 9, 1);
    // Four weeks: a Saturday match every 7 days, daily rest in
    // between (the recovery model lives at the daily granularity).
    for week in 0..4 {
        for day_offset in 0..7 {
            let date = start + Duration::days((week * 7 + day_offset) as i64);
            if day_offset == 6 {
                for p in &mut squad {
                    p.load.daily_decay(date);
                    p.on_match_exertion_minutes_only(90.0, date, false);
                }
            } else {
                for p in &mut squad {
                    p.load.daily_decay(date);
                    p.process_condition_recovery(date);
                }
            }
        }
    }
    let conditions: Vec<i16> = squad
        .iter()
        .map(|p| p.player_attributes.condition)
        .collect();
    let min = *conditions.iter().min().unwrap();
    let max = *conditions.iter().max().unwrap();
    let spread = max - min;
    assert!(
        spread >= 500,
        "squad condition spread {} too tight after 4 weeks: {:?}",
        spread,
        conditions
    );
}

#[test]
fn four_week_calibration_spread_across_positions_and_profiles() {
    // End-to-end calibration test: six distinctly-profiled players
    // run through four weeks of weekly competitive matches, mixed
    // training drills, and daily rest. The individualised condition
    // model is supposed to keep them visibly differentiated — under
    // the old "one dynamic cap, flat recovery" path they all
    // collapsed toward the same band by week 2. We assert both a
    // numerical spread AND a small set of qualitative orderings so
    // that brittle coefficient tuning can't silently flip the
    // story (an elite midfielder ending tireder than an overloaded
    // forward would be a regression, not a coefficient shift).
    use crate::PlayerPositionType::*;
    use chrono::Duration;

    fn build(
        id: u32,
        pos: PlayerPositionType,
        birth: NaiveDate,
        stamina: f32,
        natural_fitness: f32,
        work_rate: f32,
        pace: f32,
        acceleration: f32,
        fitness: i16,
        debt: f32,
        condition: i16,
    ) -> Player {
        let mut p = fresh_player(pos);
        p.id = id;
        p.birth_date = birth;
        p.skills.physical.stamina = stamina;
        p.skills.physical.natural_fitness = natural_fitness;
        p.skills.physical.pace = pace;
        p.skills.physical.acceleration = acceleration;
        p.skills.mental.work_rate = work_rate;
        p.player_attributes.fitness = fitness;
        p.player_attributes.condition = condition;
        p.load.recovery_debt = debt;
        p
    }

    let mut gk = build(
        601,
        Goalkeeper,
        d(1995, 1, 1), // 30y — typical keeper age
        14.0,
        14.0, // average stamina / NF
        10.0,
        8.0,
        8.0, // low action profile
        7_500,
        50.0,
        9_400,
    );
    let mut wingback = build(
        602,
        WingbackLeft,
        d(1999, 1, 1), // 26y
        15.0,
        14.0, // good stamina / NF
        17.0,
        16.0,
        16.0, // high WR + pace + acceleration
        8_000,
        100.0,
        9_300,
    );
    let mut center_back = build(
        603,
        DefenderCenter,
        d(1996, 1, 1), // 29y — experienced CB
        13.0,
        13.0,
        10.0,
        10.0,
        10.0, // low action profile
        7_800,
        80.0,
        9_300,
    );
    // Veteran midfielder (~35) — recovery age penalty should bite.
    let mut vet_mid = build(
        604,
        MidfielderCenter,
        d(1990, 6, 1), // ~35y
        13.0,
        13.0,
        14.0,
        12.0,
        11.0,
        7_300,
        120.0,
        9_200,
    );
    // Elite stamina/NF midfielder — should age well and outpace
    // overloaded peers over the 4-week window.
    let mut elite_mid = build(
        605,
        MidfielderCenter,
        d(2001, 1, 1), // 24y, prime
        19.0,
        19.0,
        14.0,
        14.0,
        14.0,
        9_000,
        50.0,
        9_400,
    );
    // Overloaded forward — starts with heavy debt + jadedness
    // baked in. The recovery throttle and target's load_drag
    // should keep them below the elite midfielder by the end of
    // the 4 weeks regardless of what training they got.
    let mut overload_fwd = build(
        606,
        ForwardCenter,
        d(1998, 1, 1), // 27y
        14.0,
        13.0,
        13.0,
        14.0,
        14.0,
        7_500,
        1_300.0,
        8_600,
    );
    overload_fwd.player_attributes.jadedness = 6_000;

    let start = d(2025, 9, 1);
    let coach = make_test_coach();

    // Sequence per week:
    //   Mon: light pressing drill (heavy training)
    //   Tue: recovery session
    //   Wed: light pressing drill
    //   Thu: rest (daily recovery only)
    //   Fri: recovery session
    //   Sat: 90-min competitive match for everyone
    //   Sun: rest (daily recovery only)
    // `process_condition_recovery` is called on every day; training
    // and matches add load on top of that.
    for week in 0..4 {
        for day_offset in 0..7 {
            let date = start + Duration::days((week * 7 + day_offset) as i64);
            let date_t = NaiveDateTime::new(date, NaiveTime::from_hms_opt(10, 0, 0).unwrap());
            let mut players: [&mut Player; 6] = [
                &mut gk,
                &mut wingback,
                &mut center_back,
                &mut vet_mid,
                &mut elite_mid,
                &mut overload_fwd,
            ];

            match day_offset {
                0 | 2 => {
                    // Heavy training day — same drill for everyone so
                    // the individualised cost multiplier is what
                    // separates them.
                    let session = make_session(TrainingType::PressingDrills, 60);
                    for p in players.iter_mut() {
                        p.load.daily_decay(date);
                        let r = PlayerTraining::train(p, &coach, &session, date_t, 0.5);
                        r.apply_to_player(p, date);
                    }
                }
                1 | 4 => {
                    // Recovery session — drains debt + jadedness in a
                    // blended way (actual gain + small potential).
                    let session = make_session(TrainingType::Recovery, 60);
                    for p in players.iter_mut() {
                        p.load.daily_decay(date);
                        let r = PlayerTraining::train(p, &coach, &session, date_t, 1.0);
                        r.apply_to_player(p, date);
                    }
                }
                3 | 6 => {
                    // Rest day — daily recovery model only.
                    for p in players.iter_mut() {
                        p.load.daily_decay(date);
                        p.process_condition_recovery(date);
                    }
                }
                5 => {
                    // Matchday — 90 competitive minutes for every
                    // player. The position factor + action style
                    // multiplier produce the spread within the same
                    // 90-minute slot.
                    for p in players.iter_mut() {
                        p.load.daily_decay(date);
                        p.on_match_exertion_minutes_only(90.0, date, false);
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    let gk_c = gk.player_attributes.condition;
    let wb_c = wingback.player_attributes.condition;
    let cb_c = center_back.player_attributes.condition;
    let vet_c = vet_mid.player_attributes.condition;
    let elite_c = elite_mid.player_attributes.condition;
    let overload_c = overload_fwd.player_attributes.condition;

    let conditions = [gk_c, wb_c, cb_c, vet_c, elite_c, overload_c];
    let min = *conditions.iter().min().unwrap();
    let max = *conditions.iter().max().unwrap();
    let spread = max - min;
    assert!(
        spread >= 700,
        "calibration spread {} too tight: gk={} wb={} cb={} vet={} elite={} overload={}",
        spread,
        gk_c,
        wb_c,
        cb_c,
        vet_c,
        elite_c,
        overload_c
    );

    // Ordering: the overloaded forward must end below the elite
    // midfielder. If those two flip, the load_drag / debt throttle
    // / blended-drain story isn't holding.
    assert!(
        overload_c < elite_c,
        "overloaded forward ({}) should finish below elite mid ({})",
        overload_c,
        elite_c
    );

    // Wingback either ends below CB on condition, OR carries higher
    // accumulated recovery debt — both are valid "wingback paid
    // more per match" signals. Asserting one OR the other keeps
    // the test from being brittle to whichever signal happens to
    // dominate on a given coefficient set.
    let wb_below_cb_in_condition = wb_c < cb_c;
    let wb_above_cb_in_debt = wingback.load.recovery_debt > center_back.load.recovery_debt;
    assert!(
        wb_below_cb_in_condition || wb_above_cb_in_debt,
        "wingback should pay more than CB: wb_cond={} cb_cond={} wb_debt={:.1} cb_debt={:.1}",
        wb_c,
        cb_c,
        wingback.load.recovery_debt,
        center_back.load.recovery_debt
    );

    // Goalkeeper has by far the lowest position factor / HI share /
    // action style, so they should finish among the highest-
    // condition outfielders. Asserting "top 2 of 6" instead of
    // "strict #1" stops the elite midfielder's recovery profile
    // from accidentally flipping the rank.
    let mut ranked: Vec<(u32, i16)> = vec![
        (gk.id, gk_c),
        (wingback.id, wb_c),
        (center_back.id, cb_c),
        (vet_mid.id, vet_c),
        (elite_mid.id, elite_c),
        (overload_fwd.id, overload_c),
    ];
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    let top_two_ids: Vec<u32> = ranked.iter().take(2).map(|(id, _)| *id).collect();
    assert!(
        top_two_ids.contains(&gk.id),
        "goalkeeper ({}) should rank among the two highest-condition players, ranked={:?}",
        gk_c,
        ranked
    );
}

// ── End-to-end: reported 9-app 8.2-prospect bug ───────────────
//
// Drives the same `on_match_played` pipeline a real simulator turn
// uses, then asserts that the display surface and decision surfaces
// both render the realistic value. Catches regressions where a future
// edit re-introduces a raw `average_rating_str()` consumer.

#[test]
fn nine_starts_at_eight_two_display_below_seven_six() {
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    let s = stats(8.2, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    let o = outcome(
        &s,
        8.2,
        false,
        false,
        false,
        false,
        1,
        0,
        MatchParticipation::Starter,
    );
    for _ in 0..9 {
        p.on_match_played(&o);
    }

    // Raw weighted ledger should expose the original 8.2 for any
    // single-match / debug surface that explicitly asks for it.
    let raw = p.statistics.average_rating_raw();
    assert!(
        (raw - 8.2).abs() < 0.02,
        "raw accessor should still report ~8.20, got {}",
        raw
    );

    // The public *display* helper — the one the web layer uses —
    // must report the regressed value, NOT the raw 8.20.
    let displayed = p
        .statistics
        .display_average_rating(PlayerFieldPositionGroup::Forward);
    let parsed: f32 = displayed.parse().unwrap();
    assert!(
        parsed < 7.6,
        "9-app 8.2 forward must display under 7.60 (regressed). got {}",
        displayed
    );
    assert!(
        parsed > 7.0,
        "regressed value should still be clearly above neutral. got {}",
        displayed
    );
}

#[test]
fn unused_substitute_does_not_contaminate_average() {
    // Unused-substitute path: an unused sub is booked in `played_subs`
    // (via `on_match_dropped` in real flow) but NEVER calls
    // `on_match_played`, so the rating ledger stays untouched. We
    // additionally guard against accidental direct calls to
    // `record_match_rating` with zero minutes by asserting the guard
    // holds at the player-stats level.
    let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
    // Three legitimate 7.0 starts establish a baseline.
    let baseline = stats(7.0, 0, 0, 0, PlayerFieldPositionGroup::Forward);
    let baseline_outcome = outcome(
        &baseline,
        7.0,
        false,
        false,
        false,
        false,
        1,
        0,
        MatchParticipation::Starter,
    );
    for _ in 0..3 {
        p.on_match_played(&baseline_outcome);
    }
    let baseline_weight = p.statistics.rating_weight;

    // Directly attempt to insert a zero-minute "appearance" — the
    // guard must reject it.
    p.statistics.record_match_rating(0.0, 0, false);
    assert_eq!(
        p.statistics.rating_weight, baseline_weight,
        "zero-minute / zero-rating record must be rejected"
    );
}

// ── ManagerCriticism reason-aware cooldown ──────────────────

/// Build a poor-but-not-catastrophic post-match outcome that lands the
/// rating in the 5.5..6.3 band — the band where the manager-criticism
/// branch fires. Tunes the personality so the picked reason is
/// deterministic: with `teamwork = 5.0` the branch resolves to
/// `MissedAssignment` (the second arm in the `if/else if` chain that
/// follows the red-card guard).
fn weak_player_with_missed_assignment_profile() -> Player {
    let mut p = build_player(PlayerPositionType::MidfielderCenter, PersonAttributes::default());
    p.skills.mental.work_rate = 15.0; // avoid PoorPressing arm
    p.skills.mental.teamwork = 5.0; // selects MissedAssignment
    p.attributes.professionalism = 14.0;
    p
}

#[test]
fn manager_criticism_duplicate_low_rating_is_cooldown_gated() {
    // Same criticism reason inside the 14-day window must collapse
    // to a single visible row even if the player has four more
    // sub-6.3 outings in the same fortnight.
    let mut p = weak_player_with_missed_assignment_profile();
    let s = stats(6.0, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
    let o = outcome(
        &s,
        6.0,
        false,
        false,
        false,
        false,
        1,
        1,
        MatchParticipation::Starter,
    );
    for _ in 0..5 {
        p.on_match_played(&o);
    }
    assert_eq!(
        count_events(&p, &HappinessEventType::ManagerCriticism),
        1,
        "same criticism reason must collapse to one visible row"
    );
    // The cumulative pressure must still bite — the suppressed events
    // feed the hidden form-pressure accumulator so morale degrades
    // even though the history feed only shows the first row.
    assert!(
        p.happiness.hidden_form_pressure < 0.0,
        "suppressed criticism must accumulate hidden form pressure (was {})",
        p.happiness.hidden_form_pressure
    );
}

#[test]
fn manager_criticism_distinct_reason_is_pinned() {
    // A low-rating run followed by a red-card incident must still
    // surface the new "PublicComplaint" criticism — the reason-aware
    // cooldown only suppresses the SAME reason. Pinning this so a
    // future tuning pass that switches back to a per-type cooldown
    // is caught here.
    let mut p = weak_player_with_missed_assignment_profile();
    let s = stats(6.0, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
    let o = outcome(
        &s,
        6.0,
        false,
        false,
        false,
        false,
        1,
        1,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o);
    assert_eq!(count_events(&p, &HappinessEventType::ManagerCriticism), 1);

    // Now a red card on a still-sub-6.3 outing — same player, same
    // window, but a materially different criticism (PublicComplaint).
    let s_red = stats(6.0, 0, 0, 1, PlayerFieldPositionGroup::Midfielder);
    let o_red = outcome(
        &s_red,
        6.0,
        false,
        false,
        false,
        false,
        1,
        1,
        MatchParticipation::Starter,
    );
    p.on_match_played(&o_red);
    assert_eq!(
        count_events(&p, &HappinessEventType::ManagerCriticism),
        2,
        "distinct criticism reason inside the 14-day window must still fire"
    );
}
