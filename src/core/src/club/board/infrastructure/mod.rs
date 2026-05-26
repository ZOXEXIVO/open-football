//! Yearly facility / infrastructure review. Runs at season start: the
//! board weighs cash, annual profit, FFP standing, current facility
//! levels, attendance demand and its own infrastructure priority, then
//! emits approve/reject facility decisions. `BoardResult::process` applies
//! the approvals to `club.facilities` and debits `club.finance.balance`.
//!
//! Current facility levels are read from `BoardContext` (populated by
//! `Club::build_board_context`) so the whole review lives on the board
//! side and the decisions flow back through the normal result pipeline.

use super::context::{BoardContext, FfpStatus};
use super::decision::{BoardDecision, BoardFacility, DecisionReason};
use super::ownership::OwnershipModel;
use super::strategy::InfrastructurePriority;
use super::ClubVision;
use crate::club::facilities::FacilityLevel;

pub struct FacilityReview;

impl FacilityReview {
    /// Indicative cost of a one-tier stadium expansion, scaled by current
    /// turnout demand.
    pub const STADIUM_EXPANSION_BASE_COST: i64 = 30_000_000;

    /// Minimum seasons between two board-funded facility upgrades. The board
    /// enforces this cooldown (see `ClubBoard::run_facility_review`) so even
    /// a rich owner upgrades at a believable cadence rather than every year.
    pub const COOLDOWN_SEASONS: i32 = 2;

    /// Capital the board is willing to commit to infrastructure this year:
    /// half of positive cash, all of trailing profit, plus a wealthy
    /// owner's optional injection. FFP trouble zeroes it out.
    fn capex_capacity(ctx: &BoardContext, owner: &OwnershipModel) -> i64 {
        if matches!(ctx.ffp_status, FfpStatus::Breach) {
            return 0;
        }
        let cash_share = ctx.balance.max(0) / 2;
        let profit = ctx.profit_loss_12m.max(0);
        let owner_help = if owner.wealth >= 60 {
            (owner.wealth as i64) * 400_000
        } else {
            0
        };
        let raw = cash_share + profit + owner_help;
        if matches!(ctx.ffp_status, FfpStatus::Watchlist) {
            raw / 2
        } else {
            raw
        }
    }

    /// Current level of a given sporting facility from the board context.
    fn level_of(ctx: &BoardContext, facility: BoardFacility) -> Option<FacilityLevel> {
        match facility {
            BoardFacility::Training => Some(ctx.facility_training.clone()),
            BoardFacility::Youth => Some(ctx.facility_youth.clone()),
            BoardFacility::Academy => Some(ctx.facility_academy.clone()),
            BoardFacility::Recruitment => Some(ctx.facility_recruitment.clone()),
            BoardFacility::Stadium => None,
        }
    }

    /// Pick which sporting facility to target. Honour the explicit
    /// infrastructure priority; otherwise improve the weakest of the
    /// development facilities.
    fn target_facility(vision: &ClubVision, ctx: &BoardContext) -> BoardFacility {
        match vision.infrastructure_priority {
            InfrastructurePriority::Training => BoardFacility::Training,
            InfrastructurePriority::Youth => BoardFacility::Youth,
            InfrastructurePriority::Stadium => BoardFacility::Stadium,
            InfrastructurePriority::Commercial => BoardFacility::Recruitment,
            InfrastructurePriority::None => {
                // Improve whichever development facility is weakest.
                let candidates = [
                    (BoardFacility::Training, ctx.facility_training.to_rating()),
                    (BoardFacility::Youth, ctx.facility_youth.to_rating()),
                    (BoardFacility::Academy, ctx.facility_academy.to_rating()),
                ];
                candidates
                    .into_iter()
                    .min_by_key(|(_, r)| *r)
                    .map(|(f, _)| f)
                    .unwrap_or(BoardFacility::Training)
            }
        }
    }

    fn cost_of(ctx: &BoardContext, facility: BoardFacility) -> i64 {
        // Costs scale with the country's price level — building in an
        // expensive economy costs more — on top of the club-level scaling
        // already baked into `upgrade_cost` (square of the target rating).
        let price = ctx.country_price_level.max(0.1);
        match facility {
            BoardFacility::Stadium => {
                // Pricier when the ground is already busy (demand is
                // there, but expansion is a bigger job).
                (Self::STADIUM_EXPANSION_BASE_COST as f32 * ctx.attendance_ratio.max(1.0) * price)
                    as i64
            }
            other => Self::level_of(ctx, other)
                .and_then(|l| l.next_better())
                .map(|l| (l.upgrade_cost() as f32 * price) as i64)
                .unwrap_or(i64::MAX),
        }
    }

