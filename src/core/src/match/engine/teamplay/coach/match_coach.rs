//! **The live coach.** [`MatchCoach`] — one per side — holding the
//! current instruction, the possession / shot counters the shooting
//! gate reads, and the rolling metrics window.
//!
//! `evaluate` is the escalation ladder: score, clock and fatigue in,
//! a [`CoachInstruction`] out. Every clock threshold that gates a
//! SCORE-dependent branch in it goes through
//! [`MatchContext::score_reaction_threshold`].

use crate::r#match::MatchContext;
use crate::r#match::engine::teamplay::coach::instruction::{
    CoachInstruction, InstructionCoefficients,
};
use crate::r#match::engine::teamplay::coach::metrics::{MetricSnapshot, RollingTeamMetrics};

/// Per-team coach state during a match
#[derive(Debug, Clone)]
pub struct MatchCoach {
    pub instruction: CoachInstruction,
    /// Tick when instruction was last updated
    pub last_update_tick: u64,
    /// Team's last shot tick (for team-wide shot cooldown)
    pub last_shot_tick: u64,
    /// Tick when this team most recently gained possession. Used as a
    /// build-up gate: teams can't shoot within a short window of winning
    /// the ball, which forces an outlet pass / progression instead of
    /// hack-and-counter. Updated by the match loop on possession-change.
    pub last_possession_gain_tick: u64,
    /// Shots fired in the current possession. Reset when we lose the
    /// ball (possession change TO us, FROM us). Real football: one
    /// quality chance per possession. Rebound / tap-in scrambles
    /// (ball leaves owner briefly but team keeps control) don't
    /// count as a new possession — the cap holds until the opposition
    /// touches the ball.
    pub shots_this_possession: u32,
    /// Rolling tactical metrics — populated by the match loop and read
    /// by `evaluate_with_metrics` for smarter instruction switches.
    pub metrics: RollingTeamMetrics,
    /// Cumulative possession + field-tilt counters, in ticks. Updated
    /// every tactical refresh so the engine doesn't have to walk all
    /// players on every coach eval.
    pub cum_possession_ticks: u32,
    pub cum_field_tilt_ticks: u32,
    /// Rolling-window snapshot for delta computation.
    pub metric_snapshot: MetricSnapshot,
}

impl Default for MatchCoach {
    fn default() -> Self {
        MatchCoach {
            instruction: CoachInstruction::Normal,
            last_update_tick: 0,
            last_shot_tick: 0,
            last_possession_gain_tick: 0,
            shots_this_possession: 0,
            metrics: RollingTeamMetrics::default(),
            cum_possession_ticks: 0,
            cum_field_tilt_ticks: 0,
            metric_snapshot: MetricSnapshot::default(),
        }
    }
}

impl MatchCoach {
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluate match state and decide what instruction to give.
    /// Called periodically (every ~500 ticks = ~5 seconds).
    pub fn evaluate(
        &mut self,
        score_diff: i8,          // positive = leading, negative = losing
        match_progress: f32,     // 0.0 = start, 1.0 = end of match
        avg_team_condition: f32, // 0.0-1.0
        current_tick: u64,
    ) {
        self.last_update_tick = current_tick;

        let _time_remaining = 1.0 - match_progress;
        // The escalation ladder is DISCRETE — a trailing side is on
        // `AllOutAttack` or it is not — so the regime's amplitude knob
        // cannot scale it. What it scales instead is how much of the match
        // is spent past each rung: see
        // `MatchContext::score_reaction_threshold`. At full gain these are
        // the thresholds they have always been.
        let rung = MatchContext::score_reaction_threshold;
        let is_late_game = match_progress > rung(0.75);
        let is_very_late = match_progress > rung(0.88);
        let is_first_half_end = match_progress > 0.45 && match_progress < 0.55;
        let team_tired = avg_team_condition < 0.45;

        self.instruction = match score_diff {
            // Leading by 5+ goals — shut the game down. Previously even a
            // 0-6 leader stayed on `SlowDown` early in the match, which
            // still lets forwards take shots and convert against an
            // already-collapsing defence. `WasteTime` at any clock time
            // strips the attacking urge and keeps the ball at the back.
            d if d >= 5 => CoachInstruction::WasteTime,
            // Leading by 3-4 goals
            d if d >= 3 => {
                if is_late_game {
                    CoachInstruction::WasteTime
                } else {
                    CoachInstruction::SlowDown
                }
            }
            // Leading by 2 goals
            2 => {
                if is_very_late {
                    CoachInstruction::WasteTime
                } else if is_late_game {
                    CoachInstruction::SlowDown
                } else {
                    CoachInstruction::Normal
                }
            }
            // Leading by 1 goal — don't fully park the bus until the final
            // minutes. Parking too early creates 1-0 lock-ins that
            // equalizers turn into draws: with SlowDown from minute ~67
            // the leader stopped creating while the trailer pushed, and
            // equal-strength matches drew 47% (real ~25%). Real 1-goal
            // leaders keep playing past the hour and only start killing
            // the game around minute 80 — so protection now starts at
            // ~minute 75 (progress 0.83) and the bus parks at ~minute 83.
            1 => {
                if match_progress > rung(0.92) {
                    CoachInstruction::ParkTheBus
                } else if match_progress > rung(0.83) {
                    CoachInstruction::SlowDown
                } else if is_first_half_end {
                    CoachInstruction::SlowDown
                } else if team_tired {
                    CoachInstruction::SlowDown
                } else {
                    CoachInstruction::Normal
                }
            }
            // Drawing — push for a winner from the 60th minute, all-out in final 10min
            0 => {
                if is_very_late {
                    CoachInstruction::AllOutAttack
                } else if is_late_game {
                    CoachInstruction::PushForward
                } else if team_tired {
                    CoachInstruction::SlowDown
                } else {
                    CoachInstruction::Normal
                }
            }
            // Losing by 1 — start pushing earlier to reduce draw lock-ins
            -1 => {
                if is_very_late {
                    CoachInstruction::AllOutAttack
                } else if is_late_game {
                    CoachInstruction::AllOutAttack
                } else if match_progress > rung(0.55) {
                    CoachInstruction::PushForward
                } else {
                    CoachInstruction::Normal
                }
            }
            // Losing by 2
            -2 => {
                if is_very_late {
                    CoachInstruction::AllOutAttack
                } else if is_late_game {
                    CoachInstruction::AllOutAttack
                } else if match_progress > rung(0.55) {
                    CoachInstruction::PushForward
                } else {
                    CoachInstruction::Normal
                }
            }
            // Losing by 3-4 — push hard, go all-out late
            d if d >= -4 => {
                if is_late_game {
                    CoachInstruction::AllOutAttack
                } else {
                    CoachInstruction::PushForward
                }
            }
            // Losing by 5+ — the game is gone. `AllOutAttack` from here
            // just kept conceding more because defenders pushed forward
            // into space the leader then counter-attacked through. Accept
            // the damage and hold shape (`PushForward` for chance creation
            // without gutting the back line).
            _ => CoachInstruction::PushForward,
        };
    }

