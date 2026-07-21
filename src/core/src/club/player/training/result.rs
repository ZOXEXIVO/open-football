use crate::PersonBehaviourState;
use crate::Player;
use crate::club::player::condition::{
    ConditionRecoveryModel, ConditionTargetInputs, InjuryRiskInputs,
};
use crate::club::player::development::{PositionalSkillCeilings, SkillKey};
use crate::club::player::injury::InjuryType;
use crate::league::result::LeagueProcessAccess;
use crate::utils::DateUtils;
use crate::{
    GoalkeepingGains, HappinessEventCause, HappinessEventContext, HappinessEventScope,
    HappinessEventSeverity, HappinessEventType, MentalGains, PhysicalGains, PlayerStatusType,
    TechnicalGains, TrainingEffects, TrainingEventContext, TrainingEventEvidence,
    TrainingEventReason,
};
use chrono::Datelike;
use chrono::NaiveDate;

/// Per-session outcome breakdown built by `PlayerTraining::train`.
/// Carries the sub-scores (effort / focus / physical / coach / psychological
/// / random), the chosen primary reason, and the evidence list so the
/// happiness layer can emit auditable GoodTraining / PoorTraining events
/// instead of guessing why the session swung.
#[derive(Debug, Clone)]
pub struct TrainingOutcomeBreakdown {
    pub raw_score: f32,
    pub baseline_score: f32,
    pub delta_from_baseline: f32,
    pub effort_score: f32,
    pub focus_score: f32,
    pub physical_state_score: f32,
    pub coach_fit_score: f32,
    pub tactical_fit_score: f32,
    pub psychological_score: f32,
    pub randomness_score: f32,
    pub primary_reason: TrainingEventReason,
    pub evidence: Vec<TrainingEventEvidence>,
}

pub struct PlayerTrainingResult {
    pub player_id: u32,
    pub effects: TrainingEffects,
    /// How well the player executed this session (1.0-20.0)
    pub session_performance: f32,
    /// Detailed outcome model used to drive event emission. `None` for
    /// legacy `empty()` results that don't represent a real session.
    pub outcome: Option<TrainingOutcomeBreakdown>,
}

impl PlayerTrainingResult {
    pub fn new(player_id: u32, effects: TrainingEffects) -> Self {
        PlayerTrainingResult {
            player_id,
            effects,
            session_performance: 10.0,
            outcome: None,
        }
    }

    pub fn empty(player_id: u32) -> Self {
        PlayerTrainingResult {
            player_id,
            effects: TrainingEffects {
                physical_gains: PhysicalGains::default(),
                technical_gains: TechnicalGains::default(),
                mental_gains: MentalGains::default(),
                goalkeeping_gains: GoalkeepingGains::default(),
                fatigue_change: 0.0,
                injury_risk: 0.0,
                morale_change: 0.0,
                physical_load_units: 0.0,
                high_intensity_share: 0.0,
                readiness_change: 0.0,
            },
            session_performance: 10.0,
            outcome: None,
        }
    }

    /// Apply the training effects to the player
    /// This is where the actual skill updates happen with mutable references
    pub fn process<D: LeagueProcessAccess>(&self, data: &mut D) {
        let current_date = data.date().date();
        // Get mutable reference to the player
        if let Some(player) = data.player_mut(self.player_id) {
            self.apply_to_player(player, current_date);
        }
    }

