//! Captain / leadership-driven passes: dressing-room mediation, captain
//! morale propagation, leader-to-teammate influence on relationships,
//! and the playing-time jealousy sweep (which sits here because it's a
//! status-of-the-pecking-order signal).

use super::TeamBehaviour;
use crate::club::team::behaviour::{PlayerRelationshipChangeResult, TeamBehaviourResult};
use crate::{ChangeType, Player, PlayerCollection, PlayerSquadStatus, PlayerStatusType};
use std::cmp::Ordering;

impl TeamBehaviour {
    /// Mediation quality bar: the peacemaker needs genuine dressing-room
    /// authority and a professional's touch, whoever wears the armband.
    fn qualifies_as_mediator(p: &Player) -> bool {
        p.skills.mental.leadership >= 12.0 && p.attributes.professionalism >= 13.0
    }

    /// The player who acts as "the captain" for a behaviour pass: the
    /// official club captain when he's in the squad and clears `bar`,
    /// then the official vice, then — only when the club hierarchy gives
    /// no qualifying figure — the best qualifying leader by a fully
    /// deterministic score (compound score, then lower id, so the pick
    /// never depends on roster order).
    fn acting_captain<'p>(
        players: &'p PlayerCollection,
        official_captain: Option<u32>,
        official_vice: Option<u32>,
        bar: fn(&Player) -> bool,
    ) -> Option<&'p Player> {
        let official = |id: Option<u32>| {
            id.and_then(|id| players.players.iter().find(|p| p.id == id))
                .filter(|p| bar(p))
        };

        official(official_captain)
            .or_else(|| official(official_vice))
            .or_else(|| {
                players
                    .players
                    .iter()
                    .filter(|p| bar(p))
                    .max_by(|a, b| {
                        let sa = a.skills.mental.leadership * 1.0
                            + a.attributes.professionalism * 0.6
                            + a.attributes.loyalty * 0.4;
                        let sb = b.skills.mental.leadership * 1.0
                            + b.attributes.professionalism * 0.6
                            + b.attributes.loyalty * 0.4;
                        sa.partial_cmp(&sb)
                            .unwrap_or(Ordering::Equal)
                            .then_with(|| b.id.cmp(&a.id))
                    })
            })
    }

    /// A respected captain mediates dressing-room conflicts. For each
    /// pair of teammates whose relationship has crossed below a friction
    /// threshold, the captain's leadership + professionalism is converted
    /// into a small healing nudge applied to both directions of the
    /// pair. A weak / controversial captain does nothing here.
    ///
    /// The mediator is the *official* club captain (then vice) when the
    /// appointed leader clears the quality bar — the same player the club
    /// page and morale events point at — with a senior-leader fallback
    /// only when the hierarchy offers no qualifying figure.
    ///
    /// Sits alongside the morale-spread captain pass — that one moves
    /// captain mood, this one moves teammate-to-teammate relationships.
    pub(super) fn process_captain_mediation(
        players: &PlayerCollection,
        official_captain: Option<u32>,
        official_vice: Option<u32>,
        result: &mut TeamBehaviourResult,
    ) {
        let captain = Self::acting_captain(
            players,
            official_captain,
            official_vice,
            Self::qualifies_as_mediator,
        );

        let Some(captain) = captain else { return };
        let captain_id = captain.id;

        // Mediation strength: 0..1, scales the per-pair healing nudge.
        let leadership = captain.skills.mental.leadership;
        let prof = captain.attributes.professionalism;
        let temperament = captain.attributes.temperament;
        let strength =
            ((leadership / 20.0) * 0.5 + (prof / 20.0) * 0.3 + (temperament / 20.0) * 0.2)
                .clamp(0.0, 1.0);

        if strength < 0.4 {
            return;
        }

        // Find broken pairs (relationship level <= -25) and emit a
        // small symmetric positive nudge. Cap to a few pairs per week
        // so a single captain doesn't carpet-bomb the whole squad.
        const MAX_MEDIATIONS: usize = 4;
        let mut emitted = 0;
        'outer: for i in 0..players.players.len() {
            let a = &players.players[i];
            if a.id == captain_id {
                continue;
            }
            for j in (i + 1)..players.players.len() {
                if emitted >= MAX_MEDIATIONS {
                    break 'outer;
                }
                let b = &players.players[j];
                if b.id == captain_id {
                    continue;
                }
                let level_ab = a.relations.get_player(b.id).map(|r| r.level).unwrap_or(0.0);
                if level_ab > -25.0 {
                    continue;
                }
                // Mediation effectiveness depends on how each party
                // feels about the captain. If either of them dislikes
                // the captain (relation level <= -20), the intervention
                // lands with half the force; if both already respect
                // the captain (level >= 30) it lands a quarter harder.
                let a_to_cap = a
                    .relations
                    .get_player(captain_id)
                    .map(|r| r.level)
                    .unwrap_or(0.0);
                let b_to_cap = b
                    .relations
                    .get_player(captain_id)
                    .map(|r| r.level)
                    .unwrap_or(0.0);
                let captain_relation_mult = if a_to_cap <= -20.0 || b_to_cap <= -20.0 {
                    0.5
                } else if a_to_cap >= 30.0 && b_to_cap >= 30.0 {
                    1.25
                } else {
                    1.0
                };
                // Healing nudge sized by mediation strength and how
                // bad the relationship is (worse → more visible
                // intervention, but still small per week).
                let intensity = ((-level_ab - 25.0) / 75.0).clamp(0.0, 1.0);
                let nudge =
                    ((strength * 0.4 + intensity * 0.2) * captain_relation_mult).clamp(0.0, 0.6);
                if nudge < 0.05 {
                    continue;
                }
                result
                    .players
                    .relationship_result
                    .push(PlayerRelationshipChangeResult {
                        from_player_id: a.id,
                        to_player_id: b.id,
                        relationship_change: nudge,
                        change_type: ChangeType::PersonalSupport,
                    });
                result
                    .players
                    .relationship_result
                    .push(PlayerRelationshipChangeResult {
                        from_player_id: b.id,
                        to_player_id: a.id,
                        relationship_change: nudge,
                        change_type: ChangeType::PersonalSupport,
                    });
                emitted += 1;
            }
        }
    }

    /// The captain's mood leaks out to teammates: ~±2 morale points/week
    /// based on how happy the captain is relative to neutral 50. Sits on
    /// top of the existing `process_leadership_influence` pass (which only
    /// moves relationship numbers, not morale).
    ///
    /// The propagating voice is the *official* club captain (then vice)
    /// where one clears the leadership bar — the room orbits the armband
    /// the club actually handed out — with the compound-score fallback
    /// reserved for squads whose hierarchy offers no qualifying figure.
    pub(super) fn process_captain_morale_propagation(
        players: &mut PlayerCollection,
        official_captain: Option<u32>,
        official_vice: Option<u32>,
    ) {
        // Don't let anyone with <10 leadership propagate — a weak captain
        // holds no sway over the room's mood either way.
        let captain_id_opt = Self::acting_captain(players, official_captain, official_vice, |p| {
            p.skills.mental.leadership >= 10.0
        })
        .map(|p| p.id);

        let captain_id = match captain_id_opt {
            Some(id) => id,
            None => return,
        };

        let captain_morale = match players.find(captain_id) {
            Some(c) => c.happiness.morale,
            None => return,
        };

        // Delta: captain at 50 morale → 0 effect. At 80 → +0.6, at 20 → -0.6.
        // Leadership scales the magnitude (a 20-leadership captain hits 2x
        // a 10-leadership captain).
        let captain_leadership = players
            .find(captain_id)
            .map(|c| c.skills.mental.leadership)
            .unwrap_or(10.0);
        let leadership_scale = (captain_leadership / 20.0).clamp(0.0, 1.0);
        let base_delta = (captain_morale - 50.0) * 0.02; // -1..1
        let delta = base_delta * leadership_scale; // -1..1 scaled

        if delta.abs() < 0.05 {
            return;
        }

        for player in players.players.iter_mut() {
            if player.id == captain_id {
                continue;
            }
            // Per-teammate sway — a captain someone respects lands harder,
            // someone the squad mistrusts lands weaker (or slightly
            // negative on a good-mood captain). Maps level [-100,100] →
            // multiplier [0.3, 1.5].
            let relation_mult = player
                .relations
                .get_player(captain_id)
                .map(|r| 0.9 + (r.level / 100.0).clamp(-0.6, 0.6))
                .unwrap_or(1.0);
            player.happiness.adjust_morale(delta * relation_mult);
        }
    }

    /// A formally unhappy influential player drags the room — one
    /// sulking star is a problem, a sulking leader is a crisis. Weekly
    /// leadership- and status-weighted mood ripple from every `Unh`
    /// player with real dressing-room weight onto his teammates,
    /// stronger on close friends. Deliberately bounded: per-source drag
    /// is small, the per-target total is capped, and the ripple reads
    /// the *status* (a durable grievance), not raw morale — so the
    /// loop gain stays well below 1 and low team mood alone can never
    /// snowball itself.
    pub(super) fn process_unhappy_star_contagion(players: &mut PlayerCollection) {
        struct ContagionSource {
            id: u32,
            influence: f32,
        }

        let sources: Vec<ContagionSource> = players
            .players
            .iter()
            .filter(|p| p.statuses.has(PlayerStatusType::Unh))
            .filter_map(|p| {
                let is_key_player = matches!(
                    p.contract.as_ref().map(|c| &c.squad_status),
                    Some(PlayerSquadStatus::KeyPlayer)
                );
                // Fringe players sulking barely register; the ripple is
                // about players the room actually orbits.
                if p.skills.mental.leadership < 12.0 && !is_key_player {
                    return None;
                }
                let leadership = (p.skills.mental.leadership / 20.0).clamp(0.0, 1.0);
                let status_weight = match p.contract.as_ref().map(|c| &c.squad_status) {
                    Some(PlayerSquadStatus::KeyPlayer) => 1.0,
                    Some(PlayerSquadStatus::FirstTeamRegular) => 0.7,
                    _ => 0.35,
                };
                Some(ContagionSource {
                    id: p.id,
                    influence: leadership * 0.6 + status_weight * 0.4,
                })
            })
            .collect();

        if sources.is_empty() {
            return;
        }

        const DRAG_PER_SOURCE: f32 = 0.5;
        const MAX_WEEKLY_DRAG: f32 = 1.2;

        let source_ids: Vec<u32> = sources.iter().map(|s| s.id).collect();
        for player in players.players.iter_mut() {
            if source_ids.contains(&player.id) {
                continue;
            }
            let mut drag = 0.0_f32;
            for source in &sources {
                // Close friends of the sulking star are dragged more;
                // teammates who barely know him shrug most of it off.
                let relation_mult = player
                    .relations
                    .get_player(source.id)
                    .map(|r| 0.9 + (r.level / 100.0).clamp(-0.6, 0.6))
                    .unwrap_or(0.8);
                drag += DRAG_PER_SOURCE * source.influence * relation_mult;
            }
            player.happiness.adjust_morale(-drag.min(MAX_WEEKLY_DRAG));
        }
    }

    /// High leadership players influence team morale and relationships
    pub(super) fn process_leadership_influence(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        let leaders: Vec<&Player> = players
            .players
            .iter()
            .filter(|p| p.skills.mental.leadership > 15.0)
            .collect();

        for leader in leaders {
            for player in &players.players {
                if leader.id == player.id {
                    continue;
                }

                let influence = Self::calculate_leadership_influence(leader, player);

                if influence.abs() > 0.01 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player.id,
                            to_player_id: leader.id,
                            relationship_change: influence,
                            change_type: ChangeType::MentorshipBond,
                        });
                }
            }
        }
    }

    /// Playing time jealousy
    pub(super) fn process_playing_time_jealousy(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let playing_time_i = player_i.statistics.played;
                let playing_time_j = player_j.statistics.played;

                let jealousy_factor = Self::calculate_playing_time_jealousy(
                    playing_time_i,
                    playing_time_j,
                    player_i,
                    player_j,
                );

                if jealousy_factor.abs() > 0.01 {
                    let change_type = if jealousy_factor > 0.0 {
                        ChangeType::TrainingBonding
                    } else {
                        ChangeType::CompetitionRivalry
                    };

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: jealousy_factor,
                            change_type: change_type.clone(),
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: jealousy_factor,
                            change_type,
                        });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TeamBehaviour;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, Player, PlayerAttributes, PlayerCollection, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills,
    };
    use chrono::NaiveDate;

    /// Test-fixture namespace: bare players with just enough identity for
    /// the leadership passes (id, leadership, morale).
    struct Fixture;

    impl Fixture {
        fn player(id: u32, leadership: f32, morale: f32) -> Player {
            let mut p = PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("T".to_string(), format!("P{}", id)))
                .birth_date(NaiveDate::from_ymd_opt(1995, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 20,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap();
            p.skills.mental.leadership = leadership;
            p.happiness.morale = morale;
            p
        }
    }

    /// The mood that spreads through the squad must be the *official*
    /// captain's — not whichever player happens to carry the highest raw
    /// leadership. Here the ad-hoc pick (id 1, leadership 18) is miserable
    /// while the appointed captain (id 2, leadership 12) is beaming; the
    /// bystander's morale must go UP.
    #[test]
    fn official_captain_mood_propagates_not_adhoc_leader() {
        let sulking_leader = Fixture::player(1, 18.0, 10.0);
        let official_captain = Fixture::player(2, 12.0, 90.0);
        let bystander = Fixture::player(3, 5.0, 50.0);

        let mut players =
            PlayerCollection::new(vec![sulking_leader, official_captain, bystander]);

        TeamBehaviour::process_captain_morale_propagation(&mut players, Some(2), None);

        let bystander_after = players.find(3).unwrap().happiness.morale;
        assert!(
            bystander_after > 50.0,
            "official captain's high mood should lift the room, got {}",
            bystander_after
        );
    }

    /// With no official hierarchy at all, the pass still works via the
    /// deterministic fallback pick (the strongest qualifying leader).
    #[test]
    fn fallback_leader_propagates_when_no_official_captain() {
        let leader = Fixture::player(1, 18.0, 90.0);
        let bystander = Fixture::player(2, 5.0, 50.0);

        let mut players = PlayerCollection::new(vec![leader, bystander]);

        TeamBehaviour::process_captain_morale_propagation(&mut players, None, None);

        let bystander_after = players.find(2).unwrap().happiness.morale;
        assert!(bystander_after > 50.0);
    }
}
