//! Weekly development tick: the public entry point that wires every
//! component (age curve, position weights, modifiers, maturity, coaching,
//! rolls) into a single per-skill update for one player.
//!
//! The tick reads three independent signals and stacks them under a hard
//! per-week cap so no signal — coach quality, big-club step-up, league
//! reputation, raw playing minutes — can compound into an implausible
//! one-week jump.
//!
//! Signals:
//!   * Raw age-curve band (capped per-category by the maturity model).
//!   * Senior-exposure multiplier from rolling 30-day minutes/load
//!     (replaces the previous appearance-count boost so that *minutes
//!     and physical load* drive growth, not the bare fact of being
//!     selected). A force-selected 14-year-old getting full senior loads
//!     now hits the band-overuse penalty inside this helper.
//!   * Acute-overload modifier from condition / jadedness / 7-day load /
//!     recovery debt. Hits under-18s harder than adults.
//!
//! After all multipliers stack, [`maturity::weekly_growth_cap`] caps the
//! positive change. PA is never raised as a side effect of growth — if
//! recomputed CA exceeds PA the CA is clamped down instead, since PA is
//! the player's biological ceiling and shouldn't drift upward from a
//! manager's selection choices.

use super::age_curve::*;
use super::coaching::CoachingEffect;
use super::maturity::MaturityModel;
use super::modifiers::*;
use super::position_weights::*;
use super::rolls::{RollSource, ThreadRolls};
use super::skills_array::*;

use crate::club::player::player::Player;
use crate::utils::DateUtils;
use chrono::NaiveDate;

impl Player {
    /// Weekly development tick. See module docs for the model.
    ///
    /// Routes through the deterministic roll seam under the hood, using
    /// the thread-local RNG. Tests should call
    /// [`Player::process_development_with`] with a deterministic source.
    pub fn process_development(
        &mut self,
        now: NaiveDate,
        league_reputation: u16,
        coach: &CoachingEffect,
        club_rep_0_to_1: f32,
    ) {
        self.process_development_with(
            now,
            league_reputation,
            coach,
            club_rep_0_to_1,
            &mut ThreadRolls,
        );
    }

