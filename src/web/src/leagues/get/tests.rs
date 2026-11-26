use super::fixtures::LeagueFixtures;
use crate::routes::ServerRoutes;
use crate::{
    AiConfig, EventI18nManager, GameAppData, I18nManager, NewsI18nManager, WorkerRegistry,
};
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode, Uri, header};
use axum::response::Response;
use axum::{Json, Router, routing::post};
use chrono::{NaiveDate, NaiveDateTime};
use core::club::ClubAcademy;
use core::competitions::global::GlobalCompetitions;
use core::continent::Continent;
use core::league::{
    DayMonthPeriod, League, LeagueCollection, LeagueSettings, ScheduleItem, ScheduleTour,
};
use core::r#match::{Score, TeamScore};
use core::shared::Location;
use core::{
    Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PlayerCollection,
    SimulatorData, StaffCollection, TeamBuilder, TeamCollection, TeamReputation, TeamType,
    TrainingSchedule,
};
use serde_json::{Value, json};
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;
use tower::ServiceExt;

const PAGE: &str = "/fr/leagues/france/ligue-1";
const SCORE_PATH: &str = "/decision/custom/?version=1";
const ANSWERS: &str = r#"{"answers":{"rout":{"score":2.8},"quiet":{"score":1.9},"swing":{"score":2.4},"draw":{"score":2.4},"foreign":null}}"#;

struct LeagueFixture;

