use super::fixtures::{CompletedMatch, LeagueFixtures};
use super::{LeagueGetRequest, TourSchedule};
use crate::ai::{ScoreAnswers, ScoreQuestion};
use crate::leagues::address::{LeagueAddress, LeaguePage};
use crate::{ApiResult, GameAppData};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use core::league::League;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};

const PATH: &str = "/match-highlights";
const CRITERIA: &[&str] = &[
    "Routine match with little wider fan interest",
    "Some interest for followers of the clubs",
    "Clearly noteworthy to league fans, including recorded rivalries, marquee matchups, or striking results",
    "Exceptional event of wide interest to football fans",
];

#[derive(Deserialize)]
pub(super) struct HighlightsQuery {
    snapshot: Option<String>,
}

#[derive(Hash, Serialize)]
pub(super) struct LeagueHighlights {
    #[serde(skip)]
    league_id: u32,
    league_name: String,
    results: Vec<CompletedMatch>,
}

impl LeagueHighlights {
    fn supports(league: &League) -> bool {
        !league.is_cup && !league.friendly
    }

    pub(super) fn from_schedule(league: &League, schedule: &[TourSchedule]) -> Option<Self> {
        if !Self::supports(league) {
            return None;
        }
        let results = LeagueFixtures::completed(schedule);
        (!results.is_empty()).then(|| Self {
            league_id: league.id,
            league_name: league.name.clone(),
            results,
        })
    }

    fn snapshot(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }

    pub(super) fn url(&self, league_url: &str) -> String {
        format!("{league_url}{PATH}?snapshot={}", self.snapshot())
    }

    fn questions(&self) -> BTreeMap<&str, ScoreQuestion<'static>> {
        self.results
            .iter()
            .map(|result| {
                let id = result.match_id.as_str();
                let instructions = format!(
                    "Rate fan interest in match_id {id}. The club_rivalry flag identifies a \
                     confirmed derby or rivalry. Give a confirmed rivalry or a marquee clash \
                     between prominent clubs at least level 2 even with an ordinary scoreline \
                     or a 0-0 draw. Also consider major upsets and unusual scorelines. National \
                     reputation is measured from 0 to 10000. Use the supplied fixture facts."
                );
                (
                    id,
                    ScoreQuestion::Score {
                        instructions,
                        criteria: CRITERIA,
                    },
                )
            })
            .collect()
    }

    fn select(self, answers: &ScoreAnswers) -> Option<Vec<String>> {
        let mut scored = self
            .results
            .into_iter()
            .map(|result| {
                let score = answers.score(&result.match_id)?;
                (0.0..=3.0)
                    .contains(&score)
                    .then_some((result.match_id, score))
            })
            .collect::<Option<Vec<_>>>()?;
        // A stable sort keeps equal scores in panel order.
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).expect("validated scores are finite"));
        Some(
            scored
                .into_iter()
                .take(2)
                .enumerate()
                .filter(|(rank, (_, score))| *rank == 0 || *score >= 2.0)
                .map(|(_, (id, _))| id)
                .collect(),
        )
    }

    pub(super) async fn action(
        State(state): State<GameAppData>,
        Path(path): Path<LeagueGetRequest>,
        Query(query): Query<HighlightsQuery>,
    ) -> ApiResult<Response> {
        let snapshot = query.snapshot.and_then(|token| token.parse().ok());
        let mut response = Self::lookup(&state, &path, snapshot).await?;
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        Ok(response)
    }

    async fn lookup(
        state: &GameAppData,
        path: &LeagueGetRequest,
        snapshot: Option<u64>,
    ) -> ApiResult<Response> {
        let connection = state.ai.connection().await;
        let highlights = {
            let guard = state.data.read().await;
            let Some(data) = guard.as_ref() else {
                return Ok(Json(Vec::<String>::new()).into_response());
            };
            let suffix = snapshot.map_or_else(
                || PATH.to_string(),
                |token| format!("{PATH}?snapshot={token}"),
            );
            let league = match LeagueAddress::resolve(
                data,
                &path.lang,
                &path.country_slug,
                &path.league_slug,
                &suffix,
            )? {
                LeaguePage::Found(league, _) => league,
                LeaguePage::Moved(response) => return Ok(response),
            };
            if !Self::supports(league) || connection.is_none() || snapshot.is_none() {
                None
            } else {
                Self::from_schedule(league, &LeagueFixtures::schedule(data, league))
            }
        };
        let mut ids = Vec::new();
        if let (Some(connection), Some(highlights)) = (connection, highlights)
            && snapshot == Some(highlights.snapshot())
            && let Some(answers) = connection.score(&highlights, highlights.questions()).await
            && let Some(selected) = highlights.select(&answers)
            && state.ai.is_current(&connection).await
        {
            ids = selected;
        }
        Ok(Json(ids).into_response())
    }
}
