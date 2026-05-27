//! Physical exertion side effects of featuring in a match.
//!
//! Distinct from [`super::match_play`]: that handles morale + stats
//! bookkeeping at full-time; this owns the post-match physical book
//! (load, jadedness, recovery debt, sharpness, post-match injury
//! roll). Engine-side condition drain already happened tick by tick;
//! everything here is *post-match* only.

use chrono::{Datelike, NaiveDate};

use crate::PlayerPositionType;
use crate::PlayerStatusType;
use crate::club::PlayerFieldPositionGroup;
use crate::club::player::condition::{ConditionRecoveryModel, InjuryRiskInputs};
use crate::club::player::injury::InjuryType;
use crate::club::player::player::Player;
use crate::utils::DateUtils;

/// Compact inputs to the post-match physical exertion pass. Built
/// either from a [`PlayerMatchPhysicalSnapshot`] (engine path — owns
/// the actual end-of-match energy) or via the legacy minute-count
/// fallback (`MatchExertionInputs::from_minutes`) for callers that
/// construct match results without the engine. The persisted
/// `Player::on_match_exertion` only ever reads this shape, so the two
/// paths cannot drift.
#[derive(Debug, Clone, Copy)]
pub struct MatchExertionInputs {
    /// Minutes spent on the pitch (fractional — cameo subs need
    /// sub-minute resolution to size the post-match drop sensibly).
    pub minutes: f32,
    /// Condition (0..10000) when the player took the pitch. The
    /// "tank size" at the start of *this* shift on the pitch.
    pub starting_condition: i16,
    /// Condition (0..10000) when the player left the pitch — the
    /// engine's tick-by-tick drain has been applied here. Subbed-off
    /// = exit-minute condition; played-90 = full-time condition.
    pub final_match_energy: i16,
    /// Engine-side high-intensity hint (0..1). Seeded from the
    /// player's position group share. Falls back to a neutral 0.20
    /// for the legacy minute-only path.
    pub high_intensity_load_hint: f32,
}

impl MatchExertionInputs {
    /// Synthesise inputs from a minute count when the engine couldn't
    /// supply a snapshot. Used as a fallback in
    /// `LeagueResult::apply_post_match_physical_effects` for old saves
    /// and harness callers that pre-date the snapshot pipeline. We
    /// assume the player ended the match around 50% condition (a
    /// reasonable average) and seed `starting_condition` from the
    /// player's *current* persisted condition. The post-match formula
    /// still scales by duration/position/age, so the result remains
    /// in the right ballpark — just less responsive to outliers.
    ///
    /// `high_intensity_load_hint` is seeded from the player's position
    /// group default (GK ~0.05, defender ~0.20, midfielder ~0.30,
    /// forward ~0.32) so the synthesis preserves the engine path's
    /// position-aware HI-share spread when no snapshot is available.
    pub fn from_minutes(player: &Player, minutes: f32) -> Self {
        let starting_condition = player.player_attributes.condition;
        // Estimate end-of-match condition from duration only: a 90-min
        // shift typically ends ~55% of starting energy; a cameo barely
        // dents the tank. Floors at 1500 to mirror the engine's
        // in-match clamp.
        let duration = (minutes / 90.0).clamp(0.0, 1.35);
        let energy_loss_ratio = (0.45 * duration).min(0.85);
        let final_match_energy =
            ((starting_condition as f32) * (1.0 - energy_loss_ratio)).max(1500.0) as i16;
        let group = player.position().position_group();
        MatchExertionInputs {
            minutes,
            starting_condition,
            final_match_energy,
            high_intensity_load_hint: PositionLoad::high_intensity_share(group),
        }
    }
}

/// Youth-aware match exertion modifiers. Encapsulates the three knobs the
/// post-match exertion path adjusts when the player is too young for adult
/// senior workloads. Friendlies always return the identity multipliers —
/// pre-season cameos already use a reduced friendly factor and shouldn't
/// be punished twice.
///
/// Football model: a 14-15-year-old playing 90 senior minutes carries a
/// physiological cost roughly 1.8× a peer adult's, both because the body
/// is still developing and because competitive intensity in modern senior
/// football is calibrated to adult bodies. Recovery debt scales even
/// harder (2.0×) — kids take longer to bounce back from the same load.
struct YouthMatchExertion;

impl YouthMatchExertion {
    /// Multiplier applied to physical match load (and downstream jadedness).
    fn load_multiplier(age: u8, is_friendly: bool) -> f32 {
        if is_friendly {
            return 1.0;
        }
        match age {
            0..=15 => 1.8,
            16..=17 => 1.25,
            _ => 1.0,
        }
    }

