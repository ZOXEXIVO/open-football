use super::facts::{ClubDugoutWatch, SquadPulse};
use crate::Club;
use crate::club::DistressLevel;
use crate::club::board::decision::BoardFacility;
use crate::club::news::affairs::{ClubAffair, ClubAffairLog};
use crate::club::news::types::{NewsStory, NewsStoryKind};
use chrono::NaiveDate;

/// Boardroom, balance sheet, and the two things a chairman is actually
/// judged on: who is in the dugout and what comes out of the academy.
///
/// The dugout arc is read from the club's own diary
/// ([`crate::club::news::affairs`]) rather than from the state a change
/// leaves behind. That distinction is the whole reason the diary exists:
/// an empty manager's seat cannot say whether the man was sacked,
/// poached, or has simply not been replaced yet, and the squad-wide
/// happiness events the desk used to read fire twice per change — once
/// when the caretaker steps up and again when the permanent appointment
/// lands — so the paper ran "part company with the manager" and "new man
/// in the dugout" side by side on both Mondays.
///
/// The silverware beats have no club-level flag behind them either, but
/// they do exist in the players' event feeds, so those still come off
/// the [`SquadPulse`] the squad walk already gathered.
pub struct BoardroomDesk;

impl BoardroomDesk {
    /// A pathway reputation at or above this is worth writing about —
    /// the club is producing, and the town knows the names.
    const ACADEMY_PRAISE_REPUTATION: u8 = 70;
    /// …and it needs graduates behind it, or it is a marketing line
    /// rather than a football story.
    const ACADEMY_PRAISE_GRADUATES: u16 = 6;

    /// A vacancy is not news the week it opens — the sacking is. The
    /// hunt becomes a story once it has run long enough that the club
    /// still not having a manager is itself the point.
    const HUNT_IS_NEWS_AFTER_DAYS: i64 = 7;

    pub fn file(
        out: &mut Vec<NewsStory>,
        club: &Club,
        pulse: &SquadPulse,
        dugout: Option<&ClubDugoutWatch>,
        week_start: NaiveDate,
        date: NaiveDate,
    ) {
        let settled = Self::file_affairs(out, &club.affairs, week_start, date);
        Self::file_hunt(out, club, settled, date);
        Self::file_pursuits(out, dugout, settled, date);
        Self::file_confidence(out, club, settled, date);
        Self::file_silverware(out, pulse, date);
        Self::file_academy(out, club, date);
        Self::file_accounts(out, club, date);
    }

