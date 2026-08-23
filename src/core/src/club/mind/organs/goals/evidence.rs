//! Why a goal exists — where it came from and what it latched onto.
//!
//! The renderer, the newspaper desks and the decisions register all need
//! to say *why*, and none of them should be parsing free text. A goal
//! carries its origin (one variant) and its evidence (a bit-set of
//! atoms), which together are enough to write the sentence.

/// Which faculty a goal belongs to. Routes it to the sub-mind that owns
/// it once the sub-minds land, and groups the player-profile UI now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GoalDomain {
    Career,
    Competitive,
    Professional,
    Financial,
    Social,

    // ── Staff-side domains ──────────────────────────────────────
    /// A manager's own career and the survival of this job.
    Management,
    /// What he wants from the people above him.
    Boardroom,
    /// What he wants for and from the squad he is responsible for.
    Squad,
    /// His football. No goal points here yet — the domain exists so
    /// `PhilosophyMind` has an axis to appraise on.
    Philosophy,
    /// Himself: the workload, and whether he still wants to do this.
    Welfare,
}

impl GoalDomain {
    pub fn as_i18n_key(self) -> &'static str {
        match self {
            GoalDomain::Career => "mind_goal_domain_career",
            GoalDomain::Competitive => "mind_goal_domain_competitive",
            GoalDomain::Professional => "mind_goal_domain_professional",
            GoalDomain::Financial => "mind_goal_domain_financial",
            GoalDomain::Social => "mind_goal_domain_social",
            GoalDomain::Management => "mind_goal_domain_management",
            GoalDomain::Boardroom => "mind_goal_domain_boardroom",
            GoalDomain::Squad => "mind_goal_domain_squad",
            GoalDomain::Philosophy => "mind_goal_domain_philosophy",
            GoalDomain::Welfare => "mind_goal_domain_welfare",
        }
    }
}

/// What kind of thing put the goal there. Distinct from evidence: this
/// is the *character* of the want, and it shapes how the escalation
/// ladder reads — a man pursuing an ambition and a man nursing a
/// grievance behave differently even at identical strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GoalOrigin {
    #[default]
    Unknown,
    /// His own ambition. Nothing has gone wrong; he simply wants more.
    SelfDrive,
    /// Something was done to him.
    Grievance,
    /// The world changed around him — relegation, a new manager, a
    /// rival signed.
    Circumstance,
    /// Loyalty, home, heritage.
    Attachment,
    /// Career necessity — age, no minutes, a contract running down.
    Survival,
}

impl GoalOrigin {
    /// Multiplier on how readily a goal of this character escalates. A
    /// grievance is spoken sooner than an ambition and much sooner than
    /// an attachment, at the same strength.
    pub fn escalation_bias(self) -> f32 {
        match self {
            GoalOrigin::Unknown => 1.0,
            GoalOrigin::SelfDrive => 1.0,
            GoalOrigin::Grievance => 1.15,
            GoalOrigin::Circumstance => 1.05,
            GoalOrigin::Attachment => 0.85,
            GoalOrigin::Survival => 1.10,
        }
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            GoalOrigin::Unknown => "mind_goal_origin_unknown",
            GoalOrigin::SelfDrive => "mind_goal_origin_self_drive",
            GoalOrigin::Grievance => "mind_goal_origin_grievance",
            GoalOrigin::Circumstance => "mind_goal_origin_circumstance",
            GoalOrigin::Attachment => "mind_goal_origin_attachment",
            GoalOrigin::Survival => "mind_goal_origin_survival",
        }
    }

    pub const ALL: &'static [GoalOrigin] = &[
        GoalOrigin::SelfDrive,
        GoalOrigin::Grievance,
        GoalOrigin::Circumstance,
        GoalOrigin::Attachment,
        GoalOrigin::Survival,
    ];
}

/// The concrete signals a goal latched onto. A packed bit-set so a goal
/// stays small; every atom is a thing a renderer can name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GoalEvidence(u32);

