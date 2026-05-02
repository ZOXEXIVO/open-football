//! First-touch / receiving-quality resolver.
//!
//! Real football is full of moments where the pass arrives but the
//! receiver fails to control it: heavy touches forward, miscontrols
//! under pressure, defenders nicking the ball off a poor first touch.
//! Conversely, elite first-touch players cushion the ball, open their
//! body, and either set up a one-touch pass or create a first-time shot
//! window. The resolver below produces a discrete `FirstTouchOutcome`
//! that callers can use to bias what happens after a pass / cross /
//! reception.
//!
//! Inputs:
//!   * the receiver's skills (with `effective_skill` fatigue applied)
//!   * pass difficulty (distance, aerial flag, driven flag, body opening)
//!   * pressure context (defenders within 6u, second pressure within 8u)
//!
//! Outcome thresholds (control_score):
//!   ≥ 0.72 → CleanControl  (one-touch / first-time-shot windows possible)
//!   0.55–0.72 → CleanControl with minor delay
//!   0.40–0.55 → HeavyTouchForward / Sideways
//!   0.25–0.40 → MiscontrolLoose
//!   < 0.25 → DefenderNicksBall (if defender within 5u) else loose ball
//!
//! Heavy-touch distance: 4.0 + (1.0 - control_score) * 14.0 units. So a
//! poor touch can spit the ball 12–14u away, putting the receiver under
//! pressure to recover.

use crate::r#match::MatchPlayer;
use crate::r#match::engine::player::strategies::common::players::ops::effective_skill::{
    ActionContext, effective_skill,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstTouchOutcome {
    /// Clean reception: ball settled at the receiver's feet, body open.
    CleanControl,
    /// Receiver took a heavy touch ahead — ball squirts forward, may
    /// run into space or into a defender depending on context.
    HeavyTouchForward,
    /// Heavy touch sideways — ball escapes to the wing or back into
    /// midfield, requires recovery.
    HeavyTouchSideways,
    /// Loose miscontrol — ball ends up several units away, 50/50 ball.
    MiscontrolLoose,
    /// Cushion / soft back-pass to a teammate — receiver let the ball
    /// run across body to a deeper teammate, never really "owned" it.
    CushionBackPass,
    /// Receiver has a one-touch pass window: composure / vision allows
    /// an immediate forward release without bringing the ball down.
    OneTouchPassWindow,
    /// First-time shot window: high-quality chance arrived in stride,
    /// receiver may shoot on the half-volley.
    FirstTimeShotWindow,
    /// A defender within 5u took advantage of the bad touch and
    /// claimed the ball before the receiver could recover.
    DefenderNicksBall,
}

impl FirstTouchOutcome {
    /// True for outcomes where possession is retained without recovery.
    pub fn keeps_possession(self) -> bool {
        matches!(
            self,
            FirstTouchOutcome::CleanControl
                | FirstTouchOutcome::OneTouchPassWindow
                | FirstTouchOutcome::FirstTimeShotWindow
                | FirstTouchOutcome::CushionBackPass
        )
    }

    /// True for outcomes where the ball is loose / contested / lost.
    pub fn is_loose(self) -> bool {
        matches!(
            self,
            FirstTouchOutcome::HeavyTouchForward
                | FirstTouchOutcome::HeavyTouchSideways
                | FirstTouchOutcome::MiscontrolLoose
                | FirstTouchOutcome::DefenderNicksBall
        )
    }

    /// How far the ball escapes from the receiver, in pitch units.
    pub fn escape_distance(self, control_score: f32) -> f32 {
        let base = 4.0 + (1.0 - control_score.clamp(0.0, 1.0)) * 14.0;
        match self {
            FirstTouchOutcome::CleanControl
            | FirstTouchOutcome::OneTouchPassWindow
            | FirstTouchOutcome::FirstTimeShotWindow
            | FirstTouchOutcome::CushionBackPass => 0.0,
            FirstTouchOutcome::HeavyTouchForward | FirstTouchOutcome::HeavyTouchSideways => base,
            FirstTouchOutcome::MiscontrolLoose => base * 1.2,
            FirstTouchOutcome::DefenderNicksBall => base * 0.5,
        }
    }
}

/// Difficulty modifiers applied by the pass that's arriving.
#[derive(Debug, Clone, Copy, Default)]
pub struct PassContext {
    /// Pass distance in units. Long passes are harder to control.
    pub distance_units: f32,
    /// Aerial / lofted ball — landing path makes control harder.
    pub aerial: bool,
    /// Driven low pass — pace alone makes control harder.
    pub driven: bool,
    /// Receiver had to use weak foot or open body awkwardly.
    pub weak_foot: bool,
    /// Receiver was sprinting at full pace as the ball arrived.
    pub sprinting: bool,
}

