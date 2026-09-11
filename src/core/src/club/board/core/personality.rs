//! Where a club's owner, chairman and vision come from.
//!
//! Derived once, on the first tick that carries context, from durable
//! signals the club already has — reputation, the balance sheet, the
//! league's money — so two clubs of similar size get different boardrooms
//! without a single hard-coded name.

use crate::club::board::ClubBoard;
use crate::club::board::chairman::{ChairmanAmbition, ChairmanPatience, ChairmanProfile};
use crate::club::board::context::BoardContext;
use crate::club::board::ownership::{ClubBenefactor, OwnershipModel, OwnershipType};
use crate::club::board::strategy::{
    InfrastructurePriority, ManagerAutonomy, ReviewFrequency, SquadProfile,
};
use crate::club::board::vision::{FinancialStance, VisionPlayingStyle};

impl ClubBoard {
    /// Ownership archetype → legacy chairman knobs.
    ///
    /// One table, called both at bootstrap and whenever the archetype
    /// changes under a club — a flip that set the ownership type and left
    /// the chairman's ambition and patience on the old owner's row was
    /// exactly the half-derived state the audit found.
    pub(crate) fn map_chairman_knobs(chairman: &mut ChairmanProfile, owner: &OwnershipModel) {
        chairman.ambition = match owner.ownership_type {
            OwnershipType::StateBacked => ChairmanAmbition::Reckless,
            OwnershipType::Consortium if owner.wealth() >= 70 => ChairmanAmbition::Ambitious,
            OwnershipType::FamilyOwned | OwnershipType::MemberOwned => {
                ChairmanAmbition::Conservative
            }
            _ => ChairmanAmbition::Balanced,
        };
        chairman.patience = match owner.ownership_type {
            OwnershipType::StateBacked | OwnershipType::PrivateEquity => ChairmanPatience::Low,
            OwnershipType::MemberOwned | OwnershipType::FamilyOwned => ChairmanPatience::High,
            _ => ChairmanPatience::Medium,
        };
    }

    /// Derive the ownership archetype + opening vision from durable club
    /// signals. Runs once, on the first simulate tick that has context.
    pub(crate) fn bootstrap_personality(&mut self, ctx: &BoardContext, seed: u32) {
        let owner = OwnershipModel::derive(
            ctx.reputation_score,
            ctx.balance,
            ctx.country_economic_factor,
            // The PROJECTION, not the trailing sum. Bootstrap runs on the
            // first tick a club has context, when its finance history is
            // still empty — reading the trailing zero made every
            // cash-positive club in the world state-backed with a Reckless
            // chairman for four seasons of EMA decay.
            ClubBenefactor::signal(
                ctx.balance,
                ctx.total_annual_wages as i64,
                ctx.projected_annual_income,
            ),
            seed,
        );

        // Map ownership → legacy chairman knobs so budget / patience logic
        // reflects the derived owner.
        Self::map_chairman_knobs(&mut self.chairman, &owner);

        // Opening vision from archetype — gives clubs distinct stories.
        self.vision.preferred_squad_profile = match owner.ownership_type {
            OwnershipType::StateBacked => SquadProfile::Stars,
            OwnershipType::PrivateEquity => SquadProfile::ResaleValue,
            OwnershipType::MemberOwned => SquadProfile::Youth,
            OwnershipType::Consortium => SquadProfile::PrimeAge,
            OwnershipType::LocalBusiness if ctx.reputation_score < 0.4 => SquadProfile::Youth,
            _ => SquadProfile::Balanced,
        };
        self.vision.infrastructure_priority = match owner.ownership_type {
            OwnershipType::MemberOwned => InfrastructurePriority::Youth,
            OwnershipType::FamilyOwned | OwnershipType::StateBacked => {
                InfrastructurePriority::Stadium
            }
            OwnershipType::Consortium => InfrastructurePriority::Commercial,
            OwnershipType::PrivateEquity => InfrastructurePriority::None,
            OwnershipType::LocalBusiness => InfrastructurePriority::Training,
        };
        self.vision.manager_autonomy = if owner.interference >= 60 {
            ManagerAutonomy::Low
        } else if owner.interference >= 35 {
            ManagerAutonomy::Medium
        } else {
            ManagerAutonomy::High
        };
        self.vision.review_frequency = match owner.ownership_type {
            OwnershipType::MemberOwned | OwnershipType::FamilyOwned => ReviewFrequency::Quarterly,
            _ => ReviewFrequency::Monthly,
        };
        self.vision.financial_stance = match owner.ownership_type {
            OwnershipType::StateBacked => FinancialStance::Ambitious,
            OwnershipType::PrivateEquity
            | OwnershipType::MemberOwned
            | OwnershipType::FamilyOwned => FinancialStance::Conservative,
            _ => FinancialStance::Balanced,
        };

        // …and the half of the brief that was never written. Without it
        // `playing_style` stayed `Balanced` for every club in the world, so
        // the style drag was permanently zero and every manager's style
        // alignment climbed to 100; `long_term_horizon_seasons` stayed 0, so
        // the long-term reckoning returned early for everybody; and
        // `long_term_goal` stayed `None`, so `ClubVision::budget_multiplier`
        // handed all 200-odd clubs the same 0.85.
        //
        // The style is read off the football the incumbent already plays: a
        // board that appointed him hired him to play it, and a brief written
        // over his head on day one would be friction nobody chose. A fresh
        // owner writes his own — see `VisionPlayingStyle::for_owner`.
        self.vision.install_brief(
            ctx.league_tier,
            ctx.reputation_score,
            owner.ownership_type,
            self.chairman.patience,
            VisionPlayingStyle::from_tactic(ctx.main_tactic),
        );

        self.ownership = owner;
        // The pile the owner was first seen holding — what the yearly
        // top-up refills towards, so a benefactor's cash is a fund he
        // maintains rather than a one-off pot that drains in three
        // windows.
        self.ownership
            .stamp_idle_at_derive(ClubBenefactor::idle_cash(
                ctx.balance,
                ctx.total_annual_wages as i64,
            ));
        self.personality_initialized = true;
    }
}