    /// Multiplier applied to recovery debt accumulated post-match.
    fn debt_multiplier(age: u8, is_friendly: bool) -> f32 {
        if is_friendly {
            return 1.0;
        }
        match age {
            0..=15 => 2.0,
            16..=17 => 1.30,
            _ => 1.0,
        }
    }

    /// Extra base injury rate added to the senior-match injury roll for
    /// very young players. Adolescent ligaments and growth plates carry
    /// higher risk under adult match intensity. Friendlies don't get the
    /// bump.
    fn injury_bonus(age: u8, is_friendly: bool) -> f32 {
        if is_friendly {
            return 0.0;
        }
        match age {
            0..=15 => 0.004,
            16..=17 => 0.0015,
            _ => 0.0,
        }
    }
}

impl Player {
    /// Apply the physical cost of featuring in a match. The match engine
    /// already drained condition tick-by-tick during the sim — this hook
    /// owns *post-match* effects:
    ///
    ///   * **Persisted condition drop** sized from the engine's
    ///     `final_match_energy` snapshot. Duration, position, stamina,
    ///     natural_fitness, age, depletion, high-intensity share,
    ///     congestion, and friendly-discount all feed the depletion
    ///     formula. Without this the persisted `Player.condition` could
    ///     still read 90% after a full 90-minute slog.
    ///   * Minute & physical-load bookkeeping ([`PlayerLoad`]).
    ///   * Recovery-debt accumulation, scaled by depletion + congestion.
    ///   * Jadedness, scaled by position group, minutes, and depletion.
    ///   * Match-readiness boost (sharpness).
    ///   * "Rst" status flagging when jadedness crosses the threshold.
    ///   * Post-match injury roll via the unified risk recipe.
    ///
    /// Friendlies get a reduced load and reduced injury chance, but
    /// still some sharpness gain — pre-season cameos really do build
    /// match fitness.
    ///
    /// Legacy callers without a snapshot use
    /// [`Self::on_match_exertion_minutes_only`]; that path synthesises
    /// inputs from minute count alone via
    /// [`MatchExertionInputs::from_minutes`].
    pub fn on_match_exertion(
        &mut self,
        inputs: MatchExertionInputs,
        now: NaiveDate,
        is_friendly: bool,
    ) {
        let minutes = inputs.minutes.max(0.0);
        self.load.record_match_minutes(minutes, is_friendly);

        let position = self.position();
        let group = position.position_group();
        let position_factor = PositionLoad::match_load_factor(position);
        let hi_share_default = PositionLoad::high_intensity_share(group);
        // Prefer the engine-supplied hint when it looks valid; fall
        // back to the position-group default. A negative or NaN
        // hint shouldn't poison the formula.
        let hi_share = if inputs.high_intensity_load_hint.is_finite()
            && inputs.high_intensity_load_hint > 0.0
        {
            inputs.high_intensity_load_hint.min(1.0)
        } else {
            hi_share_default
        };

        let age = DateUtils::age(self.birth_date, now);
        let maturity_load = YouthMatchExertion::load_multiplier(age, is_friendly);
        let maturity_debt = YouthMatchExertion::debt_multiplier(age, is_friendly);

        // ── PERSISTED CONDITION DROP ──────────────────────────────
        // Sized from the snapshot's `(starting → final)` energy span,
        // duration, position, stamina/NF resistance, age, congestion,
        // and friendly discount. Calibrated so a 90-min midfielder
        // who finished around 50% loses ~22-32 percentage points of
        // persisted condition (2200..3200 raw units); a GK 90' loses
        // materially less; a wingback 90' more; a 20-min cameo only
        // 3-8 points; a 90' friendly ~55% of competitive.
        //
        // The drop is decoupled from the engine's in-match floor
        // (1500): persisted condition represents *next-day* freshness,
        // which is never as bad as the worst tick during the match.
        // A player who finished on fumes still recovers overnight,
        // just from a deeper hole.
        let base_condition_drop = Self::compute_condition_drop(
            &inputs,
            position_factor,
            hi_share,
            self.skills.physical.stamina,
            self.skills.physical.natural_fitness,
            age,
            self.load.matches_last_14(),
            is_friendly,
        );
        // Action-style multiplier: a high-work-rate pressing winger
        // with pace and acceleration to burn pays more for 90 minutes
        // than a low-block CB who covered the same minutes. Applied to
        // the persisted condition drop and to the post-match recovery
        // debt below — *not* to minutes (which feed selection rotation
        // and shouldn't be inflated for the wide players who can
        // genuinely repeat the workload).
        let action_style_mult = Self::action_style_mult(
            hi_share,
            self.skills.mental.work_rate,
            self.skills.physical.pace,
            self.skills.physical.acceleration,
        );
        // ±4% deterministic noise so two identical profiles in the
        // same match don't end up at byte-for-byte identical condition
        // — stable for the `(player, date, salt)` triple. The match-
        // exertion salt keeps this stream independent from the rest /
        // training paths so a "matchday + recovery + overnight rest"
        // sequence can't have all three noise factors line up the same
        // sign for one unlucky player.
        let date_ordinal = now.num_days_from_ce();
        let exertion_noise = ConditionRecoveryModel::deterministic_noise(
            self.id,
            date_ordinal,
            ConditionRecoveryModel::NOISE_MATCH_EXERTION,
            0.04,
        );
        let condition_drop = base_condition_drop * action_style_mult * exertion_noise;

        // Floor: competitive 25%, friendly 35%, post-injury recovery
        // 40% (unless an injury actually fires later in this pass —
        // injuries get their own state machine). The floor is a worst-
        // case stabilization: the player did not collapse below this
        // even though the engine drained them to 15% mid-match. The
        // floor only LIFTS a player who started the match above the
        // floor and burned through it — a player who entered the match
        // already below the floor stays at their starting condition.
        // Match exertion never heals; recovery is owned by the daily
        // rest / training pathways.
        let post_match_floor = if is_friendly {
            3500
        } else if self.player_attributes.is_in_recovery() {
            4000
        } else {
            2500
        };
        let starting = inputs.starting_condition.max(0);
        // Persisted condition = max(starting − drop, floor), then
        // never raised above starting_condition. The min(starting)
        // clamp is the "never heal" guarantee: a player who walked
        // onto the pitch already at 20% leaves it at 20% even if the
        // 25% competitive floor would otherwise lift them. Overnight
        // recovery belongs to `process_condition_recovery`, not here.
        let computed_after_match = (starting as f32 - condition_drop).round() as i32;
        let floored = computed_after_match.max(post_match_floor as i32);
        let new_condition = floored.min(starting as i32).clamp(0, 10_000) as i16;
        self.player_attributes.condition = new_condition;

        // ── LOAD BOOKKEEPING ──────────────────────────────────────
        // Position-weighted minutes (legacy compat for the rolling
        // load windows). The friendly discount is applied inside
        // `record_match_load`.
        let condition_pct_after = self.player_attributes.condition_percentage() as f32;
        let depletion_factor = if condition_pct_after < 50.0 {
            1.0 + (50.0 - condition_pct_after) / 80.0
        } else {
            1.0
        };
        let match_load = minutes * position_factor * depletion_factor * maturity_load;
        let hi_load = match_load * hi_share;
        self.load
            .record_match_load(match_load, hi_load, is_friendly);

        let friendly_intensity = if is_friendly { 0.45 } else { 1.0 };

        // Debt: half from raw load, half from "running on fumes" tax,
        // amplified by the per-shift depletion. A 90-min midfielder
        // at 60% finish adds ~45 units; a 90-min forward at 30%
        // finish adds ~80. Maturity multiplier hits debt harder
        // (2.0× for under-15s) than load (1.8×). The same
        // action-style multiplier that inflated the condition drop
        // also lifts debt — a wide presser carries deeper tiredness
        // overnight than a low-block CB for the same minutes.
        let depletion_tax = match_load * (50.0 - condition_pct_after).max(0.0) / 100.0;
        let debt_add = (match_load * 0.5 + depletion_tax)
            * friendly_intensity
            * maturity_debt
            * action_style_mult
            / maturity_load.max(0.001);
        self.load.add_recovery_debt(debt_add);

        // ── SHARPNESS ─────────────────────────────────────────────
        // Cameo subs (<15 min) don't rebuild readiness; full 90 =
        // +3.0; friendlies sharpen at 70%.
        if minutes >= 15.0 {
            let mut readiness_boost = (minutes / 90.0) * 3.0;
            if is_friendly {
                readiness_boost *= 0.7;
            }
            self.skills.physical.match_readiness =
                (self.skills.physical.match_readiness + readiness_boost).min(20.0);
        }

        // ── JADEDNESS ─────────────────────────────────────────────
        // Scaled by match_load (which carries the youth multiplier)
        // and recent congestion, with the same friendly discount.
        let congestion = self.load.matches_last_14() as f32;
        let congestion_mult = 1.0 + (congestion - 2.0).max(0.0) * 0.20;
        let jad_gain = (match_load * friendly_intensity * 4.0 * congestion_mult).round() as i32;
        let new_jad = self.player_attributes.jadedness as i32 + jad_gain;
        self.player_attributes.jadedness = new_jad.clamp(0, 10_000) as i16;

        if self.player_attributes.jadedness > 7000
            && !self.statuses.get().contains(&PlayerStatusType::Rst)
        {
            self.statuses.add(now, PlayerStatusType::Rst);
        }

        self.player_attributes.days_since_last_match = 0;

        if !self.player_attributes.is_injured {
            let in_recovery = self.player_attributes.is_in_recovery();
            self.roll_for_match_injury(minutes, match_load, now, in_recovery, age, is_friendly);
        }
    }

