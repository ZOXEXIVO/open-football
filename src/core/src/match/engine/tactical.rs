//! Team-level tactical state shared by all eleven players on a side.
//!
//! The per-player state machines (defenders, midfielders, forwards, GK)
//! used to evaluate ball/opponent proximity from scratch every tick and
//! produced "eleven independent agents" behaviour. Real football runs
//! off team-level phases — build-up, progression, attack, transition,
//! settled defence — that every player reads and respects. This module
//! defines that shared layer; it is recomputed periodically from the
//! ball/possession/time signals and consulted by player states when
//! they make branching decisions.

use crate::r#match::{MatchField, PlayerSide};

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
/// goal, so `ball_zone_for_team(left)` returns `AttackingThird` when
/// the ball is near x = field_width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BallZone {
    /// Ball is in this team's own defensive third.
    DefensiveThird,
    /// Ball is in the middle third.
    MiddleThird,
    /// Ball is in the opponent's third (this team's attacking third).
    AttackingThird,
}

/// Team-level tactical context, shared across all eleven players. Cheap
/// to copy (plain POD).
#[derive(Debug, Clone, Copy)]
pub struct TeamTacticalState {
    pub phase: GamePhase,
    /// How many ticks the current possession has lasted (0 when we
    /// don't have the ball).
    pub possession_ticks: u32,
    /// How many ticks since possession last changed hands — used to
    /// size the transition windows (AttackingTransition /
    /// DefensiveTransition are gated on this being small).
    pub ticks_since_turnover: u32,
    /// Which third the ball is currently in, from this team's
    /// attacking perspective.
    pub ball_zone: BallZone,
    /// True if this team currently has the ball.
    pub in_possession: bool,
    /// Target x-coordinate for the back line (shared by all defenders).
    /// Approximates the "defensive line" a tactical manager sets: high
    /// when we're pressing forward, low when we're in a low block.
    pub defensive_line_x: f32,
    /// 0.0 = play normally; 1.0 = full "park the bus / waste time" mode.
    /// Rises when leading, late in the game, and/or when we are the
    /// weaker side. Read by pass selection (prefer safe backward /
    /// sideways balls) and by the forward running state (hold the ball,
    /// don't shoot speculatively). A single continuous signal keeps
    /// behaviour smooth and avoids hard "weak team" / "strong team"
    /// branching.
    pub game_management_intensity: f32,
}

impl TeamTacticalState {
    pub fn initial() -> Self {
        TeamTacticalState {
            phase: GamePhase::MidBlock,
            possession_ticks: 0,
            ticks_since_turnover: 0,
            ball_zone: BallZone::MiddleThird,
            in_possession: false,
            defensive_line_x: 0.0,
            game_management_intensity: 0.0,
        }
    }

    /// In an attacking transition: the window that opens right after
    /// winning the ball back. Short (≤ 50 ticks ≈ 5 s) — after that we
    /// move into normal progression.
    pub fn is_attacking_transition(&self) -> bool {
        matches!(self.phase, GamePhase::AttackingTransition)
    }

    /// In a defensive transition: short window after losing the ball
    /// where a counter-press can fire.
    pub fn is_defensive_transition(&self) -> bool {
        matches!(self.phase, GamePhase::DefensiveTransition)
    }

    pub fn is_settled_defending(&self) -> bool {
        matches!(self.phase, GamePhase::MidBlock | GamePhase::LowBlock)
    }
}

