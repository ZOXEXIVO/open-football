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
//!
//! All compute helpers are associated functions on `TeamTacticalState`
//! (or `BallZone` / `BallSideZone`) — there are no loose helper
//! functions in this module. Keeping the math attached to the type
//! makes the calculator boundary obvious from a `cargo doc` view.

use crate::Tactics;
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

/// Inputs to `TeamTacticalState::refresh` — bundled so the call site
/// stays readable. The engine's tick loop fills this once per refresh
/// and hands it over.
pub struct TacticalRefreshInputs<'a> {
    pub field: &'a MatchField,
    pub home_team_id: u32,
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
    /// Which lateral side of the pitch the ball is on.
    pub ball_side: BallSideZone,
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
    /// 0.0 = passive (sit deep, wait); 1.0 = full hunt-the-ball press.
    /// Combines tactic style, coach instruction, condition, and phase.
    /// Defenders/midfielders read this to decide whether to step up or
    /// drop. Tired teams or late game-management situations push toward
    /// 0.
    pub press_intensity: f32,
    /// 0.0 = stretched shape (wide, deep); 1.0 = very compact (tight
    /// vertical/horizontal distances). Used by defenders and pivots
    /// when choosing positions relative to teammates. Rises in
    /// LowBlock / DefensiveTransition / late-lead game management.
    pub compactness_target: f32,
    /// 0.0 = narrow shape (concentrate centrally); 1.0 = full width
    /// (touchline-to-touchline). Wide-play tactics + Attack phase push
    /// toward 1.0; Compact / LowBlock toward 0.
    pub team_width_target: f32,
    /// 0.0 = slow patient build-up; 1.0 = fast direct play. Drops in
    /// possession styles + game-management; rises in transitions and
    /// counter-attack tactics. Drives the forward-pass urgency in the
    /// pass evaluator and the hold-time before forwards consider a shot.
    pub tempo: f32,
    /// 0.0 = avoid risk (always recycle when in doubt); 1.0 = take any
    /// forward chance. High when chasing a goal late; low when leading
    /// late, tired, or playing defensive style. Read by the pass
    /// evaluator to bias forward vs backward passes.
    pub risk_appetite: f32,
    /// How many players (typically defenders) the team wants to keep
    /// behind the ball as rest defence during sustained attack. Falls
    /// when chasing late; rises when leading or playing
    /// counter-attacking style. Used by FB/CB states to decide whether
    /// to overlap or hold.
    pub rest_defense_count: u8,
    /// True for the short window after losing possession during which
    /// the nearest 2-3 players should counter-press instead of falling
    /// back to shape. Equivalent to phase == DefensiveTransition with
    /// a short tail.
    pub counterpress_window: bool,
    /// 0.0 = direct/long-ball when stuck; 1.0 = always recycle and
    /// rebuild. High-possession teams + leads push toward 1.0;
    /// counter-attacking + losing late toward 0.
    pub build_up_patience: f32,
    /// Lateral density signals — how many of OUR players sit in the
    /// left, center, and right thirds (vertically) of the pitch.
    /// Used as a side-overload check by the pass evaluator and by
    /// off-ball movement to avoid bunching.
    pub side_density_left: u8,
    pub side_density_center: u8,
    pub side_density_right: u8,
}

