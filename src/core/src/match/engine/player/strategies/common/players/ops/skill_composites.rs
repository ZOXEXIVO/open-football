//! Composite skill helpers shared across the match engine.
//!
//! Each composite produces a normalized 0..1 value that blends several
//! attributes through `effective_skill` (so fatigue, late-game mental
//! penalty, and stamina mitigation are applied consistently). Callers
//! turn the composite into a probability or a multiplier; the composite
//! itself never reaches outside the [0.05, 1.0] band so a single
//! call site can plug into existing clamps without surprises.
//!
//! Polish note: composites intentionally do not double-apply
//! `match_readiness` — the engine's own physics layer already reads
//! readiness for movement and the per-event evaluators that need a
//! readiness sharpening curve can multiply by `readiness_factor`
//! returned alongside the composite.
//!
//! Categories follow the spec:
//!
//!   * `passing_execution`
//!   * `long_passing`
//!   * `receiving_first_touch`
//!   * `shooting_close` / `shooting_medium` / `long_shot`
//!   * `dribble_attack`
//!   * `defensive_duel`
//!   * `interception`
//!   * `pressing`
//!   * `aerial_outfield_defender` / `aerial_outfield_attacker`
//!   * `gk_shot_stopping` / `gk_aerial` / `gk_distribution`

use crate::r#match::MatchPlayer;
use crate::r#match::engine::player::strategies::common::players::ops::effective_skill::{
    ActionContext, SkillBands, SkillCategory, effective_skill,
};

/// Category shorthands for the `SkillBands`-based reads below. Each
/// composite builds the per-(player, minute) bands ONCE and applies them
/// per attribute — `SkillBands::apply` is pinned bit-identical to the
/// per-read `effective_skill` path (see
/// `effective_skill_bit_identical_to_bands`), so this is purely a
/// recompute-elimination: the band `powf` and mitigation blend run once
/// per composite call instead of once per attribute read.
const TECH: SkillCategory = SkillCategory::Technical;
const MENT: SkillCategory = SkillCategory::Mental;
const EXPL: SkillCategory = SkillCategory::Explosive;

/// Standard 0..1 normaliser for a 1..20 skill value.
#[inline]
pub fn n(skill: f32) -> f32 {
    (skill / 20.0).clamp(0.0, 1.0)
}

/// Floor / ceiling applied to every composite output.
const COMPOSITE_FLOOR: f32 = 0.05;
const COMPOSITE_CEIL: f32 = 1.0;

#[inline]
fn clamp_composite(v: f32) -> f32 {
    v.clamp(COMPOSITE_FLOOR, COMPOSITE_CEIL)
}

/// Convenience: read a skill from the player and apply the fatigue model.
/// `accessor` returns the 1..20 raw value.
#[inline]
pub fn eff<F>(player: &MatchPlayer, ctx: ActionContext, accessor: F) -> f32
where
    F: FnOnce(&MatchPlayer) -> f32,
{
    effective_skill(player, accessor(player), ctx)
}

/// Derive minute from a `MatchContext`-style total time in milliseconds.
#[inline]
pub fn minute_from_ms(total_ms: u64) -> u32 {
    (total_ms / 60_000) as u32
}

/// Derive minute from a 10ms-tick counter (the engine's physics tick is
/// 10ms long, so 6000 ticks == 1 minute). Used by ball-side code that
/// only sees the cached tick — every other call should prefer
/// [`minute_from_ms`] against `MatchContext::total_match_time`. Both
/// helpers MUST stay in lockstep so composites in adjacent code paths
/// see the same minute.
#[inline]
pub fn minute_from_ticks(tick: u64) -> u32 {
    (tick / 6000) as u32
}

/// `match_readiness`-driven sharpening factor in [0.85, 1.0].
/// Multiply an execution composite by this when you want a readiness
/// curve on top — e.g. fresh-out-of-injury players executing at 92%.
#[inline]
pub fn readiness_factor(player: &MatchPlayer) -> f32 {
    let r = (player.skills.physical.match_readiness / 20.0).clamp(0.0, 1.0);
    0.85 + r * 0.15
}

// ---------------------------------------------------------------------------
// Passing
// ---------------------------------------------------------------------------

/// Passing execution composite.
/// `passing*0.38 + technique*0.20 + vision*0.16 + decisions*0.10
///  + composure*0.08 + concentration*0.08`
pub fn passing_execution(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.technical.passing, TECH)) * 0.38
        + n(b.apply(s.technical.technique, TECH)) * 0.20
        + n(b.apply(s.mental.vision, MENT)) * 0.16
        + n(b.apply(s.mental.decisions, MENT)) * 0.10
        + n(b.apply(s.mental.composure, MENT)) * 0.08
        + n(b.apply(s.mental.concentration, MENT)) * 0.08)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Long pass / switch composite.
/// `passing*0.30 + vision*0.24 + technique*0.20 + decisions*0.10
///  + flair*0.06 + balance*0.04 + composure*0.06`
pub fn long_passing(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.technical.passing, TECH)) * 0.30
        + n(b.apply(s.mental.vision, MENT)) * 0.24
        + n(b.apply(s.technical.technique, TECH)) * 0.20
        + n(b.apply(s.mental.decisions, MENT)) * 0.10
        + n(b.apply(s.mental.flair, MENT)) * 0.06
        + n(b.apply(s.physical.balance, TECH)) * 0.04
        + n(b.apply(s.mental.composure, MENT)) * 0.06)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Receiving / first-touch composite.
/// `first_touch*0.28 + technique*0.18 + composure*0.14 + anticipation*0.12
///  + balance*0.10 + agility*0.08 + decisions*0.06 + concentration*0.04`
pub fn receiving_first_touch(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.technical.first_touch, TECH)) * 0.28
        + n(b.apply(s.technical.technique, TECH)) * 0.18
        + n(b.apply(s.mental.composure, MENT)) * 0.14
        + n(b.apply(s.mental.anticipation, MENT)) * 0.12
        + n(b.apply(s.physical.balance, TECH)) * 0.10
        + n(b.apply(s.physical.agility, EXPL)) * 0.08
        + n(b.apply(s.mental.decisions, MENT)) * 0.06
        + n(b.apply(s.mental.concentration, MENT)) * 0.04)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

// ---------------------------------------------------------------------------
// Shooting
// ---------------------------------------------------------------------------

/// Apply the low-skill-punishing power curve used across the shooting
/// composites. A 5/20 input (0.25) maps to ~0.10 instead of staying at
/// 0.25, so a player with all-5 skills no longer behaves like half an
/// elite shooter when the components are summed.
#[inline]
fn curve(skill01: f32, exp: f32) -> f32 {
    skill01.clamp(0.0, 1.0).powf(exp)
}

