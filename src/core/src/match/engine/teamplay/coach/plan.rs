//! **The game plan.** Seven continuous dials, each 0..1, that say how a side
//! means to play whatever its formation: how high it defends, how hard it
//! presses, how compact it stays without the ball, how wide it plays with
//! it, how quickly and how directly it moves the ball, and how many men it
//! commits forward. 0.5 on every dial is the neutral plan.

use crate::r#match::engine::teamplay::coach::card::CoachCard;
use crate::r#match::engine::teamplay::tactical::TeamSkillAggregates;
use crate::r#match::MatchSquad;
use crate::PlayerFieldPositionGroup;
use serde::{Deserialize, Serialize};

/// What an eleven can carry out against this opponent, read at kickoff:
/// each composite over the opponent's, and its back line's pace over the
/// opponent's forwards', on the 0..1 attribute scale.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SquadFit {
    pub build_up: f32,
    pub press: f32,
    pub defence: f32,
    pub attack: f32,
    pub back_line_pace: f32,
}

impl SquadFit {
    /// The fit of `own`'s eleven against `opp`'s.
    pub fn of_squads(own: &MatchSquad, opp: &MatchSquad) -> Self {
        let pace = |squad: &MatchSquad, group: PlayerFieldPositionGroup| {
            let (sum, n) = squad
                .main_squad
                .iter()
                .filter(|p| p.tactical_position.current_position.position_group() == group)
                .fold((0.0f32, 0u32), |(s, n), p| (s + p.skills.physical.pace, n + 1));
            if n == 0 { 0.5 } else { sum / n as f32 / 20.0 }
        };
        Self::read(
            &TeamSkillAggregates::at_kickoff(own.main_squad.iter()),
            &TeamSkillAggregates::at_kickoff(opp.main_squad.iter()),
            pace(own, PlayerFieldPositionGroup::Defender)
                - pace(opp, PlayerFieldPositionGroup::Forward),
        )
    }

