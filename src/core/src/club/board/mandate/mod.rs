//! What the board signed the cheque for, and what it is willing to do
//! with money.
//!
//! A fee is the board's answer to a purpose, and the purpose outlives the
//! deal. [`SigningMandate`] is that answer in one object: it is stamped on
//! the negotiation when the club opens it, travels to the player's
//! [`crate::PlayerPlan`] at completion, and is read by every pass that used
//! to derive "what is he for" from age, fee or idle days.
//!
//! [`doctrine`] is the board's side of it — the price it will pay for a
//! purpose, the hearing that approves a fee, and what it will take back
//! when the same player leaves. [`ledger`] is what it remembers afterwards.

pub mod doctrine;
pub mod ledger;

use chrono::{Duration, NaiveDate};

use crate::club::player::contract::PlayerSquadStatus;
use crate::transfers::pipeline::{TransferNeedReason, TransferRequest};
use crate::transfers::squad::plan::{BriefSlot, BriefTier};
use crate::{PathwayStage, PlayerFieldPositionGroup};

pub use doctrine::{FeeEnvelope, MandateStretch, Reservation, ReservationVerdict, TargetBelief};
pub use ledger::{MandateExit, MandateLedger, MandateOutcome};

/// The man whose shirt a request is about, when the club named one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MandateIncumbent {
    pub player_id: u32,
    pub age: u8,
}

/// Who put the name in front of the board. A mandate the manager authored
/// is one his own record is judged on when the minutes never arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MandateAuthor {
    Manager,
    DirectorOfFootball,
    Board,
}

/// What the club bought him for.
///
/// Every purpose is a claim about MINUTES — which is why the purpose and
/// the curve below are the same decision stated twice, and why nothing
/// downstream needs a per-purpose branch to act on one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MandatePurpose {
    /// Bought to be in the side now.
    Starter,
    /// Bought to take a named man's shirt when he is finished with it.
    Heir {
        incumbent_id: u32,
        handover: NaiveDate,
    },
    /// Bought to share the shirt.
    Rotation,
    /// Bought so the shirt is never empty.
    Cover,
    /// Bought to own — the minutes follow the depth chart.
    Asset,
    /// Bought for what he will be.
    Prospect,
}

impl MandatePurpose {
    /// The purpose a request carries, read off the motive the club raised
    /// it with and how transformative it means the signing to be.
    ///
    /// A table on the purpose, not a ladder at the call site: the request
    /// says why and how big, and the purpose is the pair.
    pub fn from_request(
        request: &TransferRequest,
        slot: Option<&BriefSlot>,
        date: NaiveDate,
    ) -> Self {
        let tier = slot.map(|s| s.tier).unwrap_or(request.tier);
        match request.reason {
            TransferNeedReason::SuccessionPlanning => match request.incumbent {
                Some(incumbent) => MandatePurpose::Heir {
                    incumbent_id: incumbent.player_id,
                    handover: Self::handover_for(
                        request.position.position_group(),
                        incumbent.age,
                        date,
                    ),
                },
                // A succession search with no named incumbent is a club
                // grooming somebody for a shirt it has not decided about.
                None => MandatePurpose::Prospect,
            },
            TransferNeedReason::DevelopmentSigning => MandatePurpose::Prospect,
            TransferNeedReason::SquadInvestment => MandatePurpose::Asset,
            TransferNeedReason::DepthCover
            | TransferNeedReason::SquadPadding
            | TransferNeedReason::CheapReinforcement
            | TransferNeedReason::InjuryCoverLoan
            | TransferNeedReason::LoanToFillSquad => MandatePurpose::Cover,
            TransferNeedReason::ExperiencedHead => MandatePurpose::Rotation,
            TransferNeedReason::FormationGap
            | TransferNeedReason::QualityUpgrade
            | TransferNeedReason::StaffRecommendation
            | TransferNeedReason::OpportunisticLoanUpgrade => match tier {
                BriefTier::A | BriefTier::B => MandatePurpose::Starter,
                BriefTier::C => MandatePurpose::Cover,
            },
        }
    }

