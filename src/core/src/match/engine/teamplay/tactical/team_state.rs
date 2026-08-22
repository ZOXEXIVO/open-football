//! **The shared state itself.** [`TeamTacticalState`] — the plain-POD
//! block of scalars all eleven players on a side read — and the
//! `refresh` pass that recomputes both sides' copies once per tactical
//! tick.
//!
//! `refresh` is the only mutator: it reads the world through
//! [`TacticalRefreshInputs`], calls the pure math in
//! [`phase`](super::phase), [`signals`](super::signals) and
//! [`quality`](super::quality), and writes the result here.

use crate::r#match::engine::teamplay::tactical::inputs::TacticalRefreshInputs;
use crate::r#match::engine::teamplay::tactical::phase::{BallSideZone, BallZone, GamePhase};

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

    /// Whether this phase is one where committing players into the
    /// opposition box is the right call. Broader than
    /// [`is_attacking`](Self::is_attacking): a move is built in
    /// `Progression`, and the runs that arrive in the box have to start
    /// before the ball reaches the final third or they arrive late.
    /// Read by [`AttackPlan::refresh`](crate::r#match::AttackPlan::refresh).
    pub fn wants_bodies_forward(&self) -> bool {
        self.in_possession
            && matches!(
                self.phase,
                GamePhase::Attack | GamePhase::AttackingTransition | GamePhase::Progression
            )
    }

    /// Recompute both teams' tactical state in-place. Called periodically
    /// from the match tick loop (every ~10 ticks is enough — phase shifts
    /// settle over multiple seconds, not every frame).
    pub fn refresh(home: &mut Self, away: &mut Self, inputs: &TacticalRefreshInputs<'_>) {
        // Every team-quality read below goes through this. The skill
        // gates are absolute thresholds on a quantity that scales with
        // the division — see `TacticalRefreshInputs::standard_shift` — so
        // without it a fourth-tier side presses 35% softer and sits 8% of
        // the pitch deeper than the identical side one division up, for
        // no footballing reason: a manager sets his line and his press
        // against the eleven men in front of him.
        let peer = |q: f32| (q - inputs.standard_shift).clamp(0.0, 1.0);
        let gk_peer = |q: f32| (q - inputs.standard_gk_shift).clamp(0.0, 1.0);
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

        // Ball zones from each team's perspective — read off the end each
        // side is actually defending, not off a first-half assumption.
        let home_side = inputs.home_side;
        let away_side = home_side.opposite();
        home.ball_zone = BallZone::for_side(field_width, ball_x, home_side);
        away.ball_zone = BallZone::for_side(field_width, ball_x, away_side);
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

        // Skill-aware nudges to build-up patience: a side that can
        // genuinely play out (high passing-execution composite) buys
        // more recycle time; a side full of hoof-it CBs ought to bias
        // toward direct outlets.
        home.build_up_patience = Self::skill_adjusted_build_up_patience(
            home.build_up_patience,
            peer(inputs.home_skills.build_up_quality),
        );
        away.build_up_patience = Self::skill_adjusted_build_up_patience(
            away.build_up_patience,
            peer(inputs.away_skills.build_up_quality),
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
        //
        // Expressed as attacking PROGRESS (0 = own goal line, 1 = theirs)
        // and mapped back onto the pitch through the side each team is
        // actually defending. The two hand-mirrored constant tables this
        // replaces disagreed with each other — the away side's `BuildUp`
        // was 0.75 (= 0.25 progress, matching home) but its `MidBlock` was
        // `field_width - third` (= 0.333 progress) against home's `third`
        // (also 0.333), so they only happened to agree while home was
        // Left. After the half-time swap both tables were read off the
        // wrong end.
        let base_line_progress = |phase: GamePhase| -> f32 {
            match phase {
                GamePhase::HighPress | GamePhase::Attack => 0.55,
                GamePhase::AttackingTransition | GamePhase::Progression => 0.45,
                GamePhase::BuildUp => 0.25,
                GamePhase::MidBlock | GamePhase::DefensiveTransition => 1.0 / 3.0,
                GamePhase::LowBlock => 0.18,
            }
        };
        let home_base_line = home_side.x_at_progress(base_line_progress(home.phase), field_width);
        let away_base_line = away_side.x_at_progress(base_line_progress(away.phase), field_width);
        // Defensive-quality risk drop: weak back lines lower their
        // line by up to 0.06 of the pitch so a shaky 5-rated CB pair
        // doesn't get caught chasing through-balls they can't recover.
        // Home plays toward x=high, away toward x=low — invert the
        // sign accordingly.
        let home_line_drop_units =
            Self::line_height_drop(peer(inputs.home_skills.defensive_quality), field_width);
        let away_line_drop_units =
            Self::line_height_drop(peer(inputs.away_skills.defensive_quality), field_width);
        // GK-quality lift: a sweeper-keeper-class GK lets us play a
        // higher line than skill on the back four alone would warrant.
        // Bounded to +0.02 of pitch width so it's a tweak, not a
        // dominant signal.
        let home_gk_lift = Self::gk_line_lift(gk_peer(inputs.home_skills.gk_quality), field_width);
        let away_gk_lift = Self::gk_line_lift(gk_peer(inputs.away_skills.gk_quality), field_width);
        // Game-management line drop: a side protecting a result actually
        // sits DEEPER, not just slower. Before this, game management only
        // reduced the leader's own tempo/risk/press — their block stayed
        // at phase height, so "parking the bus" parked nothing: the
        // chasing side's risk lift met an unchanged defensive structure
        // and conceding teams scored the next goal 72% of the time over
        // 10+ minute horizons (equal-strength dev_match), feeding the
        // engine's chronic equalizer/draw inflation. Up to 8% of pitch
        // width at full GM pulls extra bodies between ball and goal so
        // the existing shot-clarity, block-corridor and xG channels make
        // the lead genuinely harder to break down — defense by geometry,
        // not by a hidden modifier.
        // History: 0.08 → 0.12 → 0.05. The deeper block A/B-measured
        // COUNTERPRODUCTIVE in this engine: depth invites permanent
        // siege territory, and siege volume (more willingness rolls
        // against) outweighs the chance-quality reduction the extra
        // bodies buy — post-62' trailing rate went UP, not down. The
        // engine's effective lead-protection is a MODERATE line (keep
        // the trailer's build-up further from goal) plus the counter
        // window; not a six-yard-box bus.
        let home_gm_drop = field_width * 0.05 * home.game_management_intensity;
        let away_gm_drop = field_width * 0.05 * away.game_management_intensity;
        // Drops pull the line toward that side's OWN goal and the GK lift
        // pushes it away from it, so both are signed by the side's
        // attacking direction rather than written out twice by hand.
        home.defensive_line_x = (home_base_line
            + home_side.forward_dir_x() * (home_gk_lift - home_line_drop_units - home_gm_drop))
            .clamp(0.0, field_width);
        away.defensive_line_x = (away_base_line
            + away_side.forward_dir_x() * (away_gk_lift - away_line_drop_units - away_gm_drop))
            .clamp(0.0, field_width);

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
        // High-press sustainability: when press_quality < 0.45, scale
        // the press intensity by up to 35% of the deficit so an
        // unfit/under-skilled side cannot run a hopelessly hot press
        // even if the coach asked for one.
        home.press_intensity = Self::press_skill_adjustment(
            home.press_intensity,
            peer(inputs.home_skills.press_quality),
        );
        away.press_intensity = Self::press_skill_adjustment(
            away.press_intensity,
            peer(inputs.away_skills.press_quality),
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

        // ── Home advantage (play-quality half) ───────────────────────
        // Real equal-strength matches split ~45/25/30 toward the home
        // side — home teams take ~15-20% more shots and score ~+0.35
        // goals, driven by crowd-backed front-foot play and away-side
        // caution. Until now the engine modelled NONE of this in play
        // (only the referee marginal-call bias), so equal teams played
        // neutral-venue football and piled up draws. The edge scales
        // continuously with `crowd_intensity × home_advantage` from the
        // match environment: a full derby crowd pushes the home side
        // visibly forward, an empty-stadium friendly does nothing.
        // Kept SMALL: a 3× sign-test (press +0.40×edge etc.) measured no
        // home-outcome shift at all — attacking-volume signals don't
        // convert to wins in this engine (winning teams take FEWER
        // shots; score-state drives volume, not the reverse). The
        // outcome-bearing half of home advantage therefore lives in the
        // `crowd_arousal` effective-skill multiplier (player.rs /
        // effective_skill.rs); this block just adds the visible
        // front-foot flavour — home sides pressing a touch higher and
        // playing a touch quicker — without pretending to be the edge.
        let home_edge = inputs.home_edge.clamp(0.0, 1.0);
        if home_edge > 0.0 {
            home.press_intensity = (home.press_intensity + 0.14 * home_edge).clamp(0.0, 1.0);
            home.risk_appetite = (home.risk_appetite + 0.10 * home_edge).clamp(0.0, 1.0);
            home.tempo = (home.tempo + 0.06 * home_edge).clamp(0.10, 1.0);
            // Away side: slightly more cautious on the road — lower
            // risk, a touch less press. Smaller than the home lift so
            // the NET effect matches the documented home edge without
            // turning away sides passive.
            away.risk_appetite = (away.risk_appetite - 0.06 * home_edge).clamp(0.0, 1.0);
            away.press_intensity = (away.press_intensity - 0.05 * home_edge).clamp(0.0, 1.0);
        }

        // Attacking-quality bias: a side with elite finishers chasing
        // a goal late should bias slightly more direct (higher tempo
        // + risk appetite). Bounded to ±0.05 each so it tunes existing
        // signals rather than driving them. A losing weak attacking
        // side does NOT get a free kicker bonus — only sides good
        // enough to actually convert get to play more direct.
        let home_chasing = inputs.home_score_diff < 0;
        let away_chasing = inputs.home_score_diff > 0;
        if home_chasing {
            let lift =
                Self::attacking_chase_lift(peer(inputs.home_skills.attacking_quality), minute);
            home.tempo = (home.tempo + lift).clamp(0.10, 1.0);
            home.risk_appetite = (home.risk_appetite + lift).clamp(0.0, 1.0);
        }
        if away_chasing {
            let lift =
                Self::attacking_chase_lift(peer(inputs.away_skills.attacking_quality), minute);
            away.tempo = (away.tempo + lift).clamp(0.10, 1.0);
            away.risk_appetite = (away.risk_appetite + lift).clamp(0.0, 1.0);
        }

        // Protect-lead damping: a leading side with concentrated /
        // high-teamwork players is less prone to "panic" tempo spikes
        // and rash forward passes. We damp the tempo slightly when
        // game-management intensity is high AND the side organises
        // well. The 0.85..1.05 factor keeps the effect subtle.
        if home.game_management_intensity > 0.05 {
            let damp =
                Self::protect_lead_damping(peer(inputs.home_skills.concentration_teamwork_avg));
            // Scale only the protect-lead delta, not the whole tempo.
            let delta = damp - 1.0; // negative when damping
            home.tempo = (home.tempo + delta * home.game_management_intensity).clamp(0.10, 1.0);
        }
        if away.game_management_intensity > 0.05 {
            let damp =
                Self::protect_lead_damping(peer(inputs.away_skills.concentration_teamwork_avg));
            let delta = damp - 1.0;
            away.tempo = (away.tempo + delta * away.game_management_intensity).clamp(0.10, 1.0);
        }

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
}
