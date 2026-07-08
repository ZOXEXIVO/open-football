use crate::club::staff::perception::CoachDecisionState;
use crate::{Player, PlayerFieldPositionGroup, Team};

/// The coach's read on how complete his squad is (0.0–1.0): is it big enough,
/// balanced, covered at every position, and performing? Blended from four
/// weighted factors — size (25%), recent performance (35%), perceived quality
/// spread (15%), position coverage (25%). Refreshed onto `CoachDecisionState`
/// each tick and consumed as a recruitment-urgency signal.
pub struct SquadSatisfaction;

impl SquadSatisfaction {
    pub fn compute(main_team: &Team, state: &CoachDecisionState) -> f32 {
        let players = &main_team.players.players;
        Self::size(players.len()) * 0.25
            + Self::performance(players) * 0.35
            + Self::quality_spread(players, state) * 0.15
            + Self::position_coverage(players) * 0.25
    }

    /// Optimal squad is 20–23 players.
    fn size(squad_size: usize) -> f32 {
        match squad_size {
            20..=23 => 1.0,
            18..=25 => 0.7,
            14..=17 => 0.4,
            _ => 0.1,
        }
    }

    /// Sample-size-regressed average match rating of players with 4+
    /// appearances, so a thin breakout sample doesn't inflate it. Rating
    /// 5.5–7.5 maps onto 0.0–1.0.
    fn performance(players: &[Player]) -> f32 {
        let (sum, count) = players
            .iter()
            .filter(|p| p.statistics.played + p.statistics.played_subs > 3)
            .map(|p| {
                let pos = p.position().position_group();
                p.statistics.average_rating_realistic(pos)
            })
            .fold((0.0_f32, 0u32), |(s, c), r| (s + r, c + 1));

        if count == 0 {
            return 0.5;
        }
        (((sum / count as f32) - 5.5) / 2.0).clamp(0.0, 1.0)
    }

    /// Penalises large gaps between best and worst perceived quality
    /// (gap of 10+ → 0.0, gap of 0 → 1.0).
    fn quality_spread(players: &[Player], state: &CoachDecisionState) -> f32 {
        let (count, max_q, min_q) = players
            .iter()
            .filter_map(|p| state.impressions.get(&p.id).map(|imp| imp.perceived_quality))
            .fold(
                (0u32, f32::NEG_INFINITY, f32::INFINITY),
                |(c, mx, mn), q| (c + 1, mx.max(q), mn.min(q)),
            );

        if count < 2 {
            return 0.5;
        }
        (1.0 - (max_q - min_q) / 10.0).clamp(0.0, 1.0)
    }

    /// Minimum coverage (1 GK, 3 DEF, 2 MID, 1 FWD) among players who are
    /// actually available. Injured, suspended/banned, and internationally
    /// away players don't count — a squad papered over by unavailable bodies
    /// must not read as covered (the old check only excluded the injured).
    fn position_coverage(players: &[Player]) -> f32 {
        let available: Vec<_> = players.iter().filter(|p| Self::is_available(p)).collect();

        let count = |group: PlayerFieldPositionGroup| -> usize {
            available
                .iter()
                .filter(|p| p.position().position_group() == group)
                .count()
        };

        let covered = count(PlayerFieldPositionGroup::Goalkeeper) >= 1
            && count(PlayerFieldPositionGroup::Defender) >= 3
            && count(PlayerFieldPositionGroup::Midfielder) >= 2
            && count(PlayerFieldPositionGroup::Forward) >= 1;

        if covered { 1.0 } else { 0.2 }
    }

    /// Selectable right now — not injured, not suspended/banned, not away on
    /// international duty.
    fn is_available(p: &Player) -> bool {
        !p.player_attributes.is_injured
            && !p.player_attributes.is_banned
            && !p.statuses.is_on_international_duty()
    }
}
