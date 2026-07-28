use super::facts::SquadPulse;
use crate::Club;
use crate::club::DistressLevel;
use crate::club::news::types::{NewsStory, NewsStoryKind};
use chrono::NaiveDate;

/// Boardroom, balance sheet, and the two things a chairman is actually
/// judged on: who is in the dugout and what comes out of the academy.
///
/// The dugout and silverware beats have no club-level flag behind them —
/// they exist only in the players' event feeds — so this desk reads them
/// off the [`SquadPulse`] the squad walk already gathered rather than
/// walking the roster again for itself.
pub struct BoardroomDesk;

impl BoardroomDesk {
    /// A pathway reputation at or above this is worth writing about —
    /// the club is producing, and the town knows the names.
    const ACADEMY_PRAISE_REPUTATION: u8 = 70;
    /// …and it needs graduates behind it, or it is a marketing line
    /// rather than a football story.
    const ACADEMY_PRAISE_GRADUATES: u16 = 6;

    pub fn file(out: &mut Vec<NewsStory>, club: &Club, pulse: &SquadPulse, date: NaiveDate) {
        Self::file_dugout(out, pulse, date);
        Self::file_confidence(out, club, pulse, date);
        Self::file_silverware(out, pulse, date);
        Self::file_academy(out, club, date);
        Self::file_accounts(out, club, date);
    }

    /// A change of manager is the biggest thing a local paper prints all
    /// season.
    fn file_dugout(out: &mut Vec<NewsStory>, pulse: &SquadPulse, date: NaiveDate) {
        if pulse.manager_left {
            out.push(NewsStory::new(NewsStoryKind::ManagerSacked, date));
        }
        if pulse.new_manager {
            out.push(NewsStory::new(NewsStoryKind::NewManagerArrives, date));
        }
    }

    fn file_confidence(out: &mut Vec<NewsStory>, club: &Club, pulse: &SquadPulse, date: NaiveDate) {
        // Speculation about a manager's future is not a story in the
        // week he has already gone, or the week his successor walked in.
        if !pulse.is_dugout_settled() {
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
}
