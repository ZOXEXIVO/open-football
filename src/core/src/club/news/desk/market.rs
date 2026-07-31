use super::facts::{
    ClubTransferWeek, PlayerStanding, RecentEvents, TransferMotive, TransferMove, TransferMoveKind,
};
use crate::club::news::types::{NewsStory, NewsStoryKind};
use crate::{HappinessEventType, Player};
use chrono::NaiveDate;

/// Arrivals and departures — completed business only. Everything that
/// has not been signed yet belongs to the rumour desk.
pub struct MarketDesk;

impl MarketDesk {
    /// A fee at or above this multiple of the club's most valuable asset
    /// is treated as club-record business — the framing a local paper
    /// reaches for when a deal is out of scale with everything the club
    /// has done before.
    const RECORD_MULTIPLE: i64 = 2;

    /// An arrival at or under this age is signed for the years ahead
    /// rather than for Saturday, and the paper writes him that way.
    const PROSPECT_SIGNING_AGE: u8 = 20;
    /// …and at or over this one, the story is the experience walking in.
    const VETERAN_SIGNING_AGE: u8 = 33;

    pub fn file(
        out: &mut Vec<NewsStory>,
        week: &ClubTransferWeek,
        squad_peak_value: i64,
        date: NaiveDate,
    ) {
        let record_bar = squad_peak_value.saturating_mul(Self::RECORD_MULTIPLE);

        for arrival in &week.arrivals {
            let kind = Self::arrival_kind(arrival, record_bar);
            out.push(
                NewsStory::new(kind, date)
                    .about(arrival.player_id)
                    .against(arrival.other_club_id)
                    // The age rides in `a` so the prospect and veteran
                    // framings can print it; both are gated on a real
                    // age, so the copy never quotes a zero. The scout's
                    // piece is the one exception — the figure that story
                    // is about is how sure he was.
                    .with_numbers(
                        if kind == NewsStoryKind::ScoutingCoup {
                            i32::from(arrival.scout_confidence_pct)
                        } else {
                            i32::from(arrival.age)
                        },
                        0,
                    )
                    .with_money(arrival.fee)
                    .weighted(Self::fee_weight(arrival.fee)),
            );
        }

        for departure in &week.departures {
            out.push(
                NewsStory::new(Self::departure_kind(departure, record_bar), date)
                    .about(departure.player_id)
                    .against(departure.other_club_id)
                    .with_money(departure.fee)
                    .weighted(Self::fee_weight(departure.fee)),
            );
        }
    }

    /// A scout who was this sure is a scout worth quoting. Below it the
    /// report is a recommendation rather than a verdict, and the piece
    /// would be about somebody's hunch.
    const SCOUT_CONVICTION_PCT: u8 = 55;

    /// What kind of arrival this is. Precedence runs from the framings
    /// that own the whole story down to the plain fee/no-fee split:
    /// record business dwarfs everything, then a raid on a rival and
    /// one of the club's own coming through, then the man who was
    /// already here on loan, then the man who has worn the shirt
    /// before, then age — and only then the club's own reason for
    /// doing it, which is still better than the ordinary signing
    /// report that catches everything else.
    fn arrival_kind(arrival: &TransferMove, record_bar: i64) -> NewsStoryKind {
        if arrival.kind == TransferMoveKind::Loan {
            return NewsStoryKind::LoanArrival;
        }
        // The fee is re-checked rather than trusted: a `Paid` record
        // carrying nothing is a contradiction, and the whole point of
        // this desk is that no headline ever quotes a fee of zero,
        // whichever way the record was built.
        if arrival.kind == TransferMoveKind::Paid
            && arrival.fee > 0
            && record_bar > 0
            && arrival.fee >= record_bar
        {
            return NewsStoryKind::RecordSigning;
        }
        // Where he came from outranks why he was bought: a town enjoys
        // a raid on the neighbours whatever the club's own reasoning.
        if arrival.rival {
            return NewsStoryKind::RivalRaid;
        }
        // Not a signing at all — the academy handing somebody up.
        if arrival.motive == TransferMotive::AcademyPromotion {
            return NewsStoryKind::AcademyGraduate;
        }
        if arrival.was_loan_here {
            return NewsStoryKind::LoanMadePermanent;
        }
        if arrival.returning {
            return NewsStoryKind::HomecomingSigning;
        }
        // Age 0 means "unresolved", never "newborn" — both age framings
        // need a real age behind them.
        if (1..=Self::PROSPECT_SIGNING_AGE).contains(&arrival.age) {
            return NewsStoryKind::ProspectSigned;
        }
        if arrival.age >= Self::VETERAN_SIGNING_AGE {
            return NewsStoryKind::VeteranArrives;
        }
        if let Some(kind) = Self::motive_kind(arrival) {
            return kind;
        }
        if arrival.fee <= 0 {
            return NewsStoryKind::FreeSigning;
        }
        NewsStoryKind::NewSigning
    }

