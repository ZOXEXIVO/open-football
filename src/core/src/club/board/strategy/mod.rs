//! Board strategy enums layered onto `ClubVision`. These describe *how*
//! the board wants the squad built and the club run, beyond the playing
//! style / youth / finance axes that already existed. Each feeds a real
//! decision: squad profile drives transfer governance, infrastructure
//! priority drives the facility review, manager autonomy gates how much
//! the board overrides recruitment, and review frequency controls how
//! often confidence is re-judged.

/// The kind of squad the board wants assembled. Read by transfer
/// governance to bias which incoming players are welcomed or blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SquadProfile {
    #[default]
    Balanced,
    /// Build around U23 talent; tolerate weaker-but-young, block ageing depth.
    Youth,
    /// Peak-age win-now squad (24-29).
    PrimeAge,
    /// Galáctico policy — established stars only.
    Stars,
    /// Prefer home-nation / continental players (identity, work permits).
    Domestic,
    /// Trade on profit — buy low, develop, sell high.
    ResaleValue,
}

/// Where surplus money should go when the board funds infrastructure.
/// Drives the yearly facility review's preference ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InfrastructurePriority {
    #[default]
    None,
    Training,
    Youth,
    Stadium,
    Commercial,
}

/// How much rope the manager gets on football decisions. Combined with
/// ownership interference to decide whether the director of football
/// overrides the manager and how forgiving the sacking threshold is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ManagerAutonomy {
    Low,
    #[default]
    Medium,
    High,
}

impl ManagerAutonomy {
    /// Extra poor-month grace before the board acts. High-autonomy boards
    /// trust the manager longer.
    pub fn patience_bonus(self) -> i8 {
        match self {
            ManagerAutonomy::Low => -1,
            ManagerAutonomy::Medium => 0,
            ManagerAutonomy::High => 1,
        }
    }

    /// Confidence threshold below which the DoF starts overriding the
    /// manager's recruitment choices.
    pub fn dof_override_threshold(self) -> i32 {
        match self {
            ManagerAutonomy::Low => 60,
            ManagerAutonomy::Medium => 40,
            ManagerAutonomy::High => 25,
        }
    }
}

/// How often the board formally re-evaluates the manager. Quarterly /
/// season-end boards ignore short-term wobbles between reviews.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviewFrequency {
    #[default]
    Monthly,
    Quarterly,
    SeasonEndOnly,
}

impl ReviewFrequency {
    /// Whether a full confidence re-evaluation runs on this month index
    /// (0-based months since season start). Season-end-only boards never
    /// run a mid-season full review (handled separately at season end).
    pub fn evaluates_on_month(self, month_index: u32) -> bool {
        match self {
            ReviewFrequency::Monthly => true,
            ReviewFrequency::Quarterly => month_index % 3 == 0,
            ReviewFrequency::SeasonEndOnly => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarterly_review_skips_intermediate_months() {
        let q = ReviewFrequency::Quarterly;
        assert!(q.evaluates_on_month(0));
        assert!(!q.evaluates_on_month(1));
        assert!(!q.evaluates_on_month(2));
        assert!(q.evaluates_on_month(3));
    }

    #[test]
    fn season_end_only_never_runs_monthly() {
        let s = ReviewFrequency::SeasonEndOnly;
        for m in 0..12 {
            assert!(!s.evaluates_on_month(m));
        }
    }

    #[test]
    fn high_autonomy_is_more_patient() {
        assert!(ManagerAutonomy::High.patience_bonus() > ManagerAutonomy::Low.patience_bonus());
    }
}
