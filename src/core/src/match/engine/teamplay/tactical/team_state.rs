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
use crate::r#match::engine::teamplay::tactical::signals::StyleInputs;

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
    /// The coach's stance as it reaches the levers: −1 holding on to what
    /// the side has … +1 throwing everything forward, already scaled by the
    /// regime's gain and measured from the kickoff stance the plan carries.
    pub stance: f32,
    /// The protecting share of `stance`, 0..1. Read by pass selection
    /// (prefer safe backward / sideways balls).
    pub game_management_intensity: f32,
    /// The plan's appetite for risk before the stance moves it.
    pub plan_risk: f32,
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
            stance: 0.0,
            game_management_intensity: 0.0,
            plan_risk: 0.5,
            press_intensity: 0.5,
            compactness_target: 0.5,
            team_width_target: 0.5,
            tempo: 0.5,
            risk_appetite: 0.5,
            rest_defense_count: 4,
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
        Self::refresh_ball_state(home, away, inputs);
        let (home_style, away_style, home_high_press, away_high_press) =
            Self::refresh_stance_and_styles(home, away, inputs);
        Self::refresh_phase(
            home,
            away,
            home_style,
            away_style,
            home_high_press,
            away_high_press,
        );
        Self::refresh_defensive_line(home, away, inputs, home_style, away_style);
        Self::refresh_phase_signals(
            home,
            away,
            inputs,
            home_style,
            away_style,
            home_high_press,
            away_high_press,
        );
        Self::apply_home_edge(home, away, inputs.home_edge);
        Self::clamp_press_and_rest(home, away, inputs, home_style, away_style);
        Self::refresh_side_density(home, away, inputs);
    }

    fn refresh_ball_state(home: &mut Self, away: &mut Self, inputs: &TacticalRefreshInputs<'_>) {
        let field = inputs.field;
        let field_width = field.size.width as f32;
        let field_height = field.size.height as f32;
        let ball_x = field.ball.position.x;
        let ball_y = field.ball.position.y;

        // The shared fact. With nobody on the pitch to say, keep the previous
        // flag.
        let owning_team_id = inputs.possession_holder;

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
    }

    fn refresh_stance_and_styles(
        home: &mut Self,
        away: &mut Self,
        inputs: &TacticalRefreshInputs<'_>,
    ) -> (StyleInputs, StyleInputs, bool, bool) {
        // ── No-phase-dependency signals first ────────────────────────
        // The protecting share of each side's stance, risk appetite and
        // build-up patience depend on the plan and the stance — none of
        // them on phase. Computed up-front so the phase decision can use
        // build_up_patience to size its transition window.
        home.stance = inputs.home_stance;
        away.stance = inputs.away_stance;
        home.game_management_intensity = (-home.stance).max(0.0);
        away.game_management_intensity = (-away.stance).max(0.0);

        let home_style = StyleInputs::of_plan(&inputs.home_plan);
        let away_style = StyleInputs::of_plan(&inputs.away_plan);

        home.risk_appetite = Self::compute_risk_appetite(home_style.risk, home.stance);
        away.risk_appetite = Self::compute_risk_appetite(away_style.risk, away.stance);
        home.plan_risk = home_style.risk;
        away.plan_risk = away_style.risk;

        home.build_up_patience =
            Self::compute_build_up_patience(home_style.patience, home.stance, home.risk_appetite);
        away.build_up_patience =
            Self::compute_build_up_patience(away_style.patience, away.stance, away.risk_appetite);
        let home_high_press = Self::wants_high_press(home_style.pressing, home.stance);
        let away_high_press = Self::wants_high_press(away_style.pressing, away.stance);

        // Skill-aware nudges to build-up patience: a side that can
        // genuinely play out (high passing-execution composite) buys
        // more recycle time; a side full of hoof-it CBs ought to bias
        // toward direct outlets.
        home.build_up_patience = Self::skill_adjusted_build_up_patience(
            home.build_up_patience,
            inputs.home_skills.build_up_quality,
        );
        away.build_up_patience = Self::skill_adjusted_build_up_patience(
            away.build_up_patience,
            inputs.away_skills.build_up_quality,
        );

        (home_style, away_style, home_high_press, away_high_press)
    }

    fn refresh_phase(
        home: &mut Self,
        away: &mut Self,
        home_style: StyleInputs,
        away_style: StyleInputs,
        home_high_press: bool,
        away_high_press: bool,
    ) {
        // ── Phase ────────────────────────────────────────────────────
        // Use per-team transition windows derived from the just-computed
        // patience and tactic signals.
        let home_attack_window = Self::attacking_transition_window_ticks(home.build_up_patience);
        let away_attack_window = Self::attacking_transition_window_ticks(away.build_up_patience);
        let home_def_window = Self::defensive_transition_window_ticks(home_style.counter_press);
        let away_def_window = Self::defensive_transition_window_ticks(away_style.counter_press);

        home.phase = Self::compute_phase(
            home.in_possession,
            home.ball_zone,
            home.ticks_since_turnover,
            home.possession_ticks,
            home_high_press,
            home_attack_window,
            home_def_window,
        );
        away.phase = Self::compute_phase(
            away.in_possession,
            away.ball_zone,
            away.ticks_since_turnover,
            away.possession_ticks,
            away_high_press,
            away_attack_window,
            away_def_window,
        );
    }

    fn refresh_defensive_line(
        home: &mut Self,
        away: &mut Self,
        inputs: &TacticalRefreshInputs<'_>,
        home_style: StyleInputs,
        away_style: StyleInputs,
    ) {
        let field_width = inputs.field.size.width as f32;
        let home_side = inputs.home_side;
        let away_side = home_side.opposite();

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
        let home_base_line = home_side.x_at_progress(
            Self::line_progress(
                home.phase,
                home_style.line_shift + Self::STANCE_LINE * home.stance,
            ),
            field_width,
        );
        let away_base_line = away_side.x_at_progress(
            Self::line_progress(
                away.phase,
                away_style.line_shift + Self::STANCE_LINE * away.stance,
            ),
            field_width,
        );
        // Defensive-quality risk drop: weak back lines lower their
        // line by up to 0.06 of the pitch so a shaky 5-rated CB pair
        // doesn't get caught chasing through-balls they can't recover.
        // Home plays toward x=high, away toward x=low — invert the
        // sign accordingly.
        let home_line_drop_units =
            Self::line_height_drop(inputs.home_skills.defensive_quality, field_width);
        let away_line_drop_units =
            Self::line_height_drop(inputs.away_skills.defensive_quality, field_width);
        // GK-quality lift: a sweeper-keeper-class GK lets us play a
        // higher line than skill on the back four alone would warrant.
        // Bounded to +0.02 of pitch width so it's a tweak, not a
        // dominant signal.
        let home_gk_lift = Self::gk_line_lift(inputs.home_skills.gk_quality, field_width);
        let away_gk_lift = Self::gk_line_lift(inputs.away_skills.gk_quality, field_width);
        // Drops pull the line toward that side's OWN goal and the GK lift
        // pushes it away from it, so both are signed by the side's
        // attacking direction rather than written out twice by hand.
        home.defensive_line_x = (home_base_line
            + home_side.forward_dir_x() * (home_gk_lift - home_line_drop_units))
            .clamp(0.0, field_width);
        away.defensive_line_x = (away_base_line
            + away_side.forward_dir_x() * (away_gk_lift - away_line_drop_units))
            .clamp(0.0, field_width);
    }

    fn refresh_phase_signals(
        home: &mut Self,
        away: &mut Self,
        inputs: &TacticalRefreshInputs<'_>,
        home_style: StyleInputs,
        away_style: StyleInputs,
        home_high_press: bool,
        away_high_press: bool,
    ) {
        // ── Phase-dependent signals ──────────────────────────────────
        home.press_intensity = Self::compute_press_intensity(
            home_style.pressing,
            home_style.counter_press,
            home_high_press,
            inputs.home_avg_condition,
            home.stance,
            home.is_defensive_transition(),
        );
        away.press_intensity = Self::compute_press_intensity(
            away_style.pressing,
            away_style.counter_press,
            away_high_press,
            inputs.away_avg_condition,
            away.stance,
            away.is_defensive_transition(),
        );
        // High-press sustainability: when press_quality < 0.45, scale
        // the press intensity by up to 35% of the deficit so an
        // unfit/under-skilled side cannot run a hopelessly hot press
        // even if the coach asked for one.
        home.press_intensity =
            Self::press_skill_adjustment(home.press_intensity, inputs.home_skills.press_quality);
        away.press_intensity =
            Self::press_skill_adjustment(away.press_intensity, inputs.away_skills.press_quality);

        home.compactness_target =
            Self::compute_compactness(home_style.compactness, home.phase, home.stance);
        away.compactness_target =
            Self::compute_compactness(away_style.compactness, away.phase, away.stance);

        home.team_width_target = Self::compute_team_width(home_style.width, home.phase);
        away.team_width_target = Self::compute_team_width(away_style.width, away.phase);

        home.tempo = Self::compute_tempo(home_style.tempo, home.phase);
        away.tempo = Self::compute_tempo(away_style.tempo, away.phase);
    }

    fn apply_home_edge(home: &mut Self, away: &mut Self, home_edge: f32) {
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
        let home_edge = home_edge.clamp(0.0, 1.0);
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
    }

    fn clamp_press_and_rest(
        home: &mut Self,
        away: &mut Self,
        inputs: &TacticalRefreshInputs<'_>,
        home_style: StyleInputs,
        away_style: StyleInputs,
    ) {
        home.press_intensity = home.press_intensity.min(inputs.press_ceiling);
        away.press_intensity = away.press_intensity.min(inputs.press_ceiling);

        home.rest_defense_count =
            Self::compute_rest_defense_count(home_style.rest_defence, home.phase, home.stance);
        away.rest_defense_count =
            Self::compute_rest_defense_count(away_style.rest_defence, away.phase, away.stance);
    }

    fn refresh_side_density(home: &mut Self, away: &mut Self, inputs: &TacticalRefreshInputs<'_>) {
        let field = inputs.field;
        let field_height = field.size.height as f32;

        // Side density: count own players on left/center/right thirds
        // of the pitch (vertically). Cheap O(N=22) pass.
        let mut h_left = 0u16;
        let mut h_center = 0u16;
        let mut h_right = 0u16;
        let mut a_left = 0u16;
        let mut a_center = 0u16;
        let mut a_right = 0u16;
        let third_h = field_height / 3.0;
        for p in field.players.iter().filter(|p| !p.off_pitch) {
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
