use crate::r#match::engine::player::events::players::PlayerEventDispatcher;
use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{MatchContext, MatchField, PlayerSide};
use log::debug;
use nalgebra::Vector3;

#[derive(Copy, Clone, Debug)]
pub enum BallEvent {
    Goal(BallGoalEventMetadata),
    Claimed(u32),
    /// Pass reached its intended target: (receiver_id, passer_id).
    /// Emitted by `try_pass_target_claim` so pass-completion stats
    /// can be credited exactly once per successful pass.
    PassCompleted(u32, u32),
    /// Pass intercepted by opponent: (interceptor_id, passer_id)
    Intercepted(u32, Option<u32>),
    /// Shot blocked by an outfielder: `(blocker_id, ball_position)`.
    /// Emitted by `Ball::try_block_shot` whenever a block resolves
    /// (irrespective of the deflection outcome — controlled,
    /// corner-bound, safe, loose, unlucky). Distinct from
    /// `Intercepted` so block credit cannot leak into an unrelated
    /// pass interception that happens to share the same tick.
    Blocked(u32, Vector3<f32>),
    Gained(u32),
    TakeMe(u32),
    /// Offside resolved on receiver involvement: (receiver_id,
    /// free_kick_position). Translates to PlayerEvent::Offside in the
    /// dispatcher so the player-event pipeline owns ball-stop / free-
    /// kick award.
    Offside(u32, Vector3<f32>),
    /// Carry concluded: `(carrier_id, start_position, end_position)`.
    /// Emitted by `Ball::tick_carry_tracker` when ownership changes
    /// hands. The dispatcher classifies the carry as progressive,
    /// box-entry, or none and credits the carrier's stats.
    CarryEnded(u32, Vector3<f32>, Vector3<f32>),
}

#[derive(Copy, Clone, Debug, PartialOrd, PartialEq)]
pub enum GoalSide {
    Home,
    Away,
}

#[derive(Copy, Clone, Debug)]
pub struct BallGoalEventMetadata {
    pub side: GoalSide,
    pub goalscorer_player_id: u32,
    pub assist_player_id: Option<u32>,
    pub auto_goal: bool,
}

pub struct BallEventDispatcher;

