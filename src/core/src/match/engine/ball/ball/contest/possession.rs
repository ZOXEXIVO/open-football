use crate::r#match::engine::ball::ball::{ASSIST_WINDOW_TICKS, Ball};
use crate::r#match::engine::ball::events::BallEvent;
use crate::r#match::events::EventCollection;
#[cfg(feature = "match-logs")]
use crate::r#match::player::strategies::passing::CrossType;
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::CrossDiag;

/// How the current ball carrier came by the ball.
///
/// Stamped at the event-dispatch choke point (every acquisition emits
/// exactly one ball event), so it stays correct without threading a
/// reason through the ~20 sites that assign `current_owner`. Read at
/// shot time by `shot_supply_diag`: in real football roughly 55-60% of
/// shots are struck by the player who was just passed to, and this is
/// the counter that says whether the engine feeds its shooters or lets
/// them scavenge.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PossessionSource {
    /// No acquisition recorded since the last restart.
    Unknown,
    /// Received a teammate's pass — the one that should dominate.
    PassReception,
    /// Won an uncontrolled ball: rebound, spill, deflection, failed
    /// first touch, or a clearance that dropped to them.
    LooseBall,
    /// Picked off an opponent's pass.
    Interception,
    /// Took it off an opponent in a challenge.
    Tackle,
}

impl PossessionSource {
    pub const COUNT: usize = 5;

    pub fn index(self) -> usize {
        match self {
            PossessionSource::Unknown => 0,
            PossessionSource::PassReception => 1,
            PossessionSource::LooseBall => 2,
            PossessionSource::Interception => 3,
            PossessionSource::Tackle => 4,
        }
    }

    pub const NAMES: [&'static str; Self::COUNT] =
        ["unknown", "pass", "loose", "intercept", "tackle"];
}

/// One kick in the current possession's pass chain.
///
/// The chain used to be a bare `VecDeque<u32>` of player ids, which is
/// enough for the AI heuristics that read it (one-two detection, the
/// "don't pass straight back" recency penalty) but not for crediting an
/// assist. An assist has to answer three questions a lone id cannot:
/// is the passer a TEAMMATE of the scorer, was the pass in the SAME
/// possession phase, and was it RECENT. Carrying the team and the tick
/// on every entry answers all three at the point of use.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PassChainEntry {
    pub player_id: u32,
    pub team_id: u32,
    pub tick: u64,
}

impl Ball {
    /// Snapshot the most-recent completed pass so the shot-handler
    /// key-pass linker can credit the passer when the receiver
    /// shoots within the key-pass window. Called from
    /// `credit_completed_pass` *before* `clear_pending_pass_metadata`
    /// nulls out the live pass envelope.
    #[inline]
    pub fn record_completed_pass(&mut self, passer_id: u32, receiver_id: u32, tick: u64) {
        self.last_completed_pass_passer_id = Some(passer_id);
        self.last_completed_pass_receiver_id = Some(receiver_id);
        self.last_completed_pass_tick = tick;
    }

    pub fn clear_player_reference(&mut self, player_id: u32) {
        if self.current_owner == Some(player_id) {
            self.current_owner = None;
            self.ownership_duration = 0;
            // A substituted / sent-off keeper cannot still be holding it.
            self.held_in_hands = false;
        }
        if self.previous_owner == Some(player_id) {
            self.previous_owner = None;
        }
        if self.pass_target_player_id == Some(player_id) {
            self.pass_target_player_id = None;
        }
        if self.last_release_player_id == Some(player_id) {
            self.last_release_player_id = None;
        }
        if self.last_completed_pass_passer_id == Some(player_id)
            || self.last_completed_pass_receiver_id == Some(player_id)
        {
            self.last_completed_pass_passer_id = None;
            self.last_completed_pass_receiver_id = None;
        }
        self.take_ball_notified_players
            .retain(|&id| id != player_id);
        self.recent_passers.retain(|e| e.player_id != player_id);
    }

