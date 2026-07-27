//! Selling players to fix the wage bill.
//!
//! Every other squad-management sweep in the sim lists players for *sporting*
//! reasons — surplus to requirements, idle, blocked, past it. None of them
//! ever read the club's bank balance. The result was a club that could run a
//! wage bill at seven times its revenue, watch its debt compound, and never
//! once put a single earner on the market, because no code path connected
//! "we cannot pay this" to "so sell someone".
//!
//! The sell *side* was half-wired already: [`SellerFeeFloor`] drops a club's
//! asking price to a distressed residual once its balance goes negative, and
//! `LoanOutReason::FinancialRelief` exists as an enum variant. But the
//! discount only ever applied to players who had *already* been listed for
//! sporting reasons, and `FinancialRelief` fired on a static
//! `ClubPhilosophy::LoanFocused` label assigned at world generation — never
//! on the club's actual finances. Clubs were willing to sell cheaply; they
//! were never willing to sell *at all*.
//!
//! This pass supplies the missing decision. It is deliberately shaped like a
//! real club's response, which escalates:
//!
//! * **Over budget but solvent** — move on the fringe. Surplus and rotation
//!   depth only; the spine is untouchable.
//! * **Owner-funded / severe** — first-team players go too. This is the
//!   "we have to cash in on someone" season.
//! * **Emergency / administration** — a genuine fire sale. Everyone has a
//!   price, including the captain, and the patience clock on recent signings
//!   is overridden. Selling the best asset to make payroll is exactly what
//!   distressed clubs do.
//!
//! Within each tier the ranking is *least essential first, biggest earner
//! first* — shed the most wage for the least sporting damage, rather than
//! simply selling the best player available.

use crate::club::DistressLevel;
use crate::club::finance::DebtStanding;
use crate::club::team::SquadAssetClass;

/// How much wage a club needs to shed, and whom it is willing to sell to do
/// it. Pure policy — no squad access — so the escalation ladder can be
/// tested directly.
pub struct WageReliefSale;

impl WageReliefSale {
    /// Fraction of the overrun a club tries to clear in one monthly pass.
    /// Shedding the whole gap at once would empty a squad in a single tick;
    /// real reductions run over windows.
    const MONTHLY_CLEARANCE: f64 = 0.50;

    /// Below this overrun the club is close enough to its mandate that
    /// selling someone would cost more in squad quality than it saves.
    const TOLERANCE: f64 = 0.05;

    /// Annual wage the club should try to remove from its bill this pass.
    ///
    /// Zero when the club is inside its mandate *and* not in debt trouble —
    /// a solvent club with room does not sell anyone for money.
    pub fn target_reduction(
        total_annual_wages: i64,
        wage_budget: i64,
        standing: DebtStanding,
        distress: DistressLevel,
    ) -> i64 {
        if total_annual_wages <= 0 {
            return 0;
        }

        // A club past its facility must cut regardless of what the board's
        // mandate says, because the mandate is a plan and the overdraft is a
        // fact. The ceiling here is expressed against the wage bill itself.
        let forced_cut: f64 = match standing {
            DebtStanding::Solvent | DebtStanding::Leveraged => 0.0,
            DebtStanding::OwnerFunded => 0.05,
            DebtStanding::Emergency => 0.15,
            DebtStanding::Administration => 0.25,
        };
        let distress_cut: f64 = match distress {
            DistressLevel::None => 0.0,
            DistressLevel::Distress => 0.0,
            DistressLevel::Severe => 0.05,
            DistressLevel::Insolvency => 0.10,
        };

        let budget_overrun = if wage_budget > 0 {
            let over = total_annual_wages as f64 - wage_budget as f64;
            if over / total_annual_wages as f64 > Self::TOLERANCE {
                over * Self::MONTHLY_CLEARANCE
            } else {
                0.0
            }
        } else {
            0.0
        };

        let forced = total_annual_wages as f64 * forced_cut.max(distress_cut);
        budget_overrun.max(forced).max(0.0) as i64
    }

    /// Whether a player of this class may be sold for financial reasons at
    /// this severity.
    ///
    /// The escalation is the whole point. A club merely over budget trims the
    /// fringe; a club in administration sells anyone. Tying every tier to the
    /// same protection list would either freeze distressed clubs (they can
    /// never sell enough) or gut healthy ones (they sell their captain over a
    /// rounding error).
    pub fn sellable(standing: DebtStanding, distress: DistressLevel, class: SquadAssetClass) -> bool {
        // Prospects are never sold for wage relief: they are the cheapest
        // players at the club, so selling them raises almost nothing while
        // costing the entire future squad.
        if matches!(class, SquadAssetClass::ProspectDevelopment) {
            return false;
        }
        // An unevaluated player must be assessed before he is moved on — the
        // same rule every other sweep honours.
        if matches!(class, SquadAssetClass::UnknownNeedsEvaluation) {
            return false;
        }

        let severity = Self::severity(standing, distress);
        match class {
            SquadAssetClass::TrueSurplus => true,
            SquadAssetClass::RotationUseful => severity >= 1,
            SquadAssetClass::FirstTeamUseful => severity >= 2,
            SquadAssetClass::CorePlayer => severity >= 3,
            _ => false,
        }
    }

    /// True once the club is desperate enough to override the patience clock
    /// on a recent signing. A fire sale does not respect an evaluation window.
    pub fn overrides_signing_protection(standing: DebtStanding, distress: DistressLevel) -> bool {
        Self::severity(standing, distress) >= 3
    }

