//! What the board remembers about the cheques it signed.
//!
//! A mandate closes when the player it was written for leaves, retires or
//! runs out of contract. What it closed AS — the minutes he was promised
//! against the minutes he had, and what came back of the fee — is the only
//! record the club keeps of its own recruitment, and the three things that
//! read it are the three places that should feel a wasted fee: the next
//! hearing's rope, the manager's standing with the board, and the financial
//! gauge.

use chrono::NaiveDate;

use crate::club::board::mandate::{MandateAuthor, MandatePurpose, SigningMandate};

/// How a mandate ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MandateExit {
    Sold(f64),
    Loaned,
    Released,
    Retired,
    /// The deal ran its course and he is still here.
    Kept,
}

impl MandateExit {
    /// What came back of the fee.
    pub fn recovered(self) -> f64 {
        match self {
            MandateExit::Sold(fee) => fee.max(0.0),
            _ => 0.0,
        }
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            MandateExit::Sold(_) => "mandate_exit_sold",
            MandateExit::Loaned => "mandate_exit_loaned",
            MandateExit::Released => "mandate_exit_released",
            MandateExit::Retired => "mandate_exit_retired",
            MandateExit::Kept => "mandate_exit_kept",
        }
    }
}

/// One closed mandate.
#[derive(Debug, Clone, Copy)]
pub struct MandateOutcome {
    pub player_id: u32,
    pub purpose: MandatePurpose,
    pub author: MandateAuthor,
    pub fee: f64,
    /// Share of the club's matches the mandate promised for the season it
    /// closed in, against the share he actually had.
    pub minutes_promised: f32,
    pub minutes_delivered: f32,
    pub exit: MandateExit,
    /// What the club never got back: the unamortised book at the exit,
    /// less whatever the exit recovered.
    pub loss: f64,
    pub closed: NaiveDate,
}

impl MandateOutcome {
    /// The mandate delivered so little of what it promised that the club
    /// has to answer for it — the bar the manager's record is judged on.
    pub const DELIVERED_BAR: f32 = 0.40;

    pub fn close(
        player_id: u32,
        mandate: &SigningMandate,
        delivered_share: f32,
        exit: MandateExit,
        date: NaiveDate,
    ) -> Self {
        let book = mandate.book_value(date);
        MandateOutcome {
            player_id,
            purpose: mandate.purpose,
            author: mandate.author,
            fee: mandate.approved_fee,
            minutes_promised: mandate.promised_share(date),
            minutes_delivered: delivered_share,
            exit,
            loss: (book - exit.recovered()).max(0.0),
            closed: date,
        }
    }

    /// He never got near what the club said he would.
    pub fn is_unrealised(&self) -> bool {
        self.minutes_promised > 0.0
            && self.minutes_delivered < self.minutes_promised * Self::DELIVERED_BAR
    }
}

/// The board's own record of what its money bought.
#[derive(Debug, Clone, Default)]
pub struct MandateLedger {
    rows: Vec<MandateOutcome>,
}

impl MandateLedger {
    /// Days a closed mandate stays on the board's conscience — two
    /// seasons, the span a chairman argues about.
    const MEMORY_DAYS: i64 = 730;
    /// Closed mandates the ledger carries. Past this the oldest go; a
    /// board argues about its recent record, not its whole history.
    const MAX_ROWS: usize = 24;

    pub fn rows(&self) -> &[MandateOutcome] {
        &self.rows
    }

    pub fn push(&mut self, outcome: MandateOutcome) {
        self.rows.push(outcome);
        self.prune(outcome.closed);
    }

    /// Drop everything the board has stopped arguing about.
    pub fn prune(&mut self, date: NaiveDate) {
        self.rows
            .retain(|r| (date - r.closed).num_days() <= Self::MEMORY_DAYS);
        if self.rows.len() > Self::MAX_ROWS {
            let excess = self.rows.len() - Self::MAX_ROWS;
            self.rows.drain(0..excess);
        }
    }

    /// Share of the money it has spent that the club has already written
    /// off. A board that has booked losses on a third of its recent
    /// spending approves the next fee with less rope.
    pub fn burn(&self) -> f64 {
        let spend: f64 = self.rows.iter().map(|r| r.fee).sum();
        if spend <= 0.0 {
            return 0.0;
        }
        let loss: f64 = self.rows.iter().map(|r| r.loss).sum();
        (loss / spend).clamp(0.0, 1.0)
    }

    /// What that record takes off the next hearing's stretch.
    pub fn discipline(&self) -> f64 {
        0.5 * self.burn()
    }

    /// Mandates that closed with the minutes never delivered — what the
    /// recruitment desk got wrong rather than what it overpaid for.
    pub fn unrealised(&self) -> impl Iterator<Item = &MandateOutcome> {
        self.rows.iter().filter(|r| r.is_unrealised())
    }
}

#[cfg(test)]
mod mandate_ledger_tests {
    use super::*;
    use crate::PlayerFieldPositionGroup;
    use crate::club::board::mandate::MandatePurpose;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn mandate(fee: f64, issued: NaiveDate) -> SigningMandate {
        SigningMandate::new(
            MandatePurpose::Heir {
                incumbent_id: 7,
                handover: date(2032, 7, 1),
            },
            PlayerFieldPositionGroup::Goalkeeper,
            24,
            issued,
            MandateAuthor::Manager,
        )
        .with_money(fee, 4_800_000.0)
    }

    #[test]
    fn a_written_off_fee_narrows_the_next_hearing() {
        let mut ledger = MandateLedger::default();
        assert_eq!(ledger.discipline(), 0.0);
        ledger.push(MandateOutcome::close(
            1,
            &mandate(41_600_000.0, date(2028, 7, 8)),
            0.0,
            MandateExit::Sold(8_100_000.0),
            date(2030, 7, 5),
        ));
        assert!(ledger.burn() > 0.1, "{}", ledger.burn());
        assert!((ledger.discipline() - ledger.burn() * 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_mandate_whose_minutes_never_arrived_is_flagged() {
        let outcome = MandateOutcome::close(
            1,
            &mandate(41_600_000.0, date(2028, 7, 8)),
            0.02,
            MandateExit::Sold(8_100_000.0),
            date(2030, 7, 5),
        );
        assert!(outcome.is_unrealised());
        assert!(outcome.loss > 0.0);
    }

    #[test]
    fn the_board_stops_arguing_about_it_after_two_seasons() {
        let mut ledger = MandateLedger::default();
        ledger.push(MandateOutcome::close(
            1,
            &mandate(40_000_000.0, date(2028, 7, 8)),
            0.0,
            MandateExit::Released,
            date(2030, 7, 5),
        ));
        ledger.prune(date(2033, 7, 5));
        assert!(ledger.rows().is_empty());
        assert_eq!(ledger.discipline(), 0.0);
    }
}
