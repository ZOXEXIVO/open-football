use super::{AcademyDevelopmentIdentity, AcademyPlayerPhase, AcademyTier, ClubAcademy};
use crate::Staff;
use crate::club::player::development::{MatchFootball, SessionWork, SkillKey};
use crate::context::GlobalContext;
use crate::{Person, PlayerFieldPositionGroup};
use chrono::{Datelike, NaiveDate};

/// Deterministic per-(player, date, salt) roll in `[0.0, 1.0)`: the same
/// player on the same day always rolls the same value, so academy work is
/// reproducible in tests and stable across thread scheduling.
struct AcademyRoll;

impl AcademyRoll {
    fn unit(player_id: u32, date: NaiveDate, salt: u32) -> f32 {
        let h = (player_id as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add((date.num_days_from_ce() as u64).wrapping_mul(0xC6BC_279E_9286_5A2B))
            .wrapping_add((salt as u64).wrapping_mul(0x1657_8F35_4D38_C5A7));
        let frac = ((h >> 11) as u32 as f32) / (u32::MAX as f32);
        frac.clamp(0.0, 0.999)
    }
}

/// Identity bias per skill category. Returned as
/// `(technical, mental, physical)` multipliers on the repetitions of each
/// family. Forward/midfielders in PlayerTrading get an extra technical
/// nudge — the identity is "develop sellable attackers".
pub struct IdentityTrainingMultipliers;

impl IdentityTrainingMultipliers {
    pub fn for_identity(
        identity: AcademyDevelopmentIdentity,
        group: PlayerFieldPositionGroup,
    ) -> (f32, f32, f32) {
        match identity {
            AcademyDevelopmentIdentity::Balanced => (1.00, 1.00, 1.00),
            AcademyDevelopmentIdentity::TechnicalSchool => (1.12, 1.04, 0.95),
            AcademyDevelopmentIdentity::TacticalSchool => (1.02, 1.14, 0.96),
            AcademyDevelopmentIdentity::AthleticDevelopment => (0.96, 1.00, 1.14),
            AcademyDevelopmentIdentity::PlayerTrading => match group {
                PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder => {
                    (1.12, 1.04, 1.04)
                }
                _ => (1.08, 1.04, 1.04),
            },
        }
    }
}

impl ClubAcademy {
    /// Minutes of small-sided games in one academy session.
    const GAME_MINUTES: f32 = 10.0;

    /// Each week the academy reports its sessions to every boy as work:
    /// what his phase emphasises, as much as its tier holds sessions, at the
    /// quality its environment and coaches give them. The games those
    /// sessions contain are banked as football at the academy's own
    /// standard. His own development decides what any of it does.
    pub(super) fn train_academy_players(&mut self, ctx: &GlobalContext<'_>) {
        if !ctx.simulation.is_week_beginning() {
            return;
        }

        let date = ctx.simulation.date.date();
        let tier = AcademyTier::from_level(self.level);
        let academy_env = 0.30 * ctx.club_academy_quality()
            + 0.25 * ctx.club_facilities_youth()
            + 0.20 * ctx.club_youth_coaching_quality()
            + 0.15 * tier.norm()
            + 0.10 * (self.pathway_reputation as f32 / 100.0);
        let environment = (0.70 + academy_env.clamp(0.0, 1.0) * 0.55).clamp(0.70, 1.25);
        let staff = self.coaching_staff_multipliers();
        let youth_bonus = self.youth_coaching_bonus();
        let identity = self.development_identity;
        let standard = self.standard_of_football();

        for player in &mut self.players.players {
            if player.player_attributes.is_injured {
                continue;
            }
            let phase = AcademyPlayerPhase::from_age(player.age(date));
            let sessions = tier.sessions_for_phase(phase.index());
            let group = player.position().position_group();
            let (id_tech, id_mental, id_phys) =
                IdentityTrainingMultipliers::for_identity(identity, group);
            let variance = 0.88 + AcademyRoll::unit(player.id, date, 0xACAD) * 0.24;

            let mut work = PhaseWork {
                phase,
                group,
                technical: staff.technical * id_tech,
                mental: staff.mental * id_mental,
                physical: staff.physical * id_phys,
                goalkeeping: staff.goalkeeping * id_tech,
            }
            .session();
            work.scale_quality(environment * youth_bonus * variance);

            for _ in 0..sessions {
                player.on_training_session(&work, date);
            }
            let games = MatchFootball {
                minutes: Self::GAME_MINUTES * sessions as f32,
                standard,
                slot: player.position(),
                season_rating: 0.0,
                season_games: 0,
            };
            player.bank_football(&games, date);
        }
    }

