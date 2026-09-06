//! Global-tournament post-match processor.
//!
//! Mirrors [`super::continental::WorldNationalCompetitions`]
//! but for fixtures owned by [`crate::competitions::GlobalCompetitions`]
//! (World Cup, Confederations Cup, …). Reuses the same world-wide
//! stats/Elo/schedule helpers so a goal at the World Cup updates
//! exactly the same player-side state as a goal in a continental
//! qualifier — no parallel write paths to drift apart.

use chrono::NaiveDate;
use log::info;

use super::lookups::world_country_name;
use super::stats::{
    InternationalMatchContext, apply_world_elo, apply_world_international_stats,
    collect_international_appearances, record_world_country_schedule,
};
use crate::NationalTeamLevel;
use crate::competitions::global::GlobalCompetitionFixture;
use crate::continent::Continent;
use crate::r#match::{MatchResult, MatchResultRaw};

/// Apply a single global-tournament match. The caller still owns
/// recording the result into the [`GlobalCompetitions`] state and
/// pushing the returned [`MatchResult`] into the global match store —
/// this helper is concerned only with the *player-facing* side
/// effects that should be identical across competition tiers.
pub fn apply_global_tournament_result(
    continents: &mut [Continent],
    fixture: &GlobalCompetitionFixture,
    raw: &MatchResultRaw,
    date: NaiveDate,
    competition_label: &str,
    competition_full_name: &str,
) -> MatchResult {
    let score = raw.score.as_ref().expect("match should have score").clone();
    let home_score = score.home_team.get();
    let away_score = score.away_team.get();
    let home_country_id = fixture.home_country_id;
    let away_country_id = fixture.away_country_id;

    let match_id = format!(
        "int-{}-{}-{}",
        date.format("%Y%m%d"),
        home_country_id,
        away_country_id
    );

    let appearances = collect_international_appearances(
        raw,
        home_country_id,
        away_country_id,
        home_score,
        away_score,
    );

    apply_world_international_stats(
        continents,
        home_country_id,
        away_country_id,
        &appearances,
        &InternationalMatchContext::new(date, NationalTeamLevel::Senior, competition_full_name),
    );
    apply_world_elo(
        continents,
        home_country_id,
        away_country_id,
        home_score,
        away_score,
    );

    let home_name = world_country_name(continents, home_country_id);
    let away_name = world_country_name(continents, away_country_id);

    record_world_country_schedule(
        continents,
        date,
        home_country_id,
        away_country_id,
        &home_name,
        &away_name,
        home_score,
        away_score,
        competition_full_name,
        &match_id,
        NationalTeamLevel::Senior,
    );

    info!(
        "Global competition ({}): {} {} - {} {}",
        competition_label, home_name, home_score, away_score, away_name
    );

    MatchResult {
        id: match_id,
        league_id: 0,
        league_slug: "international".to_string(),
        home_team_id: home_country_id,
        away_team_id: away_country_id,
        score,
        details: Some(raw.clone()),
        friendly: false,
    }
}
