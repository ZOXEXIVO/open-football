use super::{NewsStory, NewsStoryDetail, NewsStoryKind, PlayingTimeConcern};
use crate::{TransferInterestSource, TransferInterestStage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewsAngle {
    HatTrickWin,
    HatTrickDraw,
    HatTrickDefeat,
    SubstituteGoal,
    SubstituteAssist,
    SubstituteBoth,
    MoreMinutes,
    FirstTeam,
    Reserves,
    Backup,
    LoanAssists,
    SingleVisit,
    ReturnVisit,
    MultipleVisitors,
    LocalPress,
    NationalPress,
    AgentSounding,
    FormalApproach,
    SubmittedBid,
    RejectedBid,
    Negotiations,
    TermsAgreed,
    TermsRejected,
    CollapsedMove,
}

impl NewsAngle {
    pub const ALL: [Self; 24] = [
        Self::HatTrickWin,
        Self::HatTrickDraw,
        Self::HatTrickDefeat,
        Self::SubstituteGoal,
        Self::SubstituteAssist,
        Self::SubstituteBoth,
        Self::MoreMinutes,
        Self::FirstTeam,
        Self::Reserves,
        Self::Backup,
        Self::LoanAssists,
        Self::SingleVisit,
        Self::ReturnVisit,
        Self::MultipleVisitors,
        Self::LocalPress,
        Self::NationalPress,
        Self::AgentSounding,
        Self::FormalApproach,
        Self::SubmittedBid,
        Self::RejectedBid,
        Self::Negotiations,
        Self::TermsAgreed,
        Self::TermsRejected,
        Self::CollapsedMove,
    ];

    pub fn key_stem(self) -> &'static str {
        match self {
            Self::HatTrickWin => "hat_trick_win",
            Self::HatTrickDraw => "hat_trick_draw",
            Self::HatTrickDefeat => "hat_trick_defeat",
            Self::SubstituteGoal => "super_sub_goal",
            Self::SubstituteAssist => "super_sub_assist",
            Self::SubstituteBoth => "super_sub_both",
            Self::MoreMinutes => "bench_frustration_minutes",
            Self::FirstTeam => "bench_frustration_first_team",
            Self::Reserves => "bench_frustration_reserves",
            Self::Backup => "bench_frustration_backup",
            Self::LoanAssists => "loan_watch_assists",
            Self::SingleVisit => "scouts_watching_single",
            Self::ReturnVisit => "scouts_watching_return",
            Self::MultipleVisitors => "scouts_watching_multiple",
            Self::LocalPress => "transfer_speculation_local_press",
            Self::NationalPress => "transfer_speculation_national_press",
            Self::AgentSounding => "agent_touting_sounding",
            Self::FormalApproach => "rumour_interest_approach",
            Self::SubmittedBid => "rumour_interest_bid",
            Self::RejectedBid => "bid_rejected_confirmed",
            Self::Negotiations => "talks_expected_opened",
            Self::TermsAgreed => "personal_terms_agreed_confirmed",
            Self::TermsRejected => "turns_down_move_confirmed",
            Self::CollapsedMove => "move_collapsed_confirmed",
        }
    }

    fn transfer(source: TransferInterestSource, stage: TransferInterestStage) -> Option<Self> {
        match (source, stage) {
            (TransferInterestSource::LocalPress, _) => Some(Self::LocalPress),
            (TransferInterestSource::NationalPress, _) => Some(Self::NationalPress),
            (TransferInterestSource::AgentLeak, TransferInterestStage::AgentSoundingOut) => {
                Some(Self::AgentSounding)
            }
            (_, TransferInterestStage::ConcreteInterest) => Some(Self::FormalApproach),
            (_, TransferInterestStage::BidSubmitted) => Some(Self::SubmittedBid),
            (_, TransferInterestStage::BidRejected) => Some(Self::RejectedBid),
            (_, TransferInterestStage::NegotiationsOpened) => Some(Self::Negotiations),
            (_, TransferInterestStage::TermsAgreed) => Some(Self::TermsAgreed),
            (_, TransferInterestStage::TermsRejected) => Some(Self::TermsRejected),
            (_, TransferInterestStage::MoveCollapsed) => Some(Self::CollapsedMove),
            _ => None,
        }
    }
}

impl NewsStory {
    /// The situation the desk recorded, independent of display names and language.
    pub fn angle(&self) -> Option<NewsAngle> {
        use NewsAngle::*;
        match (self.kind, self.detail.as_deref()) {
            (NewsStoryKind::HatTrick, Some(NewsStoryDetail::Performance(deed))) => {
                Some(match deed.goals_for.cmp(&deed.goals_against) {
                    std::cmp::Ordering::Greater => HatTrickWin,
                    std::cmp::Ordering::Equal => HatTrickDraw,
                    std::cmp::Ordering::Less => HatTrickDefeat,
                })
            }
            (NewsStoryKind::SuperSub, Some(NewsStoryDetail::Performance(deed))) => {
                match (deed.goals > 0, deed.assists > 0) {
                    (true, true) => Some(SubstituteBoth),
                    (true, false) => Some(SubstituteGoal),
                    (false, true) => Some(SubstituteAssist),
                    _ => None,
                }
            }
            (NewsStoryKind::BenchFrustration, Some(NewsStoryDetail::PlayingTime(concern))) => {
                Some(match concern {
                    PlayingTimeConcern::Minutes => MoreMinutes,
                    PlayingTimeConcern::FirstTeam => FirstTeam,
                    PlayingTimeConcern::Reserves => Reserves,
                    PlayingTimeConcern::Backup => Backup,
                })
            }
            (NewsStoryKind::LoanWatchAssists, _) => Some(LoanAssists),
            (NewsStoryKind::ScoutsWatching, Some(NewsStoryDetail::Sighting(sighting))) => {
                Some(if sighting.other.is_some() {
                    MultipleVisitors
                } else if sighting.returned {
                    ReturnVisit
                } else {
                    SingleVisit
                })
            }
            (_, Some(NewsStoryDetail::TransferInterest { source, stage })) => {
                NewsAngle::transfer(*source, *stage)
            }
            _ => None,
        }
    }
}