impl BallEventDispatcher {
    pub fn dispatch(
        event: BallEvent,
        field: &mut MatchField,
        context: &MatchContext,
    ) -> Vec<Event> {
        let mut remaining_events = Vec::new();

        if context.logging_enabled {
            match event {
                BallEvent::TakeMe(_) | BallEvent::Claimed(_) => {}
                BallEvent::Intercepted(pid, _) => {
                    debug!("Ball event: Intercepted by player {}", pid);
                }
                _ => debug!("Ball event: {:?}", event),
            }
        }

        match event {
            BallEvent::Goal(metadata) => {
                // Determine which team scored based on the goalscorer's team, not goal position.
                // Goal position (GoalSide) is unreliable after halftime side swap.
                if let Some(scorer) = field
                    .players
                    .iter()
                    .find(|p| p.id == metadata.goalscorer_player_id)
                {
                    let is_home_scorer = scorer.team_id == context.score.home_team.team_id;

                    if metadata.auto_goal {
                        // Own goal — credit the opposing team
                        if is_home_scorer {
                            context.score.increment_away_goals();
                        } else {
                            context.score.increment_home_goals();
                        }
                    } else {
                        // Normal goal — credit the scorer's team
                        if is_home_scorer {
                            context.score.increment_home_goals();
                        } else {
                            context.score.increment_away_goals();
                        }
                    }
                }

                remaining_events.push(Event::PlayerEvent(PlayerEvent::Goal(
                    metadata.goalscorer_player_id,
                    metadata.auto_goal,
                )));

                if let Some(assist_id) = metadata.assist_player_id {
                    remaining_events.push(Event::PlayerEvent(PlayerEvent::Assist(assist_id)));
                }

                field.reset_players_positions();
            }
            BallEvent::Claimed(player_id) => {
                remaining_events.push(Event::PlayerEvent(PlayerEvent::ClaimBall(player_id)));
            }
            BallEvent::PassCompleted(receiver_id, passer_id) => {
                // Single completion path — `credit_completed_pass`
                // increments `passes_completed`, classifies progressive
                // / box-entry / cross-completed, and clears the
                // pending-pass metadata. The downstream ClaimBall
                // handler sees an empty pass window and won't double-
                // credit.
                PlayerEventDispatcher::credit_completed_pass(
                    receiver_id,
                    passer_id,
                    field,
                    context,
                );
                remaining_events.push(Event::PlayerEvent(PlayerEvent::ClaimBall(receiver_id)));
            }
            BallEvent::Intercepted(interceptor_id, passer_id) => {
                // Credit the interceptor. Opponent touch ends the pass
                // window — accuracy was NOT earned.
                let ball_pos = field.ball.position;
                let pending_passer = field.ball.pending_pass_passer;
                field.ball.clear_pending_pass_metadata();

                // Stamp the giveaway tracker only for genuine pass
                // interceptions (the ball was on a live pass when the
                // opponent picked it off). Shot-block interceptions
                // (try_block_shot also fires Intercepted) don't have a
                // pending pass and shouldn't charge the shooter as
                // having "given the ball away".
                if let Some(passer) = pending_passer {
                    let giver_meta = field.get_player(passer).map(|p| {
                        (
                            p.team_id,
                            PlayerEventDispatcher::zone_for_player(p, ball_pos, context),
                        )
                    });
                    if let Some((team, zone)) = giver_meta {
                        let was_own_box = zone.map_or(false, |z| z.is_own_box());
                        let was_dangerous_zone =
                            zone.map_or(false, |z| z.is_own_box() || z.is_own_third());
                        field.ball.stamp_giveaway(
                            passer,
                            team,
                            context.current_tick(),
                            was_own_box,
                        );
                        // Note the dangerous turnover on the giver's
                        // stats so the rating helper can dock the
                        // own-third / own-box penalty even if no shot
                        // converts within the response window.
                        if was_dangerous_zone {
                            if let (Some(zone), Some(giver)) = (zone, field.get_player_mut(passer))
                            {
                                giver.statistics.note_dangerous_turnover(zone);
                            }
                        }
                    }
                    // Successful pressure: opponents who were within
                    // the pressing radius at pass-emit time get
                    // promoted from raw `pressures` to
                    // `successful_pressures` because their close
                    // presence forced the turnover. Final-third wins
                    // also tag the press-zone counter.
                    let press_count = field.ball.pressers_at_pass_count as usize;
                    let pressers = field.ball.pressers_at_pass;
                    for &pid in pressers.iter().take(press_count) {
                        if let Some(presser) = field.get_player_mut(pid) {
                            presser.statistics.add_successful_pressure();
                            if let Some(zone) =
                                PlayerEventDispatcher::zone_for_player(presser, ball_pos, context)
                            {
                                presser.statistics.note_pressure_won_zone(zone);
                            }
                        }
                    }
                } else if let Some(prev_id) = passer_id {
                    let _ = prev_id; // shot-block path; no giveaway stamp
                }
                // Pressure snapshot consumed — clear so a later
                // unrelated interception doesn't reuse it.
                field.ball.pressers_at_pass_count = 0;

                if let Some(player) = field.get_player_mut(interceptor_id) {
                    player.statistics.interceptions += 1;
                    if let Some(zone) =
                        PlayerEventDispatcher::zone_for_player(player, ball_pos, context)
                    {
                        player.statistics.note_interception_zone(zone);
                    }
                }
                remaining_events.push(Event::PlayerEvent(PlayerEvent::ClaimBall(interceptor_id)));
            }
            BallEvent::Blocked(blocker_id, position) => {
                if let Some(player) = field.get_player_mut(blocker_id) {
                    player.statistics.add_block();
                    if let Some(zone) =
                        PlayerEventDispatcher::zone_for_player(player, position, context)
                    {
                        player.statistics.note_block_zone(zone);
                    }
                }
            }
            BallEvent::Gained(player_id) => {
                remaining_events.push(Event::PlayerEvent(PlayerEvent::GainBall(player_id)));
            }
            BallEvent::TakeMe(player_id) => {
                remaining_events.push(Event::PlayerEvent(PlayerEvent::TakeBall(player_id)));
            }
            BallEvent::Offside(receiver_id, position) => {
                field.ball.clear_pending_pass_metadata();
                remaining_events.push(Event::PlayerEvent(PlayerEvent::Offside(
                    receiver_id,
                    position,
                )));
            }
            BallEvent::CarryEnded(carrier_id, start, end) => {
                Self::credit_carry(carrier_id, start, end, field, context);
            }
        }

        remaining_events
    }

