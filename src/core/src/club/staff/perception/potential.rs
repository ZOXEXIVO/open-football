use chrono::NaiveDate;

use crate::utils::DateUtils;
use crate::{Player, PlayerFieldPositionGroup, Staff};

use super::profile::CoachProfile;
use super::utils::{date_to_week, perception_noise_raw};

/// Staff-formed projection of a player's ceiling. Never reads
/// `player_attributes.potential_ability` — coaches and scouts cannot see
/// the biological PA, so this is a noisy belief built from visible
/// signals (current skill level, age, mentals, training direction)
/// blended with the staff's judging skills, biases, and observation
/// history.
#[derive(Debug, Clone, Copy)]
pub struct PotentialEstimate {
    /// Believed ceiling on the 1..200 ability scale. Always ≥ visible
    /// current ability — a real coach won't tag a player as "lower
    /// ceiling than they already are."
    pub estimated_potential: u8,
    /// 0..1, higher = staff trusts the estimate.
    pub confidence: f32,
    /// Half-width of the "could be wrong by this much" band, on the
    /// 1..200 scale. Wider for poor judges and unfamiliar players.
    pub uncertainty: u8,
    /// `estimated_potential - visible_ability`. Positive = room to grow,
    /// 0 = at perceived ceiling, negative cannot occur (clamped).
    pub upside_gap: i16,
    /// Conservative "would actually bet on" projection. Optimistic
    /// `estimated_potential` reduced by an uncertainty/confidence
    /// penalty plus extra youth conservatism — UI star bands use this
    /// so a wide-band guess on a 16yo doesn't read as "future world
    /// class." Floored at visible ability.
    pub credible_potential: u8,
}

/// Inputs the estimator needs that aren't on the `Player` itself.
#[derive(Debug, Clone, Copy)]
pub struct EstimationContext {
    /// How many times this staff member has watched the player. More
    /// observations = tighter error band. 0 means a cold first read.
    pub observation_count: u8,
    /// True if the player is on the staff's main team. False for
    /// reserves / youth — visibility is lower unless the staff has
    /// strong working_with_youngsters / youth_preference.
    pub is_main_team: bool,
    /// Optional hash salt: rotate to vary noise without changing seed.
    pub salt: u32,
}

impl Default for EstimationContext {
    fn default() -> Self {
        EstimationContext {
            observation_count: 0,
            is_main_team: true,
            salt: 0xC1B5_3E11,
        }
    }
}

pub struct PotentialEstimator;

impl PotentialEstimator {
    /// Estimate using a fully-built `CoachProfile`. Preferred entry for
    /// callers that already need the profile for other perception work
    /// (squad eval, recommendations).
    pub fn estimate_from_profile(
        player: &Player,
        profile: &CoachProfile,
        ctx: &EstimationContext,
        date: NaiveDate,
    ) -> PotentialEstimate {
        Self::estimate_inner(player, profile, ctx, date)
    }

    /// Convenience entry that builds a `CoachProfile` from a `Staff`.
    pub fn estimate_for_staff(
        player: &Player,
        staff: &Staff,
        ctx: &EstimationContext,
        date: NaiveDate,
    ) -> PotentialEstimate {
        let profile = CoachProfile::from_staff(staff);
        Self::estimate_inner(player, &profile, ctx, date)
    }

    /// Visible-ability anchor used by the estimator and exposed so
    /// callers can clamp / display alongside the estimate without
    /// re-deriving it.
    pub fn visible_ability(player: &Player) -> u8 {
        player
            .skills
            .calculate_ability_for_position(player.position())
    }

