//! How deep each squad is allowed to run, and who says a keeper may move.
//!
//! Every club-level squad pass reads its thresholds from here. They used to be
//! duplicated per file, carrying "keep them in sync" comments — which is
//! exactly the sort of thing that drifts: the promotion engine and the
//! development-loan pass have to agree on when the first team is short, or the
//! one loans away the player the other is about to call up.

use std::collections::HashSet;

use chrono::NaiveDate;

use crate::club::staff::goalkeeping::{KeeperAdvice, KeeperRoomPlan};
use crate::club::staff::perception::AbilityEstimator;
use crate::{PlayerFieldPositionGroup, Team};

/// Squad sizes below which a pass stops taking players out of a team.
pub(in crate::club::core) struct SquadSize;

impl SquadSize {
    /// Minimum players a youth/reserve team should keep to remain functional.
    pub(in crate::club::core) const MIN_YOUTH: usize = 11;
    /// Minimum players the main team should keep before allowing demotions.
    pub(in crate::club::core) const MIN_MAIN: usize = 22;
    /// Observable level ABOVE the first team's promotion floor at which a
    /// promotion stops being a judgement call and becomes an obvious one —
    /// the boy is not "ready soon", he is already better than the last man
    /// in the group.
    ///
    /// Two guards stand down at this margin, and both were traps rather than
    /// policies. [`Self::MIN_YOUTH`] used to block every non-overage
    /// promotion out of a squad already at eleven, which is exactly the
    /// state a squad holding first-team players sits in — so the four real
    /// players stayed there indefinitely and the emergency call-up, which
    /// fills the LOWEST bracket first and only up to fourteen, never reached
    /// them. And a `Loa` badge blocked promotion outright, so the first loan
    /// intent that landed on such a player shut the door for good. Fielding
    /// the youth side is the academy's job; a club does not loan out the boy
    /// who has just become first-team ready.
    pub(in crate::club::core) const PROMOTION_CLEAR_MARGIN: u8 = 8;
}

/// Depth the first team needs at each position group before a hole there is
/// real. The promotion engine and the development-loan pass both read it, so
/// "is the main team short at this position?" has one answer.
pub(in crate::club::core) struct MainSquadDepth;

impl MainSquadDepth {
    /// Level an understrength group falls back to: when the main team is
    /// short at a position any decent youth fills the gap, so there is no
    /// "below the first team" band to loan from.
    pub(in crate::club::core) const GAP_FLOOR: u8 = 60;

    /// Below this many players at a group the main team is short there.
    pub(in crate::club::core) fn min_for(group: PlayerFieldPositionGroup) -> usize {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 2,
            PlayerFieldPositionGroup::Defender => 6,
            PlayerFieldPositionGroup::Midfielder => 6,
            PlayerFieldPositionGroup::Forward => 4,
        }
    }
}

/// The level at/above which a non-main player is pulled into the first team,
/// per position group — and whether that group is a body short.
///
/// One bar, read by everything that has an opinion about a promotion. The
/// weekly [`crate::Club::rebalance_squads`] promotes off it; the youth
/// development-loan pass leaves anyone at/above it for the rebalance to
/// promote instead of loaning him away; the parked-prime pass treats anyone
/// within reach of it as genuine first-team cover. They have to agree, or one
/// ships out the player another is about to call up.
///
/// Measured in the coach-observable level (visible skill + results +
/// training), on both sides of every comparison, never the hidden CA digit: a
/// promotion is a staff judgement on what they can see, the same basis as the
/// surplus trim and the squad-asset classifier.
///
/// A single global "bottom-3" floor across all positions caused a keeper
/// ping-pong: a youth GK at 82 cleared the global floor (~75) even when the
/// main team already had three senior keepers at 100+, so the depth cap
/// demoted a keeper every pass and another youth GK got promoted the next
/// week. The position-aware bar is the real signal — "does this youth displace
/// an actual peer at the same role?" — and it resolves the churn without
/// special-casing the goalkeeper position.
///
/// Snapshotted once per pass: the thresholds used to be closures re-walking
/// the whole main squad for every player in every other squad.
pub(in crate::club::core) struct PromotionBar {
    groups: [(PlayerFieldPositionGroup, u8, bool); PlayerFieldPositionGroup::COUNT],
}

impl PromotionBar {
    pub(in crate::club::core) fn snapshot(main: &Team) -> Self {
        PromotionBar {
            groups: PlayerFieldPositionGroup::ALL.map(|group| {
                let (count, worst) = main
                    .players
                    .iter()
                    .filter(|p| p.position().position_group() == group)
                    .map(AbilityEstimator::observable_level)
                    .fold((0usize, u8::MAX), |(c, w), a| (c + 1, w.min(a)));
                let short = count < MainSquadDepth::min_for(group);
                // A group short of bodies (retirement, transfer, release) takes
                // any youth above the gap floor to plug the hole; otherwise the
                // bar is strictly above the current worst — an equal level
                // wouldn't improve depth but would still trigger the demotion
                // cycle. An empty group is always short, so `worst` is never
                // read at its `u8::MAX` seed.
                let floor = if short {
                    MainSquadDepth::GAP_FLOOR
                } else {
                    worst.saturating_add(1)
                };
                (group, floor, short)
            }),
        }
    }