    /// The standard of the football an academy's boys play among
    /// themselves, on the match standard's scale: a boy of its own level
    /// finds it fitting, a better academy plays a little higher.
    fn standard_of_football(&self) -> f32 {
        0.36 + AcademyTier::from_level(self.level).norm() * 0.12
    }

    /// Per-category staff coaching multipliers (0.75..1.25). When the
    /// academy has no dedicated staff the multipliers fall back to the
    /// base-coaching curve so even a no-staff academy still trains.
    fn coaching_staff_multipliers(&self) -> StaffCategoryMultipliers {
        let tier = AcademyTier::from_level(self.level);
        // Base coaching curve: 1..10 tier maps to 0.495..0.90.
        let base_coaching = 0.45 + tier.norm() * 0.45;
        if self.staff.staffs.is_empty() {
            return StaffCategoryMultipliers {
                technical: base_coaching,
                mental: base_coaching,
                physical: base_coaching,
                goalkeeping: base_coaching,
            };
        }

        let best =
            |f: fn(&Staff) -> u8| -> u8 { self.staff.staffs.iter().map(f).max().unwrap_or(0) };

        let best_technical = best(|s| s.staff_attributes.coaching.technical);
        let best_tactical = best(|s| s.staff_attributes.coaching.tactical);
        let best_fitness = best(|s| s.staff_attributes.coaching.fitness);
        let best_gk_h = best(|s| s.staff_attributes.goalkeeping.handling);
        let best_gk_s = best(|s| s.staff_attributes.goalkeeping.shot_stopping);
        let best_gk_d = best(|s| s.staff_attributes.goalkeeping.distribution);
        let best_gk = best_gk_h.max(best_gk_s).max(best_gk_d);

        // Each staff multiplier sits in 0.75..1.25, blended with the base
        // coaching so strong infrastructure with weak staff doesn't collapse.
        let staff_mult = |score: u8| (0.75 + (score as f32 / 20.0) * 0.50).clamp(0.75, 1.25);
        StaffCategoryMultipliers {
            technical: staff_mult(best_technical).max(base_coaching),
            mental: staff_mult(best_tactical).max(base_coaching),
            physical: staff_mult(best_fitness).max(base_coaching),
            goalkeeping: staff_mult(best_gk).max(base_coaching),
        }
    }

    fn youth_coaching_bonus(&self) -> f32 {
        let best_wwy = self
            .staff
            .staffs
            .iter()
            .map(|s| s.staff_attributes.coaching.working_with_youngsters)
            .max()
            .unwrap_or(0);
        1.00 + (best_wwy as f32 / 20.0) * 0.12 // 1.00..1.12
    }
}

#[derive(Copy, Clone)]
struct StaffCategoryMultipliers {
    technical: f32,
    mental: f32,
    physical: f32,
    goalkeeping: f32,
}

/// What one academy session practises at each phase: ball mastery in the
/// foundation phase, the position's craft and reading of the game from the
/// development phase, physical work added in the professional phase. The
/// family multipliers scale each family's repetitions.
struct PhaseWork {
    phase: AcademyPlayerPhase,
    group: PlayerFieldPositionGroup,
    technical: f32,
    mental: f32,
    physical: f32,
    goalkeeping: f32,
}

/// The five drill families one session at a phase and unit practises.
/// Each list is already phase- and unit-specific; [`PhaseWork`] only scales
/// them by staff quality and identity.
struct SessionPlan {
    common: &'static [(SkillKey, f32)],
    craft: &'static [(SkillKey, f32)],
    mental: &'static [(SkillKey, f32)],
    physical: &'static [(SkillKey, f32)],
    goalkeeping: &'static [(SkillKey, f32)],
}

impl PhaseWork {
    fn session(&self) -> SessionWork {
        let plan = self.session_plan();
        let mut work = SessionWork::default();
        for (reps, scale) in [
            (plan.common, self.technical),
            (plan.craft, self.technical),
            (plan.mental, self.mental),
            (plan.physical, self.physical),
            (plan.goalkeeping, self.goalkeeping),
        ] {
            for (key, r) in reps {
                work.practise(*key, r * scale);
            }
        }
        work
    }