    /// The week's entries from the club's own diary, each one a dated
    /// fact rather than an inference from the state it left behind.
    ///
    /// Returns whether the dugout was left alone this week — the
    /// speculative pieces (the hunt, the pursuit, the manager's future)
    /// stand down on any week it was not, because a paper does not run
    /// "will he survive?" beside the news that he did not.
    fn file_affairs(
        out: &mut Vec<NewsStory>,
        affairs: &ClubAffairLog,
        week_start: NaiveDate,
        date: NaiveDate,
    ) -> bool {
        let mut settled = true;

        for entry in affairs.between(week_start, date) {
            settled &= !entry.affair.is_dugout_change();

            let story = match entry.affair {
                ClubAffair::ManagerSacked { staff_id } => {
                    NewsStory::new(NewsStoryKind::ManagerSacked, date).by_staff(staff_id)
                }
                ClubAffair::ManagerPoached {
                    staff_id,
                    to_club_id,
                } => NewsStory::new(NewsStoryKind::ManagerPoached, date)
                    .by_staff(staff_id)
                    .against(to_club_id),
                ClubAffair::ManagerAppointed {
                    staff_id,
                    from_club_id,
                } => NewsStory::new(NewsStoryKind::NewManagerArrives, date)
                    .by_staff(staff_id)
                    .against(from_club_id),
                ClubAffair::CaretakerAppointed { staff_id } => {
                    NewsStory::new(NewsStoryKind::CaretakerTakesCharge, date).by_staff(staff_id)
                }
                ClubAffair::CaretakerConfirmed { staff_id } => {
                    NewsStory::new(NewsStoryKind::CaretakerConfirmed, date).by_staff(staff_id)
                }
                ClubAffair::ManagerContractExtended { staff_id } => {
                    NewsStory::new(NewsStoryKind::ManagerContractExtended, date).by_staff(staff_id)
                }
                ClubAffair::ManagerUltimatum { staff_id } => {
                    NewsStory::new(NewsStoryKind::ManagerUltimatum, date).by_staff(staff_id)
                }
                ClubAffair::TakeoverRumour => NewsStory::new(NewsStoryKind::TakeoverRumour, date),
                ClubAffair::TakeoverCompleted => {
                    NewsStory::new(NewsStoryKind::TakeoverCompleted, date)
                }
                ClubAffair::TakeoverCollapsed => {
                    NewsStory::new(NewsStoryKind::TakeoverCollapsed, date)
                }
                ClubAffair::StadiumExpansion { capacity } => {
                    NewsStory::new(NewsStoryKind::StadiumExpansion, date)
                        .with_numbers(capacity as i32, 0)
                }
                ClubAffair::FacilityUpgrade { facility } => {
                    NewsStory::new(NewsStoryKind::FacilityUpgrade, date)
                        // Which facility, so the fact survives even
                        // though every one of them shares a piece of
                        // copy. Weighted so the training ground — the
                        // one supporters argue about — outranks the
                        // scouting network.
                        .with_numbers(Self::facility_index(facility), 0)
                        .weighted(Self::facility_weight(facility))
                }
                ClubAffair::WarChest { amount } => {
                    NewsStory::new(NewsStoryKind::WarChest, date).with_money(amount)
                }
                ClubAffair::BudgetCut { amount } => {
                    NewsStory::new(NewsStoryKind::BudgetCut, date).with_money(amount)
                }
                ClubAffair::PromiseBroken => {
                    NewsStory::new(NewsStoryKind::BoardPromiseBroken, date)
                }
            };

            out.push(story);
        }

        settled
    }

    /// The vacancy itself. A club with no manager is a story every week
    /// it stays that way, which is exactly why it is `Standing` and
    /// waits on the back catalogue rather than on the calendar.
    fn file_hunt(out: &mut Vec<NewsStory>, club: &Club, settled: bool, date: NaiveDate) {
        if !settled {
            return;
        }
        let Some(since) = club.board.manager_search_since else {
            return;
        };

        let days = (date - since).num_days();
        if days < Self::HUNT_IS_NEWS_AFTER_DAYS {
            return;
        }

        out.push(
            NewsStory::new(NewsStoryKind::ManagerHunt, date)
                .with_numbers(days.min(i32::MAX as i64) as i32, 0)
                // The longer it drags the louder it gets: a fortnight
                // without a manager is a note, two months is a crisis.
                .weighted((days / 7).min(12) as i32 * 10),
        );
    }

    /// Somebody else's manager, or somebody else after ours. Two very
    /// different weeks off the same registry of in-flight approaches.
    fn file_pursuits(
        out: &mut Vec<NewsStory>,
        dugout: Option<&ClubDugoutWatch>,
        settled: bool,
        date: NaiveDate,
    ) {
        let Some(watch) = dugout else {
            return;
        };

        // Being asked about is news whatever else happened; asking is
        // only news while the seat is genuinely still open, and a club
        // that filled it this week is no longer chasing anybody.
        if let Some(pursuit) = watch.headline_pursuit(false) {
            out.push(
                NewsStory::new(NewsStoryKind::ManagerWanted, date)
                    .by_staff(pursuit.staff_id)
                    .against(pursuit.other_club_id),
            );
        }
        if settled {
            if let Some(pursuit) = watch.headline_pursuit(true) {
                out.push(
                    NewsStory::new(NewsStoryKind::ManagerTargetLinked, date)
                        .by_staff(pursuit.staff_id)
                        .against(pursuit.other_club_id),
                );
            }
        }
    }

    fn file_confidence(out: &mut Vec<NewsStory>, club: &Club, settled: bool, date: NaiveDate) {
        // Speculation about a manager's future is not a story in the
        // week he has already gone, or the week his successor walked in.
        if !settled {
            return;
        }

        let confidence = club.board.confidence.level;

        if club.board.manager_on_final_warning || confidence <= 30 {
            let urgency = if club.board.manager_on_final_warning {
                120
            } else {
                (35 - confidence).max(0) * 4
            };
            out.push(
                NewsStory::new(NewsStoryKind::ManagerPressure, date)
                    .with_numbers(confidence, 0)
                    .weighted(urgency),
            );
        } else if confidence >= 82 {
            out.push(NewsStory::new(NewsStoryKind::BoardBacking, date).with_numbers(confidence, 0));
        }
    }

