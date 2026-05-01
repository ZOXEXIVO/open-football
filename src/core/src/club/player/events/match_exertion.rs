//! Physical exertion side effects of featuring in a match.
//!
//! Distinct from [`super::match_play`]: that handles morale + stats
//! bookkeeping at full-time; this owns the post-match physical book
//! (load, jadedness, recovery debt, sharpness, post-match injury
//! roll). Engine-side condition drain already happened tick by tick;
//! everything here is *post-match* only.

use chrono::NaiveDate;

use crate::club::player::injury::InjuryType;
use crate::club::player::player::Player;
use crate::utils::DateUtils;
use crate::PlayerStatusType;

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
    /// already drained condition tick-by-tick during the sim; this hook
    /// owns *post-match* effects only:
    ///
    ///   * minute & physical-load bookkeeping (`PlayerLoad`)
    ///   * recovery-debt accumulation, scaled by depletion + congestion
    ///   * jadedness, scaled by position group and minutes (no more
    ///     step-function 200/400)
    ///   * match-readiness boost (sharpness)
    ///   * "Rst" status flagging when jadedness crosses the threshold
    ///   * post-match injury roll, with workload spike + in-recovery
    ///     setback risk feeding the unified risk model.
    ///
    /// Friendlies get a reduced load and reduced injury chance, but
    /// still some sharpness gain — pre-season cameos really do build
    /// match fitness.
    pub fn on_match_exertion(&mut self, minutes: f32, now: NaiveDate, is_friendly: bool) {
        self.load.record_match_minutes(minutes, is_friendly);

        let position = self.position();
        let group = position.position_group();
        let position_factor = PositionLoad::match_load_factor(position);
        let hi_share = PositionLoad::high_intensity_share(group);

        let age = DateUtils::age(self.birth_date, now);
        let maturity_load = YouthMatchExertion::load_multiplier(age, is_friendly);
        let maturity_debt = YouthMatchExertion::debt_multiplier(age, is_friendly);

        // Condition entering the second half / late game already ticked
        // the engine's drain, so the post-match condition is our best
        // proxy for "how empty is the tank?". Below ~50% it amplifies
        // load/debt — running on fumes hurts more than running fresh.
        let condition_pct = self.player_attributes.condition_percentage() as f32;
        let depletion_factor = if condition_pct < 50.0 {
            1.0 + (50.0 - condition_pct) / 80.0
        } else {
            1.0
        };

        let friendly_factor = if is_friendly { 0.45 } else { 1.0 };

        // 1.0 unit per minute at neutral CB intensity, scaled by position,
        // how empty the player finished, and (for senior competitive
        // matches) the player's biological maturity. A 90-min senior
        // start for a 14yo carries 1.8× the physical load of an adult
        // peer — protecting the kids from being silently pushed past
        // sustainable workloads by a manager who insists on selecting
        // them every week.
        let match_load = minutes * position_factor * depletion_factor * friendly_factor * maturity_load;
        let hi_load = match_load * hi_share;
        self.load
            .record_match_load(match_load, hi_load, is_friendly);

        // Debt: half from raw load, half from "running on fumes" tax.
        // A 90-min midfielder at 60% finish adds ~45 units; a 90-min
        // forward at 30% finish adds ~80. Maturity multiplier hits even
        // harder (2.0× for under-15s) because adolescent recovery is
        // slower than adult recovery for the same load.
        let depletion_tax = match_load * (50.0 - condition_pct).max(0.0) / 100.0;
        let debt_add = (match_load * 0.5 + depletion_tax * friendly_factor) * maturity_debt
            / maturity_load.max(0.001);
        // Note: match_load already carries `maturity_load`. We re-scale
        // to apply the *debt-specific* maturity multiplier instead, so
        // 14yo debt grows by 2.0× while load grows by 1.8×.
        self.load.add_recovery_debt(debt_add);

        // Hard floor — engine clamps to 1500 in-match, but we lift to 30%
        // so nobody finishes the post-match book on empty.
        let condition_floor: i16 = 3000;
        if self.player_attributes.condition < condition_floor {
            self.player_attributes.condition = condition_floor;
        }

        // Sharpness: cameo subs (<15 min) don't rebuild readiness; full
        // 90 = +3.0; friendlies sharpen at 70%.
        if minutes >= 15.0 {
            let mut readiness_boost = minutes / 90.0 * 3.0;
            if is_friendly {
                readiness_boost *= 0.7;
            }
            self.skills.physical.match_readiness =
                (self.skills.physical.match_readiness + readiness_boost).min(20.0);
        }

        // Jadedness: scaled by match_load (which already carries the
        // youth multiplier) and recent congestion. A keeper's 90 now
        // adds ~160; a wingback's 90 in a 3-game week tops ~520; a 14yo
        // wingback's 90 tops ~940 — a single senior match leaves him
        // visibly drained.
        let congestion = self.load.matches_last_14() as f32;
        let congestion_mult = 1.0 + (congestion - 2.0).max(0.0) * 0.20;
        let jad_gain = (match_load * 4.0 * congestion_mult).round() as i32;
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

        let chance = self.compute_injury_risk(crate::club::player::condition::InjuryRiskInputs {
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

/// Position-specific load model used by [`Player::on_match_exertion`].
/// Encapsulates the per-position load factor (minute-equivalent
/// multiplier) and the high-intensity share of that load. Both are pure
/// lookup tables and live as associated functions for namespacing — a
/// future "stadium altitude" or "pitch surface" tweak can grow this
/// struct into a configurable model without changing the call site.
struct PositionLoad;

impl PositionLoad {
    /// Position-specific multiplier applied to match minutes when
    /// computing physical load. Calibrated so a CB at neutral intensity
    /// is the reference (1.0 ≈ minute-equivalent), keepers materially
    /// under, and modern fullbacks/wide-mids over.
    fn match_load_factor(position: crate::PlayerPositionType) -> f32 {
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
    fn high_intensity_share(group: crate::club::PlayerFieldPositionGroup) -> f32 {
        use crate::club::PlayerFieldPositionGroup::*;
        match group {
            Goalkeeper => 0.05,
            Defender => 0.20,
            Midfielder => 0.30,
            Forward => 0.32,
        }
    }
}

