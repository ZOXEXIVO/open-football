use super::*;
use crate::watchlist::routes::WatchlistRoutes;
use crate::{AiConfig, EventI18nManager, I18nManager, NewsI18nManager, WorkerRegistry};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chrono::{NaiveDate, NaiveTime};
use core::club::ClubAcademy;
use core::club::player::builder::PlayerBuilder;
use core::competitions::global::GlobalCompetitions;
use core::continent::Continent;
use core::league::LeagueCollection;
use core::shared::Location;
use core::shared::fullname::FullName;
use core::{
    Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes,
    PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositions, PlayerSkills,
    RetirementReason, StaffCollection, TeamCollection, TeamReputation, TrainingSchedule,
};
use serde_json::json;
use std::sync::{Arc, OnceLock};
use tower::ServiceExt;

const ON_TEAM: u32 = 1;
const FREE_AGENT: u32 = 2;
const RETIRED: u32 = 3;

struct WatchlistFixture;

impl WatchlistFixture {
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2030, 6, 18).unwrap()
    }

    fn player(id: u32, last_name: &str, position: PlayerPositionType) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".to_string(), last_name.to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 15,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn team(players: Vec<Player>) -> Team {
        Team::builder()
            .id(100)
            .league_id(None)
            .club_id(10)
            .name("Paris".to_string())
            .slug("paris".to_string())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(500, 500, 500))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    fn world() -> SimulatorData {
        let club = Club::new(
            10,
            "Paris".to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![Self::team(vec![Self::player(
                ON_TEAM,
                "Striker",
                PlayerPositionType::Striker,
            )])]),
            ClubFacilities::default(),
        );
        let mut retired = Self::player(RETIRED, "Keeper", PlayerPositionType::Goalkeeper);
        retired.announce_retirement(Self::today(), RetirementReason::Age);
        let mut country = Country::builder()
            .id(1)
            .code("fr".to_string())
            .slug("france".to_string())
            .name("France".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(Vec::new()))
            .clubs(vec![club])
            .build()
            .unwrap();
        country.retired_players.add(retired);
        let mut data = SimulatorData::new(
            Self::today().and_hms_opt(12, 0, 0).unwrap(),
            vec![Continent::new(
                1,
                "Europe".to_string(),
                vec![country],
                Vec::new(),
            )],
            GlobalCompetitions::new(Vec::new()),
        );
        data.free_agents.push(Self::player(
            FREE_AGENT,
            "Midfielder",
            PlayerPositionType::MidfielderCenter,
        ));
        data
    }

    fn key(data: &SimulatorData, id: u32) -> String {
        WatchKey::of(data.player(id).unwrap()).to_string()
    }

    fn stranger(id: u32) -> String {
        let player = Self::player(id, "Somebody Else", PlayerPositionType::Striker);
        WatchKey::of(&player).to_string()
    }

    fn rows(data: &SimulatorData, keys: &[String]) -> Vec<WatchlistPlayerDto> {
        let i18n = I18nManager::new().for_lang("en");
        WatchlistRows::resolve(data, keys, &i18n, "en")
    }

    fn state(data: SimulatorData) -> GameAppData {
        static DATABASE: OnceLock<Arc<database::DatabaseEntity>> = OnceLock::new();
        let i18n = Arc::new(I18nManager::new());
        GameAppData {
            database: Arc::clone(
                DATABASE.get_or_init(|| Arc::new(database::DatabaseLoader::load())),
            ),
            data: Arc::new(tokio::sync::RwLock::new(Some(Arc::new(data)))),
            process_lock: Arc::default(),
            cancel_flag: Arc::default(),
            news_i18n: Arc::new(NewsI18nManager::new()),
            events_i18n: Arc::new(EventI18nManager::new(&i18n)),
            i18n,
            workers: WorkerRegistry::empty(),
            ai: AiConfig::new(),
        }
    }

    // `#[tokio::test]` expands to `::core::future`, which this workspace's
    // own `core` crate shadows, so the runtime is built by hand.
    fn post_rows(keys: &[String]) -> String {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let app = WatchlistRoutes::routes().with_state(Self::state(Self::world()));
                let request = Request::post("/en/watchlist/rows")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({ "keys": keys }).to_string()))
                    .unwrap();
                let response = app.oneshot(request).await.unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
                String::from_utf8(bytes.to_vec()).unwrap()
            })
    }
}

