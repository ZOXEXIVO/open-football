//! **How the game is read.** The three classifications the rest of the
//! tactical layer is derived from — the team's [`GamePhase`], and the two
//! zone reads ([`BallZone`], [`BallSideZone`]) the phase is picked from.
//!
//! The phase math lives here with them: the transition windows, and the
//! pure `compute_phase` that turns possession + zone + turnover clock
//! into a phase. `refresh` in [`team_state`](super::team_state) calls it
//! once per side.

use crate::r#match::PlayerSide;
use crate::r#match::engine::teamplay::tactical::team_state::TeamTacticalState;

/// The team's high-level game phase. Recomputed from ball position,
/// possession, and recent turnover. Player states branch on this before
/// falling back to local heuristics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamePhase {
    /// We have the ball in our own third — defenders passing, GK may
    /// distribute. Players offer short outlets; forwards don't drop all
    /// the way back.
    BuildUp,
    /// We have the ball in the middle third — midfielders look for
    /// line-breaking passes, forwards make runs into channels.
    Progression,
    /// We have the ball in the attacking third — forwards position for
    /// cross / shot; midfielders arrive at the box; defenders hold.
    Attack,
    /// We just won the ball back (≤ ~5 seconds ago). Forwards sprint
    /// in behind; midfielders play direct passes; defenders don't
    /// overlap yet — it's a fast break.
    AttackingTransition,
    /// We just lost the ball (≤ ~5 seconds ago). Nearest two or three
    /// players counter-press; the rest drop toward shape. Most real
    /// goals come in these transition windows.
    DefensiveTransition,
    /// Opponent has the ball; we've settled into a mid-block. Stay
    /// compact 30-40 metres from own goal, cut passing lanes.
    MidBlock,
    /// Opponent has the ball; we've dropped into a low block. Back line
    /// inside own third, narrow. "Park the bus" style defending.
    LowBlock,
    /// Coach pushed for a high press — we hunt the ball in opponent's
    /// defensive third. Only triggers when coach says so AND we have
    /// the energy for it.
    HighPress,
}

/// Which third of the pitch the ball is in, from the *attacking*
/// perspective of a given team. A team attacks toward the opposite
/// goal, so `BallZone::for_side(left, ...)` returns `AttackingThird`
/// when the ball is near x = field_width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BallZone {
    /// Ball is in this team's own defensive third.
    DefensiveThird,
    /// Ball is in the middle third.
    MiddleThird,
    /// Ball is in the opponent's third (this team's attacking third).
    AttackingThird,
}

impl BallZone {
    /// Decide the ball zone for a team whose goal sits on `side`.
    /// Left-side teams attack right (toward large x); right-side teams
    /// attack left.
    pub fn for_side(field_width: f32, ball_x: f32, side: PlayerSide) -> BallZone {
        let third = field_width / 3.0;
        let in_own_third = match side {
            PlayerSide::Left => ball_x < third,
            PlayerSide::Right => ball_x > field_width - third,
        };
        let in_attacking_third = match side {
            PlayerSide::Left => ball_x > field_width - third,
            PlayerSide::Right => ball_x < third,
        };
        if in_own_third {
            BallZone::DefensiveThird
        } else if in_attacking_third {
            BallZone::AttackingThird
        } else {
            BallZone::MiddleThird
        }
    }
}

/// Lateral side of the pitch the ball is on. Used to bias support runs
/// and rest-defence so a team doesn't end up with the whole shape
/// concentrated on one flank during a long possession.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BallSideZone {
    Left,
    Center,
    Right,
}

impl BallSideZone {
    /// Bucket a y-coordinate on the pitch into left / center / right
    /// thirds. The pitch's vertical axis is height (y), not width.
    pub fn for_y(field_height: f32, ball_y: f32) -> BallSideZone {
        let third = field_height / 3.0;
        if ball_y < third {
            BallSideZone::Left
        } else if ball_y > field_height - third {
            BallSideZone::Right
        } else {
            BallSideZone::Center
        }
    }
}

