use super::*;

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
        context: &MatchContext,
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
        field
            .players
            .iter_mut()
            .filter(|player| !player.is_sent_off)
            .for_each(|player| player.update(context, tick_context, events));
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
            .filter(|player| !player.is_sent_off)
            .filter(|player| {
                player.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
            })
            .for_each(|player| player.update(context, tick_context, events));
    }
}
