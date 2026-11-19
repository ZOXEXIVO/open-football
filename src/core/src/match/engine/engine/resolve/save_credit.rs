//! **Crediting a save the physics already made.** `try_save_shot` in the
//! ball layer changes the ball's state mid-flight but cannot reach the
//! two players involved, so it leaves the pair on `Ball` and this drains
//! it — firing the keeper's save stat and the shooter's on-target stat,
//! exactly the events the GK state machine would have emitted had the
//! physics save not pre-empted it.

use crate::r#match::SaveOutcome;
use crate::r#match::engine::engine::*;
use crate::r#match::engine::psychology::PositiveEvent;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    /// Consume `Ball::pending_save_credit` left behind by the physics
    /// save (`try_save_shot`). When the keeper actually changed ball
    /// state mid-flight (catch, safe parry, dangerous parry), this fires
    /// the save stat for the keeper and the on-target stat for the
    /// shooter — matching the events the GK state machine would have
    /// emitted if the physics save hadn't pre-empted it.
    /// A save of a chance at least this likely to beat an ordinary keeper
    /// is a big save — one a keeper feels.
    const BIG_SAVE_XGOT: f32 = 0.35;

    pub(in crate::r#match::engine::engine) fn apply_pending_save_credit(
        field: &mut MatchField,
        context: &mut MatchContext,
    ) {
        let Some((keeper_id, shooter_id)) = field.ball.pending_save_credit.take() else {
            return;
        };
        let on_frame = std::mem::take(&mut field.ball.pending_save_on_frame);
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
        // The keeper carries the save out himself: diving, it lands him
        // with the ball or without it; on his feet, his next decision takes
        // a held ball into his body. See `MatchPlayer::on_save_outcome`.
        let held = field.ball.current_owner == Some(keeper_id);
        field.players[keeper_idx].on_save_outcome(if held {
            SaveOutcome::Held
        } else {
            SaveOutcome::Loose
        });
        // Read the outcome BEFORE resetting it — the accounting block at the
        // bottom of this function needs it, and resetting here first is why
        // the table kept reporting `parry 0` while the parry branch was
        // demonstrably firing 3662 times per 200 matches.
        let save_site = field.ball.pending_save_site;
        field.ball.pending_save_site = 1;
        let _ = save_site; // only read by the `match-logs` accounting below
        let spot_kick = std::mem::take(&mut field.ball.pending_save_spot_kick);
        // Going wide or over: he still touched it, but it kept nothing out.
        if !on_frame {
            #[cfg(feature = "match-logs")]
            save_accounting_stats::PENDING_OFF_FRAME
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            field.ball.clear_shot_metadata();
            return;
        }
        {
            let gk = &mut field.players[keeper_idx];
            // The GK denied a shot worth `shot_xg` xG — books the save,
            // the shot faced, and both xG ledgers in one call so they
            // cannot drift apart (see `note_shot_faced`).
            gk.statistics.note_shot_faced(shot_xg, true);
        }
        field.players[shooter_idx].memory.credit_shot_on_target();
        if shot_xg >= Self::BIG_SAVE_XGOT {
            context
                .psychology
                .record_positive(keeper_id, PositiveEvent::BigSave, context.current_tick());
        }
        if spot_kick {
            context.penalty_saves.push(keeper_id);
        }
        // Shot has resolved (saved). Drop the metadata so any
        // subsequent goal / save event can't double-credit. The giveaway
        // ledger stays open: a rebound goal is still the giver's error.
        field.ball.clear_shot_metadata();
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