impl GoalEvidence {
    // ── Club and league standing ────────────────────────────────
    /// He is clearly better than the club he is at.
    pub const OUTGROWN_CLUB: u32 = 1 << 0;
    /// The league itself is the ceiling, whatever the club's local size.
    pub const LEAGUE_IS_A_CEILING: u32 = 1 << 1;
    /// No route to continental football from here.
    pub const NO_CONTINENTAL_ROUTE: u32 = 1 << 2;
    /// The club is not going to win anything.
    pub const NOT_A_CONTENDER: u32 = 1 << 3;
    /// The squad around him is below his level.
    pub const SQUAD_BELOW_HIS_LEVEL: u32 = 1 << 4;
    /// The club went down.
    pub const RELEGATED: u32 = 1 << 5;

    // ── Playing time ────────────────────────────────────────────
    pub const NO_FIRST_TEAM_FOOTBALL: u32 = 1 << 6;
    pub const LOST_HIS_PLACE: u32 = 1 << 7;
    pub const PLAYED_OUT_OF_POSITION: u32 = 1 << 8;
    /// A rival was signed for his position.
    pub const RIVAL_SIGNED: u32 = 1 << 9;

    // ── The manager ─────────────────────────────────────────────
    pub const PROMISE_BROKEN: u32 = 1 << 10;
    pub const MANAGER_DOES_NOT_RATE_HIM: u32 = 1 << 11;
    pub const PUBLICLY_CRITICISED: u32 = 1 << 12;

    // ── Money ───────────────────────────────────────────────────
    pub const PAID_BELOW_HIS_PEERS: u32 = 1 << 13;
    pub const TERMS_REFUSED: u32 = 1 << 14;
    pub const CONTRACT_RUNNING_DOWN: u32 = 1 << 15;

    // ── Life and belonging ──────────────────────────────────────
    pub const HOMESICK: u32 = 1 << 16;
    pub const ISOLATED_IN_THE_SQUAD: u32 = 1 << 17;
    pub const LANGUAGE_BARRIER: u32 = 1 << 18;
    pub const FAMILY_UNSETTLED: u32 = 1 << 19;
    pub const FANS_HOSTILE: u32 = 1 << 20;
    pub const MEDIA_PRESSURE: u32 = 1 << 21;

    // ── Character and career stage ──────────────────────────────
    pub const HIGH_AMBITION: u32 = 1 << 22;
    pub const LONG_SERVICE: u32 = 1 << 23;
    pub const PRIME_YEARS_PASSING: u32 = 1 << 24;
    pub const LATE_CAREER: u32 = 1 << 25;
    /// Heritage — boyhood club, home country, a favourite.
    pub const HERITAGE_PULL: u32 = 1 << 26;
    /// He has been here long enough that nothing is left to prove.
    pub const NOTHING_LEFT_TO_PROVE: u32 = 1 << 27;
    /// Concrete outside interest exists.
    pub const CLUBS_ARE_INTERESTED: u32 = 1 << 28;

    pub const EMPTY: GoalEvidence = GoalEvidence(0);

    pub const fn of(atoms: &[u32]) -> Self {
        let mut bits = 0u32;
        let mut index = 0;
        while index < atoms.len() {
            bits |= atoms[index];
            index += 1;
        }
        GoalEvidence(bits)
    }

    #[inline]
    pub fn contains(self, atom: u32) -> bool {
        self.0 & atom != 0
    }

    #[inline]
    pub fn insert(&mut self, atom: u32) {
        self.0 |= atom;
    }

    #[inline]
    pub fn merge(&mut self, other: GoalEvidence) {
        self.0 |= other.0;
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub fn bits(self) -> u32 {
        self.0
    }

    /// How many distinct signals are behind the goal. A want with four
    /// reasons behind it is a different thing from one with a single
    /// bad week; the escalation ladder reads this.
    #[inline]
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }
}

/// Why he cannot act on a goal even though he holds it. Distinct from
/// abandoning it — a blocked goal keeps its strength and goes on
/// colouring his mood; it simply has nowhere to go this window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GoalBlocker {
    #[default]
    None,
    /// Too long left on the deal for anyone to move.
    ContractLocked,
    /// Nobody wants him.
    NoInterest,
    /// The club will not sanction it.
    ClubRefuses,
    /// He is not fit enough to be sold or selected.
    InjuryLayoff,
    /// He has just signed; the club is owed a fair look first.
    JustArrived,
    /// The window is shut.
    WindowClosed,
}