    pub fn read(
        own: &TeamSkillAggregates,
        opp: &TeamSkillAggregates,
        back_line_pace: f32,
    ) -> Self {
        SquadFit {
            build_up: own.build_up_quality - opp.build_up_quality,
            press: own.press_quality - opp.press_quality,
            defence: own.defensive_quality - opp.defensive_quality,
            attack: own.attacking_quality - opp.attacking_quality,
            back_line_pace,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GamePlan {
    pub block_height: f32,
    pub press: f32,
    pub compactness: f32,
    pub width: f32,
    pub tempo: f32,
    pub directness: f32,
    pub commitment: f32,
}

impl GamePlan {
    pub const DIALS: [&'static str; 7] = [
        "block",
        "press",
        "compact",
        "width",
        "tempo",
        "direct",
        "commit",
    ];

    pub const fn neutral() -> Self {
        GamePlan {
            block_height: 0.5,
            press: 0.5,
            compactness: 0.5,
            width: 0.5,
            tempo: 0.5,
            directness: 0.5,
            commitment: 0.5,
        }
    }

    /// How much a dial moves with the kickoff stance: block, press and
    /// commitment rise with it, directness and compactness fall.
    const DEMAND: [f32; 7] = [0.20, 0.15, -0.15, 0.0, 0.0, -0.15, 0.25];
    /// What an equal eleven can sustain on the capped dials, and how far a
    /// composite edge moves it.
    const CAPACITY: f32 = 0.75;
    const CAPACITY_PER_EDGE: f32 = 2.5;

    /// The plan a coach sets at kickoff: the football he prefers, moved by
    /// what the kickoff stance demands, then pulled toward what the eleven
    /// can carry out — harder by a coach who reads the game and adapts.
    pub fn for_kickoff(card: &CoachCard, stance: f32, fit: &SquadFit) -> Self {
        let mut dials = card.preference.dials();
        for (dial, demand) in dials.iter_mut().zip(Self::DEMAND) {
            *dial += demand * stance;
        }
        let t = &card.temperament;
        let weight = 0.3 + 0.5 * (0.5 * t.read + 0.5 * t.adaptability);
        let edge = |e: f32| Self::CAPACITY + Self::CAPACITY_PER_EDGE * e;
        let cap = |dial: f32, capacity: f32| {
            if dial > capacity {
                dial - weight * (dial - capacity)
            } else {
                dial
            }
        };
        dials[0] = cap(dials[0], edge(fit.defence) + fit.back_line_pace);
        dials[1] = cap(dials[1], edge(fit.press));
        dials[6] = cap(dials[6], edge(fit.attack));
        let floor = 1.0 - edge(fit.build_up);
        if dials[5] < floor {
            dials[5] += weight * (floor - dials[5]);
        }
        Self::from_dials(dials)
    }

    /// How far apart two plans are: the mean move across the dials.
    pub fn distance(&self, other: &GamePlan) -> f32 {
        self.dials()
            .iter()
            .zip(other.dials())
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            / 7.0
    }

    pub fn dials(&self) -> [f32; 7] {
        [
            self.block_height,
            self.press,
            self.compactness,
            self.width,
            self.tempo,
            self.directness,
            self.commitment,
        ]
    }

    pub fn from_dials(dials: [f32; 7]) -> Self {
        let [block_height, press, compactness, width, tempo, directness, commitment] =
            dials.map(|d| d.clamp(0.0, 1.0));
        GamePlan {
            block_height,
            press,
            compactness,
            width,
            tempo,
            directness,
            commitment,
        }
    }

    /// A pinned plan, as `OF_PLAN` states it: `neutral`, or dials named
    /// in [`DIALS`](Self::DIALS) with their values, separated by `/`, the
    /// rest neutral — `block=0.8/press=0.9`.
    pub fn from_pin(spec: &str) -> Result<Self, String> {
        let spec = spec.trim();
        if spec.eq_ignore_ascii_case("neutral") {
            return Ok(Self::neutral());
        }
        let mut dials = Self::neutral().dials();
        for part in spec.split('/').filter(|p| !p.trim().is_empty()) {
            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| format!("plan dial `{part}` is not `name=value`"))?;
            let index = Self::DIALS
                .iter()
                .position(|d| d.eq_ignore_ascii_case(key.trim()))
                .ok_or_else(|| {
                    format!(
                        "unknown plan dial `{}`; dials are {}",
                        key.trim(),
                        Self::DIALS.join(", ")
                    )
                })?;
            let value: f32 = value
                .trim()
                .parse()
                .map_err(|_| format!("plan dial `{}` needs a number 0..1", key.trim()))?;
            if !(0.0..=1.0).contains(&value) {
                return Err(format!("plan dial `{}` must be within 0..1", key.trim()));
            }
            dials[index] = value;
        }
        Ok(Self::from_dials(dials))
    }

    pub fn describe(&self) -> String {
        Self::DIALS
            .iter()
            .zip(self.dials())
            .map(|(name, value)| format!("{name} {value:.2}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl Default for GamePlan {
    fn default() -> Self {
        Self::neutral()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coach(preference: &str) -> CoachCard {
        let mut card = CoachCard::neutral();
        card.preference = GamePlan::from_pin(preference).unwrap();
        card
    }

    #[test]
    fn one_formation_two_plans() {
        let even = SquadFit::default();
        let pressing = GamePlan::for_kickoff(&coach("block=0.8/press=0.8"), 0.0, &even);
        let sitting = GamePlan::for_kickoff(&coach("block=0.2/press=0.2"), 0.0, &even);
        assert!(pressing.block_height > sitting.block_height);
        assert!(pressing.press > sitting.press);
    }

    #[test]
    fn the_underdog_away_plays_deeper_tighter_and_commits_fewer() {
        let card = coach("neutral");
        let favourite = GamePlan::for_kickoff(&card, 0.6, &SquadFit::default());
        let underdog = GamePlan::for_kickoff(&card, -0.6, &SquadFit::default());
        assert!(underdog.block_height < favourite.block_height);
        assert!(underdog.compactness > favourite.compactness);
        assert!(underdog.commitment < favourite.commitment);
    }

    #[test]
    fn a_plan_the_eleven_cannot_play_turns_more_direct() {
        let card = coach("direct=0.15/tempo=0.3");
        let can = GamePlan::for_kickoff(&card, 0.0, &SquadFit::default());
        let cannot = GamePlan::for_kickoff(
            &card,
            0.0,
            &SquadFit {
                build_up: -0.12,
                ..SquadFit::default()
            },
        );
        assert!(cannot.directness > can.directness);
        assert!(cannot.directness > card.preference.directness);
    }

    #[test]
    fn an_adaptable_reader_fits_the_plan_to_the_players_more() {
        let short_of_legs = SquadFit {
            press: -0.15,
            ..SquadFit::default()
        };
        let mut dogmatic = coach("press=0.9");
        dogmatic.temperament.read = 0.1;
        dogmatic.temperament.adaptability = 0.1;
        let mut pragmatic = coach("press=0.9");
        pragmatic.temperament.read = 0.9;
        pragmatic.temperament.adaptability = 0.9;
        let d = GamePlan::for_kickoff(&dogmatic, 0.0, &short_of_legs);
        let p = GamePlan::for_kickoff(&pragmatic, 0.0, &short_of_legs);
        assert!(p.press < d.press && d.press < 0.9);
    }

    #[test]
    fn a_pin_names_dials_and_leaves_the_rest_neutral() {
        let plan = GamePlan::from_pin("block=0.8/press=0.9").unwrap();
        assert_eq!(plan.block_height, 0.8);
        assert_eq!(plan.press, 0.9);
        assert_eq!(plan.tempo, 0.5);
        assert_eq!(GamePlan::from_pin("neutral").unwrap(), GamePlan::neutral());
    }

    #[test]
    fn a_bad_pin_is_refused() {
        assert!(GamePlan::from_pin("height=0.8").is_err());
        assert!(GamePlan::from_pin("press=1.4").is_err());
        assert!(GamePlan::from_pin("press").is_err());
    }
}
