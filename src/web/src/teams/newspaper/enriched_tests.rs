use super::{IssueView, Paper, PaperFor, PressDesk, PressFocus, StoryComposer, StoryView};
use crate::{I18n, I18nManager, NewsI18n, NewsI18nManager};
use chrono::NaiveDate;
use core::club::academy::ClubAcademy;
use core::club::news::{
    MatchPerformance, NewsStory, NewsStoryDetail, NewsStoryKind, NewspaperIssue,
    PlayingTimeConcern, PressMood, ScoutSighting, ScoutVisit,
};
use core::continent::Continent;
use core::league::LeagueCollection;
use core::shared::{fullname::FullName, location::Location};
use core::{
    Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, GlobalCompetitions,
    PersonAttributes, Player, PlayerAttributes, PlayerCollection, PlayerPosition,
    PlayerPositionType, PlayerPositions, PlayerSkills, SimulatorData, StaffCollection, Team,
    TeamCollection, TeamReputation, TeamType, TrainingSchedule, TransferInterestSource,
    TransferInterestStage,
};

pub(crate) struct PressFixture;

impl PressFixture {
    pub(crate) const PUBLISHER: u32 = 10;
    pub(crate) const PLAYER: u32 = 17;
    pub(crate) const CITY: u32 = 2;
    const SCOUT: u32 = 61;
    const NORTH_END: u32 = 30;
    const BOROUGH: u32 = 4;
    const ATHLETIC: u32 = 50;

    pub(crate) fn day(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, day).unwrap()
    }

    fn player() -> Player {
        Player::builder()
            .id(Self::PLAYER)
            .full_name(FullName::new("Diego".into(), "Mora".into()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::flat_for_ability(140))
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn club(id: u32, team_id: u32, club_name: &str, team_name: &str, scout_employer: u32) -> Club {
        let players = if team_id == Self::PUBLISHER {
            vec![Self::player()]
        } else {
            Vec::new()
        };
        let mut team = Team::builder()
            .id(team_id)
            .club_id(id)
            .name(team_name.into())
            .slug(team_name.to_lowercase().replace(' ', "-"))
            .team_type(TeamType::Main)
            .league_id(None)
            .reputation(TeamReputation::new(5000, 5000, 5000))
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .training_schedule(TrainingSchedule::new(
                chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            ))
            .build()
            .unwrap();
        if id == scout_employer {
            let mut scout = team.staffs.head_coach().clone();
            scout.id = Self::SCOUT;
            scout.full_name = FullName::new("Lena".into(), "Soto".into());
            team.staffs = StaffCollection::new(vec![scout]);
        }
        Club::new(
            id,
            club_name.into(),
            Location::new(1),
            ClubFinances::new(0, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![team]),
            ClubFacilities::default(),
        )
    }

    pub(crate) fn world(scout_employer: u32) -> SimulatorData {
        let clubs = [
            (1, Self::PUBLISHER, "County", "County"),
            (Self::CITY, 20, "City", "City"),
            (3, Self::NORTH_END, "North End FC", "North End U18"),
            (Self::BOROUGH, 40, "Borough", "Borough"),
            (5, Self::ATHLETIC, "Athletic FC", "Athletic U18"),
        ]
        .map(|(id, team, club_name, team_name)| {
            Self::club(id, team, club_name, team_name, scout_employer)
        });
        let country = Country::builder()
            .id(1)
            .code("en".into())
            .slug("england".into())
            .name("England".into())
            .continent_id(1)
            .leagues(LeagueCollection::new(Vec::new()))
            .clubs(clubs.into())
            .build()
            .unwrap();
        SimulatorData::new(
            Self::day(9).and_hms_opt(0, 0, 0).unwrap(),
            vec![Continent::new(
                1,
                "Europe".into(),
                vec![country],
                Vec::new(),
            )],
            GlobalCompetitions::new(Vec::new()),
        )
    }

    pub(crate) fn paper(data: &SimulatorData) -> Paper<'_> {
        let team = data.team(Self::PUBLISHER).unwrap();
        Paper {
            name: &team.name,
            slug: &team.slug,
            club_id: team.club_id,
            team_id: team.id,
        }
    }

    pub(crate) fn club_page(data: &SimulatorData, i18n: &I18n, news: &NewsI18n) -> Vec<IssueView> {
        PressDesk::typeset(data, data.team(Self::PUBLISHER).unwrap(), i18n, news)
    }

    fn view(data: &SimulatorData, story: &NewsStory, news: &NewsI18n) -> StoryView {
        let slug = data.player(Self::PLAYER).unwrap().slug();
        PressDesk::story(
            data,
            Self::paper(data),
            story,
            &I18nManager::new().for_lang("en"),
            news,
            PressFocus::player(Self::PLAYER, &slug),
        )
    }

    fn performance(
        kind: NewsStoryKind,
        goals: u16,
        assists: u16,
        score: (u8, u8),
        opponent_team_id: u32,
    ) -> NewsStory {
        NewsStory::new(kind, Self::day(9))
            .about(Self::PLAYER)
            .with_numbers(i32::from(goals + assists), 720)
            .with_detail(NewsStoryDetail::Performance(MatchPerformance {
                fixture_id: "selected-fixture".into(),
                team_id: Self::PUBLISHER,
                opponent_team_id,
                goals_for: score.0,
                goals_against: score.1,
                goals,
                assists,
                rating: 720,
            }))
    }

    fn sighting(returned: bool, named: bool, multiple: bool) -> NewsStory {
        let latest = ScoutVisit {
            date: Self::day(8),
            fixture_id: "youth-fixture".into(),
            team_id: 11,
            opponent_team_id: Self::NORTH_END,
            club_id: Self::CITY,
            scout_id: Self::SCOUT,
            scout_name: named.then(|| "Lena Soto".into()),
            player_id: Self::PLAYER,
            notable: true,
        };
        let other = multiple.then(|| ScoutVisit {
            date: Self::day(6),
            fixture_id: "other-fixture".into(),
            opponent_team_id: Self::ATHLETIC,
            club_id: Self::BOROUGH,
            scout_id: 70,
            scout_name: None,
            ..latest.clone()
        });
        NewsStory::new(NewsStoryKind::ScoutsWatching, Self::day(9))
            .about(Self::PLAYER)
            .against(Self::CITY)
            .with_detail(NewsStoryDetail::Sighting(ScoutSighting {
                latest,
                other,
                returned,
            }))
    }

    pub(crate) fn rumour(date: NaiveDate, confirmed: bool) -> NewsStory {
        let (kind, source, stage, club) = if confirmed {
            (
                NewsStoryKind::RumourInterest,
                TransferInterestSource::ConfirmedApproach,
                TransferInterestStage::ConcreteInterest,
                Self::CITY,
            )
        } else {
            (
                NewsStoryKind::AgentTouting,
                TransferInterestSource::AgentLeak,
                TransferInterestStage::AgentSoundingOut,
                0,
            )
        };
        NewsStory::new(kind, date)
            .about(Self::PLAYER)
            .against(club)
            .with_detail(NewsStoryDetail::TransferInterest { source, stage })
    }
}

