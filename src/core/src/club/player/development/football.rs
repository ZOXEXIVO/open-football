//! The football a player has played, banked for his career by how well its
//! standard fitted him, and the work each match does.

use super::ceilings::PositionalSkillCeilings;
use super::ledger::GrowthSource;
use super::skills_array::SKILL_COUNT;
use crate::club::player::maturation::MaturationGroup;
use crate::club::player::player::Player;
use crate::utils::DateUtils;
use crate::{PlayerPositionType, PlayerSkills};
use chrono::NaiveDate;

/// Football banked per maturation family, each in 0..1: a share of what a
/// career needs to open the part of a family that only matches build. It
/// never fades and a change of club does not touch it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BankedFootball {
    banked: [f32; MaturationGroup::COUNT],
}

impl BankedFootball {
    pub fn of(&self, family: MaturationGroup) -> f32 {
        self.banked[family as usize]
    }

    /// Mean over the families: one figure for a reader.
    pub fn share(&self) -> f32 {
        self.banked.iter().sum::<f32>() / MaturationGroup::COUNT as f32
    }

    pub(super) fn credit(&mut self, family: MaturationGroup, matches: f32) -> f32 {
        let slot = &mut self.banked[family as usize];
        let before = *slot;
        *slot = (*slot + matches / FootballFit::career_unit(family)).min(1.0);
        *slot - before
    }

    pub(super) fn set(&mut self, family: MaturationGroup, share: f32) {
        self.banked[family as usize] = share.clamp(0.0, 1.0);
    }
}

/// One match as a player's development reads it.
#[derive(Debug, Clone, Copy)]
pub struct MatchFootball {
    pub minutes: f32,
    /// `MatchResultRaw::standard_of_football`, 0..1.
    pub standard: f32,
    /// The slot he filled, or his own position when that is unknown.
    pub slot: PlayerPositionType,
    /// His regressed season rating going into the match.
    pub season_rating: f32,
    pub season_games: u16,
}

/// How well a match's standard fits a player's own.
pub struct FootballFit;

impl FootballFit {
    /// Football far below him teaches this much of a minute at his level.
    const FAR_BELOW: f32 = 0.25;
    /// Football far above him teaches this much.
    const FAR_ABOVE: f32 = 0.6;
    /// Football up to this far above him fits him fully.
    const STRETCH: f32 = 0.03;
    const BELOW_WIDTH: f32 = 0.07;
    const ABOVE_WIDTH: f32 = 0.12;

    /// His own standard on the match standard's scale: his position's
    /// weighted attribute level over 20, read from his ability.
    pub fn own_standard(current_ability: u8) -> f32 {
        PlayerSkills::ability_skill_level(current_ability) / 20.0
    }

    /// One smooth hump over `match − own`: full for football around his
    /// standard or a little above it, falling toward a quarter for football
    /// far below him and toward three fifths for football far above him.
    pub fn of(match_standard: f32, own: f32) -> f32 {
        let d = match_standard - own;
        if d < 0.0 {
            Self::FAR_BELOW + (1.0 - Self::FAR_BELOW) * (-(d / Self::BELOW_WIDTH).powi(2)).exp()
        } else if d <= Self::STRETCH {
            1.0
        } else {
            let over = (d - Self::STRETCH) / Self::ABOVE_WIDTH;
            Self::FAR_ABOVE + (1.0 - Self::FAR_ABOVE) * (-over * over).exp()
        }
    }

    /// Matches (90 minutes at full fit and full plasticity) a family needs
    /// banked: a regular at a fitting standard from 18 banks the outfield
    /// families by about 21 (five seasons of thirty appearances from 20 bank
    /// them whole) and a keeper's craft by about 23.
    fn career_unit(family: MaturationGroup) -> f32 {
        match family {
            MaturationGroup::Technical => 85.0,
            MaturationGroup::Mental => 105.0,
            MaturationGroup::Physical => 75.0,
            MaturationGroup::Explosive => 65.0,
            MaturationGroup::Goalkeeping => 160.0,
        }
    }

    /// How much of a match's work goes to each family, against technique.
    fn match_weight(family: MaturationGroup) -> f32 {
        match family {
            MaturationGroup::Technical => 1.0,
            MaturationGroup::Mental => 1.2,
            MaturationGroup::Physical => 0.6,
            MaturationGroup::Explosive => 0.3,
            MaturationGroup::Goalkeeping => 1.2,
        }
    }
}

impl Player {
    /// Progress a full match at a fitting standard gives an attribute his
    /// position prices at the mean.
    const MATCH_YIELD: f32 = 0.012;

