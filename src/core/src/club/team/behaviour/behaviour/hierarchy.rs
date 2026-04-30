//! Dressing-room hierarchy effects: senior-leader influence on
//! youngsters, controversial-star clique tension with the squad's
//! professionals, and a captain-mediation modifier that respects the
//! relationship the conflicted parties have with the armband.
//!
//! Leadership and captain pieces sit in `leadership.rs`; this module
//! adds the slower social-status drifts that build between teammates
//! around the captain, rather than acts the captain directly performs.

use super::TeamBehaviour;
use crate::club::team::behaviour::{
    PlayerRelationshipChangeResult, TeamBehaviourResult,
};
use crate::context::GlobalContext;
use crate::utils::DateUtils;
use crate::{ChangeType, Player, PlayerCollection};

impl TeamBehaviour {
    /// Senior leaders with high professionalism quietly raise the
    /// standards around them; a controversial high-reputation player
    /// sets a different tone, fragmenting the squad along
    /// professional/permissive lines. Both effects are small per week
    /// and confined to the relevant subgroup.
    pub(super) fn process_dressing_room_hierarchy(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        let date = ctx.simulation.date.date();
        let count = players.players.len();

        for i in 0..count {
            let leader = &players.players[i];
            let leader_age = DateUtils::age(leader.birth_date, date);
            let is_senior_leader = leader_age >= 28
                && leader.skills.mental.leadership >= 14.0
                && leader.attributes.professionalism >= 14.0;
            let is_controversial_star = leader.attributes.controversy >= 14.0
                && leader.player_attributes.world_reputation >= 5000;

            if !is_senior_leader && !is_controversial_star {
                continue;
            }

            for j in 0..count {
                if i == j {
                    continue;
                }
                let other = &players.players[j];
                let other_age = DateUtils::age(other.birth_date, date);

                let other_to_leader_rel = other
                    .relations
                    .get_player(leader.id)
                    .map(|r| r.level)
                    .unwrap_or(0.0);

                if is_senior_leader && other_age <= 22 {
                    let mut lift = senior_leader_lift(leader, other);
                    // Halve further positive drift once the youngster
                    // already idolises the senior — the bond is what
                    // it is, more weeks don't make it stronger.
                    if other_to_leader_rel >= 60.0 {
                        lift *= 0.5;
                    }
                    if lift > 0.0 {
                        // The young player gravitates toward the senior:
                        // direction is youth→leader, mirroring how
                        // mentorship admiration propagates.
                        result.players.relationship_result.push(
                            PlayerRelationshipChangeResult {
                                from_player_id: other.id,
                                to_player_id: leader.id,
                                relationship_change: lift,
                                change_type: ChangeType::MentorshipBond,
                            },
                        );
                    }
                }

                if is_controversial_star
                    && other.attributes.professionalism >= 14.0
                    && other.attributes.controversy <= 10.0
                {
                    // Personality friction should ride harder when the
                    // star is currently misbehaving (recent controversy
                    // events) — not just because of static personality
                    // numbers. A high-controversy player who has been
                    // quiet lately doesn't need a weekly drag.
                    let star_currently_poor = leader.behaviour.is_poor();
                    let mut drag = controversial_clique_tension(leader, other);
                    if !star_currently_poor {
                        drag *= 0.5;
                    }
                    // Saturation taper — keep -50 a meaningful floor
                    // rather than a stop on the way to -100.
                    if other_to_leader_rel <= -50.0 {
                        drag *= 0.5;
                    }
                    if drag < 0.0 {
                        // The professional teammate distances themselves
                        // from the controversial star, not the other way
                        // round — the star feels untouchable, the pro
                        // feels embarrassed by association.
                        result.players.relationship_result.push(
                            PlayerRelationshipChangeResult {
                                from_player_id: other.id,
                                to_player_id: leader.id,
                                relationship_change: drag,
                                change_type: ChangeType::ReputationTension,
                            },
                        );
                    }
                }
            }
        }
    }
}

fn senior_leader_lift(leader: &Player, youngster: &Player) -> f32 {
    let leadership = leader.skills.mental.leadership;
    let professionalism = leader.attributes.professionalism;
    let receptiveness = (youngster.attributes.adaptability / 20.0).max(0.4);

    let base = (leadership - 14.0) * 0.012 + (professionalism - 14.0) * 0.008;
    let lift = base.max(0.0) + 0.03;
    (lift * receptiveness).clamp(0.03, 0.10)
}

fn controversial_clique_tension(star: &Player, pro: &Player) -> f32 {
    let controversy = star.attributes.controversy;
    let star_temperament = star.attributes.temperament;
    let pro_professionalism = pro.attributes.professionalism;

    // Bigger controversy + lower temperament + stronger pro reaction
    // → bigger split. Cap as specified.
    let base = (controversy - 14.0) * 0.02
        + (20.0 - star_temperament).max(0.0) * 0.005
        + (pro_professionalism - 14.0) * 0.006;
    let drag = -(base + 0.04).max(0.04);
    drag.clamp(-0.12, -0.04)
}