#[test]
fn performances_render_their_fixture_and_legacy_copy_uses_held_facts() {
    use NewsStoryKind::{HatTrick, SuperSub};
    let world = PressFixture::world(PressFixture::CITY);
    let news = NewsI18nManager::new();
    for lang in ["en", "de", "es", "fr", "ja", "pt", "ru", "tr", "zh"] {
        let locale = news.for_lang(lang);
        for mut story in [
            PressFixture::performance(HatTrick, 3, 0, (3, 3), PressFixture::NORTH_END),
            PressFixture::performance(SuperSub, 0, 1, (1, 2), PressFixture::NORTH_END),
            PressFixture::performance(SuperSub, 1, 1, (2, 0), PressFixture::NORTH_END),
        ] {
            let first = PressFixture::view(&world, &story, &locale);
            story.phrasing = 1;
            let second = PressFixture::view(&world, &story, &locale);
            assert_ne!(first.headline.text(), second.headline.text(), "{lang}");
            assert_ne!(first.body.text(), second.body.text(), "{lang}");
            for view in [first, second] {
                assert!(
                    view.headline
                        .spans
                        .iter()
                        .any(|s| s.section == "players" && s.marked)
                );
                assert!(
                    view.body
                        .spans
                        .iter()
                        .any(|s| s.section == "teams" && s.slug == "north-end-u18")
                );
                assert!(
                    !view.body.text().contains(['{', '}']),
                    "{lang} {}",
                    view.body.text()
                );
            }
        }
    }

    let en = news.for_lang("en");
    let missing = PressFixture::performance(SuperSub, 0, 1, (1, 2), 99);
    let body = PressFixture::view(&world, &missing, &en).body.text();
    assert!(
        body.contains("set up 1") && body.contains("1‑2 defeat to the opposition"),
        "{body}"
    );
    let legacy = PressFixture::performance(HatTrick, 3, 0, (3, 3), PressFixture::NORTH_END);
    let legacy = NewsStory {
        detail: None,
        ..legacy
    };
    let body = PressFixture::view(&world, &legacy, &en).body.text();
    assert!(
        body.contains('3') && body.contains("Diego Mora") && !body.contains('‑'),
        "{body}"
    );
    let grievance = NewsStory::new(NewsStoryKind::BenchFrustration, PressFixture::day(9))
        .about(PressFixture::PLAYER)
        .with_detail(NewsStoryDetail::PlayingTime(PlayingTimeConcern::Reserves));
    let story = PressFixture::view(&world, &grievance, &en);
    assert!(!story.is_quote && story.body.text().contains("first-team opportunity"));
}

