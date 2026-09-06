//! Post-match world-wide writes: caps/goals/reputation, Elo, schedule.
//!
//! Each helper walks every continent so foreign-based squad members
//! receive their stat bumps regardless of where the match was played
//! or which competition it belonged to. Reused by both the
//! continental qualifier orchestrator and the global tournament
//! processor — single source of truth for "what happens after a
//! national-team match".

use chrono::NaiveDate;
use std::collections::HashMap;

use super::lookups::world_country_elo;
use crate::continent::Continent;
use crate::country::national::{NationalTeamFixture, NationalTeamMatchResult};
use crate::r#match::{MatchResultRaw, PlayerMatchEndStats};
use crate::{HappinessEventType, InternationalStatistics, NationalTeamLevel};

/// One player's line from a national-team match — everything the world
/// sweep needs to book the appearance, gathered once by
/// [`collect_international_appearances`] so the sweep itself never has
/// to look back at the raw result.
pub struct InternationalAppearance<'a> {
    pub stats: &'a PlayerMatchEndStats,
    pub is_starter: bool,
    pub is_motm: bool,
    /// Goals his own side conceded — the GK clean-sheet input.
    pub team_goals_against: u8,
    /// The country he turned out for, read from the squad that named
    /// him rather than from his passport.
    pub country_id: u32,
}

/// Which national fixture the appearances belong to. Carried alongside
/// the appearance map so the per-season stat slices can be labelled
/// with the competition that actually produced them.
pub struct InternationalMatchContext<'a> {
    pub season_start_year: u16,
    pub level: NationalTeamLevel,
    /// Competition name as configured ("UEFA U21 Championship",
    /// "FIFA World Cup"). Empty for a fixture with no competition
    /// behind it; the slice then renders under the generic label.
    pub competition_name: &'a str,
}

impl<'a> InternationalMatchContext<'a> {
    pub fn new(date: NaiveDate, level: NationalTeamLevel, competition_name: &'a str) -> Self {
        Self {
            season_start_year: InternationalStatistics::season_of(date),
            level,
            competition_name,
        }
    }
}

/// Turn a played national-team match into the per-player appearance map
/// the world sweep consumes.
///
/// The engine writes a stat line only for players it had on the pitch,
/// so `player_stats` is the appearance set — an unused substitute is
/// named in a squad and absent here, which is what keeps him from
/// collecting a phantom cap. Starter-vs-substitute and the side a
/// player turned out for both come from the match squads.
pub fn collect_international_appearances<'a>(
    raw: &'a MatchResultRaw,
    home_country_id: u32,
    away_country_id: u32,
    home_score: u8,
    away_score: u8,
) -> HashMap<u32, InternationalAppearance<'a>> {
    raw.player_stats
        .iter()
        .map(|(&player_id, stats)| {
            let is_home = raw.left_team_players.main.contains(&player_id)
                || raw.left_team_players.substitutes.contains(&player_id);
            let squad = if is_home {
                &raw.left_team_players
            } else {
                &raw.right_team_players
            };
            let appearance = InternationalAppearance {
                stats,
                is_starter: squad.main.contains(&player_id),
                is_motm: raw.player_of_the_match_id == Some(player_id),
                team_goals_against: if is_home { away_score } else { home_score },
                country_id: if is_home {
                    home_country_id
                } else {
                    away_country_id
                },
            };
            (player_id, appearance)
        })
        .collect()
}

/// Update apps/goals/reputation for every player who actually appeared
/// in the match, no matter which continent their club sits on.
/// `appearances` is the on-pitch appearance set (starters + subs used)
/// — squad members who didn't play are absent from it, so they don't
/// get a fake cap or a fake `NationalTeamDebut` event.
///
/// Country-weighted reputation gains: stronger nations push the
/// reputation needle further (a goal at a World Cup for Brazil counts
/// for more than the same goal for a Tier-3 nation), bounded at 0.5x
/// to 2.0x so the curve doesn't explode at extremes.
pub fn apply_world_international_stats(
    continents: &mut [Continent],
    home_country_id: u32,
    away_country_id: u32,
    appearances: &HashMap<u32, InternationalAppearance<'_>>,
    ctx: &InternationalMatchContext<'_>,
) {
    apply_world_international_stats_for_level(
        continents,
        home_country_id,
        away_country_id,
        appearances,
        ctx,
    );
}

