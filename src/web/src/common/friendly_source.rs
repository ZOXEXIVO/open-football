//! Shared resolution of the live Friendly bucket's source league.
//!
//! The Overview and History pages both need to know which league the
//! player's friendly-bucket matches were actually played in — youth
//! squads compete in `friendly` leagues ("Premier League U19",
//! "First League U18"), and the pages label the Friendly row with that
//! real league name instead of the generic "Friendly". The projection
//! receives the slug via `PlayerLiveStatsInput::friendly_source_slug`;
//! this helper owns the lookup order so the two routes cannot drift.

use core::{Player, SimulatorData, Team};

pub struct FriendlySourceSlug;

impl FriendlySourceSlug {
    /// Source slug for the live Friendly entry. Priority:
    ///   1. `player.friendly_source_slug` — set at match-record time, so
    ///      it reflects the actual league the player played friendlies
    ///      in this spell (the only authoritative source for a senior
    ///      loanee playing youth friendlies).
    ///   2. Inference from the player's current team / club roster — used
    ///      for save-loaded players who haven't played a friendly yet, or
    ///      legacy saves that pre-date the field: a non-senior team's own
    ///      league directly, else the club's first youth team with a
    ///      league.
    /// Senior callers with no youth squad fall through to empty → the
    /// projection inherits the anchor spell's league_slug → the pages
    /// render the generic "Friendly" label.
    pub fn resolve(player: &Player, team: Option<&Team>, data: &SimulatorData) -> String {
        player
            .friendly_source_slug
            .clone()
            .or_else(|| {
                team.and_then(|team| {
                    let direct = if !team.team_type.is_own_team() {
                        team.league_id
                            .and_then(|lid| data.league(lid))
                            .map(|l| l.slug.clone())
                    } else {
                        None
                    };
                    if direct.is_some() {
                        return direct;
                    }
                    data.club(team.club_id)
                        .and_then(|club| {
                            club.teams
                                .teams
                                .iter()
                                .find(|t| !t.team_type.is_own_team() && t.league_id.is_some())
                        })
                        .and_then(|youth| youth.league_id)
                        .and_then(|lid| data.league(lid))
                        .map(|l| l.slug.clone())
                })
            })
            .unwrap_or_default()
    }
}