    fn session_plan(&self) -> SessionPlan {
        let keeper = self.group == PlayerFieldPositionGroup::Goalkeeper;
        match (self.phase, keeper) {
            (AcademyPlayerPhase::Foundation, false) => Self::foundation_outfield(),
            (AcademyPlayerPhase::Foundation, true) => Self::foundation_keeper(),
            (AcademyPlayerPhase::Development, false) => self.development_outfield(),
            (AcademyPlayerPhase::Development, true) => Self::development_keeper(),
            (AcademyPlayerPhase::Professional, false) => self.professional_outfield(),
            (AcademyPlayerPhase::Professional, true) => Self::professional_keeper(),
        }
    }

    fn foundation_outfield() -> SessionPlan {
        SessionPlan {
            common: &[
                (SkillKey::FirstTouch, 0.035),
                (SkillKey::Dribbling, 0.030),
                (SkillKey::Technique, 0.025),
                (SkillKey::Passing, 0.020),
                (SkillKey::Crossing, 0.005),
            ],
            craft: &[],
            mental: &[
                (SkillKey::Teamwork, 0.015),
                (SkillKey::Decisions, 0.010),
                (SkillKey::OffTheBall, 0.010),
                (SkillKey::Flair, 0.008),
            ],
            physical: &[
                (SkillKey::Agility, 0.015),
                (SkillKey::Balance, 0.015),
                (SkillKey::Acceleration, 0.010),
            ],
            goalkeeping: &[],
        }
    }

    fn foundation_keeper() -> SessionPlan {
        SessionPlan {
            common: &[],
            craft: &[],
            mental: &[
                (SkillKey::Bravery, 0.015),
                (SkillKey::Concentration, 0.010),
                (SkillKey::Decisions, 0.008),
            ],
            physical: &[(SkillKey::Agility, 0.020), (SkillKey::Balance, 0.015)],
            goalkeeping: &[
                (SkillKey::GkHandling, 0.030),
                (SkillKey::GkReflexes, 0.020),
                (SkillKey::GkKicking, 0.015),
                (SkillKey::GkFirstTouch, 0.015),
                (SkillKey::GkThrowing, 0.010),
            ],
        }
    }

    fn development_outfield(&self) -> SessionPlan {
        SessionPlan {
            common: &[
                (SkillKey::FirstTouch, 0.025),
                (SkillKey::Passing, 0.025),
                (SkillKey::Technique, 0.020),
            ],
            craft: self.craft(
                &[
                    (SkillKey::Tackling, 0.025),
                    (SkillKey::Marking, 0.020),
                    (SkillKey::Heading, 0.015),
                ],
                &[
                    (SkillKey::Passing, 0.015),
                    (SkillKey::Crossing, 0.015),
                    (SkillKey::Dribbling, 0.015),
                ],
                &[
                    (SkillKey::Finishing, 0.025),
                    (SkillKey::Dribbling, 0.020),
                    (SkillKey::LongShots, 0.010),
                ],
            ),
            mental: &[
                (SkillKey::Positioning, 0.025),
                (SkillKey::Concentration, 0.020),
                (SkillKey::Decisions, 0.020),
                (SkillKey::Anticipation, 0.015),
                (SkillKey::Teamwork, 0.015),
                (SkillKey::Vision, 0.010),
            ],
            physical: &[
                (SkillKey::Agility, 0.015),
                (SkillKey::Balance, 0.015),
                (SkillKey::Pace, 0.010),
                (SkillKey::Stamina, 0.010),
            ],
            goalkeeping: &[],
        }
    }

