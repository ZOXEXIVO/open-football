//! **Crediting a save the physics already made.** `try_save_shot` in the
//! ball layer changes the ball's state mid-flight but cannot reach the
//! two players involved, so it leaves the pair on `Ball` and this drains
//! it — firing the keeper's save stat and the shooter's on-target stat,
//! exactly the events the GK state machine would have emitted had the
//! physics save not pre-empted it.

use crate::r#match::engine::engine::*;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    /// Consume `Ball::pending_save_credit` left behind by the physics
    /// save (`try_save_shot`). When the keeper actually changed ball
    /// state mid-flight (catch, safe parry, dangerous parry), this fires
    /// the save stat for the keeper and the on-target stat for the
    /// shooter — matching the events the GK state machine would have
    /// emitted if the physics save hadn't pre-empted it.
    pub(in crate::r#match::engine::engine) fn apply_pending_save_credit(field: &mut MatchField) {
        let Some((keeper_id, shooter_id)) = field.ball.pending_save_credit.take() else {
            return;
        };
        // One pass over the 22-player list resolves both ids. The team-
        // mismatch guard is defence in depth against any accidental
        // same-team shooter — deflections through the save handler
        // should already have been filtered upstream.
        let Some((keeper_idx, shooter_idx)) = field.two_player_indices(keeper_id, shooter_id)
        else {
            #[cfg(feature = "match-logs")]
            save_accounting_stats::PENDING_LOST_NO_PLAYER
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        };
        let keeper_team = field.players[keeper_idx].team_id;
        let shooter_team = field.players[shooter_idx].team_id;
        if keeper_team == shooter_team {
            #[cfg(feature = "match-logs")]
            save_accounting_stats::PENDING_LOST_SAME_TEAM
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        let shot_xg = field.ball.last_shot_xgot;
        // ── Make the save VISIBLE ─────────────────────────────────────
        //
        // The physics save resolves a shot entirely inside ball physics:
        // it changes ball state, credits the stats, and never touches the
        // keeper's state machine. So he made ~86 saves a match while
        // `Goalkeeper: Diving` sat below 0.25% of his ticks — the ball
        // simply stopped at a standing man, which is the "he doesn't
        // catch anything, he just sits on it" report.
        //
        // Put him into the state the save actually demanded. `reach_ratio`
        // is how far he had to stretch, and it is already the quantity the
        // save model scores, so the state and the physics agree about how
        // hard the save was rather than rolling it twice.
        {
            /// Beyond this he has left his feet. 0.22 of full stretch is
            /// roughly a step and a reach — anything past that is a dive,
            /// which is most saves a keeper makes. Only the ball hit
            /// straight at him is taken standing.
            const DIVE_STRETCH: f32 = 0.22;
            let reach = field.ball.pending_save_reach;
            let held = field.ball.current_owner == Some(keeper_id);
            let next = if reach >= DIVE_STRETCH {
                // Full-stretch — he goes to ground whether he holds it or
                // pushes it away.
                GoalkeeperState::Diving
            } else if held {
                // Straight at him and gathered: a clean catch.
                GoalkeeperState::Catching
            } else {
                // Straight at him and NOT held — he got something behind
                // it and the rebound is live. That is a parry, and
                // `Punching` is the state for it; `PreparingForSave`
                // would say he is still waiting for a shot he has already
                // stopped.
                GoalkeeperState::Punching
            };
            // …UNLESS HE IS ALREADY DOING IT.
            //
            // A keeper who left his feet during the flight (see
            // `KeeperShotDive`) is ALREADY in `Diving` when the ball
            // reaches him, and `transition_to` RESETS `in_state_time` — so
            // re-issuing the same state here restarted his dive timer at
            // the moment of contact and pinned him to the floor for another
            // full dive on top of the one he had just made. It also
            // double-counted him in the action census, which is how the
            // dive count came out at more than twice the number of saves.
            //
            // This site exists to make an INVISIBLE save visible. When the
            // save is already visible it has nothing to add, and the
            // crediting below carries on exactly as before.
            let already = field.players[keeper_idx].state == PlayerState::Goalkeeper(next);
            if !already {
                #[cfg(feature = "match-logs")]
                {
                    crate::mid_run_diag::KeeperSweepDiag::note_exit(match next {
                        GoalkeeperState::Diving => 1,
                        GoalkeeperState::Catching => 3,
                        _ => 4,
                    });
                    // …and into the action census as well. This site does
                    // not go through `PlayerMatchState::process`, so the
                    // counters there never see it — leaving the physics
                    // save, which is where most of a keeper's dives come
                    // from, out of the one table that reports how often he
                    // dives.
                    crate::mid_run_diag::KeeperActionDiag::note(match next {
                        GoalkeeperState::Diving => 0,
                        GoalkeeperState::Punching => 2,
                        _ => usize::MAX,
                    });
                }
                let gk = &mut field.players[keeper_idx];
                gk.transition_to(
                    PlayerState::Goalkeeper(next),
                    TransitionSource::EventHandler,
                );
            }
        }
        field.ball.pending_save_reach = 0.0;
        // Read the outcome BEFORE resetting it — the accounting block at the
        // bottom of this function needs it, and resetting here first is why
        // the table kept reporting `parry 0` while the parry branch was
        // demonstrably firing 3662 times per 200 matches.
        let save_site = field.ball.pending_save_site;
        field.ball.pending_save_site = 1;
        let _ = save_site; // only read by the `match-logs` accounting below
        {
            let gk = &mut field.players[keeper_idx];
            // The GK denied a shot worth `shot_xg` xG — books the save,
            // the shot faced, and both xG ledgers in one call so they
            // cannot drift apart (see `note_shot_faced`).
            gk.statistics.note_shot_faced(shot_xg, true);
        }
        field.players[shooter_idx].memory.credit_shot_on_target();
        // Shot has resolved (saved). Drop the metadata so any
        // subsequent goal / save event can't double-credit.
        field.ball.clear_shot_metadata();
        field.ball.pending_error_to_shot_player_id = None;
        #[cfg(feature = "match-logs")]
        {
            use std::sync::atomic::Ordering;
            // Book it under the outcome the physics actually produced —
            // catch, or either flavour of parry. This used to hard-code the
            // "catch" bucket because the outcome wasn't carried across, so
            // the table read `parry 0` and looked like parried shots were
            // never credited at all. See `Ball::pending_save_site`.
            let site = (save_site as usize).min(save_accounting_stats::SITE_LABELS.len() - 1);
            save_accounting_stats::SAVES_CREDITED[site].fetch_add(1, Ordering::Relaxed);
            save_accounting_stats::ON_TARGET_PAIRED[site].fetch_add(1, Ordering::Relaxed);
            // `note_shot_faced` was called above, so this column has to move
            // with it — the physics path used to leave it behind, which is
            // why `shots_faced` matched `saves` only by accident.
            save_accounting_stats::SHOTS_FACED_INC[site].fetch_add(1, Ordering::Relaxed);
            save_accounting_stats::PENDING_DELIVERED.fetch_add(1, Ordering::Relaxed);
        }
    }
}