impl LeagueFixture {
    fn date(day: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 8, day)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
    }

    fn club(id: u32, name: &str, slug: &str, national_reputation: u16) -> Club {
        let team = TeamBuilder::new()
            .id(id)
            .league_id(Some(1))
            .club_id(id)
            .name(name.to_string())
            .slug(slug.to_string())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(Vec::new()))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(500, national_reputation, 500))
            .training_schedule(TrainingSchedule::new(
                Self::date(1).time(),
                Self::date(1).time(),
            ))
            .build()
            .unwrap();
        Club::new(
            id,
            name.to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![team]),
            ClubFacilities::default(),
        )
    }

    fn item(id: &str, day: u32, result: Option<(bool, u8, u8)>) -> ScheduleItem {
        let score = result.map(|(reversed, home, away)| {
            let (first, second) = if reversed { (11, 10) } else { (10, 11) };
            let mut score = Score::new(first, second);
            score.home_team = TeamScore::new_with_score(first, home);
            score.away_team = TeamScore::new_with_score(second, away);
            score
        });
        let mut item = ScheduleItem::new(1, "ligue-1".to_string(), 10, 11, Self::date(day), score);
        item.id = id.to_string();
        item
    }

    fn world() -> SimulatorData {
        let mut league = League::new(
            1,
            "Ligue 1".to_string(),
            "ligue-1".to_string(),
            1,
            5_000,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier: 1,
                promotion_spots: 0,
                relegation_spots: 0,
                league_group: None,
                split_season: false,
            },
            false,
        );
        league.schedule.tours = vec![
            ScheduleTour {
                num: 1,
                items: vec![Self::item("earlier", 20, Some((false, 2, 0)))],
            },
            ScheduleTour {
                num: 2,
                items: vec![
                    Self::item("draw", 24, Some((false, 0, 0))),
                    Self::item("swing", 25, Some((true, 3, 4))),
                    Self::item("rout", 26, Some((false, 7, 2))),
                    Self::item("quiet", 26, Some((false, 1, 0))),
                ],
            },
            ScheduleTour {
                num: 3,
                items: vec![Self::item("upcoming", 27, None)],
            },
        ];
        let paris = Self::club(10, "Paris Saint-Germain", "paris", 8700);
        let mut marseille = Self::club(11, "Olympique de Marseille", "marseille", 7700);
        marseille.rivals.push(paris.id);
        let country = Country::builder()
            .id(1)
            .code("FR".to_string())
            .slug("france".to_string())
            .name("France".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(vec![league]))
            .clubs(vec![paris, marseille])
            .build()
            .unwrap();
        SimulatorData::new(
            Self::date(26),
            vec![Continent::new(
                1,
                "Europe".to_string(),
                vec![country],
                Vec::new(),
            )],
            GlobalCompetitions::new(Vec::new()),
        )
    }

    fn state() -> GameAppData {
        static DATABASE: OnceLock<Arc<database::DatabaseEntity>> = OnceLock::new();
        let i18n = Arc::new(I18nManager::new());
        GameAppData {
            database: Arc::clone(
                DATABASE.get_or_init(|| Arc::new(database::DatabaseLoader::load())),
            ),
            data: Arc::new(tokio::sync::RwLock::new(Some(Arc::new(Self::world())))),
            process_lock: Arc::default(),
            cancel_flag: Arc::default(),
            news_i18n: Arc::new(NewsI18nManager::new()),
            events_i18n: Arc::new(EventI18nManager::new(&i18n)),
            i18n,
            workers: WorkerRegistry::empty(),
            ai: AiConfig::new(),
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    async fn request(app: &Router, method: Method, path: &str, body: Body) -> Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn get(app: &Router, path: &str) -> Response {
        Self::request(app, Method::GET, path, Body::empty()).await
    }

    async fn body(response: Response) -> String {
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    async fn selections(app: &Router, path: &str) -> Vec<String> {
        let response = Self::get(app, path).await;
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        serde_json::from_str(&Self::body(response).await).unwrap()
    }

    fn highlights_path(html: &str) -> &str {
        html.split_once("data-match-highlights=\"")
            .unwrap()
            .1
            .split_once('"')
            .unwrap()
            .0
    }
}

struct DecisionRequest {
    uri: Uri,
    headers: HeaderMap,
    body: Value,
}

struct DecisionFixture {
    url: String,
    requests: mpsc::UnboundedReceiver<DecisionRequest>,
    server: JoinHandle<()>,
}

impl DecisionFixture {
    async fn start(status: StatusCode, body: &'static str, gate: Option<Arc<Notify>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}{SCORE_PATH}", listener.local_addr().unwrap());
        let (sender, requests) = mpsc::unbounded_channel();
        let app = Router::new().route(
            "/decision/custom/",
            post(
                move |uri: Uri, headers: HeaderMap, Json(request): Json<Value>| {
                    let sender = sender.clone();
                    let gate = gate.clone();
                    async move {
                        sender
                            .send(DecisionRequest {
                                uri,
                                headers,
                                body: request,
                            })
                            .unwrap();
                        if let Some(gate) = gate {
                            gate.notified().await;
                        }
                        (status, body)
                    }
                },
            ),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            url,
            requests,
            server,
        }
    }

    async fn received(&mut self) -> DecisionRequest {
        tokio::time::timeout(Duration::from_secs(5), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }

    async fn save(&self, app: &Router, api_key: &str) {
        let response = LeagueFixture::request(
            app,
            Method::POST,
            "/api/ai/config",
            Body::from(json!({"url": self.url, "api_key": api_key}).to_string()),
        )
        .await;
        assert_eq!(
            serde_json::from_str::<Value>(&LeagueFixture::body(response).await).unwrap()["status"],
            "ok"
        );
    }

    async fn configure(&self, state: &GameAppData) {
        state
            .ai
            .set(crate::ModelSettings {
                url: self.url.clone(),
                api_key: String::new(),
            })
            .await;
    }
}

impl Drop for DecisionFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[test]
fn scoring_uses_the_visible_completed_fixtures_in_panel_order() {
    let world = LeagueFixture::world();
    let schedule = LeagueFixtures::schedule(&world, world.league(1).unwrap());
    assert_eq!(
        schedule
            .iter()
            .map(|day| day.date.as_str())
            .collect::<Vec<_>>(),
        ["24.08.2026", "25.08.2026", "26.08.2026", "27.08.2026"]
    );
    assert_eq!(schedule[3].matches[0].match_id, "upcoming");
    assert_eq!(
        LeagueFixtures::completed(&schedule)
            .into_iter()
            .map(|r| (r.match_id, r.home_goals, r.away_goals))
            .collect::<Vec<_>>(),
        [
            ("draw".to_string(), 0, 0),
            ("swing".to_string(), 4, 3),
            ("rout".to_string(), 7, 2),
            ("quiet".to_string(), 1, 0)
        ],
    );
}

#[test]
fn canonical_lookup_posts_one_keyed_batch_using_the_saved_connection() {
    LeagueFixture::block_on(async {
        let state = LeagueFixture::state();
        let app = ServerRoutes::create().with_state(state);
        let mut decision = DecisionFixture::start(StatusCode::OK, ANSWERS, None).await;
        for key in ["secret", ""] {
            decision.save(&app, key).await;
            let probe = decision.received().await;
            assert_eq!(probe.uri, SCORE_PATH);
            assert_eq!(probe.body["state"], "ping");
            assert_eq!(
                probe.body["questions"]["ping"]["criteria"],
                json!(["no", "yes"])
            );

            let response = LeagueFixture::get(&app, "/api/ai/config").await;
            let saved: Value = serde_json::from_str(&LeagueFixture::body(response).await).unwrap();
            assert_eq!(
                saved,
                json!({"configured":true,"url":decision.url,"api_key":key})
            );

            let html = LeagueFixture::body(LeagueFixture::get(&app, PAGE).await).await;
            assert!(!html.contains(&decision.url));
            assert!(decision.requests.try_recv().is_err());
            let path = LeagueFixture::highlights_path(&html);
            assert!(path.starts_with(&format!("{PAGE}/match-highlights?snapshot=")));
            let moved = LeagueFixture::get(&app, &path.replace("/france/", "/germany/")).await;
            assert_eq!(moved.status(), StatusCode::MOVED_PERMANENTLY);
            assert_eq!(moved.headers()[header::LOCATION], path);
            assert_eq!(
                LeagueFixture::selections(&app, &format!("{path}&home_goals=99")).await,
                ["rout", "draw"]
            );

            let request = decision.received().await;
            assert_eq!(request.uri, SCORE_PATH);
            let auth = request
                .headers
                .get(header::AUTHORIZATION)
                .map(|value| value.to_str().unwrap());
            let expected = (!key.is_empty()).then_some("Bearer secret");
            assert_eq!(auth, expected);
            assert_eq!(
                request.body["state"],
                json!({
                    "league_name": "Ligue 1",
                    "results": [
                        {"match_id":"draw","home_team":"Paris Saint-Germain","away_team":"Olympique de Marseille","home_goals":0,"away_goals":0,"club_rivalry":true,"home_national_reputation":8700,"away_national_reputation":7700},
                        {"match_id":"swing","home_team":"Paris Saint-Germain","away_team":"Olympique de Marseille","home_goals":4,"away_goals":3,"club_rivalry":true,"home_national_reputation":8700,"away_national_reputation":7700},
                        {"match_id":"rout","home_team":"Paris Saint-Germain","away_team":"Olympique de Marseille","home_goals":7,"away_goals":2,"club_rivalry":true,"home_national_reputation":8700,"away_national_reputation":7700},
                        {"match_id":"quiet","home_team":"Paris Saint-Germain","away_team":"Olympique de Marseille","home_goals":1,"away_goals":0,"club_rivalry":true,"home_national_reputation":8700,"away_national_reputation":7700},
                    ],
                })
            );
            let questions = request.body["questions"].as_object().unwrap();
            assert_eq!(questions.len(), 4);
            for id in ["draw", "swing", "rout", "quiet"] {
                assert_eq!(questions[id]["type"], "score");
                let instructions = questions[id]["instructions"].as_str().unwrap();
                for term in [
                    id,
                    "club_rivalry",
                    "marquee clash",
                    "upsets",
                    "unusual scorelines",
                ] {
                    assert!(instructions.contains(term), "{instructions}");
                }
                assert_eq!(questions[id]["criteria"], questions["draw"]["criteria"]);
                assert_eq!(questions[id]["criteria"].as_array().unwrap().len(), 4);
            }
            assert!(decision.requests.try_recv().is_err());
        }
    });
}

#[test]
fn disabled_stale_and_empty_lookups_skip_the_model() {
    LeagueFixture::block_on(async {
        let state = LeagueFixture::state();
        let app = ServerRoutes::create().with_state(state.clone());
        let mut decision = DecisionFixture::start(StatusCode::OK, ANSWERS, None).await;
        decision.configure(&state).await;
        let html = LeagueFixture::body(LeagueFixture::get(&app, PAGE).await).await;
        let path = LeagueFixture::highlights_path(&html);

        let response =
            LeagueFixture::request(&app, Method::DELETE, "/api/ai/config", Body::empty()).await;
        assert!(
            LeagueFixture::body(response)
                .await
                .contains("\"status\":\"ok\"")
        );
        assert!(LeagueFixture::selections(&app, path).await.is_empty());
        assert!(
            !LeagueFixture::body(LeagueFixture::get(&app, PAGE).await)
                .await
                .contains("data-match-highlights")
        );

        decision.configure(&state).await;
        let mut world = LeagueFixture::world();
        let tours = &mut world.continents[0].countries[0].leagues.leagues[0]
            .schedule
            .tours;
        tours[1].items[1].result = Some(Score::new(10, 11));
        *state.data.write().await = Some(Arc::new(world));
        assert!(LeagueFixture::selections(&app, path).await.is_empty());

        let mut world = LeagueFixture::world();
        world.date = LeagueFixture::date(27);
        *state.data.write().await = Some(Arc::new(world));
        assert!(LeagueFixture::selections(&app, path).await.is_empty());
        let only_upcoming = LeagueFixture::body(LeagueFixture::get(&app, PAGE).await).await;
        assert!(!only_upcoming.contains("data-match-highlights"));
        assert!(only_upcoming.contains("/fr/match/upcoming"));
        assert!(decision.requests.try_recv().is_err());
    });
}

#[test]
fn a_delayed_answer_does_not_hold_the_page_or_survive_a_replaced_connection() {
    LeagueFixture::block_on(async {
        let state = LeagueFixture::state();
        let app = ServerRoutes::create().with_state(state.clone());
        let gate = Arc::new(Notify::new());
        let mut decision =
            DecisionFixture::start(StatusCode::OK, ANSWERS, Some(Arc::clone(&gate))).await;
        decision.configure(&state).await;
        let html = LeagueFixture::body(LeagueFixture::get(&app, PAGE).await).await;
        let path = LeagueFixture::highlights_path(&html).to_string();
        let result = tokio::spawn({
            let app = app.clone();
            async move { LeagueFixture::selections(&app, &path).await }
        });
        decision.received().await;
        assert!(!result.is_finished());

        let refreshed =
            tokio::time::timeout(Duration::from_secs(2), LeagueFixture::get(&app, PAGE))
                .await
                .unwrap();
        assert_eq!(LeagueFixture::body(refreshed).await, html);
        let mut data = tokio::time::timeout(Duration::from_secs(2), state.data.write())
            .await
            .unwrap();
        *data = Some(Arc::new(LeagueFixture::world()));
        drop(data);
        state.ai.clear().await;
        decision.configure(&state).await;
        gate.notify_one();
        assert!(result.await.unwrap().is_empty());

        let mut previous = 0;
        for id in ["draw", "swing", "rout", "quiet", "upcoming"] {
            let offset = html.find(&format!("href=\"/fr/match/{id}\"")).unwrap();
            assert!(offset > previous);
            previous = offset;
        }
        assert!(html.contains("href=\"/fr/teams/paris\""));
        assert!(html.contains("href=\"/fr/teams/marseille\""));
        assert!(html.contains("0 – 0"));
        assert!(html.contains("4 – 3"));
        assert_eq!(
            html.matches(
                r#"<span class="visually-hidden fx-highlight-label" hidden>Match marquant</span>"#
            )
            .count(),
            4
        );
        assert!(html.contains(r#"class="fm-fixture" data-highlight-match="draw""#));
        assert!(!html.contains("data-highlight-match=\"upcoming\""));
    });
}

#[test]
fn http_errors_and_timeouts_return_empty_highlights() {
    LeagueFixture::block_on(async {
        let state = LeagueFixture::state();
        let app = ServerRoutes::create().with_state(state.clone());
        let http_error = DecisionFixture::start(StatusCode::CREATED, ANSWERS, None).await;
        http_error.configure(&state).await;
        let html = LeagueFixture::body(LeagueFixture::get(&app, PAGE).await).await;
        let path = LeagueFixture::highlights_path(&html);
        assert!(LeagueFixture::selections(&app, path).await.is_empty());

        let mut timed_out =
            DecisionFixture::start(StatusCode::OK, ANSWERS, Some(Arc::new(Notify::new()))).await;
        timed_out.configure(&state).await;
        let (ids, _) = tokio::join!(LeagueFixture::selections(&app, path), timed_out.received());
        assert!(ids.is_empty());
    });
}

#[test]
fn lookup_selects_the_best_match_and_rejects_incomplete_or_invalid_batches() {
    LeagueFixture::block_on(async {
        let state = LeagueFixture::state();
        let app = ServerRoutes::create().with_state(state.clone());
        for (answers, expected) in [
            (
                r#"{"answers":{"draw":{"score":1.3},"swing":{"score":0},"rout":{"score":1.999},"quiet":{"score":1}}}"#,
                &["rout"][..],
            ),
            (
                r#"{"answers":{"draw":{"score":2.0},"swing":{"score":1.999},"rout":{"score":1.3},"quiet":{"score":1}}}"#,
                &["draw"][..],
            ),
            (
                r#"{"answers":{"draw":{"score":-0.0},"swing":{"score":0.0},"rout":{"score":0},"quiet":{"score":0}}}"#,
                &["draw"][..],
            ),
            (
                r#"{"answers":{"draw":{"score":2.0},"swing":{"score":2.0},"rout":{"score":3},"quiet":{"score":0}}}"#,
                &["rout", "draw"][..],
            ),
            (
                r#"{"answers":{"draw":{"score":2.001},"swing":{"score":2.002},"rout":{"score":3},"quiet":{"score":0}}}"#,
                &["rout", "swing"][..],
            ),
            (
                r#"{"answers":{"draw":{"score":3},"swing":{"score":2},"rout":{"score":0}}}"#,
                &[][..],
            ),
            (
                r#"{"answers":{"draw":{"score":3},"swing":{"score":2},"rout":{"score":0},"quiet":{"score":"2"}}}"#,
                &[][..],
            ),
            (
                r#"{"answers":{"draw":{"score":3},"swing":{"score":2},"rout":{"score":0},"quiet":{"score":-0.01}}}"#,
                &[][..],
            ),
            (
                r#"{"answers":{"draw":{"score":3},"swing":{"score":2},"rout":{"score":0},"quiet":{"score":3.01}}}"#,
                &[][..],
            ),
            ("{invalid JSON", &[][..]),
        ] {
            let decision = DecisionFixture::start(StatusCode::OK, answers, None).await;
            decision.configure(&state).await;
            let html = LeagueFixture::body(LeagueFixture::get(&app, PAGE).await).await;
            let path = LeagueFixture::highlights_path(&html);
            assert_eq!(
                LeagueFixture::selections(&app, path).await,
                expected,
                "{answers}"
            );
        }
    });
}

#[test]
fn the_match_marker_has_a_label_in_every_language() {
    let i18n = I18nManager::new();
    for &lang in crate::i18n::SUPPORTED_LANG_CODES {
        let locale = i18n.for_lang(lang);
        let label = locale.t("match_highlight");
        assert_ne!(label, "match_highlight");
        if lang != "en" {
            assert_ne!(label, "Noteworthy match", "{lang}");
        }
    }
}
