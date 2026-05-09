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
    ActionContext, SkillCategory, effective_skill,
};

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

/// Convert a match minute (0..120) to the three `ActionContext`s the
/// composites need.
#[inline]
fn ctxs(minute: u32) -> (ActionContext, ActionContext, ActionContext) {
    (
        ActionContext::technical(minute),
        ActionContext::mental(minute),
        ActionContext::explosive(minute),
    )
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
    let (tech, mental, _) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, tech, |_| s.technical.passing)) * 0.38
        + n(eff(player, tech, |_| s.technical.technique)) * 0.20
        + n(eff(player, mental, |_| s.mental.vision)) * 0.16
        + n(eff(player, mental, |_| s.mental.decisions)) * 0.10
        + n(eff(player, mental, |_| s.mental.composure)) * 0.08
        + n(eff(player, mental, |_| s.mental.concentration)) * 0.08)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Long pass / switch composite.
/// `passing*0.30 + vision*0.24 + technique*0.20 + decisions*0.10
///  + flair*0.06 + balance*0.04 + composure*0.06`
pub fn long_passing(player: &MatchPlayer, minute: u32) -> f32 {
    let (tech, mental, _) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, tech, |_| s.technical.passing)) * 0.30
        + n(eff(player, mental, |_| s.mental.vision)) * 0.24
        + n(eff(player, tech, |_| s.technical.technique)) * 0.20
        + n(eff(player, mental, |_| s.mental.decisions)) * 0.10
        + n(eff(player, mental, |_| s.mental.flair)) * 0.06
        + n(eff(player, tech, |_| s.physical.balance)) * 0.04
        + n(eff(player, mental, |_| s.mental.composure)) * 0.06)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Receiving / first-touch composite.
/// `first_touch*0.28 + technique*0.18 + composure*0.14 + anticipation*0.12
///  + balance*0.10 + agility*0.08 + decisions*0.06 + concentration*0.04`
pub fn receiving_first_touch(player: &MatchPlayer, minute: u32) -> f32 {
    let (tech, mental, expl) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, tech, |_| s.technical.first_touch)) * 0.28
        + n(eff(player, tech, |_| s.technical.technique)) * 0.18
        + n(eff(player, mental, |_| s.mental.composure)) * 0.14
        + n(eff(player, mental, |_| s.mental.anticipation)) * 0.12
        + n(eff(player, tech, |_| s.physical.balance)) * 0.10
        + n(eff(player, expl, |_| s.physical.agility)) * 0.08
        + n(eff(player, mental, |_| s.mental.decisions)) * 0.06
        + n(eff(player, mental, |_| s.mental.concentration)) * 0.04)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

// ---------------------------------------------------------------------------
// Shooting
// ---------------------------------------------------------------------------

/// Close-range shooting composite.
/// `finishing*0.34 + composure*0.22 + first_touch*0.14 + technique*0.12
///  + decisions*0.10 + balance*0.05 + concentration*0.03`
pub fn shooting_close(player: &MatchPlayer, minute: u32) -> f32 {
    let (tech, mental, _) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, tech, |_| s.technical.finishing)) * 0.34
        + n(eff(player, mental, |_| s.mental.composure)) * 0.22
        + n(eff(player, tech, |_| s.technical.first_touch)) * 0.14
        + n(eff(player, tech, |_| s.technical.technique)) * 0.12
        + n(eff(player, mental, |_| s.mental.decisions)) * 0.10
        + n(eff(player, tech, |_| s.physical.balance)) * 0.05
        + n(eff(player, mental, |_| s.mental.concentration)) * 0.03)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Medium-range shooting composite.
/// `finishing*0.28 + technique*0.20 + composure*0.17 + decisions*0.13
///  + long_shots*0.12 + first_touch*0.05 + balance*0.05`
pub fn shooting_medium(player: &MatchPlayer, minute: u32) -> f32 {
    let (tech, mental, _) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, tech, |_| s.technical.finishing)) * 0.28
        + n(eff(player, tech, |_| s.technical.technique)) * 0.20
        + n(eff(player, mental, |_| s.mental.composure)) * 0.17
        + n(eff(player, mental, |_| s.mental.decisions)) * 0.13
        + n(eff(player, tech, |_| s.technical.long_shots)) * 0.12
        + n(eff(player, tech, |_| s.technical.first_touch)) * 0.05
        + n(eff(player, tech, |_| s.physical.balance)) * 0.05)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Long-shot composite.
