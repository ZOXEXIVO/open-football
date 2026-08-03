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
    Absorbing, CareerRecord, ClubDugoutWatch, ClubLoanWatch, ClubTransferWeek, ContinentalNight,
    CupTie, KeeperMatchFacts, LoanWatchEntry, ManagerPursuit, MatchDramaFacts, MatchStarFacts,
    OutfieldMatchFacts, PlayerStanding, PlayoffTie, RecentEvents, SquadPulse, StandingSnapshot,
    TransferMotive, TransferMove, TransferMoveKind, WeeklyMatchFacts,
};
pub use fans::{FansDesk, TownMood};
pub use loan::LoanDesk;
pub use market::MarketDesk;
pub use pitch::{MatchDesk, TableDesk};
pub use rumour::RumourDesk;
pub use squad::SquadDesk;

#[cfg(test)]
mod tests {
    use super::facts::{
        ClubTransferWeek, MatchStarFacts, OutfieldMatchFacts, StandingSnapshot, TransferMotive,
        TransferMove, TransferMoveKind, WeeklyMatchFacts,
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
                motive: TransferMotive::Unknown,
                scout_confidence_pct: 0,
                scout_urged_it: false,
                rival: false,
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

    /// One afternoon gets one sidebar, and it is always the biggest
    /// angle available.
    ///
    /// A 4-3 won from two down, in stoppage time, with ten men and six
    /// goals in it is five true stories about the same ninety minutes.
    /// Printing all five is what a machine would do; a paper picks the
    /// one a supporter would lead with, and the plain report is still
    /// there underneath it.
    #[test]
    fn one_afternoon_earns_one_sidebar_and_it_is_the_biggest_angle() {
        use super::facts::MatchDramaFacts;

        const TEAM: u32 = 1;
        const OPPONENT: u32 = 40;

        let file = |drama: MatchDramaFacts, goals_for: u8, goals_against: u8| {
            let mut facts = WeeklyMatchFacts::empty();
            facts.drama.insert((TEAM, OPPONENT), drama);

            let mut out = Vec::new();
            MatchDesk::file(
                &mut out,
                &[Fixture::played(OPPONENT, goals_for, goals_against)],
                &FxHashSet::default(),
                &facts,
                &Fixture::team(TEAM),
            );
            Fixture::kinds(&out)
        };

        let everything = MatchDramaFacts {
            team_goals: 4,
            total_goals: 7,
            winner_minute: 91,
            max_deficit: 2,
            max_lead: 1,
            early_goals: 2,
            reply_minutes: 2,
            red_card: true,
            won: true,
        };

        let kinds = file(everything, 4, 3);
        let drama: Vec<NewsStoryKind> = kinds
            .iter()
            .copied()
            .filter(|kind| *kind != NewsStoryKind::LeagueWin)
            .collect();
        assert_eq!(
            drama,
            vec![NewsStoryKind::ComebackWin],
            "the comeback is the story; the other four angles are not printed alongside it"
        );

        // Strip the comeback and the next angle down takes over, all
        // the way to an afternoon with no shape worth a sidebar.
        let ladder = [
            (
                MatchDramaFacts {
                    max_deficit: 0,
                    ..everything
                },
                Some(NewsStoryKind::StoppageTimeDrama),
            ),
            (
                MatchDramaFacts {
                    max_deficit: 0,
                    winner_minute: 87,
                    ..everything
                },
                Some(NewsStoryKind::LateWinner),
            ),
            (
                MatchDramaFacts {
                    max_deficit: 0,
                    winner_minute: 20,
                    ..everything
                },
                Some(NewsStoryKind::TenManWin),
            ),
            (
                MatchDramaFacts {
                    max_deficit: 0,
                    winner_minute: 20,
                    red_card: false,
                    ..everything
                },
                Some(NewsStoryKind::GoalFest),
            ),
            (
                MatchDramaFacts {
                    max_deficit: 0,
                    winner_minute: 20,
                    red_card: false,
                    total_goals: 3,
                    ..everything
                },
                Some(NewsStoryKind::EarlyBlitz),
            ),
            (
                MatchDramaFacts {
                    max_deficit: 0,
                    winner_minute: 20,
                    red_card: false,
                    total_goals: 3,
                    early_goals: 1,
                    ..everything
                },
                Some(NewsStoryKind::InstantReply),
            ),
            (
                MatchDramaFacts {
                    max_deficit: 0,
                    winner_minute: 20,
                    red_card: false,
                    total_goals: 3,
                    early_goals: 1,
                    reply_minutes: 0,
                    ..everything
                },
                None,
            ),
        ];

        for (drama, expected) in ladder {
            let kinds = file(drama, 4, 3);
            let found: Vec<NewsStoryKind> = kinds
                .iter()
                .copied()
                .filter(|kind| *kind != NewsStoryKind::LeagueWin)
                .collect();
            assert_eq!(
                found,
                expected.into_iter().collect::<Vec<_>>(),
                "wrong angle for {:?}",
                drama
            );
        }
    }

    /// The pin that keeps a double fixture honest: drama recorded for
    /// one afternoon must not be printed under the other's scoreline.
    #[test]
    fn a_second_meeting_does_not_borrow_the_firsts_drama() {
        use super::facts::MatchDramaFacts;

        const TEAM: u32 = 1;
        const OPPONENT: u32 = 40;

        let mut facts = WeeklyMatchFacts::empty();
        facts.drama.insert(
            (TEAM, OPPONENT),
            MatchDramaFacts {
                team_goals: 4,
                total_goals: 7,
                max_deficit: 2,
                won: true,
                ..Default::default()
            },
        );

        let mut out = Vec::new();
        MatchDesk::file(
            &mut out,
            // Same opponent, a different afternoon: 1-0, not 4-3.
            &[Fixture::played(OPPONENT, 1, 0)],
            &FxHashSet::default(),
            &facts,
            &Fixture::team(TEAM),
        );

        assert_eq!(
            Fixture::kinds(&out),
            vec![NewsStoryKind::LeagueWin],
            "the 4-3's comeback must not be told about the 1-0"
        );
    }

    /// An absence is named where a supporter would recognise the name,
    /// and stays generic where naming it would be precision nobody
    /// asked for.
    ///
    /// "Out for eleven weeks" is a squad note; "he has done his
    /// cruciate" is news, and the engine has always recorded which of
    /// the two it was. Only the three families a supporter can picture
    /// are split out — a dead leg stays the generic blow, because a
    /// paper that names every knock is a medical bulletin.
    #[test]
    fn an_injury_is_named_only_where_the_name_means_something() {
        use crate::InjuryType;

        let filed = |injury: Option<InjuryType>| -> NewsStoryKind {
            let mut player = Verdicts::player(PlayerPositionType::Striker);
            player.player_attributes.is_injured = true;
            player.player_attributes.injury_days_remaining = 90;
            player.player_attributes.injury_type = injury;
            SquadDesk::injury_kind(&player)
        };

        assert_eq!(
            filed(Some(InjuryType::HamstringStrain)),
            NewsStoryKind::HamstringBlow
        );
        assert_eq!(
            filed(Some(InjuryType::ACLTear)),
            NewsStoryKind::KneeLigamentBlow
        );
        assert_eq!(
            filed(Some(InjuryType::BrokenLeg)),
            NewsStoryKind::BrokenBoneBlow
        );

        // A knock with no recognisable name, and a record with no
        // injury type at all, both stay the piece that was always
        // there rather than inventing a diagnosis.
        assert_eq!(
            filed(Some(InjuryType::BackSpasm)),
            NewsStoryKind::InjuryBlow
        );
        assert_eq!(filed(None), NewsStoryKind::InjuryBlow);
    }

    /// A loan finally says how it went.
    ///
    /// The verdict is recorded at the moment of return — it has to be,
    /// because the borrowing season's numbers are frozen and reset
    /// seconds later — and the homecoming piece printed the same "he is
    /// back" for a player who started every week and one who never got
    /// off the bench. Those are opposite outcomes for the club that
    /// sent him.
    #[test]
    fn a_homecoming_says_whether_the_loan_worked() {
        use crate::LoanSpellVerdict as How;

        assert_eq!(
            SquadDesk::homecoming_kind(How::Standout),
            NewsStoryKind::LoanReturnTriumph
        );
        assert_eq!(
            SquadDesk::homecoming_kind(How::Successful),
            NewsStoryKind::LoanReturnTriumph
        );
        assert_eq!(
            SquadDesk::homecoming_kind(How::Peripheral),
            NewsStoryKind::LoanReturnWasted
        );
        assert_eq!(
            SquadDesk::homecoming_kind(How::Struggled),
            NewsStoryKind::LoanReturnWasted
        );

        // A spell too short to read is not a verdict, and inventing one
        // from a small sample is exactly what the record refuses to do.
        assert_eq!(
            SquadDesk::homecoming_kind(How::Inconclusive),
            NewsStoryKind::LoanReturn
        );
        assert_eq!(
            SquadDesk::homecoming_kind(How::Steady),
            NewsStoryKind::LoanReturn
        );
    }

    /// A European night is reported as one.
    ///
    /// Continental results have always reached club papers — they go
    /// into the same global store the weekly gather walks, which is why
    /// a European hat-trick was reported correctly all along. What was
    /// missing was the label: every one of those midweeks was filed as
    /// an ordinary league game, so a club could beat the continent's
    /// best on a Wednesday and read about it as a routine Saturday.
    #[test]
    fn a_european_night_is_not_filed_as_a_league_saturday() {
        const TEAM: u32 = 1;
        const OPPONENT: u32 = 70;

        let filed = |goals_for: u8, goals_against: u8| -> NewsStoryKind {
            let mut out = Vec::new();
            MatchDesk::file(
                &mut out,
                &[IssueResult {
                    date: Fixture::day(),
                    opponent_team_id: OPPONENT,
                    goals_for,
                    goals_against,
                    competition: ResultCompetition::Continental,
                    is_home: true,
                }],
                &FxHashSet::default(),
                &WeeklyMatchFacts::empty(),
                &Fixture::team(TEAM),
            );
            out[0].kind
        };

        assert_eq!(filed(2, 1), NewsStoryKind::ContinentalNightWin);
        assert_eq!(filed(1, 1), NewsStoryKind::ContinentalNightWin);
        assert_eq!(filed(1, 2), NewsStoryKind::ContinentalDefeat);
        assert_eq!(filed(4, 0), NewsStoryKind::ContinentalRout);
        assert_eq!(filed(0, 3), NewsStoryKind::ContinentalHiding);

        // The same scorelines on a domestic Saturday stay domestic —
        // this is a label on the fixture, not a new way of reading a
        // margin.
        let mut league = Vec::new();
        MatchDesk::file(
            &mut league,
            &[Fixture::played(OPPONENT, 2, 1)],
            &FxHashSet::default(),
            &WeeklyMatchFacts::empty(),
            &Fixture::team(TEAM),
        );
        assert_eq!(Fixture::kinds(&league), vec![NewsStoryKind::LeagueWin]);
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

    /// The club's own reason for a signing, and where it sits against
    /// the framings that were already there.
    ///
    /// A motive is worth a headline where the alternative is "the club
    /// have signed a player", and worth nothing where a better sentence
    /// already exists — a raid on a rival, an academy graduate and a
    /// teenager are all told better by what they are than by why the
    /// club did it.
    #[test]
    fn a_signings_motive_is_printed_where_it_beats_the_plain_report() {
        let filed = |arrival: TransferMove| -> NewsStoryKind {
            let week = ClubTransferWeek {
                arrivals: vec![arrival],
                departures: Vec::new(),
            };
            let mut out = Vec::new();
            MarketDesk::file(&mut out, &week, 100_000_000, Fixture::day());
            out[0].kind
        };

        let plain = |motive: TransferMotive| TransferMove {
            motive,
            ..Fixture::moved(11, 90, 2_000_000, TransferMoveKind::Paid)
        };

        assert_eq!(
            filed(plain(TransferMotive::Succession)),
            NewsStoryKind::SuccessionSigning
        );
        assert_eq!(
            filed(plain(TransferMotive::FormationGap)),
            NewsStoryKind::GapPlugged
        );
        assert_eq!(
            filed(plain(TransferMotive::DepthCover)),
            NewsStoryKind::DepthSigning
        );
        assert_eq!(
            filed(plain(TransferMotive::QualityUpgrade)),
            NewsStoryKind::MarqueeUpgrade
        );
        assert_eq!(
            filed(plain(TransferMotive::Unknown)),
            NewsStoryKind::NewSigning,
            "a move with nothing to say about itself stays the ordinary report"
        );

        // A raid outranks whatever the club's own paperwork says.
        assert_eq!(
            filed(TransferMove {
                rival: true,
                ..plain(TransferMotive::DepthCover)
            }),
            NewsStoryKind::RivalRaid
        );

        // …as do the academy and the age framings.
        assert_eq!(
            filed(plain(TransferMotive::AcademyPromotion)),
            NewsStoryKind::AcademyGraduate
        );
        assert_eq!(
            filed(TransferMove {
                age: 18,
                ..plain(TransferMotive::QualityUpgrade)
            }),
            NewsStoryKind::ProspectSigned,
            "a teenager is told by his age, not by the club's reasoning"
        );

        // The two money framings must never reach the editor without a
        // fee behind them, or the printability gate drops the arrival
        // from the paper altogether.
        assert_eq!(
            filed(TransferMove {
                fee: 0,
                kind: TransferMoveKind::Free,
                ..plain(TransferMotive::QualityUpgrade)
            }),
            NewsStoryKind::FreeSigning
        );
        assert_eq!(
            filed(TransferMove {
                fee: 0,
                kind: TransferMoveKind::Free,
                ..plain(TransferMotive::Bargain)
            }),
            NewsStoryKind::FreeSigning
        );
    }

    /// The scout's verdict prints only when a report actually recorded
    /// one — "the scouts were 0 per cent sure" is the class of sentence
    /// this paper does not print.
    #[test]
    fn the_scouts_verdict_needs_a_scout() {
        let filed = |arrival: TransferMove| -> (NewsStoryKind, i32) {
            let week = ClubTransferWeek {
                arrivals: vec![arrival],
                departures: Vec::new(),
            };
            let mut out = Vec::new();
            MarketDesk::file(&mut out, &week, 100_000_000, Fixture::day());
            (out[0].kind, out[0].a)
        };

        let base = Fixture::moved(11, 90, 2_000_000, TransferMoveKind::Paid);

        let (kind, figure) = filed(TransferMove {
            scout_urged_it: true,
            scout_confidence_pct: 78,
            ..base
        });
        assert_eq!(kind, NewsStoryKind::ScoutingCoup);
        assert_eq!(figure, 78, "the piece is about how sure he was");

        assert_eq!(
            filed(TransferMove {
                scout_urged_it: true,
                scout_confidence_pct: 20,
                ..base
            })
            .0,
            NewsStoryKind::NewSigning,
            "a hunch is not a verdict"
        );

        assert_eq!(
            filed(TransferMove {
                motive: TransferMotive::ScoutFind,
                scout_confidence_pct: 0,
                ..base
            })
            .0,
            NewsStoryKind::NewSigning,
            "a recommendation with no report behind it has no figure to quote"
        );
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
                goals: 1,
                man_of_the_match: true,
                ..Verdicts::blank()
            }),
            vec![NewsStoryKind::MatchMasterclass]
        );

        // …but two goals outrank the mark that came with them. "He was
        // magnificent" is a verdict; "he scored twice" is what
        // happened, and a column that led on the first while holding
        // the second was burying its own story.
        assert_eq!(
            filed(OutfieldMatchFacts {
                best_rating: 840,
                worst_rating: 840,
                goals: 2,
                man_of_the_match: true,
                ..Verdicts::blank()
            }),
            vec![NewsStoryKind::BraceHero]
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
                false,
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