    fn development_keeper() -> SessionPlan {
        SessionPlan {
            common: &[],
            craft: &[],
            mental: &[
                (SkillKey::Positioning, 0.025),
                (SkillKey::Concentration, 0.020),
                (SkillKey::Composure, 0.015),
                (SkillKey::Decisions, 0.015),
            ],
            physical: &[
                (SkillKey::Agility, 0.020),
                (SkillKey::Jumping, 0.015),
                (SkillKey::Acceleration, 0.010),
            ],
            goalkeeping: &[
                (SkillKey::GkHandling, 0.025),
                (SkillKey::GkReflexes, 0.025),
                (SkillKey::GkOneOnOnes, 0.020),
                (SkillKey::GkKicking, 0.020),
                (SkillKey::GkPassing, 0.015),
                (SkillKey::GkCommunication, 0.015),
                (SkillKey::GkAerialReach, 0.010),
                (SkillKey::GkCommandOfArea, 0.010),
            ],
        }
    }

    fn professional_outfield(&self) -> SessionPlan {
        SessionPlan {
            common: &[(SkillKey::Technique, 0.015), (SkillKey::FirstTouch, 0.015)],
            craft: self.craft(
                &[
                    (SkillKey::Tackling, 0.020),
                    (SkillKey::Marking, 0.020),
                    (SkillKey::Heading, 0.020),
                    (SkillKey::Passing, 0.010),
                ],
                &[
                    (SkillKey::Passing, 0.020),
                    (SkillKey::Crossing, 0.015),
                    (SkillKey::Dribbling, 0.015),
                    (SkillKey::LongShots, 0.010),
                ],
                &[
                    (SkillKey::Finishing, 0.025),
                    (SkillKey::Dribbling, 0.015),
                    (SkillKey::Heading, 0.010),
                    (SkillKey::LongShots, 0.015),
                ],
            ),
            mental: &[
                (SkillKey::Composure, 0.020),
                (SkillKey::Concentration, 0.020),
                (SkillKey::Decisions, 0.020),
                (SkillKey::Positioning, 0.020),
                (SkillKey::Anticipation, 0.015),
                (SkillKey::Determination, 0.010),
                (SkillKey::WorkRate, 0.010),
            ],
            physical: &[
                (SkillKey::Strength, 0.025),
                (SkillKey::Stamina, 0.025),
                (SkillKey::Pace, 0.020),
                (SkillKey::Acceleration, 0.015),
                (SkillKey::Jumping, 0.015),
                (SkillKey::NaturalFitness, 0.010),
                (SkillKey::Agility, 0.010),
            ],
            goalkeeping: &[],
        }
    }

    fn professional_keeper() -> SessionPlan {
        SessionPlan {
            common: &[],
            craft: &[],
            mental: &[
                (SkillKey::Positioning, 0.020),
                (SkillKey::Concentration, 0.020),
                (SkillKey::Composure, 0.020),
                (SkillKey::Decisions, 0.015),
                (SkillKey::Anticipation, 0.015),
                (SkillKey::Leadership, 0.010),
            ],
            physical: &[
                (SkillKey::Strength, 0.020),
                (SkillKey::Jumping, 0.020),
                (SkillKey::Agility, 0.015),
                (SkillKey::Acceleration, 0.010),
                (SkillKey::Stamina, 0.010),
            ],
            goalkeeping: &[
                (SkillKey::GkHandling, 0.020),
                (SkillKey::GkReflexes, 0.020),
                (SkillKey::GkOneOnOnes, 0.020),
                (SkillKey::GkAerialReach, 0.020),
                (SkillKey::GkCommandOfArea, 0.020),
                (SkillKey::GkRushingOut, 0.015),
                (SkillKey::GkPunching, 0.015),
                (SkillKey::GkKicking, 0.015),
                (SkillKey::GkPassing, 0.015),
                (SkillKey::GkCommunication, 0.015),
                (SkillKey::GkThrowing, 0.010),
            ],
        }
    }