    /// Staff-free observable ceiling — the "any reasonable observer"
    /// baseline: visible ability plus an age/mentals/training growth
    /// allowance. Deterministic (no judging noise, no staff biases) and
    /// built ONLY from visible signals, so club decision paths with no
    /// staff context (market snapshots, listing decisions, contract
    /// packages) get a potential proxy without ever touching the hidden
    /// biological PA. Prefer scout-assessed values when a dossier
    /// exists; this is the fallback.
    pub fn observable_ceiling(player: &Player, date: NaiveDate) -> u8 {
        let visible_ca = Self::visible_ability(player);
        let age = DateUtils::age(player.birth_date, date);
        let group = player.position().position_group();
        let age_room = Self::age_growth_room(age, group);

        let mentals = &player.skills.mental;
        let attitude_avg = (mentals.determination
            + player.attributes.professionalism
            + mentals.work_rate
            + player.attributes.ambition)
            / 4.0;
        let attitude_factor = ((attitude_avg - 8.0) / 12.0).clamp(0.0, 1.0);
        let ceiling_avg =
            (mentals.composure + mentals.decisions + mentals.anticipation + mentals.concentration)
                / 4.0;
        let ceiling_factor = ((ceiling_avg - 8.0) / 12.0).clamp(0.0, 1.0);
        let trend = Self::training_trend_signal(player);

        let realisation = (attitude_factor * 0.55 + trend * 0.15).clamp(0.0, 1.0);
        let combined = (realisation * 0.55 + ceiling_factor * 0.45).clamp(0.0, 1.0);

        let projected = visible_ca as f32 + age_room * combined;
        (projected.round() as i16).clamp(visible_ca as i16, 200) as u8
    }

