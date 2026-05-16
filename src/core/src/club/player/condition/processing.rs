use crate::club::PlayerStatusType;
use crate::club::player::player::Player;
use crate::league::SeasonPhase;
use crate::utils::DateUtils;
use chrono::{Datelike, NaiveDate};

/// Minimum condition floor for injured players (30%)
const INJURY_CONDITION_FLOOR: i16 = 3000;

/// Minimum fitness floor for injured players (20%)
const INJURY_FITNESS_FLOOR: i16 = 2000;

/// Inputs that feed `ConditionRecoveryModel::individualized_target` and
/// `daily_recovery_rate`. Lives here so callers (daily rest, training
/// result) feed the exact same coefficients and the per-player target
/// can't drift between code paths. The struct is `Copy` and cheap to
/// build at the call site — no need to cache it.
#[derive(Debug, Clone, Copy)]
pub struct ConditionTargetInputs {
    /// Genetic recovery ceiling (0..20).
    pub natural_fitness: f32,
    /// Stamina (0..20).
    pub stamina: f32,
    /// Chronic fitness base (0..10000).
    pub chronic_fitness: i16,
    /// Match sharpness (0..20).
    pub match_readiness: f32,
    /// Recent physical load — `PlayerLoad::physical_load_7`.
    pub physical_load_7: f32,
    /// Deep tiredness backlog — `PlayerLoad::recovery_debt`.
    pub recovery_debt: f32,
    /// Long-term tiredness (0..10000).
    pub jadedness: i16,
    /// Age in whole years.
    pub age: u8,
}

/// Daily condition-recovery model for non-injured players. Bundles the
/// per-day caps and throttles so callers (training result, daily rest
/// recovery) read the same constants and can't drift. All knobs are
/// associated constants / fns so a future "altitude camp" or
/// "fitness-coach diet plan" growth path adds methods without
/// rippling through the codebase.
pub struct ConditionRecoveryModel;

impl ConditionRecoveryModel {
    /// Legacy baseline (88%) — kept as an associated constant so older
    /// call sites (UI labels, save-import migrations) that still read it
    /// don't break. The active recovery path uses
    /// `individualized_target` instead, which produces a per-player
    /// ceiling and is what every new caller should reach for.
    pub const NORMAL_BASE: i16 = 8800;

    /// Legacy per-NF uplift constant. Same back-compat note as
    /// `NORMAL_BASE` — kept readable, no longer used by the deficit-
    /// based recovery path.
    pub const NF_UPLIFT_PER_POINT: f32 = 35.0;

    /// Lower clamp on the per-player daily condition target. Below this
    /// even an overloaded vet bounces back partway every night — the
    /// body is never at zero.
    pub const TARGET_FLOOR: f32 = 7_600.0;

    /// Upper clamp on the per-player daily target. Genuine elites
    /// (high NF + stamina + fresh) can sit close to 97%, but persisted
    /// condition past this would imply a 24/7 cryo chamber.
    pub const TARGET_CEILING: f32 = 9_700.0;

    /// Floor on raw daily recovery for a player below the
    /// `LOW_CONDITION_FLOOR_THRESHOLD` percentage. A genuinely
    /// shattered player still recovers a measurable amount overnight —
    /// the asymptotic shape would otherwise leave deep holes refilling
    /// agonisingly slowly when the deficit-fraction is also small.
    pub const MIN_DAILY_RECOVERY_DEEP: u16 = 120;

    /// Ordinary daily recovery cap. Recovery sessions in the training
    /// path can exceed this separately — that's the whole point of
    /// scheduling them.
    pub const MAX_DAILY_RECOVERY_NORMAL: u16 = 650;

    /// Condition-percentage threshold under which the deep-recovery
    /// floor kicks in. Above this the asymptotic shape governs.
    pub const LOW_CONDITION_FLOOR_THRESHOLD: u32 = 65;

    /// Recovery-debt level at which the daily-recovery throttle starts
    /// biting. Mirrors the UI "heavy legs" threshold
    /// (`RECOVERY_DEBT_HEAVY` in `load.rs`). Below this the multiplier
    /// is exactly 1.0; above it the smoothstep curve takes over.
    pub const DEBT_THROTTLE_START: f32 = 350.0;

    /// Recovery-debt level at which the throttle reaches its floor.
    /// Between `DEBT_THROTTLE_START` and `DEBT_THROTTLE_FULL` the
    /// multiplier follows a smoothstep curve (3t² − 2t³) so the
    /// transition is C¹-continuous — no cliff at the start, no cliff
    /// at the floor.
    pub const DEBT_THROTTLE_FULL: f32 = 1_500.0;

    /// Lower bound on the daily-recovery multiplier when debt is at or
    /// above `DEBT_THROTTLE_FULL`. The body always recovers something
    /// each day — even a structurally-overloaded pro is not at zero.
    pub const MIN_RECOVERY_MULT: f32 = 0.45;

    /// Slow-drain coefficient applied to the daily natural-recovery
    /// magnitude when consuming recovery_debt outside training. A
    /// player resting on a no-training day still bleeds off some
    /// deep tiredness — just slower than a structured recovery
    /// session would.
    pub const NATURAL_DEBT_DRAIN_FACTOR: f32 = 0.12;

    /// Noise salt for daily rest condition recovery. Mixed into the
    /// hash alongside `(player_id, date_ordinal)` so the rest path's
    /// noise stream is decorrelated from training recovery and post-
    /// match exertion — otherwise a single player/date seed would push
    /// rest gain, training gain, and match drain in the same direction
    /// on the same day. Magic-number constants are arbitrary but must
    /// be distinct across `NOISE_*` salts.
    pub const NOISE_REST_RECOVERY: u32 = 0xA11C_E001;