#[test]
fn scouting_briefs_and_articles_keep_the_original_visitors_and_fixtures() {
    let before = PressFixture::world(PressFixture::CITY);
    let after = PressFixture::world(PressFixture::BOROUGH);
    assert_eq!(
        before
            .staff_with_team(PressFixture::SCOUT)
            .unwrap()
            .1
            .club_id,
        PressFixture::CITY
    );
    assert_eq!(
        after
            .staff_with_team(PressFixture::SCOUT)
            .unwrap()
            .1
            .club_id,
        PressFixture::BOROUGH
    );
    let news = NewsI18nManager::new().for_lang("en");
    let i18n = I18nManager::new().for_lang("en");
    let paper = PressFixture::paper(&after);
    for (returned, named, multiple) in [
        (false, true, false),
        (true, true, false),
        (true, true, true),
        (true, false, false),
    ] {
        let mut story = PressFixture::sighting(returned, named, multiple);
        for phrasing in 0..2 {
            story.phrasing = phrasing;
            let original = PressFixture::view(&before, &story, &news);
            let current = PressFixture::view(&after, &story, &news);
            let headline = current.headline.text();
            let body = current.body.text();
            assert_eq!(headline, original.headline.text());
            assert_eq!(body, original.body.text());
            assert!(
                headline.contains("City") && headline.contains("Diego Mora"),
                "{headline}"
            );
            assert!(
                body.contains("City")
                    && body.contains("North End U18")
                    && body.contains("2026-03-08"),
                "{body}"
            );
            assert_eq!(body.contains("Lena Soto"), named);
            assert_eq!(
                body.contains("Borough") && body.contains("Athletic U18"),
                multiple
            );
            assert!(!body.contains(['{', '}']));

            let issue = NewspaperIssue {
                number: 2,
                date: story.date,
                mood: PressMood::Steady,
                stories: [
                    vec![NewsStory::new(NewsStoryKind::RedCard, story.date); 3],
                    vec![story.clone()],
                ]
                .concat(),
                results: Vec::new(),
            };
            let issue = PressDesk::issue(
                &after,
                PaperFor::Own(paper),
                &issue,
                "County",
                &i18n,
                &news,
                PressFocus::none(),
            );
            assert_eq!(issue.briefs[0].headline.text(), headline);
        }
    }

    let legacy = NewsStory {
        detail: None,
        ..PressFixture::sighting(false, true, false)
    };
    let legacy = PressFixture::view(&after, &legacy, &news);
    let legacy_copy = format!("{} {}", legacy.headline.text(), legacy.body.text());
    assert!(
        legacy_copy.contains("interest")
            && !legacy_copy.contains("North End")
            && !legacy_copy.contains("Lena Soto"),
        "{legacy_copy}"
    );
}

#[test]
fn press_and_agent_headlines_can_stand_on_their_own() {
    let data = PressFixture::world(0);
    let news = NewsI18nManager::new().for_lang("en");
    let paper = PressFixture::paper(&data);
    for mut story in [
        NewsStory::new(NewsStoryKind::TransferSpeculation, PressFixture::day(9))
            .about(PressFixture::PLAYER)
            .with_detail(NewsStoryDetail::TransferInterest {
                source: TransferInterestSource::LocalPress,
                stage: TransferInterestStage::LooseRumour,
            }),
        PressFixture::rumour(PressFixture::day(9), false),
    ] {
        for index in 0..2 {
            story.phrasing = index;
            let attribution = if story.kind == NewsStoryKind::AgentTouting {
                "agent"
            } else {
                "local"
            };
            let brief = StoryComposer::headline(&data, &story, &news, paper).text();
            let article = PressFixture::view(&data, &story, &news);
            assert_eq!(brief, article.headline.text());
            assert!(
                brief.to_lowercase().contains(attribution) && !brief.contains("City"),
                "{brief}"
            );
            let body = article.body.text();
            assert!(
                !body.contains(['{', '}']) && !body.contains("City") && !body.contains("bid"),
                "{body}"
            );
        }
    }
}