    /// 0 = healthy, 3 = fire sale.
    fn severity(standing: DebtStanding, distress: DistressLevel) -> u8 {
        let by_standing = match standing {
            DebtStanding::Solvent => 0,
            DebtStanding::Leveraged => 1,
            DebtStanding::OwnerFunded => 2,
            DebtStanding::Emergency | DebtStanding::Administration => 3,
        };
        let by_distress = match distress {
            DistressLevel::None => 0,
            DistressLevel::Distress => 1,
            DistressLevel::Severe => 2,
            DistressLevel::Insolvency => 3,
        };
        by_standing.max(by_distress)
    }

    /// Sort rank for sale order: least essential first. Ties break on salary
    /// (highest first) at the call site, so the club sheds the most wage it
    /// can for the least sporting damage.
    pub fn sale_priority(class: SquadAssetClass) -> u8 {
        match class {
            SquadAssetClass::TrueSurplus => 0,
            SquadAssetClass::RotationUseful => 1,
            SquadAssetClass::FirstTeamUseful => 2,
            SquadAssetClass::CorePlayer => 3,
            _ => 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_club_inside_its_mandate_sells_nobody() {
        assert_eq!(
            WageReliefSale::target_reduction(
                50_000_000,
                60_000_000,
                DebtStanding::Solvent,
                DistressLevel::None
            ),
            0
        );
    }

    #[test]
    fn a_marginal_overrun_is_tolerated() {
        // 2% over is not worth breaking up a squad for.
        assert_eq!(
            WageReliefSale::target_reduction(
                51_000_000,
                50_000_000,
                DebtStanding::Solvent,
                DistressLevel::None
            ),
            0
        );
    }

    #[test]
    fn a_real_overrun_produces_a_real_target() {
        // The case from the live sim: wages far beyond the mandate.
        let target = WageReliefSale::target_reduction(
            140_000_000,
            60_000_000,
            DebtStanding::Leveraged,
            DistressLevel::Distress,
        );
        assert_eq!(target, 40_000_000);
    }

    #[test]
    fn debt_forces_a_cut_even_without_a_budget() {
        // A club with no mandate recorded still has to cut once it is past
        // its facility — the overdraft is a fact, the mandate is a plan.
        let target = WageReliefSale::target_reduction(
            100_000_000,
            0,
            DebtStanding::Emergency,
            DistressLevel::Insolvency,
        );
        assert!(target >= 15_000_000, "expected a forced cut, got {target}");
    }

    #[test]
    fn protection_escalates_with_severity() {
        use SquadAssetClass::*;

        // Healthy club over budget: fringe only.
        let healthy = (DebtStanding::Solvent, DistressLevel::None);
        assert!(WageReliefSale::sellable(healthy.0, healthy.1, TrueSurplus));
        assert!(!WageReliefSale::sellable(healthy.0, healthy.1, RotationUseful));
        assert!(!WageReliefSale::sellable(healthy.0, healthy.1, CorePlayer));

        // Leveraged: rotation depth becomes available.
        let leveraged = (DebtStanding::Leveraged, DistressLevel::None);
        assert!(WageReliefSale::sellable(leveraged.0, leveraged.1, RotationUseful));
        assert!(!WageReliefSale::sellable(leveraged.0, leveraged.1, FirstTeamUseful));

        // Owner-funded: cash in on a first-teamer.
        let funded = (DebtStanding::OwnerFunded, DistressLevel::None);
        assert!(WageReliefSale::sellable(funded.0, funded.1, FirstTeamUseful));
        assert!(!WageReliefSale::sellable(funded.0, funded.1, CorePlayer));

        // Administration: everyone has a price.
        let admin = (DebtStanding::Administration, DistressLevel::Insolvency);
        assert!(WageReliefSale::sellable(admin.0, admin.1, CorePlayer));
    }

    #[test]
    fn prospects_are_never_sold_for_wage_relief() {
        // They earn least and are worth most later — selling them raises
        // nothing and costs the future squad.
        for standing in [
            DebtStanding::Solvent,
            DebtStanding::Leveraged,
            DebtStanding::OwnerFunded,
            DebtStanding::Emergency,
            DebtStanding::Administration,
        ] {
            assert!(!WageReliefSale::sellable(
                standing,
                DistressLevel::Insolvency,
                SquadAssetClass::ProspectDevelopment
            ));
        }
    }

    #[test]
    fn unevaluated_players_are_assessed_before_being_moved_on() {
        assert!(!WageReliefSale::sellable(
            DebtStanding::Administration,
            DistressLevel::Insolvency,
            SquadAssetClass::UnknownNeedsEvaluation
        ));
    }

    #[test]
    fn distress_alone_can_drive_the_escalation() {
        // A club can be technically inside its facility but classified
        // insolvent against its wage scale; that must still open the sale.
        assert!(WageReliefSale::sellable(
            DebtStanding::Solvent,
            DistressLevel::Insolvency,
            SquadAssetClass::CorePlayer
        ));
    }

    #[test]
    fn signing_patience_only_yields_in_a_genuine_fire_sale() {
        assert!(!WageReliefSale::overrides_signing_protection(
            DebtStanding::Leveraged,
            DistressLevel::Distress
        ));
        assert!(WageReliefSale::overrides_signing_protection(
            DebtStanding::Emergency,
            DistressLevel::Severe
        ));
    }

    #[test]
    fn sale_order_sheds_the_fringe_before_the_spine() {
        use SquadAssetClass::*;
        assert!(WageReliefSale::sale_priority(TrueSurplus) < WageReliefSale::sale_priority(RotationUseful));
        assert!(
            WageReliefSale::sale_priority(RotationUseful)
                < WageReliefSale::sale_priority(FirstTeamUseful)
        );
        assert!(
            WageReliefSale::sale_priority(FirstTeamUseful)
                < WageReliefSale::sale_priority(CorePlayer)
        );
    }
}