    /// Apply the per-player mutations (skills, condition, load,
    /// jadedness, readiness, injury roll, training event). Exposed
    /// separately from `process()` so tests can drive a `Player` in
    /// isolation without standing up a full `SimulatorData`. The
    /// `process()` entry point is still the canonical caller — it
    /// owns the data lookup and date threading.
    pub fn apply_to_player(&self, player: &mut Player, current_date: NaiveDate) {
        {
            // Gate skill gains by ability gap: players near potential barely grow
            let current_ability = player.player_attributes.current_ability as f32;
            let potential_ability = player.player_attributes.potential_ability as f32;
            let growth_factor = if potential_ability <= 0.0 {
                0.05 // No potential set — minimal growth
            } else {
                // PA is the ceiling, same contract as the weekly development
                // tick's CA budget: gains taper continuously to zero as CA
                // approaches PA and stop entirely at/above it. A floor here
                // would let thousands of sessions drip skills past PA and
                // manufacture CA > PA states the development tick then
                // freezes forever.
                let gap_ratio = (potential_ability - current_ability) / potential_ability;
                gap_ratio.clamp(0.0, 1.0)
            };

            // Per-skill positional ceilings — the same PA × position-weight
            // table the weekly development tick enforces, so training can't
            // push a skill past the profile the tick would allow. A value
            // already above its ceiling (import, legacy state) is frozen at
            // its current level, never cut.
            let ceilings = PositionalSkillCeilings::for_player(player);
            let raise = |current: f32, gain: f32, key: SkillKey| -> f32 {
                let ceiling = ceilings.get(key).max(current);
                (current + gain * growth_factor)
                    .min(ceiling)
                    .clamp(1.0, 20.0)
            };

            let g = &self.effects.physical_gains;
            let s = &mut player.skills.physical;
            s.stamina = raise(s.stamina, g.stamina, SkillKey::Stamina);
            s.strength = raise(s.strength, g.strength, SkillKey::Strength);
            s.pace = raise(s.pace, g.pace, SkillKey::Pace);
            s.agility = raise(s.agility, g.agility, SkillKey::Agility);
            s.balance = raise(s.balance, g.balance, SkillKey::Balance);
            s.jumping = raise(s.jumping, g.jumping, SkillKey::Jumping);
            s.natural_fitness = raise(s.natural_fitness, g.natural_fitness, SkillKey::NaturalFitness);

            let g = &self.effects.technical_gains;
            let s = &mut player.skills.technical;
            s.first_touch = raise(s.first_touch, g.first_touch, SkillKey::FirstTouch);
            s.passing = raise(s.passing, g.passing, SkillKey::Passing);
            s.crossing = raise(s.crossing, g.crossing, SkillKey::Crossing);
            s.dribbling = raise(s.dribbling, g.dribbling, SkillKey::Dribbling);
            s.finishing = raise(s.finishing, g.finishing, SkillKey::Finishing);
            s.heading = raise(s.heading, g.heading, SkillKey::Heading);
            s.tackling = raise(s.tackling, g.tackling, SkillKey::Tackling);
            s.technique = raise(s.technique, g.technique, SkillKey::Technique);
            s.marking = raise(s.marking, g.marking, SkillKey::Marking);
            s.long_shots = raise(s.long_shots, g.long_shots, SkillKey::LongShots);
            s.free_kicks = raise(s.free_kicks, g.free_kicks, SkillKey::FreeKicks);
            s.corners = raise(s.corners, g.corners, SkillKey::Corners);
            s.penalty_taking = raise(s.penalty_taking, g.penalty_taking, SkillKey::PenaltyTaking);

            let g = &self.effects.mental_gains;
            let s = &mut player.skills.mental;
            s.concentration = raise(s.concentration, g.concentration, SkillKey::Concentration);
            s.decisions = raise(s.decisions, g.decisions, SkillKey::Decisions);
            s.positioning = raise(s.positioning, g.positioning, SkillKey::Positioning);
            s.teamwork = raise(s.teamwork, g.teamwork, SkillKey::Teamwork);
            s.vision = raise(s.vision, g.vision, SkillKey::Vision);
            s.work_rate = raise(s.work_rate, g.work_rate, SkillKey::WorkRate);
            s.leadership = raise(s.leadership, g.leadership, SkillKey::Leadership);
            s.composure = raise(s.composure, g.composure, SkillKey::Composure);
            s.anticipation = raise(s.anticipation, g.anticipation, SkillKey::Anticipation);
            s.bravery = raise(s.bravery, g.bravery, SkillKey::Bravery);
            s.off_the_ball = raise(s.off_the_ball, g.off_the_ball, SkillKey::OffTheBall);

            let g = &self.effects.goalkeeping_gains;
            let s = &mut player.skills.goalkeeping;
            s.handling = raise(s.handling, g.handling, SkillKey::GkHandling);
            s.reflexes = raise(s.reflexes, g.reflexes, SkillKey::GkReflexes);
            s.one_on_ones = raise(s.one_on_ones, g.one_on_ones, SkillKey::GkOneOnOnes);
            s.aerial_reach = raise(s.aerial_reach, g.aerial_reach, SkillKey::GkAerialReach);
            s.command_of_area =
                raise(s.command_of_area, g.command_of_area, SkillKey::GkCommandOfArea);
            s.communication = raise(s.communication, g.communication, SkillKey::GkCommunication);
            s.rushing_out = raise(s.rushing_out, g.rushing_out, SkillKey::GkRushingOut);
            s.punching = raise(s.punching, g.punching, SkillKey::GkPunching);
            s.kicking = raise(s.kicking, g.kicking, SkillKey::GkKicking);
            s.throwing = raise(s.throwing, g.throwing, SkillKey::GkThrowing);

            // Recalculate current_ability from actual skill values, weighted
            // by the player's primary position. Using the position-weighted
            // path keeps natural development and training in agreement —
            // both feed back through the same ability function so neither
            // path can quietly contradict the other's growth gate. Clamped
            // to PA like the development tick stores it: legacy players
            // hydrated with over-ceiling skill profiles must not have their
            // CA digit re-inflated past PA every session.
            let position = player.position();
            player.player_attributes.current_ability = player
                .skills
                .calculate_ability_for_position(position)
                .min(player.player_attributes.potential_ability);

            // Update rolling training performance (exponential moving average)
            // Alpha = 0.3 for first 5 sessions (fast warmup), then 0.15 (slower, more stable)
            let alpha = if player.training.sessions_completed < 5 {
                0.3
            } else {
                0.15
            };
            player.training.training_performance = player.training.training_performance
                * (1.0 - alpha)
                + self.session_performance * alpha;
            player.training.sessions_completed =
                player.training.sessions_completed.saturating_add(1);

            // ── PRE-SESSION STATE SNAPSHOT ──────────────────────────
            // The session's load / debt / jadedness / fitness decisions
            // must read the state the player walked INTO the session
            // with, not the state after `fatigue_change` has been
            // applied. Without this, a recovery session that lifts
            // condition by +800 would "look" like the player started
            // the day fresh (and stop scaling jadedness vulnerability),
            // and a heavy drill that drops condition by +180 would
            // appear to start the day with the post-session deficit
            // already in place (double-counting the tiredness). Also
            // critical for fitness gating: the freshly-incremented
            // recovery_debt must not block this same session's chronic
            // fitness gain — adaptation is decided by what the body
            // brought to the session, not by the bill it leaves with.
            let pre_condition = player.player_attributes.condition;
            let pre_condition_pct = player.player_attributes.condition_percentage();
            let pre_recovery_debt = player.load.recovery_debt;

            // Apply fatigue changes.
            //
            // POSITIVE fatigue_change (heavy drill): subtracts from
            // condition straight — no individualisation, intensity
            // hurts everyone roughly the same.
            //
            // NEGATIVE fatigue_change (recovery session): the magnitude
            // becomes a *potential* gain, not a guaranteed one. A
            // genuinely depleted player banks most of it; a player
            // already near their target gains barely anything. Elite
            // recovery profiles (high NF / chronic fitness /
            // professionalism, low recovery debt) absorb more of the
            // potential than average bodies. An overloaded player can't
            // reset on a single ice bath.
            //
            // The target is computed via the same `individualized_target`
            // the daily-rest path uses so the training day and the
            // rest day agree on what "fully recovered" means for this
            // player today.
            // Floor at 30% — condition never drops below this in training.
            let new_condition = if self.effects.fatigue_change >= 0.0 {
                // Individualise the acute cost: elite stamina / NF /
                // chronic fitness pay less for the same heavy drill;
                // overloaded / jaded / very young / veteran bodies pay
                // more. Reads PRE-session debt / jadedness / condition
                // so the same drill on a fresh squad and a battered
                // squad doesn't cost the same. The recovery branch is
                // already individualised via `individualized_target`,
                // so the multiplier intentionally lives only here.
                let age = DateUtils::age(player.birth_date, current_date);
                let cost_mult = ConditionRecoveryModel::training_fatigue_cost_mult(
                    player.skills.physical.stamina,
                    player.skills.physical.natural_fitness,
                    player.player_attributes.fitness,
                    pre_recovery_debt,
                    player.player_attributes.jadedness,
                    age,
                );
                (pre_condition as f32 - self.effects.fatigue_change * cost_mult)
                    .clamp(3_000.0, 10_000.0)
            } else {
                let recovery_potential = -self.effects.fatigue_change;
                let age = DateUtils::age(player.birth_date, current_date);
                let target = ConditionRecoveryModel::individualized_target(ConditionTargetInputs {
                    natural_fitness: player.skills.physical.natural_fitness,
                    stamina: player.skills.physical.stamina,
                    chronic_fitness: player.player_attributes.fitness,
                    match_readiness: player.skills.physical.match_readiness,
                    physical_load_7: player.load.physical_load_7,
                    recovery_debt: pre_recovery_debt,
                    jadedness: player.player_attributes.jadedness,
                    age,
                });
                let deficit = (target - pre_condition as f32).max(0.0);
                let nf01 = (player.skills.physical.natural_fitness / 20.0).clamp(0.0, 1.0);
                let chronic_fitness01 =
                    (player.player_attributes.fitness as f32 / 10_000.0).clamp(0.0, 1.0);
                let professionalism01 = (player.attributes.professionalism / 20.0).clamp(0.0, 1.0);
                // The club's recovery-facility / sports-science quality
                // is already folded into `recovery_potential` upstream
                // (PlayerTraining::train scales recovery sessions by
                // `TrainingFacilities::get_recovery_modifier`). To avoid
                // double-counting, use a neutral 0.5 here — the design
                // term lives in the formula so future per-player staff
                // modelling can drop in without touching call sites.
                let staff_fitness01 = 0.5_f32;
                let efficiency = (0.45
                    + nf01 * 0.18
                    + chronic_fitness01 * 0.12
                    + professionalism01 * 0.08
                    + staff_fitness01 * 0.10
                    - (pre_recovery_debt / 2_000.0).clamp(0.0, 1.0) * 0.15)
                    .clamp(0.35, 1.15);
                // ±5% deterministic noise — same per-(player, date)
                // stability guarantee as the rest path. The training-
                // recovery salt keeps this stream independent from the
                // daily-rest stream so the same player isn't pushed in
                // the same noise direction on a day that has both a
                // recovery session and a rest pass.
                let date_ordinal = current_date.num_days_from_ce();
                let noise = ConditionRecoveryModel::deterministic_noise(
                    player.id,
                    date_ordinal,
                    ConditionRecoveryModel::NOISE_TRAINING_RECOVERY,
                    0.05,
                );
                let condition_gain = recovery_potential.min(deficit * 0.75) * efficiency * noise;
                (pre_condition as f32 + condition_gain).clamp(3_000.0, 10_000.0)
            };
            player.player_attributes.condition = new_condition as i16;

            // Workload bookkeeping into PlayerLoad. Heavy sessions add to
            // physical_load_7/30 + recovery_debt; recovery sessions burn
            // off accumulated debt. This is the single signal selection
            // and injury-risk consult — keeping training and matches on
            // the same scale.
            //
            // Condition-aware: training on tired legs adds more recovery
            // debt and a touch more jadedness than the same session on
            // fresh legs — read from PRE-session condition so the
            // multiplier reflects the state the body actually started
            // the drill in. Models the real-world cost of "the squad's
            // gassed but the manager still ran a hard pressing drill".
            if self.effects.physical_load_units > 0.0 {
                let hi = self.effects.physical_load_units * self.effects.high_intensity_share;
                player
                    .load
                    .record_training_load(self.effects.physical_load_units, hi);

                // Per-spec debt formula:
                //   training_debt_add = physical_load_units
                //       * (0.12 + 0.18 * high_intensity_share)
                // Condition multipliers (cumulative bands, not overlapping):
                //   * <60% condition  → ×1.25
                //   * <45% condition  → ×1.50 (replaces ×1.25)
                // A tired squad accumulates debt faster than a fresh
                // one for the same drill, which is the missing
                // feedback loop between "we keep running hard
                // sessions" and "injuries pile up".
                let hi_share = self.effects.high_intensity_share.clamp(0.0, 1.0);
                let mut training_debt_add =
                    self.effects.physical_load_units * (0.12 + 0.18 * hi_share);
                if pre_condition_pct < 45 {
                    training_debt_add *= 1.50;
                } else if pre_condition_pct < 60 {
                    training_debt_add *= 1.25;
                }
                player.load.add_recovery_debt(training_debt_add);

                // Jadedness from training: high-intensity work and
                // sessions on tired legs both push harder. A 90-min
                // pressing drill on a fresh midfielder adds ~50; on
                // a 50%-condition midfielder it adds ~63. The
                // vulnerability multiplier reads PRE-session condition
                // so a recovery + heavy back-to-back doesn't artificially
                // soften the heavy drill. Recovery sessions handle
                // jadedness on the negative branch.
                let hi_mult = 0.75 + hi_share * 1.25;
                let fatigue_vulnerability = if pre_condition_pct < 60 { 1.25 } else { 1.0 };
                let jadedness_gain =
                    (self.effects.physical_load_units * hi_mult * fatigue_vulnerability).round()
                        as i32;
                let new_jad =
                    (player.player_attributes.jadedness as i32 + jadedness_gain).clamp(0, 10_000);
                player.player_attributes.jadedness = new_jad as i16;
            }
            if self.effects.fatigue_change < 0.0 {
                // Blended drain: most of the debt / jadedness clearance
                // is driven by the ACTUAL condition gain banked above,
                // with a small floor of "raw potential". The two terms
                // capture two different bits of real physiology:
                //
                //  * actual_condition_gain → the body did refill, so
                //    soft-tissue and CNS bookkeeping wind down. Sized
                //    so a Recovery worth -800 that lands as a +600 gain
                //    burns ~132 debt and ~30 jadedness points — a real
                //    bounce-back, but never a structural reset.
                //  * recovery_potential → the session itself (ice bath,
                //    massage, mobility) still touches the body even
                //    when the player walked in near their daily target
                //    and gained almost no condition. A small floor so
                //    one Recovery on a near-fresh squad reads as
                //    "tiny but non-zero", not "completely wasted".
                //
                // Net effect: a player at 93% with 900 debt drains far
                // less debt from a Recovery than a player at 55% with
                // the same session. Recovery sessions can no longer
                // erase major overload on their own — the deep tank
                // requires actual condition recovery to clear.
                let recovery_potential = -self.effects.fatigue_change;
                let actual_condition_gain = (new_condition - pre_condition as f32).max(0.0);
                let debt_drain = actual_condition_gain * 0.22 + recovery_potential * 0.08;
                player.load.consume_recovery_debt(debt_drain);
                let jadedness_drain = actual_condition_gain * 0.05 + recovery_potential * 0.025;
                let jad_reduction = jadedness_drain.round() as i32;
                let new_jad = (player.player_attributes.jadedness as i32 - jad_reduction).max(0);
                player.player_attributes.jadedness = new_jad as i16;
            }

            // ── FITNESS (chronic base) ──────────────────────────────
            // Endurance / Strength / Speed / Pressing work builds the
            // long-term aerobic / structural base — but only when the
            // player can actually absorb it. The previous model used
            // three hard cliffs (HI > 0.15, condition_pct ≥ 55,
            // recovery_debt < 500) that caused the absorption to flip
            // between "full adaptation" and "zero adaptation" on a
            // single unit. We replace those with a smooth absorption
            // multiplier so a player on the edge of any one criterion
            // still adapts proportionally to how compromised they are.
            //
            // The PRE-session recovery_debt is the right denominator:
            // a heavy pressing drill should not be allowed to block its
            // OWN adaptation by booking debt before the absorption
            // calculation reads it.
            //
            // Hard floor at pre_condition_pct < 45: even a smooth model
            // refuses to certify "training on fumes" as adaptive load.
            // The session still books load + debt + jadedness; the
            // player just doesn't bank a chronic-fitness gain from it.
            let load_units = self.effects.physical_load_units;
            let hi_share = self.effects.high_intensity_share.clamp(0.0, 1.0);
            if load_units > 0.0 && pre_condition_pct >= 45 {
                // condition_absorption: 0 at 45%, 1 at ≥ 80% — a tired
                // player completes the drill but adapts poorly.
                let condition_absorption =
                    (((pre_condition_pct as f32) - 45.0) / 35.0).clamp(0.0, 1.0);
                // debt_absorption: 1 at debt 0, 0.15 at debt 1200+ —
                // floors at 0.15 so chronic gains stay possible even
                // with elevated debt, just heavily attenuated.
                let debt_absorption = (1.0 - (pre_recovery_debt / 1200.0)).clamp(0.15, 1.0);
                // intensity_absorption: a high-intensity drill is the
                // canonical adaptation stimulus (0.65 + share*0.70 ≈
                // 0.76..1.35); low-intensity recovery work still adds a
                // tiny chronic gain (0.25) — sat in an ice bath builds
                // no aerobic base, but a light tactical jog does some.
                let intensity_absorption = if hi_share >= 0.15 {
                    0.65 + hi_share * 0.70
                } else {
                    0.25
                };
                let fitness_gain = load_units
                    * 0.45
                    * condition_absorption
                    * debt_absorption
                    * intensity_absorption;
                if fitness_gain > 0.0 {
                    let new_fitness =
                        (player.player_attributes.fitness as f32 + fitness_gain).min(10_000.0);
                    player.player_attributes.fitness = new_fitness as i16;
                }
            }

            // Apply injury risk — unified recipe. Translate the
            // session-type base risk into the shared model so the same
            // workload signals (jadedness, condition, ACWR spike,
            // congestion, recovery phase) are read by spontaneous,
            // training, and match paths consistently.
            let intensity_factor =
                ((self.effects.fatigue_change.abs() + self.effects.physical_load_units) / 60.0)
                    .clamp(0.4, 2.0);
            let in_recovery = player.player_attributes.is_in_recovery();
            let chance = player.compute_injury_risk(InjuryRiskInputs {
                base_rate: self.effects.injury_risk.max(0.0),
                intensity: intensity_factor,
                in_recovery,
                medical_multiplier: 1.0,
                now: current_date,
            });
            if rand::random::<f32>() < chance {
                let age = DateUtils::age(player.birth_date, current_date);
                let condition_pct = player.player_attributes.condition_percentage();
                let natural_fitness = player.skills.physical.natural_fitness;
                let injury_proneness = player.player_attributes.injury_proneness;

                let injury = InjuryType::random_training_injury(
                    age,
                    condition_pct,
                    natural_fitness,
                    injury_proneness,
                );
                player.player_attributes.set_injury(injury, age);
                player.statuses.add(current_date, PlayerStatusType::Inj);
            }

            // Update match readiness from the per-session readiness_change
            // — replaces the old blanket "any negative fatigue gives +2"
            // rule. PressingDrills / MatchPreparation now sharpen players
            // properly; passive video / RestDay barely move the needle.
            if self.effects.readiness_change != 0.0 {
                player.skills.physical.match_readiness = (player.skills.physical.match_readiness
                    + self.effects.readiness_change)
                    .clamp(0.0, 20.0);
            }
            // Very intense sessions on already-tired legs blunt readiness
            // (a hard pressing drill on a Friday with the legs gone).
            if self.effects.fatigue_change > 120.0
                && player.player_attributes.condition_percentage() < 60
            {
                player.skills.physical.match_readiness =
                    (player.skills.physical.match_readiness - 0.5).max(0.0);
            }

            // Emit GoodTraining / PoorTraining strictly from the
            // session-outcome model. Sessions whose outcome is neutral —
            // a routine, expected-quality day — produce *no* training
            // event so the player's history doesn't fill up with
            // GoodTraining lines for every TeamShape or RestDay.
            if let Some(outcome) = self.outcome.as_ref() {
                Self::maybe_emit_training_event(
                    player,
                    outcome,
                    self.effects.morale_change,
                    current_date,
                );
            }
        }
    }