    fn estimate_inner(
        player: &Player,
        profile: &CoachProfile,
        ctx: &EstimationContext,
        date: NaiveDate,
    ) -> PotentialEstimate {
        let visible_ca = Self::visible_ability(player);
        let age = DateUtils::age(player.birth_date, date);
        let group = player.position().position_group();

        // ── Age growth window: room above visible CA the coach
        // believes is *biologically* available. A 16yo can still gain
        // 50+ points in principle; a 27yo basically cannot. Late-window
        // values stay non-zero so a generational kid in a mental-led
        // role can still creep upward in the coach's eyes.
        let age_room = Self::age_growth_room(age, group);

        // ── Mental drivers of growth realisation. Determination and
        // professionalism control whether the room actually gets
        // converted; ambition and work_rate add a small extra push.
        let mentals = &player.skills.mental;
        let attitude_avg = (mentals.determination
            + player.attributes.professionalism
            + mentals.work_rate
            + player.attributes.ambition)
            / 4.0;
        let attitude_factor = ((attitude_avg - 8.0) / 12.0).clamp(0.0, 1.0);

        // ── Mental ceiling indicators. Composure / decisions /
        // anticipation / concentration project tactical and on-pitch
        // intelligence growth, especially relevant for late developers.
        let ceiling_avg =
            (mentals.composure + mentals.decisions + mentals.anticipation + mentals.concentration)
                / 4.0;
        let ceiling_factor = ((ceiling_avg - 8.0) / 12.0).clamp(0.0, 1.0);

        // ── Technical projection signal: high first_touch / technique
        // / vision suggests a player whose ceiling is wider than their
        // current CA implies, especially for outfield roles.
        let tech_signal = if group == PlayerFieldPositionGroup::Goalkeeper {
            // GK technical projection comes from goalkeeping handling /
            // reflexes / command, not outfield technique.
            let gk = &player.skills.goalkeeping;
            (gk.handling + gk.reflexes + gk.command_of_area) / 3.0
        } else {
            let t = &player.skills.technical;
            (t.first_touch + t.technique + mentals.vision) / 3.0
        };
        let tech_indicator = ((tech_signal - 11.0) / 9.0).clamp(-0.6, 1.0);

        // ── Training trend: a player improving across the board
        // signals the development plan is working; declining signals
        // staleness and pulls the projection down.
        let training_trend = Self::training_trend_signal(player);

        // ── Realised ceiling (before staff bias / noise).
        // Combined growth factor blends the realisation drivers
        // (attitude/training) with the structural ceiling drivers
        // (mental/technical signals). Each contributes ~half so a
        // pure-attitude grinder doesn't project unrealistically high
        // without the cognitive ceiling to back it up.
        let realisation = (attitude_factor * 0.55 + training_trend * 0.15).clamp(0.0, 1.0);
        let ceiling_quality =
            (ceiling_factor * 0.65 + tech_indicator.max(0.0) * 0.35).clamp(0.0, 1.2);
        let combined_factor = (realisation * 0.55 + ceiling_quality * 0.45).clamp(0.0, 1.1);

        let mut projected = visible_ca as f32 + age_room * combined_factor;

        // ── Physical-bias overrating. Coaches with high
        // physical_bias_youth see height + raw pace as a future ceiling
        // marker — sometimes correctly, often not. Elite youth coaches
        // (working_with_youngsters → high in CoachProfile via
        // physical_bias_youth dampening) are less prone to this.
        let bias_room = if age <= 21 {
            let height = player.player_attributes.height as f32;
            let height_signal = ((height - 178.0) / 12.0).clamp(-0.4, 1.0);
            let pace_signal = ((player.skills.physical.pace - 12.0) / 8.0).clamp(0.0, 1.0);
            let raw_bias = (height_signal * 0.4 + pace_signal * 0.6) * profile.physical_bias_youth;
            // Penalty when the kid is physically loud but technically
            // hollow — high bias staff still get pulled toward this
            // overrating, but not as much as if technique were strong.
            let tech_penalty = if tech_indicator < 0.0 {
                tech_indicator * 4.0
            } else {
                0.0
            };
            (raw_bias * 8.0 + tech_penalty).clamp(-6.0, 14.0)
        } else {
            0.0
        };
        projected += bias_room;

        // ── Staff style nudge. Tactical/creative-leaning lenses see
        // hidden mental ceiling sooner than physically-leaning lenses.
        let lens = &profile.perception_lens;
        let style_nudge = (lens.mental_weight - 0.35) * (ceiling_factor * 8.0);
        projected += style_nudge;

        // ── Visibility & familiarity. Reserve / youth players are
        // harder to read unless the coach prefers youth or has the
        // "working with youngsters" lens (which feeds youth_preference
        // and dampens physical bias in the profile).
        let visibility = if ctx.is_main_team {
            1.0
        } else {
            (0.55 + profile.youth_preference * 0.4).min(1.0)
        };
        let obs_clamped = ctx.observation_count.min(20) as f32;
        let observation_factor = obs_clamped / 20.0;

        // ── Error width. Worst case (rookie scout, first cold read,
        // foreign reserve kid) gets ~30 CA of swing. Veteran chief scout
        // who's watched the player a dozen times collapses to ~3.
        let base_error = 30.0 * (1.0 - profile.potential_accuracy);
        let visibility_inflation = (1.0 - visibility) * 12.0;
        let observation_relief = observation_factor * (base_error * 0.55);
        let youth_relief = if age <= 21 {
            // Working-with-youngsters specialists read youth ceilings
            // much more accurately than their generic judging skill
            // alone implies — proxy via dampened physical bias and the
            // youth_preference channel.
            (1.0 - profile.physical_bias_youth) * 6.0 + profile.youth_preference * 4.0
        } else {
            0.0
        };
        let error_width = (base_error + visibility_inflation - observation_relief - youth_relief)
            .clamp(2.0, 38.0);

        // ── Deterministic noise. Existing perception code uses
        // (coach_seed, player_id, salt) hashing; reuse it so saves stay
        // stable and the same staff/player/date pair always produces
        // the same estimate. Belief re-rolls run on a ~monthly cadence,
        // not weekly — a coach's read of a ceiling drifts slowly, and a
        // weekly re-roll made borderline star ratings flip every Monday
        // on pure noise.
        let period = date_to_week(date) / 4;
        let salt = ctx
            .salt
            .wrapping_add(period.wrapping_mul(11))
            .wrapping_add(ctx.observation_count as u32 * 0x9E37);
        let noise = perception_noise_raw(profile.coach_seed, player.id, salt);
        // Negativity bias skews the *direction* of the random read for
        // pessimistic coaches — they more often land on the lower side
        // of the band. Optimism is already expressed via the youth
        // physical-bias channel, so we don't add a separate
        // youth_preference offset here (that would blanket-promote any
        // young player a youth specialist looks at, regardless of
        // whether the visible signals support it).
        let pessimism_offset = -(profile.negativity_bias - 0.5) * 3.0;
        projected += noise * error_width * 0.5 + pessimism_offset;

        // ── Clamping. Estimate must be at least visible_ca (you don't
        // tell a 130 CA player "your ceiling is 110"), and stays in the
        // 1..200 ability scale. Top-end is also softly capped at
        // visible_ca + age_room + bias_room so a noise spike on a
        // 25yo won't suddenly grant a +50 ceiling.
        let upper_cap =
            (visible_ca as f32 + age_room.max(8.0) + bias_room.max(0.0) + 6.0).min(200.0);
        let estimated = projected.clamp(visible_ca as f32, upper_cap.max(visible_ca as f32));
        let estimated_potential = estimated.round().clamp(1.0, 200.0) as u8;

        // ── Confidence: function of staff potential_accuracy,
        // observations, visibility. Capped at 0.95 so an elite scout
        // never feels omniscient.
        let confidence = (0.20
            + profile.potential_accuracy * 0.55
            + observation_factor * 0.20
            + (visibility - 0.55) * 0.10
            - (1.0 - profile.judging_accuracy) * 0.05)
            .clamp(0.05, 0.95);

        let uncertainty = (error_width / 2.0).round().clamp(1.0, 60.0) as u8;
        let upside_gap = estimated_potential as i16 - visible_ca as i16;

        // Credible projection: pull the optimistic ceiling toward the
        // visible floor by an uncertainty * (1.15 - confidence) penalty.
        // The 1.15 ceiling/0.25 floor on the multiplier keeps even an
        // elite scout's bet honest (some penalty always applies) while
        // capping a clueless one's penalty so credible never collapses
        // to zero. Extra youth conservatism for under-18s with broad
        // age-room — without it, every 16yo with mid-range mentals
        // saturates the 5★ potential band against any baseline.
        let confidence_factor = (1.15 - confidence).clamp(0.25, 1.15);
        let mut credible = estimated_potential as f32 - uncertainty as f32 * confidence_factor;
        if age < 18 {
            let years_below = (18 - age) as f32;
            credible -= years_below * 1.5 * (1.0 - confidence);
        }
        let credible_potential = credible.clamp(visible_ca as f32, 200.0).round() as u8;

        PotentialEstimate {
            estimated_potential,
            confidence,
            uncertainty,
            upside_gap,
            credible_potential,
        }
    }

