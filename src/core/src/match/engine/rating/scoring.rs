//! Attacking-output rating components: scoring events, shooting threat,
//! chance creation, ball progression, retention, and touch quality.
//!
//! Each method returns a small signed value in "rating units"; magnitudes
//! are deliberately modest — they get multiplied by the position weight
//! (<= ~1.1) before contributing to the rating.

use super::{RatingContext, RatingMath};
use crate::PlayerFieldPositionGroup;

impl<'a> RatingContext<'a> {
    /// Shooting threat: xG generated, shots on target, with a wasted-
    /// xG penalty for high-quality chances missed and a shot-spam
    /// penalty for high-volume low-quality attempts.
    ///
    /// Forwards face a stricter calibration on the negative side: the
    /// wasted-xG threshold drops to 0.40 and the per-unit drag is
    /// heavier, and the no-SOT spam drag is heavier too. A forward
    /// who shoots without threatening the goal is observably failing
    /// at their primary role.
    pub(super) fn shooting(&self) -> f32 {
        let s = self.stats;
        if s.shots_total == 0 && s.xg <= 0.0 {
            return 0.0;
        }

        let is_forward = self.pos == PlayerFieldPositionGroup::Forward;

        // Chance VOLUME only — getting into positions to score is worth
        // something on its own. Deliberately smaller than it once was
        // (0.46): conversion is now priced separately and in full by
        // , and at the old weight the two nearly
        // cancelled, leaving a striker who burned 2.5 xG sitting on the
        // population mean.
        let xg_value = RatingMath::sat(s.xg, 1.8) * 0.30;
        // SoT credit lifted 0.34 → 0.42 in the same pass — putting a
        // shot on target IS the headline forward action even without
        // scoring. Together with the xG lift this moves an active
        // goalless shift (~2 SOT, 0.5 xG) by ≈ +0.07 while a no-SOT
        // passenger gains nothing.
        let sot_value = RatingMath::sat(s.shots_on_target as f32, 2.5) * 0.42;
        let mut shooting = xg_value + sot_value;

        // Wasted high-xG chances are NOT charged here — `finishing` in
        // `outfield.rs` owns goals-minus-xG, once. Charging the
        // misses here as well as there double-billed the same afternoon.

        // Shot accuracy band — small lift for hitting the target.
        // Gated to 2+ attempts: accuracy from a single shot is not a
        // signal (one speculative miss was reading as a -0.07 "bad
        // accuracy" verdict, one tidy finish as a +0.08 bonus on top
        // of the SoT credit it already earned).
        if s.shots_total >= 2 {
            let accuracy = s.shots_on_target as f32 / s.shots_total as f32;
            shooting += RatingMath::signed_sat(accuracy - 0.40, 0.30) * 0.08;
        }

        // Shot spam: a wasteful low-skill finisher who keeps launching
        // speculative attempts gets a visible drag. Coefficients
        // softened in 2026-06 round 5 after the dev_match benchmark
        // showed forward goalless tier at 5.65 — the shot-spam +
        // no-SoT spam + wasted-xG triplet was double/triple-biting on
        // the same bad-finishing forward.
        if s.shots_total >= 3 {
            let xg_per_shot = s.xg / s.shots_total as f32;
            if xg_per_shot < 0.10 {
                let spam_coef = if is_forward { 0.26 } else { 0.22 };
                shooting -= RatingMath::sat(s.shots_total as f32 - 2.0, 4.0) * spam_coef;
            }
        }

        // No-goal, no-SOT spammer: drag scales with raw shot volume
        // even when xG is small. Forward coef cut 0.30 → 0.24 — the
        // accuracy band above already reads the same 0-for-N signal
        // negatively at 2+ shots, and the double-bite was holding
        // quiet-forward shifts in the 5.5s instead of the FM-style
        // "poor/quiet ≈ 6.0" anchor.
        if s.goals == 0 && s.shots_on_target == 0 && s.shots_total >= 2 {
            // Softened: the engine forward takes ~3.4 shots with ~1.1 on
            // target, so a 0-SOT match is common and these drags fired
            // far more often than on the real stat lines the season
            // fixtures are built from — holding FWD season means ~0.3
            // under band while the goal-scoring tail was already tamed.
            let nosot_coef = if is_forward { 0.15 } else { 0.18 };
            shooting -= RatingMath::sat(s.shots_total as f32 - 1.0, 3.0) * nosot_coef;
        }

        shooting
    }

