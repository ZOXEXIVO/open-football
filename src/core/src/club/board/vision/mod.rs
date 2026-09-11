//! What the board wants the club to become.
//!
//! [`ClubVision`] is the brief the manager is judged against: how the team
//! should play, who it should sign, what it is trying to win and by when.
//! Each axis is advisory — the manager may ignore it — but the board scores
//! him on it every month, and the transfer committee reads it before it
//! signs anybody.

use crate::MatchTacticType;
use crate::club::board::chairman::ChairmanPatience;
use crate::club::board::ownership::OwnershipType;
use crate::club::board::strategy::{
    InfrastructurePriority, ManagerAutonomy, ReviewFrequency, SquadProfile,
};

/// Long-term club vision — the direction the board wants the manager to
/// take the club. Drives expectations, recruitment preferences, and
/// manager-board friction. Each item is advisory: the manager can ignore
/// it but the board will judge them against it at season's end.
#[derive(Debug, Clone, Default)]
pub struct ClubVision {
    pub playing_style: VisionPlayingStyle,
    pub youth_focus: VisionYouthFocus,
    pub signing_preference: SigningPreference,
    pub financial_stance: FinancialStance,
    pub long_term_goal: Option<LongTermGoal>,
    /// Seasons allotted for the manager to reach `long_term_goal`.
    pub long_term_horizon_seasons: u8,
    /// The kind of squad the board wants assembled. Biases transfer
    /// governance and the squad-building component score.
    pub preferred_squad_profile: SquadProfile,
    /// Where surplus capital should go — drives the yearly facility review.
    pub infrastructure_priority: InfrastructurePriority,
    /// How much football autonomy the manager is granted. Combined with
    /// ownership interference to set autonomy, DoF override, and patience.
    pub manager_autonomy: ManagerAutonomy,
    /// How often the board runs a full confidence re-evaluation.
    pub review_frequency: ReviewFrequency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisionPlayingStyle {
    #[default]
    Balanced,
    AttackingFootball,
    Possession,
    HighPressing,
    DefensiveSolid,
    CounterAttack,
    DirectPlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisionYouthFocus {
    #[default]
    Balanced,
    /// Promote youth aggressively, prefer home-grown signings.
    DevelopYouth,
    /// Proven quality only; youth serves as backup.
    SignExperienced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SigningPreference {
    #[default]
    Anyone,
    /// Prefer home-nation or home-continent signings.
    Domestic,
    /// Actively scout cheaper regions for value gems.
    ValueHunter,
    /// Top-tier names only.
    Marquee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FinancialStance {
    #[default]
    Balanced,
    /// Spend now, worry later.
    Ambitious,
    /// Live within wage budget; no loans.
    Conservative,
    /// Cost-cutting mode — sell high, minimise outgoings.
    Austerity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LongTermGoal {
    WinLeague,
    WinDomesticCup,
    WinContinental,
    PromotionToTopFlight,
    EstablishTopHalf,
    Survive,
}

impl VisionPlayingStyle {
    /// The style a board would read off the football its team is already
    /// playing.
    ///
    /// Used when a vision is first installed under a manager already in
    /// post. A board that appointed the incumbent hired him to play what he
    /// plays, so deriving the brief from his shape means day one carries no
    /// manufactured friction — the drag only appears if one of them
    /// changes.
    pub fn from_tactic(tactic: Option<MatchTacticType>) -> Self {
        let Some(tactic) = tactic else {
            return VisionPlayingStyle::Balanced;
        };
        match tactic {
            MatchTacticType::T343 | MatchTacticType::T4222 => VisionPlayingStyle::AttackingFootball,
            MatchTacticType::T433 | MatchTacticType::T4231 => VisionPlayingStyle::Possession,
            MatchTacticType::T4411 | MatchTacticType::T4141 => VisionPlayingStyle::CounterAttack,
            MatchTacticType::T451 | MatchTacticType::T1333 => VisionPlayingStyle::DefensiveSolid,
            MatchTacticType::T4312
            | MatchTacticType::T442
            | MatchTacticType::T442Diamond
            | MatchTacticType::T442Narrow
            | MatchTacticType::T442DiamondWide
            | MatchTacticType::T352 => VisionPlayingStyle::Balanced,
        }
    }

    /// The football a fresh owner wants to watch.
    ///
    /// Used when the brief is written before the manager rather than after
    /// him — a takeover, or an appointment the board makes on its own terms
    /// — so the next man is hired to play something, and can be judged on
    /// whether he does. `roll` is a reproducible 0..99 draw, so two clubs
    /// with the same owner archetype do not all want the same football.
    pub fn for_owner(owner: OwnershipType, roll: u8) -> Self {
        let palette: &[VisionPlayingStyle] = match owner {
            OwnershipType::StateBacked => &[
                VisionPlayingStyle::AttackingFootball,
                VisionPlayingStyle::Possession,
            ],
            OwnershipType::PrivateEquity => &[
                VisionPlayingStyle::Balanced,
                VisionPlayingStyle::CounterAttack,
            ],
            OwnershipType::MemberOwned => &[
                VisionPlayingStyle::Possession,
                VisionPlayingStyle::HighPressing,
            ],
            OwnershipType::FamilyOwned => &[
                VisionPlayingStyle::Balanced,
                VisionPlayingStyle::DefensiveSolid,
            ],
            OwnershipType::Consortium => &[
                VisionPlayingStyle::Balanced,
                VisionPlayingStyle::HighPressing,
            ],
            OwnershipType::LocalBusiness => &[
                VisionPlayingStyle::Balanced,
                VisionPlayingStyle::DirectPlay,
                VisionPlayingStyle::CounterAttack,
            ],
        };
        palette[roll as usize % palette.len()]
    }

    /// How poorly does `tactic` embody this style? 0 = fine, up to 2 = a
    /// strong clash. Read as a monthly confidence drag, so the board
    /// slowly loses patience with a manager whose football is not the
    /// football he was hired to play. `Balanced` never drags.
    pub fn drag_against(self, tactic: MatchTacticType) -> i32 {
        use MatchTacticType::*;
        use VisionPlayingStyle::*;

        // Bias each formation on two axes: attacking weight (more forwards)
        // and possession weight (tight midfield). Hand-tuned from conventional
        // football wisdom rather than derived from match-engine values.
        let (attacking, possession) = match tactic {
            T343 => (2, 0),
            T4222 => (2, 1),
            T433 => (1, 2),
            T4231 => (1, 2),
            T4312 => (1, 1),
            T442 => (0, 0),
            T442Diamond | T442Narrow | T442DiamondWide => (0, 1),
            T352 => (0, 0),
            T4411 => (-1, 0),
            T4141 => (-1, 1),
            T451 => (-2, 0),
            T1333 => (-2, -1),
        };

        match self {
            Balanced => 0,
            AttackingFootball => (1 - attacking).max(0),
            DefensiveSolid => (1 + attacking).max(0),
            Possession => (1 - possession).max(0),
            DirectPlay => (possession).max(0),
            HighPressing => (1 - possession).max(0) + (0 - attacking).max(0),
            CounterAttack => (attacking - 1).max(0),
        }
    }
}

impl VisionYouthFocus {
    /// Reputation below which a board has no realistic route to the top of
    /// the market and builds through its own academy instead.
    const HOMEGROWN_BAR: f32 = 0.40;

    /// Where the board wants its players to come from, given who owns the
    /// club and what kind of squad it has asked for.
    pub fn derive(owner: OwnershipType, profile: SquadProfile, reputation: f32) -> Self {
        if matches!(owner, OwnershipType::MemberOwned)
            || matches!(profile, SquadProfile::Youth)
            || reputation < Self::HOMEGROWN_BAR
        {
            return VisionYouthFocus::DevelopYouth;
        }
        if matches!(owner, OwnershipType::StateBacked) || matches!(profile, SquadProfile::Stars) {
            return VisionYouthFocus::SignExperienced;
        }
        VisionYouthFocus::Balanced
    }
}

impl SigningPreference {
    /// Reputation below which a club shops in the value market because the
    /// top of it will not take its calls.
    const VALUE_HUNTING_BAR: f32 = 0.50;

    /// The kind of name the board wants to see arrive.
    pub fn derive(owner: OwnershipType, reputation: f32) -> Self {
        match owner {
            OwnershipType::StateBacked => SigningPreference::Marquee,
            OwnershipType::MemberOwned => SigningPreference::Domestic,
            OwnershipType::PrivateEquity => SigningPreference::ValueHunter,
            OwnershipType::LocalBusiness if reputation < Self::VALUE_HUNTING_BAR => {
                SigningPreference::ValueHunter
            }
            _ => SigningPreference::Anyone,
        }
    }
}

impl LongTermGoal {
    /// Reputation bands that decide what a top-flight board thinks it is
    /// entitled to.
    const ELITE: f32 = 0.85;
    const STRONG: f32 = 0.70;
    const ESTABLISHED: f32 = 0.50;
    /// Reputation a second-tier club needs before promotion is the brief
    /// rather than survival.
    const PROMOTION_CANDIDATE: f32 = 0.55;

    /// What the board is actually trying to achieve, and how many seasons
    /// it will wait — read off the division it plays in, the standing it
    /// carries and the pocket behind it.
    ///
    /// Every club gets one. Leaving the goal unset is what made the
    /// long-term reckoning unreachable and pinned every board in the world
    /// to the same ambition multiplier on its transfer budget.
    pub fn derive(tier: u8, reputation: f32, owner: OwnershipType) -> (Option<Self>, u8) {
        if tier >= 2 {
            return if reputation >= Self::PROMOTION_CANDIDATE {
                (Some(LongTermGoal::PromotionToTopFlight), 2)
            } else {
                (Some(LongTermGoal::Survive), 2)
            };
        }
        if reputation >= Self::ELITE {
            return (Some(LongTermGoal::WinLeague), 3);
        }
        if reputation >= Self::STRONG {
            let continental = matches!(
                owner,
                OwnershipType::StateBacked | OwnershipType::Consortium
            );
            let goal = if continental {
                LongTermGoal::WinContinental
            } else {
                LongTermGoal::EstablishTopHalf
            };
            return (Some(goal), 3);
        }
        if reputation >= Self::ESTABLISHED {
            return (Some(LongTermGoal::EstablishTopHalf), 3);
        }
        (Some(LongTermGoal::Survive), 2)
    }

    /// The next thing to want, once this one has been won.
    ///
    /// A board that got what it asked for does not ask for it again — it
    /// asks for more. Promotion is the exception: a promoted club starts
    /// the next horizon trying to stay up, in a division it has just
    /// reached.
    pub fn next_rung(self) -> Self {
        match self {
            LongTermGoal::Survive => LongTermGoal::EstablishTopHalf,
            LongTermGoal::EstablishTopHalf => LongTermGoal::WinDomesticCup,
            LongTermGoal::WinDomesticCup => LongTermGoal::WinLeague,
            LongTermGoal::WinLeague | LongTermGoal::WinContinental => LongTermGoal::WinContinental,
            LongTermGoal::PromotionToTopFlight => LongTermGoal::Survive,
        }
    }
}

impl ClubVision {
    /// Fewest seasons a board will wait on a long-term goal before
    /// reckoning with the manager over it.
    pub const HORIZON_MIN_SEASONS: u8 = 2;

    /// Most seasons it will wait, however patient the chairman.
    pub const HORIZON_MAX_SEASONS: u8 = 5;

    /// Install the brief a fresh boardroom writes: what to win, by when,
    /// who to sign and what kind of football to play.
    ///
    /// `style` is passed in rather than derived here because the two
    /// callers want different things from it — a bootstrap reads the
    /// incumbent's own shape so day one carries no manufactured friction,
    /// while a takeover or a new appointment picks from the owner's taste.
    pub fn install_brief(
        &mut self,
        tier: u8,
        reputation: f32,
        owner: OwnershipType,
        patience: ChairmanPatience,
        style: VisionPlayingStyle,
    ) {
        self.playing_style = style;
        self.youth_focus =
            VisionYouthFocus::derive(owner, self.preferred_squad_profile, reputation);
        self.signing_preference = SigningPreference::derive(owner, reputation);

        let (goal, horizon) = LongTermGoal::derive(tier, reputation, owner);
        self.long_term_goal = goal;
        self.long_term_horizon_seasons = Self::horizon_for(horizon, patience);
    }

    /// A patient chairman gives the project an extra season; an impatient
    /// one takes one away. Bounded at both ends so no board waits a decade
    /// and none reckons before a squad has been assembled.
    pub fn horizon_for(base: u8, patience: ChairmanPatience) -> u8 {
        let adjusted = match patience {
            ChairmanPatience::High => base as i16 + 1,
            ChairmanPatience::Low => base as i16 - 1,
            ChairmanPatience::Medium => base as i16,
        };
        adjusted.clamp(
            Self::HORIZON_MIN_SEASONS as i16,
            Self::HORIZON_MAX_SEASONS as i16,
        ) as u8
    }

    /// Ambition multiplier on the season's transfer budget. Title chasers
    /// get the biggest war chest; survival sides keep the wallet shut. A
    /// board with no stated goal is rated mid-table.
    pub fn budget_multiplier(&self) -> f64 {
        match self.long_term_goal {
            Some(LongTermGoal::WinLeague)
            | Some(LongTermGoal::WinContinental)
            | Some(LongTermGoal::WinDomesticCup)
            | Some(LongTermGoal::PromotionToTopFlight) => 1.35,
            Some(LongTermGoal::EstablishTopHalf) => 1.15,
            Some(LongTermGoal::Survive) => 0.55,
            None => 0.85,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_vision_never_drags() {
        for t in MatchTacticType::all() {
            assert_eq!(VisionPlayingStyle::Balanced.drag_against(t), 0);
        }
    }

    #[test]
    fn attacking_vision_punishes_defensive_formations() {
        assert!(VisionPlayingStyle::AttackingFootball.drag_against(MatchTacticType::T451) > 0);
        assert!(VisionPlayingStyle::AttackingFootball.drag_against(MatchTacticType::T1333) > 0);
    }

    #[test]
    fn attacking_vision_accepts_attacking_formations() {
        assert_eq!(
            VisionPlayingStyle::AttackingFootball.drag_against(MatchTacticType::T343),
            0
        );
        assert_eq!(
            VisionPlayingStyle::AttackingFootball.drag_against(MatchTacticType::T4222),
            0
        );
    }

    #[test]
    fn defensive_vision_punishes_attacking_formations() {
        assert!(VisionPlayingStyle::DefensiveSolid.drag_against(MatchTacticType::T343) > 0);
        assert!(VisionPlayingStyle::DefensiveSolid.drag_against(MatchTacticType::T4222) > 0);
    }

    #[test]
    fn possession_vision_accepts_possession_formations() {
        assert_eq!(
            VisionPlayingStyle::Possession.drag_against(MatchTacticType::T433),
            0
        );
        assert_eq!(
            VisionPlayingStyle::Possession.drag_against(MatchTacticType::T4231),
            0
        );
    }

    #[test]
    fn counter_attack_vision_prefers_modest_formations() {
        // T442 = balanced → fits counter-attack fine.
        assert_eq!(
            VisionPlayingStyle::CounterAttack.drag_against(MatchTacticType::T442),
            0
        );
        // T343 = all-out attack → clashes with counter-attack's defensive base.
        assert!(VisionPlayingStyle::CounterAttack.drag_against(MatchTacticType::T343) > 0);
    }
}