    /// Share of a career a man the club has said nothing about needs left
    /// for it to have bought a project rather than a body. The board's
    /// reading of the same question the coach's pathway asks of his own
    /// squad — see [`crate::PathwayStage`].
    const DEVELOP_RUNWAY: f32 = 0.45;

    /// The purpose implied by the role the club promised him, and by what
    /// it can see when it promised nothing.
    ///
    /// Every negotiated signing names a squad status, and that promise IS
    /// the minutes claim — it answers the same question a recruitment
    /// request would have. A move nobody negotiated (a manual transfer, a
    /// loan bought out, a free agent walking in) leaves the club having
    /// said nothing, and then what it has bought is what it can see: a man
    /// with a career ahead of him and room to grow is a project, anybody
    /// else is depth.
    pub fn from_promise(status: &PlayerSquadStatus, runway: f32, below_first_team: bool) -> Self {
        match status {
            PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular => {
                MandatePurpose::Starter
            }
            PlayerSquadStatus::FirstTeamSquadRotation => MandatePurpose::Rotation,
            PlayerSquadStatus::HotProspectForTheFuture | PlayerSquadStatus::DecentYoungster => {
                MandatePurpose::Prospect
            }
            _ if runway >= Self::DEVELOP_RUNWAY && below_first_team => MandatePurpose::Prospect,
            _ => MandatePurpose::Cover,
        }
    }

    /// The purpose implied by where a club already has a player on its own
    /// pathway — the read for a squad that existed before any of this did.
    pub fn from_stage(stage: PathwayStage) -> Self {
        match stage {
            PathwayStage::Core | PathwayStage::Starter | PathwayStage::SellAtPeak => {
                MandatePurpose::Starter
            }
            PathwayStage::Rotation => MandatePurpose::Rotation,
            PathwayStage::Academy | PathwayStage::Prospect | PathwayStage::LoanOut => {
                MandatePurpose::Prospect
            }
            PathwayStage::Reassess | PathwayStage::MoveOn => MandatePurpose::Cover,
        }
    }

    /// The i18n key the transfers page names the purpose with.
    pub fn as_i18n_key(self) -> &'static str {
        match self {
            MandatePurpose::Starter => "mandate_purpose_starter",
            MandatePurpose::Heir { .. } => "mandate_purpose_heir",
            MandatePurpose::Rotation => "mandate_purpose_rotation",
            MandatePurpose::Cover => "mandate_purpose_cover",
            MandatePurpose::Asset => "mandate_purpose_asset",
            MandatePurpose::Prospect => "mandate_purpose_prospect",
        }
    }

    /// When the incumbent stops being first choice — the last season he
    /// holds the shirt, so the heir's curve is centred on the one after.
    fn handover_for(
        group: PlayerFieldPositionGroup,
        incumbent_age: u8,
        date: NaiveDate,
    ) -> NaiveDate {
        // The same horizon the succession audit plans against, one season
        // short of the end: a club hands the shirt over before the man in
        // it stops, not the summer after.
        let end_age: u8 = match group {
            PlayerFieldPositionGroup::Goalkeeper => 38,
            PlayerFieldPositionGroup::Defender => 35,
            PlayerFieldPositionGroup::Midfielder => 34,
            PlayerFieldPositionGroup::Forward => 33,
        } - 1;
        let years = end_age.saturating_sub(incumbent_age) as i64;
        date + Duration::days(years * 365)
    }
}

/// The share of the club's matches a signing is expected to play, season
/// by season.
///
/// One logistic ramp for every purpose: a floor it starts at, a ceiling it
/// reaches, the season it is half-way there and how sharply it turns. A
/// starter is at his ceiling on day one; an heir climbs to it the season
/// the man ahead of him is finished; a prospect climbs slowly and never
/// all the way. Nothing downstream asks WHICH purpose it is looking at —
/// it asks the curve what was promised for the season in front of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinutesCurve {
    floor: f32,
    ceil: f32,
    s_half: f32,
    width: f32,
}

