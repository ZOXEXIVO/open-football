//! Weekly manager-trust drift driven by football context: does the
//! coach's quality match what the player's stature expects of them, is
//! the player getting the role their squad status implies, are the
//! manager's promises being kept. Effects are deliberately small —
//! most weekly deltas land in the -0.05..+0.10 range — so individual
//! talk outcomes and performance reactions remain the dominant movers.
//!
//! Distinct from `manager_talks.rs`: that file fires episodic
//! conversations and applies their immediate fallout. This module
//! shifts the *background* relationship every week regardless of
//! whether a talk happened, so a quietly disrespected key player
//! erodes trust over months even without any explicit confrontation.

use super::TeamBehaviour;
use crate::club::team::behaviour::TeamBehaviourResult;
use crate::context::GlobalContext;
use crate::utils::DateUtils;
use crate::{
    ChangeType, Player, PlayerCollection, PlayerSquadStatus, RelationshipChange,
    StaffCollection, StaffPosition,
};

impl TeamBehaviour {
    /// Adjust each non-loanee player's relationship with the head
    /// manager by a small, context-driven amount. Updates the staff
    /// relation, the coach_credibility / manager_relationship happiness
    /// factors, and rapport — the four state stores that downstream
    /// systems (talks, training, morale) read to decide how the player
    /// engages with the coach.
    pub(super) fn process_manager_relationship_context(
        players: &mut PlayerCollection,
        staffs: &StaffCollection,
        _result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        let manager = match staffs.find_by_position(StaffPosition::Manager) {
            Some(m) => m,
            None => return,
        };
        let manager_id = manager.id;
        let date = ctx.simulation.date.date();

        // Coach quality 0..1 — weighted toward people-skills (the things
        // a player actually feels week to week) over hard tactics.
        let mental = &manager.staff_attributes.mental;
        let knowledge = &manager.staff_attributes.knowledge;
        let coaching = &manager.staff_attributes.coaching;
        let coach_quality = ((mental.man_management as f32 * 0.30
            + mental.motivating as f32 * 0.20
            + mental.discipline as f32 * 0.15
            + knowledge.tactical_knowledge as f32 * 0.20
            + coaching.mental as f32 * 0.10
            + coaching.technical as f32 * 0.05)
            / 20.0)
            .clamp(0.0, 1.0);

        for player in players.iter_mut() {
            // Loanees report to their parent club's bond — the
            // borrower-side coach is a temporary stop, so background
            // drift would muddy the parent relation when they return.
            if player.is_on_loan() {
                continue;
            }

            // What the player's stature implies they should expect.
            // Combines CA (raw playing level) with reputation (squad
            // pecking-order) so a high-CA youngster doesn't expect the
            // same coach as a 9000-rep veteran. Capped at 1.0 to match
            // the coach_quality ceiling — a perfect-attribute coach
            // satisfies even an elite player. The previous 1.25 cap
            // guaranteed a permanent negative gap (and weekly trust
            // erosion) for top players regardless of who managed them.
            let player_expectation = (player.player_attributes.current_ability as f32 / 160.0
                + player.player_attributes.current_reputation as f32 / 12000.0)
                .clamp(0.35, 1.0);
            let credibility_gap = coach_quality - player_expectation;

            // Personality reception — how readily the player listens to
            // the coach regardless of *what* is being said. High values
            // are pros, loyalists, calm types, adaptable types, and
            // low-controversy players. Each term normalised 0..1, all
            // weights sum to 1.0 so the result stays in 0..1.
            let attrs = &player.attributes;
            let personality_reception = (attrs.professionalism / 20.0 * 0.25
                + attrs.loyalty / 20.0 * 0.20
                + attrs.temperament / 20.0 * 0.20
                + attrs.adaptability / 20.0 * 0.15
                + (20.0 - attrs.controversy) / 20.0 * 0.20)
                .clamp(0.0, 1.0);

            // Role pressure — how far the player's actual deployment
            // diverges from what their squad status implies. Football-aware:
            // an injured / banned / recovering player isn't being snubbed
            // tactically (they're not available), and starter_ratio /
            // appearances_tracked give a far truer picture of "am I getting
            // starts?" than days-since-any-match alone.
            let age = DateUtils::age(player.birth_date, date);
            let role_pressure = compute_role_pressure(player, age);

            // Promise track-record. promise_trust is roughly -10..+6
            // (kept promises lift, broken ones erode). Asymmetric
            // clamp mirrors that — broken promises hit harder than
            // kept ones reward.
            let promise_modifier =
                (player.happiness.factors.promise_trust / 15.0).clamp(-0.35, 0.25);

            let trust_delta = ((credibility_gap * 0.18
                + promise_modifier * 0.10
                + role_pressure * 0.10)
                * personality_reception)
                .clamp(-0.18, 0.18);

            if trust_delta.abs() < 0.005 {
                continue;
            }

            let change = if trust_delta >= 0.0 {
                RelationshipChange::positive(
                    ChangeType::CoachingSuccess,
                    trust_delta.abs(),
                )
            } else {
                RelationshipChange::negative(
                    ChangeType::TacticalDisagreement,
                    trust_delta.abs(),
                )
            };
            player
                .relations
                .update_staff_relationship(manager_id, change, date);

            // coach_credibility happiness factor — target-based
            // smoothing toward credibility_gap * 6.0 so a long-running
            // mismatch settles at a stable level instead of drifting
            // monotonically week after week. Daily happiness recalc may
            // still overwrite this from `calculate_coach_credibility`;
            // the smoothing keeps the team-behaviour layer from
            // saturating the factor in the days between recalcs.
            let credibility_target = (credibility_gap * 6.0).clamp(-8.0, 6.0);
            let credibility_current = player.happiness.factors.coach_credibility;
            player.happiness.factors.coach_credibility =
                (credibility_current + (credibility_target - credibility_current) * 0.20)
                    .clamp(-8.0, 6.0);

            // manager_relationship factor — target-based smoothing too,
            // sized so a sustained trust_delta of ±0.18 lands the target
            // around the upper third of the ±15 clamp. Prevents weekly
            // accumulation from saturating before the daily recalc runs.
            let mgr_target = (trust_delta * 30.0).clamp(-15.0, 15.0);
            let mgr_current = player.happiness.factors.manager_relationship;
            player.happiness.factors.manager_relationship =
                (mgr_current + (mgr_target - mgr_current) * 0.20).clamp(-15.0, 15.0);

            // Rapport — convert the trust delta into a small rapport
            // tick. on_negative carries the asymmetric ramp built into
            // PlayerRapport, so calling it directly is the right thing
            // for negative deltas rather than passing a negative value.
            let amount = (trust_delta * 20.0).round() as i16;
            if amount > 0 {
                player.rapport.on_positive(manager_id, date, amount);
            } else if amount < 0 {
                player.rapport.on_negative(manager_id, date, amount.abs());
            }
        }
    }
}