impl TeamTacticalState {
    /// Default transition window in physics ticks (10 ms each).
    /// 350 ticks = 3.5 sim seconds — the canonical "modern football"
    /// counter-window covers ~3-5 s. The legacy 50-tick value claimed
    /// 5 s in its comment but was actually 0.5 s, which collapsed the
    /// transition phases into one-frame blips and let
    /// `is_defensive_transition` go false before any defender could
    /// even start a counterpress.
    pub const DEFAULT_TRANSITION_WINDOW_TICKS: u32 = 350;

    /// Attacking-transition window scales with `build_up_patience`:
    /// patient possession sides hold the "just won the ball" mindset
    /// longer (slowly turn the press into a settled progression);
    /// counter-attacking sides shorten it and drop into Attack/Progression
    /// faster. Range 250-400 ticks (2.5-4.0 s).
    pub fn attacking_transition_window_ticks(build_up_patience: f32) -> u32 {
        let p = build_up_patience.clamp(0.0, 1.0);
        (250.0 + (400.0 - 250.0) * p) as u32
    }

    /// Defensive-transition window scales with `counter_press_intensity`:
    /// counter-pressing teams (high counter_press) hold the "press the
    /// loss" window longer; low counter-press teams collapse the window
    /// and drop straight into shape. Range 220-500 ticks (2.2-5.0 s).
    pub fn defensive_transition_window_ticks(counter_press_intensity: f32) -> u32 {
        let p = counter_press_intensity.clamp(0.0, 1.0);
        (220.0 + (500.0 - 220.0) * p) as u32
    }