impl MinutesCurve {
    /// Share of the promise a club has to deliver before it may say it has
    /// given him his chance. One tolerance, every purpose.
    pub const TOLERANCE: f32 = 0.6;
    /// Seasons a mandate may run before the board looks at it whatever the
    /// curve says.
    pub const MAX_HORIZON: u8 = 5;
    /// Share at which a man is simply in the side — the point the
    /// replacement he displaces stops being a spare body and becomes the
    /// incumbent.
    pub const STARTER_SHARE: f32 = 0.8;

    /// A curve that is at its ceiling from the first season. `s_half` well
    /// below zero, so the ramp is behind the signing rather than ahead of
    /// it.
    const IMMEDIATE: f32 = -10.0;
    const DEFAULT_WIDTH: f32 = 0.6;

    /// The minutes a purpose promises.
    ///
    /// `issued` and `player_age` place the curve in time: an heir's ramp is
    /// centred on the seasons between the signing and the handover, and a
    /// prospect signed at twenty-three has less of a climb ahead of him
    /// than one signed at seventeen.
    pub fn for_purpose(
        purpose: MandatePurpose,
        group: PlayerFieldPositionGroup,
        issued: NaiveDate,
        player_age: u8,
    ) -> Self {
        match purpose {
            MandatePurpose::Starter => MinutesCurve {
                floor: 0.80,
                ceil: 0.90,
                s_half: Self::IMMEDIATE,
                width: Self::DEFAULT_WIDTH,
            },
            MandatePurpose::Heir { handover, .. } => MinutesCurve {
                floor: 0.15,
                ceil: 0.85,
                s_half: Self::seasons_between(issued, handover),
                width: Self::DEFAULT_WIDTH,
            },
            MandatePurpose::Rotation => MinutesCurve {
                floor: 0.45,
                ceil: 0.55,
                s_half: Self::IMMEDIATE,
                width: Self::DEFAULT_WIDTH,
            },
            MandatePurpose::Cover => MinutesCurve {
                floor: 0.20,
                ceil: 0.25,
                s_half: Self::IMMEDIATE,
                width: Self::DEFAULT_WIDTH,
            },
            // An asset has to beat the k-th man who already plays, so the
            // shirt he takes is the depth chart's own: where one man starts
            // he is the starter, where six do he is one of six.
            MandatePurpose::Asset => {
                let ceil = 0.55 + 0.30 / group.typical_starters() as f32;
                MinutesCurve {
                    floor: ceil - 0.15,
                    ceil,
                    s_half: Self::IMMEDIATE,
                    width: Self::DEFAULT_WIDTH,
                }
            }
            // The older he is when he arrives, the less of a climb the club
            // is paying for — a twenty-two-year-old project is one season
            // from the reckoning, a sixteen-year-old is four.
            MandatePurpose::Prospect => MinutesCurve {
                floor: 0.05,
                ceil: 0.60,
                s_half: ((22.0 - player_age as f32) * 0.5).clamp(0.5, 3.5),
                width: 1.0,
            },
        }
    }

    /// The share promised for the `season`-th season of the mandate,
    /// counting the season it was signed in as zero.
    pub fn share(&self, season: u8) -> f32 {
        let t = (season as f32 - self.s_half) / self.width.max(0.05);
        let sigma = 1.0 / (1.0 + (-t).exp());
        self.floor + (self.ceil - self.floor) * sigma
    }

    /// The season the club looks at the mandate whatever has happened —
    /// one past the season the curve was meant to have turned.
    pub fn review_horizon(&self) -> u8 {
        (self
            .s_half
            .ceil()
            .clamp(0.0, (Self::MAX_HORIZON - 1) as f32) as u8)
            + 1
    }

    /// How much of the shirt he is promised the moment he walks in. The
    /// one reading everything else needs about WHOSE minutes he takes.
    pub fn opening_share(&self) -> f32 {
        self.share(0)
    }

