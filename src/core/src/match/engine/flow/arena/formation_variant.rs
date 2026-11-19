//! **Who stands where**: a side laid out on a formation, the role a side
//! short of men gives up, and how the short line spreads itself.
//!
//! Every slot in a formation is a role, and the live shape is drawn from
//! the slots the players stand in. A change of formation and a man lost are
//! one operation: the men on the pitch take the roles that move the side
//! least, and a side with fewer men than roles leaves empty the ones from
//! the line the coach can spare — the attack when the side is protecting or
//! level, the midfield when it has to chase. The line left short then
//! spreads its remaining men across its whole width instead of leaving a
//! hole where the man was.

use crate::r#match::{
    MatchField, POSITION_POSITIONING, PlayerSide, PositionType, TransitionSource,
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

    /// `departed` has just left `team_id` a man short: the side re-lays its
    /// men onto the roles it had, leaving empty the one it can best afford.
    /// Returns the role left empty.
    pub fn reshape(
        field: &mut MatchField,
        team_id: u32,
        departed: PlayerPositionType,
        spare: FormationLine,
    ) -> PlayerPositionType {
        if FormationLine::of(departed).is_none() {
            return departed;
        }
        let mut roles: Vec<PlayerPositionType> = field
            .players
            .iter()
            .filter(|p| p.team_id == team_id && !p.off_pitch)
            .map(|p| p.tactical_position.current_position)
            .collect();
        roles.push(departed);
        Self::lay_out(field, team_id, &roles, spare)
            .first()
            .copied()
            .unwrap_or(departed)
    }

    /// Lay `team_id`'s outfielders on the pitch onto the outfield `roles`.
    /// Each takes the role that moves the side least — the smallest sum of
    /// squared steps from the slot he holds — so a gap is filled by a chain
    /// of short steps rather than one long one. A side with fewer men than
    /// roles leaves empty the ones it can best afford, from the line the
    /// coach can `spare` while it has two to give. Returns the roles left
    /// empty.
    pub fn lay_out(
        field: &mut MatchField,
        team_id: u32,
        roles: &[PlayerPositionType],
        spare: FormationLine,
    ) -> Vec<PlayerPositionType> {
        let side = field.side_of(team_id);
        let (taken, empty) = Self::assignment(field, team_id, roles, spare);
        for (index, role) in taken {
            let Some(spot) = Self::slot(role, side) else {
                continue;
            };
            let player = &mut field.players[index];
            if player.tactical_position.current_position == role {
                continue;
            }
            player.tactical_position.current_position = role;
            player.tactical_position.regenerate_waypoints(Some(side));
            player.rebuild_waypoint_cache();
            player.start_position = spot;
            player.set_default_state(TransitionSource::Reset);
        }
        empty
    }

    /// Whether every man `team_id` has on the pitch can play the role he
    /// would take if the side laid out on `roles`.
    pub fn can_fill(
        field: &MatchField,
        team_id: u32,
        roles: &[PlayerPositionType],
        spare: FormationLine,
    ) -> bool {
        Self::assignment(field, team_id, roles, spare)
            .0
            .iter()
            .all(|&(index, role)| field.players[index].role_levels.can_play(role))
    }

    /// Who takes which outfield role if `team_id` lays out on `roles`: each
    /// man's index in `field.players` with his role, and the roles left
    /// empty.
    fn assignment(
        field: &MatchField,
        team_id: u32,
        roles: &[PlayerPositionType],
        spare: FormationLine,
    ) -> (Vec<(usize, PlayerPositionType)>, Vec<PlayerPositionType>) {
        const NEVER: f32 = 1.0e9;
        let side = field.side_of(team_id);
        let roles: Vec<PlayerPositionType> =
            roles.iter().copied().filter(|r| !r.is_goalkeeper()).collect();
        let men: Vec<usize> = field
            .players
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.team_id == team_id
                    && !p.off_pitch
                    && !p.tactical_position.current_position.is_goalkeeper()
            })
            .map(|(i, _)| i)
            .collect();
        let mut short: Vec<FormationLine> = Vec::new();
        for _ in men.len()..roles.len() {
            short.push(Self::affordable(&roles, &short, spare));
        }
        let mut costs: Vec<Vec<f32>> = men
            .iter()
            .map(|&i| {
                let from = Self::slot(field.players[i].tactical_position.current_position, side);
                roles
                    .iter()
                    .map(|&role| match (from, Self::slot(role, side)) {
                        (Some(a), Some(b)) => (a - b).norm_squared(),
                        _ => NEVER,
                    })
                    .collect()
            })
            .collect();
        costs.extend(short.iter().map(|&line| {
            roles
                .iter()
                .map(|&role| {
                    if FormationLine::of(role) == Some(line) {
                        0.0
                    } else {
                        NEVER
                    }
                })
                .collect()
        }));
        if costs.len() != roles.len() || roles.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let column = Self::hungarian(&costs);
        let taken = men
            .iter()
            .zip(&column)
            .map(|(&index, &c)| (index, roles[c]))
            .collect();
        let empty = column[men.len()..].iter().map(|&c| roles[c]).collect();
        (taken, empty)
    }

    /// Kuhn–Munkres on a square cost matrix: the column for each row.
    fn hungarian(costs: &[Vec<f32>]) -> Vec<usize> {
        let n = costs.len();
        let (mut u, mut v) = (vec![0.0f64; n + 1], vec![0.0f64; n + 1]);
        let (mut p, mut way) = (vec![0usize; n + 1], vec![0usize; n + 1]);
        for i in 1..=n {
            p[0] = i;
            let mut j0 = 0;
            let mut min = vec![f64::INFINITY; n + 1];
            let mut used = vec![false; n + 1];
            loop {
                used[j0] = true;
                let i0 = p[j0];
                let mut delta = f64::INFINITY;
                let mut j1 = 0;
                for j in 1..=n {
                    if used[j] {
                        continue;
                    }
                    let cur = costs[i0 - 1][j - 1] as f64 - u[i0] - v[j];
                    if cur < min[j] {
                        min[j] = cur;
                        way[j] = j0;
                    }
                    if min[j] < delta {
                        delta = min[j];
                        j1 = j;
                    }
                }
                for j in 0..=n {
                    if used[j] {
                        u[p[j]] += delta;
                        v[j] -= delta;
                    } else {
                        min[j] -= delta;
                    }
                }
                j0 = j1;
                if p[j0] == 0 {
                    break;
                }
            }
            loop {
                let j1 = way[j0];
                p[j0] = p[j1];
                j0 = j1;
                if j0 == 0 {
                    break;
                }
            }
        }
        let mut column = vec![0usize; n];
        for j in 1..=n {
            if p[j] > 0 {
                column[p[j] - 1] = j - 1;
            }
        }
        column
    }

    /// The line that gives up a role: the one the coach would spare while
    /// it has two left, otherwise the midfield, the attack, the defence in
    /// that order. `taken` are the lines already giving one up.
    fn affordable(
        roles: &[PlayerPositionType],
        taken: &[FormationLine],
        spare: FormationLine,
    ) -> FormationLine {
        let left = |line: FormationLine| {
            roles
                .iter()
                .filter(|r| FormationLine::of(**r) == Some(line))
                .count()
                .saturating_sub(taken.iter().filter(|&&t| t == line).count())
        };
        [
            spare,
            FormationLine::Midfield,
            FormationLine::Attack,
            FormationLine::Defence,
        ]
        .into_iter()
        .find(|&line| left(line) >= 2)
        .unwrap_or(spare)
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