/// Football-aware role-pressure signal. Returns a small negative number
/// when the player's actual minutes/starts fall short of what their
/// squad status implies, a small positive number for prospects who are
/// getting role clarity, and 0 in every other case. Skips the pressure
/// entirely when the player is unavailable (injured, banned, in recovery)
/// — they aren't being snubbed, they're just out of action.
fn compute_role_pressure(player: &Player, age: u8) -> f32 {
    if player.player_attributes.is_injured
        || player.player_attributes.is_banned
        || player.player_attributes.is_in_recovery()
    {
        return 0.0;
    }

    let squad_status = match player.contract.as_ref() {
        Some(c) => &c.squad_status,
        None => return 0.0,
    };

    let days = player.player_attributes.days_since_last_match;
    let starter_ratio = player.happiness.starter_ratio;
    let apps_tracked = player.happiness.appearances_tracked;

    match squad_status {
        // Key player wants starts. Penalise only after either
        //   * 5+ tracked apps with a starter_ratio below 0.45 (they
        //     have a sample size and they're not starting), or
        //   * a 3-week gap since *any* match (clear benching).
        PlayerSquadStatus::KeyPlayer => {
            let bad_starts = apps_tracked >= 5 && starter_ratio < 0.45;
            if bad_starts || days > 21 {
                -0.35
            } else {
                0.0
            }
        }
        // First-team regular tolerates rotation; complains only if
        // starter_ratio falls below 0.30 or the gap exceeds 24 days.
        PlayerSquadStatus::FirstTeamRegular => {
            let bad_starts = apps_tracked >= 5 && starter_ratio < 0.30;
            if bad_starts || days > 24 {
                -0.25
            } else {
                0.0
            }
        }
        // Rotation player: pressure only when they're not seeing the
        // pitch at all — a month without an appearance.
        PlayerSquadStatus::FirstTeamSquadRotation => {
            if days > 30 {
                -0.12
            } else {
                0.0
            }
        }
        // Prospects benefit from role clarity: a positive drift only
        // when they're young, recently active, and the starter_ratio
        // is climbing (proxy: ≥0.20).
        PlayerSquadStatus::HotProspectForTheFuture
        | PlayerSquadStatus::DecentYoungster => {
            if age <= 21
                && days < 30
                && (apps_tracked >= 1 || starter_ratio >= 0.20)
            {
                0.04
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        ContractType, PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills,
    };
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn pro_personality() -> PersonAttributes {
        PersonAttributes {
            adaptability: 14.0,
            ambition: 14.0,
            controversy: 5.0,
            loyalty: 14.0,
            pressure: 12.0,
            professionalism: 16.0,
            sportsmanship: 14.0,
            temperament: 14.0,
            consistency: 12.0,
            important_matches: 12.0,
            dirtiness: 5.0,
        }
    }

    fn skills() -> PlayerSkills {
        PlayerSkills::default()
    }

    fn build_player_with_status(
        ca: u8,
        rep: i16,
        squad_status: PlayerSquadStatus,
        birth_date: NaiveDate,
    ) -> Player {
        let mut player_attrs = PlayerAttributes::default();
        player_attrs.current_ability = ca;
        player_attrs.current_reputation = rep;
        player_attrs.world_reputation = rep;

        let mut contract = PlayerClubContract::new(50_000, d(2030, 6, 1));
        contract.squad_status = squad_status;
        contract.contract_type = ContractType::FullTime;

        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(birth_date)
            .country_id(1)
            .attributes(pro_personality())
            .skills(skills())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(player_attrs)
            .contract(Some(contract))
            .build()
            .unwrap()
    }

    #[test]
    fn elite_player_with_perfect_coach_no_longer_drifts_negative() {
        // Replicates the bug where the old 1.25 expectation cap meant a
        // CA-180 / rep-9000 player perpetually saw a negative
        // credibility_gap even with the best possible manager.
        let player = build_player_with_status(180, 9000, PlayerSquadStatus::KeyPlayer, d(1995, 1, 1));
        let player_expectation = (player.player_attributes.current_ability as f32 / 160.0
            + player.player_attributes.current_reputation as f32 / 12000.0)
            .clamp(0.35, 1.0);

        // Best possible coach quality is 1.0; new cap means gap is >= 0
        // for an elite player rather than the legacy -0.25.
        let coach_quality: f32 = 1.0;
        let credibility_gap = coach_quality - player_expectation;
        assert!(
            credibility_gap >= -0.001,
            "elite player expectation should not exceed perfect coach quality (gap = {})",
            credibility_gap
        );
    }

    #[test]
    fn injured_key_player_has_no_role_pressure_loss() {
        let mut player = build_player_with_status(
            150,
            5_000,
            PlayerSquadStatus::KeyPlayer,
            d(1995, 1, 1),
        );
        player.player_attributes.is_injured = true;
        player.player_attributes.injury_days_remaining = 21;
        player.player_attributes.days_since_last_match = 30;
        let role_pressure = compute_role_pressure(&player, 30);
        assert_eq!(role_pressure, 0.0, "injured key player should not feel role pressure");
    }

    #[test]
    fn key_player_starting_regularly_does_not_lose_trust_to_role_pressure() {
        let mut player = build_player_with_status(
            150,
            5_000,
            PlayerSquadStatus::KeyPlayer,
            d(1995, 1, 1),
        );
        player.happiness.starter_ratio = 0.85;
        player.happiness.appearances_tracked = 10;
        player.player_attributes.days_since_last_match = 4;
        assert_eq!(compute_role_pressure(&player, 30), 0.0);
    }

    #[test]
    fn key_player_benched_with_low_starter_ratio_feels_role_pressure() {
        let mut player = build_player_with_status(
            150,
            5_000,
            PlayerSquadStatus::KeyPlayer,
            d(1995, 1, 1),
        );
        player.happiness.starter_ratio = 0.20;
        player.happiness.appearances_tracked = 8;
        player.player_attributes.days_since_last_match = 6;
        assert!(compute_role_pressure(&player, 30) < 0.0);
    }

    #[test]
    fn prospect_with_no_minutes_gets_no_positive_drift() {
        // Old code gave +0.05 just for being young with days_since < 30.
        // Now we require either appearances_tracked > 0 or a non-trivial
        // starter_ratio.
        let mut player = build_player_with_status(
            90,
            500,
            PlayerSquadStatus::HotProspectForTheFuture,
            d(2007, 1, 1),
        );
        player.happiness.starter_ratio = 0.0;
        player.happiness.appearances_tracked = 0;
        player.player_attributes.days_since_last_match = 12;
        assert_eq!(compute_role_pressure(&player, 19), 0.0);
    }

    #[test]
    fn prospect_with_recent_appearances_gets_positive_drift() {
        let mut player = build_player_with_status(
            90,
            500,
            PlayerSquadStatus::HotProspectForTheFuture,
            d(2007, 1, 1),
        );
        player.happiness.starter_ratio = 0.30;
        player.happiness.appearances_tracked = 3;
        player.player_attributes.days_since_last_match = 8;
        assert!(compute_role_pressure(&player, 19) > 0.0);
    }
}