    /// The role a curve like this IS, said in the vocabulary the personal
    /// terms negotiate in. A promise about minutes and a promise about
    /// standing are the same promise, and a request that names one should
    /// not be able to contradict the other.
    pub fn promised_status(&self) -> PlayerSquadStatus {
        if self.ceil >= 0.75 {
            PlayerSquadStatus::FirstTeamRegular
        } else if self.ceil >= 0.40 {
            PlayerSquadStatus::FirstTeamSquadRotation
        } else {
            PlayerSquadStatus::MainBackupPlayer
        }
    }

    fn seasons_between(from: NaiveDate, to: NaiveDate) -> f32 {
        ((to - from).num_days() as f32 / 365.0).clamp(0.0, (Self::MAX_HORIZON - 1) as f32)
    }
}

/// What the board signed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SigningMandate {
    pub purpose: MandatePurpose,
    /// Expected share of the club's matches per season over the deal.
    pub minutes: MinutesCurve,
    pub approved_fee: f64,
    pub approved_wage: f64,
    pub amortisation_years: u8,
    pub issued: NaiveDate,
    pub author: MandateAuthor,
}

impl SigningMandate {
    /// Years a fee is written off over. The IFRS football-finance norm,
    /// and the number the club's own books amortise against.
    pub const DEFAULT_AMORTISATION_YEARS: u8 = 4;

    pub fn new(
        purpose: MandatePurpose,
        group: PlayerFieldPositionGroup,
        player_age: u8,
        issued: NaiveDate,
        author: MandateAuthor,
    ) -> Self {
        SigningMandate {
            purpose,
            minutes: MinutesCurve::for_purpose(purpose, group, issued, player_age),
            approved_fee: 0.0,
            approved_wage: 0.0,
            amortisation_years: Self::DEFAULT_AMORTISATION_YEARS,
            issued,
            author,
        }
    }

    pub fn with_money(mut self, fee: f64, wage: f64) -> Self {
        self.approved_fee = fee.max(0.0);
        self.approved_wage = wage.max(0.0);
        self
    }

    /// A mandate the club actually paid for. The squad a world starts with
    /// carries mandates too, so that the passes below have one thing to
    /// read — but nothing was approved for them, and the older sweeps keep
    /// their own arithmetic for those.
    #[inline]
    pub fn is_purchase(&self) -> bool {
        self.approved_fee > 0.0
    }

    /// Whole seasons since the mandate was issued.
    pub fn season_index(&self, date: NaiveDate) -> u8 {
        let years = (date - self.issued).num_days().max(0) / 365;
        years.min(u8::MAX as i64) as u8
    }

    /// What was promised for the season the club is in.
    pub fn promised_share(&self, date: NaiveDate) -> f32 {
        self.minutes.share(self.season_index(date))
    }

    /// He is getting what the club said he would.
    pub fn on_schedule(&self, date: NaiveDate, delivered_share: f32) -> bool {
        delivered_share >= self.promised_share(date) * MinutesCurve::TOLERANCE
    }

    /// The shirt this mandate is about, when it named one.
    pub fn incumbent_id(&self) -> Option<u32> {
        match self.purpose {
            MandatePurpose::Heir { incumbent_id, .. } => Some(incumbent_id),
            _ => None,
        }
    }

    /// When the club means the shirt to change hands.
    pub fn handover(&self) -> Option<NaiveDate> {
        match self.purpose {
            MandatePurpose::Heir { handover, .. } => Some(handover),
            _ => None,
        }
    }

    /// Unamortised balance of the fee — what the club still has on its
    /// books for him. Straight-line over the same years its finance
    /// department charges the P&L against.
    pub fn book_value(&self, date: NaiveDate) -> f64 {
        if self.approved_fee <= 0.0 {
            return 0.0;
        }
        let years = (date - self.issued).num_days().max(0) as f64 / 365.0;
        let life = self.amortisation_years.max(1) as f64;
        self.approved_fee * (1.0 - years / life).max(0.0)
    }
}