    /// Back-compat shim for callers that only know the minute count
    /// (legacy harnesses, intelligence tests, etc). Synthesises a
    /// `MatchExertionInputs` from the player's current state and the
    /// supplied minutes, then runs the full exertion pass.
    pub fn on_match_exertion_minutes_only(
        &mut self,
        minutes: f32,
        now: NaiveDate,
        is_friendly: bool,
    ) {
        let inputs = MatchExertionInputs::from_minutes(self, minutes);
        self.on_match_exertion(inputs, now, is_friendly);
    }

    /// Tactical / role / style multiplier applied on top of the
    /// position-and-physiology drop. A high-work-rate pressing winger
    /// with pace and acceleration bills more per minute than the same
    /// minutes from a low-block CB; that's what makes "a 90 from a
    /// wide presser" feel different from "a 90 from a back-three CB"
    /// in the daily standings.
    ///
    /// Sits in the 0.90..1.32 band for typical players (HI share
    /// 0..0.4, all three traits 0..20):
    ///   neutral CB (HI 0.2, traits 10): ~1.05
    ///   pressing winger (HI 0.35, WR 17, pace 16, acc 16): ~1.23
    pub fn action_style_mult(
        high_intensity_share: f32,
        work_rate: f32,
        pace: f32,
        acceleration: f32,
    ) -> f32 {
        let hi = high_intensity_share.clamp(0.0, 1.0);
        let wr01 = (work_rate / 20.0).clamp(0.0, 1.0);
        let pace01 = (pace / 20.0).clamp(0.0, 1.0);
        let acc01 = (acceleration / 20.0).clamp(0.0, 1.0);
        0.90 + hi * 0.20 + wr01 * 0.10 + pace01 * 0.06 + acc01 * 0.06
    }

