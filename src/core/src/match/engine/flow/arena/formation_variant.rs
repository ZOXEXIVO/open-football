//! **A side short of men**: which line gives up a man, who moves across to
//! fill the role that was left, and how the short line spreads itself.
//!
//! Every slot in a formation is a role, and the live shape is drawn from
//! the slots the players stand in. Losing a man leaves one empty. The coach
//! names the line he can spare — the attack when the side is protecting or
//! level, the midfield when it has to chase — and every line between the
//! gap and that one hands its nearest man across, so the role that matters
//! is filled and the one left empty is the one he chose. The line left
//! short then spreads its remaining men across its whole width instead of
//! leaving a hole where the man was.

use crate::r#match::{
    MatchField, MatchPlayer, POSITION_POSITIONING, PlayerSide, PositionType, TransitionSource,
};
use crate::{PlayerFieldPositionGroup, PlayerPositionType};
use nalgebra::Vector3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormationLine {
    Defence,
    Midfield,
    Attack,
}

impl FormationLine {
    /// The outfield line a role belongs to; the keeper is in none.
    pub fn of(position: PlayerPositionType) -> Option<Self> {
        match position.position_group() {
            PlayerFieldPositionGroup::Goalkeeper => None,
            PlayerFieldPositionGroup::Defender => Some(Self::Defence),
            PlayerFieldPositionGroup::Midfielder => Some(Self::Midfield),
            PlayerFieldPositionGroup::Forward => Some(Self::Attack),
        }
    }

    fn depth(self) -> i8 {
        match self {
            Self::Defence => 0,
            Self::Midfield => 1,
            Self::Attack => 2,
        }
    }

    /// The next line from this one toward `target`.
    fn toward(self, target: Self) -> Self {
        let step = (target.depth() - self.depth()).signum();
        match self.depth() + step {
            0 => Self::Defence,
            1 => Self::Midfield,
            _ => Self::Attack,
        }
    }
}

/// A role a side is playing without for the rest of the match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VacatedSlot {
    pub team_id: u32,
    pub position: PlayerPositionType,
}

pub struct FormationVariant;

impl FormationVariant {
    /// Where the slot for `position` stands on `side`'s half.
    pub fn slot(position: PlayerPositionType, side: PlayerSide) -> Option<Vector3<f32>> {
        POSITION_POSITIONING
            .iter()
            .find(|(pos, _, _)| *pos == position)
            .and_then(|(_, home, away)| match (side, home, away) {
                (PlayerSide::Left, PositionType::Home(x, y), _)
                | (PlayerSide::Right, _, PositionType::Away(x, y)) => {
                    Some(Vector3::new(*x as f32, *y as f32, 0.0))
                }
                _ => None,
            })
    }

    /// `departed` has just left `team_id` a man short: move men across
    /// from the lines between the gap and the one the coach can `spare`.
    /// Returns the role left empty.
    pub fn reshape(
        field: &mut MatchField,
        team_id: u32,
        departed: PlayerPositionType,
        spare: FormationLine,
    ) -> PlayerPositionType {
        let Some(mut gap_line) = FormationLine::of(departed) else {
            return departed;
        };
        let side = field.side_of(team_id);
        let target = Self::affordable(field, team_id, gap_line, spare);
        let mut gap = departed;
        while gap_line != target {
            let donor_line = gap_line.toward(target);
            let Some(gap_spot) = Self::slot(gap, side) else {
                break;
            };
            let Some(donor) = field
                .players
                .iter_mut()
                .filter(|p| {
                    p.team_id == team_id
                        && !p.off_pitch
                        && FormationLine::of(p.tactical_position.current_position)
                            == Some(donor_line)
                })
                .min_by(|a, b| {
                    let near = |p: &MatchPlayer| {
                        Self::slot(p.tactical_position.current_position, side)
                            .map_or(f32::MAX, |spot| (spot - gap_spot).norm())
                    };
                    near(a).total_cmp(&near(b))
                })
            else {
                break;
            };
            let left = donor.tactical_position.current_position;
            donor.tactical_position.current_position = gap;
            donor.tactical_position.regenerate_waypoints(Some(side));
            donor.start_position = gap_spot;
            donor.set_default_state(TransitionSource::Reset);
            gap = left;
            gap_line = donor_line;
        }
        gap
    }

    /// The line that gives up the man: the one the coach would spare if
    /// it has a man to give, otherwise the midfield, the attack, the
    /// defence in that order. A line with one man left gives up nobody.
    fn affordable(
        field: &MatchField,
        team_id: u32,
        gap_line: FormationLine,
        spare: FormationLine,
    ) -> FormationLine {
        let count = |line: FormationLine| {
            field
                .players
                .iter()
                .filter(|p| {
                    p.team_id == team_id
                        && !p.off_pitch
                        && FormationLine::of(p.tactical_position.current_position) == Some(line)
                })
                .count()
        };
        [
            spare,
            FormationLine::Midfield,
            FormationLine::Attack,
            FormationLine::Defence,
        ]
        .into_iter()
        .find(|&line| line == gap_line || count(line) >= 2)
        .unwrap_or(gap_line)
    }

    /// Spread every line a man short across the width it had when it was
    /// whole. A row is the slots standing at one depth.
    pub fn respace(field: &mut MatchField) {
        for vacated in field.vacated.clone() {
            let side = field.side_of(vacated.team_id);
            let Some(gap) = Self::slot(vacated.position, side) else {
                continue;
            };
            let same_row = |spot: Vector3<f32>| (spot.x - gap.x).abs() < 0.5;
            let mut row: Vec<(usize, Vector3<f32>)> = field
                .players
                .iter()
                .enumerate()
                .filter(|(_, p)| p.team_id == vacated.team_id && !p.off_pitch)
                .filter_map(|(i, p)| {
                    Self::slot(p.tactical_position.current_position, side)
                        .filter(|spot| same_row(*spot))
                        .map(|spot| (i, spot))
                })
                .collect();
            if row.is_empty() {
                continue;
            }
            let empty_in_row = field
                .vacated
                .iter()
                .filter(|v| v.team_id == vacated.team_id)
                .filter_map(|v| Self::slot(v.position, side))
                .filter(|spot| same_row(*spot));
            let (low, high) = row
                .iter()
                .map(|(_, spot)| *spot)
                .chain(empty_in_row)
                .fold((f32::MAX, f32::MIN), |(lo, hi), spot| {
                    (lo.min(spot.y), hi.max(spot.y))
                });
            row.sort_by(|a, b| a.1.y.total_cmp(&b.1.y));
            let n = row.len();
            for (rank, (i, spot)) in row.into_iter().enumerate() {
                let y = if n == 1 {
                    (low + high) * 0.5
                } else {
                    low + (high - low) * rank as f32 / (n - 1) as f32
                };
                field.players[i].start_position = Vector3::new(spot.x, y, 0.0);
            }
        }
    }
}
