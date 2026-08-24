//! **What `refresh` is handed.** The per-tick input bundle
//! ([`TacticalRefreshInputs`]) and the per-team skill composites
//! ([`TeamSkillAggregates`]) the engine fills it with.
//!
//! Neither type computes anything: the tick loop walks the players once,
//! fills these, and hands them to
//! [`TeamTacticalState::refresh`](super::team_state::TeamTacticalState::refresh),
//! which is the only reader.

use crate::Tactics;
use crate::r#match::{MatchField, PlayerSide};

/// Inputs to `TeamTacticalState::refresh` — bundled so the call site
/// stays readable. The engine's tick loop fills this once per refresh
/// and hands it over.
/// Per-team skill composite aggregates derived from the engine's
/// per-player skill composites. All fields are 0..1 normalised. A
/// neutral squad sits near 0.50; elite ~0.70+; weak ~0.35-.
#[derive(Debug, Clone, Copy)]
pub struct TeamSkillAggregates {
    /// Average passing-execution composite of defenders + midfielders.
    pub build_up_quality: f32,
    /// Average pressing composite of outfielders.
    pub press_quality: f32,
    /// Average defensive-duel ⊕ interception composite of defenders +
    /// midfielders.
    pub defensive_quality: f32,
    /// Average shooting + dribble + off-ball composite of forwards +
    /// attacking midfielders. (Not consumed by `refresh()` directly —
    /// kept on the inputs so coach-eval and per-player reads can lift
    /// the same number.)
    pub attacking_quality: f32,
    /// Goalkeeper shot stopping ⊕ distribution composite.
    pub gk_quality: f32,
    /// Mean of (concentration + teamwork) / 2 across the outfield. Used
    /// by lead-protection logic to damp panic shape changes.
    pub concentration_teamwork_avg: f32,
    /// Highest `on_field_leadership` composite among the players
    /// currently on the pitch — the loudest organising voice, not an
    /// average. Organisation in football comes from the one man doing
    /// it, and averaging eleven players hides him completely.
    ///
    /// Read by `ShapeDiscipline` to decide how tightly the block is
    /// held. Kept on the *team* aggregate rather than resolved
    /// per-player because it is a team property and this struct is
    /// already recomputed once per ~100 ticks instead of once per
    /// player per tick.
    pub top_leadership: f32,
    /// **The goalkeeper's organising voice**, from `sc::gk_communication`
    /// (communication 0.45 + command_of_area 0.25 + leadership 0.15 +
    /// concentration 0.10 + positioning 0.05).
    ///
    /// Kept apart from `top_leadership` deliberately. That one is a
    /// maximum over the whole eleven and answers "who is running this
    /// side"; a keeper only wins it if he out-leads the captain. This
    /// answers a different and narrower question — how much help the men
    /// IN FRONT OF HIM are getting from behind — and every side has a
    /// goalkeeper, so it is always live.
    ///
    /// Read by `ShapeDiscipline::organisation`, weighted by line: a keeper
    /// can shout a centre-half into position and can do very little about
    /// a winger forty metres up the pitch.
    pub keeper_voice: f32,
}

impl TeamSkillAggregates {
    /// Population mean of `keeper_voice`, so the term multiplying the
    /// shape recall is centred and a median keeper leaves the calibrated
    /// discipline exactly where it was. ⚠ Measured off the `KEEPER VOICE`
    /// block in `dev_match stats`, not assumed — see the note on
    /// `SaveModel::POPULATION_HANDLING` for why 0.5 is the wrong guess for
    /// any composite that runs through `keeper_curve`.
    pub const KEEPER_VOICE_REFERENCE: f32 = 0.560;
    /// Neutral default — used when the team has no players or as a
    /// fallback for callers that don't compute composites.
    pub const fn neutral() -> Self {
        Self {
            build_up_quality: 0.5,
            press_quality: 0.5,
            defensive_quality: 0.5,
            attacking_quality: 0.5,
            gk_quality: 0.5,
            concentration_teamwork_avg: 0.5,
            top_leadership: 0.5,
            keeper_voice: Self::KEEPER_VOICE_REFERENCE,
        }
    }
}

impl Default for TeamSkillAggregates {
    fn default() -> Self {
        Self::neutral()
    }
}

pub struct TacticalRefreshInputs<'a> {
    pub field: &'a MatchField,
    pub home_team_id: u32,
    /// Which end the HOME side is defending right now. Sides swap at
    /// half time (`MatchField::swap_squads`), so this cannot be assumed:
    /// every zone / line-height number below is expressed relative to a
    /// team's own goal, and reading it off the wrong end inverts the
    /// entire tactical layer for one half of every match — a team in its
    /// own six-yard box would classify as `AttackingThird`, pick the
    /// `Attack` phase, and push its defensive line to 55% of the pitch
    /// while under siege.
    pub home_side: PlayerSide,
    pub tick_interval: u32,
    pub coach_wants_high_press_home: bool,
    pub coach_wants_high_press_away: bool,
    pub home_score_diff: i8,
    pub match_time_ms: u64,
    pub home_avg_ability: u16,
    pub away_avg_ability: u16,
    pub home_avg_condition: f32,
    pub away_avg_condition: f32,
    pub home_tactics: &'a Tactics,
    pub away_tactics: &'a Tactics,
    /// Per-team skill composite aggregates. `engine.rs` walks the
    /// active players once and fills these so `refresh()` can tune
    /// line height, press sustainability, and build-up patience using
    /// the team's actual collective ability — not just the raw `current
    /// _ability` average.
    pub home_skills: TeamSkillAggregates,
    pub away_skills: TeamSkillAggregates,
    /// Home-crowd edge in [0, 1]: `crowd_intensity × home_advantage`
    /// from the match environment. Drives the front-foot home lift
    /// (press / risk / tempo) and the away caution drop in `refresh` —
    /// the play-quality half of home advantage (the referee
    /// marginal-call half lives in `RefereeProfile::home_bias`).
    pub home_edge: f32,
    /// How far the standard of football in this fixture sits from the
    /// division the skill gates below were fitted in — see
    /// `MatchStandard`. Subtracted from every team-quality read in
    /// `refresh`, because those gates are ABSOLUTE thresholds (0.45 /
    /// 0.55 / 0.65) on a quantity that scales with the division, so read
    /// raw they hand every side below mid-table a weak press and a deep
    /// line and every side above it a high one. Measured, the whole
    /// family switches over between levels 8 and 12 — which is exactly
    /// where the `levels` sweep steps.
    pub standard_shift: f32,
    /// The goalkeeping equivalent — `gk_quality` blends the goalkeeping
    /// attributes, which the generator does not hand the same population
    /// mean as the outfield ones, so `gk_line_lift` needs its own.
    pub standard_gk_shift: f32,
}