    /// Record a passer in the recent passers ring buffer.
    /// Skips consecutive duplicates and caps at 5 entries.
    pub fn record_passer(&mut self, passer_id: u32, team_id: u32, tick: u64) {
        // Skip consecutive duplicates
        if self.recent_passers.back().map(|e| e.player_id) == Some(passer_id) {
            return;
        }
        if self.recent_passers.len() >= 5 {
            self.recent_passers.pop_front();
        }
        self.recent_passers.push_back(PassChainEntry {
            player_id: passer_id,
            team_id,
            tick,
        });
    }

    /// The teammate whose pass should be credited with an assist for a
    /// goal scored by `scorer_id` of `scorer_team_id` at `tick`, if any.
    ///
    /// Walks the chain newest-first and applies the three rules a real
    /// assist obeys:
    ///
    ///  1. **Same team.** The credited player must be a teammate of the
    ///     scorer. Without this the resolver happily handed the assist to
    ///     the goalkeeper whose goal kick got turned over — measured at
    ///     71% of all assists, 63% of them to keepers.
    ///  2. **Same possession.** Stop at the first opponent entry. A pass
    ///     made before the other team had the ball belongs to an earlier
    ///     phase of play, not to this goal.
    ///  3. **Recent.** The pass has to have led to the goal, so it must
    ///     land inside `ASSIST_WINDOW_TICKS`. This is what stops a goal
    ///     kick from being an "assist" for a solo run half a minute later.
    pub fn assist_for_goal(&self, scorer_id: u32, scorer_team_id: u32, tick: u64) -> Option<u32> {
        #[cfg(feature = "match-logs")]
        use std::sync::atomic::Ordering;
        #[cfg(feature = "match-logs")]
        crate::r#match::engine::ball::ball::assist_diag::GOALS.fetch_add(1, Ordering::Relaxed);

        for entry in self.recent_passers.iter().rev() {
            // Rule 2: an opponent touched the chain — earlier entries
            // belong to a possession that is not this one.
            if entry.team_id != scorer_team_id {
                #[cfg(feature = "match-logs")]
                {
                    crate::r#match::engine::ball::ball::assist_diag::OPPONENT_CHAIN
                        .fetch_add(1, Ordering::Relaxed);
                    crate::r#match::engine::ball::ball::assist_diag::OPPONENT_CHAIN_AGE
                        .fetch_add(tick.saturating_sub(entry.tick), Ordering::Relaxed);
                    if self
                        .recent_passers
                        .iter()
                        .any(|e| e.team_id == scorer_team_id && e.player_id != scorer_id)
                    {
                        crate::r#match::engine::ball::ball::assist_diag::OPPONENT_CHAIN_HAS_TEAMMATE.fetch_add(1, Ordering::Relaxed);
                    }
                }
                return None;
            }
            if entry.player_id == scorer_id {
                continue;
            }
            // Rule 3: `tick` is monotonic within a match, but stay
            // defensive about the ordering anyway.
            let delay = tick.saturating_sub(entry.tick);
            if delay > ASSIST_WINDOW_TICKS {
                #[cfg(feature = "match-logs")]
                crate::r#match::engine::ball::ball::assist_diag::STALE
                    .fetch_add(1, Ordering::Relaxed);
                return None;
            }
            #[cfg(feature = "match-logs")]
            {
                crate::r#match::engine::ball::ball::assist_diag::CREDITED
                    .fetch_add(1, Ordering::Relaxed);
                crate::r#match::engine::ball::ball::assist_diag::CREDITED_DELAY_TICKS
                    .fetch_add(delay, Ordering::Relaxed);
            }
            return Some(entry.player_id);
        }
        #[cfg(feature = "match-logs")]
        {
            if self.recent_passers.is_empty() {
                crate::r#match::engine::ball::ball::assist_diag::EMPTY_CHAIN
                    .fetch_add(1, Ordering::Relaxed);
            } else {
                crate::r#match::engine::ball::ball::assist_diag::SCORER_ONLY
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        None
    }

    /// Clear the recent passers history (e.g. on tackles, interceptions, clearances).
    pub fn clear_pass_history(&mut self) {
        self.recent_passers.clear();
    }

    /// Label how `player_id` came by the ball.
    ///
    /// Ignores repeat events for a player who already has it: `Claimed`
    /// fires to re-affirm existing ownership as well as to acquire, so
    /// without this guard a receiver's `PassReception` was relabelled
    /// `LooseBall` a second later while the ball was still at his feet —
    /// which read as 97% of shots coming from loose balls.
    /// For the same carrier only a MORE SPECIFIC label may overwrite: a
    /// repeat `Claimed` must not downgrade a reception to a loose ball,
    /// but the pass-completion credit that lands just after a bare
    /// `Claimed` (a teammate other than the intended target collected
    /// it) must be allowed to upgrade it.
    pub fn note_possession_source(&mut self, player_id: u32, source: PossessionSource) {
        if self.possession_source_for == Some(player_id) && source == PossessionSource::LooseBall {
            return;
        }
        self.possession_source_for = Some(player_id);
        self.possession_source = source;
    }

    /// Note that `team_id` now has the ball, dropping the pass chain only
    /// if the ball genuinely changed hands.
    ///
    /// The recovery paths (loose ball gained, ball headed clear, tackle)
    /// all used to wipe the chain unconditionally. But a loose ball won
    /// by a TEAMMATE is the same attacking phase: a cross flicked on at
    /// the near post, a rebound off a block, a knock-down in the box. The
    /// cross that started the move is still the assist if the move ends
    /// in a goal, and wiping it left the resolver with nothing to credit
    /// on roughly a third of all goals (`crate::r#match::engine::ball::ball::assist_diag::EMPTY_CHAIN`).
    ///
    /// Only a change of TEAM ends the phase.
    pub fn note_possession(&mut self, team_id: u32) {
        if self.recent_passers.back().map(|e| e.team_id) != Some(team_id) {
            self.recent_passers.clear();
        }
    }

    /// Clear the pass-window metadata used by the pass-completion classifier
    /// and the key-pass linker. Called whenever the live pass is no longer
    /// in flight (claim, interception, expiry, set-piece restart).
    #[inline]
    pub fn clear_pending_pass_metadata(&mut self) {
        // A lofted delivery being disarmed here never reached the aerial
        // contest. Record the height it died at — that says whether it
        // was cut out on the way up, at head height, or after landing,
        // and those are three different bugs.
        #[cfg(feature = "match-logs")]
        if !self.cross_contest_resolved && self.pending_cross_type.is_some_and(CrossType::is_lofted)
        {
            CrossDiag::note_disarmed_at(self.position.z);
        }
        self.pending_pass_passer = None;
        self.pending_pass_origin = None;
        self.pending_pass_target = None;
        self.pending_pass_was_cross = false;
        self.pending_cross_type = None;
        // Disarm the aerial contest with the delivery it belonged to — a
        // cross that has been claimed, cleared or intercepted is over.
        self.cross_contest_resolved = true;
    }

    /// Drop any in-flight shot metadata (xG / shooter id). Called once
    /// the shot resolves (save / goal / wide / over / opponent claim).
    #[inline]
    pub fn clear_shot_metadata(&mut self) {
        self.last_shot_xgot = 0.0;
        self.last_shot_shooter_id = None;
        // A dead ball ends the shot: without this a stale strike would
        // let the next pass that rolls over the line stand as a goal.
        self.last_shot_struck_tick = 0;
    }

    /// Stamp the giveaway tracker for the player who just lost the ball
    /// via a misplaced pass / lost tackle / dispossession. Subsequent
    /// shot / goal events from the opposing team within the response
    /// window will be charged back as an error to this player. The
    /// `was_own_box` flag is read later by the goal handler to layer the
    /// own-box-extra penalty on top of `errors_leading_to_goal`.
    #[inline]
    pub fn stamp_giveaway(&mut self, player_id: u32, team_id: u32, tick: u64, was_own_box: bool) {
        self.last_giveaway_player_id = Some(player_id);
        self.last_giveaway_team_id = Some(team_id);
        self.last_giveaway_tick = tick;
        self.last_giveaway_was_own_box = was_own_box;
    }

    /// Drop the giveaway tracker — the response window has expired or
    /// the giver's team has recovered the ball.
    #[inline]
    pub fn clear_giveaway(&mut self) {
        self.last_giveaway_player_id = None;
        self.last_giveaway_team_id = None;
        self.last_giveaway_was_own_box = false;
    }

    /// Detect and resolve carry transitions. Called once per tick from
    /// `update` / `update_light`, after `process_ownership` has settled
    /// the current owner. When the owner changes (or goes None) we emit
    /// a `BallEvent::CarryEnded` for the previous carrier; the
    /// dispatcher classifies the carry and credits the carrier's stats.
    /// A new carry starts the moment ownership lands on a player.
    pub fn tick_carry_tracker(&mut self, events: &mut EventCollection) {
        match (self.carry_owner, self.current_owner) {
            (Some(prev), Some(curr)) if prev == curr => {
                // Same carrier — nothing to emit.
            }
            (Some(prev), _) => {
                // Carry ended (owner changed or went None).
                events.add_ball_event(BallEvent::CarryEnded(
                    prev,
                    self.carry_start_position,
                    self.position,
                ));
                self.carry_owner = self.current_owner;
                self.carry_start_position = self.position;
            }
            (None, Some(curr)) => {
                // Carry begins.
                self.carry_owner = Some(curr);
                self.carry_start_position = self.position;
            }
            (None, None) => {}
        }
    }
}

#[cfg(test)]
mod completed_pass_tests {
    use super::*;
    use nalgebra::Vector3;

    #[test]
    fn record_completed_pass_populates_snapshot() {
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.record_completed_pass(7, 11, 1234);
        assert_eq!(ball.last_completed_pass_passer_id, Some(7));
        assert_eq!(ball.last_completed_pass_receiver_id, Some(11));
        assert_eq!(ball.last_completed_pass_tick, 1234);
    }

    #[test]
    fn clear_pending_pass_metadata_does_not_clear_completed_snapshot() {
        // Regression: the centralized completion path used to clear
        // pending_pass_passer immediately, leaving the shot-handler
        // key-pass linker without a passer to credit. The completed
        // snapshot survives the pending clear.
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.pending_pass_passer = Some(7);
        ball.pending_pass_set_tick = 100;
        ball.pending_pass_origin = Some(Vector3::new(50.0, 100.0, 0.0));
        ball.pending_pass_target = Some(Vector3::new(150.0, 100.0, 0.0));
        ball.pending_pass_was_cross = true;
        ball.record_completed_pass(7, 11, 200);
        ball.clear_pending_pass_metadata();
        assert!(ball.pending_pass_passer.is_none());
        assert!(ball.pending_pass_origin.is_none());
        assert!(ball.pending_pass_target.is_none());
        assert!(!ball.pending_pass_was_cross);
        // The completed snapshot stays — the key-pass linker reads it.
        assert_eq!(ball.last_completed_pass_passer_id, Some(7));
        assert_eq!(ball.last_completed_pass_receiver_id, Some(11));
        assert_eq!(ball.last_completed_pass_tick, 200);
    }

    #[test]
    fn clear_player_reference_drops_completed_pass_snapshot() {
        // If a player is removed (red card, sub), any completed-pass
        // metadata referencing them must be cleared so the next shot
        // doesn't credit a phantom key pass.
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.record_completed_pass(7, 11, 200);
        ball.clear_player_reference(7);
        assert!(ball.last_completed_pass_passer_id.is_none());
        assert!(ball.last_completed_pass_receiver_id.is_none());

        // Receiver removal also wipes (consistency).
        ball.record_completed_pass(7, 11, 300);
        ball.clear_player_reference(11);
        assert!(ball.last_completed_pass_passer_id.is_none());
        assert!(ball.last_completed_pass_receiver_id.is_none());
    }
}

#[cfg(test)]
mod assist_tests {
    use super::*;

    const HOME: u32 = 1;
    const AWAY: u32 = 2;

    fn ball() -> Ball {
        Ball::with_coord(840.0, 545.0)
    }

    #[test]
    fn credits_the_teammate_who_played_the_last_pass() {
        let mut ball = ball();
        ball.record_passer(7, HOME, 1000);
        ball.record_passer(9, HOME, 1200);
        assert_eq!(ball.assist_for_goal(10, HOME, 1300), Some(9));
    }

    #[test]
    fn never_credits_an_opponent() {
        // The headline bug: an away keeper's goal kick sat in the ring,
        // the home team turned it over and scored, and the resolver
        // handed the keeper an assist for the goal he conceded. Across a
        // season that put goalkeepers at the top of the assist charts.
        let mut ball = ball();
        ball.record_passer(200, AWAY, 1000); // away GK's goal kick
        assert_eq!(ball.assist_for_goal(10, HOME, 1200), None);
    }

    #[test]
    fn stops_at_a_possession_break() {
        // Home passed, the away team had it and passed too, then home
        // won it back and scored without a pass. The earlier home pass
        // belongs to a different phase of play — no assist.
        let mut ball = ball();
        ball.record_passer(7, HOME, 800);
        ball.record_passer(200, AWAY, 1000);
        assert_eq!(ball.assist_for_goal(10, HOME, 1100), None);
    }

    #[test]
    fn skips_the_scorer_but_keeps_walking_back() {
        // Give-and-go: 7 passes, gets it back, scores. The assist is the
        // teammate who returned it, not 7 himself.
        let mut ball = ball();
        ball.record_passer(9, HOME, 1000);
        ball.record_passer(7, HOME, 1100);
        ball.record_passer(9, HOME, 1200);
        assert_eq!(ball.assist_for_goal(7, HOME, 1250), Some(9));
    }

    #[test]
    fn a_chain_holding_only_the_scorer_yields_nothing() {
        let mut ball = ball();
        ball.record_passer(7, HOME, 1000);
        assert_eq!(ball.assist_for_goal(7, HOME, 1100), None);
    }

    #[test]
    fn a_stale_pass_is_not_an_assist() {
        // A goal kick is not the assist for a solo run that ends half a
        // minute later, however unbroken the possession was.
        let mut ball = ball();
        ball.record_passer(1, HOME, 1000);
        let late = 1000 + ASSIST_WINDOW_TICKS + 1;
        assert_eq!(ball.assist_for_goal(10, HOME, late), None);
        // One tick inside the window still counts.
        assert_eq!(
            ball.assist_for_goal(10, HOME, 1000 + ASSIST_WINDOW_TICKS),
            Some(1)
        );
    }

    #[test]
    fn empty_chain_yields_nothing() {
        assert_eq!(ball().assist_for_goal(10, HOME, 500), None);
    }

    #[test]
    fn possession_survives_a_teammate_winning_a_loose_ball() {
        // A cross flicked on, a rebound off a block, a knock-down in the
        // box — same attacking phase, so the cross is still the assist.
        let mut ball = ball();
        ball.record_passer(2, HOME, 1000);
        ball.note_possession(HOME);
        assert_eq!(ball.assist_for_goal(9, HOME, 1150), Some(2));
    }

    #[test]
    fn possession_drops_the_chain_when_the_ball_changes_hands() {
        let mut ball = ball();
        ball.record_passer(2, HOME, 1000);
        ball.note_possession(AWAY);
        assert!(ball.recent_passers.is_empty());
    }

    #[test]
    fn chain_entries_carry_team_and_tick() {
        let mut ball = ball();
        ball.record_passer(7, HOME, 1000);
        // Consecutive duplicates are still collapsed.
        ball.record_passer(7, HOME, 1050);
        assert_eq!(ball.recent_passers.len(), 1);
        let entry = ball.recent_passers.back().unwrap();
        assert_eq!(entry.player_id, 7);
        assert_eq!(entry.team_id, HOME);
        assert_eq!(entry.tick, 1000);
    }

    #[test]
    fn ring_caps_at_five_and_drops_the_oldest() {
        let mut ball = ball();
        for i in 0..7u32 {
            ball.record_passer(i, HOME, 1000 + i as u64);
        }
        assert_eq!(ball.recent_passers.len(), 5);
        assert_eq!(ball.recent_passers.front().unwrap().player_id, 2);
        assert_eq!(ball.recent_passers.back().unwrap().player_id, 6);
    }
}
