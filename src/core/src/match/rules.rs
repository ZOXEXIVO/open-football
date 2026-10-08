//! Per-competition match rules — substitution allowance, bench size
//! and a couple of related knobs the engine consults at kickoff. The
//! struct is the single source of truth for "what kind of match is
//! this?": the engine reads it once into `MatchContext`, the squad
//! builder reads it to size the bench, and the higher-level
//! orchestration (league, cup, continental) sets it according to
//! competition convention.

use crate::r#match::fixture::CompetitionKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchRules {
    /// Maximum substitutions per team across the whole match. Modern
    /// FA/UEFA convention is 5; classic rules used 3. Friendlies
    /// effectively waive the cap (`usize::MAX`). Knockout ties get a
    /// +1 bonus on entering extra time — the engine applies that
    /// independently, *on top of* this cap.
    pub max_substitutions_per_team: usize,
    /// Per-stoppage cap. Real referees typically allow up to three
    /// players to leave the pitch at one whistle; we cap at this in a
    /// single substitution pass so a manager can't unload the entire
    /// bench at minute 60. Lower numbers throttle substitutions across
    /// more frequent passes.
    pub max_substitutions_per_pass: usize,
    /// Number of bench slots the squad builder is allowed to fill.
    /// Friendlies pass `usize::MAX`; competitive matches default to 9.
    pub bench_size: usize,
    /// Whether knockout ties get the canonical +1 substitution slot
    /// upon entering extra time. FIFA-style rule, defaults true.
    pub allow_extra_time_extra_sub: bool,
}

impl MatchRules {
    /// Modern league / cup / continental rule set: 5 substitutions per
    /// team, 9-player bench, ET bonus enabled. The default for every
    /// non-friendly match unless a competition explicitly overrides.
    pub const fn modern() -> Self {
        Self {
            max_substitutions_per_team: 5,
            max_substitutions_per_pass: 3,
            bench_size: 9,
            allow_extra_time_extra_sub: true,
        }
    }

    /// Classic three-substitution rule, kept for legacy / regional
    /// competitions whose laws still cap subs at 3.
    pub const fn classic() -> Self {
        Self {
            max_substitutions_per_team: 3,
            max_substitutions_per_pass: 2,
            bench_size: 7,
            allow_extra_time_extra_sub: false,
        }
    }

    /// Friendlies: managers rotate freely, no cap. Bench size also
    /// uncapped so a pre-season match can field 22 different players.
    pub const fn friendly() -> Self {
        Self {
            max_substitutions_per_team: usize::MAX,
            max_substitutions_per_pass: usize::MAX,
            bench_size: usize::MAX,
            allow_extra_time_extra_sub: false,
        }
    }

    /// International (national-team) fixtures. Currently identical to
    /// `modern()` — separate variant kept so future tuning (FIFA-only
    /// concussion subs, extra-time rules) lands in one place.
    pub const fn international() -> Self {
        Self::modern()
    }

    /// The rules a competition of this kind plays to, when it has not
    /// opted into others (`classic()`).
    pub const fn resolve_default(competition: CompetitionKind) -> Self {
        match competition {
            CompetitionKind::Friendly => Self::friendly(),
            CompetitionKind::International => Self::international(),
            CompetitionKind::League | CompetitionKind::DomesticCup | CompetitionKind::Continental => {
                Self::modern()
            }
        }
    }
}

impl Default for MatchRules {
    fn default() -> Self {
        Self::modern()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modern_default_caps_subs_at_five() {
        let r = MatchRules::modern();
        assert_eq!(r.max_substitutions_per_team, 5);
        assert!(r.allow_extra_time_extra_sub);
    }

    #[test]
    fn classic_caps_subs_at_three_no_et_bonus() {
        let r = MatchRules::classic();
        assert_eq!(r.max_substitutions_per_team, 3);
        assert!(!r.allow_extra_time_extra_sub);
    }

    #[test]
    fn friendly_waives_caps_via_usize_max() {
        let r = MatchRules::friendly();
        assert_eq!(r.max_substitutions_per_team, usize::MAX);
        assert_eq!(r.bench_size, usize::MAX);
        assert!(!r.allow_extra_time_extra_sub);
    }

    #[test]
    fn resolve_default_routes_friendly_to_unlimited() {
        let r = MatchRules::resolve_default(CompetitionKind::Friendly);
        assert_eq!(r.max_substitutions_per_team, usize::MAX);
    }

    #[test]
    fn resolve_default_routes_competitive_to_modern() {
        let r = MatchRules::resolve_default(CompetitionKind::League);
        assert_eq!(r.max_substitutions_per_team, 5);
        assert!(r.allow_extra_time_extra_sub);
    }

    #[test]
    fn international_currently_aliases_modern() {
        let r = MatchRules::international();
        let m = MatchRules::modern();
        assert_eq!(r, m);
    }
}