/// Decide the ball zone for a team whose goal sits on `side`. Left-side
/// teams attack right (toward large x); right-side teams attack left.
fn ball_zone_for_side(field_width: f32, ball_x: f32, side: PlayerSide) -> BallZone {
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

/// Compute the phase for a team given the current world state and this
/// team's rolling counters. Pure function — all state mutations happen
/// in `update_tactical_states`.
fn compute_phase(
    in_possession: bool,
    ball_zone: BallZone,
    ticks_since_turnover: u32,
    possession_ticks: u32,
    high_press_allowed: bool,
) -> GamePhase {
    // Transition windows are the highest-priority phase: they override
    // settled states because goals disproportionately come from them.
    const TRANSITION_WINDOW_TICKS: u32 = 50; // ~5 sim seconds

    if in_possession {
        if ticks_since_turnover < TRANSITION_WINDOW_TICKS && possession_ticks < TRANSITION_WINDOW_TICKS {
            return GamePhase::AttackingTransition;
        }
        return match ball_zone {
            BallZone::DefensiveThird => GamePhase::BuildUp,
            BallZone::MiddleThird => GamePhase::Progression,
            BallZone::AttackingThird => GamePhase::Attack,
        };
    }

    // Out of possession.
    if ticks_since_turnover < TRANSITION_WINDOW_TICKS {
        return GamePhase::DefensiveTransition;
    }
    if high_press_allowed && matches!(ball_zone, BallZone::AttackingThird | BallZone::MiddleThird) {
        return GamePhase::HighPress;
    }
    match ball_zone {
        BallZone::DefensiveThird => GamePhase::LowBlock,
        _ => GamePhase::MidBlock,
    }
}

/// Compute game-management intensity from this team's perspective.
/// Continuous [0.0, 1.0] signal driving "hold the score" behaviour:
/// safer passes, slower tempo, hold the ball. A single scalar covers
/// every case — strong team protecting a lead, underdog clinging to a
/// narrow upset, team settling for a point late — instead of hard
/// branching per scenario.
fn compute_game_management_intensity(
    score_diff: i8,
    minute: f32,
    my_avg_ability: u16,
    opp_avg_ability: u16,
) -> f32 {
    // Ramp up from minute 60 onward; max at minute 90.
    let late_factor = ((minute - 60.0).max(0.0) / 30.0).min(1.0);
    // Positive = we're the weaker side. 40 CA ≈ one league tier.
    let ability_gap = ((opp_avg_ability as f32 - my_avg_ability as f32) / 40.0).clamp(-1.0, 1.0);

    if score_diff > 0 {
        // Leading — defend the score. A 1-goal lead in the final
        // 5 minutes should push teams firmly into possession mode; the
        // previous curve (0.15 lead_base + 0.35 late_bonus = 0.50 max
        // for an equal-squad 1-goal lead) sat just under the 0.55
        // prefer_possession threshold, so teams kept attacking like the
        // match was balanced. Bumped to 0.22 base + 0.48 late so an
        // equal-squad 1-goal lead at minute 90 reaches 0.70.
        let lead_base = 0.22 + 0.18 * ((score_diff - 1).clamp(0, 2) as f32);
        let weaker_bonus = 0.25 * ability_gap.max(0.0);
        let late_bonus = 0.48 * late_factor;
        (lead_base + weaker_bonus + late_bonus).clamp(0.0, 0.95)
    } else if score_diff == 0 && ability_gap > 0.2 && late_factor > 0.5 {
        // Weaker team late in a draw plays for the point.
        (0.15 + 0.20 * late_factor).clamp(0.0, 0.5)
    } else {
        0.0
    }
}

/// Recompute both teams' tactical state in-place. Called periodically
/// from the match tick loop (every ~10 ticks is enough — phase shifts
/// settle over multiple seconds, not every frame).
pub fn update_tactical_states(
    home: &mut TeamTacticalState,
    away: &mut TeamTacticalState,
    field: &MatchField,
    home_team_id: u32,
    tick_interval: u32,
    coach_wants_high_press_home: bool,
    coach_wants_high_press_away: bool,
    home_score_diff: i8,
    match_time_ms: u64,
    home_avg_ability: u16,
    away_avg_ability: u16,
) {
    let field_width = field.size.width as f32;
    let ball_x = field.ball.position.x;

    // Determine which side the ball owner plays on. If no owner, keep
    // the previous possession flag (we're in a loose-ball moment and
    // the prior team still has "last touch" status).
    let owning_team_id = field
        .ball
        .current_owner
        .and_then(|id| field.players.iter().find(|p| p.id == id))
        .map(|p| p.team_id);

    let prev_home_possession = home.in_possession;
    let home_now_has_ball = match owning_team_id {
        Some(id) => id == home_team_id,
        None => prev_home_possession,
    };
    let away_now_has_ball = match owning_team_id {
        Some(id) => id != home_team_id,
        None => !prev_home_possession,
    };

    let home_turned_over = home.in_possession != home_now_has_ball;
    let away_turned_over = away.in_possession != away_now_has_ball;

    // Update rolling counters.
    home.in_possession = home_now_has_ball;
    away.in_possession = away_now_has_ball;

    home.ticks_since_turnover = if home_turned_over {
        0
    } else {
        home.ticks_since_turnover.saturating_add(tick_interval)
    };
    away.ticks_since_turnover = if away_turned_over {
        0
    } else {
        away.ticks_since_turnover.saturating_add(tick_interval)
    };

    home.possession_ticks = if home_now_has_ball {
        home.possession_ticks.saturating_add(tick_interval)
    } else {
        0
    };
    away.possession_ticks = if away_now_has_ball {
        away.possession_ticks.saturating_add(tick_interval)
    } else {
        0
    };

    // Ball zones from each team's perspective (left attacks right, etc.)
    home.ball_zone = ball_zone_for_side(field_width, ball_x, PlayerSide::Left);
    away.ball_zone = ball_zone_for_side(field_width, ball_x, PlayerSide::Right);

    // Phase.
    home.phase = compute_phase(
        home.in_possession,
        home.ball_zone,
        home.ticks_since_turnover,
        home.possession_ticks,
        coach_wants_high_press_home,
    );
    away.phase = compute_phase(
        away.in_possession,
        away.ball_zone,
        away.ticks_since_turnover,
        away.possession_ticks,
        coach_wants_high_press_away,
    );

    // Defensive line x: interpolate between "deep" (own third boundary)
    // and "high" (opponent's half) based on phase. This gives defenders
    // a shared reference frame for how high to push.
    let third = field_width / 3.0;
    home.defensive_line_x = match home.phase {
        GamePhase::HighPress | GamePhase::Attack => field_width * 0.55,
        GamePhase::AttackingTransition | GamePhase::Progression => field_width * 0.45,
        GamePhase::BuildUp => field_width * 0.25,
        GamePhase::MidBlock | GamePhase::DefensiveTransition => third,
        GamePhase::LowBlock => field_width * 0.18,
    };
    away.defensive_line_x = match away.phase {
        GamePhase::HighPress | GamePhase::Attack => field_width * 0.45,
        GamePhase::AttackingTransition | GamePhase::Progression => field_width * 0.55,
        GamePhase::BuildUp => field_width * 0.75,
        GamePhase::MidBlock | GamePhase::DefensiveTransition => field_width - third,
        GamePhase::LowBlock => field_width * 0.82,
    };

    let minute = (match_time_ms as f32) / 60_000.0;
    home.game_management_intensity = compute_game_management_intensity(
        home_score_diff, minute, home_avg_ability, away_avg_ability,
    );
    away.game_management_intensity = compute_game_management_intensity(
        -home_score_diff, minute, away_avg_ability, home_avg_ability,
    );
}

#[cfg(test)]
mod tests {
    use super::compute_game_management_intensity;

    #[test]
    fn losing_side_never_manages_the_game() {
        assert_eq!(compute_game_management_intensity(-1, 85.0, 120, 140), 0.0);
        assert_eq!(compute_game_management_intensity(-2, 30.0, 150, 150), 0.0);
    }

    #[test]
    fn early_small_lead_produces_mild_signal() {
        let v = compute_game_management_intensity(1, 20.0, 140, 140);
        assert!(v > 0.0 && v < 0.25, "got {v}");
    }

    #[test]
    fn weaker_side_protecting_late_lead_parks_the_bus() {
        let strong_even = compute_game_management_intensity(1, 85.0, 150, 150);
        let weak_late = compute_game_management_intensity(1, 85.0, 110, 150);
        assert!(weak_late > strong_even, "weak_late={weak_late} strong_even={strong_even}");
        assert!(weak_late > 0.5, "got {weak_late}");
    }

    #[test]
    fn weaker_side_late_draw_plays_for_point() {
        let v = compute_game_management_intensity(0, 85.0, 110, 150);
        assert!(v > 0.0 && v < 0.5, "got {v}");
    }

    #[test]
    fn intensity_is_clamped_below_one() {
        let v = compute_game_management_intensity(5, 90.0, 100, 160);
        assert!(v <= 0.95, "got {v}");
    }
}