    /// Pure helper for the persisted condition-drop formula. Lives
    /// outside `on_match_exertion` so tests can pin the curve without
    /// constructing a full `Player`. All inputs are physical /
    /// situational; no `&self` so the post-match path can also reach
    /// in for diagnostics without re-running the whole exertion pass.
    ///
    /// Note: this returns the *base* drop. The tactical/style
    /// multiplier and exertion noise are applied in
    /// `on_match_exertion`, not here, so this remains a stable
    /// reference curve for unit tests.
    pub fn compute_condition_drop(
        inputs: &MatchExertionInputs,
        position_factor: f32,
        high_intensity_share: f32,
        stamina: f32,
        natural_fitness: f32,
        age: u8,
        matches_last_14: u8,
        is_friendly: bool,
    ) -> f32 {
        let minutes = inputs.minutes.max(0.0);
        if minutes < 1.0 {
            return 0.0;
        }
        // Duration scales sub-linearly: a 90-min slog doesn't cost 2×
        // a 45-min slog (the first minute is "free" from a depletion
        // standpoint, the last minutes are the most expensive — the
        // shape is captured by .powf(0.90), not pure linear).
        let duration = (minutes / 90.0).clamp(0.05, 1.35);

        // Stamina / natural_fitness resistance: high values reduce
        // the drop. Stamina pulls 0.85..1.20; NF 0.90..1.15.
        let stamina_clamped = stamina.clamp(0.0, 20.0);
        let nf_clamped = natural_fitness.clamp(0.0, 20.0);
        let stamina_resistance = 1.20 - 0.35 * (stamina_clamped / 20.0);
        let natural_fitness_resistance = 1.15 - 0.25 * (nf_clamped / 20.0);

        // Age multiplier — kids and veterans bleed more condition for
        // the same workload.
        let age_mult: f32 = match age {
            0..=15 => 1.45,
            16..=17 => 1.25,
            18..=21 => 1.05,
            22..=29 => 1.00,
            _ => (1.0 + (age as f32 - 30.0) * 0.035).clamp(1.0, 1.25),
        };

        // How much of the tank the player burned during this shift on
        // the pitch. Calibrated against the engine floor (1500): a
        // player who started at 9000 and ended at 1500 has drained
        // 7500, which the formula reads as "fully gassed".
        let energy_span = (inputs.starting_condition - inputs.final_match_energy).max(0) as f32;
        let match_energy_drop = (energy_span / 6500.0).clamp(0.0, 1.0);
        let hi_share = high_intensity_share.clamp(0.0, 1.0);
        let intensity_mult = 0.85 + 0.50 * match_energy_drop + 0.20 * hi_share;

        let congestion_mult = 1.0 + (matches_last_14.saturating_sub(2) as f32) * 0.07;
        let friendly_mult = if is_friendly { 0.55 } else { 1.0 };

        2300.0
            * duration.powf(0.90)
            * position_factor
            * stamina_resistance
            * natural_fitness_resistance
            * age_mult
            * intensity_mult
            * congestion_mult
            * friendly_mult
    }