    fn maybe_emit_training_event(
        player: &mut Player,
        outcome: &TrainingOutcomeBreakdown,
        morale_change: f32,
        current_date: NaiveDate,
    ) {
        // Hard gates — outcome quality, not session-type morale.
        let raw = outcome.raw_score;
        let delta = outcome.delta_from_baseline;
        let age = DateUtils::age(player.birth_date, current_date);
        let leadership = player.skills.mental.leadership;
        let professionalism = player.attributes.professionalism;
        let condition_pct = player.player_attributes.condition_percentage();
        let in_recovery = player.player_attributes.is_in_recovery();

        let high_effort = outcome.effort_score;
        let physical_state = outcome.physical_state_score;

        let positive_qualifies = (raw >= 14.0 && delta >= 1.5)
            || raw >= 16.0
            || (raw >= 13.5 && delta >= 2.0 && age <= 21)
            || (raw >= 13.5 && age >= 30 && leadership >= 14.0);

        let recovery_or_fatigue_cause = matches!(
            outcome.primary_reason,
            TrainingEventReason::StruggledWithIntensity
                | TrainingEventReason::ReturningFromInjuryNotSharp
        );
        let physically_compromised = condition_pct < 60 || in_recovery;
        let attitude_failure =
            high_effort <= 7.0 && professionalism <= 7.0 && physical_state >= 10.0;
        let distraction_failure = (player.happiness.morale < 35.0
            || player.happiness.recent_events.iter().any(|e| {
                e.days_ago <= 21
                    && matches!(
                        e.event_type,
                        HappinessEventType::TransferRumour
                            | HappinessEventType::AgentStirsInterest
                            | HappinessEventType::TransferSpeculationDistracts
                            | HappinessEventType::WantedByBiggerClub
                            | HappinessEventType::InterestFromBiggerClub
                    )
            }))
            && raw <= 8.0
            && delta <= -2.0;

        let negative_qualifies =
            (raw <= 6.5 && delta <= -1.5) || raw <= 5.5 || attitude_failure || distraction_failure;

        // Block PoorTraining when fatigue is the actual cause but the
        // reason machine somehow produced an attitude label — a tired
        // player who failed an intense session must not be tagged poor
        // attitude.
        let block_poor_for_fatigue =
            !recovery_or_fatigue_cause && physically_compromised && raw > 5.5 && !attitude_failure;

        let (event_type, magnitude) = if positive_qualifies && morale_change > 0.0 {
            (HappinessEventType::GoodTraining, morale_change * 5.0)
        } else if negative_qualifies && !block_poor_for_fatigue && morale_change < 0.0 {
            (HappinessEventType::PoorTraining, morale_change * 5.0)
        } else if recovery_or_fatigue_cause && raw <= 6.5 && morale_change < 0.0 {
            (HappinessEventType::PoorTraining, morale_change * 5.0)
        } else {
            if morale_change.abs() > 0.001 {
                player.happiness.adjust_morale(morale_change * 1.5);
            }
            return;
        };

        let positive = matches!(event_type, HappinessEventType::GoodTraining);
        let mut training_ctx = TrainingEventContext::new(
            outcome.primary_reason,
            outcome.raw_score,
            player.training.training_performance,
        );
        for ev in &outcome.evidence {
            training_ctx = training_ctx.with_evidence(*ev);
        }
        // PoorAttitude must never carry fatigue / recovery / overload
        // evidence — copy would contradict itself.
        if outcome.primary_reason == TrainingEventReason::PoorAttitude {
            training_ctx.evidence.retain(|e| {
                !matches!(
                    e,
                    TrainingEventEvidence::FatigueLimited
                        | TrainingEventEvidence::RecoveryLimited
                        | TrainingEventEvidence::Overloaded
                )
            });
        }

        // Visible-event cooldown. Morale still moves on every session;
        // only the timeline entry is suppressed when an event of the
        // same type with the same primary reason fired recently. This
        // keeps a chronic flashpoint (e.g. a player whose attitude is
        // poor every week) from spamming the player's history.
        let cooldown_days =
            Self::training_event_cooldown_days(event_type.clone(), outcome.primary_reason);
        let suppressed = Self::recent_training_event_with_reason(
            player,
            event_type.clone(),
            outcome.primary_reason,
            cooldown_days,
        );
        if !suppressed {
            let happiness_ctx = HappinessEventContext::new(
                HappinessEventCause::Other,
                HappinessEventSeverity::from_magnitude(magnitude),
                HappinessEventScope::TrainingGround,
            )
            .with_training_context(training_ctx);
            player.happiness.add_event_with_context(
                event_type.clone(),
                magnitude,
                None,
                happiness_ctx,
            );
        }
        player.happiness.adjust_morale(morale_change * 3.0);

        if !positive
            && outcome.primary_reason == TrainingEventReason::PoorAttitude
            && morale_change.abs() > 0.4
        {
            player.behaviour.state = match player.behaviour.state {
                PersonBehaviourState::Good => PersonBehaviourState::Normal,
                PersonBehaviourState::Normal => PersonBehaviourState::Poor,
                other => other,
            };
        }
        if positive && morale_change >= 0.4 {
            player.behaviour.try_increase();
        }
    }