#[test]
fn a_matching_key_yields_a_team_row() {
    let data = WatchlistFixture::world();
    let key = WatchlistFixture::key(&data, ON_TEAM);
    let rows = WatchlistFixture::rows(&data, std::slice::from_ref(&key));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].watch_key, key);
    assert_eq!(rows[0].team_name, "Paris");
    assert!(!rows[0].retired);
}

#[test]
fn a_key_for_another_person_under_the_same_id_yields_no_row() {
    let data = WatchlistFixture::world();
    let keys = [ON_TEAM, FREE_AGENT, RETIRED].map(WatchlistFixture::stranger);
    assert!(WatchlistFixture::rows(&data, &keys).is_empty());
}

#[test]
fn an_unknown_id_yields_no_row() {
    let data = WatchlistFixture::world();
    let keys = [WatchlistFixture::stranger(999), "garbage".to_string()];
    assert!(WatchlistFixture::rows(&data, &keys).is_empty());
}

#[test]
fn a_free_agent_is_labelled_free_agent() {
    let data = WatchlistFixture::world();
    let rows = WatchlistFixture::rows(&data, &[WatchlistFixture::key(&data, FREE_AGENT)]);
    let i18n = I18nManager::new().for_lang("en");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].team_name, i18n.t("free_agent"));
    assert!(!rows[0].retired);
}

#[test]
fn a_retired_player_is_labelled_retired() {
    let data = WatchlistFixture::world();
    let rows = WatchlistFixture::rows(&data, &[WatchlistFixture::key(&data, RETIRED)]);
    let i18n = I18nManager::new().for_lang("en");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].team_name, i18n.t("retired"));
    assert!(rows[0].retired);
    assert_eq!(rows[0].conditions, 0);
}

#[test]
fn a_duplicated_key_yields_one_row() {
    let data = WatchlistFixture::world();
    let key = WatchlistFixture::key(&data, ON_TEAM);
    let rows = WatchlistFixture::rows(&data, &[key.clone(), key]);
    assert_eq!(rows.len(), 1);
}

#[test]
fn rows_are_sorted_by_position() {
    let data = WatchlistFixture::world();
    let keys = [ON_TEAM, FREE_AGENT, RETIRED].map(|id| WatchlistFixture::key(&data, id));
    let order: Vec<_> = WatchlistFixture::rows(&data, &keys)
        .iter()
        .map(|row| row.position_sort)
        .collect();
    assert_eq!(
        order,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::Striker,
        ]
    );
}

#[test]
fn the_rows_endpoint_renders_only_matching_keys() {
    let data = WatchlistFixture::world();
    let kept = WatchlistFixture::key(&data, ON_TEAM);
    let stale = WatchlistFixture::stranger(FREE_AGENT);
    let html = WatchlistFixture::post_rows(&[kept.clone(), stale.clone()]);
    assert_eq!(html.matches("<tr data-watch-key=").count(), 1);
    assert!(html.contains(&format!("<tr data-watch-key=\"{kept}\">")));
    assert!(!html.contains(&stale));
}

#[test]
fn the_rows_endpoint_shows_the_empty_state_when_nothing_resolves() {
    let html = WatchlistFixture::post_rows(&[WatchlistFixture::stranger(999)]);
    assert!(!html.contains("data-watch-key"));
    assert!(html.contains(I18nManager::new().for_lang("en").t("no_watchlist_players")));
}