    /// Whether the team should allow a shot right now. **Always yes.**
    ///
    /// 2026-08-16: the three team-level shot gates are REMOVED — a
    /// spacing cooldown (one shot per 7.5 s), a build-up delay (no shot
    /// within 1 s of winning the ball), and a per-possession quota (at
    /// most two attempts).
    ///
    /// None of them has a real-life analogue. Football has no per-team
    /// shot timer and no attempt quota: a side that works three openings
    /// in one spell of pressure takes three shots. What these gates did
    /// was veto a strike on WHO SHOT LAST AND WHEN, without being able to
    /// see the chance at all — so a speculative 25-yarder locked the team
    /// out of a tap-in four seconds later. The note kept below recorded
    /// the symptom without drawing the conclusion: across the shot-bar
    /// titration ON-TARGET shots FELL 3.0 → 2.4 per team while total
    /// shots rose 12.6 → 30.1.
    ///
    /// Shot volume is not theirs to hold down. It belongs to defending —
    /// pressure, blocks, and the quality of the look — which is where the
    /// pressing work will put it. See `SHOT_BAR_BASE` for the rest of the
    /// teardown.
    ///
    /// Kept as a method, and the counters it read (`last_shot_tick`,
    /// `shots_this_possession`, `last_possession_gain_tick`) are still
    /// maintained, so a real constraint can be reintroduced here without
    /// re-plumbing every caller.
    pub fn can_shoot(&self, _current_tick: u64, _rebound_live: bool) -> bool {
        true
    }

    /// The gates this replaced, preserved for the pressing work.
    ///
    /// Their reasoning is sound about SPAM and wrong about football: they
    /// filter by timing rather than by chance quality. If something like
    /// them comes back, it should read the look, not the clock.
    #[allow(dead_code)]
    fn legacy_shot_gates(&self, current_tick: u64, rebound_live: bool) -> bool {
        // Per-team shot cadence — see type docs for the full rationale.
        // 500 → 750 ticks (~7.5s spacing). Briefly tried 1000 but it
        // disproportionately hurt weak teams: they shoot rarely already,
        // and a 10s gate killed too many of their quick-counter chances
        // (rebounds, second-balls in the box) while barely throttling
        // strong sides who already pace their shots. 750 hits the
        // strong side enough to matter without crushing the upset tail.
        //
        // `rebound_live` (a dangerous parry / loose block deflection in
        // the last ~3 s — see `TeamOps::can_shoot`) suspends the
        // spacing and build-up gates: the rebound arrives 0.5-1.5 s
        // after the original shot, so without the exemption the spacing
        // gate blocked EVERY second-chance strike — contradicting the
        // possession-cap design note below, and deleting one of
        // football's core goal patterns (~4-6% of real goals).
        //
        // Suspected 2026-08-13 of costing more than it saves: it filters
        // by TIMING rather than by chance quality, so a speculative
        // 25-yarder locks the team out of a tap-in four seconds later,
        // and across the shot-bar titration ON-TARGET shots FELL 3.0 →
        // 2.4 per team while total shots went 12.6 → 30.1. Tried at 250
        // ticks: goals 1.90 → 2.00 and 0-0 15% → 13%, both inside the
        // n=60 noise floor, and `ball STUCK` went the wrong way (29.9 →
        // 47.8 s/match). Left at 750 for want of evidence — re-test it
        // with n≥300 alongside the chance-supply work.
        let shot_spaced = rebound_live || current_tick.saturating_sub(self.last_shot_tick) >= 750;
        // Build-up gate: a team that just won possession can't fire
        // within ~1 second. Real football: even elite counter-attacks
        // need at least one progressive pass before a shot arrives.
        let settled =
            rebound_live || current_tick.saturating_sub(self.last_possession_gain_tick) >= 100;
        // Possession-phase shot cap: at most TWO shots per possession.
        // Real football: a possession typically produces ONE chance,
        // but rebounds (saved/blocked → ball comes back to attackers)
        // are a real and common path to goals — Klopp-era Liverpool
        // and most pressing teams convert plenty from rebounds.
        // Cap of 1 forbade ALL rebound shots, including legitimate ones
        // where the GK parries to a striker's feet. Cap of 2 paired
        // with the 5s team-shot cooldown still rules out box-scramble
        // spam (4 shots in 2s) but unlocks the realistic "shoot →
        // parry → tap-in" pattern.
        let phase_allows = self.shots_this_possession < 2;
        shot_spaced && settled && phase_allows
    }