    /// Cooldown window (days) before another visible training event of
    /// the same type+reason can fire. PoorAttitude is the longest so a
    /// chronic low-pro player doesn't rack up weekly entries; the
    /// fatigue / recovery branches sit slightly below the generic
    /// poor/good cap so the player still gets occasional updates.
    fn training_event_cooldown_days(
        event_type: HappinessEventType,
        reason: TrainingEventReason,
    ) -> u16 {
        match (event_type, reason) {
            (HappinessEventType::PoorTraining, TrainingEventReason::PoorAttitude) => 14,
            (HappinessEventType::PoorTraining, TrainingEventReason::StruggledWithIntensity) => 5,
            (
                HappinessEventType::PoorTraining,
                TrainingEventReason::ReturningFromInjuryNotSharp,
            ) => 7,
            (HappinessEventType::PoorTraining, _) => 7,
            (HappinessEventType::GoodTraining, _) => 7,
            _ => 7,
        }
    }

    fn recent_training_event_with_reason(
        player: &Player,
        event_type: HappinessEventType,
        reason: TrainingEventReason,
        days: u16,
    ) -> bool {
        player.happiness.recent_events.iter().any(|e| {
            e.event_type == event_type
                && e.days_ago <= days
                && e.context
                    .as_ref()
                    .and_then(|c| c.training_context.as_ref())
                    .map(|tc| tc.reason == reason)
                    .unwrap_or(false)
        })
    }
}