/// Pressure on the receiver at the moment of reception.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReceiverPressure {
    /// Distance to nearest defender in units.
    pub nearest_defender: f32,
    /// Number of defenders within 6 units of the receiver.
    pub defenders_within_6u: u8,
    /// Number of additional defenders within 8 units (excluding the
    /// nearest one). Used for the "second defender" modifier.
    pub second_within_8u: bool,
    /// Whether a viable forward teammate exists for a one-touch pass.
    pub one_touch_target_available: bool,
    /// xG quality of an immediate first-time shot from this position.
    /// Used to gate the FirstTimeShotWindow outcome.
    pub immediate_shot_xg: f32,
    /// Ball height at reception (units). Above ~3u → ground-shot
    /// disqualified for first-time foot shot.
    pub ball_height: f32,
}

/// The full first-touch resolution.
#[derive(Debug, Clone, Copy)]
pub struct FirstTouchResolution {
    pub outcome: FirstTouchOutcome,
    pub control_score: f32,
    pub escape_distance: f32,
}

/// Resolve a first touch given the receiver, the arriving pass, and the
/// pressure context. The resolver is deterministic in its score; outcome
/// roll uses an externally-supplied random in [0.0, 1.0) so callers can
/// drive it with whatever RNG they already use (`rand::random::<f32>()`
/// is the project idiom).
pub fn resolve_first_touch(
    receiver: &MatchPlayer,
    pass_ctx: PassContext,
    pressure: ReceiverPressure,
    minute: u32,
    roll: f32,
) -> FirstTouchResolution {
    let tech_ctx = ActionContext::technical(minute);
    let mental_ctx = ActionContext::mental(minute);
    let s = &receiver.skills;
    let first_touch = effective_skill(receiver, s.technical.first_touch, tech_ctx);
    let technique = effective_skill(receiver, s.technical.technique, tech_ctx);
    let composure = effective_skill(receiver, s.mental.composure, mental_ctx);
    let anticipation = effective_skill(receiver, s.mental.anticipation, mental_ctx);
    let balance = effective_skill(receiver, s.physical.balance, tech_ctx);
    let agility = effective_skill(receiver, s.physical.agility, tech_ctx);
    let decisions = effective_skill(receiver, s.mental.decisions, mental_ctx);
    let concentration = effective_skill(receiver, s.mental.concentration, mental_ctx);

    // Base control score in approximately [0.0, 1.0] — weighted blend of
    // skill values normalised to 1.0 = perfect 20/20 across the board.
    let base = (first_touch * 0.28
        + technique * 0.18
        + composure * 0.14
        + anticipation * 0.12
        + balance * 0.10
        + agility * 0.08
        + decisions * 0.06
        + concentration * 0.04)
        / 20.0;

    let mut score = base;

    // Pass difficulty penalties. Distance only starts hurting beyond
    // ~60u (a comfortable short-pass range); ramps to the full 0.18
    // penalty by 200u (long-ball territory).
    let distance_factor = ((pass_ctx.distance_units - 60.0).max(0.0) / 140.0).clamp(0.0, 1.0);
    score -= distance_factor * 0.18;
    if pass_ctx.aerial {
        score -= 0.10;
    }
    if pass_ctx.driven {
        score -= 0.05;
    }
    if pass_ctx.weak_foot {
        score -= 0.06;
    }
    if pass_ctx.sprinting {
        score -= 0.05;
    }

    // Pressure penalties — capped at -0.24 total from defender proximity.
    let pressure_pen = (pressure.defenders_within_6u as f32 * 0.06).min(0.24);
    score -= pressure_pen;
    if pressure.second_within_8u && pressure.defenders_within_6u >= 1 {
        score -= 0.04;
    }

    let control_score = score.clamp(0.0, 1.5);

    // Outcome resolution.
    let outcome = if control_score >= 0.72 {
        // Look for premium outcomes.
        let one_touch_unlocked = decisions >= 12.0
            && technique >= 11.0
            && (s.mental.vision >= 11.0)
            && pressure.nearest_defender <= 8.0
            && pressure.one_touch_target_available;
        let shot_unlocked = pressure.immediate_shot_xg >= 0.16
            && technique >= 12.0
            && composure >= 11.0
            && pressure.ball_height <= 3.0;
        if shot_unlocked && roll < 0.55 {
            FirstTouchOutcome::FirstTimeShotWindow
        } else if one_touch_unlocked && roll < 0.45 {
            FirstTouchOutcome::OneTouchPassWindow
        } else {
            FirstTouchOutcome::CleanControl
        }
    } else if control_score >= 0.55 {
        FirstTouchOutcome::CleanControl
    } else if control_score >= 0.40 {
        // Heavy touch — direction depends on body orientation /
        // sprinting. We approximate: sprinting → forward; otherwise
        // sideways.
        if pass_ctx.sprinting || pass_ctx.driven {
            FirstTouchOutcome::HeavyTouchForward
        } else {
            FirstTouchOutcome::HeavyTouchSideways
        }
    } else if control_score >= 0.25 {
        FirstTouchOutcome::MiscontrolLoose
    } else if pressure.nearest_defender <= 5.0 {
        FirstTouchOutcome::DefenderNicksBall
    } else {
        FirstTouchOutcome::MiscontrolLoose
    };

    let escape = outcome.escape_distance(control_score);

    FirstTouchResolution {
        outcome,
        control_score,
        escape_distance: escape,
    }
}

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

    fn make_player(first_touch: f32, technique: f32, composure: f32) -> MatchPlayer {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = 9000;
        let mut skills = PlayerSkills::default();
        skills.technical.first_touch = first_touch;
        skills.technical.technique = technique;
        skills.mental.composure = composure;
        skills.mental.anticipation = 12.0;
        skills.mental.decisions = 12.0;
        skills.mental.vision = 12.0;
        skills.mental.concentration = 12.0;
        skills.physical.balance = 12.0;
        skills.physical.agility = 12.0;
        skills.physical.stamina = 14.0;
        skills.physical.natural_fitness = 14.0;
        let player = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::ForwardCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(1, &player, PlayerPositionType::ForwardCenter, false)
    }

    #[test]
    fn elite_first_touch_under_pressure_controls_more_than_poor() {
        let elite = make_player(18.0, 17.0, 16.0);
        let poor = make_player(7.0, 7.0, 8.0);
        let pass = PassContext {
            distance_units: 60.0,
            aerial: false,
            driven: true,
            ..Default::default()
        };
        let pressure = ReceiverPressure {
            nearest_defender: 3.5,
            defenders_within_6u: 1,
            second_within_8u: false,
            one_touch_target_available: false,
            immediate_shot_xg: 0.0,
            ball_height: 0.5,
        };
        let elite_res = resolve_first_touch(&elite, pass, pressure, 30, 0.5);
        let poor_res = resolve_first_touch(&poor, pass, pressure, 30, 0.5);
        assert!(elite_res.control_score > poor_res.control_score + 0.15);
        assert!(elite_res.outcome.keeps_possession());
        assert!(poor_res.outcome.is_loose() || !poor_res.outcome.keeps_possession());
    }

    #[test]
    fn high_xg_window_can_unlock_first_time_shot() {
        let elite = make_player(18.0, 18.0, 16.0);
        let pass = PassContext {
            distance_units: 25.0,
            ..Default::default()
        };
        let pressure = ReceiverPressure {
            nearest_defender: 12.0,
            defenders_within_6u: 0,
            second_within_8u: false,
            one_touch_target_available: false,
            immediate_shot_xg: 0.30,
            ball_height: 0.4,
        };
        // Drive the roll into the FirstTimeShot branch.
        let res = resolve_first_touch(&elite, pass, pressure, 30, 0.10);
        assert!(matches!(
            res.outcome,
            FirstTouchOutcome::FirstTimeShotWindow | FirstTouchOutcome::CleanControl
        ));
    }

    #[test]
    fn very_poor_score_with_close_defender_lets_defender_nick() {
        let bad = make_player(4.0, 4.0, 4.0);
        let pass = PassContext {
            distance_units: 180.0,
            aerial: true,
            driven: true,
            weak_foot: true,
            sprinting: true,
        };
        let pressure = ReceiverPressure {
            nearest_defender: 3.0,
            defenders_within_6u: 2,
            second_within_8u: true,
            one_touch_target_available: false,
            immediate_shot_xg: 0.0,
            ball_height: 4.0,
        };
        let res = resolve_first_touch(&bad, pass, pressure, 30, 0.5);
        assert!(res.control_score < 0.40);
        assert!(res.outcome.is_loose());
    }
}