    /// His unit's own craft.
    fn craft(
        &self,
        defender: &'static [(SkillKey, f32)],
        midfielder: &'static [(SkillKey, f32)],
        forward: &'static [(SkillKey, f32)],
    ) -> &'static [(SkillKey, f32)] {
        match self.group {
            PlayerFieldPositionGroup::Defender => defender,
            PlayerFieldPositionGroup::Midfielder => midfielder,
            PlayerFieldPositionGroup::Forward => forward,
            PlayerFieldPositionGroup::Goalkeeper => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PhaseWork;
    use crate::club::academy::tuning::AcademyTier;
    use crate::club::academy::{AcademyPlayerPhase, ClubAcademy};
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::development::{PositionalSkillCeilings, SessionWork, SkillKey};
    use crate::context::{GlobalContext, SimulationContext};
    use crate::shared::fullname::FullName;
    use crate::utils::DateUtils;
    use crate::{
        PersonAttributes, Player, PlayerAttributes, PlayerFieldPositionGroup, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills,
    };
    use chrono::{Datelike, Duration, NaiveDate, Weekday};

    const GOALKEEPING: [SkillKey; 13] = [
        SkillKey::GkAerialReach,
        SkillKey::GkCommandOfArea,
        SkillKey::GkCommunication,
        SkillKey::GkEccentricity,
        SkillKey::GkFirstTouch,
        SkillKey::GkHandling,
        SkillKey::GkKicking,
        SkillKey::GkOneOnOnes,
        SkillKey::GkPassing,
        SkillKey::GkPunching,
        SkillKey::GkReflexes,
        SkillKey::GkRushingOut,
        SkillKey::GkThrowing,
    ];

    fn goalkeeping(p: &mut Player) -> [&mut f32; 13] {
        let g = &mut p.skills.goalkeeping;
        [
            &mut g.aerial_reach,
            &mut g.command_of_area,
            &mut g.communication,
            &mut g.eccentricity,
            &mut g.first_touch,
            &mut g.handling,
            &mut g.kicking,
            &mut g.one_on_ones,
            &mut g.passing,
            &mut g.punching,
            &mut g.reflexes,
            &mut g.rushing_out,
            &mut g.throwing,
        ]
    }

    fn boy(id: u32, position: PlayerPositionType, born: NaiveDate, pa: u8, ca: u8) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Academy".to_string(), format!("{id}")))
            .birth_date(born)
            .country_id(1)
            .attributes(PersonAttributes {
                professionalism: 14.0,
                ambition: 14.0,
                ..PersonAttributes::default()
            })
            .skills(PlayerSkills::flat_for_ability(ca))
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes {
                potential_ability: pa,
                current_ability: ca,
                condition: 9500,
                ..PlayerAttributes::default()
            })
            .build()
            .unwrap()
    }

    fn season(academy: &mut ClubAcademy, start: NaiveDate) {
        for day in 0..365 {
            let date = start + Duration::days(day);
            let ctx =
                GlobalContext::new(SimulationContext::new(date.and_hms_opt(0, 0, 0).unwrap()));
            academy.train_academy_players(&ctx);
        }
    }

    /// A PA 150 keeper of 18 whose goalkeeping sits just under the
    /// first-team ceilings for his age on `today`.
    fn eighteen_year_old_keeper(today: NaiveDate) -> Player {
        let born = NaiveDate::from_ymd_opt(today.year() - 18, 3, 1).unwrap();
        let mut keeper = boy(7, PlayerPositionType::Goalkeeper, born, 150, 70);
        keeper.draw_blueprint(today);
        let age = DateUtils::age_in_years(keeper.birth_date, today);
        let ceilings = PositionalSkillCeilings::for_player(&keeper, age);
        for (value, key) in goalkeeping(&mut keeper).into_iter().zip(GOALKEEPING) {
            *value = (ceilings.get(key) - 0.3).max(1.0);
        }
        keeper
    }

    /// A season of academy work never takes a keeper past what the first
    /// team's ceilings allow for his age, timing and banked football.
    #[test]
    fn an_academy_keeper_cannot_outgrow_the_first_teams_ceiling() {
        let start = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let mut academy = ClubAcademy::new(20);
        academy.players.add(eighteen_year_old_keeper(start));
        for day in 0..365 {
            let date = start + Duration::days(day);
            let ctx =
                GlobalContext::new(SimulationContext::new(date.and_hms_opt(0, 0, 0).unwrap()));
            academy.train_academy_players(&ctx);
            if date.weekday() != Weekday::Mon {
                continue;
            }
            let keeper = &mut academy.players.players[0];
            let age = DateUtils::age_in_years(keeper.birth_date, date);
            let ceilings = PositionalSkillCeilings::for_player(keeper, age);
            for (value, key) in goalkeeping(keeper).into_iter().zip(GOALKEEPING) {
                let ceiling = ceilings.get(key);
                assert!(
                    *value <= ceiling + 1e-4,
                    "{key:?} reached {value:.3} on {date}, past the first-team ceiling {ceiling:.3}"
                );
            }
        }
    }