    pub(in crate::club::core) fn floor(&self, group: PlayerFieldPositionGroup) -> u8 {
        self.groups
            .iter()
            .find(|(g, _, _)| *g == group)
            .map(|(_, floor, _)| *floor)
            .unwrap_or(u8::MAX)
    }

    /// Is the first team a body short in this group? A promotion into a hole
    /// is never held up by the squad it comes out of — the youth side can be
    /// topped up, a matchday XI cannot.
    pub(in crate::club::core) fn main_short(&self, group: PlayerFieldPositionGroup) -> bool {
        self.groups
            .iter()
            .find(|(g, _, _)| *g == group)
            .map(|(_, _, short)| *short)
            .unwrap_or(false)
    }
}

/// Per-position depth a single non-competing squad keeps before the remainder
/// are loaned out for development. Smaller than a senior squad's depth: such a
/// side plays roughly once a week, so a third keeper or a deep outfield
/// reserve never sees minutes and develops better on loan.
pub(in crate::club::core) struct YouthSquadDepth;

impl YouthSquadDepth {
    pub(in crate::club::core) fn keep_for(group: PlayerFieldPositionGroup) -> usize {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 2,
            PlayerFieldPositionGroup::Defender => 7,
            PlayerFieldPositionGroup::Midfielder => 7,
            PlayerFieldPositionGroup::Forward => 5,
        }
    }
}

/// Policy for the age-based youth development-loan pass.
pub(in crate::club::core) struct YouthDevelopmentLoanPolicy;

impl YouthDevelopmentLoanPolicy {
    /// Age at/above which a youth player is treated as ready for senior loan
    /// football. Below it he keeps developing in the youth side rather than
    /// being shipped to a senior club too early.
    pub(in crate::club::core) const SENIOR_LOAN_AGE: u8 = 18;

    /// Players a youth squad must retain per group so it can still field a
    /// match — the development-loan pass never strips a group below this. A
    /// 4-4-2 fielding-XI footprint; deeper squads loan the senior-ready fringe
    /// above it. Deliberately below [`YouthSquadDepth::keep_for`]: the surplus
    /// pass trims to a comfortable rotation depth, this pass then loans the
    /// blocked-but-ready players down toward a usable XI.
    pub(in crate::club::core) fn min_field(group: PlayerFieldPositionGroup) -> usize {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 1,
            PlayerFieldPositionGroup::Defender => 4,
            PlayerFieldPositionGroup::Midfielder => 4,
            PlayerFieldPositionGroup::Forward => 2,
        }
    }
}

/// The goalkeeping department's say over which keepers the squad passes may
/// move.
///
/// Two judgements they cannot make for themselves, both of them specialist
/// ones. Which keeper the club is actively building around — he is not
/// surplus and he is not stagnating, whatever the depth chart says, because
/// the first team has just started naming him. And which keeper is blocked
/// badly enough that a season of men's football elsewhere is the only thing
/// left to give him — a judgement about the whole keeper room, not about
/// this one squad's depth at a position.
///
/// An empty plan protects nobody and asks for nobody, so a club that has
/// never reviewed its keeper room gets exactly the audit it always had.
pub(in crate::club::core) struct KeeperLoanView {
    protected: HashSet<u32>,
    wanted_out: HashSet<u32>,
}

impl KeeperLoanView {
    pub(in crate::club::core) fn of(plan: Option<&KeeperRoomPlan>, today: NaiveDate) -> Self {
        let mut protected: HashSet<u32> = HashSet::new();
        let mut wanted_out: HashSet<u32> = HashSet::new();
        let Some(plan) = plan else {
            return KeeperLoanView {
                protected,
                wanted_out,
            };
        };

        for (player_id, assignment) in plan.assignments() {
            if assignment.tier.promises_minutes() || assignment.tier.is_senior_group() {
                protected.insert(player_id);
            }
        }
        protected.extend(plan.heir());
        protected.extend(plan.nominated(today));

        for advice in plan.recommendations() {
            if advice.advice != KeeperAdvice::LoanHimOutForMinutes {
                continue;
            }
            if let Some(id) = advice.player_id {
                wanted_out.insert(id);
            }
        }
        // A keeper cannot be both. The department's own review never says
        // both about one man, but a plan read mid-revision could.
        wanted_out.retain(|id| !protected.contains(id));

        KeeperLoanView {
            protected,
            wanted_out,
        }
    }

    pub(in crate::club::core) fn protects(&self, player_id: u32) -> bool {
        self.protected.contains(&player_id)
    }

    pub(in crate::club::core) fn wants_out(&self, player_id: u32) -> bool {
        self.wanted_out.contains(&player_id)
    }
}
