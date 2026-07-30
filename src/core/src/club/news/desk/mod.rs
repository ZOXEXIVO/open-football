//! The desks. Each one reads a slice of club state and files candidate
//! stories; none of them decides what actually goes to print — that is
//! the editor's job.
//!
//! | Module      | Files                                                    |
//! |-------------|----------------------------------------------------------|
//! | [`facts`]   | The gathered inputs a desk cannot read off a single club  |
//! | [`pitch`]   | Match reports, runs of form, the league table             |
//! | [`squad`]   | Form, fitness, discipline, milestones, contracts          |
//! | [`dugout`]  | The manager and his players; the squad-wide mood          |
//! | [`market`]  | Completed transfer business, and the verdict on it        |
//! | [`rumour`]  | Everything about a player's future that has not happened  |
//! | [`loan`]    | The loan column: players out on loan, and what they say   |
//! | [`fans`]    | The terraces and the press box                            |
//! | [`board`]   | Boardroom, academy, balance sheet                         |
//!
//! The squad desk owns the single pass over a club's rosters; the
//! dugout, fans, rumour and market-verdict detectors hang off it, and
//! it returns a [`facts::SquadPulse`] carrying the club-level moods
//! they tallied on the way through. Splitting the walk would multiply
//! the per-week cost across every club in the world for no editorial
//! gain — and the aggregate stories could not be told at all.

pub mod board;
pub mod dugout;
pub mod facts;
pub mod fans;
pub mod loan;
pub mod market;
pub mod pitch;
pub mod rumour;
pub mod squad;

pub use board::BoardroomDesk;
pub use dugout::DugoutDesk;
pub use facts::{
    Absorbing, CareerRecord, ClubDugoutWatch, ClubLoanWatch, ClubTransferWeek, CupTie,
    KeeperMatchFacts, LoanWatchEntry, ManagerPursuit, MatchStarFacts, OutfieldMatchFacts,
    PlayerStanding, RecentEvents, SquadPulse, StandingSnapshot, TransferMove, TransferMoveKind,
    WeeklyMatchFacts,
};
pub use fans::FansDesk;
pub use loan::LoanDesk;
pub use market::MarketDesk;
pub use pitch::{MatchDesk, TableDesk};
pub use rumour::RumourDesk;
pub use squad::SquadDesk;

#[cfg(test)]
mod tests {
    use super::facts::{
        ClubTransferWeek, MatchStarFacts, OutfieldMatchFacts, StandingSnapshot, TransferMove,
        TransferMoveKind, WeeklyMatchFacts,
    };
    use super::squad::SquadDesk;
    use super::{MarketDesk, MatchDesk, TableDesk};
    use crate::club::news::types::{IssueResult, NewsStory, NewsStoryKind, ResultCompetition};
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::FullName;
    use crate::{
        PersonAttributes, Player, PlayerAttributes, PlayerCollection, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, StaffCollection, Team, TeamBuilder,
        TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::NaiveDate;
    use rustc_hash::FxHashSet;

    struct Fixture;

    impl Fixture {
        /// The first team, and one of the club's own academy squads —
        /// both sides whose football its paper may report.
        const OUR_SIDE: u32 = 1;
        const OUR_YOUTH_SIDE: u32 = 2;
        /// A side the club does not own: a national team, or the club
        /// that sold the player last week.
        const SOMEBODY_ELSE: u32 = 999;

        fn day() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 3, 2).unwrap()
        }

        fn our_sides() -> FxHashSet<u32> {
            [Self::OUR_SIDE, Self::OUR_YOUTH_SIDE].into_iter().collect()
        }

        fn table(position: u8, teams: u8, played: u8, total_rounds: u8) -> StandingSnapshot {
            StandingSnapshot {
                position,
                teams,
                points: played,
                played,
                total_rounds,
            }
        }

        fn moved(player_id: u32, other: u32, fee: i64, kind: TransferMoveKind) -> TransferMove {
            TransferMove {
                player_id,
                other_club_id: other,
                fee,
                kind,
                age: 26,
                returning: false,
                was_loan_here: false,
            }
        }

