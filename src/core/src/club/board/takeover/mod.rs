//! Rare ownership-change events. A club whose owner wants out (high
//! `exit_pressure`) under financial strain — or a rapidly rising club that
//! becomes an attractive asset — can attract a takeover. Rumours simmer
//! for a few months, then either complete (new, usually wealthier owner,
//! fresh strategy) or collapse (instability, short budget freeze).
//!
//! All decisions are driven by explicit inputs plus a caller-supplied
//! `roll` (0-99) so the engine stays deterministic and testable; the board
//! feeds it `IntegerUtils::random(0, 100)` in production.

use super::context::{BoardContext, FfpStatus};
use super::decision::BoardDecision;
use super::ownership::{OwnershipModel, OwnershipType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TakeoverStatus {
    #[default]
    None,
    Rumoured,
    Failed,
    Completed,
}

#[derive(Debug, Clone, Default)]
pub struct TakeoverWatch {
    pub status: TakeoverStatus,
    /// Months the watch has spent in the current status.
    pub months_in_status: u8,
    /// Set for one tick after a failed takeover so the board can apply a
    /// short-term budget freeze.
    pub just_failed: bool,
}

impl TakeoverWatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Months a rumour simmers before it resolves.
    const RESOLVE_AFTER_MONTHS: u8 = 3;

    fn financial_trouble(ctx: &BoardContext) -> bool {
        ctx.balance < 0
            || matches!(ctx.ffp_status, FfpStatus::Breach)
            || ctx.profit_loss_12m < 0
    }

    /// Is the club ripe for a takeover rumour right now?
    pub fn eligible_for_rumour(owner: &OwnershipModel, ctx: &BoardContext) -> bool {
        // A frustrated owner under financial strain wants to sell...
        let owner_wants_out = owner.exit_pressure >= 45 && Self::financial_trouble(ctx);
        // ...or a strongly-rising club becomes a coveted asset.
        let attractive_asset = ctx.reputation_score >= 0.82;
        owner_wants_out || attractive_asset
    }

    /// Monthly tick. Returns a takeover decision when the state changes.
    pub fn tick(
        &mut self,
        owner: &OwnershipModel,
        ctx: &BoardContext,
        roll: u8,
    ) -> Option<BoardDecision> {
        self.just_failed = false;
        match self.status {
            TakeoverStatus::None | TakeoverStatus::Failed => {
                if Self::eligible_for_rumour(owner, ctx) {
                    // ~8% monthly chance when eligible — keeps it rare.
                    let chance = if owner.exit_pressure >= 70 { 12 } else { 8 };
                    if roll < chance {
                        self.status = TakeoverStatus::Rumoured;
                        self.months_in_status = 0;
                        return Some(BoardDecision::StartTakeoverRumour);
                    }
                }
                self.months_in_status = self.months_in_status.saturating_add(1);
                None
            }
            TakeoverStatus::Rumoured => {
                self.months_in_status = self.months_in_status.saturating_add(1);
                if self.months_in_status >= Self::RESOLVE_AFTER_MONTHS {
                    // Completion likelier when the owner is desperate to
                    // leave or the club is a glittering prize.
                    let completion = if owner.exit_pressure >= 70 || ctx.reputation_score >= 0.85 {
                        55
                    } else {
                        40
                    };
                    if roll < completion {
                        self.status = TakeoverStatus::Completed;
                        self.months_in_status = 0;
                        Some(BoardDecision::CompleteTakeover)
                    } else {
                        self.status = TakeoverStatus::Failed;
                        self.months_in_status = 0;
                        self.just_failed = true;
                        None
                    }
                } else {
                    None
                }
            }
            TakeoverStatus::Completed => None,
        }
    }

    pub fn rumour_active(&self) -> bool {
        matches!(self.status, TakeoverStatus::Rumoured)
    }
}

pub struct TakeoverEngine;

impl TakeoverEngine {
    /// The ownership model installed after a successful takeover. New
    /// owners arrive richer and hungrier; `seed` (club id) keeps the
    /// archetype deterministic and varied.
    pub fn post_takeover_owner(seed: u32) -> OwnershipModel {
        let ownership_type = match seed % 3 {
            0 => OwnershipType::StateBacked,
            1 => OwnershipType::PrivateEquity,
            _ => OwnershipType::Consortium,
        };
        let wealth = match ownership_type {
            OwnershipType::StateBacked => 95,
            OwnershipType::PrivateEquity => 80,
            _ => 70,
        };
        OwnershipModel {
            ownership_type,
            wealth,
            interference: 70,
            risk_tolerance: 85,
            exit_pressure: 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distressed_ctx() -> BoardContext {
        let mut c = BoardContext::new();
        c.balance = -80_000_000;
        c.profit_loss_12m = -20_000_000;
        c.ffp_status = FfpStatus::Breach;
        c.reputation_score = 0.5;
        c
    }

    #[test]
    fn frustrated_owner_under_strain_is_eligible() {
        let owner = OwnershipModel {
            exit_pressure: 60,
            ..Default::default()
        };
        assert!(TakeoverWatch::eligible_for_rumour(&owner, &distressed_ctx()));
    }

    #[test]
    fn stable_healthy_club_not_eligible() {
        let owner = OwnershipModel {
            exit_pressure: 10,
            ..Default::default()
        };
        let mut ctx = BoardContext::new();
        ctx.balance = 50_000_000;
        ctx.reputation_score = 0.5;
        assert!(!TakeoverWatch::eligible_for_rumour(&owner, &ctx));
    }

    #[test]
    fn low_roll_starts_rumour_then_completes() {
        let owner = OwnershipModel {
            exit_pressure: 75,
            ..Default::default()
        };
        let ctx = distressed_ctx();
        let mut watch = TakeoverWatch::new();
        // First eligible tick with a low roll opens the rumour.
        let d = watch.tick(&owner, &ctx, 0);
        assert_eq!(d, Some(BoardDecision::StartTakeoverRumour));
        assert!(watch.rumour_active());
        // Simmer until resolution, then a low roll completes it.
        watch.tick(&owner, &ctx, 99); // month 1, no resolve
        let d = watch.tick(&owner, &ctx, 0); // month 2 -> resolves at >=3? need one more
        // RESOLVE_AFTER_MONTHS = 3: ticks increment months_in_status to 3.
        let d = d.or_else(|| watch.tick(&owner, &ctx, 0));
        assert_eq!(d, Some(BoardDecision::CompleteTakeover));
        assert!(matches!(watch.status, TakeoverStatus::Completed));
    }

    #[test]
    fn high_roll_fails_takeover() {
        let owner = OwnershipModel {
            exit_pressure: 75,
            ..Default::default()
        };
        let ctx = distressed_ctx();
        let mut watch = TakeoverWatch::new();
        watch.tick(&owner, &ctx, 0); // rumour
        watch.tick(&owner, &ctx, 99);
        watch.tick(&owner, &ctx, 99);
        watch.tick(&owner, &ctx, 99);
        assert!(matches!(watch.status, TakeoverStatus::Failed));
    }

    #[test]
    fn post_takeover_owner_is_wealthier() {
        let owner = TakeoverEngine::post_takeover_owner(0);
        assert!(owner.wealth >= 70);
        assert!(owner.exit_pressure < 20);
    }
}