    /// The season's verdicts. Silverware, promotion and relegation can
    /// coexist in one strange week (a title won IS a promotion), so each
    /// files independently and the editor's ranking sorts the page.
    fn file_silverware(out: &mut Vec<NewsStory>, pulse: &SquadPulse, date: NaiveDate) {
        if pulse.trophy {
            out.push(NewsStory::new(NewsStoryKind::TrophyWon, date));
        }
        if pulse.promotion {
            out.push(NewsStory::new(NewsStoryKind::PromotionWon, date));
        }
        if pulse.relegated {
            out.push(NewsStory::new(NewsStoryKind::RelegationConfirmed, date));
        }
        if pulse.cup_final_lost {
            out.push(NewsStory::new(NewsStoryKind::CupFinalHeartbreak, date));
        }
        if pulse.europe {
            out.push(NewsStory::new(NewsStoryKind::EuropeSecured, date));
        }
    }

    fn file_academy(out: &mut Vec<NewsStory>, club: &Club, date: NaiveDate) {
        let reputation = club.academy.pathway_reputation;
        let graduates = club.academy.graduates_produced;

        if reputation < Self::ACADEMY_PRAISE_REPUTATION
            || graduates < Self::ACADEMY_PRAISE_GRADUATES
        {
            return;
        }

        out.push(
            NewsStory::new(NewsStoryKind::AcademyPraise, date)
                .with_numbers(graduates as i32, club.academy.level() as i32)
                .weighted((reputation as i32 - Self::ACADEMY_PRAISE_REPUTATION as i32) * 2),
        );
    }

    fn file_accounts(out: &mut Vec<NewsStory>, club: &Club, date: NaiveDate) {
        let severity = match club.finance.distress_level {
            DistressLevel::Insolvency => 220,
            DistressLevel::Severe => 130,
            DistressLevel::Distress => 50,
            DistressLevel::None => 0,
        };
        if severity > 0 {
            out.push(
                NewsStory::new(NewsStoryKind::MoneyWorries, date)
                    .with_numbers(severity, 0)
                    .weighted(severity),
            );
        }
    }

    /// Stable ordinal for a facility, carried in the story so the fact
    /// survives even though every facility shares one piece of copy.
    fn facility_index(facility: BoardFacility) -> i32 {
        match facility {
            BoardFacility::Training => 1,
            BoardFacility::Youth => 2,
            BoardFacility::Academy => 3,
            BoardFacility::Recruitment => 4,
            BoardFacility::Stadium => 5,
        }
    }

    /// How much a supporter cares. New training pitches get talked
    /// about; a better scouting database does not.
    fn facility_weight(facility: BoardFacility) -> i32 {
        match facility {
            BoardFacility::Training => 30,
            BoardFacility::Academy | BoardFacility::Youth => 20,
            BoardFacility::Stadium => 10,
            BoardFacility::Recruitment => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::facts::ManagerPursuit;
    use super::*;
    use crate::club::news::affairs::ClubAffairLog;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, d).unwrap()
    }

    fn kinds(stories: &[NewsStory]) -> Vec<NewsStoryKind> {
        stories.iter().map(|story| story.kind).collect()
    }

