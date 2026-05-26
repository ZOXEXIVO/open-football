//! Attacking-output rating components: scoring events, shooting threat,
//! chance creation, ball progression, retention, and touch quality.
//!
//! Each method returns a small signed value in "rating units"; magnitudes
//! are deliberately modest — they get multiplied by the position weight
//! (<= ~1.1) before contributing to the rating.

use super::{RatingContext, sat, signed_sat};
use crate::PlayerFieldPositionGroup;

impl<'a> RatingContext<'a> {
    /// Direct goal-event impact: goals scored + clinical (over-xG) +
    /// decisive (the goal won the match). Saturates so a hat-trick is
    /// rewarded but not 3× a single goal.
    pub(super) fn scoring_event(&self) -> f32 {
        let s = self.stats;
        if s.goals == 0 {
            return 0.0;
        }
        let g = s.goals as f32;
        // sat(1, 1.6) ≈ 0.46; sat(2) ≈ 0.71; sat(3) ≈ 0.85.
        let raw = sat(g, 1.6) * 2.55;

        // Clinical-finisher bonus: goals beyond xG → premium for
        // converting tougher chances or being lethal in front of goal.
        let over = (g - s.xg).max(0.0);
        let clinical = sat(over, 1.0) * 0.15;

        // Decisive-goal nudge — the goal mattered to the scoreline.
        let decisive = if self.team_goals > self.opponent_goals {
            0.08
        } else {
            0.0
        };

        raw + clinical + decisive
    }

    /// Shooting threat: xG generated, shots on target, with a wasted-
    /// xG penalty for high-quality chances missed and a shot-spam
    /// penalty for high-volume low-quality attempts.
    pub(super) fn shooting(&self) -> f32 {
        let s = self.stats;
        if s.shots_total == 0 && s.xg <= 0.0 {
            return 0.0;
        }

        let xg_value = sat(s.xg, 1.8) * 0.30;
        let sot_value = sat(s.shots_on_target as f32, 2.5) * 0.22;
        let mut shooting = xg_value + sot_value;

        // Wasted high xG: created premium chances, scored nothing.
        if s.goals == 0 && s.xg > 0.6 {
            shooting -= sat(s.xg - 0.6, 1.2) * 0.55;
        }

        // Shot accuracy band — small lift for hitting the target.
        if s.shots_total > 0 {
            let accuracy = s.shots_on_target as f32 / s.shots_total as f32;
            shooting += signed_sat(accuracy - 0.40, 0.30) * 0.08;
        }

        // Shot spam: a busier threshold (≥ 3 shots, was 5) and a heavier
        // per-event drag so a wasteful low-skill finisher who keeps
        // launching speculative attempts is visibly penalised. A genuine
        // creator hitting target with 3+ SOT recovers most of this via
        // the SOT credit above.
        if s.shots_total >= 3 {
            let xg_per_shot = s.xg / s.shots_total as f32;
            if xg_per_shot < 0.10 {
                shooting -= sat(s.shots_total as f32 - 2.0, 4.0) * 0.45;
            }
        }

        // No-goal, no-SOT spammer: drag scales with raw shot volume
        // even when xG is small — a low-skill forward hammering speculative
        // off-target attempts looks busy on `shots_total` but produced
        // nothing the keeper had to deal with.
        if s.goals == 0 && s.shots_on_target == 0 && s.shots_total >= 2 {
            shooting -= sat(s.shots_total as f32 - 1.0, 3.0) * 0.30;
        }

        shooting
    }

    /// Chance creation: assists, key passes, passes/carries into the
    /// box, completed crosses, xG buildup, zone-aware lane bonuses.
    ///
    /// Coefficients are deliberately modest — a real "good creator"
    /// (3 KP + 3 box entries + 4 progressive) lands routine ~0.65,
    /// not the inflated ~1.1 that drove ordinary playmakers to 7.4
    /// on routine alone. Assist event itself still pays well; the
    /// surrounding chain-building creates the lift, but doesn't take
    /// the player into the elite band without a goal-contribution.
    pub(super) fn creation(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;

        let assists = sat(s.assists as f32, 1.6) * 1.10;

        let key = sat(s.key_passes as f32, 3.5) * 0.42;

        // Box entries — combine passes-into-box and carries-into-box so
        // the same delivery doesn't pay double if both fired.
        let box_entries = sat(s.passes_into_box as f32 + z.carries_into_box as f32, 5.0) * 0.30;

        // Cross output: completed crosses help, failed crosses drag.
        let cross_credit = sat(s.crosses_completed as f32, 3.5) * 0.13;
        let cross_failed = s.crosses_attempted.saturating_sub(s.crosses_completed) as f32;
        let cross_penalty = sat(cross_failed, 5.0) * 0.22;

        // xG buildup — chains the player participated in that ended
        // in a shot. Clean "made the chance happen" signal.
        let xg_chain = sat(s.xg_buildup.max(0.0), 1.2) * 0.30;

        // Zone-aware lane creation — smaller weights because the same
        // events typically tick `passes_into_box` / `key_passes` too.
        let lanes = sat(
            z.half_space_passes_into_box as f32
                + z.central_passes_into_box as f32
                + z.switches_of_play as f32,
            7.0,
        ) * 0.12;

        // Progressive into final third — chance build-up that didn't
        // reach the box.
        let into_final_third = sat(
            z.progressive_passes_into_final_third as f32
                + z.progressive_carries_into_final_third as f32,
            7.0,
        ) * 0.08;

        assists + key + box_entries + cross_credit - cross_penalty
            + xg_chain
            + lanes
            + into_final_third
    }