impl GoalBlocker {
    #[inline]
    pub fn is_blocked(self) -> bool {
        self != GoalBlocker::None
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            GoalBlocker::None => "mind_goal_blocker_none",
            GoalBlocker::ContractLocked => "mind_goal_blocker_contract",
            GoalBlocker::NoInterest => "mind_goal_blocker_no_interest",
            GoalBlocker::ClubRefuses => "mind_goal_blocker_club_refuses",
            GoalBlocker::InjuryLayoff => "mind_goal_blocker_injury",
            GoalBlocker::JustArrived => "mind_goal_blocker_just_arrived",
            GoalBlocker::WindowClosed => "mind_goal_blocker_window",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_atoms_are_distinct_bits() {
        let atoms = [
            GoalEvidence::OUTGROWN_CLUB,
            GoalEvidence::LEAGUE_IS_A_CEILING,
            GoalEvidence::NO_CONTINENTAL_ROUTE,
            GoalEvidence::NOT_A_CONTENDER,
            GoalEvidence::SQUAD_BELOW_HIS_LEVEL,
            GoalEvidence::RELEGATED,
            GoalEvidence::NO_FIRST_TEAM_FOOTBALL,
            GoalEvidence::LOST_HIS_PLACE,
            GoalEvidence::PLAYED_OUT_OF_POSITION,
            GoalEvidence::RIVAL_SIGNED,
            GoalEvidence::PROMISE_BROKEN,
            GoalEvidence::MANAGER_DOES_NOT_RATE_HIM,
            GoalEvidence::PUBLICLY_CRITICISED,
            GoalEvidence::PAID_BELOW_HIS_PEERS,
            GoalEvidence::TERMS_REFUSED,
            GoalEvidence::CONTRACT_RUNNING_DOWN,
            GoalEvidence::HOMESICK,
            GoalEvidence::ISOLATED_IN_THE_SQUAD,
            GoalEvidence::LANGUAGE_BARRIER,
            GoalEvidence::FAMILY_UNSETTLED,
            GoalEvidence::FANS_HOSTILE,
            GoalEvidence::MEDIA_PRESSURE,
            GoalEvidence::HIGH_AMBITION,
            GoalEvidence::LONG_SERVICE,
            GoalEvidence::PRIME_YEARS_PASSING,
            GoalEvidence::LATE_CAREER,
            GoalEvidence::HERITAGE_PULL,
            GoalEvidence::NOTHING_LEFT_TO_PROVE,
            GoalEvidence::CLUBS_ARE_INTERESTED,
        ];
        let combined = GoalEvidence::of(&atoms);
        assert_eq!(
            combined.count() as usize,
            atoms.len(),
            "two evidence atoms share a bit"
        );
    }

    #[test]
    fn merging_accumulates_without_duplicating() {
        let mut evidence = GoalEvidence::of(&[GoalEvidence::HOMESICK]);
        evidence.merge(GoalEvidence::of(&[
            GoalEvidence::HOMESICK,
            GoalEvidence::LANGUAGE_BARRIER,
        ]));
        assert_eq!(evidence.count(), 2);
        assert!(evidence.contains(GoalEvidence::LANGUAGE_BARRIER));
    }

    #[test]
    fn a_grievance_escalates_sooner_than_an_attachment() {
        assert!(
            GoalOrigin::Grievance.escalation_bias() > GoalOrigin::Attachment.escalation_bias(),
            "being wronged does not work like feeling fond"
        );
    }

    #[test]
    fn every_origin_has_a_sane_bias_and_a_key() {
        let mut keys: Vec<&str> = GoalOrigin::ALL.iter().map(|o| o.as_i18n_key()).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len());

        for origin in GoalOrigin::ALL {
            let bias = origin.escalation_bias();
            assert!(
                (0.5..=1.5).contains(&bias),
                "{origin:?} bias {bias} is out of band"
            );
        }
    }

    #[test]
    fn the_default_blocker_is_not_a_block() {
        assert!(!GoalBlocker::default().is_blocked());
        assert!(GoalBlocker::ContractLocked.is_blocked());
    }
}