#[cfg(test)]
mod potential_ceiling_tests {
    //! Training must honour the same PA contract as the weekly
    //! development tick: no skill gains at/above the ceiling, and the
    //! stored CA digit never re-inflated past PA. The old 0.05
    //! growth-factor floor let thousands of sessions drip skills past
    //! PA, manufacturing CA > PA states whose goalkeeping the
    //! development tick then froze forever.

    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::position::{PlayerPosition, PlayerPositions};
    use crate::shared::fullname::FullName;
    use crate::{PersonAttributes, PlayerAttributes, PlayerPositionType, PlayerSkills};

    fn make_player(skills: PlayerSkills, ca: u8, pa: u8) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ca;
        attrs.potential_ability = pa;
        attrs.condition = 9500;
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".to_string(), "Player".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 20,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    fn session_with_gains(player_id: u32) -> PlayerTrainingResult {
        PlayerTrainingResult::new(
            player_id,
            TrainingEffects {
                physical_gains: PhysicalGains {
                    stamina: 0.5,
                    ..Default::default()
                },
                technical_gains: TechnicalGains {
                    passing: 0.5,
                    ..Default::default()
                },
                mental_gains: MentalGains {
                    concentration: 0.5,
                    ..Default::default()
                },
                goalkeeping_gains: GoalkeepingGains::default(),
                fatigue_change: 0.0,
                injury_risk: 0.0,
                morale_change: 0.0,
                physical_load_units: 0.0,
                high_intensity_share: 0.0,
                readiness_change: 0.0,
            },
        )
    }

    fn apply_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()
    }

    #[test]
    fn at_potential_player_gains_nothing_from_training() {
        let mut p = make_player(PlayerSkills::flat_for_ability(140), 140, 140);
        let before = (
            p.skills.physical.stamina,
            p.skills.technical.passing,
            p.skills.mental.concentration,
        );

        session_with_gains(p.id).apply_to_player(&mut p, apply_date());

        assert_eq!(
            before,
            (
                p.skills.physical.stamina,
                p.skills.technical.passing,
                p.skills.mental.concentration,
            ),
            "a player at PA must not gain skills from training"
        );
        assert!(
            p.player_attributes.current_ability <= p.player_attributes.potential_ability,
            "stored CA {} exceeds PA {}",
            p.player_attributes.current_ability,
            p.player_attributes.potential_ability
        );
    }

    #[test]
    fn legacy_over_ceiling_ca_is_clamped_not_reinflated() {
        // A player hydrated with skills whose derived CA sits above the
        // assigned PA (freeze-not-cut import). Training must neither grow
        // his skills further nor re-stamp the over-PA digit.
        let mut p = make_player(PlayerSkills::flat_for_ability(120), 109, 101);
        let stamina_before = p.skills.physical.stamina;

        session_with_gains(p.id).apply_to_player(&mut p, apply_date());

        assert_eq!(
            stamina_before, p.skills.physical.stamina,
            "over-ceiling player must not keep growing through training"
        );
        assert_eq!(
            p.player_attributes.current_ability, 101,
            "stored CA must be clamped to PA, not recomputed past it"
        );
    }

    #[test]
    fn training_respects_positional_skill_ceiling() {
        // A striker's tackling ceiling is PA-derived × 0.35 — far below
        // his flat skill baseline here, so tackling must freeze while a
        // normally-weighted skill still grows.
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = 80;
        attrs.potential_ability = 160;
        attrs.condition = 9500;
        let mut p = PlayerBuilder::new()
            .id(2)
            .full_name(FullName::new("Test".to_string(), "Striker".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::flat_for_ability(80))
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();

        let tackling_before = p.skills.technical.tackling;
        let effects = TrainingEffects {
            physical_gains: PhysicalGains {
                stamina: 0.5,
                ..Default::default()
            },
            technical_gains: TechnicalGains {
                tackling: 0.5,
                ..Default::default()
            },
            mental_gains: MentalGains::default(),
            goalkeeping_gains: GoalkeepingGains::default(),
            fatigue_change: 0.0,
            injury_risk: 0.0,
            morale_change: 0.0,
            physical_load_units: 0.0,
            high_intensity_share: 0.0,
            readiness_change: 0.0,
        };
        let stamina_before = p.skills.physical.stamina;
        PlayerTrainingResult::new(p.id, effects).apply_to_player(&mut p, apply_date());

        assert_eq!(
            tackling_before, p.skills.technical.tackling,
            "striker tackling above its positional ceiling must freeze, not grow"
        );
        assert!(
            p.skills.physical.stamina > stamina_before,
            "a normally-weighted skill below its ceiling must still grow"
        );
    }

    #[test]
    fn below_potential_player_still_gains() {
        let mut p = make_player(PlayerSkills::flat_for_ability(80), 80, 160);
        let before = (
            p.skills.physical.stamina,
            p.skills.technical.passing,
            p.skills.mental.concentration,
        );

        session_with_gains(p.id).apply_to_player(&mut p, apply_date());

        assert!(
            p.skills.physical.stamina > before.0
                && p.skills.technical.passing > before.1
                && p.skills.mental.concentration > before.2,
            "a player far below PA must still absorb session gains"
        );
    }
}
