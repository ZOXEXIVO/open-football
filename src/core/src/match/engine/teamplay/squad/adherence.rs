//! **How far each man follows the plan** — one reading per player, from
//! what he understands of the game, how he works for others, the role and
//! the shape he knows, and whether he believes in the coach. Read onto each
//! man with the skill aggregates, and read by the shape pull, the press and
//! the jobs the plans hand out. It never touches a skill.

use crate::RoleFamiliarity;
use crate::r#match::MatchPlayer;

/// A player's reading, centred so the level-14 population averages 1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adherence {
    /// Holding the plan's place and taking up its jobs.
    pub place: f32,
    /// Joining the press and staying in it.
    pub press: f32,
}

impl Adherence {
    pub const NEUTRAL: Adherence = Adherence {
        place: 1.0,
        press: 1.0,
    };

    const UNDERSTANDING: f32 = 0.30;
    const TEAMWORK: f32 = 0.20;
    const ROLE: f32 = 0.20;
    const SIDE: f32 = 0.15;
    const BUY_IN: f32 = 0.15;
    /// The press is the place reading plus the legs and the will to keep
    /// at it.
    const PRESS_PLACE: f32 = 0.60;
    const WORK_RATE: f32 = 0.25;
    const CONDITION: f32 = 0.15;
    /// The raw readings' mean over the level-14 harness population at
    /// kickoff: fresh, natural in their roles, a settled side, neutral
    /// buy-in.
    const PLACE_REFERENCE: f32 = 0.759;
    const PRESS_REFERENCE: f32 = 0.747;
    /// The longest a newly assigned job takes to reach a man, in ticks,
    /// and how far below the population his place reading must fall for
    /// it to take that long.
    pub const JOB_LATENCY: u64 = 50;
    const JOB_LATENCY_SPAN: f32 = 0.3;

    /// `player`'s reading in a side whose familiarity with its formation is
    /// `side_familiarity`.
    pub fn of(player: &MatchPlayer, side_familiarity: f32) -> Self {
        let (place, press) = Self::raw(player, side_familiarity);
        Adherence {
            place: place / Self::PLACE_REFERENCE,
            press: press / Self::PRESS_REFERENCE,
        }
    }

    /// Ticks a newly assigned run, cover or tracking job takes to reach
    /// him: none at the population's reading or above, growing as his
    /// place reading falls, up to [`JOB_LATENCY`](Self::JOB_LATENCY).
    pub fn job_latency(&self) -> u64 {
        (((1.0 - self.place) / Self::JOB_LATENCY_SPAN).clamp(0.0, 1.0) * Self::JOB_LATENCY as f32)
            as u64
    }

    /// The uncentred readings, for re-deriving the references.
    pub fn raw(player: &MatchPlayer, side_familiarity: f32) -> (f32, f32) {
        let m = &player.skills.mental;
        let understanding = (m.decisions + m.positioning + m.anticipation + m.concentration) / 80.0;
        let role = RoleFamiliarity::credit_at(
            player
                .role_levels
                .level(player.tactical_position.current_position),
        );
        let place = Self::UNDERSTANDING * understanding
            + Self::TEAMWORK * m.teamwork / 20.0
            + Self::ROLE * role
            + Self::SIDE * side_familiarity.clamp(0.0, 1.0)
            + Self::BUY_IN * player.buy_in;
        let condition = (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
        let press = Self::PRESS_PLACE * place
            + Self::WORK_RATE * m.work_rate / 20.0
            + Self::CONDITION * condition;
        (place, press)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::mind::KickoffMind;
    use crate::club::player::{PlayerPosition, PlayerPositions};
    use crate::r#match::team::ShapeDiscipline;
    use crate::{PersonAttributes, PlayerAttributes, PlayerPositionType, PlayerSkills, RoleLevels};
    use chrono::NaiveDate;

    const ROLE: PlayerPositionType = PlayerPositionType::MidfielderCenterLeft;

    /// A level-14 central midfielder, fresh, natural in the role, with
    /// neutral buy-in.
    fn player() -> MatchPlayer {
        let mut skills = PlayerSkills::default();
        let m = &mut skills.mental;
        for skill in [
            &mut m.decisions,
            &mut m.positioning,
            &mut m.anticipation,
            &mut m.concentration,
            &mut m.teamwork,
            &mut m.work_rate,
        ] {
            *skill = 14.0;
        }
        MatchPlayer::from_inputs(
            1,
            1,
            [0.0; 3],
            [0.0; 3],
            PersonAttributes::default(),
            PlayerAttributes {
                condition: 10_000,
                ..PlayerAttributes::default()
            },
            skills,
            ROLE,
            None,
            Vec::new(),
            NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
            false,
            10_000,
            0.0,
            1.0,
            1.0,
            KickoffMind::neutral(),
            RoleLevels::natural(ROLE),
            0.5,
            false,
        )
    }

    fn reading(change: impl FnOnce(&mut MatchPlayer)) -> Adherence {
        let mut p = player();
        change(&mut p);
        Adherence::of(&p, 1.0)
    }

    #[test]
    fn understanding_the_game_holds_the_place_and_the_press() {
        let base = reading(|_| {});
        let sharp = reading(|p| {
            let m = &mut p.skills.mental;
            m.decisions = 18.0;
            m.positioning = 18.0;
            m.anticipation = 18.0;
            m.concentration = 18.0;
        });
        assert!(sharp.place > base.place && sharp.press > base.press);
    }

    #[test]
    fn teamwork_holds_the_place() {
        assert!(reading(|p| p.skills.mental.teamwork = 18.0).place > reading(|_| {}).place);
    }

    #[test]
    fn work_rate_keeps_a_player_in_the_press() {
        let base = reading(|_| {});
        let worker = reading(|p| p.skills.mental.work_rate = 18.0);
        assert!(worker.press > base.press);
        assert_eq!(worker.place, base.place);
    }

    #[test]
    fn the_press_fades_with_the_legs() {
        let fresh = reading(|_| {});
        let tired = reading(|p| p.player_attributes.condition = 5_000);
        assert!(tired.press < fresh.press);
        assert_eq!(tired.place, fresh.place);
    }

    #[test]
    fn a_centre_back_in_midfield_holds_the_place_less_than_a_natural() {
        let natural = reading(|_| {});
        let centre_back = reading(|p| {
            p.role_levels = RoleLevels::of(&PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::DefenderCenterLeft,
                    level: 20,
                }],
            })
        });
        assert!(centre_back.place < natural.place);
    }

    #[test]
    fn out_of_position_a_man_is_recalled_to_his_place_less_hard() {
        let natural = reading(|_| {});
        let centre_back = reading(|p| {
            p.role_levels = RoleLevels::of(&PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::DefenderCenterLeft,
                    level: 20,
                }],
            })
        });
        let pull = |place: f32| ShapeDiscipline::pull(200.0, ShapeDiscipline::held(place, 0.5));
        assert!(pull(centre_back.place) < pull(natural.place));
        assert!((ShapeDiscipline::held(1.0, 0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_side_new_to_its_shape_holds_it_less() {
        let p = player();
        assert!(Adherence::of(&p, 0.3).place < Adherence::of(&p, 1.0).place);
    }

    #[test]
    fn belief_in_the_coach_holds_the_place() {
        assert!(reading(|p| p.buy_in = 0.9).place > reading(|p| p.buy_in = 0.1).place);
    }
}