/// `long_shots*0.34 + technique*0.24 + composure*0.14 + decisions*0.12
///  + finishing*0.08 + strength*0.04 + balance*0.04`
pub fn long_shot(player: &MatchPlayer, minute: u32) -> f32 {
    let (tech, mental, expl) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, tech, |_| s.technical.long_shots)) * 0.34
        + n(eff(player, tech, |_| s.technical.technique)) * 0.24
        + n(eff(player, mental, |_| s.mental.composure)) * 0.14
        + n(eff(player, mental, |_| s.mental.decisions)) * 0.12
        + n(eff(player, tech, |_| s.technical.finishing)) * 0.08
        + n(eff(player, expl, |_| s.physical.strength)) * 0.04
        + n(eff(player, tech, |_| s.physical.balance)) * 0.04)
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
    let (tech, mental, expl) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, tech, |_| s.technical.dribbling)) * 0.25
        + n(eff(player, tech, |_| s.technical.technique)) * 0.17
        + n(eff(player, mental, |_| s.mental.flair)) * 0.10
        + n(eff(player, expl, |_| s.physical.agility)) * 0.14
        + n(eff(player, expl, |_| s.physical.acceleration)) * 0.10
        + n(eff(player, tech, |_| s.physical.balance)) * 0.09
        + n(eff(player, mental, |_| s.mental.composure)) * 0.07
        + n(eff(player, mental, |_| s.mental.decisions)) * 0.05
        + n(eff(player, expl, |_| s.physical.strength)) * 0.03)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Defensive duel composite (defender side of a 1v1).
/// `tackling*0.24 + positioning*0.17 + anticipation*0.15 + marking*0.13
///  + strength*0.10 + balance*0.07 + agility*0.06 + concentration*0.05
///  + bravery*0.03`
pub fn defensive_duel(player: &MatchPlayer, minute: u32) -> f32 {
    let (tech, mental, expl) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, tech, |_| s.technical.tackling)) * 0.24
        + n(eff(player, mental, |_| s.mental.positioning)) * 0.17
        + n(eff(player, mental, |_| s.mental.anticipation)) * 0.15
        + n(eff(player, tech, |_| s.technical.marking)) * 0.13
        + n(eff(player, expl, |_| s.physical.strength)) * 0.10
        + n(eff(player, tech, |_| s.physical.balance)) * 0.07
        + n(eff(player, expl, |_| s.physical.agility)) * 0.06
        + n(eff(player, mental, |_| s.mental.concentration)) * 0.05
        + n(eff(player, mental, |_| s.mental.bravery)) * 0.03)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Interception composite.
/// `anticipation*0.24 + positioning*0.20 + concentration*0.16 + acceleration*0.12
///  + pace*0.10 + marking*0.08 + decisions*0.06 + agility*0.04`
pub fn interception(player: &MatchPlayer, minute: u32) -> f32 {
    let (tech, mental, expl) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, mental, |_| s.mental.anticipation)) * 0.24
        + n(eff(player, mental, |_| s.mental.positioning)) * 0.20
        + n(eff(player, mental, |_| s.mental.concentration)) * 0.16
        + n(eff(player, expl, |_| s.physical.acceleration)) * 0.12
        + n(eff(player, expl, |_| s.physical.pace)) * 0.10
        + n(eff(player, tech, |_| s.technical.marking)) * 0.08
        + n(eff(player, mental, |_| s.mental.decisions)) * 0.06
        + n(eff(player, expl, |_| s.physical.agility)) * 0.04)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Pressing composite.
/// `work_rate*0.24 + stamina*0.18 + aggression*0.14 + acceleration*0.12
///  + pace*0.10 + decisions*0.08 + teamwork*0.08 + concentration*0.06`
pub fn pressing(player: &MatchPlayer, minute: u32) -> f32 {
    let (_, mental, expl) = ctxs(minute);
    // `stamina` is the fatigue mitigator itself, so reading it through
    // the explosive band still bumps it via the band when condition is
    // low — that's the desired direction (a tired player presses less).
    let s = &player.skills;
    let v = (n(eff(player, mental, |_| s.mental.work_rate)) * 0.24
        + n(eff(player, expl, |_| s.physical.stamina)) * 0.18
        + n(eff(player, mental, |_| s.mental.aggression)) * 0.14
        + n(eff(player, expl, |_| s.physical.acceleration)) * 0.12
        + n(eff(player, expl, |_| s.physical.pace)) * 0.10
        + n(eff(player, mental, |_| s.mental.decisions)) * 0.08
        + n(eff(player, mental, |_| s.mental.teamwork)) * 0.08
        + n(eff(player, mental, |_| s.mental.concentration)) * 0.06)
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
    let (_, mental, expl) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, mental, |_| s.mental.off_the_ball)) * 0.35
        + n(eff(player, mental, |_| s.mental.anticipation)) * 0.20
        + n(eff(player, mental, |_| s.mental.decisions)) * 0.15
        + n(eff(player, expl, |_| s.physical.acceleration)) * 0.10
        + n(eff(player, expl, |_| s.physical.pace)) * 0.08
        + n(eff(player, mental, |_| s.mental.teamwork)) * 0.07
        + n(eff(player, mental, |_| s.mental.bravery)) * 0.05)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Shot-selection composite — should the player shoot or pass / hold?
