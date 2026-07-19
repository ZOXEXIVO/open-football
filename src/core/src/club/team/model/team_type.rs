use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeamType {
    Main = 0,
    B = 1,
    Reserve = 2,
    U18 = 3,
    U19 = 4,
    U20 = 5,
    U21 = 6,
    U23 = 7,
    /// Senior reserve squad that competes in a real lower division under the
    /// "{Club} 2" naming convention (e.g. "Ural 2", "Zenit 2"). Behaves like
    /// `B` in most respects (senior bracket, finance/transfer/staff handling)
    /// but renders as the suffix "2" so the team name reads naturally.
    Second = 8,
}

impl TeamType {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            TeamType::Main => "first_team",
            TeamType::B => "b_team",
            TeamType::Reserve => "reserve_team",
            TeamType::U18 => "under_18s",
            TeamType::U19 => "under_19s",
            TeamType::U20 => "under_20s",
            TeamType::U21 => "under_21s",
            TeamType::U23 => "under_23s",
            TeamType::Second => "second_team",
        }
    }

    /// Maximum player age allowed on this team type (None = no limit)
    pub fn max_age(&self) -> Option<u8> {
        match self {
            TeamType::U18 => Some(18),
            TeamType::U19 => Some(19),
            _ => None,
        }
    }

    /// Last eligible age for an age-restricted development squad. The
    /// season a player reaches it is his showcase year — the club must
    /// decide whether he steps up or leaves, so development selection
    /// gives him a minutes bump. `None` for senior team types.
    pub fn development_age_cap(&self) -> Option<u8> {
        match self {
            TeamType::U18 => Some(18),
            TeamType::U19 => Some(19),
            TeamType::U20 => Some(20),
            TeamType::U21 => Some(21),
            TeamType::U23 => Some(23),
            _ => None,
        }
    }

    /// Age-restricted development squads (U18..U23). They compete in
    /// friendly youth leagues, so they never accumulate the *official*
    /// appearances the senior squad-utilization audit reads — the idle-days
    /// / official-games signals don't apply to them, and they must be
    /// assessed on positional depth instead.
    pub fn is_youth(&self) -> bool {
        matches!(
            self,
            TeamType::U18 | TeamType::U19 | TeamType::U20 | TeamType::U21 | TeamType::U23
        )
    }

    /// Senior squads sitting below the first team (B / Reserve / Second).
    /// A player here is an adult playing real football at the wrong level
    /// — the "stuck in the reserves" career-ambition audits key off this,
    /// while age-restricted youth squads (U18..U23) stay on the normal
    /// development pathway.
    pub fn is_senior_reserve(&self) -> bool {
        matches!(self, TeamType::B | TeamType::Reserve | TeamType::Second)
    }

    /// Senior squads that compete in a real league under their own brand.
    /// Used by the player-history pipeline (so a B/Second player's stats
    /// show their actual team and league) and by the web layer to decide
    /// when a team gets its own finances/transfers/etc. tabs.
    ///
    /// Reserve is intentionally excluded: it shares the Main team's brand
    /// and plays in a synthetic sub-league.
    pub fn is_own_team(&self) -> bool {
        matches!(self, TeamType::Main | TeamType::B | TeamType::Second)
    }

    /// Menu-row label for a team grouped under its parent club. Senior
    /// reserves (B, Second) carry their own canonical name like
    /// "Spartak Moscow 2" or "Ural B Team", so the row shows the team
    /// name as-is. Everything else (Main, Reserve, U18..U23) renders as
    /// "{Club}  |  {i18n type label}", e.g. "Spartak Moscow | First team".
    pub fn menu_label(&self, club_name: &str, team_name: &str, i18n_type_label: &str) -> String {
        match self {
            TeamType::B | TeamType::Second => team_name.to_string(),
            _ => format!("{}  |  {}", club_name, i18n_type_label),
        }
    }

    /// Sort priority for the parent club's left-menu listing. Lower comes
    /// first, so Main appears at the top, Second right after, then B,
    /// Reserve, and youth squads in descending age. Reputation tiebreaks
    /// between teams of the same type within a single club (rare).
    pub fn menu_order(&self) -> u8 {
        match self {
            TeamType::Main => 0,
            TeamType::Second => 1,
            TeamType::B => 2,
            TeamType::Reserve => 3,
            TeamType::U23 => 4,
            TeamType::U21 => 5,
            TeamType::U20 => 6,
            TeamType::U19 => 7,
            TeamType::U18 => 8,
        }
    }

    /// Youth team progression order: U18 → U19 → U20 → U21 → U23
    pub const YOUTH_PROGRESSION: &'static [TeamType] = &[
        TeamType::U18,
        TeamType::U19,
        TeamType::U20,
        TeamType::U21,
        TeamType::U23,
    ];
}

impl fmt::Display for TeamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TeamType::Main => write!(f, "First team"),
            TeamType::B => write!(f, "B Team"),
            TeamType::Reserve => write!(f, "Reserve team"),
            TeamType::U18 => write!(f, "U18"),
            TeamType::U19 => write!(f, "U19"),
            TeamType::U20 => write!(f, "U20"),
            TeamType::U21 => write!(f, "U21"),
            TeamType::U23 => write!(f, "U23"),
            // Renders as " Team" so the runtime team-name formula
            // `format!("{} {}", t.name, team_type)` turns the satellite-curated
            // "Spartak Moscow 2" into "Spartak Moscow 2 Team" without
            // double-tagging the digit. The "2" lives in the data, the suffix
            // lives in the type.
            TeamType::Second => write!(f, "Team"),
        }
    }
}

impl FromStr for TeamType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Main" => Ok(TeamType::Main),
            "B" => Ok(TeamType::B),
            "Reserve" => Ok(TeamType::Reserve),
            "U18" => Ok(TeamType::U18),
            "U19" => Ok(TeamType::U19),
            "U20" => Ok(TeamType::U20),
            "U21" => Ok(TeamType::U21),
            "U23" => Ok(TeamType::U23),
            "Second" => Ok(TeamType::Second),
            _ => Err(format!("'{}' is not a valid value for WSType", s)),
        }
    }
}