/// Close-range shooting composite (skill-curved).
/// `finishing^1.65*0.42 + composure^1.45*0.22 + first_touch^1.45*0.13
///  + technique^1.45*0.10 + decisions^1.35*0.08 + balance^1.25*0.05`
pub fn shooting_close(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (curve(n(b.apply(s.technical.finishing, TECH)), 1.65) * 0.42
        + curve(n(b.apply(s.mental.composure, MENT)), 1.45) * 0.22
        + curve(n(b.apply(s.technical.first_touch, TECH)), 1.45) * 0.13
        + curve(n(b.apply(s.technical.technique, TECH)), 1.45) * 0.10
        + curve(n(b.apply(s.mental.decisions, MENT)), 1.35) * 0.08
        + curve(n(b.apply(s.physical.balance, TECH)), 1.25) * 0.05)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Medium-range shooting composite (skill-curved).
/// `finishing^1.65*0.30 + technique^1.55*0.22 + long_shots^1.65*0.18
///  + composure^1.45*0.14 + decisions^1.35*0.10 + balance^1.25*0.06`
pub fn shooting_medium(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (curve(n(b.apply(s.technical.finishing, TECH)), 1.65) * 0.30
        + curve(n(b.apply(s.technical.technique, TECH)), 1.55) * 0.22
        + curve(n(b.apply(s.technical.long_shots, TECH)), 1.65) * 0.18
        + curve(n(b.apply(s.mental.composure, MENT)), 1.45) * 0.14
        + curve(n(b.apply(s.mental.decisions, MENT)), 1.35) * 0.10
        + curve(n(b.apply(s.physical.balance, TECH)), 1.25) * 0.06)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Long-shot composite (skill-curved).
/// `long_shots^1.75*0.38 + technique^1.60*0.24 + composure^1.45*0.13
///  + decisions^1.40*0.11 + strength^1.25*0.07 + balance^1.25*0.07`
pub fn long_shot(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (curve(n(b.apply(s.technical.long_shots, TECH)), 1.75) * 0.38
        + curve(n(b.apply(s.technical.technique, TECH)), 1.60) * 0.24
        + curve(n(b.apply(s.mental.composure, MENT)), 1.45) * 0.13
        + curve(n(b.apply(s.mental.decisions, MENT)), 1.40) * 0.11
        + curve(n(b.apply(s.physical.strength, EXPL)), 1.25) * 0.07
        + curve(n(b.apply(s.physical.balance, TECH)), 1.25) * 0.07)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

// ---------------------------------------------------------------------------
// Carrying / duels
// ---------------------------------------------------------------------------

/// Dribble attack composite (attacker side of a 1v1 duel).
/// `dribbling*0.25 + technique*0.17 + flair*0.10 + agility*0.14
///  + acceleration*0.10 + balance*0.09 + composure*0.07
///  + decisions*0.05 + strength*0.03`
pub fn dribble_attack(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.technical.dribbling, TECH)) * 0.25
        + n(b.apply(s.technical.technique, TECH)) * 0.17
        + n(b.apply(s.mental.flair, MENT)) * 0.10
        + n(b.apply(s.physical.agility, EXPL)) * 0.14
        + n(b.apply(s.physical.acceleration, EXPL)) * 0.10
        + n(b.apply(s.physical.balance, TECH)) * 0.09
        + n(b.apply(s.mental.composure, MENT)) * 0.07
        + n(b.apply(s.mental.decisions, MENT)) * 0.05
        + n(b.apply(s.physical.strength, EXPL)) * 0.03)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Defensive duel composite (defender side of a 1v1).
/// `tackling*0.24 + positioning*0.17 + anticipation*0.15 + marking*0.13
///  + strength*0.10 + balance*0.07 + agility*0.06 + concentration*0.05
///  + bravery*0.03`
pub fn defensive_duel(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.technical.tackling, TECH)) * 0.24
        + n(b.apply(s.mental.positioning, MENT)) * 0.17
        + n(b.apply(s.mental.anticipation, MENT)) * 0.15
        + n(b.apply(s.technical.marking, TECH)) * 0.13
        + n(b.apply(s.physical.strength, EXPL)) * 0.10
        + n(b.apply(s.physical.balance, TECH)) * 0.07
        + n(b.apply(s.physical.agility, EXPL)) * 0.06
        + n(b.apply(s.mental.concentration, MENT)) * 0.05
        + n(b.apply(s.mental.bravery, MENT)) * 0.03)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Interception composite.
/// `anticipation*0.24 + positioning*0.20 + concentration*0.16 + acceleration*0.12
///  + pace*0.10 + marking*0.08 + decisions*0.06 + agility*0.04`
pub fn interception(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.mental.anticipation, MENT)) * 0.24
        + n(b.apply(s.mental.positioning, MENT)) * 0.20
        + n(b.apply(s.mental.concentration, MENT)) * 0.16
        + n(b.apply(s.physical.acceleration, EXPL)) * 0.12
        + n(b.apply(s.physical.pace, EXPL)) * 0.10
        + n(b.apply(s.technical.marking, TECH)) * 0.08
        + n(b.apply(s.mental.decisions, MENT)) * 0.06
        + n(b.apply(s.physical.agility, EXPL)) * 0.04)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Pressing composite.
/// `work_rate*0.24 + stamina*0.18 + aggression*0.14 + acceleration*0.12
///  + pace*0.10 + decisions*0.08 + teamwork*0.08 + concentration*0.06`
pub fn pressing(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    // `stamina` is the fatigue mitigator itself, so reading it through
    // the explosive band still bumps it via the band when condition is
    // low — that's the desired direction (a tired player presses less).
    let s = &player.skills;
    let v = (n(b.apply(s.mental.work_rate, MENT)) * 0.24
        + n(b.apply(s.physical.stamina, EXPL)) * 0.18
        + n(b.apply(s.mental.aggression, MENT)) * 0.14
        + n(b.apply(s.physical.acceleration, EXPL)) * 0.12
        + n(b.apply(s.physical.pace, EXPL)) * 0.10
        + n(b.apply(s.mental.decisions, MENT)) * 0.08
        + n(b.apply(s.mental.teamwork, MENT)) * 0.08
        + n(b.apply(s.mental.concentration, MENT)) * 0.06)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

// ---------------------------------------------------------------------------
// Selection / decision composites
//
// Execution composites (passing_execution, shooting_close, ...) score
// HOW WELL a player would perform a chosen action. The selection
// composites below score WHETHER the player should pick that action in
// the first place — used by decision sites (shoot vs pass, force a
// chance vs delay, etc.). Keeping the two cleanly separated means the
// engine doesn't conflate "good shooter" with "good chooser of when to
// shoot": a pure poacher (high finishing, mid decisions) and a
// playmaking forward (mid finishing, high decisions) read distinctly.
// ---------------------------------------------------------------------------

/// Off-ball attacking-movement composite.
/// `off_the_ball*0.35 + anticipation*0.20 + decisions*0.15
///  + acceleration*0.10 + pace*0.08 + teamwork*0.07 + bravery*0.05`
/// (5% bravery makes the composite sum to 1.0 — runs in behind require
/// some bravery, this isn't a free slot.)
pub fn off_ball_attack(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.mental.off_the_ball, MENT)) * 0.35
        + n(b.apply(s.mental.anticipation, MENT)) * 0.20
        + n(b.apply(s.mental.decisions, MENT)) * 0.15
        + n(b.apply(s.physical.acceleration, EXPL)) * 0.10
        + n(b.apply(s.physical.pace, EXPL)) * 0.08
        + n(b.apply(s.mental.teamwork, MENT)) * 0.07
        + n(b.apply(s.mental.bravery, MENT)) * 0.05)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Shot-selection composite — should the player shoot or pass / hold?
