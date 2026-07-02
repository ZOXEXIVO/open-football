use super::*;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::PlayerState;

/// Reduced-cadence (LOD) distance gate: players further than this from
/// the ball — in a passive movement state, while the ball is owned —
/// run their full AI every OTHER full tick instead of every full tick.
/// 250 units ≈ 31 m: far enough that a skipped decision costs ~20 ms of
/// reaction latency on a player who is out of the play entirely.
const LOD_DISTANCE_SQ: f32 = 250.0 * 250.0;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    // ───────────────────────────────────────────────────────────────────────
    // Position recording
    // ───────────────────────────────────────────────────────────────────────

    /// Record positions every 30ms (every 3rd tick) instead of every 10ms.
    pub(super) const POSITION_RECORD_INTERVAL_MS: u64 = 30;

    #[inline]
    pub fn write_match_positions(
        field: &mut MatchField,
        timestamp: u64,
        match_data: &mut ResultMatchPositionData,
    ) {
        if !match_data.is_tracking_positions() {
            return;
        }

        if timestamp % Self::POSITION_RECORD_INTERVAL_MS != 0 {
            return;
        }

        let track_events = match_data.is_tracking_events();

        // Don't record sent-off players — their state doesn't advance and
        // their position is a dummy off-pitch stash. A recorded sample
        // would show them as a ghost sprite in the replay viewer.
        field.players.iter().filter(|p| !p.is_sent_off).for_each(|player| {
            // Diagnostic: catch players pinned at ANY field boundary.
            // `check_boundary_collision` clamps to 0..=field_width and
            // 0..=field_height; a steering error that consistently
            // points off-pitch leaves the player stuck there.
            // Rate-limit to once per 30s of match time per player so a
            // persistent stall doesn't spam the log every 30ms sample.
            let field_w = field.size.width as f32;
            let field_h = field.size.height as f32;
            let near_left = player.position.x < 1.0;
            let near_right = player.position.x > field_w - 1.0;
            let near_top = player.position.y < 1.0;
            let near_bottom = player.position.y > field_h - 1.0;
            if (near_left || near_right) && (near_top || near_bottom)
                && timestamp % 30_000 < Self::POSITION_RECORD_INTERVAL_MS
            {
                match_log_debug!(
                    "player at corner: t={}ms id={} team={} state={:?} tactical={:?} pos=({:.1}, {:.1}) velocity=({:.2}, {:.2})",
                    timestamp,
                    player.id,
                    player.team_id,
                    player.state,
                    player.tactical_position.current_position,
                    player.position.x,
                    player.position.y,
                    player.velocity.x,
                    player.velocity.y,
                );
            }
            match_data.add_player_positions(player.id, timestamp, player.position);
            if track_events {
                match_data.add_player_state(player.id, timestamp, player.state.compact_id(), &player.state);
            }
        });

        match_data.add_ball_positions(timestamp, field.ball.position);
    }

    // ───────────────────────────────────────────────────────────────────────
    // Ball & player dispatchers
    // ───────────────────────────────────────────────────────────────────────

    pub(super) fn play_ball(
        field: &mut MatchField,
        context: &mut MatchContext,
        tick_context: &GameTickContext,
        events: &mut EventCollection,
    ) {
        field
            .ball
            .update(context, &field.players, tick_context, events);
    }

    pub(super) fn play_players(
        field: &mut MatchField,
        context: &mut MatchContext,
        tick_context: &GameTickContext,
        events: &mut EventCollection,
    ) {
        let ball_owned = tick_context.ball.is_owned;
        let ball_owner = tick_context.ball.current_owner;
        let ball_pos = tick_context.positions.ball.position;
        // Stagger phase flips every full tick (full ticks land on every
        // other engine tick, so `>> 1` advances once per full tick).
        // Half the LOD-eligible players update on even phases, half on
        // odd — the whole far side never skips the same tick.
        let stagger = (context.current_tick() >> 1) & 1;

        field
            .players
            .iter_mut()
            .enumerate()
            .filter(|(_, player)| !player.is_sent_off)
            .for_each(|(idx, player)| {
                // Reduced AI cadence for players far away from an OWNED
                // ball in passive shape-keeping states. The skipped tick
                // is a light-tick-style move (previous velocity carries)
                // with fatigue / state timers / cooldowns still running
                // at full rate — see `MatchPlayer::lod_skip_update`.
                // A loose ball disables the gate entirely: chase
                // designation and interception windows need every
                // player sharp.
                if ball_owned
                    && ball_owner != Some(player.id)
                    && Self::lod_reduced_cadence_eligible(player.state)
                    && (player.position - ball_pos).norm_squared() > LOD_DISTANCE_SQ
                    && (idx as u64 & 1) == stagger
                {
                    player.lod_skip_update(context);
                    return;
                }
                player.update(idx, context, tick_context, events)
            });
    }

    /// Passive shape-keeping states where a far-from-ball player's
    /// decision can safely wait one extra full tick (~20 ms). Action
    /// states (shooting, passing, tackling, heading, take-ball, every
    /// goalkeeper state, …) always run at full cadence, as does any
    /// state not explicitly listed here.
    fn lod_reduced_cadence_eligible(state: PlayerState) -> bool {
        match state {
            PlayerState::Defender(s) => matches!(
                s,
                DefenderState::Standing
                    | DefenderState::Walking
                    | DefenderState::Running
                    | DefenderState::Returning
                    | DefenderState::Resting
                    | DefenderState::HoldingLine
            ),
            PlayerState::Midfielder(s) => matches!(
                s,
                MidfielderState::Standing
                    | MidfielderState::Walking
                    | MidfielderState::Running
                    | MidfielderState::Returning
                    | MidfielderState::Resting
            ),
            PlayerState::Forward(s) => matches!(
                s,
                ForwardState::Standing
                    | ForwardState::Walking
                    | ForwardState::Running
                    | ForwardState::Returning
                    | ForwardState::Resting
            ),
            _ => false,
        }
    }

    /// Run the AI for *only* the goalkeepers this tick. Used during
    /// shot flight on light ticks: the 50% AI cadence across all 22
    /// players was fine for normal play, but during a ~10 tick shot
    /// window the GK missed half their decisions and never closed on
    /// the intercept. This fills in those ticks without re-evaluating
    /// 20 outfielders for zero behavioural gain.
    pub(super) fn play_goalkeepers(
        field: &mut MatchField,
        context: &mut MatchContext,
        tick_context: &GameTickContext,
        events: &mut EventCollection,
    ) {
        field
            .players
            .iter_mut()
            .enumerate()
            .filter(|(_, player)| !player.is_sent_off)
            .filter(|(_, player)| {
                player.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
            })
            .for_each(|(idx, player)| player.update(idx, context, tick_context, events));
    }
}
