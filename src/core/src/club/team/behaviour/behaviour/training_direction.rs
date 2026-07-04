//! Individual training direction — the coach actually using the
//! personal-plan machinery. Real-football moves: retraining a young
//! squad player toward a position group the squad is thin in, a
//! fitness block for an injury-prone senior, weak-foot work for a
//! lopsided youngster, targeted set-piece / specialty polishing where
//! a positional skill lags the player's own technical level, a
//! structured reintegration program after injury, and a mentality
//! program for fragile young talents. Plans progress monthly and
//! expire naturally; effects touch position familiarity, injury
//! proneness, foot balance, one lagging specialty skill, or the
//! personality axes — never CA — so the development calibration is
//! untouched.

use super::TeamBehaviour;
use crate::club::person::Person;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::context::GlobalContext;
use crate::{
    HappinessEventType, IndividualTrainingPlan, Player, PlayerCollection, PlayerFieldPositionGroup,
    PlayerPositionType, SkillType, StaffCollection, TrainingFocus,
};
use chrono::{Datelike, NaiveDate};
use std::cmp::Ordering;
use std::collections::HashMap;

/// Pure calculus for the monthly training-direction pass, separated so
/// the candidate rules can be unit-tested without a full squad sim.
pub(super) struct TrainingDirection;

impl TrainingDirection {
    /// Retraining is for the mouldable — past this age a player is what
    /// he is.
    const RETRAIN_MAX_AGE: u8 = 26;
    /// A secondary position must already be half-formed to build on.
    const RETRAIN_MIN_LEVEL: u8 = 10;
    const RETRAIN_MAX_LEVEL: u8 = 14;
    /// Retraining tops out at solid-backup familiarity — the training
    /// pitch makes a usable option, not a natural.
    const RETRAIN_TARGET_LEVEL: u8 = 15;
    /// Plans lapse when they've run their course.
    const RETRAIN_MAX_DAYS: i64 = 240;
    const FITNESS_MAX_DAYS: i64 = 120;
    /// Fitness block eligibility: a senior with a real injury record.
    const FITNESS_MIN_AGE: u8 = 27;
    /// Weak-foot work is for the mouldable, like retraining.
    const WEAK_FOOT_MAX_AGE: u8 = 24;
    /// Only a genuinely lopsided player gets the program (0-100 axis).
    const WEAK_FOOT_ELIGIBLE_MAX: u8 = 55;
    /// The training pitch makes a serviceable second foot, not a
    /// natural one.
    const WEAK_FOOT_TARGET: u8 = 70;
    const WEAK_FOOT_STEP: u8 = 2;
    const WEAK_FOOT_MAX_DAYS: i64 = 240;
    /// Specialty polishing: the target skill must clearly lag the
    /// player's own technical level (0-20 axis) and stay a specialist
    /// improvement, never a general development channel.
    const SPECIFIC_SKILL_MAX_AGE: u8 = 26;
    const SPECIFIC_SKILL_LAG: f32 = 2.5;
    const SPECIFIC_SKILL_ELIGIBLE_MAX: f32 = 12.0;
    const SPECIFIC_SKILL_CAP: f32 = 14.0;
    const SPECIFIC_SKILL_STEP: f32 = 0.5;
    const SPECIFIC_SKILL_MAX_DAYS: i64 = 180;
    /// Reintegration program runs until the medical recovery flag
    /// clears (or a hard cap for the pathological case).
    const RECOVERY_MAX_DAYS: i64 = 90;
    /// Mentality program: young and mentally fragile on at least one
    /// axis. Slow drift, mentorship-sized steps.
    const MENTAL_MAX_AGE: u8 = 23;
    const MENTAL_FRAGILE: f32 = 9.0;
    const MENTAL_STEP: f32 = 0.25;
    const MENTAL_MAX_DAYS: i64 = 120;

    /// The retraining target for one player, given which groups are
    /// thin and whether his own group is crowded: his best half-formed
    /// secondary position belonging to a thin group.
    pub(super) fn retrain_target(
        player: &Player,
        own_group_overfull: bool,
        thin_groups: &[PlayerFieldPositionGroup],
    ) -> Option<PlayerPositionType> {
        if !own_group_overfull {
            return None;
        }
        let primary_group = player.position().position_group();
        player
            .positions
            .positions
            .iter()
            .filter(|p| p.position.position_group() != primary_group)
            .filter(|p| thin_groups.contains(&p.position.position_group()))
            .filter(|p| (Self::RETRAIN_MIN_LEVEL..=Self::RETRAIN_MAX_LEVEL).contains(&p.level))
            .max_by_key(|p| p.level)
            .map(|p| p.position)
    }