    /// The bug the diary was built to fix. A sacking and the caretaker
    /// stepping up happen on the same morning; the permanent appointment
    /// lands three weeks later. Those are three pieces of news across two
    /// editions — and the squad-event pair the desk used to read fired on
    /// both mornings, so it printed "part company with the manager" and
    /// "new man in the dugout" side by side, twice, and could not have
    /// said which of the two had actually happened either time.
    #[test]
    fn a_sacking_and_the_appointment_that_follows_it_are_different_weeks() {
        let mut log = ClubAffairLog::new();
        log.record(ClubAffair::ManagerSacked { staff_id: 11 }, day(2));
        log.record(ClubAffair::CaretakerAppointed { staff_id: 12 }, day(2));
        log.record(
            ClubAffair::ManagerAppointed {
                staff_id: 13,
                from_club_id: 0,
            },
            day(23),
        );

        let mut sacking_week = Vec::new();
        let settled = BoardroomDesk::file_affairs(&mut sacking_week, &log, day(1), day(8));
        assert!(!settled, "the dugout changed hands this week");
        assert_eq!(
            kinds(&sacking_week),
            vec![
                NewsStoryKind::ManagerSacked,
                NewsStoryKind::CaretakerTakesCharge
            ],
            "the permanent appointment is three weeks away and is not this week's news"
        );
        assert_eq!(
            sacking_week[0].staff_id, 11,
            "the paper names the man who went"
        );
        assert_eq!(sacking_week[1].staff_id, 12, "and the man minding the shop");

        let mut appointment_week = Vec::new();
        BoardroomDesk::file_affairs(&mut appointment_week, &log, day(22), day(29));
        assert_eq!(
            kinds(&appointment_week),
            vec![NewsStoryKind::NewManagerArrives],
            "by the week the successor arrives, the sacking is old news"
        );
        assert_eq!(appointment_week[0].staff_id, 13);
    }

    /// Losing a manager to a bigger club and being talked into sacking
    /// one are opposite mornings. Both used to arrive at the desk as the
    /// same squad-wide event and print the same headline.
    #[test]
    fn a_poach_is_never_reported_as_a_sacking() {
        let mut log = ClubAffairLog::new();
        log.record(
            ClubAffair::ManagerPoached {
                staff_id: 4,
                to_club_id: 77,
            },
            day(3),
        );

        let mut out = Vec::new();
        BoardroomDesk::file_affairs(&mut out, &log, day(1), day(8));

        assert_eq!(kinds(&out), vec![NewsStoryKind::ManagerPoached]);
        assert_eq!(out[0].staff_id, 4);
        assert_eq!(out[0].other_id, 77, "and it names the club that took him");
    }

    /// The board's money is only a story when there is some. A figure
    /// that never arrived would print as "$0.00" — the sentence this
    /// system has been caught making twice.
    #[test]
    fn the_boardrooms_money_carries_the_figure_it_quotes() {
        let mut log = ClubAffairLog::new();
        log.record(ClubAffair::WarChest { amount: 12_000_000 }, day(3));
        log.record(ClubAffair::BudgetCut { amount: 4_000_000 }, day(4));

        let mut out = Vec::new();
        BoardroomDesk::file_affairs(&mut out, &log, day(1), day(8));

        for story in &out {
            assert!(
                story.money > 0,
                "{:?} declares a fee and must carry one",
                story.kind
            );
            assert!(story.kind.quotes_a_fee());
        }
    }

    /// A week in which the manager changed is not a week for wondering
    /// whether he will survive — nor for reporting that the club is
    /// chasing somebody, when it has just stopped.
    #[test]
    fn the_speculation_stands_down_the_week_the_dugout_changes() {
        let watch = ClubDugoutWatch {
            pursuits: vec![ManagerPursuit {
                staff_id: 5,
                other_club_id: 9,
                we_are_asking: true,
            }],
        };

        let mut busy = Vec::new();
        BoardroomDesk::file_pursuits(&mut busy, Some(&watch), false, day(9));
        assert!(
            busy.is_empty(),
            "a club that filled its seat this week is not still chasing anybody"
        );

        let mut settled = Vec::new();
        BoardroomDesk::file_pursuits(&mut settled, Some(&watch), true, day(9));
        assert_eq!(kinds(&settled), vec![NewsStoryKind::ManagerTargetLinked]);
    }

    /// Being asked about is news whatever else happened — the club has
    /// no say in it, and its supporters find out from the papers.
    #[test]
    fn an_approach_for_our_manager_is_news_even_in_a_busy_week() {
        let watch = ClubDugoutWatch {
            pursuits: vec![ManagerPursuit {
                staff_id: 5,
                other_club_id: 9,
                we_are_asking: false,
            }],
        };

        let mut out = Vec::new();
        BoardroomDesk::file_pursuits(&mut out, Some(&watch), false, day(9));

        assert_eq!(kinds(&out), vec![NewsStoryKind::ManagerWanted]);
        assert_eq!(out[0].staff_id, 5);
        assert_eq!(out[0].other_id, 9);
    }
}