/// `composure*0.25 + decisions*0.25 + finishing*0.18 + long_shots*0.12
///  + vision*0.10 + teamwork*0.10`
pub fn shot_selection(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.mental.composure, MENT)) * 0.25
        + n(b.apply(s.mental.decisions, MENT)) * 0.25
        + n(b.apply(s.technical.finishing, TECH)) * 0.18
        + n(b.apply(s.technical.long_shots, TECH)) * 0.12
        + n(b.apply(s.mental.vision, MENT)) * 0.10
        + n(b.apply(s.mental.teamwork, MENT)) * 0.10)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Pass-selection composite — picking the right ball, not executing it.
/// `decisions*0.25 + vision*0.25 + passing*0.18 + composure*0.12
///  + teamwork*0.12 + flair*0.08`
pub fn pass_selection(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.mental.decisions, MENT)) * 0.25
        + n(b.apply(s.mental.vision, MENT)) * 0.25
        + n(b.apply(s.technical.passing, TECH)) * 0.18
        + n(b.apply(s.mental.composure, MENT)) * 0.12
        + n(b.apply(s.mental.teamwork, MENT)) * 0.12
        + n(b.apply(s.mental.flair, MENT)) * 0.08)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Defensive positioning composite — reading the play without the ball.
/// `positioning*0.30 + anticipation*0.22 + concentration*0.18
///  + decisions*0.12 + teamwork*0.10 + acceleration*0.08`
pub fn defensive_positioning(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.mental.positioning, MENT)) * 0.30
        + n(b.apply(s.mental.anticipation, MENT)) * 0.22
        + n(b.apply(s.mental.concentration, MENT)) * 0.18
        + n(b.apply(s.mental.decisions, MENT)) * 0.12
        + n(b.apply(s.mental.teamwork, MENT)) * 0.10
        + n(b.apply(s.physical.acceleration, EXPL)) * 0.08)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

// ---------------------------------------------------------------------------
// Aerial outfield
// ---------------------------------------------------------------------------

/// Aerial composite for a defender (positioning slot is `positioning`).
pub fn aerial_outfield_defender(player: &MatchPlayer, minute: u32) -> f32 {
    aerial_outfield_inner(player, minute, true)
}

/// Aerial composite for an attacker (positioning slot is `off_the_ball`).
pub fn aerial_outfield_attacker(player: &MatchPlayer, minute: u32) -> f32 {
    aerial_outfield_inner(player, minute, false)
}

fn aerial_outfield_inner(player: &MatchPlayer, minute: u32, defender: bool) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let positioning_or_otb = if defender {
        n(b.apply(s.mental.positioning, MENT))
    } else {
        n(b.apply(s.mental.off_the_ball, MENT))
    };
    let v = (n(b.apply(s.technical.heading, TECH)) * 0.28
        + n(b.apply(s.physical.jumping, EXPL)) * 0.24
        + n(b.apply(s.physical.strength, EXPL)) * 0.16
        + n(b.apply(s.mental.bravery, MENT)) * 0.12
        + positioning_or_otb * 0.10
        + n(b.apply(s.mental.anticipation, MENT)) * 0.06
        + n(b.apply(s.physical.balance, TECH)) * 0.04)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

// ---------------------------------------------------------------------------
// Goalkeeping
// ---------------------------------------------------------------------------

/// GK shot stopping composite.
/// `reflexes*0.30 + handling*0.18 + agility*0.16 + positioning*0.10
///  + concentration*0.10 + anticipation*0.08 + one_on_ones*0.08`
pub fn gk_shot_stopping(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    // GK skills are technical-feel (reflexes/handling/one_on_ones) so we
    // route them through the technical category which has the smallest
    // fatigue penalty.
    let v = (n(b.apply(s.goalkeeping.reflexes, TECH)) * 0.30
        + n(b.apply(s.goalkeeping.handling, TECH)) * 0.18
        + n(b.apply(s.physical.agility, EXPL)) * 0.16
        + n(b.apply(s.mental.positioning, MENT)) * 0.10
        + n(b.apply(s.mental.concentration, MENT)) * 0.10
        + n(b.apply(s.mental.anticipation, MENT)) * 0.08
        + n(b.apply(s.goalkeeping.one_on_ones, TECH)) * 0.08)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// GK aerial composite.
/// `aerial_reach*0.28 + command_of_area*0.20 + handling*0.14 + jumping*0.12
///  + strength*0.10 + bravery*0.08 + communication*0.08`
pub fn gk_aerial(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.goalkeeping.aerial_reach, TECH)) * 0.28
        + n(b.apply(s.goalkeeping.command_of_area, MENT)) * 0.20
        + n(b.apply(s.goalkeeping.handling, TECH)) * 0.14
        + n(b.apply(s.physical.jumping, EXPL)) * 0.12
        + n(b.apply(s.physical.strength, EXPL)) * 0.10
        + n(b.apply(s.mental.bravery, MENT)) * 0.08
        + n(b.apply(s.goalkeeping.communication, MENT)) * 0.08)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// GK communication / organisation composite — shouting, marshalling
/// the defensive line, calling for crosses. Used by tactical-organisation
/// damping and by GK area-control logic.
/// `communication*0.45 + command_of_area*0.25 + leadership*0.15
///  + concentration*0.10 + positioning*0.05`
pub fn gk_communication(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.goalkeeping.communication, MENT)) * 0.45
        + n(b.apply(s.goalkeeping.command_of_area, MENT)) * 0.25
        + n(b.apply(s.mental.leadership, MENT)) * 0.15
        + n(b.apply(s.mental.concentration, MENT)) * 0.10
        + n(b.apply(s.mental.positioning, MENT)) * 0.05)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// GK cross-claim composite — collecting / punching crosses, dealing
/// with high balls into the box. Subtly different from `gk_aerial`:
/// claim_cross weights handling/positioning over raw aerial reach so
/// a small but well-positioned keeper still earns claim credit.
/// `aerial_reach*0.28 + command_of_area*0.22 + handling*0.18
///  + positioning*0.12 + anticipation*0.10 + jumping*0.10`
pub fn gk_claim_cross(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.goalkeeping.aerial_reach, TECH)) * 0.28
        + n(b.apply(s.goalkeeping.command_of_area, MENT)) * 0.22
        + n(b.apply(s.goalkeeping.handling, TECH)) * 0.18
        + n(b.apply(s.mental.positioning, MENT)) * 0.12
        + n(b.apply(s.mental.anticipation, MENT)) * 0.10
        + n(b.apply(s.physical.jumping, EXPL)) * 0.10)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// GK distribution composite.
