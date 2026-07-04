//! Individual training direction — the coach actually using the
//! personal-plan machinery. Two real-football moves, both previously
//! absent: retraining a young squad player toward a position group the
//! squad is thin in (solving a hole on the training pitch instead of
//! the market), and putting an injury-prone senior through a fitness
//! block. Plans progress monthly and expire naturally; effects touch
//! position familiarity and injury proneness only — never CA — so the
//! development calibration is untouched.

use super::TeamBehaviour;
use crate::club::person::Person;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::context::GlobalContext;
use crate::{
    HappinessEventType, IndividualTrainingPlan, Player, PlayerCollection,
    PlayerFieldPositionGroup, PlayerPositionType, StaffCollection, TrainingFocus,
};
use chrono::{Datelike, NaiveDate};
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
                _ => {
                    // Focus kinds without a wired monthly effect yet
                    // simply run their course.
                    if age_days >= Self::FITNESS_MAX_DAYS {
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
        if staffs.social_head_coach().is_none() {
            return;
        }

        // Progress / expire existing plans.
        for player in players.players.iter_mut() {
            if player.individual_training.is_some() && TrainingDirection::progress(player, today)
            {
                player.individual_training = None;
            }
        }

        // Squad shape: contracted, present players per position group.
        let mut group_counts: HashMap<PlayerFieldPositionGroup, usize> = HashMap::new();
        for p in players.players.iter() {
            if p.is_on_loan() || p.contract.is_none() {
                continue;
            }
            *group_counts.entry(p.position().position_group()).or_default() += 1;
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

        for player in players.players.iter_mut() {
            if player.individual_training.is_some() || player.is_on_loan() {
                continue;
            }
            let age = player.age(today);

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
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositions, PlayerSkills};

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
