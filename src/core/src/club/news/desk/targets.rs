use super::facts::{ClubTargetsWeek, PursuitStage};
use super::market::MarketDesk;
use crate::club::news::types::{NewsStory, NewsStoryKind};
use chrono::NaiveDate;

/// The club's own business, before it is done.
///
/// The market desk reports a signing on the day the paperwork clears,
/// and the rumour mill reports who wants OUR players. Between them they
/// had nothing to say about the thing a transfer window is actually
/// made of from the club's side of the table: the bid that went in on
/// Tuesday, the fee agreed on Thursday, the medical booked for Monday,
/// the price the board would not pay and the player who said no. The
/// simulation runs every one of those as a phase of a negotiation; this
/// desk reads them off the club's live negotiations and prints the
/// stage each pursuit has reached.
pub struct TargetsDesk;

impl TargetsDesk {
    pub fn file(out: &mut Vec<NewsStory>, week: &ClubTargetsWeek, date: NaiveDate) {
        for pursuit in &week.pursuits {
            let kind = match (pursuit.stage, pursuit.is_loan) {
                (PursuitStage::BidLodged, true) => NewsStoryKind::LoanApproach,
                (PursuitStage::BidLodged, false) => NewsStoryKind::BidLodged,
                // A loan has no fee to agree, so its terms-agreed stage
                // is told as the medical it leads to — the copy for the
                // fee piece is built around the number.
                (PursuitStage::FeeAgreed, true) => NewsStoryKind::MedicalBooked,
                (PursuitStage::FeeAgreed, false) => NewsStoryKind::FeeAgreed,
                (PursuitStage::MedicalBooked, _) => NewsStoryKind::MedicalBooked,
                // …and a loan the other club would not entertain is
                // "not for sale" rather than "too dear": there was no
                // price to baulk at.
                (PursuitStage::PricedOut, true) => NewsStoryKind::NotForSale,
                (PursuitStage::PricedOut, false) => NewsStoryKind::PricedOut,
                (PursuitStage::NotForSale, _) => NewsStoryKind::NotForSale,
                (PursuitStage::PlayerSaidNo, _) => NewsStoryKind::TargetSaysNo,
                (PursuitStage::MedicalFailed, _) => NewsStoryKind::MedicalFailed,
                (PursuitStage::WindowShut, _) => NewsStoryKind::DealDiesAtDeadline,
            };

            out.push(
                NewsStory::new(kind, date)
                    .about(pursuit.player_id)
                    .against(pursuit.selling_club_id)
                    .with_money(pursuit.fee)
                    .weighted(MarketDesk::fee_weight(pursuit.fee)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TargetsDesk;
    use crate::club::news::desk::facts::{ClubTargetsWeek, PursuitStage, TargetPursuit};
    use crate::club::news::editor::NewsEditor;
    use crate::club::news::types::{NewsStory, NewsStoryKind};
    use chrono::NaiveDate;
    use std::collections::VecDeque;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 6).unwrap()
    }

    fn pursuit(stage: PursuitStage, is_loan: bool, fee: i64) -> TargetPursuit {
        TargetPursuit {
            player_id: 7,
            selling_club_id: 44,
            fee,
            is_loan,
            stage,
        }
    }

    fn file(pursuits: Vec<TargetPursuit>) -> Vec<NewsStory> {
        let mut out = Vec::new();
        TargetsDesk::file(&mut out, &ClubTargetsWeek { pursuits }, day());
        out
    }

    #[test]
    fn a_bid_is_reported_with_its_number_and_the_club_it_went_to() {
        let out = file(vec![pursuit(PursuitStage::BidLodged, false, 4_000_000)]);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, NewsStoryKind::BidLodged);
        assert_eq!(out[0].other_id, 44);
        assert_eq!(out[0].money, 4_000_000);
    }

    /// A loan has no fee, so the pieces written around one are never
    /// filed for it — and the editor would refuse them anyway.
    #[test]
    fn a_loan_pursuit_never_reaches_the_copy_that_quotes_a_fee() {
        let out = file(vec![
            pursuit(PursuitStage::BidLodged, true, 0),
            pursuit(PursuitStage::FeeAgreed, true, 0),
            pursuit(PursuitStage::PricedOut, true, 0),
        ]);

        let kinds: Vec<NewsStoryKind> = out.iter().map(|story| story.kind).collect();
        assert_eq!(
            kinds,
            vec![
                NewsStoryKind::LoanApproach,
                NewsStoryKind::MedicalBooked,
                NewsStoryKind::NotForSale
            ]
        );
        assert!(
            kinds.iter().all(|kind| !kind.quotes_a_fee()),
            "a loan pursuit reached a fee story: {:?}",
            kinds
        );
        assert_eq!(
            NewsEditor::compile(out, &VecDeque::new()).len(),
            3,
            "every loan line must survive the editor's figure gate"
        );
    }

    #[test]
    fn every_ending_has_its_own_line() {
        let out = file(vec![
            pursuit(PursuitStage::PlayerSaidNo, false, 3_000_000),
            pursuit(PursuitStage::MedicalFailed, false, 3_000_000),
            pursuit(PursuitStage::NotForSale, false, 3_000_000),
            pursuit(PursuitStage::WindowShut, false, 3_000_000),
        ]);
        let kinds: Vec<NewsStoryKind> = out.iter().map(|story| story.kind).collect();

        assert_eq!(
            kinds,
            vec![
                NewsStoryKind::TargetSaysNo,
                NewsStoryKind::MedicalFailed,
                NewsStoryKind::NotForSale,
                NewsStoryKind::DealDiesAtDeadline
            ]
        );
    }
}