impl TeamTacticalState {
    pub fn initial() -> Self {
        TeamTacticalState {
            phase: GamePhase::MidBlock,
            possession_ticks: 0,
            ticks_since_turnover: 0,
            ball_zone: BallZone::MiddleThird,
            ball_side: BallSideZone::Center,
            in_possession: false,
            defensive_line_x: 0.0,
            game_management_intensity: 0.0,
            press_intensity: 0.5,
            compactness_target: 0.5,
            team_width_target: 0.5,
            tempo: 0.5,
            risk_appetite: 0.5,
            rest_defense_count: 4,
            counterpress_window: false,
            build_up_patience: 0.5,
            side_density_left: 4,
            side_density_center: 3,
            side_density_right: 4,
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

    /// True if this team is in the build-up phase (own ball, own third).
    pub fn is_build_up(&self) -> bool {
        matches!(self.phase, GamePhase::BuildUp)
    }

    /// True if this team is settled in attacking third with the ball
    /// (or in the immediate transition window into it).
    pub fn is_attacking(&self) -> bool {
        matches!(
            self.phase,
            GamePhase::Attack | GamePhase::AttackingTransition
        )
    }

    /// Recompute both teams' tactical state in-place. Called periodically
    /// from the match tick loop (every ~10 ticks is enough — phase shifts
    /// settle over multiple seconds, not every frame).
    pub fn refresh(home: &mut Self, away: &mut Self, inputs: &TacticalRefreshInputs<'_>) {
        let field = inputs.field;
        let field_width = field.size.width as f32;
        let field_height = field.size.height as f32;
        let ball_x = field.ball.position.x;
        let ball_y = field.ball.position.y;

        // Determine which side the ball owner plays on. If no owner,
        // keep the previous possession flag (we're in a loose-ball
        // moment and the prior team still has "last touch" status).
        let owning_team_id = field
            .ball
            .current_owner
            .and_then(|id| field.players.iter().find(|p| p.id == id))
            .map(|p| p.team_id);

        let prev_home_possession = home.in_possession;
        let home_now_has_ball = match owning_team_id {
            Some(id) => id == inputs.home_team_id,
            None => prev_home_possession,
        };
        let away_now_has_ball = match owning_team_id {
            Some(id) => id != inputs.home_team_id,
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
            home.ticks_since_turnover
                .saturating_add(inputs.tick_interval)
        };
        away.ticks_since_turnover = if away_turned_over {
            0
        } else {
            away.ticks_since_turnover
                .saturating_add(inputs.tick_interval)
        };

        home.possession_ticks = if home_now_has_ball {
            home.possession_ticks.saturating_add(inputs.tick_interval)
        } else {
            0
        };
        away.possession_ticks = if away_now_has_ball {
            away.possession_ticks.saturating_add(inputs.tick_interval)
        } else {
            0
        };

        // Ball zones from each team's perspective.
        home.ball_zone = BallZone::for_side(field_width, ball_x, PlayerSide::Left);
        away.ball_zone = BallZone::for_side(field_width, ball_x, PlayerSide::Right);
        let side_zone = BallSideZone::for_y(field_height, ball_y);
        home.ball_side = side_zone;
        away.ball_side = side_zone;

        // ── No-phase-dependency signals first ────────────────────────
        // game_management_intensity, risk_appetite and build_up_patience
        // depend on score / time / ability / tactic — none of them on
        // phase. Compute them up-front so the phase decision can use
        // build_up_patience to size its transition window.
        let minute = (inputs.match_time_ms as f32) / 60_000.0;
        home.game_management_intensity = Self::compute_game_management_intensity(
            inputs.home_score_diff,
            minute,
            inputs.home_avg_ability,
            inputs.away_avg_ability,
        );
        away.game_management_intensity = Self::compute_game_management_intensity(
            -inputs.home_score_diff,
            minute,
            inputs.away_avg_ability,
            inputs.home_avg_ability,
        );

        let home_pressing = inputs.home_tactics.pressing_intensity();
        let home_counter_press = inputs.home_tactics.counter_press_intensity();
        let home_compact = inputs.home_tactics.compactness();
        let away_pressing = inputs.away_tactics.pressing_intensity();
        let away_counter_press = inputs.away_tactics.counter_press_intensity();
        let away_compact = inputs.away_tactics.compactness();

        home.risk_appetite = Self::compute_risk_appetite(
            inputs.home_score_diff,
            minute,
            home.game_management_intensity,
            home_pressing,
        );
        away.risk_appetite = Self::compute_risk_appetite(
            -inputs.home_score_diff,
            minute,
            away.game_management_intensity,
            away_pressing,
        );

        home.build_up_patience = Self::compute_build_up_patience(
            home_pressing,
            home_counter_press,
            home.game_management_intensity,
            home.risk_appetite,
        );
        away.build_up_patience = Self::compute_build_up_patience(
            away_pressing,
            away_counter_press,
            away.game_management_intensity,
            away.risk_appetite,
        );

        // ── Phase ────────────────────────────────────────────────────
        // Use per-team transition windows derived from the just-computed
        // patience and tactic signals.
        let home_attack_window = Self::attacking_transition_window_ticks(home.build_up_patience);
        let away_attack_window = Self::attacking_transition_window_ticks(away.build_up_patience);
        let home_def_window = Self::defensive_transition_window_ticks(home_counter_press);
        let away_def_window = Self::defensive_transition_window_ticks(away_counter_press);

        home.phase = Self::compute_phase(
            home.in_possession,
            home.ball_zone,
            home.ticks_since_turnover,
            home.possession_ticks,
            inputs.coach_wants_high_press_home,
            home_attack_window,
            home_def_window,
        );
        away.phase = Self::compute_phase(
            away.in_possession,
            away.ball_zone,
            away.ticks_since_turnover,
            away.possession_ticks,
            inputs.coach_wants_high_press_away,
            away_attack_window,
            away_def_window,
        );

        // Defensive line x: interpolate between "deep" (own third
        // boundary) and "high" (opponent's half) based on phase. Gives
        // defenders a shared reference frame for how high to push.
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

        // ── Phase-dependent signals ──────────────────────────────────
        home.press_intensity = Self::compute_press_intensity(
            home_pressing,
            home_counter_press,
            inputs.coach_wants_high_press_home,
            inputs.home_avg_condition,
            home.game_management_intensity,
            home.is_defensive_transition(),
        );
        away.press_intensity = Self::compute_press_intensity(
            away_pressing,
            away_counter_press,
            inputs.coach_wants_high_press_away,
            inputs.away_avg_condition,
            away.game_management_intensity,
            away.is_defensive_transition(),
        );

        home.compactness_target =
            Self::compute_compactness(home_compact, home.phase, home.game_management_intensity);
        away.compactness_target =
            Self::compute_compactness(away_compact, away.phase, away.game_management_intensity);

        home.team_width_target = Self::compute_team_width(home_compact, home.phase);
        away.team_width_target = Self::compute_team_width(away_compact, away.phase);

        home.tempo = Self::compute_tempo(
            home_pressing,
            home_counter_press,
            home.phase,
            home.game_management_intensity,
        );
        away.tempo = Self::compute_tempo(
            away_pressing,
            away_counter_press,
            away.phase,
            away.game_management_intensity,
        );

        home.rest_defense_count = Self::compute_rest_defense_count(
            inputs.home_tactics.defender_count(),
            home.phase,
            inputs.home_score_diff,
            minute,
        );
        away.rest_defense_count = Self::compute_rest_defense_count(
            inputs.away_tactics.defender_count(),
            away.phase,
            -inputs.home_score_diff,
            minute,
        );

        home.counterpress_window = home.is_defensive_transition();
        away.counterpress_window = away.is_defensive_transition();

        // Side density: count own players on left/center/right thirds
        // of the pitch (vertically). Cheap O(N=22) pass.
        let mut h_left = 0u16;
        let mut h_center = 0u16;
        let mut h_right = 0u16;
        let mut a_left = 0u16;
        let mut a_center = 0u16;
        let mut a_right = 0u16;
        let third_h = field_height / 3.0;
        for p in field.players.iter().filter(|p| !p.is_sent_off) {
            let zone = if p.position.y < third_h {
                0
            } else if p.position.y > field_height - third_h {
                2
            } else {
                1
            };
            let is_home = p.team_id == inputs.home_team_id;
            match (is_home, zone) {
                (true, 0) => h_left += 1,
                (true, 1) => h_center += 1,
                (true, 2) => h_right += 1,
                (false, 0) => a_left += 1,
                (false, 1) => a_center += 1,
                (false, 2) => a_right += 1,
                _ => {}
            }
        }
        home.side_density_left = h_left.min(11) as u8;
        home.side_density_center = h_center.min(11) as u8;
        home.side_density_right = h_right.min(11) as u8;
        away.side_density_left = a_left.min(11) as u8;
        away.side_density_center = a_center.min(11) as u8;
        away.side_density_right = a_right.min(11) as u8;
    }

    // ──────────────────────────────────────────────────────────────────
    // Pure compute helpers — no side effects, easy to unit-test. Kept
    // as associated functions on `TeamTacticalState` so all the team-
    // level math lives behind a single struct boundary instead of as
    // free helpers floating in the module.
    // ──────────────────────────────────────────────────────────────────

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
    fn compute_phase(
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

    /// Game-management intensity from this team's perspective.
    /// Continuous [0.0, 1.0] signal driving "hold the score" behaviour:
    /// safer passes, slower tempo, hold the ball. A single scalar
    /// covers every case — strong team protecting a lead, underdog
    /// clinging to a narrow upset, team settling for a point late —
    /// instead of hard branching per scenario.
    fn compute_game_management_intensity(
        score_diff: i8,
        minute: f32,
        my_avg_ability: u16,
        opp_avg_ability: u16,
    ) -> f32 {
        // Ramp up from minute 60 onward; max at minute 90.
        let late_factor = ((minute - 60.0).max(0.0) / 30.0).min(1.0);
        // Positive = we're the weaker side. 40 CA ≈ one league tier.
        let ability_gap =
            ((opp_avg_ability as f32 - my_avg_ability as f32) / 40.0).clamp(-1.0, 1.0);

        if score_diff > 0 {
            // Leading — defend the score. Lead_base 0.22 + late_bonus
            // 0.48 puts an equal-squad 1-goal lead at minute 90 at 0.70,
            // crossing the 0.55 prefer_possession threshold so coaches
            // and per-tick decisions both shift to "protect".
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

    /// Press intensity — how aggressively we hunt the ball when we
    /// don't have it. Combines tactical style, coach intent, fatigue,
    /// and game state. Pure function so it's testable.
    ///
    /// A team only "presses" when it has the energy and stylistic
    /// intent. A tired late-lead team should sit off; a fresh attacking
    /// 4-3-3 with PushForward instruction should hunt.
    fn compute_press_intensity(
        tactic_pressing: f32,
        counter_press: f32,
        coach_high_press: bool,
        avg_condition: f32,
        game_management_intensity: f32,
        in_defensive_transition: bool,
    ) -> f32 {
        // Counter-press: short burst right after losing possession.
        // Even a defensive tactic does this for a few seconds.
        let base = if in_defensive_transition {
            tactic_pressing.max(counter_press * 0.95)
        } else {
            tactic_pressing
        };

        // Coach instruction can push press up but never below the
        // tactical floor.
        let with_coach = if coach_high_press {
            (base + 0.15).min(1.0)
        } else {
            base
        };

        // Fatigue penalty: condition < 0.4 strongly suppresses press;
        // full condition leaves it untouched. 1.0 at cond ≥ 0.7, 0.4
        // at cond = 0.0.
        let fatigue_mult = (0.4 + (avg_condition / 0.7).min(1.0) * 0.6).clamp(0.4, 1.0);

        // Game management: protecting a lead late = sit off.
        let gm_mult = (1.0 - game_management_intensity * 0.55).clamp(0.45, 1.0);

        (with_coach * fatigue_mult * gm_mult).clamp(0.0, 1.0)
    }

    /// Compactness target — how tight the shape should be vertically
    /// and horizontally. Rises in defensive phases and falls in attack
    /// (need width to stretch defenders).
    fn compute_compactness(
        tactic_compactness: f32,
        phase: GamePhase,
        game_management_intensity: f32,
    ) -> f32 {
        let phase_bias: f32 = match phase {
            GamePhase::LowBlock => 0.20,
            GamePhase::MidBlock | GamePhase::DefensiveTransition => 0.10,
            GamePhase::Attack | GamePhase::AttackingTransition => -0.15,
            GamePhase::HighPress => -0.05,
            GamePhase::BuildUp | GamePhase::Progression => 0.0,
        };
        (tactic_compactness + phase_bias + game_management_intensity * 0.15).clamp(0.0, 1.0)
    }

    /// Width target — how spread out we want to be laterally. Inverse
    /// of compactness, with a phase bias that pushes width up in attack
    /// and down when defending.
    fn compute_team_width(tactic_compactness: f32, phase: GamePhase) -> f32 {
        let base_width = (1.0 - tactic_compactness).clamp(0.0, 1.0);
        let phase_bias: f32 = match phase {
            GamePhase::Attack => 0.15,
            GamePhase::Progression | GamePhase::AttackingTransition => 0.05,
            GamePhase::BuildUp => 0.10, // CBs split, full-backs push wide
            GamePhase::LowBlock => -0.20,
            GamePhase::MidBlock => -0.10,
            GamePhase::DefensiveTransition => -0.05,
            GamePhase::HighPress => 0.0,
        };
        (base_width + phase_bias).clamp(0.0, 1.0)
    }

    /// Tempo — how fast we want to play. Counter-attack and
    /// transitions are high tempo; possession styles and game
    /// management are slow.
    fn compute_tempo(
        tactic_pressing: f32,
        counter_press: f32,
        phase: GamePhase,
        game_management_intensity: f32,
    ) -> f32 {
        // Default tempo from style — pressing and counter-attack
        // tactics play faster.
        let style_tempo = (tactic_pressing * 0.5 + counter_press * 0.5).clamp(0.0, 1.0);
        let phase_tempo: f32 = match phase {
            GamePhase::AttackingTransition | GamePhase::DefensiveTransition => 0.95,
            GamePhase::Attack | GamePhase::HighPress => 0.75,
            GamePhase::Progression => 0.55,
            GamePhase::BuildUp => 0.35,
            GamePhase::MidBlock | GamePhase::LowBlock => 0.40,
        };
        let blended = style_tempo * 0.4 + phase_tempo * 0.6;
        // Game-management drags tempo down — protecting a lead = slow.
        (blended - game_management_intensity * 0.40).clamp(0.10, 1.0)
    }

    /// Risk appetite — willingness to take a forward pass / shot when
    /// the safe option exists. Low when leading or game-managing; HIGH
    /// when chasing late.
    fn compute_risk_appetite(
        score_diff: i8,
        minute: f32,
        game_management_intensity: f32,
        tactic_pressing: f32,
    ) -> f32 {
        // Base from tactic — attacking tactics start more risk-tolerant.
        let base = 0.45 + tactic_pressing * 0.20;
        // Chasing late = take risks. Symmetric with game-management.
        let chasing_factor = if score_diff < 0 {
            let late_factor = ((minute - 60.0).max(0.0) / 30.0).min(1.0);
            let deficit = (-score_diff).min(3) as f32;
            (0.20 + deficit * 0.10) * (0.4 + late_factor * 0.6)
        } else {
            0.0
        };
        let base_with_chase = base + chasing_factor;
        // Game-management directly suppresses risk. A 0.7 GM signal
        // cuts risk_appetite roughly in half.
        (base_with_chase - game_management_intensity * 0.55).clamp(0.05, 1.0)
    }

    /// Rest-defence count — how many players the team keeps as a
    /// safety shield behind the ball during a settled attack. Function
    /// of the number of nominal defenders, the phase, and game state.
    fn compute_rest_defense_count(
        nominal_defender_count: usize,
        phase: GamePhase,
        score_diff: i8,
        minute: f32,
    ) -> u8 {
        let base = nominal_defender_count.clamp(2, 5) as i8;
        let phase_delta: i8 = match phase {
            // Sustained attack — pull one defender forward to overload.
            GamePhase::Attack => -1,
            GamePhase::AttackingTransition | GamePhase::HighPress => -1,
            // Defending — everyone at home.
            GamePhase::LowBlock => 1,
            _ => 0,
        };
        // Chasing late — sacrifice a defender to push for the goal.
        let chasing_delta: i8 = if score_diff < 0 && minute > 75.0 {
            -1
        } else if score_diff > 0 && minute > 75.0 {
            1
        } else {
            0
        };
        (base + phase_delta + chasing_delta).clamp(2, 5) as u8
    }

    /// Build-up patience — how willing we are to recycle when forward
    /// progress is hard. High in possession styles + leading; low in
    /// counter-attack / chasing.
    fn compute_build_up_patience(
        tactic_pressing: f32,
        counter_press: f32,
        game_management_intensity: f32,
        risk_appetite: f32,
    ) -> f32 {
        // Possession style: counter_press elevated relative to
        // pressing intensity. Counter-attack: counter_press low.
        let possession_signal = (counter_press - tactic_pressing.min(counter_press)).max(0.0);
        let base = 0.45 + possession_signal * 0.40;
        let gm_bonus = game_management_intensity * 0.30;
        let risk_penalty = (1.0 - risk_appetite) * 0.10; // risk-averse → patient
        (base + gm_bonus + risk_penalty).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn losing_side_never_manages_the_game() {
        assert_eq!(
            TeamTacticalState::compute_game_management_intensity(-1, 85.0, 120, 140),
            0.0
        );
        assert_eq!(
            TeamTacticalState::compute_game_management_intensity(-2, 30.0, 150, 150),
            0.0
        );
    }

    #[test]
    fn early_small_lead_produces_mild_signal() {
        let v = TeamTacticalState::compute_game_management_intensity(1, 20.0, 140, 140);
        assert!(v > 0.0 && v < 0.25, "got {v}");
    }

    #[test]
    fn weaker_side_protecting_late_lead_parks_the_bus() {
        let strong_even = TeamTacticalState::compute_game_management_intensity(1, 85.0, 150, 150);
        let weak_late = TeamTacticalState::compute_game_management_intensity(1, 85.0, 110, 150);
        assert!(
            weak_late > strong_even,
            "weak_late={weak_late} strong_even={strong_even}"
        );
        assert!(weak_late > 0.5, "got {weak_late}");
    }

    #[test]
    fn weaker_side_late_draw_plays_for_point() {
        let v = TeamTacticalState::compute_game_management_intensity(0, 85.0, 110, 150);
        assert!(v > 0.0 && v < 0.5, "got {v}");
    }

    #[test]
    fn intensity_is_clamped_below_one() {
        let v = TeamTacticalState::compute_game_management_intensity(5, 90.0, 100, 160);
        assert!(v <= 0.95, "got {v}");
    }

    #[test]
    fn fresh_attacking_press_is_high() {
        // Pressing tactic, fresh players, no game management → high.
        let v = TeamTacticalState::compute_press_intensity(1.0, 0.9, false, 0.9, 0.0, false);
        assert!(v > 0.85, "got {v}");
    }

    #[test]
    fn tired_team_presses_less_than_fresh() {
        let fresh = TeamTacticalState::compute_press_intensity(0.8, 0.7, false, 0.9, 0.0, false);
        let tired = TeamTacticalState::compute_press_intensity(0.8, 0.7, false, 0.30, 0.0, false);
        assert!(tired < fresh, "tired={tired} fresh={fresh}");
    }

    #[test]
    fn late_lead_suppresses_press() {
        let normal = TeamTacticalState::compute_press_intensity(0.8, 0.7, false, 0.9, 0.0, false);
        let leading = TeamTacticalState::compute_press_intensity(0.8, 0.7, false, 0.9, 0.7, false);
        assert!(leading < normal, "leading={leading} normal={normal}");
    }

    #[test]
    fn defensive_transition_boosts_press() {
        // Even a defensive tactic counter-presses briefly.
        let v = TeamTacticalState::compute_press_intensity(0.3, 0.85, false, 0.9, 0.0, true);
        assert!(v > 0.5, "got {v}");
    }

    #[test]
    fn compactness_rises_in_low_block() {
        let attacking = TeamTacticalState::compute_compactness(0.5, GamePhase::Attack, 0.0);
        let low_block = TeamTacticalState::compute_compactness(0.5, GamePhase::LowBlock, 0.0);
        assert!(low_block > attacking);
    }

    #[test]
    fn width_rises_in_attack() {
        let attacking = TeamTacticalState::compute_team_width(0.5, GamePhase::Attack);
        let low_block = TeamTacticalState::compute_team_width(0.5, GamePhase::LowBlock);
        assert!(attacking > low_block);
    }

    #[test]
    fn tempo_high_in_transition_low_in_buildup() {
        let trans = TeamTacticalState::compute_tempo(0.6, 0.6, GamePhase::AttackingTransition, 0.0);
        let build = TeamTacticalState::compute_tempo(0.6, 0.6, GamePhase::BuildUp, 0.0);
        assert!(trans > build, "trans={trans} build={build}");
    }

    #[test]
    fn game_management_drops_tempo() {
        let normal = TeamTacticalState::compute_tempo(0.6, 0.6, GamePhase::Progression, 0.0);
        let managing = TeamTacticalState::compute_tempo(0.6, 0.6, GamePhase::Progression, 0.7);
        assert!(managing < normal);
    }

    #[test]
    fn risk_appetite_rises_when_chasing_late() {
        let drawn = TeamTacticalState::compute_risk_appetite(0, 80.0, 0.0, 0.6);
        let chasing = TeamTacticalState::compute_risk_appetite(-2, 80.0, 0.0, 0.6);
        assert!(chasing > drawn, "chasing={chasing} drawn={drawn}");
    }

    #[test]
    fn risk_appetite_falls_when_leading_late() {
        let normal = TeamTacticalState::compute_risk_appetite(0, 80.0, 0.0, 0.6);
        let leading = TeamTacticalState::compute_risk_appetite(1, 85.0, 0.7, 0.6);
        assert!(leading < normal, "leading={leading} normal={normal}");
    }

    #[test]
    fn rest_defense_drops_when_chasing_late() {
        let normal =
            TeamTacticalState::compute_rest_defense_count(4, GamePhase::Progression, 0, 60.0);
        let chasing_late =
            TeamTacticalState::compute_rest_defense_count(4, GamePhase::Attack, -1, 85.0);
        assert!(chasing_late < normal);
    }

    #[test]
    fn rest_defense_rises_when_leading_late() {
        let normal =
            TeamTacticalState::compute_rest_defense_count(4, GamePhase::Progression, 0, 60.0);
        let leading_late =
            TeamTacticalState::compute_rest_defense_count(4, GamePhase::LowBlock, 1, 85.0);
        assert!(leading_late >= normal);
    }

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

    #[test]
    fn build_up_patience_higher_in_possession_style_with_lead() {
        let direct = TeamTacticalState::compute_build_up_patience(0.9, 0.4, 0.0, 0.6);
        let possession_lead = TeamTacticalState::compute_build_up_patience(0.4, 0.9, 0.7, 0.3);
        assert!(
            possession_lead > direct,
            "poss_lead={possession_lead} direct={direct}"
        );
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