/// Level-aware post-match write. Every appearance books its stat line
/// into the player's own national-team ledger
/// ([`Player::international_statistics_mut`](crate::Player::international_statistics_mut))
/// — the club career ledger is untouched, because a cap is earned for a
/// country and not for an employer.
///
/// On top of that, `ctx.level` decides the caps side: Senior bumps
/// `international_apps` / `international_goals`, reputation, and fires
/// the first-cap debut event; U21 bumps only
/// `under_21_international_apps` / `under_21_international_goals` — it
/// never touches senior caps or fires the senior debut event, so a
/// player's senior record is built solely by senior appearances.
pub fn apply_world_international_stats_for_level(
    continents: &mut [Continent],
    home_country_id: u32,
    away_country_id: u32,
    appearances: &HashMap<u32, InternationalAppearance<'_>>,
    ctx: &InternationalMatchContext<'_>,
) {
    let is_senior = ctx.level == NationalTeamLevel::Senior;
    let mut country_weights: HashMap<u32, f32> = HashMap::new();

    if is_senior {
        for continent in continents.iter() {
            for country in continent.countries.iter() {
                if country.id != home_country_id && country.id != away_country_id {
                    continue;
                }
                let country_rep = country.reputation as f32;
                // Country reputation is a 0..10000 scale. Dividing by 500
                // saturated the clamp for every real footballing nation, so a
                // San Marino cap was worth exactly as much international fame
                // as a Brazil cap. Scale across the real range instead, with
                // an average nation landing near 1.0.
                let country_weight = (0.4 + 1.6 * (country_rep / 10_000.0)).clamp(0.4, 2.0);
                for s in &country.national_team.squad {
                    country_weights.insert(s.player_id, country_weight);
                }
            }
        }
    }

    for continent in continents.iter_mut() {
        for country in continent.countries.iter_mut() {
            for club in country.clubs.iter_mut() {
                for team in club.teams.iter_mut() {
                    for player in team.players.iter_mut() {
                        let Some(appearance) = appearances.get(&player.id) else {
                            continue;
                        };

                        // The appearance itself, on the record where a
                        // reader can find it: one slice per (season,
                        // level, competition), with the same stat line a
                        // league game leaves behind. The rating is the
                        // engine's own — no settlement pass runs on an
                        // international — which is exactly the number the
                        // match page prints.
                        let is_goalkeeper = player.position().is_goalkeeper();
                        player
                            .international_statistics_mut(
                                ctx.season_start_year,
                                ctx.level,
                                appearance.country_id,
                                ctx.competition_name,
                            )
                            .record_match_line(
                                appearance.stats,
                                appearance.stats.match_rating,
                                appearance.is_starter,
                                appearance.is_motm,
                                is_goalkeeper,
                                appearance.team_goals_against,
                            );

                        let goals = appearance.stats.goals;

                        if !is_senior {
                            player.player_attributes.under_21_international_apps += 1;
                            player.player_attributes.under_21_international_goals += goals;
                            continue;
                        }

                        let country_weight =
                            country_weights.get(&player.id).copied().unwrap_or(1.0);

                        let was_uncapped = player.player_attributes.international_apps == 0;
                        player.player_attributes.international_apps += 1;
                        player.player_attributes.international_goals += goals;

                        let goal_bonus = goals.min(3) as f32 * 20.0;
                        let base = 15.0;
                        let raw = base + goal_bonus;
                        let current_delta = (raw * 0.6 * country_weight) as i16;
                        let home_delta = (raw * 0.8 * country_weight) as i16;
                        let world_delta = (raw * 1.0 * country_weight) as i16;

                        player.player_attributes.update_reputation(
                            current_delta,
                            home_delta,
                            world_delta,
                        );

                        if was_uncapped {
                            // First international cap — fires on the
                            // actual on-pitch appearance, not call-up.
                            player.happiness.add_event_default_with_cooldown(
                                HappinessEventType::NationalTeamDebut,
                                3650,
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Update Elo for both countries after a national-team match.
/// Operates across the entire world so a continental qualifier and a
/// global tournament both go through the same Elo path.
pub fn apply_world_elo(
    continents: &mut [Continent],
    home_country_id: u32,
    away_country_id: u32,
    home_score: u8,
    away_score: u8,
) {
    let home_elo = world_country_elo(continents, home_country_id);
    let away_elo = world_country_elo(continents, away_country_id);

    for continent in continents.iter_mut() {
        for country in continent.countries.iter_mut() {
            if country.id == home_country_id {
                country
                    .national_team
                    .update_elo(home_score, away_score, away_elo);
            } else if country.id == away_country_id {
                country
                    .national_team
                    .update_elo(away_score, home_score, home_elo);
            }
        }
    }
}

/// Push a fixture entry into each country's national-team schedule so
/// the web layer can render the match in both teams' fixture lists.
#[allow(clippy::too_many_arguments)]
pub fn record_world_country_schedule(
    continents: &mut [Continent],
    date: NaiveDate,
    home_country_id: u32,
    away_country_id: u32,
    home_name: &str,
    away_name: &str,
    home_score: u8,
    away_score: u8,
    competition_name: &str,
    match_id: &str,
    level: NationalTeamLevel,
) {
    for continent in continents.iter_mut() {
        for country in continent.countries.iter_mut() {
            // Route the fixture into the schedule of the matching level —
            // U21 fixtures must not appear in the senior fixture list.
            let is_home = country.id == home_country_id;
            let is_away = country.id == away_country_id;
            if !is_home && !is_away {
                continue;
            }
            let schedule = match level {
                NationalTeamLevel::Senior => &mut country.national_team.schedule,
                NationalTeamLevel::Under21 => &mut country.u21_national_team.schedule,
            };
            if is_home {
                schedule.push(NationalTeamFixture {
                    date,
                    opponent_country_id: away_country_id,
                    opponent_country_name: away_name.to_string(),
                    is_home: true,
                    competition_name: competition_name.to_string(),
                    match_id: match_id.to_string(),
                    result: Some(NationalTeamMatchResult {
                        home_score,
                        away_score,
                        date,
                        opponent_country_id: away_country_id,
                    }),
                });
            } else {
                schedule.push(NationalTeamFixture {
                    date,
                    opponent_country_id: home_country_id,
                    opponent_country_name: home_name.to_string(),
                    is_home: false,
                    competition_name: competition_name.to_string(),
                    match_id: match_id.to_string(),
                    result: Some(NationalTeamMatchResult {
                        home_score: away_score,
                        away_score: home_score,
                        date,
                        opponent_country_id: home_country_id,
                    }),
                });
            }
        }
    }
}
