//! High-level strategy the coach approaches a fixture with.
//!
//! Distinct from [`SelectionPolicy`] — the policy is a tactical
//! squad-rotation knob the existing selection code already implements,
//! while [`CoachStrategy`] is the broader stance the coach takes that
//! drives *how strongly* each assessment dimension nudges the score.
//! The selection layer reads both: policy still picks the rotation
//! shape, strategy decides how much weight a poor-form signal carries
//! in that shape.
//!
//! The strategy is derived once per match-day from the staff's
//! personality, the club philosophy, the match importance, the
//! competition, and the squad state. It never replaces the policy —
//! it sits next to it as the personality-aware companion.

use crate::club::ClubPhilosophy;
use crate::club::staff::CoachProfile;

/// The coach's broad approach to a given match-day.
///
/// Ordered roughly by aggressiveness of selection — `WinNow` cares
/// about the best XI right now, `DevelopYouth` accepts a quality drop
/// for the long run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoachStrategy {
    /// Field the best XI possible — limit rotation, prioritise form.
    WinNow,
    /// Spread minutes / rest a few starters — balance result vs load.
    RotateForLoad,
    /// Hand minutes to young players in their pathway window.
    DevelopYouth,
    /// Lead in hand, see it out — defensive bias, protect what we have.
    ProtectLead,
    /// Need a goal — attacking bias, willingness to throw bodies forward.
    ChaseGame,
    /// Multi-year rebuild — long-form trust over short-form pressure.
    RebuildSquad,
    /// Trust the established core — fringe options stay benched.
    TrustCore,
    /// Cup tie against a weaker opponent — give cup chances to fringe.
    CupOpportunity,
    /// Succession-planning — the heir to a senior gets the start.
    SuccessionPlanning,
}

impl CoachStrategy {
    /// Stable i18n token for tomorrow's UI.
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            CoachStrategy::WinNow => "coach_strategy_win_now",
            CoachStrategy::RotateForLoad => "coach_strategy_rotate_for_load",
            CoachStrategy::DevelopYouth => "coach_strategy_develop_youth",
            CoachStrategy::ProtectLead => "coach_strategy_protect_lead",
            CoachStrategy::ChaseGame => "coach_strategy_chase_game",
            CoachStrategy::RebuildSquad => "coach_strategy_rebuild_squad",
            CoachStrategy::TrustCore => "coach_strategy_trust_core",
            CoachStrategy::CupOpportunity => "coach_strategy_cup_opportunity",
            CoachStrategy::SuccessionPlanning => "coach_strategy_succession_planning",
        }
    }
}

/// Inputs to [`StrategyDeriver::derive`]. Wrapped in a small input
/// struct so callers can build it once per match-day and the deriver's
/// signature stays compact as new factors are added.
#[derive(Debug, Clone)]
pub struct StrategyInputs<'a> {
    pub profile: &'a CoachProfile,
    pub philosophy: Option<ClubPhilosophy>,
    pub match_importance: f32,
    pub is_friendly: bool,
    pub is_cup: bool,
    pub is_continental: bool,
    pub is_derby: bool,
    /// 1.0 = own team much stronger than opponent, < 1.0 = weaker.
    /// Drives Cup vs WinNow framing.
    pub strength_ratio: f32,
    /// Squad-depth heuristic in [0.0, 1.0]: 1.0 = deep bench, 0.0 = thin.
    /// Thin squads stay closer to WinNow regardless of staff personality.
    pub squad_depth: f32,
}

/// Stateless namespace owning the strategy-derivation logic. Picking
/// the strategy is hand-tuned but the rules collapse cleanly: a friendly
/// or low-importance fixture is always Develop/Rotate; an important
/// league fixture is WinNow modulated by squad depth and conservatism;
/// a cup tie against a weaker side becomes CupOpportunity; etc.
pub struct StrategyDeriver;

impl StrategyDeriver {
    pub fn derive(inputs: &StrategyInputs<'_>) -> CoachStrategy {
        // Friendlies always lean to development.
        if inputs.is_friendly {
            return CoachStrategy::DevelopYouth;
        }

        // Cup ties against weaker opposition → opportunity bias only
        // when the coach isn't deeply conservative and the depth allows.
        if inputs.is_cup
            && !inputs.is_continental
            && inputs.strength_ratio >= 1.35
            && inputs.squad_depth >= 0.5
            && inputs.profile.conservatism < 0.7
        {
            return CoachStrategy::CupOpportunity;
        }

        // Low-importance / dead-rubber games → rotate or develop based on
        // coach's youth_preference. A youth-focused coach lifts straight to
        // DevelopYouth.
        if inputs.match_importance < 0.30 {
            if inputs.profile.youth_preference >= 0.55 {
                return CoachStrategy::DevelopYouth;
            }
            return CoachStrategy::RotateForLoad;
        }
        if inputs.match_importance < 0.50 {
            return CoachStrategy::RotateForLoad;
        }

        // Build/rebuild philosophy + a tactical coach who values
        // long-form data → RebuildSquad, trades short-form pressure
        // for the long-run picture.
        if matches!(
            inputs.philosophy,
            Some(ClubPhilosophy::DevelopAndSell) | Some(ClubPhilosophy::LoanFocused)
        ) && inputs.profile.youth_preference >= 0.50
            && inputs.profile.conservatism < 0.6
        {
            return CoachStrategy::RebuildSquad;
        }

        // Big games (cup / continental / derby) with a deeply conservative
        // or risk-averse coach lean to TrustCore — the heir doesn't get
        // his chance in the marquee fixture.
        if (inputs.is_cup || inputs.is_continental || inputs.is_derby)
            && inputs.match_importance >= 0.75
            && inputs.profile.conservatism >= 0.6
        {
            return CoachStrategy::TrustCore;
        }

        // High-importance fixture with a non-conservative coach and
        // succession opportunities lifts to SuccessionPlanning — used
        // when the coach has been blooding a 19-year-old behind a
        // senior. We default to WinNow when nothing tilts that way.
        if inputs.match_importance >= 0.75 && inputs.profile.youth_preference >= 0.65 {
            return CoachStrategy::SuccessionPlanning;
        }

        CoachStrategy::WinNow
    }
}