    /// A match he played: banked by its minutes, how well its standard fits
    /// him and how plastic he still is, and its work on the attributes his
    /// slot prices.
    pub fn bank_football(&mut self, football: &MatchFootball, date: NaiveDate) {
        if football.minutes <= 0.0 {
            return;
        }
        self.development.ledger.roll(date);
        self.development.layoff.on_work();
        let age = DateUtils::age_in_years(self.birth_date, date);
        let own = FootballFit::own_standard(self.player_attributes.current_ability);
        let fit = FootballFit::of(football.standard, own);
        let matches = football.minutes / 90.0 * fit;
        let mut banked = 0.0;
        for family in MaturationGroup::ALL {
            let plasticity = self.development.plasticity(age, family);
            banked += self
                .development
                .football
                .credit(family, matches * plasticity);
        }
        self.development.ledger.record_football(
            football.minutes,
            fit,
            football.standard < own,
            banked / MaturationGroup::COUNT as f32,
        );

        let prices = PlayerSkills::ability_weights(football.slot);
        let weights = self.development.weights(self);
        let present: Vec<f32> = (0..SKILL_COUNT)
            .filter(|i| weights[*i] > 0.0 && !PositionalSkillCeilings::is_condition(*i))
            .map(|i| prices[i])
            .collect();
        let mean_price = present.iter().sum::<f32>() / present.len().max(1) as f32;
        if mean_price <= 0.0 {
            return;
        }
        let rating = if football.season_games == 0 {
            1.0
        } else {
            (1.0 + (football.season_rating - 7.0) * 0.10).clamp(0.85, 1.25)
        };
        let mut work = [0.0f32; SKILL_COUNT];
        for (i, slot) in work.iter_mut().enumerate() {
            if weights[i] <= 0.0 || PositionalSkillCeilings::is_condition(i) {
                continue;
            }
            let family = PositionalSkillCeilings::maturation_group(i);
            *slot = matches
                * Self::MATCH_YIELD
                * rating
                * FootballFit::match_weight(family)
                * prices[i]
                / mean_price;
        }
        self.grow(&work, date, GrowthSource::Matches);
    }
}

/// A player's place in the side the world opens with, for one whose
/// career is not recorded: inside his group's starting slots a regular, in
/// the tier behind them a squad player, beyond it on the fringe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SquadRole {
    share: f32,
}

impl SquadRole {
    const REGULAR: f32 = 1.0;
    const SQUAD: f32 = 0.3;
    const FRINGE: f32 = 0.05;
    /// A side whose league is played as friendlies plays youth or reserve
    /// football, a level a regular there has long outgrown.
    const FRIENDLY_LEAGUE: f32 = 0.5;

    /// `rank` is his ability rank in his position group (0 the best), of a
    /// group that starts `starters`.
    pub fn of(rank: usize, starters: usize, plays_friendlies: bool) -> Self {
        let share = if rank < starters {
            Self::REGULAR
        } else if rank < starters * 2 {
            Self::SQUAD
        } else {
            Self::FRINGE
        };
        let level = if plays_friendlies {
            Self::FRIENDLY_LEAGUE
        } else {
            1.0
        };
        SquadRole {
            share: share * level,
        }
    }
}

impl Player {
    /// Minutes a recorded substitute appearance stands for.
    const SUB_MINUTES: f32 = 20.0;
    /// League and cup matches in a regular's season.
    const SEASON_MATCHES: f32 = 38.0;
    /// The age a regular's career is taken to start at.
    const CAREER_START: f32 = 17.0;

    /// A world opens: bank the football every season of his record holds,
    /// each at the plasticity of his age that season and a fitting
    /// standard. With no season recorded, what `role` implies: a regular's
    /// football since 17, in proportion to his place.
    pub fn seed_football(&mut self, opening: NaiveDate, role: Option<SquadRole>) {
        let born = self.birth_date;
        let mut matches = [0.0f32; MaturationGroup::COUNT];
        let items = &self.statistics_history.items;
        if !items.is_empty() {
            for item in items {
                let minutes = item.statistics.played as f32 * 90.0
                    + item.statistics.played_subs as f32 * Self::SUB_MINUTES;
                let middle = NaiveDate::from_ymd_opt(item.season.start_year as i32 + 1, 1, 1)
                    .unwrap_or(opening);
                let age = DateUtils::age_in_years(born, middle);
                for family in MaturationGroup::ALL {
                    matches[family as usize] +=
                        minutes / 90.0 * self.development.plasticity(age, family);
                }
            }
        } else if let Some(role) = role {
            let now = DateUtils::age_in_years(born, opening);
            let mut age = Self::CAREER_START + 0.5;
            while age < now {
                for family in MaturationGroup::ALL {
                    matches[family as usize] += Self::SEASON_MATCHES
                        * role.share
                        * self.development.plasticity(age, family);
                }
                age += 1.0;
            }
        }
        for family in MaturationGroup::ALL {
            self.development.football.set(
                family,
                matches[family as usize] / FootballFit::career_unit(family),
            );
        }
    }
}
