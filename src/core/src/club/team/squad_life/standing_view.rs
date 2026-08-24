//! Per-player [`SquadStandingView`] snapshots.
//!
//! The sibling of [`SquadSocialViewBuilder`]: pre-compute, once per
//! weekly team pass, everything a player's read of his own prospects
//! needs and that a lone `Player` cannot possibly know — who picks the
//! side, who is in front of him at his position, how good and how old
//! that man is, where he sits in the pay order, and what the coaching
//! here can still teach him.
//!
//! **Every comparison is observable.** Ranks come from
//! [`AbilityEstimator::observable_level`], never from
//! `player_attributes.current_ability`: a player is no better a judge of
//! a teammate than a coach is, and the hidden digit is nobody's to read.
//!
//! [`SquadSocialViewBuilder`]: super::social_view::SquadSocialViewBuilder
//! [`AbilityEstimator::observable_level`]: crate::club::staff::perception::AbilityEstimator::observable_level

use chrono::NaiveDate;

use crate::club::player::core::player::SquadStandingView;
use crate::club::staff::StaffCollection;
use crate::club::staff::perception::AbilityEstimator;
use crate::club::team::model::TeamCoachingScores;
use crate::utils::DateUtils;
use crate::{Player, PlayerPositionType};

/// Builds [`SquadStandingView`] snapshots for a whole squad.
pub struct SquadStandingViewBuilder;

/// One player, reduced to the numbers the ranking needs.
struct Peer {
    id: u32,
    group_index: usize,
    is_keeper: bool,
    level: u8,
    age: u8,
    salary: u32,
}

impl SquadStandingViewBuilder {
    /// Refresh every player's standing view from the current roster.
    ///
    /// Two sorts and a linear walk over n ≤ 30. The observable level is
    /// computed once per player and reused — it is by far the most
    /// expensive part, and recomputing it inside the comparison would
    /// make the pass quadratic in the expensive term.
    pub fn refresh(
        players: &mut [Player],
        staffs: &StaffCollection,
        captain: Option<u32>,
        vice_captain: Option<u32>,
        today: NaiveDate,
    ) {
        if players.is_empty() {
            return;
        }

        let head_coach_id = staffs.head_coach().id;
        let coaching = TeamCoachingScores::from_staffs(staffs);
        // A keeper is taught by the goalkeeping bench and by nobody
        // else. Reading him against the outfield staff would make every
        // club with a good fitness coach look like a good place to be a
        // goalkeeper.
        let outfield_ceiling = coaching
            .technical
            .max(coaching.mental)
            .max(coaching.fitness);

        let peers: Vec<Peer> = players
            .iter()
            .map(|p| Peer {
                id: p.id,
                group_index: p.position().position_group().index(),
                is_keeper: p.position() == PlayerPositionType::Goalkeeper,
                level: AbilityEstimator::observable_level(p),
                age: DateUtils::age(p.birth_date, today) as u8,
                salary: p.contract.as_ref().map(|c| c.salary).unwrap_or(0),
            })
            .collect();

        // The pay order, best paid first. A loanee is ranked on what the
        // side he is actually at pays him, which is what the rest of the
        // dressing room would know about him.
        let mut wage_order: Vec<(u32, u32)> = peers.iter().map(|p| (p.id, p.salary)).collect();
        wage_order.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

        // The queue at each position group, best first. Ties break on id
        // so the order is stable across ticks — a rank that flickered
        // would read to the mind as losing and winning his place every
        // single week.
        let mut groups: [Vec<(u32, u8, u8)>; 4] = Default::default();
        for peer in &peers {
            groups[peer.group_index].push((peer.id, peer.level, peer.age));
        }
        for group in groups.iter_mut() {
            group.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        }

        let squad_size = players.len().min(u8::MAX as usize) as u8;

        for (player, peer) in players.iter_mut().zip(peers.iter()) {
            let group = &groups[peer.group_index];
            let rank = group
                .iter()
                .position(|(id, _, _)| *id == peer.id)
                .map(|i| (i + 1).min(u8::MAX as usize) as u8)
                .unwrap_or(0);

            // The man in his shirt is whoever is directly above him; for
            // the man at the top of the queue it is whoever is chasing
            // him. Same relationship, read from either end, and both ends
            // are things a player actually thinks about.
            let rival = if rank > 1 {
                group.get(rank as usize - 2)
            } else {
                group.get(1)
            };

            let (top_rival_id, top_rival_age, rival_gap) = match rival {
                Some((id, level, age)) => (
                    *id,
                    *age,
                    (peer.level as i16 - *level as i16).clamp(-100, 100) as i8,
                ),
                None => (0, 0, 0),
            };

            let wage_rank = wage_order
                .iter()
                .position(|(id, _)| *id == peer.id)
                .map(|i| (i + 1).min(u8::MAX as usize) as u8)
                .unwrap_or(0);

            player.squad_standing_view = Some(SquadStandingView {
                head_coach_id,
                pecking_rank: rank,
                rivals_at_position: group.len().saturating_sub(1).min(u8::MAX as usize) as u8,
                top_rival_id,
                top_rival_age,
                rival_gap,
                is_captain: captain == Some(peer.id),
                is_vice_captain: vice_captain == Some(peer.id),
                wage_rank,
                squad_size,
                observable_level: peer.level,
                coaching_ceiling: if peer.is_keeper {
                    coaching.goalkeeping
                } else {
                    outfield_ceiling
                },
            });
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

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2030, 6, 1).unwrap()
    }