/// `composure*0.25 + decisions*0.25 + finishing*0.18 + long_shots*0.12
///  + vision*0.10 + teamwork*0.10`
pub fn shot_selection(player: &MatchPlayer, minute: u32) -> f32 {
    let (tech, mental, _) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, mental, |_| s.mental.composure)) * 0.25
        + n(eff(player, mental, |_| s.mental.decisions)) * 0.25
        + n(eff(player, tech, |_| s.technical.finishing)) * 0.18
        + n(eff(player, tech, |_| s.technical.long_shots)) * 0.12
        + n(eff(player, mental, |_| s.mental.vision)) * 0.10
        + n(eff(player, mental, |_| s.mental.teamwork)) * 0.10)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Pass-selection composite — picking the right ball, not executing it.
/// `decisions*0.25 + vision*0.25 + passing*0.18 + composure*0.12
///  + teamwork*0.12 + flair*0.08`
pub fn pass_selection(player: &MatchPlayer, minute: u32) -> f32 {
    let (tech, mental, _) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, mental, |_| s.mental.decisions)) * 0.25
        + n(eff(player, mental, |_| s.mental.vision)) * 0.25
        + n(eff(player, tech, |_| s.technical.passing)) * 0.18
        + n(eff(player, mental, |_| s.mental.composure)) * 0.12
        + n(eff(player, mental, |_| s.mental.teamwork)) * 0.12
        + n(eff(player, mental, |_| s.mental.flair)) * 0.08)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// Defensive positioning composite — reading the play without the ball.
/// `positioning*0.30 + anticipation*0.22 + concentration*0.18
///  + decisions*0.12 + teamwork*0.10 + acceleration*0.08`
pub fn defensive_positioning(player: &MatchPlayer, minute: u32) -> f32 {
    let (_, mental, expl) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, mental, |_| s.mental.positioning)) * 0.30
        + n(eff(player, mental, |_| s.mental.anticipation)) * 0.22
        + n(eff(player, mental, |_| s.mental.concentration)) * 0.18
        + n(eff(player, mental, |_| s.mental.decisions)) * 0.12
        + n(eff(player, mental, |_| s.mental.teamwork)) * 0.10
        + n(eff(player, expl, |_| s.physical.acceleration)) * 0.08)
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
    let (tech, mental, expl) = ctxs(minute);
    let s = &player.skills;
    let positioning_or_otb = if defender {
        n(eff(player, mental, |_| s.mental.positioning))
    } else {
        n(eff(player, mental, |_| s.mental.off_the_ball))
    };
    let v = (n(eff(player, tech, |_| s.technical.heading)) * 0.28
        + n(eff(player, expl, |_| s.physical.jumping)) * 0.24
        + n(eff(player, expl, |_| s.physical.strength)) * 0.16
        + n(eff(player, mental, |_| s.mental.bravery)) * 0.12
        + positioning_or_otb * 0.10
        + n(eff(player, mental, |_| s.mental.anticipation)) * 0.06
        + n(eff(player, tech, |_| s.physical.balance)) * 0.04)
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
    let (_, mental, expl) = ctxs(minute);
    let s = &player.skills;
    // GK skills are technical-feel (reflexes/handling/one_on_ones) so we
    // route them through the technical category which has the smallest
    // fatigue penalty.
    let tech = ActionContext::technical(minute);
    let v = (n(eff(player, tech, |_| s.goalkeeping.reflexes)) * 0.30
        + n(eff(player, tech, |_| s.goalkeeping.handling)) * 0.18
        + n(eff(player, expl, |_| s.physical.agility)) * 0.16
        + n(eff(player, mental, |_| s.mental.positioning)) * 0.10
        + n(eff(player, mental, |_| s.mental.concentration)) * 0.10
        + n(eff(player, mental, |_| s.mental.anticipation)) * 0.08
        + n(eff(player, tech, |_| s.goalkeeping.one_on_ones)) * 0.08)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// GK aerial composite.