    /// Noise salt for training-day recovery sessions. See
    /// [`Self::NOISE_REST_RECOVERY`] for why salts exist at all.
    pub const NOISE_TRAINING_RECOVERY: u32 = 0xA11C_E002;

    /// Noise salt for post-match exertion. See
    /// [`Self::NOISE_REST_RECOVERY`].
    pub const NOISE_MATCH_EXERTION: u32 = 0xA11C_E003;

    /// Legacy dynamic-cap helper. Still callable by code paths that
    /// don't have the full per-player picture (UI sanity defaults,
    /// import migrations) but new logic should use
    /// `individualized_target` instead — that's the model the deficit-
    /// based daily recovery is built on.
    pub fn dynamic_cap(natural_fitness: f32) -> i16 {
        let nf = natural_fitness.clamp(0.0, 20.0);
        Self::NORMAL_BASE + (nf * Self::NF_UPLIFT_PER_POINT) as i16
    }

    /// Per-player daily condition target. Replaces the narrow 8800..9500
    /// dynamic cap with a band that actually reflects the player's
    /// physical profile and recent load:
    ///
    ///   * Weak physical profile, high load:        76..84%
    ///   * Normal senior pro, neutral load:         86..92%
    ///   * Elite stamina/NF/fitness, low load:      94..97%
    ///
    /// Result is the asymptote the deficit-based recovery converges
    /// toward; it is *not* a hard cap. The training path and the daily
    /// rest path both feed this so the two cannot disagree on what
    /// "fully recovered" means for any given player today.
    pub fn individualized_target(i: ConditionTargetInputs) -> f32 {
        let nf01 = (i.natural_fitness / 20.0).clamp(0.0, 1.0);
        let stamina01 = (i.stamina / 20.0).clamp(0.0, 1.0);
        let chronic_fitness01 = (i.chronic_fitness as f32 / 10_000.0).clamp(0.0, 1.0);
        let match_readiness01 = (i.match_readiness / 20.0).clamp(0.0, 1.0);

        let age_target_mult = match i.age {
            0..=17 => 0.96,
            18..=21 => 0.985,
            22..=28 => 1.00,
            29..=32 => 0.985,
            33..=35 => 0.955,
            _ => 0.925,
        };

        let load_drag = (i.physical_load_7 / 620.0).clamp(0.0, 1.0) * 450.0
            + (i.recovery_debt / 1_500.0).clamp(0.0, 1.0) * 700.0
            + (i.jadedness as f32 / 10_000.0).clamp(0.0, 1.0) * 500.0;

        let base_target = 8_200.0
            + nf01 * 550.0
            + stamina01 * 300.0
            + chronic_fitness01 * 450.0
            + match_readiness01 * 180.0;

        (base_target * age_target_mult - load_drag).clamp(Self::TARGET_FLOOR, Self::TARGET_CEILING)
    }

    /// Fraction of the remaining deficit (target − current) the player
    /// recovers per day at neutral phase / no jadedness / no debt. A
    /// higher rate means a more responsive bounce-back — elites with
    /// elite recovery genetics close the gap quickly, average bodies
    /// take their time. The multiplicative modifiers in
    /// `process_condition_recovery` (age, debt throttle, jadedness,
    /// season phase) further scale this.
    pub fn daily_recovery_rate(natural_fitness: f32, chronic_fitness: i16, professionalism: f32) -> f32 {
        let nf01 = (natural_fitness / 20.0).clamp(0.0, 1.0);
        let chronic_fitness01 = (chronic_fitness as f32 / 10_000.0).clamp(0.0, 1.0);
        let professionalism01 = (professionalism / 20.0).clamp(0.0, 1.0);
        0.22 + nf01 * 0.10 + chronic_fitness01 * 0.06 + professionalism01 * 0.04
    }

    /// Age-driven recovery multiplier — youth bounce back fast, vets
    /// take longer. Applied on top of `daily_recovery_rate` in the
    /// per-day recovery formula.
    pub fn age_recovery_mult(age: u8) -> f32 {
        match age {
            0..=21 => 1.08,
            22..=29 => 1.00,
            30..=32 => 0.92,
            33..=35 => 0.82,
            _ => 0.72,
        }
    }

    /// Jadedness-driven recovery multiplier. A jaded player's body is
    /// still bookkeeping deep tiredness — overnight rest is less
    /// efficient.
    pub fn jadedness_recovery_mult(jadedness: i16) -> f32 {
        let j01 = (jadedness as f32 / 10_000.0).clamp(0.0, 1.0);
        1.0 - j01 * 0.25
    }