    /// Ball progression and dribbling: progressive passes, progressive
    /// carries, carry distance, take-ons. Failed dribbles drag harder
    /// than success rewards — a low-skill dribbler who keeps trying
    /// 1v1s and losing is visibly costing the team.
    ///
    /// Coefficients are tuned so that "moved the ball forward" stats
    /// register but don't dominate. A progressive pass / carry is
    /// observable evidence — it earns Tier B in the soft-cap ladder —
    /// but the raw component contribution stays modest.
    pub(super) fn progression(&self) -> f32 {
        let s = self.stats;

        let pp = sat(s.progressive_passes as f32, 6.0) * 0.26;
        let pc = sat(s.progressive_carries as f32, 5.0) * 0.24;
        let cd = sat(s.carry_distance as f32 / 1000.0, 1.8) * 0.10;

        let drib_w = match self.pos {
            PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder => 0.26,
            _ => 0.14,
        };
        let dribbles = sat(s.successful_dribbles as f32, 3.5) * drib_w;

        let failed = s.attempted_dribbles.saturating_sub(s.successful_dribbles) as f32;
        // Failed-dribble drag is tighter saturation (3.0 vs 4.0) and
        // a heavier per-event weight so a poor 1v1 record visibly hurts.
        // Forwards still get a small discount because the position
        // expects them to take risks.
        let failed_w = if self.pos == PlayerFieldPositionGroup::Forward {
            0.26
        } else {
            0.34
        };
        let failed_drib = sat(failed, 3.0) * failed_w;

        pp + pc + cd + dribbles - failed_drib
    }

    /// Pass-completion quality × volume. A high-volume accurate passer
    /// in midfield is rewarded; a low-completion volume passer is
    /// dragged. Volume gates the magnitude (a 10-pass shift moves the
    /// retention component very little).
    ///
    /// First-touch quality enters here as a small drag from
    /// `miscontrols` and `heavy_touches`. The drag is conservative
    /// because the engine producers for those counters are still being
    /// wired up — once they fire reliably, every event registers as a
    /// visible loss of control without needing a coefficient bump.
    pub(super) fn retention(&self) -> f32 {
        let s = self.stats;
        let touch_drag = self.touch_quality();
        if s.passes_attempted < 10 {
            return touch_drag;
        }
        let pct = s.passes_completed as f32 / s.passes_attempted as f32;
        let volume = sat(s.passes_attempted as f32, 45.0); // saturates by ~90 attempts
        // 0.74 is the league-average baseline. High pass completion alone
        // should not be a large bonus — a tidy 90% recycler isn't elite.
        // The coefficient is intentionally modest so that retention has
        // to combine with progression / creation to push a rating up.
        let pass_signal = signed_sat(pct - 0.74, 0.18) * volume * 0.30;
        pass_signal + touch_drag
    }

    /// Touch-quality drag from miscontrols and heavy touches. Returns
    /// a non-positive value (0 if no events recorded). Saturating so
    /// a single bad touch isn't catastrophic but accumulating losses
    /// of control visibly drag the rating.
    ///
    /// The producer (`add_miscontrol` / `add_heavy_touch`) IS wired in
    /// `match/engine/player/events/players.rs` — it fires per receive
    /// roll against `first_touch_loss_probability`, which scales with
    /// (1 − first_touch_skill)² · pressure. A low-skill player under
    /// regular pressure will accumulate 3-5 events per 90 minutes,
    /// landing roughly −0.45 to −0.6 of rating drag.
    pub(super) fn touch_quality(&self) -> f32 {
        let s = self.stats;
        let m = s.miscontrols as f32;
        let h = s.heavy_touches as f32 * 0.5;
        if m + h <= 0.0 {
            return 0.0;
        }
        // sat(3, 5) ≈ 0.45 → ~ -0.38 rating units at three miscontrols;
        // sat(5, 5) ≈ 0.63 → ~ -0.54 at five. Strong enough that
        // low-first-touch players visibly drop, gentle enough that one
        // mishit doesn't define the match.
        -sat(m + h, 5.0) * 0.85
    }
}