    /// The weak-foot work target for one player: the lower foot when
    /// the player is genuinely lopsided and young enough to change.
    pub(super) fn weak_foot_eligible(player: &Player, age: u8) -> bool {
        age <= Self::WEAK_FOOT_MAX_AGE
            && !player.positions.is_goalkeeper()
            && player.foots.left.min(player.foots.right) <= Self::WEAK_FOOT_ELIGIBLE_MAX
            && player.skills.technical.technique >= 10.0
    }

    /// The specialty-skill target for one player: a position-relevant
    /// trainable specialty that clearly lags his own technical level.
    pub(super) fn specific_skill_target(player: &Player, age: u8) -> Option<SkillType> {
        if age > Self::SPECIFIC_SKILL_MAX_AGE {
            return None;
        }
        let candidates: &[SkillType] = match player.position().position_group() {
            PlayerFieldPositionGroup::Goalkeeper => return None,
            PlayerFieldPositionGroup::Defender => {
                &[SkillType::Heading, SkillType::Tackling, SkillType::Crossing]
            }
            PlayerFieldPositionGroup::Midfielder => &[
                SkillType::Tackling,
                SkillType::LongShots,
                SkillType::Crossing,
                SkillType::FreeKicks,
            ],
            PlayerFieldPositionGroup::Forward => &[
                SkillType::Heading,
                SkillType::Dribbling,
                SkillType::LongShots,
                SkillType::Penalties,
            ],
        };
        let reference = player.skills.technical.average();
        candidates
            .iter()
            .map(|s| (s.clone(), Self::skill_value(player, s)))
            .filter(|(_, v)| {
                *v <= Self::SPECIFIC_SKILL_ELIGIBLE_MAX
                    && *v <= reference - Self::SPECIFIC_SKILL_LAG
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
            .map(|(s, _)| s)
    }

    fn skill_value(player: &Player, skill: &SkillType) -> f32 {
        let t = &player.skills.technical;
        match skill {
            SkillType::FreeKicks => t.free_kicks,
            SkillType::Penalties => t.penalty_taking,
            SkillType::LongShots => t.long_shots,
            SkillType::Heading => t.heading,
            SkillType::Tackling => t.tackling,
            SkillType::Crossing => t.crossing,
            SkillType::Dribbling => t.dribbling,
        }
    }

    fn bump_skill(player: &mut Player, skill: &SkillType, step: f32, cap: f32) -> f32 {
        let t = &mut player.skills.technical;
        let value = match skill {
            SkillType::FreeKicks => &mut t.free_kicks,
            SkillType::Penalties => &mut t.penalty_taking,
            SkillType::LongShots => &mut t.long_shots,
            SkillType::Heading => &mut t.heading,
            SkillType::Tackling => &mut t.tackling,
            SkillType::Crossing => &mut t.crossing,
            SkillType::Dribbling => &mut t.dribbling,
        };
        if *value < cap {
            *value = (*value + step).min(cap);
        }
        *value
    }

    /// True when the player is young and mentally fragile on at least
    /// one of the axes the mentality program can move.
    pub(super) fn mental_development_eligible(player: &Player, age: u8) -> bool {
        age <= Self::MENTAL_MAX_AGE
            && (player.attributes.pressure <= Self::MENTAL_FRAGILE
                || player.attributes.temperament <= Self::MENTAL_FRAGILE
                || player.attributes.professionalism <= Self::MENTAL_FRAGILE)
    }

    /// Nudge the weakest fragile personality axis by one program step.
    fn nudge_weakest_mental_axis(player: &mut Player) {
        let attrs = &mut player.attributes;
        let axes: [(&str, f32); 3] = [
            ("pressure", attrs.pressure),
            ("temperament", attrs.temperament),
            ("professionalism", attrs.professionalism),
        ];
        let weakest = axes
            .iter()
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
            .map(|(name, _)| *name)
            .unwrap_or("pressure");
        let target = match weakest {
            "temperament" => &mut attrs.temperament,
            "professionalism" => &mut attrs.professionalism,
            _ => &mut attrs.pressure,
        };
        *target = (*target + Self::MENTAL_STEP).min(20.0);
    }

    /// Advance an existing plan by one month. Returns `true` when the
    /// plan has run its course and should be cleared.
    fn progress(player: &mut Player, today: NaiveDate) -> bool {
        let Some(plan) = player.individual_training.clone() else {
            return false;
        };
        let age_days = plan
            .started
            .map(|s| (today - s).num_days())
            .unwrap_or(i64::MAX);
        let mut done = false;
        for focus in &plan.focus_areas {
            match focus {
                TrainingFocus::PositionRetraining(target) => {
                    if let Some(entry) = player
                        .positions
                        .positions
                        .iter_mut()
                        .find(|p| p.position == *target)
                    {
                        if entry.level < Self::RETRAIN_TARGET_LEVEL {
                            entry.level += 1;
                        }
                        if entry.level >= Self::RETRAIN_TARGET_LEVEL {
                            done = true;
                        }
                    }
                    if age_days >= Self::RETRAIN_MAX_DAYS {
                        done = true;
                    }
                }
                TrainingFocus::FitnessBuilding => {
                    if player.player_attributes.injury_proneness > 10 {
                        player.player_attributes.injury_proneness -= 1;
                    }
                    if age_days >= Self::FITNESS_MAX_DAYS {
                        done = true;
                    }
                }
                TrainingFocus::WeakFootImprovement => {
                    let (left, right) = (player.foots.left, player.foots.right);
                    let weak = left.min(right);
                    if weak < Self::WEAK_FOOT_TARGET {
                        let bumped = (weak + Self::WEAK_FOOT_STEP).min(Self::WEAK_FOOT_TARGET);
                        if left < right {
                            player.foots.left = bumped;
                        } else {
                            player.foots.right = bumped;
                        }
                        if bumped >= Self::WEAK_FOOT_TARGET {
                            done = true;
                        }
                    } else {
                        done = true;
                    }
                    if age_days >= Self::WEAK_FOOT_MAX_DAYS {
                        done = true;
                    }
                }
                TrainingFocus::SpecificSkill(skill) => {
                    let value = Self::bump_skill(
                        player,
                        skill,
                        Self::SPECIFIC_SKILL_STEP,
                        Self::SPECIFIC_SKILL_CAP,
                    );
                    if value >= Self::SPECIFIC_SKILL_CAP
                        || age_days >= Self::SPECIFIC_SKILL_MAX_DAYS
                    {
                        done = true;
                    }
                }
                TrainingFocus::InjuryRecovery => {
                    // The effect is passive — the eligibility evaluator
                    // reads the plan and softens the returning-from-
                    // injury caution while the program runs. It ends
                    // when the medical flag clears.
                    if !player.player_attributes.is_in_recovery()
                        || age_days >= Self::RECOVERY_MAX_DAYS
                    {
                        done = true;
                    }
                }
                TrainingFocus::MentalDevelopment => {
                    Self::nudge_weakest_mental_axis(player);
                    if age_days >= Self::MENTAL_MAX_DAYS {
                        done = true;
                    }
                }
            }
        }
        done
    }
}

impl TeamBehaviour {
    /// Monthly training-direction pass: progress existing personal
    /// plans, then let the coach set at most one retraining plan and
    /// one fitness block per squad per month.
    pub(super) fn process_training_direction(
        players: &mut PlayerCollection,
        staffs: &StaffCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return;
        }
        let Some(head_coach) = staffs.social_head_coach() else {
            return;
        };

        // Progress / expire existing plans.
        for player in players.players.iter_mut() {
            if player.individual_training.is_some() && TrainingDirection::progress(player, today) {
                player.individual_training = None;
            }
        }

        // Squad shape: contracted, present players per position group.
        let mut group_counts: HashMap<PlayerFieldPositionGroup, usize> = HashMap::new();
        for p in players.players.iter() {
            if p.is_on_loan() || p.contract.is_none() {
                continue;
            }
            *group_counts
                .entry(p.position().position_group())
                .or_default() += 1;
        }
        let groups = [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ];
        let thin_groups: Vec<PlayerFieldPositionGroup> = groups
            .iter()
            .copied()
            .filter(|g| group_counts.get(g).copied().unwrap_or(0) < g.ideal_squad_depth())
            .collect();

        let cfg = HappinessConfig::default();
        let mut retrain_assigned = false;
        let mut fitness_assigned = false;
        let mut recovery_assigned = false;
        let mut weak_foot_assigned = false;
        let mut specific_skill_assigned = false;
        let mut mental_assigned = false;
        // Mentality programs are a man-manager's tool — a coach without
        // the people skills doesn't run one.
        let coach_runs_mental_programs = head_coach.staff_attributes.mental.man_management >= 12;

        for player in players.players.iter_mut() {
            if player.individual_training.is_some() || player.is_on_loan() {
                continue;
            }
            let age = player.age(today);

            // Reintegration program: a player in medical recovery gets a
            // structured return — the eligibility evaluator softens the
            // returning-from-injury selection caution while it runs.
            if !recovery_assigned && player.player_attributes.is_in_recovery() {
                player.individual_training = Some(IndividualTrainingPlan {
                    player_id: player.id,
                    focus_areas: vec![TrainingFocus::InjuryRecovery],
                    intensity_modifier: 0.8,
                    special_instructions: Vec::new(),
                    started: Some(today),
                });
                player.happiness.add_event_with_cooldown(
                    HappinessEventType::PersonalTrainingPlanSet,
                    cfg.catalog.personal_training_plan_set,
                    120,
                );
                recovery_assigned = true;
                continue;
            }

            // Retraining: young squad player from a crowded group with a
            // half-formed foothold in a thin one.
            if !retrain_assigned && age <= TrainingDirection::RETRAIN_MAX_AGE {
                let own_group = player.position().position_group();
                let overfull = group_counts.get(&own_group).copied().unwrap_or(0)
                    > own_group.ideal_squad_depth();
                if let Some(target) =
                    TrainingDirection::retrain_target(player, overfull, &thin_groups)
                {
                    player.individual_training = Some(IndividualTrainingPlan {
                        player_id: player.id,
                        focus_areas: vec![TrainingFocus::PositionRetraining(target)],
                        intensity_modifier: 1.0,
                        special_instructions: Vec::new(),
                        started: Some(today),
                    });
                    // A role change reads as investment to the willing
                    // pro and as a slight to the unwilling.
                    let magnitude = if player.attributes.professionalism < 10.0 {
                        -cfg.catalog.personal_training_plan_set
                    } else {
                        cfg.catalog.personal_training_plan_set
                    };
                    player.happiness.add_event_with_cooldown(
                        HappinessEventType::PersonalTrainingPlanSet,
                        magnitude,
                        120,
                    );
                    retrain_assigned = true;
                    continue;
                }
            }

            // Fitness block: an injury-plagued senior gets a program.
            if !fitness_assigned
                && age >= TrainingDirection::FITNESS_MIN_AGE
                && player.player_attributes.injury_count >= 3
                && player.player_attributes.injury_proneness > 10
                && !player.player_attributes.is_injured
            {
                player.individual_training = Some(IndividualTrainingPlan {
                    player_id: player.id,
                    focus_areas: vec![TrainingFocus::FitnessBuilding],
                    intensity_modifier: 1.1,
                    special_instructions: Vec::new(),
                    started: Some(today),
                });
                player.happiness.add_event_with_cooldown(
                    HappinessEventType::PersonalTrainingPlanSet,
                    cfg.catalog.personal_training_plan_set,
                    120,
                );
                fitness_assigned = true;
                continue;
            }

            // Weak-foot work: a lopsided youngster with the technique
            // to learn gets a second foot built up to serviceable.
            if !weak_foot_assigned && TrainingDirection::weak_foot_eligible(player, age) {
                player.individual_training = Some(IndividualTrainingPlan {
                    player_id: player.id,
                    focus_areas: vec![TrainingFocus::WeakFootImprovement],
                    intensity_modifier: 1.0,
                    special_instructions: Vec::new(),
                    started: Some(today),
                });
                player.happiness.add_event_with_cooldown(
                    HappinessEventType::PersonalTrainingPlanSet,
                    cfg.catalog.personal_training_plan_set,
                    120,
                );
                weak_foot_assigned = true;
                continue;
            }

            // Specialty polishing: a position-relevant skill that
            // clearly lags the player's own technical level.
            if !specific_skill_assigned {
                if let Some(skill) = TrainingDirection::specific_skill_target(player, age) {
                    player.individual_training = Some(IndividualTrainingPlan {
                        player_id: player.id,
                        focus_areas: vec![TrainingFocus::SpecificSkill(skill)],
                        intensity_modifier: 1.0,
                        special_instructions: Vec::new(),
                        started: Some(today),
                    });
                    player.happiness.add_event_with_cooldown(
                        HappinessEventType::PersonalTrainingPlanSet,
                        cfg.catalog.personal_training_plan_set,
                        120,
                    );
                    specific_skill_assigned = true;
                    continue;
                }
            }

            // Mentality program: a fragile young talent under a coach
            // with the people skills to run one.
            if !mental_assigned
                && coach_runs_mental_programs
                && TrainingDirection::mental_development_eligible(player, age)
            {
                player.individual_training = Some(IndividualTrainingPlan {
                    player_id: player.id,
                    focus_areas: vec![TrainingFocus::MentalDevelopment],
                    intensity_modifier: 1.0,
                    special_instructions: Vec::new(),
                    started: Some(today),
                });
                player.happiness.add_event_with_cooldown(
                    HappinessEventType::PersonalTrainingPlanSet,
                    cfg.catalog.personal_training_plan_set,
                    120,
                );
                mental_assigned = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositions, PlayerSkills,
    };

    fn player_with_positions(id: u32, positions: Vec<(PlayerPositionType, u8)>) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("T".into(), id.to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2002, 3, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: positions
                    .into_iter()
                    .map(|(position, level)| PlayerPosition { position, level })
                    .collect(),
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    #[test]
    fn crowded_forward_with_midfield_foothold_is_retrained() {
        let p = player_with_positions(
            1,
            vec![
                (PlayerPositionType::Striker, 20),
                (PlayerPositionType::MidfielderCenter, 12),
            ],
        );
        let thin = vec![PlayerFieldPositionGroup::Midfielder];
        assert_eq!(
            TrainingDirection::retrain_target(&p, true, &thin),
            Some(PlayerPositionType::MidfielderCenter)
        );
    }

    #[test]
    fn no_retraining_without_a_crowded_home_group() {
        let p = player_with_positions(
            1,
            vec![
                (PlayerPositionType::Striker, 20),
                (PlayerPositionType::MidfielderCenter, 12),
            ],
        );
        let thin = vec![PlayerFieldPositionGroup::Midfielder];
        assert_eq!(TrainingDirection::retrain_target(&p, false, &thin), None);
    }

    #[test]
    fn raw_or_finished_secondaries_are_not_retraining_material() {
        // Level 6 is too raw, level 18 needs no retraining.
        let p = player_with_positions(
            1,
            vec![
                (PlayerPositionType::Striker, 20),
                (PlayerPositionType::MidfielderCenter, 6),
                (PlayerPositionType::MidfielderLeft, 18),
            ],
        );
        let thin = vec![PlayerFieldPositionGroup::Midfielder];
        assert_eq!(TrainingDirection::retrain_target(&p, true, &thin), None);
    }

    #[test]
    fn retraining_plan_raises_familiarity_monthly_and_completes() {
        let mut p = player_with_positions(
            1,
            vec![
                (PlayerPositionType::Striker, 20),
                (PlayerPositionType::MidfielderCenter, 13),
            ],
        );
        let start = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        p.individual_training = Some(IndividualTrainingPlan {
            player_id: 1,
            focus_areas: vec![TrainingFocus::PositionRetraining(
                PlayerPositionType::MidfielderCenter,
            )],
            intensity_modifier: 1.0,
            special_instructions: Vec::new(),
            started: Some(start),
        });

        // Month 1: 13 → 14, not done yet.
        assert!(!TrainingDirection::progress(&mut p, start));
        // Month 2: 14 → 15 — target familiarity reached, plan done.
        assert!(TrainingDirection::progress(
            &mut p,
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()
        ));
        let level = p
            .positions
            .positions
            .iter()
            .find(|e| e.position == PlayerPositionType::MidfielderCenter)
            .unwrap()
            .level;
        assert_eq!(level, 15, "retraining tops out at solid-backup familiarity");
    }
}