    /// Same as [`Player::process_development`] but the per-skill rolls
    /// come from `rolls`. This is the testable seam — pin the rolls to a
    /// known value and the output becomes a pure function of the inputs.
    pub fn process_development_with(
        &mut self,
        now: NaiveDate,
        league_reputation: u16,
        coach: &CoachingEffect,
        club_rep_0_to_1: f32,
        rolls: &mut impl RollSource,
    ) {
        let age = DateUtils::age(self.birth_date, now);
        let pa = self.player_attributes.potential_ability as f32;
        let ca = self.player_attributes.current_ability;

        // Body state gates everything else.
        let fitness = if self.player_attributes.is_injured {
            FitnessState::Injured
        } else if self.player_attributes.is_in_recovery() {
            FitnessState::Recovering
        } else {
            FitnessState::Fit
        };

        // Injured players don't develop. Their skills are frozen until they
        // come back — no growth, no decline. The CA recalculation is also
        // skipped because the underlying skills haven't moved.
        if fitness == FitnessState::Injured {
            return;
        }

        let pos = self.position();
        let pos_group = pos_group_from(pos);
        let dev_weights = position_dev_weights(pos_group);

        // Base ceiling from PA (PA 200 -> ceiling 20.0)
        let base_ceiling = (pa / 200.0 * 20.0).clamp(1.0, 20.0);

        // ── Compute shared multipliers ────────────────────────────────

        let personality = DevelopmentModifiers::personality_multiplier(
            self.attributes.professionalism,
            self.attributes.ambition,
            self.skills.mental.determination,
            self.skills.mental.work_rate,
        );

        // Replaces the old appearance-count multiplier. Reads rolling
        // 30-day minutes and physical load straight off PlayerLoad so a
        // forced selection that doesn't actually translate into minutes
        // doesn't fire, and a forced selection that *does* push a 14yo
        // past 600 senior minutes/month penalises growth instead of
        // boosting it.
        let exposure_mult = MaturityModel::senior_exposure_multiplier(
            age,
            self.load.minutes_last_30,
            self.load.physical_load_30,
            league_reputation,
            ca,
        );

        // Friendly/official ratio still informs growth — competitive
        // games stress the player more than pre-season cameos. Kept on
        // the same scale as before for backward-compatible behaviour at
        // adult ages, but its weight is dwarfed by exposure_mult for
        // youngsters now.
        let official_games = self.statistics.total_games() + self.cup_statistics.total_games();
        let friendly_games = self.friendly_statistics.total_games();
        let official_bonus =
            DevelopmentModifiers::official_match_bonus(official_games, friendly_games);

        // Rating multiplier feeds long-form development scaling, so use
        // the regressed season average rather than the raw weighted form.
        // A youngster with three 8.5s shouldn't have his attributes growing
        // at top-talent speed just because the sample is tiny.
        let pos = self.position().position_group();
        let rating_mult = DevelopmentModifiers::rating_multiplier(
            self.statistics.average_rating_realistic(pos),
            official_games,
        );

        let decline_prot = DevelopmentModifiers::decline_protection(
            self.skills.physical.natural_fitness,
            self.attributes.professionalism,
        );

        let comp_quality = DevelopmentModifiers::competition_quality_multiplier(league_reputation);

        // Raw step-up bonus from the adaptation system, dampened by an
        // age factor: under-15s get nothing (they train with the academy
        // regardless of brand), 16-17s get 20% of the bonus, 18-year-olds
        // 65%, adults the full effect.
        let step_up_raw = self.step_up_development_multiplier(now, club_rep_0_to_1);
        let step_up_age = MaturityModel::step_up_age_factor(age);
        let step_up_mult = 1.0 + (step_up_raw - 1.0) * step_up_age;

        // Workload / fitness / readiness modifiers.
        let condition_pct = self.player_attributes.condition_percentage();
        let jadedness = self.player_attributes.jadedness;
        let workload_growth =
            DevelopmentModifiers::workload_growth_modifier(condition_pct, jadedness);
        let workload_decline =
            DevelopmentModifiers::workload_decline_amplifier(condition_pct, jadedness);
        let readiness_mult =
            DevelopmentModifiers::match_readiness_multiplier(self.skills.physical.match_readiness);

        // Acute overload: 7-day load + condition + jadedness + recovery
        // debt. Independent of the rolling-minutes signal so a player
        // who's just been smashed by three matches in a week sees growth
        // suppressed even if his 30-day total still fits the optimal band.
        let overload_mult = MaturityModel::overload_development_modifier(
            age,
            self.load.physical_load_7,
            condition_pct,
            jadedness,
            self.load.recovery_debt,
        );

        // Recovering from an injury: the body is healing, not adapting.
        // Mental skills (study video, learn the playbook) can still nudge
        // forward at a reduced rate; everything else is frozen.
        let recovering = fitness == FitnessState::Recovering;

        // ── Process each skill ────────────────────────────────────────

        let mut skills = skills_to_array(self);
        // Pre-tick snapshot for the CA-budget pass below.
        let pre_tick = skills;

        for i in 0..SKILL_COUNT {
            if i == SK_MATCH_READINESS {
                continue; // managed by training/match system
            }
            if dev_weights[i] <= 0.0 {
                // Outfield players neither train nor decay GK-specific
                // skills — the attribute simply isn't part of their game.
                continue;
            }

            let cat = skill_category(i);

            if recovering && cat != SkillCategory::Mental {
                continue;
            }

            let peak_offset = individual_peak_offset(i);
            let effective_age = (age as i16 - peak_offset as i16).clamp(14, 45) as u8;

            // Per-skill ceiling: position weight determines how high this skill can go.
            let skill_ceiling = (base_ceiling * dev_weights[i]).clamp(1.0, 20.0);

            // Per-skill gap factor (replaces global PA-CA gap).
            let gap = DevelopmentModifiers::skill_gap_factor(skills[i], skill_ceiling);

            // Base rate from age curve.
            let (min_rate, max_rate) = base_weekly_rate(effective_age, cat);
            let roll = rolls.roll_unit().clamp(0.0, 1.0);
            let base = min_rate + roll * (max_rate - min_rate);

            // Position weight scales growth rate: key skills develop faster.
            let pos_rate_mult = dev_weights[i];

            // Coach effectiveness by category, plus a youth bonus for
            // players under 23 (using Head of Youth Development attribute).
            let coach_mult = coach.for_category(cat);
            let youth_coach_mult = if age < 23 { coach.youth_bonus } else { 1.0 };

            // Biological maturity gate — applies only to growth so that
            // declines (negative base) at very young ages aren't a thing
            // we have to reason about.
            let maturity_mult = MaturityModel::biological_maturity_multiplier(age, cat);

            let change = if base > 0.0 {
                let raw = base
                    * personality
                    * exposure_mult
                    * official_bonus
                    * rating_mult
                    * gap
                    * pos_rate_mult
                    * comp_quality
                    * coach_mult
                    * youth_coach_mult
                    * step_up_mult
                    * workload_growth
                    * readiness_mult
                    * overload_mult
                    * maturity_mult;
                // Soft per-week, per-category cap on positive growth.
                //
                // Implemented as `cap * (1 - exp(-raw / cap))` rather than
                // `raw.min(cap)` so that two stacks both above the cap
                // still differentiate (elite coach > neutral coach,
                // professional > slacker). A hard clip at the cap erases
                // differential growth whenever both stacks happen to land
                // in the saturation zone — which is exactly the zone the
                // cap exists to bound.
                //
                // Saturation shape:
                //   raw =   cap     → 0.63 * cap
                //   raw = 2*cap     → 0.86 * cap
                //   raw = 3*cap     → 0.95 * cap
                //   raw → ∞         → cap
                //
                // The cap is scaled by `pos_rate_mult` so a position-key
                // skill (forward finishing, weight 1.5) saturates higher
                // than an irrelevant one (forward tackling, weight 0.35).
                // This keeps the cap from flattening positional
                // differentiation that the rest of the pipeline produces.
                let cap = (MaturityModel::weekly_growth_cap(age, cat) * pos_rate_mult).max(0.001);
                cap * (1.0 - (-raw / cap).exp())
            } else {
                // Decline: position-irrelevant skills decline slightly faster;
                // key skills are more "maintained" by regular use. Great
                // coaches slow decline a little (load + technique management)
                // — the multiplier drops BELOW 1.0 as coach quality rises,
                // shrinking the negative base. Workload amplifier accelerates
                // decline for chronically tired players.
                let decline_pos_mult = (2.0 - dev_weights[i]).clamp(0.5, 1.5);
                let decline_coach_protection = (1.0 - (coach_mult - 1.0) * 0.5).clamp(0.6, 1.0);
                base * decline_prot * decline_pos_mult * decline_coach_protection * workload_decline
            };

            let prev = skills[i];
            let new_val = prev + change;

            skills[i] = if change > 0.0 {
                // The ceiling gates growth — it never confiscates points the
                // player already owns. Real-world imports legitimately carry
                // skills above the PA×weight ceiling (a target man's heading,
                // a poacher's composure); those freeze rather than get cut.
                new_val.min(skill_ceiling.max(prev)).clamp(1.0, 20.0)
            } else {
                new_val.clamp(1.0, 20.0)
            };
        }

        // ── CA budget: PA bounds the position-weighted average ────────
        //
        // Per-skill ceilings alone let the weighted average drift past PA
        // (key-skill ceilings sit at PA×1.2-1.5). FM semantics: at the
        // ceiling, attributes stop accumulating — the CA digit must stay
        // honest about what the match engine actually reads. If this
        // week's growth would push recomputed CA over PA, scale the
        // positive deltas down until it fits; declines always land.
        let position = self.position();
        let pa_u8 = self.player_attributes.potential_ability;
        let ca_of = |arr: &[f32; SKILL_COUNT]| -> u8 {
            let mut probe = self.skills.clone();
            write_array_into(&mut probe, arr);
            probe.calculate_ability_for_position(position)
        };
        let mut recomputed_ca = ca_of(&skills);
        if recomputed_ca > pa_u8 {
            let post = skills;
            let blend = |t: f32| -> [f32; SKILL_COUNT] {
                let mut out = pre_tick;
                for i in 0..SKILL_COUNT {
                    let d = post[i] - pre_tick[i];
                    out[i] = pre_tick[i] + if d > 0.0 { d * t } else { d };
                }
                out
            };
            let floor = blend(0.0);
            if ca_of(&floor) >= pa_u8 {
                // Already at (or, in legacy CA>PA states, above) the
                // ceiling before any growth — no positive gains this week.
                skills = floor;
            } else {
                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                for _ in 0..6 {
                    let mid = (lo + hi) * 0.5;
                    if ca_of(&blend(mid)) > pa_u8 {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                skills = blend(lo);
            }
            recomputed_ca = ca_of(&skills);
        }

        write_skills_back(self, &skills);

        // PA is the biological ceiling — never raised as a development
        // side effect (a manager picking a kid for the first team must
        // not bump his potential). The budget pass above keeps growth
        // inside PA; the final min() covers u8 rounding and legacy
        // CA > PA states.
        self.player_attributes.current_ability =
            recomputed_ca.min(self.player_attributes.potential_ability);
    }
}