/// `aerial_reach*0.28 + command_of_area*0.20 + handling*0.14 + jumping*0.12
///  + strength*0.10 + bravery*0.08 + communication*0.08`
pub fn gk_aerial(player: &MatchPlayer, minute: u32) -> f32 {
    let (tech, mental, expl) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, tech, |_| s.goalkeeping.aerial_reach)) * 0.28
        + n(eff(player, mental, |_| s.goalkeeping.command_of_area)) * 0.20
        + n(eff(player, tech, |_| s.goalkeeping.handling)) * 0.14
        + n(eff(player, expl, |_| s.physical.jumping)) * 0.12
        + n(eff(player, expl, |_| s.physical.strength)) * 0.10
        + n(eff(player, mental, |_| s.mental.bravery)) * 0.08
        + n(eff(player, mental, |_| s.goalkeeping.communication)) * 0.08)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// GK communication / organisation composite — shouting, marshalling
/// the defensive line, calling for crosses. Used by tactical-organisation
/// damping and by GK area-control logic.
/// `communication*0.45 + command_of_area*0.25 + leadership*0.15
///  + concentration*0.10 + positioning*0.05`
pub fn gk_communication(player: &MatchPlayer, minute: u32) -> f32 {
    let (tech, mental, _) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, mental, |_| s.goalkeeping.communication)) * 0.45
        + n(eff(player, mental, |_| s.goalkeeping.command_of_area)) * 0.25
        + n(eff(player, mental, |_| s.mental.leadership)) * 0.15
        + n(eff(player, mental, |_| s.mental.concentration)) * 0.10
        + n(eff(player, mental, |_| s.mental.positioning)) * 0.05)
        .clamp(0.0, 1.0);
    // Suppress unused-variable warning when communication-only callers
    // skip the tech band; it's still loaded above when handling fires.
    let _ = tech;
    clamp_composite(v)
}

/// GK cross-claim composite — collecting / punching crosses, dealing
/// with high balls into the box. Subtly different from `gk_aerial`:
/// claim_cross weights handling/positioning over raw aerial reach so
/// a small but well-positioned keeper still earns claim credit.
/// `aerial_reach*0.28 + command_of_area*0.22 + handling*0.18
///  + positioning*0.12 + anticipation*0.10 + jumping*0.10`
pub fn gk_claim_cross(player: &MatchPlayer, minute: u32) -> f32 {
    let (tech, mental, expl) = ctxs(minute);
    let s = &player.skills;
    let v = (n(eff(player, tech, |_| s.goalkeeping.aerial_reach)) * 0.28
        + n(eff(player, mental, |_| s.goalkeeping.command_of_area)) * 0.22
        + n(eff(player, tech, |_| s.goalkeeping.handling)) * 0.18
        + n(eff(player, mental, |_| s.mental.positioning)) * 0.12
        + n(eff(player, mental, |_| s.mental.anticipation)) * 0.10
        + n(eff(player, expl, |_| s.physical.jumping)) * 0.10)
        .clamp(0.0, 1.0);
    clamp_composite(v)
}

/// GK distribution composite.
/// `goalkeeping.passing*0.24 + kicking*0.22 + throwing*0.18 + vision*0.12
///  + decisions*0.10 + composure*0.08 + technique/first_touch*0.06`
pub fn gk_distribution(player: &MatchPlayer, minute: u32) -> f32 {
    let (tech, mental, _) = ctxs(minute);
    let s = &player.skills;
    // `technique/first_touch` slot — average the two technical reads
    // since the spec leaves it explicitly as either-or.
    let touch_avg = 0.5
        * (n(eff(player, tech, |_| s.technical.technique))
            + n(eff(player, tech, |_| s.goalkeeping.first_touch)));
    let v = (n(eff(player, tech, |_| s.goalkeeping.passing)) * 0.24
        + n(eff(player, tech, |_| s.goalkeeping.kicking)) * 0.22
        + n(eff(player, tech, |_| s.goalkeeping.throwing)) * 0.18
        + n(eff(player, mental, |_| s.mental.vision)) * 0.12
        + n(eff(player, mental, |_| s.mental.decisions)) * 0.10
        + n(eff(player, mental, |_| s.mental.composure)) * 0.08
        + touch_avg * 0.06)
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