/// `goalkeeping.passing*0.24 + kicking*0.22 + throwing*0.18 + vision*0.12
///  + decisions*0.10 + composure*0.08 + technique/first_touch*0.06`
pub fn gk_distribution(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    // `technique/first_touch` slot — average the two technical reads
    // since the spec leaves it explicitly as either-or.
    let touch_avg = 0.5
        * (n(b.apply(s.technical.technique, TECH))
            + n(b.apply(s.goalkeeping.first_touch, TECH)));
    let v = (n(b.apply(s.goalkeeping.passing, TECH)) * 0.24
        + n(b.apply(s.goalkeeping.kicking, TECH)) * 0.22
        + n(b.apply(s.goalkeeping.throwing, TECH)) * 0.18
        + n(b.apply(s.mental.vision, MENT)) * 0.12
        + n(b.apply(s.mental.decisions, MENT)) * 0.10
        + n(b.apply(s.mental.composure, MENT)) * 0.08
        + touch_avg * 0.06)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

// ---------------------------------------------------------------------------
// Generic mobility / decision / loose-ball composites
// ---------------------------------------------------------------------------

/// General mobility composite — how fast and balanced a player moves
/// without the ball. Used by pressing chase speed, recovery runs,
/// and movement-cost multipliers.
/// `pace*0.25 + acceleration*0.25 + agility*0.20 + balance*0.12
///  + stamina*0.10 + natural_fitness*0.08`
pub fn mobility(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.physical.pace, EXPL)) * 0.25
        + n(b.apply(s.physical.acceleration, EXPL)) * 0.25
        + n(b.apply(s.physical.agility, EXPL)) * 0.20
        + n(b.apply(s.physical.balance, TECH)) * 0.12
        + n(b.apply(s.physical.stamina, EXPL)) * 0.10
        + n(b.apply(s.physical.natural_fitness, EXPL)) * 0.08)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Generic decision-quality composite — used as the read-the-game
/// signal in spots where no single execution composite fits.
/// `decisions*0.32 + composure*0.18 + concentration*0.18
///  + anticipation*0.14 + teamwork*0.10 + vision*0.08`
pub fn decision_quality(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.mental.decisions, MENT)) * 0.32
        + n(b.apply(s.mental.composure, MENT)) * 0.18
        + n(b.apply(s.mental.concentration, MENT)) * 0.18
        + n(b.apply(s.mental.anticipation, MENT)) * 0.14
        + n(b.apply(s.mental.teamwork, MENT)) * 0.10
        + n(b.apply(s.mental.vision, MENT)) * 0.08)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Carrying-with-ball speed composite — feeds the dribble-run movement
/// speed multiplier. Slightly different from `dribble_attack` (which
/// resolves the duel itself); this one models how fast the carry is.
/// `dribbling*0.22 + technique*0.16 + pace*0.20 + acceleration*0.18
///  + agility*0.14 + balance*0.10`
pub fn movement_speed_with_ball(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.technical.dribbling, TECH)) * 0.22
        + n(b.apply(s.technical.technique, TECH)) * 0.16
        + n(b.apply(s.physical.pace, EXPL)) * 0.20
        + n(b.apply(s.physical.acceleration, EXPL)) * 0.18
        + n(b.apply(s.physical.agility, EXPL)) * 0.14
        + n(b.apply(s.physical.balance, TECH)) * 0.10)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Loose-ball claim composite — who wins a 50/50 race for an unowned
/// ball. Distinct from `defensive_duel` (which is challenging an
/// opponent in possession). Heavily explosive with anticipation +
/// bravery + strength weighting for the actual contact moment.
/// `acceleration*0.24 + pace*0.18 + anticipation*0.18 + bravery*0.12
///  + strength*0.10 + balance*0.08 + concentration*0.06 + decisions*0.04`
pub fn loose_ball_claim(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.physical.acceleration, EXPL)) * 0.24
        + n(b.apply(s.physical.pace, EXPL)) * 0.18
        + n(b.apply(s.mental.anticipation, MENT)) * 0.18
        + n(b.apply(s.mental.bravery, MENT)) * 0.12
        + n(b.apply(s.physical.strength, EXPL)) * 0.10
        + n(b.apply(s.physical.balance, TECH)) * 0.08
        + n(b.apply(s.mental.concentration, MENT)) * 0.06
        + n(b.apply(s.mental.decisions, MENT)) * 0.04)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Tackle-timing composite — picks WHEN to slide / step in. Heavier
/// on decisions + positioning than `defensive_duel` which weights the
/// tackle itself. Used for ball-ownership challenger ranking.
/// `tackling*0.30 + decisions*0.18 + positioning*0.14 + aggression*0.12
///  + composure*0.10 + strength*0.08 + agility*0.05 + bravery*0.03`
pub fn tackle_timing(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.technical.tackling, TECH)) * 0.30
        + n(b.apply(s.mental.decisions, MENT)) * 0.18
        + n(b.apply(s.mental.positioning, MENT)) * 0.14
        + n(b.apply(s.mental.aggression, MENT)) * 0.12
        + n(b.apply(s.mental.composure, MENT)) * 0.10
        + n(b.apply(s.physical.strength, EXPL)) * 0.08
        + n(b.apply(s.physical.agility, EXPL)) * 0.05
        + n(b.apply(s.mental.bravery, MENT)) * 0.03)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// GK rush-out composite — sweeper-keeper decision to leave the line.
/// Reads `rushing_out` as the primary skill plus the explosive burst
/// to actually arrive in time, gated by decisions / anticipation.
/// `rushing_out*0.26 + acceleration*0.18 + pace*0.12 + decisions*0.16
///  + anticipation*0.12 + bravery*0.08 + one_on_ones*0.08`
pub fn gk_rush_out(player: &MatchPlayer, minute: u32) -> f32 {
    let b = SkillBands::for_player(player, minute);
    let s = &player.skills;
    let v = (n(b.apply(s.goalkeeping.rushing_out, TECH)) * 0.26
        + n(b.apply(s.physical.acceleration, EXPL)) * 0.18
        + n(b.apply(s.physical.pace, EXPL)) * 0.12
        + n(b.apply(s.mental.decisions, MENT)) * 0.16
        + n(b.apply(s.mental.anticipation, MENT)) * 0.12
        + n(b.apply(s.mental.bravery, MENT)) * 0.08
        + n(b.apply(s.goalkeeping.one_on_ones, TECH)) * 0.08)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

// ---------------------------------------------------------------------------
// Re-exports for convenience
// ---------------------------------------------------------------------------

pub use crate::r#match::engine::player::strategies::common::players::ops::effective_skill::{
    ActionContext as EffActionContext, SkillCategory as EffSkillCategory,
};