    /// Compute the phase for a team given the current world state and
    /// rolling counters. Transition windows are configurable per side
    /// so possession-style and counter-attack sides resolve their
    /// transitions on different timescales. Pure — all state mutations
    /// happen in `refresh`.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_phase(
        in_possession: bool,
        ball_zone: BallZone,
        ticks_since_turnover: u32,
        possession_ticks: u32,
        high_press_allowed: bool,
        attack_window_ticks: u32,
        defensive_window_ticks: u32,
    ) -> GamePhase {
        if in_possession {
            // Attacking transition: the ball was JUST won. We use the
            // shorter of the two clocks so a slow possession buildup
            // (high possession_ticks but stale turnover) doesn't get
            // mis-flagged as a counter window.
            if ticks_since_turnover < attack_window_ticks && possession_ticks < attack_window_ticks
            {
                return GamePhase::AttackingTransition;
            }
            return match ball_zone {
                BallZone::DefensiveThird => GamePhase::BuildUp,
                BallZone::MiddleThird => GamePhase::Progression,
                BallZone::AttackingThird => GamePhase::Attack,
            };
        }

        // Out of possession.
        if ticks_since_turnover < defensive_window_ticks {
            return GamePhase::DefensiveTransition;
        }
        if high_press_allowed
            && matches!(ball_zone, BallZone::AttackingThird | BallZone::MiddleThird)
        {
            return GamePhase::HighPress;
        }
        match ball_zone {
            BallZone::DefensiveThird => GamePhase::LowBlock,
            _ => GamePhase::MidBlock,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ball_zone_for_left_team_in_own_third() {
        // Left team's defensive third is the small-x side of the pitch.
        let z = BallZone::for_side(900.0, 100.0, PlayerSide::Left);
        assert_eq!(z, BallZone::DefensiveThird);
    }

    #[test]
    fn ball_zone_for_right_team_in_own_third() {
        let z = BallZone::for_side(900.0, 800.0, PlayerSide::Right);
        assert_eq!(z, BallZone::DefensiveThird);
    }

    #[test]
    fn ball_side_zone_buckets_by_y() {
        assert_eq!(BallSideZone::for_y(540.0, 50.0), BallSideZone::Left);
        assert_eq!(BallSideZone::for_y(540.0, 270.0), BallSideZone::Center);
        assert_eq!(BallSideZone::for_y(540.0, 500.0), BallSideZone::Right);
    }

    /// Default windows used by the legacy-shape tests below. The
    /// `refresh` path always passes per-team-derived windows, but the
    /// pure `compute_phase` is parameterised so test cases can pin the
    /// window precisely.
    const W_ATTACK: u32 = TeamTacticalState::DEFAULT_TRANSITION_WINDOW_TICKS;
    const W_DEF: u32 = TeamTacticalState::DEFAULT_TRANSITION_WINDOW_TICKS;

    #[test]
    fn just_won_ball_is_attacking_transition() {
        // In possession with low ticks_since_turnover and low possession
        // ticks → AttackingTransition (overrides settled-phase mapping).
        let phase = TeamTacticalState::compute_phase(
            true,
            BallZone::MiddleThird,
            50,
            50,
            false,
            W_ATTACK,
            W_DEF,
        );
        assert_eq!(phase, GamePhase::AttackingTransition);
    }

    #[test]
    fn just_lost_ball_is_defensive_transition() {
        let phase = TeamTacticalState::compute_phase(
            false,
            BallZone::MiddleThird,
            50,
            0,
            false,
            W_ATTACK,
            W_DEF,
        );
        assert_eq!(phase, GamePhase::DefensiveTransition);
    }

    #[test]
    fn settled_possession_phase_follows_ball_zone() {
        // After the transition window expires, the phase depends only
        // on which third of the pitch the ball is in (from this team's
        // attacking perspective). 600 ticks ≈ 6 s, well past the
        // 350-tick default window.
        assert_eq!(
            TeamTacticalState::compute_phase(
                true,
                BallZone::DefensiveThird,
                600,
                600,
                false,
                W_ATTACK,
                W_DEF
            ),
            GamePhase::BuildUp
        );
        assert_eq!(
            TeamTacticalState::compute_phase(
                true,
                BallZone::MiddleThird,
                600,
                600,
                false,
                W_ATTACK,
                W_DEF
            ),
            GamePhase::Progression
        );
        assert_eq!(
            TeamTacticalState::compute_phase(
                true,
                BallZone::AttackingThird,
                600,
                600,
                false,
                W_ATTACK,
                W_DEF
            ),
            GamePhase::Attack
        );
    }

    #[test]
    fn settled_defense_phase_follows_ball_zone() {
        // Out of possession with stale turnover counter — settled
        // defending. LowBlock when ball is in OUR own third (from this
        // team's perspective), MidBlock otherwise.
        assert_eq!(
            TeamTacticalState::compute_phase(
                false,
                BallZone::DefensiveThird,
                700,
                0,
                false,
                W_ATTACK,
                W_DEF
            ),
            GamePhase::LowBlock
        );
        assert_eq!(
            TeamTacticalState::compute_phase(
                false,
                BallZone::MiddleThird,
                700,
                0,
                false,
                W_ATTACK,
                W_DEF
            ),
            GamePhase::MidBlock
        );
        assert_eq!(
            TeamTacticalState::compute_phase(
                false,
                BallZone::AttackingThird,
                700,
                0,
                false,
                W_ATTACK,
                W_DEF
            ),
            GamePhase::MidBlock
        );
    }

    #[test]
    fn high_press_overrides_mid_block_when_coach_calls_for_it() {
        let phase = TeamTacticalState::compute_phase(
            false,
            BallZone::MiddleThird,
            700,
            0,
            true, // coach wants high press
            W_ATTACK,
            W_DEF,
        );
        assert_eq!(phase, GamePhase::HighPress);
    }

    #[test]
    fn high_press_does_not_override_low_block_when_ball_is_deep() {
        // High press only fires when the ball is in middle/attacking
        // third — pressing deep in your own box is just bad shape, so
        // we never move into HighPress with the ball in our own third.
        let phase = TeamTacticalState::compute_phase(
            false,
            BallZone::DefensiveThird,
            700,
            0,
            true,
            W_ATTACK,
            W_DEF,
        );
        assert_eq!(phase, GamePhase::LowBlock);
    }

    // ──────────────────────────────────────────────────────────────────
    // Transition window — real tick units. MATCH_TIME_INCREMENT_MS is 10,
    // so 100 ticks = 1 sim second. The legacy 50-tick window claimed
    // "≈5 s" but was actually 0.5 s. The new defaults give ~3.5 s.
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn fifty_ticks_after_turnover_is_still_transition() {
        // 50 ticks = 0.5 s. Should be deeply inside both attacking and
        // defensive transition windows under any reasonable settings.
        let win_a = TeamTacticalState::attacking_transition_window_ticks(0.5);
        let win_d = TeamTacticalState::defensive_transition_window_ticks(0.5);
        assert!(win_a > 50);
        assert!(win_d > 50);
        let phase = TeamTacticalState::compute_phase(
            true,
            BallZone::MiddleThird,
            50,
            50,
            false,
            win_a,
            win_d,
        );
        assert_eq!(phase, GamePhase::AttackingTransition);
    }

    #[test]
    fn five_hundred_ticks_after_loss_is_settled_unless_high_counterpress() {
        // Defensive style: short defensive window (~220 ticks). 500
        // ticks of out-of-possession have moved them into a settled
        // block, NOT a transition.
        let defensive_window = TeamTacticalState::defensive_transition_window_ticks(0.0);
        assert!(defensive_window < 500);
        let phase_low_counter = TeamTacticalState::compute_phase(
            false,
            BallZone::MiddleThird,
            500,
            0,
            false,
            W_ATTACK,
            defensive_window,
        );
        assert_eq!(phase_low_counter, GamePhase::MidBlock);

        // Counter-pressing style: long defensive window. 500 ticks is
        // still inside it, so the counter-press phase still holds.
        let counterpress_window = TeamTacticalState::defensive_transition_window_ticks(1.0);
        assert!(counterpress_window >= 500);
        let phase_high_counter = TeamTacticalState::compute_phase(
            false,
            BallZone::MiddleThird,
            499,
            0,
            false,
            W_ATTACK,
            counterpress_window,
        );
        assert_eq!(phase_high_counter, GamePhase::DefensiveTransition);
    }

    #[test]
    fn attacking_window_grows_with_build_up_patience() {
        let direct = TeamTacticalState::attacking_transition_window_ticks(0.0);
        let patient = TeamTacticalState::attacking_transition_window_ticks(1.0);
        assert!(patient > direct);
        // Bounds: 250..400 ticks per spec.
        assert_eq!(direct, 250);
        assert_eq!(patient, 400);
    }

    #[test]
    fn defensive_window_grows_with_counter_press() {
        let low = TeamTacticalState::defensive_transition_window_ticks(0.0);
        let high = TeamTacticalState::defensive_transition_window_ticks(1.0);
        assert!(high > low);
        // Bounds: 220..500 ticks per spec.
        assert_eq!(low, 220);
        assert_eq!(high, 500);
    }

    // ──────────────────────────────────────────────────────────────────
    // PlayerSide math — the bug-proof tests. The legacy formulas were
    // asymmetric: they accidentally classified right-side teams as
    // "always defensive third, never attacking third". Lock that down.
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn left_attacking_progress_increases_with_x() {
        let s = PlayerSide::Left;
        assert_eq!(s.attacking_progress_x(0.0, 900.0), 0.0);
        assert!((s.attacking_progress_x(450.0, 900.0) - 0.5).abs() < 1e-4);
        assert_eq!(s.attacking_progress_x(900.0, 900.0), 1.0);
    }

    #[test]
    fn right_attacking_progress_increases_as_x_decreases() {
        // Bug check: a right-side player at x=50 should be DEEP in
        // their attacking third (progress > 0.66), not in their own.
        let s = PlayerSide::Right;
        assert!(s.attacking_progress_x(50.0, 900.0) > 0.66);
        assert!(s.attacking_progress_x(850.0, 900.0) < 0.33);
        assert!((s.attacking_progress_x(450.0, 900.0) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn left_forward_delta_signs() {
        let s = PlayerSide::Left;
        assert!(s.forward_delta(100.0, 200.0) > 0.0);
        assert!(s.forward_delta(200.0, 100.0) < 0.0);
    }

    #[test]
    fn right_forward_delta_signs() {
        let s = PlayerSide::Right;
        assert!(s.forward_delta(800.0, 700.0) > 0.0); // Right team forward = lower x
        assert!(s.forward_delta(700.0, 800.0) < 0.0);
    }

    #[test]
    fn forward_delta_norm_is_signed_and_bounded() {
        let s = PlayerSide::Left;
        let v = s.forward_delta_norm(0.0, 900.0, 900.0);
        assert!((v - 1.0).abs() < 1e-4);
        let v = s.forward_delta_norm(900.0, 0.0, 900.0);
        assert!((v + 1.0).abs() < 1e-4);
    }
}