    /// Maximum ability points above visible CA the coach believes the
    /// player could biologically reach. Single source of truth for the
    /// age curve — don't replicate it in callers.
    fn age_growth_room(age: u8, group: PlayerFieldPositionGroup) -> f32 {
        let base = match age {
            0..=15 => 70.0,
            16 => 60.0,
            17 => 52.0,
            18 => 44.0,
            19 => 35.0,
            20 => 27.0,
            21 => 20.0,
            22 => 14.0,
            23 => 9.0,
            24 => 6.0,
            25 => 3.5,
            26 => 2.0,
            27 => 1.0,
            _ => 0.5,
        };
        // Goalkeepers and centre-backs peak later than forwards/wide
        // attackers — keep a mild positive shift on the late-window
        // points for those positions so a 28yo CB still has a sliver of
        // believable upside in the coach's eyes.
        let late_dev_bonus = if age >= 24 {
            match group {
                PlayerFieldPositionGroup::Goalkeeper => 3.5,
                PlayerFieldPositionGroup::Defender => 2.0,
                PlayerFieldPositionGroup::Midfielder => 1.0,
                PlayerFieldPositionGroup::Forward => 0.0,
            }
        } else {
            0.0
        };
        base + late_dev_bonus
    }

    /// Net training direction signal in [-0.5, 0.5]. Positive when
    /// recent training has been productive across all three skill
    /// blocks; negative when stagnation/decline shows up in the
    /// trailing window. Falls back to neutral 0.0 with no records.
    fn training_trend_signal(player: &Player) -> f32 {
        let records = player.training_history.records();
        if records.is_empty() {
            return 0.0;
        }
        let oldest = &records[0].skills;
        let current = &player.skills;
        let tech = current.technical.average() - oldest.technical.average();
        let mental = current.mental.average() - oldest.mental.average();
        let physical = current.physical.average() - oldest.physical.average();
        let raw = (tech + mental + physical) / 3.0;
        (raw / 1.5).clamp(-0.5, 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::PersonAttributes;
    use crate::club::StaffStub;
    use crate::club::player::generators::PlayerGenerator;
    use crate::{PeopleNameGeneratorData, PlayerPositionType};
    use chrono::{Datelike, NaiveDate};

    fn names() -> PeopleNameGeneratorData {
        PeopleNameGeneratorData {
            first_names: vec!["Test".to_string()],
            last_names: vec!["Player".to_string()],
            nicknames: Vec::new(),
        }
    }

    fn player_aged(age: u8, level: u8) -> Player {
        let today = NaiveDate::from_ymd_opt(2026, 5, 8).unwrap();
        let mut p = PlayerGenerator::generate(
            1,
            today,
            PlayerPositionType::MidfielderCenter,
            level,
            &names(),
        );
        let target_year = today.year() - age as i32;
        p.birth_date = NaiveDate::from_ymd_opt(target_year, 1, 1).unwrap();
        p
    }

    /// Normalize every input the estimator actually reads — skills,
    /// person attributes, and the body fields used in physical bias.
    /// Without this, `PlayerGenerator` randomizes height/weight/etc.
    /// so two "identical visible skill" players still differ in the
    /// physical-bias channel.
    fn flat_skills(p: &mut Player, value: f32) {
        let t = &mut p.skills.technical;
        t.corners = value;
        t.crossing = value;
        t.dribbling = value;
        t.finishing = value;
        t.first_touch = value;
        t.free_kicks = value;
        t.heading = value;
        t.long_shots = value;
        t.long_throws = value;
        t.marking = value;
        t.passing = value;
        t.penalty_taking = value;
        t.tackling = value;
        t.technique = value;
        let m = &mut p.skills.mental;
        m.aggression = value;
        m.anticipation = value;
        m.bravery = value;
        m.composure = value;
        m.concentration = value;
        m.decisions = value;
        m.determination = value;
        m.flair = value;
        m.leadership = value;
        m.off_the_ball = value;
        m.positioning = value;
        m.teamwork = value;
        m.vision = value;
        m.work_rate = value;
        let ph = &mut p.skills.physical;
        ph.acceleration = value;
        ph.agility = value;
        ph.balance = value;
        ph.jumping = value;
        ph.natural_fitness = value;
        ph.pace = value;
        ph.stamina = value;
        ph.strength = value;
        p.attributes = PersonAttributes {
            adaptability: value,
            ambition: value,
            controversy: value,
            loyalty: value,
            pressure: value,
            professionalism: value,
            sportsmanship: value,
            temperament: value,
            consistency: value,
            important_matches: value,
            dirtiness: value,
        };
        // Body fields the estimator reads. Pin both to the same
        // unremarkable mid-range so two "identical visible" players
        // really are identical — generator randomization on height /
        // weight would otherwise leak into the physical-bias channel.
        p.player_attributes.height = 180;
        p.player_attributes.weight = 75;
    }

    fn staff_with(judging_pot: u8, judging_ab: u8, working_youth: u8) -> Staff {
        let mut s = StaffStub::default();
        s.id = 7;
        s.staff_attributes.knowledge.judging_player_potential = judging_pot;
        s.staff_attributes.knowledge.judging_player_ability = judging_ab;
        s.staff_attributes.coaching.working_with_youngsters = working_youth;
        // Reasonable mental/coaching defaults for a credible profile.
        s.staff_attributes.mental.adaptability = 12;
        s.staff_attributes.mental.determination = 12;
        s.staff_attributes.mental.discipline = 12;
        s.staff_attributes.mental.man_management = 12;
        s.staff_attributes.coaching.attacking = 12;
        s.staff_attributes.coaching.defending = 12;
        s.staff_attributes.coaching.fitness = 12;
        s.staff_attributes.coaching.mental = 12;
        s.staff_attributes.coaching.tactical = 12;
        s.staff_attributes.coaching.technical = 12;
        s
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 5, 8).unwrap()
    }

    /// Two players with identical visible skills but different hidden
    /// PA must read identically to the same staff/seed — the entire
    /// point of the helper is that biological PA never leaks.
    #[test]
    fn identical_visible_skills_yield_identical_estimates_regardless_of_pa() {
        let staff = staff_with(15, 15, 12);
        let date = today();
        let ctx = EstimationContext::default();

        let mut p1 = player_aged(19, 100);
        let mut p2 = player_aged(19, 100);
        p1.id = 100;
        p2.id = 100; // same id ⇒ same noise hash
        flat_skills(&mut p1, 12.0);
        flat_skills(&mut p2, 12.0);
        // Sabotage the hidden PA — must NOT change the estimate.
        p1.player_attributes.potential_ability = 200;
        p2.player_attributes.potential_ability = 80;

        let e1 = PotentialEstimator::estimate_for_staff(&p1, &staff, &ctx, date);
        let e2 = PotentialEstimator::estimate_for_staff(&p2, &staff, &ctx, date);
        assert_eq!(
            e1.estimated_potential, e2.estimated_potential,
            "estimate must depend on visible skills, not hidden PA",
        );
        assert!((e1.confidence - e2.confidence).abs() < 1e-6);
    }

    /// Higher judging_player_potential ⇒ tighter uncertainty band and
    /// higher confidence. Two staff watching the same player.
    #[test]
    fn higher_judging_reduces_uncertainty_and_lifts_confidence() {
        let date = today();
        let ctx = EstimationContext::default();
        let mut p = player_aged(20, 110);
        flat_skills(&mut p, 13.0);

        let weak = staff_with(3, 3, 3);
        let elite = staff_with(20, 18, 14);

        let e_weak = PotentialEstimator::estimate_for_staff(&p, &weak, &ctx, date);
        let e_elite = PotentialEstimator::estimate_for_staff(&p, &elite, &ctx, date);

        assert!(
            e_elite.uncertainty < e_weak.uncertainty,
            "elite uncertainty {} should be < weak {}",
            e_elite.uncertainty,
            e_weak.uncertainty,
        );
        assert!(
            e_elite.confidence > e_weak.confidence,
            "elite confidence {} should exceed weak {}",
            e_elite.confidence,
            e_weak.confidence,
        );
    }

    /// More observations of the same player shrink the error and raise
    /// the confidence. Both effects compound — a chief scout who's
    /// watched a target weekly should be more sure than the same
    /// scout's first cold read.
    #[test]
    fn observations_tighten_estimate_and_raise_confidence() {
        let staff = staff_with(12, 12, 12);
        let date = today();
        let mut p = player_aged(18, 90);
        flat_skills(&mut p, 11.0);

        let cold = EstimationContext {
            observation_count: 0,
            ..EstimationContext::default()
        };
        let warm = EstimationContext {
            observation_count: 18,
            ..EstimationContext::default()
        };
        let cold_e = PotentialEstimator::estimate_for_staff(&p, &staff, &cold, date);
        let warm_e = PotentialEstimator::estimate_for_staff(&p, &staff, &warm, date);

        assert!(warm_e.confidence > cold_e.confidence);
        assert!(warm_e.uncertainty < cold_e.uncertainty);
    }

    /// Young player with strong mentals projects higher than an
    /// otherwise-identical player whose mentals are rock-bottom. Same
    /// staff, same age, same visible CA shell — only the mental drivers
    /// differ.
    #[test]
    fn high_mental_youth_projects_higher_than_low_mental_youth() {
        let staff = staff_with(15, 15, 14);
        let date = today();
        let ctx = EstimationContext::default();

        let mut high = player_aged(17, 70);
        flat_skills(&mut high, 11.0);
        // Boost the realisation drivers and ceiling indicators.
        high.skills.mental.determination = 18.0;
        high.skills.mental.work_rate = 17.0;
        high.skills.mental.composure = 16.0;
        high.skills.mental.decisions = 17.0;
        high.skills.mental.anticipation = 17.0;
        high.skills.mental.concentration = 16.0;
        high.attributes.professionalism = 18.0;
        high.attributes.ambition = 17.0;

        let mut low = player_aged(17, 70);
        flat_skills(&mut low, 11.0);
        low.skills.mental.determination = 4.0;
        low.skills.mental.work_rate = 4.0;
        low.skills.mental.composure = 5.0;
        low.skills.mental.decisions = 5.0;
        low.skills.mental.anticipation = 5.0;
        low.skills.mental.concentration = 5.0;
        low.attributes.professionalism = 4.0;
        low.attributes.ambition = 4.0;

        let high_e = PotentialEstimator::estimate_for_staff(&high, &staff, &ctx, date);
        let low_e = PotentialEstimator::estimate_for_staff(&low, &staff, &ctx, date);
        assert!(
            high_e.estimated_potential > low_e.estimated_potential,
            "high-mental youth ({}) must project higher than low-mental ({})",
            high_e.estimated_potential,
            low_e.estimated_potential,
        );
    }

    /// Estimate must never fall below the visible current ability —
    /// staff don't tell you the ceiling is below where the player
    /// already plays.
    #[test]
    fn estimate_never_falls_below_visible_ability() {
        let date = today();
        let ctx = EstimationContext::default();

        // Sweep age + judging skill space — every combination must
        // satisfy the floor invariant, especially when noise is at its
        // worst (low judging_player_potential).
        for age in [16u8, 19, 22, 26, 30, 34] {
            for judging in [1u8, 5, 10, 15, 20] {
                let staff = staff_with(judging, judging, judging.min(15));
                let mut p = player_aged(age, 130);
                flat_skills(&mut p, 13.0);
                let e = PotentialEstimator::estimate_for_staff(&p, &staff, &ctx, date);
                let visible = PotentialEstimator::visible_ability(&p);
                assert!(
                    e.estimated_potential >= visible,
                    "estimate {} below visible {} (age {}, judging {})",
                    e.estimated_potential,
                    visible,
                    age,
                    judging,
                );
                assert!(e.estimated_potential >= 1 && e.estimated_potential <= 200);
            }
        }
    }

    /// Same staff, same player, same date ⇒ exact same estimate.
    /// Saves rely on this — a scout report on Tuesday must match the
    /// scout report on the same Tuesday after a load-save round-trip.
    #[test]
    fn estimate_is_deterministic_for_fixed_inputs() {
        let staff = staff_with(10, 10, 10);
        let date = today();
        let ctx = EstimationContext::default();
        let mut p = player_aged(20, 120);
        flat_skills(&mut p, 12.5);

        let a = PotentialEstimator::estimate_for_staff(&p, &staff, &ctx, date);
        let b = PotentialEstimator::estimate_for_staff(&p, &staff, &ctx, date);
        assert_eq!(a.estimated_potential, b.estimated_potential);
        assert_eq!(a.confidence.to_bits(), b.confidence.to_bits());
        assert_eq!(a.uncertainty, b.uncertainty);
    }

    /// Physically dominant but technically poor youth should be
    /// over-rated by physically-biased weak coaches more than they are
    /// by elite youth coaches whose `working_with_youngsters` damps
    /// the bias.
    #[test]
    fn physical_youth_overrating_smaller_for_youth_coach() {
        let date = today();
        let ctx = EstimationContext::default();

        let mut p = player_aged(18, 85);
        flat_skills(&mut p, 9.0);
        // Physically loud, technically hollow.
        p.skills.physical.pace = 19.0;
        p.skills.physical.acceleration = 18.0;
        p.skills.physical.strength = 17.0;
        p.player_attributes.height = 192;
        p.skills.technical.first_touch = 6.0;
        p.skills.technical.technique = 6.0;
        p.skills.mental.vision = 6.0;

        // Authoritarian-leaning, low working_with_youngsters ⇒ high physical_bias_youth.
        let physical_coach = staff_with(8, 8, 2);
        // Strong working_with_youngsters ⇒ damped bias.
        let youth_coach = staff_with(8, 8, 18);

        let a = PotentialEstimator::estimate_for_staff(&p, &physical_coach, &ctx, date);
        let b = PotentialEstimator::estimate_for_staff(&p, &youth_coach, &ctx, date);
        assert!(
            a.estimated_potential >= b.estimated_potential,
            "physically-biased coach ({}) should not under-rate vs youth specialist ({})",
            a.estimated_potential,
            b.estimated_potential,
        );
    }

    /// Confidence is bounded — never a perfect 1.0 and never below the
    /// 0.05 floor. Avoids the "elite scout knows everything" antipattern.
    #[test]
    fn confidence_stays_inside_reasonable_band() {
        let date = today();
        let ctx = EstimationContext::default();
        let mut p = player_aged(20, 100);
        flat_skills(&mut p, 12.0);

        let weak = staff_with(1, 1, 1);
        let elite = staff_with(20, 20, 20);
        let we = PotentialEstimator::estimate_for_staff(&p, &weak, &ctx, date);
        let ee = PotentialEstimator::estimate_for_staff(&p, &elite, &ctx, date);
        assert!(we.confidence >= 0.05);
        assert!(ee.confidence <= 0.95);
    }

    /// Credible projection floors at visible ability — the public
    /// contract behind "we don't tell you the ceiling is below where
    /// you already play." Replaces the old star-floor clamp.
    #[test]
    fn credible_potential_never_below_visible_ability() {
        let date = today();
        let ctx = EstimationContext::default();

        for age in [16u8, 19, 22, 26, 30, 34] {
            for judging in [1u8, 5, 10, 15, 20] {
                let staff = staff_with(judging, judging, judging.min(15));
                let mut p = player_aged(age, 130);
                flat_skills(&mut p, 13.0);
                let est = PotentialEstimator::estimate_for_staff(&p, &staff, &ctx, date);
                let visible = PotentialEstimator::visible_ability(&p);
                assert!(
                    est.credible_potential >= visible,
                    "credible {} below visible {} (age {}, judging {})",
                    est.credible_potential,
                    visible,
                    age,
                    judging,
                );
            }
        }
    }

    /// Hidden PA must not leak via the credible projection either —
    /// mirrors the existing `estimated_potential` invariant.
    #[test]
    fn credible_potential_independent_of_hidden_pa() {
        let staff = staff_with(15, 15, 12);
        let date = today();
        let ctx = EstimationContext::default();

        let mut p1 = player_aged(19, 100);
        let mut p2 = player_aged(19, 100);
        p1.id = 300;
        p2.id = 300;
        flat_skills(&mut p1, 12.0);
        flat_skills(&mut p2, 12.0);
        p1.player_attributes.potential_ability = 200;
        p2.player_attributes.potential_ability = 60;

        let e1 = PotentialEstimator::estimate_for_staff(&p1, &staff, &ctx, date);
        let e2 = PotentialEstimator::estimate_for_staff(&p2, &staff, &ctx, date);
        assert_eq!(
            e1.credible_potential, e2.credible_potential,
            "credible projection must depend only on visible signals",
        );
    }
}