        fn kinds(stories: &[NewsStory]) -> Vec<NewsStoryKind> {
            stories.iter().map(|story| story.kind).collect()
        }

        fn team(id: u32) -> Team {
            TeamBuilder::new()
                .id(id)
                .club_id(1)
                .name("Team".to_string())
                .slug("team".to_string())
                .team_type(TeamType::Main)
                .league_id(Some(1))
                .reputation(TeamReputation::new(500, 500, 500))
                .players(PlayerCollection::new(Vec::new()))
                .staffs(StaffCollection::new(Vec::new()))
                .training_schedule(TrainingSchedule::new(
                    chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                    chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn played(opponent: u32, goals_for: u8, goals_against: u8) -> IssueResult {
            IssueResult {
                date: Self::day(),
                opponent_team_id: opponent,
                goals_for,
                goals_against,
                competition: ResultCompetition::League,
                is_home: true,
            }
        }
    }

    /// The report carries its protagonist: the star recorded for that
    /// team, that opponent and that exact tally rides onto the story,
    /// while a second result the star did not belong to stays about the
    /// club alone.
    #[test]
    fn a_match_report_names_the_man_who_won_it() {
        const TEAM: u32 = 1;
        const OPPONENT: u32 = 40;
        const OTHER_OPPONENT: u32 = 41;

        let mut facts = WeeklyMatchFacts::empty();
        facts.stars.insert(
            (TEAM, OPPONENT),
            MatchStarFacts {
                player_id: 9,
                goals: 2,
                team_goals: 2,
            },
        );

        let mut out = Vec::new();
        MatchDesk::file(
            &mut out,
            &[
                Fixture::played(OPPONENT, 2, 1),
                Fixture::played(OTHER_OPPONENT, 1, 0),
            ],
            &FxHashSet::default(),
            &facts,
            &Fixture::team(TEAM),
        );

        let starred = out
            .iter()
            .find(|story| story.other_id == OPPONENT)
            .expect("the 2-1 was reported");
        assert_eq!(starred.player_id, 9, "the report must carry its scorer");
        assert!(
            starred.home,
            "the report must remember which end of the fixture it was"
        );

        let plain = out
            .iter()
            .find(|story| story.other_id == OTHER_OPPONENT)
            .expect("the 1-0 was reported");
        assert_eq!(
            plain.player_id, 0,
            "a result the week recorded no star for stays about the club"
        );
    }

    #[test]
    fn a_derby_outranks_the_margin() {
        assert_eq!(
            MatchDesk::classify(1, 0, true),
            NewsStoryKind::DerbyWin,
            "a one-goal derby win is still a derby story"
        );
        assert_eq!(MatchDesk::classify(0, 4, true), NewsStoryKind::DerbyDefeat);
        assert_eq!(MatchDesk::classify(4, 0, false), NewsStoryKind::Rout);
        assert_eq!(MatchDesk::classify(0, 3, false), NewsStoryKind::HeavyDefeat);
        assert_eq!(MatchDesk::classify(2, 1, false), NewsStoryKind::LeagueWin);
        assert_eq!(
            MatchDesk::classify(1, 2, false),
            NewsStoryKind::LeagueDefeat
        );
        assert_eq!(MatchDesk::classify(1, 1, false), NewsStoryKind::LeagueDraw);
    }

    /// A 0-0 and a 2-2 are not the same afternoon, and a paper that
    /// writes them the same way is a paper nobody reads twice.
    #[test]
    fn a_goalless_afternoon_is_its_own_report() {
        assert_eq!(
            MatchDesk::classify(0, 0, false),
            NewsStoryKind::GoallessDraw
        );
        assert_eq!(MatchDesk::classify(2, 2, false), NewsStoryKind::LeagueDraw);
        assert_eq!(
            MatchDesk::classify(0, 0, true),
            NewsStoryKind::GoallessDraw,
            "a derby only gets its own report when somebody won it"
        );
    }

    #[test]
    fn the_table_is_not_a_story_in_august() {
        let mut out = Vec::new();
        TableDesk::file(&mut out, Some(Fixture::table(1, 20, 4, 38)), Fixture::day());

        assert!(
            out.is_empty(),
            "four games in, top spot means nothing and no paper leads on it"
        );
    }

    #[test]
    fn a_title_race_and_a_drop_fight_both_reach_the_page() {
        let mut leaders = Vec::new();
        TableDesk::file(
            &mut leaders,
            Some(Fixture::table(2, 20, 24, 38)),
            Fixture::day(),
        );
        assert_eq!(Fixture::kinds(&leaders), vec![NewsStoryKind::TitleCharge]);

        let mut strugglers = Vec::new();
        TableDesk::file(
            &mut strugglers,
            Some(Fixture::table(19, 20, 24, 38)),
            Fixture::day(),
        );
        assert_eq!(
            Fixture::kinds(&strugglers),
            vec![NewsStoryKind::RelegationFight]
        );

        let mut mid = Vec::new();
        TableDesk::file(
            &mut mid,
            Some(Fixture::table(10, 20, 24, 38)),
            Fixture::day(),
        );
        assert!(mid.is_empty(), "mid-table is not news");
    }

    #[test]
    fn a_fee_out_of_scale_with_the_squad_is_record_business() {
        let week = ClubTransferWeek {
            arrivals: vec![
                Fixture::moved(11, 90, 30_000_000, TransferMoveKind::Paid),
                Fixture::moved(12, 91, 2_000_000, TransferMoveKind::Paid),
            ],
            departures: vec![
                Fixture::moved(13, 92, 40_000_000, TransferMoveKind::Paid),
                Fixture::moved(14, 93, 1_000_000, TransferMoveKind::Paid),
            ],
        };

        let mut out = Vec::new();
        MarketDesk::file(&mut out, &week, 10_000_000, Fixture::day());

        assert_eq!(
            Fixture::kinds(&out),
            vec![
                NewsStoryKind::RecordSigning,
                NewsStoryKind::NewSigning,
                NewsStoryKind::StarSold,
                NewsStoryKind::PlayerSold,
            ]
        );
    }

    #[test]
    fn free_transfers_and_loans_are_told_apart_from_purchases() {
        let week = ClubTransferWeek {
            arrivals: vec![
                Fixture::moved(11, 90, 0, TransferMoveKind::Free),
                Fixture::moved(12, 91, 0, TransferMoveKind::Loan),
            ],
            departures: vec![Fixture::moved(13, 92, 0, TransferMoveKind::Loan)],
        };

        let mut out = Vec::new();
        MarketDesk::file(&mut out, &week, 10_000_000, Fixture::day());

        assert_eq!(
            Fixture::kinds(&out),
            vec![
                NewsStoryKind::FreeSigning,
                NewsStoryKind::LoanArrival,
                NewsStoryKind::LoanExit,
            ]
        );
    }

    /// The bug this desk was rebuilt around: a player leaving for
    /// nothing was reported as a sale, and the page printed the fee it
    /// had — "$0.00" — under a headline that said the club had sold him.
    #[test]
    fn a_departure_with_no_fee_is_never_reported_as_a_sale() {
        let week = ClubTransferWeek {
            arrivals: Vec::new(),
            departures: vec![
                Fixture::moved(13, 92, 0, TransferMoveKind::Free),
                Fixture::moved(14, 93, 0, TransferMoveKind::Paid),
            ],
        };

        let mut out = Vec::new();
        MarketDesk::file(&mut out, &week, 10_000_000, Fixture::day());

        for story in &out {
            assert_ne!(
                story.kind,
                NewsStoryKind::PlayerSold,
                "nobody was sold for nothing"
            );
            assert_ne!(story.kind, NewsStoryKind::StarSold);
        }
    }

    /// The framings the enrichment pass unlocks: a loanee kept, a
    /// returning favourite, a teenager for the future, and experience
    /// through the door. Each is its own story, never a generic
    /// "new signing" line.
    #[test]
    fn an_arrival_the_town_already_knows_is_not_a_stranger() {
        let buyout = TransferMove {
            was_loan_here: true,
            ..Fixture::moved(11, 90, 3_000_000, TransferMoveKind::Paid)
        };
        let homecoming = TransferMove {
            returning: true,
            ..Fixture::moved(12, 91, 0, TransferMoveKind::Free)
        };
        let prospect = TransferMove {
            age: 18,
            ..Fixture::moved(13, 92, 500_000, TransferMoveKind::Paid)
        };
        let veteran = TransferMove {
            age: 35,
            ..Fixture::moved(14, 93, 0, TransferMoveKind::Free)
        };

        let week = ClubTransferWeek {
            arrivals: vec![buyout, homecoming, prospect, veteran],
            departures: Vec::new(),
        };

        let mut out = Vec::new();
        MarketDesk::file(&mut out, &week, 100_000_000, Fixture::day());

        assert_eq!(
            Fixture::kinds(&out),
            vec![
                NewsStoryKind::LoanMadePermanent,
                NewsStoryKind::HomecomingSigning,
                NewsStoryKind::ProspectSigned,
                NewsStoryKind::VeteranArrives,
            ]
        );
    }

    /// Record business owns the page whoever the player is — and a
    /// buy-out beats a homecoming, because "we kept him" is the fresher
    /// half of the same story.
    #[test]
    fn record_business_outranks_a_familiar_face() {
        let returning_record = TransferMove {
            returning: true,
            ..Fixture::moved(11, 90, 50_000_000, TransferMoveKind::Paid)
        };
        let buyout_and_returning = TransferMove {
            was_loan_here: true,
            returning: true,
            ..Fixture::moved(12, 91, 2_000_000, TransferMoveKind::Paid)
        };

        let week = ClubTransferWeek {
            arrivals: vec![returning_record, buyout_and_returning],
            departures: Vec::new(),
        };

        let mut out = Vec::new();
        MarketDesk::file(&mut out, &week, 10_000_000, Fixture::day());

        assert_eq!(
            Fixture::kinds(&out),
            vec![
                NewsStoryKind::RecordSigning,
                NewsStoryKind::LoanMadePermanent,
            ]
        );
    }

    /// Age 0 means the newsroom could not resolve the player, and an
    /// unresolved age must never write him up as a teenager.
    #[test]
    fn an_unresolved_age_never_reads_as_a_teenager() {
        let unknown = TransferMove {
            age: 0,
            ..Fixture::moved(11, 90, 2_000_000, TransferMoveKind::Paid)
        };

        let week = ClubTransferWeek {
            arrivals: vec![unknown],
            departures: Vec::new(),
        };

        let mut out = Vec::new();
        MarketDesk::file(&mut out, &week, 100_000_000, Fixture::day());

        assert_eq!(Fixture::kinds(&out), vec![NewsStoryKind::NewSigning]);
    }

    /// The goalkeeper's week, and the order the page tells it in. A
    /// paper never runs "he was magnificent" and "he was at fault"
    /// about the same man on the same day, so exactly one line survives
    /// per keeper — and which one is a judgement about what a reader
    /// remembers, not about which number is biggest.
    #[test]
    fn a_keepers_week_is_told_in_one_line_sourest_first() {
        use crate::club::news::KeeperMatchFacts;
        use crate::club::news::desk::squad::SquadDesk;
        use rustc_hash::FxHashMap;

        let filed = |facts: KeeperMatchFacts| -> Vec<NewsStoryKind> {
            let mut keepers: FxHashMap<(u32, u32), KeeperMatchFacts> = FxHashMap::default();
            keepers.insert((1, Fixture::OUR_SIDE), facts);

            let week = crate::club::news::WeeklyMatchFacts {
                keepers,
                ..crate::club::news::WeeklyMatchFacts::empty()
            };

            let mut out = Vec::new();
            SquadDesk::file_keeper_deeds(&mut out, 1, &Fixture::our_sides(), &week, Fixture::day());
            Fixture::kinds(&out)
        };

        // A shoot-out is remembered by its ending, so the save leads
        // even on a night he was beaten four times in normal time.
        assert_eq!(
            filed(KeeperMatchFacts {
                saves: 7,
                conceded: 4,
                penalties_saved: 2,
                errors_leading_to_goal: 1,
            }),
            vec![NewsStoryKind::KeeperPenaltySave]
        );

        // His own mistake outranks his own shot-stopping.
        assert_eq!(
            filed(KeeperMatchFacts {
                saves: 8,
                conceded: 2,
                penalties_saved: 0,
                errors_leading_to_goal: 1,
            }),
            vec![NewsStoryKind::KeeperBlunder]
        );

        // Eight saves in a 4-0 defeat is a story about the saves. The
        // scoreline belongs to the ten in front of him.
        assert_eq!(
            filed(KeeperMatchFacts {
                saves: 8,
                conceded: 4,
                penalties_saved: 0,
                errors_leading_to_goal: 0,
            }),
            vec![NewsStoryKind::KeeperMasterclass]
        );

        // Beaten four times without having had much to do is the other
        // afternoon entirely.
        assert_eq!(
            filed(KeeperMatchFacts {
                saves: 1,
                conceded: 4,
                penalties_saved: 0,
                errors_leading_to_goal: 0,
            }),
            vec![NewsStoryKind::KeeperOverrun]
        );
    }

    /// An ordinary afternoon is not news. A keeper who made two saves
    /// and picked the ball out once had the week nearly every keeper
    /// has, and a page that reported it would report every one of them.
    #[test]
    fn a_routine_afternoon_between_the_posts_is_not_a_story() {
        use crate::club::news::KeeperMatchFacts;
        use crate::club::news::desk::squad::SquadDesk;
        use rustc_hash::FxHashMap;

        let mut keepers: FxHashMap<(u32, u32), KeeperMatchFacts> = FxHashMap::default();
        keepers.insert(
            (1, Fixture::OUR_SIDE),
            KeeperMatchFacts {
                saves: 2,
                conceded: 1,
                penalties_saved: 0,
                errors_leading_to_goal: 0,
            },
        );

        let week = crate::club::news::WeeklyMatchFacts {
            keepers,
            ..crate::club::news::WeeklyMatchFacts::empty()
        };

        let mut out = Vec::new();
        SquadDesk::file_keeper_deeds(&mut out, 1, &Fixture::our_sides(), &week, Fixture::day());

        assert!(out.is_empty(), "{:?}", Fixture::kinds(&out));
    }

    /// The afternoon a club paper is not entitled to.
    ///
    /// The week's facts are gathered from every competitive fixture in
    /// the world, internationals included, and the club press then walks
    /// its own roster over them. Keyed by player alone, "is he on my
    /// books" was the only question asked — so a keeper's six saves for
    /// his country came out under his club's nameplate, in copy that
    /// names the club ("{player} keeps {club} in it on his own") about
    /// ninety minutes the club did not play. The reader's own check is
    /// the give-away: the match is nowhere on the player's record,
    /// because it was never his club's match.
    #[test]
    fn a_paper_never_claims_an_afternoon_played_for_somebody_else() {
        use crate::club::news::KeeperMatchFacts;
        use crate::club::news::desk::squad::SquadDesk;
        use rustc_hash::FxHashMap;

        let masterclass = KeeperMatchFacts {
            saves: 8,
            conceded: 1,
            penalties_saved: 0,
            errors_leading_to_goal: 0,
        };

        let filed = |side: u32| -> Vec<NewsStoryKind> {
            let mut keepers: FxHashMap<(u32, u32), KeeperMatchFacts> = FxHashMap::default();
            keepers.insert((1, side), masterclass);

            let week = crate::club::news::WeeklyMatchFacts {
                keepers,
                ..crate::club::news::WeeklyMatchFacts::empty()
            };

            let mut out = Vec::new();
            SquadDesk::file_keeper_deeds(&mut out, 1, &Fixture::our_sides(), &week, Fixture::day());
            Fixture::kinds(&out)
        };

        assert_eq!(
            filed(Fixture::OUR_YOUTH_SIDE),
            vec![NewsStoryKind::KeeperMasterclass],
            "a youth keeper's afternoon for one of the club's own sides is the club's news"
        );
        assert!(
            filed(Fixture::SOMEBODY_ELSE).is_empty(),
            "an afternoon played for a national team or a former club is not this club's"
        );
    }

    /// One outfield player, one afternoon, one line — and the order the
    /// page tells it in.
    ///
    /// The precedence is a judgement about what a reader remembers rather
    /// than about which number is biggest, exactly as it is for the
    /// goalkeeper. A paper does not print "the best man on the pitch" and
    /// "his error decided it" about one player on one page.
    #[test]
    fn an_outfield_players_week_is_told_in_one_line_loudest_first() {
        let filed = |facts: OutfieldMatchFacts| -> Vec<NewsStoryKind> {
            Verdicts::filed(PlayerPositionType::Striker, facts, false)
        };

        // A goal in his own net leads over everything he did well.
        assert_eq!(
            filed(OutfieldMatchFacts {
                own_goals: 1,
                best_rating: 830,
                worst_rating: 550,
                man_of_the_match: true,
                ..Verdicts::blank()
            }),
            vec![NewsStoryKind::OwnGoalShame]
        );

        // The spot-kick he missed outranks his mark for the ninety
        // minutes: a shoot-out is remembered by its ending.
        assert_eq!(
            filed(OutfieldMatchFacts {
                penalties_missed: 1,
                best_rating: 780,
                worst_rating: 780,
                ..Verdicts::blank()
            }),
            vec![NewsStoryKind::PenaltyMissed]
        );

        // His mistake, their goal — above a masterclass in the same week.
        assert_eq!(
            filed(OutfieldMatchFacts {
                errors_leading_to_goal: 1,
                best_rating: 820,
                worst_rating: 600,
                ..Verdicts::blank()
            }),
            vec![NewsStoryKind::CostlyError]
        );

        // Nothing went wrong, and the mark is in the eights.
        assert_eq!(
            filed(OutfieldMatchFacts {
                best_rating: 840,
                worst_rating: 840,
                goals: 2,
                man_of_the_match: true,
                ..Verdicts::blank()
            }),
            vec![NewsStoryKind::MatchMasterclass]
        );

        // Man of the match without the eights: the engine's own verdict,
        // which the newsroom recorded and threw away for a year.
        assert_eq!(
            filed(OutfieldMatchFacts {
                best_rating: 760,
                worst_rating: 760,
                man_of_the_match: true,
                ..Verdicts::blank()
            }),
            vec![NewsStoryKind::ManOfTheMatch]
        );

        // Three made for other people is the story of his afternoon.
        assert_eq!(
            filed(OutfieldMatchFacts {
                best_rating: 770,
                worst_rating: 770,
                assists: 3,
                ..Verdicts::blank()
            }),
            vec![NewsStoryKind::AssistShow]
        );

        // The chances were there and none of them went in.
        assert_eq!(
            filed(OutfieldMatchFacts {
                best_rating: 640,
                worst_rating: 640,
                shots: 5,
                xg_x100: 140,
                ..Verdicts::blank()
            }),
            vec![NewsStoryKind::WastefulFinishing]
        );

        // Marked down and taken off: the more specific story wins.
        assert_eq!(
            filed(OutfieldMatchFacts {
                best_rating: 540,
                worst_rating: 540,
                worst_rating_minutes: 55,
                worst_rating_started: true,
                worst_rating_hooked: true,
                ..Verdicts::blank()
            }),
            vec![NewsStoryKind::HookedEarly]
        );

        // …and the same mark without the substitution.
        assert_eq!(
            filed(OutfieldMatchFacts {
                best_rating: 540,
                worst_rating: 540,
                worst_rating_minutes: 90,
                worst_rating_started: true,
                ..Verdicts::blank()
            }),
            vec![NewsStoryKind::MatchStinker]
        );
    }

    /// An ordinary afternoon is not news.
    ///
    /// This is the bar the whole ratings page hangs on: a 6.6 in a 1-1
    /// with two tackles and a shot is the week nearly every outfield
    /// player in the world has, and a page that reported it would report
    /// all of them.
    #[test]
    fn a_routine_afternoon_is_not_a_verdict() {
        assert!(
            Verdicts::filed(
                PlayerPositionType::MidfielderCenter,
                OutfieldMatchFacts {
                    best_rating: 660,
                    worst_rating: 660,
                    worst_rating_minutes: 90,
                    worst_rating_started: true,
                    shots: 1,
                    xg_x100: 15,
                    key_passes: 2,
                    successful_dribbles: 1,
                    defensive_actions: 4,
                    fouls: 1,
                    ..Verdicts::blank()
                },
                false,
            )
            .is_empty(),
            "the median afternoon must leave the page alone"
        );

        // A player who never came off the bench carries an all-zero
        // line, and a mark of nothing is not a bad mark — it is no mark.
        assert!(
            Verdicts::filed(
                PlayerPositionType::Striker,
                OutfieldMatchFacts {
                    ..Verdicts::blank()
                },
                false,
            )
            .is_empty(),
            "an unused substitute has not been marked at all"
        );
    }

    /// The three false positives the detectors are shaped around, each of
    /// which would print a sentence the paper cannot stand behind.
    #[test]
    fn a_verdict_never_reports_something_that_did_not_happen() {
        // A man carried off injured on the half hour has also "been taken
        // off early having been marked down". Reporting it as a
        // manager's verdict invents a decision nobody made — so the
        // substitution reason, not the minute, is what qualifies.
        let injured = OutfieldMatchFacts {
            best_rating: 555,
            worst_rating: 555,
            worst_rating_minutes: 31,
            worst_rating_started: true,
            worst_rating_hooked: false,
            ..Verdicts::blank()
        };
        assert_eq!(
            Verdicts::filed(PlayerPositionType::MidfielderCenter, injured, false),
            vec![NewsStoryKind::MatchStinker],
            "an injury swap must not read as a manager's verdict"
        );

        // A forward who tracked back all afternoon is not the rearguard
        // story, however many times he put a foot in.
        assert!(
            Verdicts::filed(
                PlayerPositionType::Striker,
                OutfieldMatchFacts {
                    best_rating: 745,
                    worst_rating: 745,
                    defensive_actions: 12,
                    shut_out: true,
                    ..Verdicts::blank()
                },
                false,
            )
            .is_empty(),
            "the defensive piece belongs to a defender"
        );

        // The same shift in a hiding is a man drowning, not a man in
        // command — defensive volume RISES when a side is under the cosh,
        // so the clean sheet is what tells the two apart.
        assert!(
            Verdicts::filed(
                PlayerPositionType::DefenderCenter,
                OutfieldMatchFacts {
                    best_rating: 745,
                    worst_rating: 745,
                    defensive_actions: 14,
                    shut_out: false,
                    ..Verdicts::blank()
                },
                false,
            )
            .is_empty(),
            "fourteen clearances in a 4-0 defeat is not a rearguard piece"
        );

        // …and with the clean sheet behind it, it is.
        assert_eq!(
            Verdicts::filed(
                PlayerPositionType::DefenderCenter,
                OutfieldMatchFacts {
                    best_rating: 745,
                    worst_rating: 745,
                    defensive_actions: 11,
                    shut_out: true,
                    ..Verdicts::blank()
                },
                false,
            ),
            vec![NewsStoryKind::DefensiveRock]
        );
    }

    /// Winning a derby is the loudest thing a player can do in a shirt,
    /// and it outranks whatever else the same week did to him.
    #[test]
    fn the_man_who_won_a_derby_leads_his_own_week() {
        assert_eq!(
            Verdicts::filed(
                PlayerPositionType::Striker,
                OutfieldMatchFacts {
                    best_rating: 780,
                    worst_rating: 520,
                    goals: 1,
                    own_goals: 1,
                    ..Verdicts::blank()
                },
                true,
            ),
            vec![NewsStoryKind::DerbyHero]
        );
    }

    /// Every verdict that quotes a mark out of ten has to carry one, or
    /// the editor's own well-formedness gate silently swallows the story.
    /// This is the desk-side half of `a_story_never_quotes_a_figure_it_
    /// does_not_have`.
    #[test]
    fn every_verdict_that_quotes_a_mark_carries_one() {
        let cases = [
            (
                PlayerPositionType::Striker,
                OutfieldMatchFacts {
                    best_rating: 845,
                    worst_rating: 845,
                    ..Verdicts::blank()
                },
            ),
            (
                PlayerPositionType::Striker,
                OutfieldMatchFacts {
                    best_rating: 755,
                    worst_rating: 755,
                    man_of_the_match: true,
                    ..Verdicts::blank()
                },
            ),
            (
                PlayerPositionType::MidfielderCenter,
                OutfieldMatchFacts {
                    best_rating: 525,
                    worst_rating: 525,
                    worst_rating_minutes: 90,
                    worst_rating_started: true,
                    ..Verdicts::blank()
                },
            ),
            (
                PlayerPositionType::MidfielderCenter,
                OutfieldMatchFacts {
                    best_rating: 560,
                    worst_rating: 560,
                    worst_rating_minutes: 48,
                    worst_rating_started: true,
                    worst_rating_hooked: true,
                    ..Verdicts::blank()
                },
            ),
        ];

        for (position, facts) in cases {
            let stories = Verdicts::stories(position, facts, false);
            for story in &stories {
                if story.kind.quotes_a_rating() {
                    assert!(story.b > 0, "{:?} would print a mark of 0.00", story.kind);
                }
            }
            assert_eq!(stories.len(), 1, "one verdict per player per week");
        }
    }

    /// Builds the ratings desk's inputs without a world: an outfield
    /// player at a given position and one week of match facts.
    struct Verdicts;

    impl Verdicts {
        fn blank() -> OutfieldMatchFacts {
            OutfieldMatchFacts::default()
        }

        fn player(position: PlayerPositionType) -> Player {
            PlayerBuilder::new()
                .id(1)
                .full_name(FullName::new("Test".to_string(), "Player".to_string()))
                .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(140))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 20,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap()
        }

        fn stories(
            position: PlayerPositionType,
            facts: OutfieldMatchFacts,
            derby_hero: bool,
        ) -> Vec<NewsStory> {
            let player = Self::player(position);
            let mut outfield: rustc_hash::FxHashMap<(u32, u32), OutfieldMatchFacts> =
                rustc_hash::FxHashMap::default();
            outfield.insert((player.id, Fixture::OUR_SIDE), facts);

            let week = WeeklyMatchFacts {
                outfield,
                ..WeeklyMatchFacts::empty()
            };

            let mut out = Vec::new();
            SquadDesk::file_outfield_deeds(
                &mut out,
                &player,
                &Fixture::our_sides(),
                &week,
                derby_hero,
                Fixture::day(),
            );
            out
        }

        fn filed(
            position: PlayerPositionType,
            facts: OutfieldMatchFacts,
            derby_hero: bool,
        ) -> Vec<NewsStoryKind> {
            Fixture::kinds(&Self::stories(position, facts, derby_hero))
        }
    }

    #[test]
    fn a_bigger_fee_carries_more_weight_than_a_smaller_one() {
        let week = ClubTransferWeek {
            arrivals: vec![
                Fixture::moved(11, 90, 1_000_000, TransferMoveKind::Paid),
                Fixture::moved(12, 91, 25_000_000, TransferMoveKind::Paid),
            ],
            departures: Vec::new(),
        };

        let mut out = Vec::new();
        MarketDesk::file(&mut out, &week, 500_000_000, Fixture::day());

        assert!(
            out[1].priority > out[0].priority,
            "the expensive signing must outrank the cheap one on the page"
        );
    }
}