    /// Tiny deterministic noise factor so two identical-profile players
    /// don't track each other exactly day-to-day. ±`amplitude` band,
    /// seeded by `(player_id, date_ordinal, salt)` so the same day for
    /// the same player and salt always returns the same value (no
    /// save-load drift, stable for tests). The `salt` decouples noise
    /// streams across physical systems: without it, daily rest,
    /// training recovery, and post-match exertion would all share the
    /// same per-(player, date) draw and could systematically push the
    /// same player's three physical updates in the same direction on
    /// the same day. Callers pass one of the `NOISE_*` constants on
    /// `ConditionRecoveryModel`. Pure helper — caller multiplies
    /// whatever value they're noising up.
    ///
    /// Example:
    /// `recovery * deterministic_noise(id, date, NOISE_REST_RECOVERY, 0.03)`
    /// gives the recovery a ±3% per-day jitter that does not echo into
    /// the training or match-exertion paths.
    pub fn deterministic_noise(
        player_id: u32,
        date_ordinal: i32,
        salt: u32,
        amplitude: f32,
    ) -> f32 {
        // Mix the inputs through a small avalanche so adjacent ids /
        // adjacent days / adjacent salts don't produce correlated
        // noise. The output lands in [-amplitude, +amplitude] around
        // 1.0. Salt is folded into the player id half of the seed so
        // it actually changes the avalanche path, not just the final
        // bits of the hash.
        let pid_mixed = (player_id as u64) ^ ((salt as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93));
        let mut h = pid_mixed
            ^ ((date_ordinal as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        h ^= h >> 30;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 27;
        h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
        h ^= h >> 31;
        // Map low 24 bits to a unit interval, then to [-1, +1].
        let unit = ((h & 0x00FF_FFFF) as f32) / 0x00FF_FFFF as f32;
        let signed = unit * 2.0 - 1.0;
        1.0 + signed * amplitude
    }

    /// Multiplier applied to the *acute* condition cost of a positive
    /// training session (heavy drill that drops condition). Lets a
    /// session's persisted condition hit individualise the same way
    /// recovery does — elite stamina/NF/chronic-fitness bodies pay
    /// less, overloaded / jaded / very young / veteran bodies pay
    /// more. Sits in 0.82..1.35; clamped tighter than the recovery
    /// band so a session can't flip into "free" or "ruinous" purely
    /// from physiology — the coach's drill choice still dominates.
    ///
    /// Inputs: stamina (0..20), natural_fitness (0..20), chronic_fitness
    /// (0..10000), recovery_debt (0..~2000), jadedness (0..10000), age.
    ///
    /// Only meaningful on the positive `fatigue_change` branch in the
    /// training-result pipeline. The recovery branch already
    /// individualises via `individualized_target` + an efficiency
    /// term — applying this multiplier there would double-count
    /// physiology.
    pub fn training_fatigue_cost_mult(
        stamina: f32,
        natural_fitness: f32,
        chronic_fitness: i16,
        recovery_debt: f32,
        jadedness: i16,
        age: u8,
    ) -> f32 {
        let stamina01 = (stamina / 20.0).clamp(0.0, 1.0);
        let nf01 = (natural_fitness / 20.0).clamp(0.0, 1.0);
        let chronic01 = (chronic_fitness as f32 / 10_000.0).clamp(0.0, 1.0);
        let debt01 = (recovery_debt / 1_500.0).clamp(0.0, 1.0);
        let jad01 = (jadedness as f32 / 10_000.0).clamp(0.0, 1.0);

        // Age band: kids and vets pay more acute cost for the same
        // drill (developing bodies / declining recovery + repair).
        let age_mult: f32 = match age {
            0..=17 => 1.12,
            18..=21 => 1.04,
            22..=29 => 1.00,
            30..=32 => 1.05,
            33..=35 => 1.12,
            _ => 1.20,
        };

        // Physical resistance: high stamina / NF / chronic fitness all
        // shave the cost. Capped so an "elite-everything" 26-year-old
        // doesn't make heavy drills free.
        let resistance = 1.18
            - stamina01 * 0.14
            - nf01 * 0.10
            - chronic01 * 0.08;

        // Overload tax: a jaded / debt-laden body burns more condition
        // for the same drill.
        let overload = 1.0 + debt01 * 0.18 + jad01 * 0.12;

        (resistance * overload * age_mult).clamp(0.82, 1.35)
    }

    /// Daily-rest condition recovery throttle. Returns 1.0 below the
    /// `DEBT_THROTTLE_START` "heavy legs" threshold and falls smoothly
    /// (smoothstep, 3t² − 2t³) to `MIN_RECOVERY_MULT` (0.45) as debt
    /// reaches `DEBT_THROTTLE_FULL` (1500). The body prioritises
    /// clearing structural fatigue over restoring acute freshness.
    ///
    /// The smoothstep replaces the old linear cliff which dropped the
    /// multiplier from 1.0 to ~0.77 the moment debt crossed 350 —
    /// realistically a player at debt 351 is barely more limited than
    /// one at debt 349.
    pub fn debt_throttle(recovery_debt: f32) -> f32 {
        if recovery_debt <= Self::DEBT_THROTTLE_START {
            return 1.0;
        }
        let span = Self::DEBT_THROTTLE_FULL - Self::DEBT_THROTTLE_START;
        if span <= 0.0 {
            return Self::MIN_RECOVERY_MULT;
        }
        let t = ((recovery_debt - Self::DEBT_THROTTLE_START) / span).clamp(0.0, 1.0);
        let smooth_t = t * t * (3.0 - 2.0 * t);
        1.0 - smooth_t * (1.0 - Self::MIN_RECOVERY_MULT)
    }
}

impl Player {
    /// Daily condition processing (rest day — no training scheduled).
    /// Deficit-based, asymptotic recovery toward a per-player target.
    ///
    /// Two players with the same condition but different stamina /
    /// natural_fitness / chronic_fitness / age / recent load will
    /// recover by different amounts overnight. The recovery shape is
    /// asymptotic — a player near their target gains little, a deeply
    /// depleted player gains the most — so ordinary rest never washes
    /// the squad into a flat distribution.
    pub(crate) fn process_condition_recovery(&mut self, now: NaiveDate) {
        let natural_fitness = self.skills.physical.natural_fitness;

        if self.player_attributes.is_injured {
            self.process_injury_condition_decay(natural_fitness);
            self.player_attributes.days_since_last_match += 1;
            return;
        }

        let age = DateUtils::age(self.birth_date, now);
        let jadedness = self.player_attributes.jadedness;
        let condition = self.player_attributes.condition;

        // Per-player target (asymptote, not a hard cap). Built from
        // stamina, NF, chronic fitness, match readiness, recent load,
        // recovery debt, jadedness, age — every input the spec cares
        // about, in one helper, in one place.
        let target = ConditionRecoveryModel::individualized_target(ConditionTargetInputs {
            natural_fitness,
            stamina: self.skills.physical.stamina,
            chronic_fitness: self.player_attributes.fitness,
            match_readiness: self.skills.physical.match_readiness,
            physical_load_7: self.load.physical_load_7,
            recovery_debt: self.load.recovery_debt,
            jadedness,
            age,
        });

        if (condition as f32) < target {
            // Deficit-based recovery: the further below target, the
            // larger the chunk recovered today. Asymptotic shape — a
            // player at 70% recovers more than one at 92% on the same
            // body, both crossing the gap toward target. Net effect:
            // depleted players catch up, near-fresh players coast.
            let deficit = target - condition as f32;

            let base_rate = ConditionRecoveryModel::daily_recovery_rate(
                natural_fitness,
                self.player_attributes.fitness,
                self.attributes.professionalism,
            );

            // Modifiers — age, debt throttle, jadedness, season phase.
            // Multiplicative so an "old & jaded in mid-season" player
            // recovers far less than an "elite young in winter break"
            // even from the same deficit.
            let age_mult = ConditionRecoveryModel::age_recovery_mult(age);
            let debt_mult = ConditionRecoveryModel::debt_throttle(self.load.recovery_debt);
            let jaded_mult = ConditionRecoveryModel::jadedness_recovery_mult(jadedness);
            let phase_bonus = SeasonPhase::from_date(now).condition_recovery_multiplier();

            // Small deterministic per-player noise so two identical
            // profiles don't track each other minute-for-minute over
            // weeks of simulation. ±3% per day, stable for the
            // (player, date, salt) triple. The rest-recovery salt
            // keeps this stream independent from the training-recovery
            // and match-exertion streams so all three can't pile up
            // for the same player on the same day.
            let date_ordinal = now.num_days_from_ce();
            let noise = ConditionRecoveryModel::deterministic_noise(
                self.id,
                date_ordinal,
                ConditionRecoveryModel::NOISE_REST_RECOVERY,
                0.03,
            );

            let raw_recovery =
                deficit * base_rate * age_mult * debt_mult * jaded_mult * phase_bonus * noise;

            // Clamp the daily change so an enormous deficit (e.g.
            // returning from a multi-day idle gap) doesn't snap back in
            // one tick, and a deeply depleted player gets at least the
            // deep-recovery floor even if their deficit-fraction would
            // otherwise produce a microscopic gain.
            let mut recovery = raw_recovery.max(0.0) as u16;
            recovery = recovery.min(ConditionRecoveryModel::MAX_DAILY_RECOVERY_NORMAL);
            if self.player_attributes.condition_percentage()
                < ConditionRecoveryModel::LOW_CONDITION_FLOOR_THRESHOLD
            {
                recovery = recovery.max(ConditionRecoveryModel::MIN_DAILY_RECOVERY_DEEP);
            }

            // Never overshoot the target — the asymptote is the
            // contract, not a suggestion.
            let max_gain = (target - condition as f32).max(0.0) as u16;
            self.player_attributes.rest(recovery.min(max_gain));
        }

        // Natural recovery debt drain — independent of training. The
        // body's slow constant-time housekeeping bleeds off some debt
        // even on pure rest days. Phase bonus folds in (off-season /
        // winter-break = bigger drain).
        let phase_bonus = SeasonPhase::from_date(now).condition_recovery_multiplier();
        let natural_debt_reduction = (200.0 + (natural_fitness / 20.0) * 300.0)
            * ConditionRecoveryModel::NATURAL_DEBT_DRAIN_FACTOR
            * phase_bonus;
        self.load.consume_recovery_debt(natural_debt_reduction);

        // Jadedness natural decay: -150/day when no match for 3+ days
        if self.player_attributes.days_since_last_match > 3 {
            self.player_attributes.jadedness = (self.player_attributes.jadedness - 150).max(0);
        }

        // Remove Rst status when jadedness drops below threshold
        if self.player_attributes.jadedness < 4000 {
            self.statuses.remove(PlayerStatusType::Rst);
        }

        // Fitness atrophy during prolonged inactivity. Defined narrowly:
        // > 14 days since last match AND the rolling physical-load
        // window is near zero (i.e. no meaningful training either).
        // Without the load gate we'd decay fitness for any player
        // training every day but with no matches in the calendar —
        // exactly backwards from intent. Decay is 10..30/day depending
        // on age (older players atrophy faster) so a player abandoned
        // on a free transfer or a long-injured pro losing their
        // base fitness reads correctly.
        if self.player_attributes.days_since_last_match > 14 && self.load.physical_load_7 < 15.0 {
            let age_decay_base = if age >= 30 { 30.0 } else { 10.0 + (age as f32 - 16.0).max(0.0) };
            let fitness_decay = age_decay_base.clamp(10.0, 30.0) as i16;
            self.player_attributes.fitness =
                (self.player_attributes.fitness - fitness_decay).max(INJURY_FITNESS_FLOOR);
        }

        // Increment days since last match
        self.player_attributes.days_since_last_match += 1;
    }

    /// Injured players lose condition, fitness, and match readiness daily.
    /// Higher natural_fitness slows the decay.
    /// Condition floors at 30% — even injured players maintain baseline condition.
    fn process_injury_condition_decay(&mut self, natural_fitness: f32) {
        let nf_factor = 1.0 - (natural_fitness / 20.0) * 0.7; // 0.3..1.0

        // Condition decay: ~30-100/day depending on natural_fitness
        // At nf=20: ~30/day, at nf=1: ~98/day
        let condition_decay = (100.0 * nf_factor) as i16;
        self.player_attributes.condition =
            (self.player_attributes.condition - condition_decay).max(INJURY_CONDITION_FLOOR);

        // Fitness decay: ~15-50/day depending on natural_fitness
        let fitness_decay = (50.0 * nf_factor) as i16;
        self.player_attributes.fitness =
            (self.player_attributes.fitness - fitness_decay).max(INJURY_FITNESS_FLOOR);

        // Match readiness decays faster during injury (not training or playing)
        self.skills.physical.match_readiness =
            (self.skills.physical.match_readiness - 0.2).max(0.0);
    }

    /// Players not playing lose match sharpness over time — except in
    /// pre-season, when structured training rebuilds it.
    pub(crate) fn process_match_readiness_decay(&mut self, now: NaiveDate) {
        if self.player_attributes.is_injured {
            // Already handled in process_injury_condition_decay
            return;
        }

        let phase = SeasonPhase::from_date(now);
        let phase_gain = phase.match_readiness_gain();
        if phase_gain > 0.0 {
            // Pre-season / winter-break conditioning actively rebuilds
            // sharpness — override the idle-decay branch entirely.
            self.skills.physical.match_readiness =
                (self.skills.physical.match_readiness + phase_gain).min(20.0);
            return;
        }

        if self.player_attributes.days_since_last_match > 7 {
            // Accelerated decay after a week without matches
            self.skills.physical.match_readiness =
                (self.skills.physical.match_readiness - 0.15).max(0.0);
        } else if self.player_attributes.days_since_last_match > 3 {
            // Gradual decay
            self.skills.physical.match_readiness =
                (self.skills.physical.match_readiness - 0.08).max(0.0);
        }
        // No decay for first 3 days (normal rest period)
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::{ConditionRecoveryModel, ConditionTargetInputs};
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::condition::ConditionLabel;
    use crate::club::player::player::Player;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };
    use chrono::NaiveDate;

    #[test]
    fn deterministic_noise_is_stable_for_same_player_date_salt() {
        let a = ConditionRecoveryModel::deterministic_noise(
            42,
            12345,
            ConditionRecoveryModel::NOISE_REST_RECOVERY,
            0.05,
        );
        let b = ConditionRecoveryModel::deterministic_noise(
            42,
            12345,
            ConditionRecoveryModel::NOISE_REST_RECOVERY,
            0.05,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn deterministic_noise_decouples_streams_by_salt() {
        // The whole point of the salt is to keep rest / training /
        // match-exertion noise streams independent for the same player
        // on the same day. If the salts produce identical values then
        // the salt isn't actually mixed into the avalanche and the
        // streams are still coupled.
        let pid = 9001;
        let date_ordinal = 740_123; // arbitrary
        let rest = ConditionRecoveryModel::deterministic_noise(
            pid,
            date_ordinal,
            ConditionRecoveryModel::NOISE_REST_RECOVERY,
            0.05,
        );
        let training = ConditionRecoveryModel::deterministic_noise(
            pid,
            date_ordinal,
            ConditionRecoveryModel::NOISE_TRAINING_RECOVERY,
            0.05,
        );
        let exertion = ConditionRecoveryModel::deterministic_noise(
            pid,
            date_ordinal,
            ConditionRecoveryModel::NOISE_MATCH_EXERTION,
            0.05,
        );
        assert_ne!(rest, training, "rest and training salts collided");
        assert_ne!(rest, exertion, "rest and match-exertion salts collided");
        assert_ne!(training, exertion, "training and match-exertion salts collided");
    }

    #[test]
    fn deterministic_noise_stays_within_amplitude() {
        // Sweep a few players / dates / salts and confirm the output
        // never escapes the ±amplitude band — the recovery / exertion
        // paths rely on this invariant when sizing their gains.
        let amp = 0.05_f32;
        let salts = [
            ConditionRecoveryModel::NOISE_REST_RECOVERY,
            ConditionRecoveryModel::NOISE_TRAINING_RECOVERY,
            ConditionRecoveryModel::NOISE_MATCH_EXERTION,
        ];
        for pid in [1u32, 17, 250, 9_999, 1_234_567] {
            for date in [1_i32, 100, 740_000, 800_000] {
                for s in salts {
                    let v = ConditionRecoveryModel::deterministic_noise(pid, date, s, amp);
                    assert!(v >= 1.0 - amp - 1e-5 && v <= 1.0 + amp + 1e-5,
                        "noise {} out of band for pid={} date={} salt={:#x}", v, pid, date, s);
                }
            }
        }
    }

    #[test]
    fn debt_throttle_returns_one_at_or_below_threshold() {
        assert_eq!(ConditionRecoveryModel::debt_throttle(0.0), 1.0);
        assert_eq!(ConditionRecoveryModel::debt_throttle(200.0), 1.0);
        assert_eq!(
            ConditionRecoveryModel::debt_throttle(ConditionRecoveryModel::DEBT_THROTTLE_START),
            1.0
        );
    }

    #[test]
    fn debt_throttle_just_above_threshold_stays_close_to_one() {
        // The whole point of the smoothstep replacement: at debt 351
        // the throttle should be effectively 1.0, not the old ~0.77
        // cliff the linear formula produced.
        let just_above = ConditionRecoveryModel::debt_throttle(351.0);
        assert!(
            just_above > 0.999,
            "throttle just above start should be ~1.0, got {}",
            just_above
        );
    }

    #[test]
    fn debt_throttle_floors_at_min_recovery_mult_when_debt_is_extreme() {
        let at_full =
            ConditionRecoveryModel::debt_throttle(ConditionRecoveryModel::DEBT_THROTTLE_FULL);
        assert!(
            (at_full - ConditionRecoveryModel::MIN_RECOVERY_MULT).abs() < 1e-4,
            "throttle at full should equal MIN_RECOVERY_MULT, got {}",
            at_full
        );
        let way_past =
            ConditionRecoveryModel::debt_throttle(ConditionRecoveryModel::DEBT_THROTTLE_FULL * 2.0);
        assert!(
            (way_past - ConditionRecoveryModel::MIN_RECOVERY_MULT).abs() < 1e-4,
            "throttle past full should clamp to MIN_RECOVERY_MULT, got {}",
            way_past
        );
    }

    #[test]
    fn debt_throttle_is_monotonic_decreasing() {
        // The smoothstep must never temporarily reverse direction —
        // a higher debt always means a smaller recovery multiplier.
        let mut prev = ConditionRecoveryModel::debt_throttle(0.0);
        let mut debt = 0.0;
        while debt <= 2_000.0 {
            let next = ConditionRecoveryModel::debt_throttle(debt);
            assert!(
                next <= prev + 1e-5,
                "throttle non-monotonic at debt {}: prev={} next={}",
                debt,
                prev,
                next
            );
            prev = next;
            debt += 25.0;
        }
    }

    #[test]
    fn debt_throttle_mid_band_is_between_endpoints() {
        // A debt halfway through the throttle band should sit between
        // the endpoints (sanity check against accidentally inverting
        // the smoothstep).
        let mid_debt = (ConditionRecoveryModel::DEBT_THROTTLE_START
            + ConditionRecoveryModel::DEBT_THROTTLE_FULL)
            / 2.0;
        let mid = ConditionRecoveryModel::debt_throttle(mid_debt);
        assert!(mid < 1.0);
        assert!(mid > ConditionRecoveryModel::MIN_RECOVERY_MULT);
    }

    #[test]
    fn training_fatigue_cost_mult_elite_pays_less_than_average() {
        // Same drill, same age, same overload state. The only thing
        // differing is the physical profile — elite stamina / NF /
        // chronic fitness must show up as a meaningfully lower acute
        // cost. If this collapses to the same number then the
        // individualisation isn't really happening.
        let elite = ConditionRecoveryModel::training_fatigue_cost_mult(
            18.0, 18.0, 9_000, 100.0, 1_500, 26,
        );
        let average = ConditionRecoveryModel::training_fatigue_cost_mult(
            10.0, 10.0, 6_000, 100.0, 1_500, 26,
        );
        assert!(
            elite < average - 0.05,
            "elite {} must clearly beat average {}",
            elite,
            average
        );
    }

    #[test]
    fn training_fatigue_cost_mult_overloaded_pays_more_than_fresh() {
        // Same body, same drill — only debt / jadedness differ.
        let fresh = ConditionRecoveryModel::training_fatigue_cost_mult(
            14.0, 14.0, 7_500, 50.0, 1_500, 26,
        );
        let overloaded = ConditionRecoveryModel::training_fatigue_cost_mult(
            14.0, 14.0, 7_500, 1_300.0, 7_500, 26,
        );
        assert!(
            overloaded > fresh + 0.05,
            "overloaded {} must clearly exceed fresh {}",
            overloaded,
            fresh
        );
    }

    #[test]
    fn training_fatigue_cost_mult_age_extremes_pay_more() {
        // Same physical profile, same overload state — only age
        // differs. A 16-year-old and a 36-year-old should both pay
        // more than a 26-year-old for the same drill.
        let prime = ConditionRecoveryModel::training_fatigue_cost_mult(
            14.0, 14.0, 7_500, 100.0, 1_500, 26,
        );
        let kid = ConditionRecoveryModel::training_fatigue_cost_mult(
            14.0, 14.0, 7_500, 100.0, 1_500, 16,
        );
        let vet = ConditionRecoveryModel::training_fatigue_cost_mult(
            14.0, 14.0, 7_500, 100.0, 1_500, 36,
        );
        assert!(kid > prime, "kid {} should exceed prime {}", kid, prime);
        assert!(vet > prime, "vet {} should exceed prime {}", vet, prime);
    }

    #[test]
    fn training_fatigue_cost_mult_stays_within_band() {
        // Both extremes (all-elite + fresh + prime, all-poor + jaded +
        // veteran) must respect the design 0.82..1.35 band — the
        // multiplier is meant to tilt, not to make heavy drills free
        // or ruinous.
        let best = ConditionRecoveryModel::training_fatigue_cost_mult(
            20.0, 20.0, 10_000, 0.0, 0, 26,
        );
        let worst = ConditionRecoveryModel::training_fatigue_cost_mult(
            0.0, 0.0, 0, 2_000.0, 10_000, 40,
        );
        assert!(best >= 0.82 - 1e-5 && best <= 1.35 + 1e-5, "best out of band: {}", best);
        assert!(worst >= 0.82 - 1e-5 && worst <= 1.35 + 1e-5, "worst out of band: {}", worst);
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_player(birth: NaiveDate, natural_fitness: f32) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = 5_000;
        attrs.fitness = 7_000;
        attrs.jadedness = 2_000;
        attrs.days_since_last_match = 1;
        let mut skills = PlayerSkills::default();
        skills.physical.natural_fitness = natural_fitness;
        skills.physical.match_readiness = 13.0;
        PlayerBuilder::new()
            .id(11)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(birth)
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    #[test]
    fn one_match_per_week_mostly_recovers_by_match_day() {
        // 50% condition midfielder with a week's rest. Rest-only
        // recovery is intentionally slow — players rely on training
        // recovery sessions to top up — but a week should still bring
        // condition back to a "selectable" zone (≥75%).
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.player_attributes.condition = 5_000;
        let start = d(2025, 9, 1);
        for i in 0..7 {
            p.process_condition_recovery(start + chrono::Duration::days(i));
        }
        assert!(
            p.player_attributes.condition >= 7_500,
            "condition after week of rest: {}",
            p.player_attributes.condition
        );
        // And it should NOT have overshot the 9000 normal level.
        assert!(p.player_attributes.condition <= 9_000);
    }

    #[test]
    fn older_player_recovers_slower_than_younger_with_same_nf() {
        let mut young = make_player(d(2003, 1, 1), 14.0); // ~22 in 2025
        let mut old = make_player(d(1990, 1, 1), 14.0); // ~35 in 2025
        young.player_attributes.condition = 5_000;
        old.player_attributes.condition = 5_000;
        // The deficit-based recovery is asymptotic — the per-day cap
        // pins both players together for the first couple of days
        // when the deficit is huge. Run over a week of distinct
        // calendar dates so the post-cap asymptotic regime (where the
        // older player's smaller recovery rate visibly trails the
        // younger player's) has time to express itself.
        let start = d(2025, 9, 1);
        for i in 0..7 {
            let day = start + chrono::Duration::days(i);
            young.process_condition_recovery(day);
            old.process_condition_recovery(day);
        }
        assert!(
            young.player_attributes.condition > old.player_attributes.condition,
            "young {} should recover faster than old {}",
            young.player_attributes.condition,
            old.player_attributes.condition
        );
    }

    #[test]
    fn high_natural_fitness_helps_recovery_but_isnt_immune() {
        // Different ids so deterministic noise can break ties on
        // days where both players are pegged at the per-day cap.
        let mut elite = make_player_with_id(d(2000, 1, 1), 19.0, 101);
        let mut average = make_player_with_id(d(2000, 1, 1), 8.0, 102);
        elite.player_attributes.condition = 5_000;
        average.player_attributes.condition = 5_000;
        // Multi-day window so the deficit-based recovery's individual
        // rate (higher for elite NF) actually pulls away from the
        // per-day cap regime that pins them together early on.
        let start = d(2025, 9, 1);
        for i in 0..7 {
            let day = start + chrono::Duration::days(i);
            elite.process_condition_recovery(day);
            average.process_condition_recovery(day);
        }
        assert!(
            elite.player_attributes.condition > average.player_attributes.condition,
            "elite NF {} should beat avg NF {}",
            elite.player_attributes.condition,
            average.player_attributes.condition
        );
        // Neither should be at full — recovery is asymptotic, not
        // a refill.
        assert!(elite.player_attributes.condition < 9_700);
    }

    fn make_player_with_id(birth: NaiveDate, natural_fitness: f32, id: u32) -> Player {
        let mut p = make_player(birth, natural_fitness);
        p.id = id;
        p
    }

    #[test]
    fn condition_label_returning_from_short_injury() {
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.player_attributes.is_injured = false;
        p.player_attributes.recovery_days_remaining = 5;
        assert_eq!(p.condition_label(), ConditionLabel::ReturningFromInjury);
    }

    #[test]
    fn condition_label_limited_minutes_for_long_recovery() {
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.player_attributes.is_injured = false;
        p.player_attributes.recovery_days_remaining = 21;
        assert_eq!(
            p.condition_label(),
            ConditionLabel::LimitedMinutesRecommended
        );
    }

    #[test]
    fn condition_label_heavy_legs_when_debt_is_high() {
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.load.recovery_debt = 500.0;
        p.player_attributes.condition = 9_500;
        assert_eq!(p.condition_label(), ConditionLabel::HeavyLegs);
    }

    #[test]
    fn condition_label_needs_rest_when_jaded() {
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.player_attributes.jadedness = 8_000;
        assert_eq!(p.condition_label(), ConditionLabel::NeedsRest);
    }

    #[test]
    fn condition_label_lacking_match_sharpness() {
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.skills.physical.match_readiness = 6.0;
        p.player_attributes.days_since_last_match = 21;
        assert_eq!(p.condition_label(), ConditionLabel::LackingMatchSharpness);
    }

    #[test]
    fn condition_label_fresh_for_well_rested_sharp_player() {
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.player_attributes.condition = 9_500;
        p.player_attributes.jadedness = 1_500;
        p.skills.physical.match_readiness = 16.0;
        // No matches in last 7d, no debt
        assert_eq!(p.condition_label(), ConditionLabel::Fresh);
    }

    // ════════════════════════════════════════════════════════════════
    // Individualized recovery — proves the deficit-based model
    // actually keeps players physically distinct, instead of washing
    // the squad toward a single dynamic cap. These tests cover the
    // acceptance criteria from the condition redesign: stamina/NF
    // spread, debt spread, training-session non-equalization,
    // position-driven match drain, and a multi-week squad spread.
    // ════════════════════════════════════════════════════════════════

    fn make_pro_player(
        id: u32,
        birth: NaiveDate,
        natural_fitness: f32,
        stamina: f32,
        fitness: i16,
    ) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = 5_000;
        attrs.fitness = fitness;
        attrs.jadedness = 2_000;
        attrs.days_since_last_match = 1;
        let mut skills = PlayerSkills::default();
        skills.physical.natural_fitness = natural_fitness;
        skills.physical.stamina = stamina;
        skills.physical.match_readiness = 13.0;
        let mut person_attrs = PersonAttributes::default();
        person_attrs.professionalism = 12.0;
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(birth)
            .country_id(1)
            .attributes(person_attrs)
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    #[test]
    fn same_minutes_different_stamina_nf_diverge_after_seven_days() {
        // Two players starting from the same condition. One has elite
        // stamina+NF+fitness; the other is average across the board.
        // After a week of identical rest days they should NOT land at
        // the same condition — the individualized target and the
        // recovery-rate uplift for elite physiology must show up.
        let mut elite =
            make_pro_player(201, d(2000, 1, 1), 18.0, 17.0, 8_500);
        let mut average =
            make_pro_player(202, d(2000, 1, 1), 10.0, 10.0, 6_500);
        elite.player_attributes.condition = 5_000;
        average.player_attributes.condition = 5_000;

        let start = d(2025, 9, 1);
        for i in 0..7 {
            let day = start + chrono::Duration::days(i);
            elite.process_condition_recovery(day);
            average.process_condition_recovery(day);
        }
        let spread =
            (elite.player_attributes.condition - average.player_attributes.condition) as i32;
        assert!(
            spread >= 250,
            "elite ({}) - average ({}) spread {} should be ≥ 250",
            elite.player_attributes.condition,
            average.player_attributes.condition,
            spread
        );
    }

    #[test]
    fn high_recovery_debt_suppresses_recovery_relative_to_low_debt() {
        // Two players identical in every way except recovery debt.
        // After three days the heavy-debt one should be visibly
        // behind, because both the per-day debt throttle AND the
        // target's load_drag pull the heavy-debt player's daily
        // recovery downward.
        let mut fresh = make_pro_player(301, d(2000, 1, 1), 14.0, 14.0, 7_500);
        let mut burdened =
            make_pro_player(302, d(2000, 1, 1), 14.0, 14.0, 7_500);
        fresh.player_attributes.condition = 5_000;
        burdened.player_attributes.condition = 5_000;
        // Heavy legs: ~halfway through the throttle band, and enough
        // load_drag to suppress the target meaningfully.
        burdened.load.recovery_debt = 1_100.0;

        let start = d(2025, 9, 1);
        for i in 0..3 {
            let day = start + chrono::Duration::days(i);
            fresh.process_condition_recovery(day);
            burdened.process_condition_recovery(day);
        }
        assert!(
            fresh.player_attributes.condition > burdened.player_attributes.condition,
            "fresh ({}) should outpace burdened ({})",
            fresh.player_attributes.condition,
            burdened.player_attributes.condition
        );
    }

    #[test]
    fn overloaded_target_is_lower_than_fresh_target() {
        // Pure helper test — target() responds to load_drag, debt, and
        // jadedness. An overloaded player's daily ceiling must sit
        // beneath a fresh player's with the same physiology.
        let fresh_target = ConditionRecoveryModel::individualized_target(ConditionTargetInputs {
            natural_fitness: 14.0,
            stamina: 14.0,
            chronic_fitness: 7_500,
            match_readiness: 14.0,
            physical_load_7: 50.0,
            recovery_debt: 50.0,
            jadedness: 1_500,
            age: 26,
        });
        let overloaded_target =
            ConditionRecoveryModel::individualized_target(ConditionTargetInputs {
                natural_fitness: 14.0,
                stamina: 14.0,
                chronic_fitness: 7_500,
                match_readiness: 14.0,
                physical_load_7: 580.0,
                recovery_debt: 1_300.0,
                jadedness: 6_500,
                age: 26,
            });
        assert!(
            fresh_target > overloaded_target + 500.0,
            "fresh target {} should be much higher than overloaded {}",
            fresh_target,
            overloaded_target
        );
        assert!(overloaded_target >= ConditionRecoveryModel::TARGET_FLOOR);
        assert!(fresh_target <= ConditionRecoveryModel::TARGET_CEILING);
    }

    #[test]
    fn squad_spread_survives_four_weeks_of_alternating_rest() {
        // Ten differentiated players running the daily recovery for
        // four simulated weeks. Acceptance criterion: condition spread
        // across the squad must remain meaningful — under the old
        // model they all collapsed into 88-95%.
        let start = d(2025, 9, 1);
        let mut squad: Vec<Player> = (0..10)
            .map(|i| {
                // Spread NF / stamina / fitness / age across the squad
                // so the targets and rates span a realistic band.
                let nf = 6.0 + (i as f32) * 1.4;
                let stamina = 7.0 + (i as f32) * 1.2;
                let fitness = 6_000 + (i as i16) * 350;
                let age_year = 1986 + i;
                let birth = NaiveDate::from_ymd_opt(age_year as i32, 1, 1).unwrap();
                let mut p = make_pro_player(1_000 + i as u32, birth, nf, stamina, fitness);
                // Random-ish starting condition between 55% and 80%
                p.player_attributes.condition = 5_500 + (i as i16 * 200);
                p.load.physical_load_7 = (i as f32) * 60.0;
                p.load.recovery_debt = (i as f32) * 120.0;
                p
            })
            .collect();

        for i in 0..28 {
            let day = start + chrono::Duration::days(i);
            for p in &mut squad {
                p.process_condition_recovery(day);
            }
        }
        let conditions: Vec<i16> =
            squad.iter().map(|p| p.player_attributes.condition).collect();
        let min = *conditions.iter().min().unwrap();
        let max = *conditions.iter().max().unwrap();
        assert!(
            (max - min) >= 400,
            "squad spread collapsed: min={} max={} (all {:?})",
            min,
            max,
            conditions
        );
    }
}