#[cfg(test)]
mod minutes_curve_tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn curve(purpose: MandatePurpose, age: u8) -> MinutesCurve {
        MinutesCurve::for_purpose(
            purpose,
            PlayerFieldPositionGroup::Goalkeeper,
            date(2028, 7, 1),
            age,
        )
    }

    #[test]
    fn a_starter_is_promised_the_shirt_from_the_first_season() {
        let c = curve(MandatePurpose::Starter, 27);
        assert!(c.share(0) > 0.85, "{}", c.share(0));
        assert_eq!(c.review_horizon(), 1);
    }

    #[test]
    fn an_heir_is_promised_almost_nothing_until_the_handover() {
        // The shape the whole doctrine turns on: a 24-year-old bought
        // behind a 33-year-old keeper is a bench purpose for four seasons
        // and a starter after.
        let c = curve(
            MandatePurpose::Heir {
                incumbent_id: 7,
                handover: date(2032, 7, 1),
            },
            24,
        );
        assert!(c.share(0) < 0.25, "{}", c.share(0));
        assert!(c.share(4) > 0.45, "{}", c.share(4));
        assert!(c.share(4) > c.share(0) * 2.0);
        assert!(c.review_horizon() >= 4, "{}", c.review_horizon());
    }

    #[test]
    fn cover_is_promised_a_fifth_of_the_season_and_judged_at_once() {
        let c = curve(MandatePurpose::Cover, 30);
        assert!(c.share(0) < 0.30);
        assert_eq!(c.review_horizon(), 1);
    }

    #[test]
    fn an_asset_takes_the_shirt_where_only_one_man_wears_it() {
        let keeper = MinutesCurve::for_purpose(
            MandatePurpose::Asset,
            PlayerFieldPositionGroup::Goalkeeper,
            date(2028, 7, 1),
            24,
        );
        let defender = MinutesCurve::for_purpose(
            MandatePurpose::Asset,
            PlayerFieldPositionGroup::Defender,
            date(2028, 7, 1),
            24,
        );
        assert!(keeper.share(0) > defender.share(0));
    }

    #[test]
    fn the_curve_never_leaves_its_own_band() {
        let c = curve(MandatePurpose::Prospect, 18);
        for season in 0..=10u8 {
            let share = c.share(season);
            assert!((0.0..=1.0).contains(&share), "{season}: {share}");
            assert!((0.04..=0.61).contains(&share), "{season}: {share}");
        }
    }

    /// The club's books and the number under its own asking price have to
    /// be the same number read from two places. They are computed
    /// independently — one as a monthly charge on the P&L, the other as a
    /// straight line off the transfer date — so nothing but a test keeps
    /// them together.
    #[test]
    fn the_book_and_the_clubs_own_amortisation_agree() {
        use crate::club::finance::ClubFinances;

        const FEE: f64 = 40_000_000.0;
        let issued = date(2028, 7, 8);
        let mandate = SigningMandate::new(
            MandatePurpose::Starter,
            PlayerFieldPositionGroup::Midfielder,
            25,
            issued,
            MandateAuthor::Manager,
        )
        .with_money(FEE, 4_000_000.0);

        let mut finance = ClubFinances::new(0, Vec::new());
        finance.register_transfer_purchase(FEE, mandate.amortisation_years);
        let charged: i64 = (0..12).map(|_| finance.tick_amortization()).sum();

        let unamortised = FEE - charged as f64;
        let book = mandate.book_value(issued + Duration::days(365));
        assert!(
            (unamortised - book).abs() < FEE * 0.01,
            "a year in, the P&L says {unamortised} is left and the book says {book}"
        );
    }

    #[test]
    fn the_book_runs_out_exactly_when_the_amortisation_does() {
        let mandate = SigningMandate::new(
            MandatePurpose::Starter,
            PlayerFieldPositionGroup::Goalkeeper,
            24,
            date(2028, 7, 8),
            MandateAuthor::Board,
        )
        .with_money(40_000_000.0, 4_000_000.0);
        assert!((mandate.book_value(date(2028, 7, 8)) - 40_000_000.0).abs() < 1.0);
        assert!(mandate.book_value(date(2030, 7, 8)) > 19_000_000.0);
        assert!(mandate.book_value(date(2030, 7, 8)) < 21_000_000.0);
        assert_eq!(mandate.book_value(date(2033, 7, 8)), 0.0);
    }
}
