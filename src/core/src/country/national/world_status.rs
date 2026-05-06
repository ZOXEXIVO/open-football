//! World-wide passes that toggle `PlayerStatusType::Int` across every
//! club in every continent based on the current squads. Foreign-based
//! call-ups (a Brazilian playing in Spain, …) only get the right flag
//! when this scans every continent's clubs — the per-country call-up
//! can't reach them without breaking borrows.

use super::NationalTeam;
use crate::continent::Continent;
use crate::{
    HappinessEventCause, HappinessEventContext, HappinessEventScope, HappinessEventSeverity,
    HappinessEventType, NationalTeamEventContext, NationalTeamEventKind, PlayerStatusType,
};
use chrono::NaiveDate;
use std::collections::HashSet;

impl NationalTeam {
    /// Apply / release `PlayerStatusType::Int` across every club in
    /// every continent, based on each country's current squad.
    ///
    /// Fires happiness events on transitions: a fresh call-up is a big
    /// moment for a young pro; being dropped after a run of caps hurts
    /// pride. Keeping events here (not in the per-country call-up)
    /// means each player only sees one event per cycle even if their
    /// nation has already been processed before their club's continent.
    pub(crate) fn apply_callup_statuses_across_world(
        continents: &mut [Continent],
        date: NaiveDate,
    ) {
        let mut called_up: HashSet<u32> = HashSet::new();
        for continent in continents.iter() {
            for country in continent.countries.iter() {
                for sp in &country.national_team.squad {
                    called_up.insert(sp.player_id);
                }
            }
        }

        for continent in continents.iter_mut() {
            for country in continent.countries.iter_mut() {
                for club in country.clubs.iter_mut() {
                    for team in club.teams.iter_mut() {
                        for player in team.players.iter_mut() {
                            let is_called_up = called_up.contains(&player.id);
                            let was_in = player.statuses.get().contains(&PlayerStatusType::Int);

                            if is_called_up {
                                player.statuses.add(date, PlayerStatusType::Int);
                                if !was_in {
                                    let caps = player.player_attributes.international_apps;
                                    let mag = if caps == 0 {
                                        10.0
                                    } else if caps < 10 {
                                        6.0
                                    } else {
                                        3.0
                                    };
                                    let kind = if caps == 0 {
                                        NationalTeamEventKind::FirstCallup
                                    } else {
                                        NationalTeamEventKind::Recall
                                    };
                                    let nctx = NationalTeamEventContext::new(kind)
                                        .with_previous_caps(caps);
                                    let happiness_ctx = HappinessEventContext::new(
                                        HappinessEventCause::Other,
                                        HappinessEventSeverity::from_magnitude(mag),
                                        HappinessEventScope::Personal,
                                    )
                                    .with_national_team_context(nctx);
                                    player.happiness.add_event_with_context(
                                        HappinessEventType::NationalTeamCallup,
                                        mag,
                                        None,
                                        happiness_ctx,
                                    );
                                }
                            } else if was_in {
                                player.statuses.remove(PlayerStatusType::Int);
                                let caps = player.player_attributes.international_apps;
                                let mag = if caps >= 20 {
                                    -6.0
                                } else if caps >= 5 {
                                    -4.0
                                } else {
                                    -2.0
                                };
                                let nctx = NationalTeamEventContext::new(
                                    NationalTeamEventKind::DroppedDueToCompetition,
                                )
                                .with_previous_caps(caps);
                                let happiness_ctx = HappinessEventContext::new(
                                    HappinessEventCause::Other,
                                    HappinessEventSeverity::from_magnitude(mag),
                                    HappinessEventScope::Personal,
                                )
                                .with_national_team_context(nctx);
                                player.happiness.add_event_with_context(
                                    HappinessEventType::NationalTeamDropped,
                                    mag,
                                    None,
                                    happiness_ctx,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// World-wide variant of `release_callup_statuses_across_continent`.
    pub(crate) fn release_callup_statuses_across_world(continents: &mut [Continent]) {
        let mut released_ids: HashSet<u32> = HashSet::new();
        for continent in continents.iter() {
            for country in continent.countries.iter() {
                for sp in &country.national_team.squad {
                    released_ids.insert(sp.player_id);
                }
            }
        }

        for continent in continents.iter_mut() {
            for country in continent.countries.iter_mut() {
                for club in country.clubs.iter_mut() {
                    for team in club.teams.iter_mut() {
                        for player in team.players.iter_mut() {
                            if released_ids.contains(&player.id) {
                                player.statuses.remove(PlayerStatusType::Int);
                            }
                        }
                    }
                }
            }
        }
    }
}