    /// Quality is expressed through SKILLS, never the CA digit — the
    /// observable estimator does not read the digit, so a fixture that
    /// set it would rank every player identically.
    fn player(id: u32, position: PlayerPositionType, quality: u8, born: i32) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".into(), format!("Peer{}", id)))
            .birth_date(NaiveDate::from_ymd_opt(born, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .player_attributes(PlayerAttributes::default())
            .skills(PlayerSkills::flat_for_ability(quality))
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 20,
                }],
            })
            .build()
            .unwrap()
    }

    fn refresh(players: &mut Vec<Player>) {
        SquadStandingViewBuilder::refresh(
            players,
            &StaffCollection::new(Vec::new()),
            None,
            None,
            today(),
        );
    }

    fn view(players: &[Player], id: u32) -> SquadStandingView {
        players
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.squad_standing_view)
            .expect("every player in the squad gets a view")
    }

    #[test]
    fn the_queue_is_ranked_by_what_can_be_seen() {
        let mut squad = vec![
            player(1, PlayerPositionType::Striker, 150, 2000),
            player(2, PlayerPositionType::Striker, 110, 2000),
            player(3, PlayerPositionType::Striker, 90, 2000),
        ];
        refresh(&mut squad);

        assert_eq!(view(&squad, 1).pecking_rank, 1);
        assert_eq!(view(&squad, 2).pecking_rank, 2);
        assert_eq!(view(&squad, 3).pecking_rank, 3);
        assert_eq!(view(&squad, 2).rivals_at_position, 2);
    }

    #[test]
    fn the_rival_is_the_man_directly_in_front_of_him() {
        let mut squad = vec![
            player(1, PlayerPositionType::Striker, 150, 2000),
            player(2, PlayerPositionType::Striker, 110, 2000),
            player(3, PlayerPositionType::Striker, 90, 2000),
        ];
        refresh(&mut squad);

        // Third in the queue thinks about the man above him, not about
        // the star he is nowhere near.
        assert_eq!(view(&squad, 3).top_rival_id, 2);
        assert!(view(&squad, 3).rival_gap < 0);

        // And the man at the top thinks about whoever is chasing him.
        assert_eq!(view(&squad, 1).top_rival_id, 2);
        assert!(view(&squad, 1).rival_gap > 0);
    }

    #[test]
    fn a_man_alone_in_his_position_has_no_rival() {
        let mut squad = vec![
            player(1, PlayerPositionType::Goalkeeper, 120, 2000),
            player(2, PlayerPositionType::Striker, 120, 2000),
        ];
        refresh(&mut squad);

        let keeper = view(&squad, 1);
        assert_eq!(keeper.pecking_rank, 1);
        assert_eq!(keeper.rivals_at_position, 0);
        assert_eq!(keeper.top_rival_id, 0);
        assert_eq!(keeper.rival_gap, 0);
    }

    #[test]
    fn the_rivals_age_travels_with_him() {
        let mut squad = vec![
            player(1, PlayerPositionType::Striker, 150, 1996),
            player(2, PlayerPositionType::Striker, 110, 2010),
        ];
        refresh(&mut squad);

        // A boy of twenty behind a man of thirty-four is in a different
        // situation from one behind a peer, and the rule that reads this
        // needs the age to tell them apart.
        assert_eq!(view(&squad, 2).top_rival_age, 34);
    }

    #[test]
    fn the_ranking_is_stable_between_identical_ticks() {
        let mut squad = vec![
            player(1, PlayerPositionType::Striker, 120, 2000),
            player(2, PlayerPositionType::Striker, 120, 2000),
        ];
        refresh(&mut squad);
        let first = (view(&squad, 1).pecking_rank, view(&squad, 2).pecking_rank);
        refresh(&mut squad);
        let second = (view(&squad, 1).pecking_rank, view(&squad, 2).pecking_rank);

        assert_eq!(
            first, second,
            "a rank that flickers reads as losing and winning his place every week"
        );
    }

    #[test]
    fn wage_standing_is_neutral_when_it_is_unknown() {
        let unranked = SquadStandingView::default();
        assert_eq!(
            unranked.wage_standing(),
            0.5,
            "an unranked squad is no view, not poverty"
        );
    }

    #[test]
    fn wage_standing_runs_from_the_top_earner_down() {
        let top = SquadStandingView {
            wage_rank: 1,
            squad_size: 21,
            ..Default::default()
        };
        let bottom = SquadStandingView {
            wage_rank: 21,
            squad_size: 21,
            ..Default::default()
        };
        assert_eq!(top.wage_standing(), 1.0);
        assert_eq!(bottom.wage_standing(), 0.0);
    }
}