#[allow(dead_code)]
fn _category_check() -> SkillCategory {
    SkillCategory::Technical
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerSkills;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
    };
    use chrono::NaiveDate;

    fn build_player(fill: f32, condition: i16) -> MatchPlayer {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = condition;
        attrs.jadedness = 0;
        let mut skills = PlayerSkills::default();
        skills.technical.passing = fill;
        skills.technical.technique = fill;
        skills.technical.first_touch = fill;
        skills.technical.finishing = fill;
        skills.technical.long_shots = fill;
        skills.technical.dribbling = fill;
        skills.technical.tackling = fill;
        skills.technical.marking = fill;
        skills.technical.heading = fill;
        skills.technical.crossing = fill;
        skills.technical.corners = fill;
        skills.technical.free_kicks = fill;
        skills.technical.long_throws = fill;
        skills.technical.penalty_taking = fill;
        skills.mental.vision = fill;
        skills.mental.decisions = fill;
        skills.mental.composure = fill;
        skills.mental.concentration = fill;
        skills.mental.anticipation = fill;
        skills.mental.flair = fill;
        skills.mental.positioning = fill;
        skills.mental.off_the_ball = fill;
        skills.mental.work_rate = fill;
        skills.mental.aggression = fill;
        skills.mental.bravery = fill;
        skills.mental.teamwork = fill;
        skills.mental.determination = fill;
        skills.mental.leadership = fill;
        skills.physical.balance = fill;
        skills.physical.agility = fill;
        skills.physical.acceleration = fill;
        skills.physical.pace = fill;
        skills.physical.strength = fill;
        skills.physical.jumping = fill;
        skills.physical.stamina = fill;
        skills.physical.natural_fitness = fill;
        skills.physical.match_readiness = fill;
        skills.goalkeeping.reflexes = fill;
        skills.goalkeeping.handling = fill;
        skills.goalkeeping.one_on_ones = fill;
        skills.goalkeeping.aerial_reach = fill;
        skills.goalkeeping.command_of_area = fill;
        skills.goalkeeping.communication = fill;
        skills.goalkeeping.kicking = fill;
        skills.goalkeeping.throwing = fill;
        skills.goalkeeping.passing = fill;
        skills.goalkeeping.first_touch = fill;
        skills.goalkeeping.rushing_out = fill;
        skills.goalkeeping.punching = fill;
        let player = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(1, &player, PlayerPositionType::MidfielderCenter, false)
    }

    #[test]
    fn n_clamps() {
        assert_eq!(n(-3.0), 0.0);
        assert_eq!(n(0.0), 0.0);
        assert!((n(10.0) - 0.5).abs() < 1e-6);
        assert!((n(20.0) - 1.0).abs() < 1e-6);
        assert_eq!(n(40.0), 1.0);
    }

    #[test]
    fn passing_execution_monotonic_in_skill() {
        let poor = build_player(6.0, 9000);
        let avg = build_player(11.0, 9000);
        let elite = build_player(18.0, 9000);
        let p = passing_execution(&poor, 30);
        let a = passing_execution(&avg, 30);
        let e = passing_execution(&elite, 30);
        assert!(p < a, "poor {p} >= avg {a}");
        assert!(a < e, "avg {a} >= elite {e}");
    }

    #[test]
    fn shooting_close_monotonic_and_bounded() {
        let poor = build_player(6.0, 9000);
        let elite = build_player(19.0, 9000);
        let p = shooting_close(&poor, 30);
        let e = shooting_close(&elite, 30);
        assert!(p >= COMPOSITE_FLOOR);
        assert!(e <= COMPOSITE_CEIL);
        assert!(p < e);
    }

    #[test]
    fn long_shot_loads_long_shots_attribute_more_than_close() {
        // Build a player with 17 long_shots / 7 finishing vs 7 long_shots
        // / 17 finishing. The `long_shot` composite must rank the first
        // higher; `shooting_close` must rank the second higher.
        let mut a = build_player(7.0, 9000);
        let mut b = build_player(7.0, 9000);
        a.skills.technical.long_shots = 17.0;
        a.skills.technical.technique = 14.0;
        b.skills.technical.finishing = 17.0;
        b.skills.mental.composure = 14.0;
        let a_long = long_shot(&a, 30);
        let b_long = long_shot(&b, 30);
        let a_close = shooting_close(&a, 30);
        let b_close = shooting_close(&b, 30);
        assert!(
            a_long > b_long,
            "long_shots should dominate long_shot composite"
        );
        assert!(
            b_close > a_close,
            "finishing should dominate shooting_close"
        );
    }

    #[test]
    fn fatigue_drops_explosive_more_than_technical() {
        // Same skill sheet, two condition levels.
        let fresh = build_player(15.0, 9500);
        let tired = build_player(15.0, 2500);
        let press_fresh = pressing(&fresh, 80);
        let press_tired = pressing(&tired, 80);
        let pass_fresh = passing_execution(&fresh, 80);
        let pass_tired = passing_execution(&tired, 80);
        // Pressing is heavily explosive — should fall harder than passing.
        assert!(press_fresh > press_tired);
        assert!(pass_fresh > pass_tired);
        let press_drop = press_fresh - press_tired;
        let pass_drop = pass_fresh - pass_tired;
        assert!(
            press_drop > pass_drop,
            "press_drop={press_drop} should exceed pass_drop={pass_drop}"
        );
    }

    #[test]
    fn defensive_duel_loads_marking_and_positioning() {
        let mut clean = build_player(10.0, 9000);
        clean.skills.technical.marking = 18.0;
        clean.skills.mental.positioning = 18.0;
        clean.skills.mental.concentration = 16.0;
        let mut sloppy = build_player(10.0, 9000);
        sloppy.skills.technical.marking = 6.0;
        sloppy.skills.mental.positioning = 6.0;
        sloppy.skills.mental.concentration = 6.0;
        assert!(defensive_duel(&clean, 30) > defensive_duel(&sloppy, 30));
    }

    #[test]
    fn gk_shot_stopping_loads_reflexes_and_handling() {
        let mut elite_gk = build_player(10.0, 9000);
        elite_gk.skills.goalkeeping.reflexes = 19.0;
        elite_gk.skills.goalkeeping.handling = 18.0;
        elite_gk.skills.physical.agility = 17.0;
        let weak_gk = build_player(6.0, 9000);
        assert!(gk_shot_stopping(&elite_gk, 30) > gk_shot_stopping(&weak_gk, 30));
    }

    #[test]
    fn gk_aerial_loads_command_and_aerial_reach() {
        let mut elite = build_player(8.0, 9000);
        elite.skills.goalkeeping.aerial_reach = 18.0;
        elite.skills.goalkeeping.command_of_area = 17.0;
        elite.skills.goalkeeping.communication = 16.0;
        elite.skills.goalkeeping.handling = 16.0;
        let weak = build_player(8.0, 9000);
        assert!(gk_aerial(&elite, 30) > gk_aerial(&weak, 30) + 0.10);
    }

    #[test]
    fn gk_distribution_loads_passing_and_kicking() {
        let mut elite = build_player(8.0, 9000);
        elite.skills.goalkeeping.passing = 17.0;
        elite.skills.goalkeeping.kicking = 17.0;
        elite.skills.goalkeeping.throwing = 16.0;
        let weak = build_player(8.0, 9000);
        assert!(gk_distribution(&elite, 30) > gk_distribution(&weak, 30) + 0.05);
    }

    #[test]
    fn aerial_attacker_uses_off_the_ball_not_positioning() {
        let mut a = build_player(10.0, 9000);
        a.skills.mental.positioning = 18.0;
        a.skills.mental.off_the_ball = 4.0;
        let mut b = build_player(10.0, 9000);
        b.skills.mental.positioning = 4.0;
        b.skills.mental.off_the_ball = 18.0;
        // Attacker reads `off_the_ball`, so b should outperform a.
        assert!(aerial_outfield_attacker(&b, 30) > aerial_outfield_attacker(&a, 30));
        // Defender reads `positioning`, so a should outperform b.
        assert!(aerial_outfield_defender(&a, 30) > aerial_outfield_defender(&b, 30));
    }

    #[test]
    fn readiness_factor_in_band() {
        let mut p = build_player(15.0, 9000);
        p.skills.physical.match_readiness = 0.0;
        let low = readiness_factor(&p);
        p.skills.physical.match_readiness = 20.0;
        let high = readiness_factor(&p);
        assert!((low - 0.85).abs() < 1e-4);
        assert!((high - 1.00).abs() < 1e-4);
    }

    #[test]
    fn minute_from_ms_works() {
        assert_eq!(minute_from_ms(0), 0);
        assert_eq!(minute_from_ms(60_000), 1);
        assert_eq!(minute_from_ms(2_700_000), 45);
    }

    #[test]
    fn minute_from_ticks_agrees_with_minute_from_ms() {
        // The two helpers must report the same minute for matched
        // inputs (1 tick == 10ms == 1/6000 of a minute). Without this
        // tie ball-side and engine-side composite calls would drift.
        for minute in [0u64, 1, 7, 22, 45, 67, 89, 90, 105].iter().copied() {
            let ms = minute * 60_000;
            let ticks = ms / 10;
            assert_eq!(minute_from_ms(ms), minute_from_ticks(ticks));
        }
    }

    // ── Selection / decision composites ────────────────────────────

    #[test]
    fn off_ball_attack_loads_off_the_ball() {
        let mut a = build_player(8.0, 9000);
        a.skills.mental.off_the_ball = 18.0;
        a.skills.mental.anticipation = 16.0;
        let b = build_player(8.0, 9000);
        let high = off_ball_attack(&a, 30);
        let low = off_ball_attack(&b, 30);
        assert!(high > low + 0.05);
        assert!(high <= 1.0 && low >= 0.05);
    }

    #[test]
    fn shot_selection_high_decisions_outranks_pure_finisher() {
        // Equal finishing, decisions/composure separates the two.
        let mut poacher = build_player(10.0, 9000);
        poacher.skills.technical.finishing = 17.0;
        poacher.skills.mental.decisions = 8.0;
        poacher.skills.mental.composure = 8.0;
        poacher.skills.mental.vision = 8.0;
        let mut chooser = build_player(10.0, 9000);
        chooser.skills.technical.finishing = 17.0;
        chooser.skills.mental.decisions = 17.0;
        chooser.skills.mental.composure = 16.0;
        chooser.skills.mental.vision = 14.0;
        assert!(shot_selection(&chooser, 30) > shot_selection(&poacher, 30));
    }

    #[test]
    fn pass_selection_loads_decisions_and_vision() {
        let mut weak = build_player(10.0, 9000);
        weak.skills.mental.decisions = 6.0;
        weak.skills.mental.vision = 6.0;
        let mut strong = build_player(10.0, 9000);
        strong.skills.mental.decisions = 18.0;
        strong.skills.mental.vision = 18.0;
        assert!(pass_selection(&strong, 30) > pass_selection(&weak, 30) + 0.10);
    }

    #[test]
    fn defensive_positioning_monotonic() {
        let poor = build_player(6.0, 9000);
        let avg = build_player(11.0, 9000);
        let elite = build_player(18.0, 9000);
        assert!(defensive_positioning(&poor, 30) < defensive_positioning(&avg, 30));
        assert!(defensive_positioning(&avg, 30) < defensive_positioning(&elite, 30));
    }

    #[test]
    fn gk_communication_loads_communication_and_command() {
        let mut elite = build_player(8.0, 9000);
        elite.skills.goalkeeping.communication = 18.0;
        elite.skills.goalkeeping.command_of_area = 17.0;
        elite.skills.mental.leadership = 15.0;
        let weak = build_player(8.0, 9000);
        assert!(gk_communication(&elite, 30) > gk_communication(&weak, 30) + 0.10);
    }

    #[test]
    fn gk_claim_cross_loads_aerial_reach_and_command() {
        let mut elite = build_player(8.0, 9000);
        elite.skills.goalkeeping.aerial_reach = 18.0;
        elite.skills.goalkeeping.command_of_area = 17.0;
        elite.skills.goalkeeping.handling = 15.0;
        elite.skills.mental.positioning = 14.0;
        let weak = build_player(8.0, 9000);
        assert!(gk_claim_cross(&elite, 30) > gk_claim_cross(&weak, 30) + 0.05);
    }

    // ── Weight-sum guard ───────────────────────────────────────────
    //
    // Every composite blends weighted normalised attribute reads. If
    // any new composite's weights drift away from 1.0, the output
    // band silently shifts (a 1.05-summing composite caps out below
    // the COMPOSITE_CEIL on max-skill players, a 0.95-summing one
    // never reaches it). Pin a saturated player to within rounding
    // of `COMPOSITE_CEIL` so missed-by-0.01 bugs surface immediately.
    #[test]
    fn new_composite_weights_sum_to_one() {
        let max = build_player(20.0, 9000);
        let m = 30u32;
        let composites: Vec<(&str, f32)> = vec![
            ("mobility", mobility(&max, m)),
            ("decision_quality", decision_quality(&max, m)),
            (
                "movement_speed_with_ball",
                movement_speed_with_ball(&max, m),
            ),
            ("loose_ball_claim", loose_ball_claim(&max, m)),
            ("tackle_timing", tackle_timing(&max, m)),
            ("gk_rush_out", gk_rush_out(&max, m)),
        ];
        for (name, value) in composites {
            assert!(
                (value - COMPOSITE_CEIL).abs() < 1e-3,
                "{name} max-skill output {value} should equal {COMPOSITE_CEIL} \
                 — weights likely don't sum to 1.0"
            );
        }
    }

    // ── New composites + cross-composite ranking ───────────────────

    #[test]
    fn mobility_loads_pace_and_acceleration() {
        let mut a = build_player(8.0, 9000);
        a.skills.physical.pace = 18.0;
        a.skills.physical.acceleration = 18.0;
        a.skills.physical.agility = 16.0;
        let weak = build_player(8.0, 9000);
        assert!(mobility(&a, 30) > mobility(&weak, 30) + 0.10);
    }

    #[test]
    fn loose_ball_claim_ranks_acceleration_and_anticipation() {
        let mut fast = build_player(8.0, 9000);
        fast.skills.physical.acceleration = 18.0;
        fast.skills.physical.pace = 17.0;
        fast.skills.mental.anticipation = 17.0;
        let slow = build_player(8.0, 9000);
        assert!(loose_ball_claim(&fast, 30) > loose_ball_claim(&slow, 30) + 0.10);
    }

    #[test]
    fn tackle_timing_loads_tackling_and_decisions() {
        let mut elite = build_player(8.0, 9000);
        elite.skills.technical.tackling = 18.0;
        elite.skills.mental.decisions = 17.0;
        elite.skills.mental.positioning = 16.0;
        let weak = build_player(8.0, 9000);
        assert!(tackle_timing(&elite, 30) > tackle_timing(&weak, 30) + 0.10);
    }

    #[test]
    fn movement_speed_with_ball_loads_dribbling_and_pace() {
        let mut elite = build_player(8.0, 9000);
        elite.skills.technical.dribbling = 18.0;
        elite.skills.technical.technique = 16.0;
        elite.skills.physical.pace = 17.0;
        elite.skills.physical.acceleration = 17.0;
        let weak = build_player(8.0, 9000);
        assert!(movement_speed_with_ball(&elite, 30) > movement_speed_with_ball(&weak, 30) + 0.10);
    }

    #[test]
    fn decision_quality_loads_decisions_composure_concentration() {
        let mut elite = build_player(8.0, 9000);
        elite.skills.mental.decisions = 18.0;
        elite.skills.mental.composure = 17.0;
        elite.skills.mental.concentration = 16.0;
        let weak = build_player(8.0, 9000);
        assert!(decision_quality(&elite, 30) > decision_quality(&weak, 30) + 0.10);
    }

    #[test]
    fn gk_rush_out_loads_rushing_out_and_speed() {
        let mut elite = build_player(8.0, 9000);
        elite.skills.goalkeeping.rushing_out = 18.0;
        elite.skills.physical.acceleration = 17.0;
        elite.skills.mental.decisions = 16.0;
        elite.skills.mental.anticipation = 15.0;
        let weak = build_player(8.0, 9000);
        assert!(gk_rush_out(&elite, 30) > gk_rush_out(&weak, 30) + 0.10);
    }

    #[test]
    fn explosive_drops_more_than_technical_under_fatigue() {
        // Same skill sheet, low condition. The mobility composite is
        // entirely explosive-banded and should drop more than
        // passing_execution (predominantly technical/mental) in the
        // same player.
        let fresh = build_player(15.0, 9500);
        let tired = build_player(15.0, 2500);
        let mob_drop = mobility(&fresh, 80) - mobility(&tired, 80);
        let pass_drop = passing_execution(&fresh, 80) - passing_execution(&tired, 80);
        assert!(mob_drop > 0.0);
        assert!(pass_drop > 0.0);
        assert!(
            mob_drop > pass_drop,
            "mobility drop {mob_drop} should exceed passing drop {pass_drop}"
        );
    }

    #[test]
    fn elite_stamina_preserves_late_game_skill() {
        let mut elite_fit = build_player(15.0, 3500);
        elite_fit.skills.physical.stamina = 19.0;
        elite_fit.skills.physical.natural_fitness = 18.0;
        let mut poor_fit = build_player(15.0, 3500);
        poor_fit.skills.physical.stamina = 8.0;
        poor_fit.skills.physical.natural_fitness = 8.0;
        // Same base skill, same condition — the elite-fit player
        // should retain more of their passing execution at minute 85.
        let elite = passing_execution(&elite_fit, 85);
        let poor = passing_execution(&poor_fit, 85);
        assert!(elite > poor);
    }

    #[test]
    fn elite_dribbler_outranks_poor_dribbler_under_same_conditions() {
        let mut elite = build_player(8.0, 9000);
        elite.skills.technical.dribbling = 19.0;
        elite.skills.technical.technique = 18.0;
        elite.skills.physical.agility = 17.0;
        elite.skills.physical.acceleration = 17.0;
        let poor = build_player(8.0, 9000);
        assert!(dribble_attack(&elite, 30) > dribble_attack(&poor, 30) + 0.15);
    }

    #[test]
    fn elite_defender_outranks_poor_defender_in_duel_and_claim() {
        let mut elite = build_player(8.0, 9000);
        elite.skills.technical.tackling = 18.0;
        elite.skills.technical.marking = 17.0;
        elite.skills.mental.positioning = 17.0;
        elite.skills.mental.anticipation = 16.0;
        elite.skills.physical.strength = 16.0;
        elite.skills.physical.acceleration = 16.0;
        let poor = build_player(8.0, 9000);
        assert!(defensive_duel(&elite, 30) > defensive_duel(&poor, 30) + 0.15);
        assert!(interception(&elite, 30) > interception(&poor, 30) + 0.10);
        assert!(tackle_timing(&elite, 30) > tackle_timing(&poor, 30) + 0.10);
    }

    #[test]
    fn new_composites_monotonic_across_skill_band() {
        // Sweep poor/avg/elite for every new composite — confirms each
        // is strictly increasing in skill (no spurious mid-curve dips
        // from a botched weight).
        let poor = build_player(6.0, 9000);
        let avg = build_player(11.0, 9000);
        let elite = build_player(18.0, 9000);
        let m = 30u32;
        let pairs: &[(&str, fn(&MatchPlayer, u32) -> f32)] = &[
            ("mobility", mobility),
            ("decision_quality", decision_quality),
            ("movement_speed_with_ball", movement_speed_with_ball),
            ("loose_ball_claim", loose_ball_claim),
            ("tackle_timing", tackle_timing),
            ("gk_rush_out", gk_rush_out),
        ];
        for (name, f) in pairs {
            let p = f(&poor, m);
            let a = f(&avg, m);
            let e = f(&elite, m);
            assert!(p < a, "{name}: poor {p} >= avg {a}");
            assert!(a < e, "{name}: avg {a} >= elite {e}");
        }
    }

    #[test]
    fn tackle_timing_favours_smart_over_pure_aggression() {
        // A "smart" defender (decisions + positioning + composure) must
        // outscore a "pure brawler" (aggression + strength only) on
        // tackle_timing — the spec's whole point is that snapping into
        // tackles late is worse than reading the play.
        let mut smart = build_player(8.0, 9000);
        smart.skills.technical.tackling = 14.0;
        smart.skills.mental.decisions = 18.0;
        smart.skills.mental.positioning = 17.0;
        smart.skills.mental.composure = 16.0;
        let mut brawler = build_player(8.0, 9000);
        brawler.skills.technical.tackling = 14.0;
        brawler.skills.mental.aggression = 20.0;
        brawler.skills.physical.strength = 19.0;
        brawler.skills.mental.bravery = 18.0;
        // Same tackling skill, but the smart defender's reading slot
        // (decisions 0.18 + positioning 0.14 + composure 0.10 = 0.42 of
        // weight) outweighs the brawler's slot (aggression 0.12 +
        // strength 0.08 + bravery 0.03 = 0.23). The composite must
        // reflect that.
        assert!(
            tackle_timing(&smart, 30) > tackle_timing(&brawler, 30),
            "smart {} should beat brawler {} on tackle_timing",
            tackle_timing(&smart, 30),
            tackle_timing(&brawler, 30),
        );
    }

    #[test]
    fn loose_ball_claim_favours_fast_anticipator_over_strong_slow() {
        // A fast/anticipating player (low strength) should beat a
        // strong/slow player on loose-ball races. Acceleration 0.24 +
        // pace 0.18 + anticipation 0.18 = 0.60 of weight outweighs
        // strength 0.10 + bravery 0.12 = 0.22.
        let mut sprinter = build_player(8.0, 9000);
        sprinter.skills.physical.acceleration = 19.0;
        sprinter.skills.physical.pace = 18.0;
        sprinter.skills.mental.anticipation = 17.0;
        let mut bruiser = build_player(8.0, 9000);
        bruiser.skills.physical.strength = 19.0;
        bruiser.skills.mental.bravery = 17.0;
        assert!(
            loose_ball_claim(&sprinter, 30) > loose_ball_claim(&bruiser, 30),
            "sprinter {} should beat bruiser {} on loose-ball claim",
            loose_ball_claim(&sprinter, 30),
            loose_ball_claim(&bruiser, 30),
        );
    }

    #[test]
    fn movement_speed_with_ball_does_not_create_extreme_speeds() {
        // The spec maps the composite to `0.78 + composite * 0.42` and
        // then expects that band to land near 0.80..1.20. Verify the
        // composite itself stays inside [0.05, 1.0] so the downstream
        // multiplier never goes outside the calibrated band.
        let m = 30u32;
        // Saturated max-skill player
        let max = build_player(20.0, 9000);
        let v_max = movement_speed_with_ball(&max, m);
        assert!(v_max <= 1.0, "max v = {v_max}");
        // Worst player at low condition still bounded above the floor
        let mut worst = build_player(1.0, 1500);
        worst.skills.physical.stamina = 1.0;
        worst.skills.physical.natural_fitness = 1.0;
        let v_min = movement_speed_with_ball(&worst, m);
        assert!(v_min >= COMPOSITE_FLOOR, "min v = {v_min}");
        // Translate to the carry-multiplier band the engine actually uses.
        let mult_max = (0.78 + v_max * 0.42).clamp(0.75, 1.00);
        let mult_min = (0.78 + v_min * 0.42).clamp(0.75, 1.00);
        // Final multiplier never exceeds 1.0 (no carrier *gains* speed)
        // and never falls below 0.75 (the carry-cost ceiling).
        assert!(mult_max <= 1.0 + 1e-6);
        assert!(mult_min >= 0.75 - 1e-6);
    }

    #[test]
    fn tired_elite_can_still_outperform_fresh_poor_when_gap_is_large() {
        // Skill gap 18 vs 6 should beat a 15% fatigue penalty.
        let mut tired_elite = build_player(18.0, 3500); // ~35% condition
        tired_elite.skills.physical.stamina = 12.0;
        tired_elite.skills.physical.natural_fitness = 12.0;
        let fresh_poor = build_player(6.0, 9500);
        assert!(passing_execution(&tired_elite, 80) > passing_execution(&fresh_poor, 80));
        assert!(dribble_attack(&tired_elite, 80) > dribble_attack(&fresh_poor, 80));
    }

    // ── Bounded-output sanity for every composite ──────────────────

    #[test]
    fn all_composites_stay_bounded_for_min_and_max_skills() {
        let zero = build_player(1.0, 9000);
        let max = build_player(20.0, 9000);
        let m = 30u32;
        let cases: Vec<(&str, f32, f32)> = vec![
            (
                "passing_execution",
                passing_execution(&zero, m),
                passing_execution(&max, m),
            ),
            (
                "long_passing",
                long_passing(&zero, m),
                long_passing(&max, m),
            ),
            (
                "receiving_first_touch",
                receiving_first_touch(&zero, m),
                receiving_first_touch(&max, m),
            ),
            (
                "shooting_close",
                shooting_close(&zero, m),
                shooting_close(&max, m),
            ),
            (
                "shooting_medium",
                shooting_medium(&zero, m),
                shooting_medium(&max, m),
            ),
            ("long_shot", long_shot(&zero, m), long_shot(&max, m)),
            (
                "dribble_attack",
                dribble_attack(&zero, m),
                dribble_attack(&max, m),
            ),
            (
                "defensive_duel",
                defensive_duel(&zero, m),
                defensive_duel(&max, m),
            ),
            (
                "interception",
                interception(&zero, m),
                interception(&max, m),
            ),
            ("pressing", pressing(&zero, m), pressing(&max, m)),
            (
                "aerial_outfield_defender",
                aerial_outfield_defender(&zero, m),
                aerial_outfield_defender(&max, m),
            ),
            (
                "aerial_outfield_attacker",
                aerial_outfield_attacker(&zero, m),
                aerial_outfield_attacker(&max, m),
            ),
            (
                "off_ball_attack",
                off_ball_attack(&zero, m),
                off_ball_attack(&max, m),
            ),
            (
                "shot_selection",
                shot_selection(&zero, m),
                shot_selection(&max, m),
            ),
            (
                "pass_selection",
                pass_selection(&zero, m),
                pass_selection(&max, m),
            ),
            (
                "defensive_positioning",
                defensive_positioning(&zero, m),
                defensive_positioning(&max, m),
            ),
            (
                "gk_shot_stopping",
                gk_shot_stopping(&zero, m),
                gk_shot_stopping(&max, m),
            ),
            ("gk_aerial", gk_aerial(&zero, m), gk_aerial(&max, m)),
            (
                "gk_communication",
                gk_communication(&zero, m),
                gk_communication(&max, m),
            ),
            (
                "gk_claim_cross",
                gk_claim_cross(&zero, m),
                gk_claim_cross(&max, m),
            ),
            (
                "gk_distribution",
                gk_distribution(&zero, m),
                gk_distribution(&max, m),
            ),
            ("mobility", mobility(&zero, m), mobility(&max, m)),
            (
                "decision_quality",
                decision_quality(&zero, m),
                decision_quality(&max, m),
            ),
            (
                "movement_speed_with_ball",
                movement_speed_with_ball(&zero, m),
                movement_speed_with_ball(&max, m),
            ),
            (
                "loose_ball_claim",
                loose_ball_claim(&zero, m),
                loose_ball_claim(&max, m),
            ),
            (
                "tackle_timing",
                tackle_timing(&zero, m),
                tackle_timing(&max, m),
            ),
            ("gk_rush_out", gk_rush_out(&zero, m), gk_rush_out(&max, m)),
        ];
        for (name, lo, hi) in cases {
            assert!(
                lo >= COMPOSITE_FLOOR && lo <= COMPOSITE_CEIL,
                "{name} lo={lo} out of bounds"
            );
            assert!(
                hi >= COMPOSITE_FLOOR && hi <= COMPOSITE_CEIL,
                "{name} hi={hi} out of bounds"
            );
            assert!(hi > lo, "{name} hi={hi} <= lo={lo}");
        }
    }
}