impl Default for CoachStrategy {
    fn default() -> Self {
        CoachStrategy::WinNow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Staff;
    use crate::club::staff::CoachingStyle;
    use crate::club::staff::StaffStub;

    struct StaffFixture;

    impl StaffFixture {
        fn youth_focused() -> Staff {
            let mut s = Self::baseline();
            s.staff_attributes.coaching.working_with_youngsters = 18;
            s.coaching_style = CoachingStyle::Transformational;
            s
        }

        fn conservative() -> Staff {
            let mut s = Self::baseline();
            s.staff_attributes.mental.adaptability = 5;
            s.staff_attributes.mental.discipline = 18;
            s.coaching_style = CoachingStyle::Authoritarian;
            s
        }

        fn baseline() -> Staff {
            let mut staff = StaffStub::default();
            staff.id = 1;
            staff.staff_attributes.knowledge.judging_player_ability = 14;
            staff.staff_attributes.knowledge.judging_player_potential = 14;
            staff.staff_attributes.mental.adaptability = 12;
            staff.staff_attributes.mental.determination = 12;
            staff.staff_attributes.mental.man_management = 12;
            staff.staff_attributes.mental.motivating = 12;
            staff.staff_attributes.mental.discipline = 12;
            staff.staff_attributes.coaching.tactical = 12;
            staff.staff_attributes.coaching.fitness = 12;
            staff.staff_attributes.coaching.mental = 12;
            staff.staff_attributes.coaching.working_with_youngsters = 8;
            staff.coaching_style = CoachingStyle::Democratic;
            staff
        }
    }

    #[test]
    fn friendly_always_devyouth() {
        let staff = StaffFixture::baseline();
        let profile = CoachProfile::from_staff(&staff);
        let inputs = StrategyInputs {
            profile: &profile,
            philosophy: None,
            match_importance: 0.95,
            is_friendly: true,
            is_cup: false,
            is_continental: false,
            is_derby: false,
            strength_ratio: 1.0,
            squad_depth: 0.7,
        };
        assert_eq!(
            StrategyDeriver::derive(&inputs),
            CoachStrategy::DevelopYouth
        );
    }

    #[test]
    fn cup_against_weaker_with_depth_is_opportunity() {
        let staff = StaffFixture::baseline();
        let profile = CoachProfile::from_staff(&staff);
        let inputs = StrategyInputs {
            profile: &profile,
            philosophy: None,
            match_importance: 0.55,
            is_friendly: false,
            is_cup: true,
            is_continental: false,
            is_derby: false,
            strength_ratio: 1.5,
            squad_depth: 0.7,
        };
        assert_eq!(
            StrategyDeriver::derive(&inputs),
            CoachStrategy::CupOpportunity
        );
    }

    #[test]
    fn conservative_coach_in_big_match_picks_trustcore() {
        let staff = StaffFixture::conservative();
        let profile = CoachProfile::from_staff(&staff);
        let inputs = StrategyInputs {
            profile: &profile,
            philosophy: None,
            match_importance: 0.95,
            is_friendly: false,
            is_cup: true,
            is_continental: true,
            is_derby: false,
            strength_ratio: 1.0,
            squad_depth: 0.7,
        };
        assert_eq!(StrategyDeriver::derive(&inputs), CoachStrategy::TrustCore);
    }

    #[test]
    fn youth_coach_high_importance_picks_succession() {
        let staff = StaffFixture::youth_focused();
        let profile = CoachProfile::from_staff(&staff);
        let inputs = StrategyInputs {
            profile: &profile,
            philosophy: None,
            match_importance: 0.85,
            is_friendly: false,
            is_cup: false,
            is_continental: false,
            is_derby: false,
            strength_ratio: 1.0,
            squad_depth: 0.7,
        };
        assert_eq!(
            StrategyDeriver::derive(&inputs),
            CoachStrategy::SuccessionPlanning
        );
    }
}
