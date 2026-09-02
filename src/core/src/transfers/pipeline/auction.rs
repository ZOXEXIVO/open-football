//! L5 — price discovery: what happens when two clubs want the same player,
//! and what happens in the last week of a window.
//!
//! The market had neither. Competition was a flat `+5 per rival, max +15` on
//! the seller's engagement roll — a nudge on whether the seller would TALK,
//! with no effect at all on the PRICE — and the deadline was a −0.10 nudge on
//! the seller's reservation. So no two clubs ever fought over a player, and
//! nothing ever closed at a premium because the clock had run out. Between
//! them those are a fifth of a real window's fee moves.
//!
//! An auction here is not a new negotiation type. It is a reading of the
//! negotiations that already exist against one player: the highest live bid
//! becomes the floor the next bid has to clear, each buyer keeps raising only
//! while it is below its own [`super::upgrade_math::UpgradeMath`] ceiling, and
//! the seller's willingness to engage scales with how many people are asking
//! rather than with a fixed bonus.

use crate::transfers::pipeline::planning::BriefTier;

/// What the market looks like around one player right now.
#[derive(Debug, Clone, Copy)]
pub struct AuctionState {
    /// Live negotiations for him, EXCLUDING the one being resolved.
    pub rivals: u32,
    /// Highest live offer from any of those rivals. Zero when there are
    /// none.
    pub leading_bid: f64,
}

impl AuctionState {
    /// Smallest raise, as a share of the leader, that counts as a new bid.
    /// Below it the round has produced no movement and the seller takes the
    /// leader.
    pub const MIN_RAISE: f64 = 0.04;
    /// How much the seller's willingness to engage rises per rival, and
    /// where it saturates. A queue at the door is leverage, but a seller
    /// who would refuse one club at a given price does not accept four
    /// times the same price — it is the FEE that competition moves, and
    /// that happens through [`Self::floor`].
    const LEVERAGE_PER_RIVAL: f32 = 4.0;
    const LEVERAGE_MAX: f32 = 12.0;
    /// Acceptance points the leading bid gains for BEING the leading bid,
    /// doubled on the last day of the window. A seller with three offers in
    /// front of it takes the best one — it is not choosing between that bid
    /// and nothing.
    pub const LEADER_BONUS: f32 = 10.0;
    /// What is left of a trailing bid's acceptance chance. Not zero: the
    /// leader can still fail a medical, run out of budget or be refused
    /// personal terms, and a seller occasionally prefers a slightly lower
    /// bid from a club it would rather deal with.
    pub const TRAILING_SHARE: f32 = 0.45;

    pub fn is_contested(&self) -> bool {
        self.rivals > 0
    }

    /// The seller's engagement lift from being wanted by more than one club.
    pub fn seller_leverage(&self) -> f32 {
        (self.rivals as f32 * Self::LEVERAGE_PER_RIVAL).min(Self::LEVERAGE_MAX)
    }

    /// The number a competing bid has to beat to stay in the auction.
    ///
    /// This is what turns rivalry into money: a buyer whose own valuation
    /// says the player is worth more keeps clearing the floor, and the one
    /// whose ceiling is lower simply stops raising and drops out — which is
    /// exactly how a real bidding war resolves, and why the winner pays
    /// more than he would have unopposed.
    pub fn floor(&self) -> f64 {
        if self.leading_bid <= 0.0 {
            return 0.0;
        }
        self.leading_bid * (1.0 + Self::MIN_RAISE)
    }

    /// True when this buyer's own bid is the one in front. A leader raises
    /// against nobody, so it holds.
    pub fn leads_with(&self, own_offer: f64) -> bool {
        own_offer >= self.leading_bid
    }
}

/// The last days of a window, and what they do to both sides.
#[derive(Debug, Clone, Copy)]
pub struct DeadlineWindow {
    /// Days until the window shuts. `None` outside a window.
    pub days_left: Option<i64>,
    /// Total length of the window in days, so `pressure` is a fraction of
    /// the window rather than of a fixed calendar.
    pub window_days: i64,
}

impl DeadlineWindow {
    /// Days at the end of a window in which behaviour changes on both
    /// sides. One week is the shape of the real thing — roughly a fifth to
    /// a third of a window's fee moves close in it.
    pub const DEADLINE_DAYS: i64 = 7;
    /// Most a buyer will pay over the asking price purely because the clock
    /// has run out and its brief is still unfilled.
    pub const DEADLINE_PREMIUM: f64 = 0.20;

    pub fn closed() -> Self {
        DeadlineWindow {
            days_left: None,
            window_days: 0,
        }
    }

    pub fn of(days_left: i64, window_days: i64) -> Self {
        DeadlineWindow {
            days_left: Some(days_left.max(0)),
            window_days: window_days.max(1),
        }
    }