    /// Classify a concluded carry and credit progressive_carries /
    /// carries-into-box / carry_distance on the carrier's stats.
    /// Also credits `successful_dribbles` for opponents the carrier
    /// physically ran past during the carry (within the lateral
    /// pressure cone, between start and end), provided possession
    /// stayed with the carrier's team — a carry ending in a tackle
    /// is classified upstream as a failed dribble and must not also
    /// fire a successful one here.
    fn credit_carry(
        carrier_id: u32,
        start: Vector3<f32>,
        end: Vector3<f32>,
        field: &mut MatchField,
        context: &MatchContext,
    ) {
        let (side, carrier_team_id) = match field.get_player(carrier_id) {
            Some(p) => match p.side {
                Some(s) => (s, p.team_id),
                None => return,
            },
            None => return,
        };
        let field_w = context.field_size.width as f32;
        let forward_progress = side.forward_delta(start.x, end.x);
        if forward_progress <= 0.0 {
            return;
        }
        let end_in_final_third = side.attacking_progress_x(end.x, field_w) >= 2.0 / 3.0;
        let start_in_final_third = side.attacking_progress_x(start.x, field_w) >= 2.0 / 3.0;
        // Progressive carry threshold: ≥25u outside final third, ≥12u inside.
        let progressive_threshold = if start_in_final_third { 12.0 } else { 25.0 };
        let is_progressive = forward_progress >= progressive_threshold;

        let is_home = side == PlayerSide::Left;
        let opp_box = context.penalty_area(!is_home);
        let started_outside_box = !opp_box.contains(&start);
        let ended_in_box = opp_box.contains(&end);

        // Carry ended via opponent dispossession? If the new ball owner
        // is on the opposing team, the tackle handler already credited
        // a failed dribble — don't double-count it as a beat here.
        let new_owner_team = field
            .ball
            .current_owner
            .and_then(|id| field.get_player(id))
            .map(|p| p.team_id);
        let dispossessed_by_opponent = new_owner_team
            .map(|nt| nt != carrier_team_id)
            .unwrap_or(false);

        // Successful-dribble producer: count opponents who were on the
        // carry line between start and end (carrier physically ran past
        // their pressure cone). Defenders sitting right at the start or
        // right at the end are excluded — neither was "beaten".
        let beaten_count = if !dispossessed_by_opponent && forward_progress >= 12.0 {
            Self::count_beaten_on_carry_path(
                start,
                end,
                field
                    .players
                    .iter()
                    .filter(|p| p.team_id != carrier_team_id)
                    .map(|p| p.position),
            )
        } else {
            0
        };

        if let Some(carrier) = field.get_player_mut(carrier_id) {
            carrier.statistics.carry_distance = carrier
                .statistics
                .carry_distance
                .saturating_add(forward_progress as u32);
            if is_progressive {
                carrier.statistics.progressive_carries =
                    carrier.statistics.progressive_carries.saturating_add(1);
                if end_in_final_third && !start_in_final_third {
                    carrier.statistics.note_progressive_carry_into_final_third();
                }
            }
            if started_outside_box && ended_in_box {
                carrier.statistics.note_carry_into_box();
            }
            for _ in 0..beaten_count {
                carrier.statistics.add_successful_dribble();
            }
        }
    }