    /// The club's own reason for a signing, where it is worth a
    /// headline of its own.
    ///
    /// The two money framings are gated on a real fee: a free transfer
    /// whose motive happened to be "quality upgrade" must fall through
    /// to the free-signing report rather than reach the editor as a
    /// piece about a fee of nothing, which the printability gate would
    /// then drop — losing the arrival from the paper altogether.
    fn motive_kind(arrival: &TransferMove) -> Option<NewsStoryKind> {
        // The scout's piece quotes how sure he was, so it is only
        // available when a report actually recorded a figure — a
        // signing the department merely recommended, with no report
        // behind it, would otherwise print "the scouts were 0% sure".
        let has_verdict = arrival.scout_confidence_pct > 0;

        // A scout who pushed hard and was listened to is a better story
        // than the department's standing brief, so it is asked first.
        if has_verdict
            && arrival.scout_urged_it
            && arrival.scout_confidence_pct >= Self::SCOUT_CONVICTION_PCT
        {
            return Some(NewsStoryKind::ScoutingCoup);
        }

        match arrival.motive {
            TransferMotive::QualityUpgrade if arrival.fee > 0 => {
                Some(NewsStoryKind::MarqueeUpgrade)
            }
            TransferMotive::Bargain if arrival.fee > 0 => Some(NewsStoryKind::BargainBuy),
            TransferMotive::Succession => Some(NewsStoryKind::SuccessionSigning),
            TransferMotive::FormationGap => Some(NewsStoryKind::GapPlugged),
            TransferMotive::ScoutFind if has_verdict => Some(NewsStoryKind::ScoutingCoup),
            TransferMotive::DepthCover => Some(NewsStoryKind::DepthSigning),
            // Development and experience are already told by the
            // prospect and veteran framings above, which reach the same
            // players with a better sentence. Anything else is the
            // ordinary report.
            _ => None,
        }
    }

    /// A departure with no fee is not a sale. Printing "the club sold
    /// him for $0.00" is the one line that tells a reader the page was
    /// generated rather than written — a released player, an expiring
    /// contract and a Bosman all leave for nothing, and each of them is
    /// its own kind of story.
    fn departure_kind(departure: &TransferMove, record_bar: i64) -> NewsStoryKind {
        match departure.kind {
            // A loan out is normally furniture. It stops being
            // furniture when the borrowing club's own stated reason for
            // taking him was to develop him: read from this side of the
            // deal, that is a lad going somewhere he will actually
            // play, which is the thing his own supporters want to know.
            TransferMoveKind::Loan if departure.motive.is_development() => {
                NewsStoryKind::LoanedOutToGrow
            }
            TransferMoveKind::Loan => NewsStoryKind::LoanExit,
            _ if departure.fee <= 0 => NewsStoryKind::FreeExit,
            TransferMoveKind::Free => NewsStoryKind::FreeExit,
            TransferMoveKind::Paid if record_bar > 0 && departure.fee >= record_bar => {
                NewsStoryKind::StarSold
            }
            TransferMoveKind::Paid => NewsStoryKind::PlayerSold,
        }
    }

    /// Money talks, but with diminishing returns: the jump from one to
    /// ten million is far bigger news than ninety to a hundred.
    fn fee_weight(fee: i64) -> i32 {
        if fee <= 0 {
            return 0;
        }
        let millions = fee as f64 / 1_000_000.0;
        ((millions + 1.0).ln() * 45.0) as i32
    }

    /// Long enough after a move for a verdict to be fair, and short
    /// enough that the move is still what people are judging him on.
    const VERDICT_WINDOW_DAYS: i64 = 550;
    /// A verdict needs a sample. Below this he has not been given a
    /// chance, and saying he has failed would be the paper's error
    /// rather than the player's.
    const VERDICT_MIN_APPS: i32 = 8;

    /// The verdict on business already done, filed from the squad walk.
    ///
    /// Signing a player is a story on the day; whether he was worth it
    /// is the story for the year that follows, and it is the one
    /// supporters actually argue about. The dressing room already keeps
    /// this score — the senior pros lose patience with a flop, and the
    /// room re-rates a man who proves he belongs — so the desk reads
    /// their verdict rather than inventing one from the fee.
    pub fn file_verdict(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate) {
        let Some(joined) = player.last_transfer_date() else {
            return;
        };
        if (date - joined).num_days() > Self::VERDICT_WINDOW_DAYS || player.is_on_loan() {
            return;
        }

        let stats = &player.statistics;
        let apps = stats.played as i32 + stats.played_subs as i32;
        let rating = stats.average_rating_realistic(player.position().position_group());
        if apps < Self::VERDICT_MIN_APPS || rating <= 0.0 {
            return;
        }

        let feed = RecentEvents::fortnight(player);
        let importance = PlayerStanding::importance(player);

        // The room has stopped covering for him.
        if feed
            .any_of(&[
                HappinessEventType::LosingPatienceWithSigning,
                HappinessEventType::FeelsDressingRoomPressure,
                HappinessEventType::BigClubAuraFaded,
                HappinessEventType::OverawedByEliteClub,
                HappinessEventType::StepDownEmbarrassment,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::SigningNotWorking, date)
                    .about(player.id)
                    .with_numbers(apps, (rating * 100.0) as i32)
                    .weighted(importance),
            );
            return;
        }

        // …or it has decided he belongs after all.
        if feed
            .any_of(&[
                HappinessEventType::ProvedLevelAfterMove,
                HappinessEventType::TrustedAfterStepUp,
                HappinessEventType::AdaptationBreakthrough,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::SigningComesGood, date)
                    .about(player.id)
                    .with_numbers(apps, (rating * 100.0) as i32)
                    .weighted(importance / 2),
            );
        }
    }
}