    /// Chance creation: key passes, passes/carries into the box,
    /// completed crosses, xG buildup, zone-aware lane bonuses.
    ///
    /// Assists deliberately do NOT live here — they are routed through
    /// [`Self::scoring_event`] alongside goals, so the same `scoring`
    /// profile weight drives all decisive attacking events. This keeps
    /// the per-position dial coherent (a striker's assist pays through
    /// the same channel as a goal) and prevents the creation soft-cap
    /// from accidentally suppressing assist credit.
    ///
    /// Coefficients are deliberately modest — a real "good creator"
    /// (3 KP + 3 box entries + 4 progressive) lands routine ~0.6,
    /// not the inflated ~1.1 that drove ordinary playmakers to 7.4
    /// on routine alone. The surrounding chain-building creates the
    /// lift, but doesn't take the player into the elite band without
    /// a goal contribution.
    pub(super) fn creation(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;

        let key = RatingMath::sat(s.key_passes as f32, 3.5) * 0.42;

        // Box entries — combine passes-into-box and carries-into-box so
        // the same delivery doesn't pay double if both fired.
        let box_entries =
            RatingMath::sat(s.passes_into_box as f32 + z.carries_into_box as f32, 5.0) * 0.30;

        // Cross output: completed crosses help, failed crosses drag.
        // Failed-cross penalty softened (was sat(failed, 5.0) * 0.22):
        // a routine fullback attempts 3-5 crosses per match and
        // completes 1-2 (real-football reference: ~25% completion).
        // The prior coefficient hit them with -0.07 to -0.14 routine
        // drag for normal workload, contributing to the Cambiaso 6.20
        // average. The gentler curve still drags a player who can't
        // hit a cross to save their life, but absorbs ordinary fullback
        // crossing volume.
        let cross_credit = RatingMath::sat(s.crosses_completed as f32, 3.5) * 0.13;
        let cross_failed = s.crosses_attempted.saturating_sub(s.crosses_completed) as f32;
        let cross_penalty = RatingMath::sat(cross_failed, 10.0) * 0.10;

        // xG buildup — chains the player participated in that ended
        // in a shot. Clean "made the chance happen" signal.
        let xg_chain = RatingMath::sat(s.xg_buildup.max(0.0), 1.2) * 0.30;

        // Zone-aware lane creation — smaller weights because the same
        // events typically tick `passes_into_box` / `key_passes` too.
        let lanes = RatingMath::sat(
            z.half_space_passes_into_box as f32
                + z.central_passes_into_box as f32
                + z.switches_of_play as f32,
            7.0,
        ) * 0.12;

        // Progressive into final third — chance build-up that didn't
        // reach the box.
        let into_final_third = RatingMath::sat(
            z.progressive_passes_into_final_third as f32
                + z.progressive_carries_into_final_third as f32,
            7.0,
        ) * 0.08;

        key + box_entries + cross_credit - cross_penalty + xg_chain + lanes + into_final_third
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

        let pp = RatingMath::sat(s.progressive_passes as f32, 6.0) * 0.26;
        let pc = RatingMath::sat(s.progressive_carries as f32, 5.0) * 0.24;
        let cd = RatingMath::sat(s.carry_distance as f32 / 1000.0, 1.8) * 0.10;

        let drib_w = match self.pos {
            PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder => 0.26,
            _ => 0.14,
        };
        let dribbles = RatingMath::sat(s.successful_dribbles as f32, 3.5) * drib_w;

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
        let failed_drib = RatingMath::sat(failed, 3.0) * failed_w;

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
        let volume = RatingMath::sat(s.passes_attempted as f32, 45.0); // saturates by ~90 attempts
        // 0.74 is the league-average baseline. Coefficient lifted
        // 0.30 → 0.50 in the FM-parity DEF/MID season pass: the
        // recycler archetype (60+ passes at ~90%) was accumulating to
        // ~6.38 against the believable 6.50-6.75 band — high-volume
        // accurate circulation is the role's primary output and FM
        // credits it as solid. The elite band still needs progression
        // / creation: the recycler guards (`safe_recycler...`,
        // `high_pass_completion...`) hold tidy volume below 7.0.
        let pass_signal = RatingMath::signed_sat(pct - 0.74, 0.18) * volume * 0.50;
        pass_signal + touch_drag
    }

    /// Touch-quality drag from miscontrols and heavy touches. Returns
    /// a non-positive value (0 if no events recorded). Saturating so
    /// a single bad touch isn't catastrophic but accumulating losses
    /// of control visibly drag the rating.
    ///
    /// The producer (`add_miscontrol` / `add_heavy_touch`) IS wired in
    /// `match/engine/player/events/players.rs` — it fires per receive
    /// roll against `first_touch_loss_probability`: a pressured lane
    /// scaling with (1 − composite)^2.5 · pressure plus an unforced
    /// pressure-independent lane (1 − composite)² · 0.05, where the
    /// composite reads first_touch / technique / composure /
    /// anticipation / decisions. A low-skill player under regular
    /// pressure accumulates 3-5 events per 90; a weak-mentals player
    /// even unmarked leaks 1-2, landing −0.2 to −0.6 of rating drag.
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
        -RatingMath::sat(m + h, 5.0) * 0.85
    }
}