    /// Run the review and return the (usually one) facility decision.
    pub fn run(ctx: &BoardContext, vision: &ClubVision, owner: &OwnershipModel) -> Vec<BoardDecision> {
        let mut out = Vec::new();
        let capacity = Self::capex_capacity(ctx, owner);

        // Stadium expansion is its own track. It's only justified by an
        // explicit board mandate at solid demand, OR by genuinely strong
        // sustained demand — a wealthy owner alone is *not* enough, so the
        // board doesn't pour concrete on the back of one good month.
        let wants_stadium = match vision.infrastructure_priority {
            InfrastructurePriority::Stadium => ctx.attendance_ratio >= 1.1,
            _ => owner.wealth >= 70 && ctx.attendance_ratio >= 1.2,
        };
        if wants_stadium {
            let cost = Self::cost_of(ctx, BoardFacility::Stadium);
            if cost <= capacity {
                out.push(BoardDecision::ApproveFacilityUpgrade {
                    facility: BoardFacility::Stadium,
                    cost,
                });
                return out;
            } else if matches!(vision.infrastructure_priority, InfrastructurePriority::Stadium) {
                out.push(BoardDecision::RejectFacilityUpgrade {
                    facility: BoardFacility::Stadium,
                    reason: DecisionReason::DebtTooHigh,
                });
                return out;
            }
        }

        let facility = Self::target_facility(vision, ctx);
        let cost = Self::cost_of(ctx, facility);

        // Already maxed out (cost saturated).
        if cost == i64::MAX {
            return out;
        }

        if matches!(ctx.ffp_status, FfpStatus::Breach) {
            out.push(BoardDecision::RejectFacilityUpgrade {
                facility,
                reason: DecisionReason::FfpPressure,
            });
        } else if cost <= capacity {
            out.push(BoardDecision::ApproveFacilityUpgrade { facility, cost });
        } else if matches!(vision.infrastructure_priority, InfrastructurePriority::None) {
            // No explicit mandate and can't afford it — quietly decline.
            out.push(BoardDecision::RejectFacilityUpgrade {
                facility,
                reason: DecisionReason::LowPriority,
            });
        } else {
            out.push(BoardDecision::RejectFacilityUpgrade {
                facility,
                reason: DecisionReason::DebtTooHigh,
            });
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rich_ctx() -> BoardContext {
        let mut c = BoardContext::new();
        c.balance = 200_000_000;
        c.profit_loss_12m = 40_000_000;
        c.ffp_status = FfpStatus::Clean;
        c.attendance_ratio = 1.0;
        c
    }

    #[test]
    fn healthy_finances_and_priority_approve_upgrade() {
        let mut vision = ClubVision::default();
        vision.infrastructure_priority = InfrastructurePriority::Training;
        let owner = OwnershipModel {
            wealth: 80,
            ..Default::default()
        };
        let decisions = FacilityReview::run(&rich_ctx(), &vision, &owner);
        assert!(decisions.iter().any(|d| matches!(
            d,
            BoardDecision::ApproveFacilityUpgrade {
                facility: BoardFacility::Training,
                ..
            }
        )));
    }

    #[test]
    fn ffp_breach_rejects_upgrade() {
        let mut ctx = rich_ctx();
        ctx.ffp_status = FfpStatus::Breach;
        let mut vision = ClubVision::default();
        vision.infrastructure_priority = InfrastructurePriority::Training;
        let owner = OwnershipModel::default();
        let decisions = FacilityReview::run(&ctx, &vision, &owner);
        assert!(decisions.iter().any(|d| matches!(
            d,
            BoardDecision::RejectFacilityUpgrade {
                reason: DecisionReason::FfpPressure,
                ..
            }
        )));
    }

    #[test]
    fn broke_club_with_no_priority_declines() {
        let mut ctx = BoardContext::new();
        ctx.balance = 0;
        ctx.profit_loss_12m = -5_000_000;
        let vision = ClubVision::default(); // InfrastructurePriority::None
        let owner = OwnershipModel {
            wealth: 30,
            ..Default::default()
        };
        let decisions = FacilityReview::run(&ctx, &vision, &owner);
        assert!(decisions.iter().any(|d| matches!(
            d,
            BoardDecision::RejectFacilityUpgrade {
                reason: DecisionReason::LowPriority,
                ..
            }
        )));
    }

    #[test]
    fn high_attendance_wealthy_owner_expands_stadium() {
        let mut ctx = rich_ctx();
        ctx.attendance_ratio = 1.25;
        let vision = ClubVision::default();
        let owner = OwnershipModel {
            wealth: 85,
            ..Default::default()
        };
        let decisions = FacilityReview::run(&ctx, &vision, &owner);
        assert!(decisions.iter().any(|d| matches!(
            d,
            BoardDecision::ApproveFacilityUpgrade {
                facility: BoardFacility::Stadium,
                ..
            }
        )));
    }

    #[test]
    fn upgrade_cost_increases_with_level() {
        assert!(FacilityLevel::Average.upgrade_cost() < FacilityLevel::Superb.upgrade_cost());
    }

    #[test]
    fn wealthy_owner_alone_does_not_expand_stadium_on_modest_demand() {
        // A rich owner with only mildly-above-average demand (1.15) and no
        // explicit stadium mandate should NOT trigger an expansion — that's
        // the over-eager behaviour we want to avoid.
        let mut ctx = rich_ctx();
        ctx.attendance_ratio = 1.15;
        let vision = ClubVision::default(); // no stadium priority
        let owner = OwnershipModel {
            wealth: 90,
            ..Default::default()
        };
        let decisions = FacilityReview::run(&ctx, &vision, &owner);
        assert!(
            !decisions.iter().any(|d| matches!(
                d,
                BoardDecision::ApproveFacilityUpgrade {
                    facility: BoardFacility::Stadium,
                    ..
                }
            )),
            "modest demand + no mandate must not expand the stadium: {decisions:?}"
        );
    }

    #[test]
    fn cost_scales_with_country_price_level() {
        let mut cheap = rich_ctx();
        cheap.country_price_level = 0.5;
        let mut dear = rich_ctx();
        dear.country_price_level = 2.0;
        assert!(
            FacilityReview::cost_of(&dear, BoardFacility::Training)
                > FacilityReview::cost_of(&cheap, BoardFacility::Training),
            "a pricier economy should cost more to build in"
        );
    }
}