    /// 0.0 at the start of the window, 1.0 on the last day. Continuous, so
    /// nothing happens overnight.
    pub fn pressure(&self) -> f32 {
        match self.days_left {
            None => 0.0,
            Some(days) => {
                if days >= Self::DEADLINE_DAYS {
                    0.0
                } else {
                    1.0 - (days as f32 / Self::DEADLINE_DAYS as f32)
                }
            }
        }
    }

    /// Share of the window still to run, 1.0 on the first day. Feeds the
    /// opening-offer ratio.
    pub fn days_left_fraction(&self) -> f32 {
        match self.days_left {
            None => 1.0,
            Some(days) => (days as f32 / self.window_days as f32).clamp(0.0, 1.0),
        }
    }

    pub fn is_deadline_week(&self) -> bool {
        matches!(self.days_left, Some(d) if d < Self::DEADLINE_DAYS)
    }

    /// What a buyer will add to the asking price in the last days because
    /// the shirt is still empty.
    ///
    /// Only for the tiers a club cannot walk away from: a transformative or
    /// upgrade slot left unfilled is a season's plan going wrong, while
    /// cover has a loan market to fall back on and therefore pays nothing
    /// extra.
    pub fn premium_for(&self, tier: BriefTier, slot_unfilled: bool) -> f64 {
        if !slot_unfilled {
            return 0.0;
        }
        let scale = match tier {
            BriefTier::A => 1.0,
            BriefTier::B => 0.6,
            BriefTier::C => 0.0,
        };
        Self::DEADLINE_PREMIUM * scale * self.pressure() as f64
    }

    /// True when a cover slot should stop trying to buy and go and borrow
    /// instead — the existing panic-loan path is what answers this.
    pub fn should_convert_to_loan(&self, tier: BriefTier) -> bool {
        matches!(tier, BriefTier::C) && self.is_deadline_week()
    }
}

#[cfg(test)]
mod auction_tests {
    use super::*;

    #[test]
    fn an_uncontested_bid_faces_no_floor() {
        let quiet = AuctionState {
            rivals: 0,
            leading_bid: 0.0,
        };
        assert!(!quiet.is_contested());
        assert_eq!(quiet.floor(), 0.0);
        assert_eq!(quiet.seller_leverage(), 0.0);
    }

    #[test]
    fn a_rival_bid_becomes_the_floor_for_the_next_one() {
        let contested = AuctionState {
            rivals: 1,
            leading_bid: 20_000_000.0,
        };
        assert!(contested.floor() > contested.leading_bid);
        assert!(
            (contested.floor() - 20_800_000.0).abs() < 1.0,
            "{}",
            contested.floor()
        );
    }

    #[test]
    fn seller_leverage_saturates_rather_than_running_away() {
        let two = AuctionState {
            rivals: 2,
            leading_bid: 1.0,
        };
        let many = AuctionState {
            rivals: 9,
            leading_bid: 1.0,
        };
        assert!(many.seller_leverage() > two.seller_leverage());
        assert!(many.seller_leverage() <= AuctionState::LEVERAGE_MAX);
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;

    #[test]
    fn pressure_is_zero_until_the_last_week_and_one_on_the_last_day() {
        assert_eq!(DeadlineWindow::of(30, 60).pressure(), 0.0);
        assert_eq!(DeadlineWindow::of(7, 60).pressure(), 0.0);
        assert!(DeadlineWindow::of(3, 60).pressure() > 0.0);
        assert!((DeadlineWindow::of(0, 60).pressure() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn only_a_slot_the_club_cannot_walk_away_from_pays_a_premium() {
        let last_day = DeadlineWindow::of(0, 60);
        assert!(last_day.premium_for(BriefTier::A, true) > 0.0);
        assert!(
            last_day.premium_for(BriefTier::B, true) < last_day.premium_for(BriefTier::A, true)
        );
        assert_eq!(last_day.premium_for(BriefTier::C, true), 0.0);
        assert_eq!(last_day.premium_for(BriefTier::A, false), 0.0);
    }

    #[test]
    fn the_premium_is_bounded_by_its_own_constant() {
        let last_day = DeadlineWindow::of(0, 60);
        assert!(last_day.premium_for(BriefTier::A, true) <= DeadlineWindow::DEADLINE_PREMIUM);
    }

    #[test]
    fn cover_goes_and_borrows_instead_of_paying_up() {
        assert!(DeadlineWindow::of(2, 60).should_convert_to_loan(BriefTier::C));
        assert!(!DeadlineWindow::of(2, 60).should_convert_to_loan(BriefTier::A));
        assert!(!DeadlineWindow::of(30, 60).should_convert_to_loan(BriefTier::C));
    }

    #[test]
    fn a_shut_window_exerts_no_pressure_at_all() {
        let shut = DeadlineWindow::closed();
        assert_eq!(shut.pressure(), 0.0);
        assert!(!shut.is_deadline_week());
        assert_eq!(shut.days_left_fraction(), 1.0);
    }
}