    /// Record that this team just won possession. Starts the build-up
    /// gate AND resets the per-possession shot counter.
    pub fn record_possession_gain(&mut self, current_tick: u64) {
        self.last_possession_gain_tick = current_tick;
        self.shots_this_possession = 0;
    }

    pub fn record_shot(&mut self, current_tick: u64) {
        self.last_shot_tick = current_tick;
        self.shots_this_possession += 1;
    }

    /// Returns the active instruction's tactical coefficients (risk,
    /// tempo, defensive-line, width). Consumers use these to bias
    /// scoring decisions without needing to match on `CoachInstruction`
    /// at every call site.
    pub fn coefficients(&self) -> InstructionCoefficients {
        InstructionCoefficients::for_instruction(self.instruction)
    }

    /// xG/territory-aware variant of `evaluate`. Falls back to the
    /// classic score/time/condition logic and then upgrades or
    /// downgrades the choice based on rolling metrics. Real football:
    /// a 0-0 team dominating xG shouldn't go AllOutAttack; a leading
    /// team being tilted should drop deeper rather than just slow down.
    pub fn evaluate_with_metrics(
        &mut self,
        score_diff: i8,
        match_progress: f32,
        avg_team_condition: f32,
        current_tick: u64,
        metrics: RollingTeamMetrics,
    ) {
        self.evaluate(score_diff, match_progress, avg_team_condition, current_tick);
        self.metrics = metrics;

        let xg_diff_15 = metrics.xg_for_last_15 - metrics.xg_against_last_15;
        let rung = MatchContext::score_reaction_threshold;
        let is_late = match_progress > rung(0.66);
        let is_very_late = match_progress > rung(0.83);

        // Drawing but dominating xG → don't blow the shape. Stay on
        // PushForward (or Normal) instead of AllOutAttack.
        if score_diff == 0 && is_late && xg_diff_15 >= 0.7 {
            if matches!(self.instruction, CoachInstruction::AllOutAttack) {
                self.instruction = CoachInstruction::PushForward;
            }
        }

        // Drawing late and getting outxG'd badly → push harder than the
        // base evaluator decided.
        if score_diff == 0 && is_very_late && xg_diff_15 <= -0.5 {
            self.instruction = CoachInstruction::AllOutAttack;
        }

        // Leading by 1 late but conceding heavy xG → switch from
        // WasteTime/SlowDown to a compact mid/low block (we approximate
        // "compact mid block" with ParkTheBus's posture but only after
        // 75').
        if score_diff == 1
            && match_progress > rung(0.83)
            && metrics.xg_against_last_15 > 0.6
            && matches!(
                self.instruction,
                CoachInstruction::WasteTime | CoachInstruction::SlowDown
            )
        {
            self.instruction = CoachInstruction::ParkTheBus;
        }

        // Leading by 2+ but heavily field-tilted by opponent → don't be
        // passive; hold ball with SlowDown (allow safer outlet passes)
        // instead of WasteTime.
        if score_diff >= 2
            && metrics.field_tilt_last_10 > 0.65
            && matches!(self.instruction, CoachInstruction::WasteTime)
        {
            self.instruction = CoachInstruction::SlowDown;
        }

        // Failing press → drop the line. Captured here as switching
        // from PushForward / AllOutAttack to Normal when the pressing
        // isn't producing turnovers and the team is tired.
        if metrics.press_success_rate_last_10 < 0.35
            && avg_team_condition < 0.55
            && matches!(
                self.instruction,
                CoachInstruction::AllOutAttack | CoachInstruction::PushForward
            )
        {
            self.instruction = CoachInstruction::Normal;
        }
    }
}