    /// A graduate keeps what he built: an attribute above its ceiling is
    /// not cut by a week of academy work.
    #[test]
    fn a_graduate_keeps_what_he_built() {
        let start = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let mut keeper = eighteen_year_old_keeper(start);
        keeper.skills.goalkeeping.handling = 18.0;
        let mut academy = ClubAcademy::new(20);
        academy.players.add(keeper);
        season(&mut academy, start);
        assert!(academy.players.players[0].skills.goalkeeping.handling >= 18.0);
    }

    #[test]
    fn a_boy_banks_football_in_the_academy() {
        let start = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let born = NaiveDate::from_ymd_opt(2011, 3, 1).unwrap();
        let mut player = boy(11, PlayerPositionType::MidfielderCenter, born, 130, 40);
        player.draw_blueprint(start);
        let before = player.development.football.share();
        let mut academy = ClubAcademy::new(10);
        academy.players.add(player);
        season(&mut academy, start);
        assert!(academy.players.players[0].development.football.share() > before);
    }

    #[test]
    fn a_better_academy_more_growth() {
        let start = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let born = NaiveDate::from_ymd_opt(2012, 3, 1).unwrap();
        let grown = |level: u8| {
            let mut player = boy(12, PlayerPositionType::MidfielderCenter, born, 140, 35);
            player.draw_blueprint(start);
            let before: f32 = player.skills.iter_all().map(|(_, v)| v).sum();
            let mut academy = ClubAcademy::new(level);
            academy.players.add(player);
            season(&mut academy, start);
            let after: f32 = academy.players.players[0]
                .skills
                .iter_all()
                .map(|(_, v)| v)
                .sum();
            after - before
        };
        let (best, worst) = (grown(20), grown(1));
        assert!(best > worst, "level 20 grew {best:.2}, level 1 {worst:.2}");
    }

    fn professional_phase_session() -> SessionWork {
        PhaseWork {
            phase: AcademyPlayerPhase::Professional,
            group: PlayerFieldPositionGroup::Midfielder,
            technical: 1.0,
            mental: 1.0,
            physical: 1.0,
            goalkeeping: 1.0,
        }
        .session()
    }

    fn sixteen_year_old(today: NaiveDate) -> Player {
        let born = NaiveDate::from_ymd_opt(today.year() - 16, 3, 1).unwrap();
        let mut player = boy(13, PlayerPositionType::MidfielderCenter, born, 140, 60);
        player.draw_blueprint(today);
        player
    }

    fn attributes(player: &Player) -> Vec<f32> {
        player.skills.iter_all().map(|(_, v)| v).collect()
    }

    #[test]
    fn the_academy_and_a_youth_squad_are_one_rule() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 7).unwrap();
        let work = professional_phase_session();
        let mut youth_boy = sixteen_year_old(today);
        let mut academy = ClubAcademy::new(10);
        academy.players.add(youth_boy.clone());
        academy.players.players[0].on_training_session(&work, today);
        youth_boy.on_training_session(&work, today);
        assert_eq!(
            attributes(&academy.players.players[0]),
            attributes(&youth_boy)
        );
    }

    #[test]
    fn moving_up_changes_nothing_by_itself() {
        let monday = NaiveDate::from_ymd_opt(2026, 9, 7).unwrap();
        let work = professional_phase_session();
        let mut twin = sixteen_year_old(monday);
        let mut academy = ClubAcademy::new(10);
        academy.players.add(twin.clone());
        academy.players.players[0].on_training_session(&work, monday);
        let mut promoted = academy.players.players.remove(0);
        let next = monday + Duration::days(7);
        promoted.on_training_session(&work, next);
        twin.on_training_session(&work, monday);
        twin.on_training_session(&work, next);
        assert_eq!(attributes(&promoted), attributes(&twin));
    }

    #[test]
    fn base_coaching_stays_under_one_at_level_20() {
        let tier_norm = AcademyTier::from_level(20).norm();
        let base_coaching = 0.45 + tier_norm * 0.45;
        assert!(base_coaching <= 0.90 + 1e-6);
        let weak_base = 0.45 + AcademyTier::from_level(1).norm() * 0.45;
        assert!((0.49..=0.55).contains(&weak_base));
    }
}