    /// Match injury roll using the unified risk model. Inputs feed the
    /// shared `compute_injury_risk` helper so spontaneous, training,
    /// match, and setback risks all read from the same recipe.
    fn roll_for_match_injury(
        &mut self,
        minutes: f32,
        match_load: f32,
        now: NaiveDate,
        in_recovery: bool,
        age: u8,
        is_friendly: bool,
    ) {
        let natural_fitness = self.skills.physical.natural_fitness;
        let condition_pct = self.player_attributes.condition_percentage();
        let injury_proneness = self.player_attributes.injury_proneness;

        // Base rate: 0.5% scaled by minutes; the unified helper applies
        // the multiplicative modifiers (proneness, age, NF, jadedness,
        // workload spike, last body part, congestion, in-recovery). For
        // adolescent players in senior competitive matches we add a
        // maturity-driven base bump on top — adult-intensity football on
        // not-yet-adult ligaments / growth plates.
        let base_rate = (0.005 + YouthMatchExertion::injury_bonus(age, is_friendly))
            * (minutes / 90.0).max(0.05);

        let intensity = (match_load / 90.0).clamp(0.4, 2.0);

        let chance = self.compute_injury_risk(InjuryRiskInputs {
            base_rate,
            intensity,
            in_recovery,
            medical_multiplier: 1.0,
            now,
        });

        if rand::random::<f32>() < chance {
            let injury = InjuryType::random_match_injury(
                minutes,
                age,
                condition_pct,
                natural_fitness,
                injury_proneness,
            );
            self.player_attributes.set_injury(injury);
            self.statuses.add(now, PlayerStatusType::Inj);
        }
    }
}

/// Position-specific load model used by [`Player::on_match_exertion`]
/// and the match engine's end-of-match snapshot path.
/// Encapsulates the per-position load factor (minute-equivalent
/// multiplier) and the high-intensity share of that load. Both are pure
/// lookup tables and live as associated functions for namespacing — a
/// future "stadium altitude" or "pitch surface" tweak can grow this
/// struct into a configurable model without changing the call site.
pub struct PositionLoad;

impl PositionLoad {
    /// Position-specific multiplier applied to match minutes when
    /// computing physical load. Calibrated so a CB at neutral intensity
    /// is the reference (1.0 ≈ minute-equivalent), keepers materially
    /// under, and modern fullbacks/wide-mids over.
    pub fn match_load_factor(position: PlayerPositionType) -> f32 {
        use crate::PlayerPositionType::*;
        match position {
            Goalkeeper => 0.45,
            Sweeper | DefenderCenter | DefenderCenterLeft | DefenderCenterRight => 0.85,
            DefenderLeft | DefenderRight => 1.05,
            WingbackLeft | WingbackRight => 1.18,
            DefensiveMidfielder => 0.95,
            MidfielderCenter | MidfielderCenterLeft | MidfielderCenterRight => 1.05,
            MidfielderLeft | MidfielderRight => 1.10,
            AttackingMidfielderLeft | AttackingMidfielderRight => 1.05,
            AttackingMidfielderCenter => 0.95,
            Striker | ForwardCenter => 0.95,
            ForwardLeft | ForwardRight => 1.05,
        }
    }

    /// Share of physical load that comes from high-intensity actions
    /// (sprints, presses, repeated accelerations). Forwards and wide
    /// midfielders sprint more than holding mids; keepers very little.
    pub fn high_intensity_share(group: PlayerFieldPositionGroup) -> f32 {
        use crate::club::PlayerFieldPositionGroup::*;
        match group {
            Goalkeeper => 0.05,
            Defender => 0.20,
            Midfielder => 0.30,
            Forward => 0.32,
        }
    }
}