    /// Count opponents the carrier physically ran past on this carry.
    /// Geometry-only: an opponent is "beaten" if their CURRENT position
    /// projects onto the carry line between start and end (window
    /// `[3u .. carry_len - 4u]`) and is within a 5u lateral pressure
    /// cone of that line. Approximate — opponents move during the carry
    /// too — but a deterministic stat-line signal that low-HQ ball
    /// carriers who can't beat anyone will lack and elite ball-carriers
    /// will accumulate. Capped at 3 per single carry: nobody beats 4+
    /// defenders in one run.
    fn count_beaten_on_carry_path(
        start: Vector3<f32>,
        end: Vector3<f32>,
        opponent_positions: impl Iterator<Item = Vector3<f32>>,
    ) -> u16 {
        let carry_vec = end - start;
        let carry_len = carry_vec.magnitude();
        if carry_len < 10.0 {
            return 0;
        }
        let carry_dir = carry_vec / carry_len;
        let mut beaten: u16 = 0;
        for opp_pos in opponent_positions {
            let to_opp = opp_pos - start;
            let along = to_opp.dot(&carry_dir);
            if along < 3.0 || along > carry_len - 4.0 {
                continue;
            }
            let proj = start + carry_dir * along;
            let perpendicular = (opp_pos - proj).magnitude();
            if perpendicular < 5.0 {
                beaten = beaten.saturating_add(1);
            }
        }
        beaten.min(3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32) -> Vector3<f32> {
        Vector3::new(x, y, 0.0)
    }

    #[test]
    fn beaten_returns_zero_for_short_carry() {
        // Less than 10u carry → no successful-dribble credit, even if
        // opponents litter the path. Short carries aren't dribbles.
        let opps = [v(50.0, 100.0), v(55.0, 100.0)];
        let r = BallEventDispatcher::count_beaten_on_carry_path(
            v(48.0, 100.0),
            v(54.0, 100.0),
            opps.iter().copied(),
        );
        assert_eq!(r, 0);
    }

    #[test]
    fn beaten_counts_opponent_on_carry_line() {
        // 30u carry from (10,100) to (40,100). An opponent at (25,102)
        // is on the line, between the cutoffs, within 5u lateral → one
        // beaten defender.
        let opps = [v(25.0, 102.0)];
        let r = BallEventDispatcher::count_beaten_on_carry_path(
            v(10.0, 100.0),
            v(40.0, 100.0),
            opps.iter().copied(),
        );
        assert_eq!(r, 1);
    }

    #[test]
    fn beaten_excludes_opponent_outside_carry_window() {
        // Opponent right at the start (along < 3) — not beaten.
        let opps = [v(11.0, 100.0)];
        let r = BallEventDispatcher::count_beaten_on_carry_path(
            v(10.0, 100.0),
            v(40.0, 100.0),
            opps.iter().copied(),
        );
        assert_eq!(r, 0);
        // Opponent right at the end (along > carry_len - 4) — also
        // not beaten (they arrived, didn't get past).
        let opps = [v(38.0, 100.0)];
        let r = BallEventDispatcher::count_beaten_on_carry_path(
            v(10.0, 100.0),
            v(40.0, 100.0),
            opps.iter().copied(),
        );
        assert_eq!(r, 0);
    }

    #[test]
    fn beaten_excludes_opponent_outside_lateral_cone() {
        // Opponent on the line direction but 8u to the side — not on
        // the pressure cone, didn't have to be beaten.
        let opps = [v(25.0, 110.0)];
        let r = BallEventDispatcher::count_beaten_on_carry_path(
            v(10.0, 100.0),
            v(40.0, 100.0),
            opps.iter().copied(),
        );
        assert_eq!(r, 0);
    }

    #[test]
    fn beaten_caps_at_three() {
        // Five opponents on the carry line — cap at 3.
        let opps = [
            v(15.0, 100.0),
            v(20.0, 101.0),
            v(25.0, 99.0),
            v(30.0, 100.0),
            v(33.0, 102.0),
        ];
        let r = BallEventDispatcher::count_beaten_on_carry_path(
            v(10.0, 100.0),
            v(40.0, 100.0),
            opps.iter().copied(),
        );
        assert_eq!(r, 3);
    }

    #[test]
    fn beaten_handles_diagonal_carry() {
        // 30u diagonal carry from (10,100) to (40,130). An opponent at
        // (25,115) lies on the diagonal (midpoint) and counts.
        let opps = [v(25.0, 115.0)];
        let r = BallEventDispatcher::count_beaten_on_carry_path(
            v(10.0, 100.0),
            v(40.0, 130.0),
            opps.iter().copied(),
        );
        assert_eq!(r, 1);
    }
}
