//! The gate stack the loan agreement replaced, kept for
//! `OF_LOAN_AGREEMENT_OFF`.
//!
//! Every rule here was a live conjunctive gate before a loan became a
//! product of four continuous terms. It lives in one file so the census
//! arm restores what it claims to restore, and so the default path
//! carries none of it.

use crate::PlayerFieldPositionGroup;
use crate::transfers::loan::guard::{LoanAssetGuard, LoanBorrowerProfile};
use crate::transfers::pipeline::PlayerSummary;

use crate::transfers::gate::EffectivePlayerReputation;
use crate::transfers::pipeline::trace::{MarketSwitches, TransferTrace};

use crate::transfers::squad::LevelBand;

use super::{BorrowerPositionDepth, ForeignUnsolicitedLoanTarget};

/// What the HEAD stack read about one domestic pair. Assembled by the
/// sweep and handed over whole, so nothing about the arm leaks back into
/// the default path.
pub(in crate::transfers::loan) struct LegacyDomesticGate<'a> {
    pub(in crate::transfers::loan) player_id: u32,
    pub(in crate::transfers::loan) borrower_name: &'a str,
    pub(in crate::transfers::loan) group: PlayerFieldPositionGroup,
    pub(in crate::transfers::loan) ability: u8,
    pub(in crate::transfers::loan) is_development: bool,
    pub(in crate::transfers::loan) parent_best_in_group: u8,
    pub(in crate::transfers::loan) level: &'a LoanDestinationLevel,
    pub(in crate::transfers::loan) depth: &'a BorrowerPositionDepth,
    pub(in crate::transfers::loan) guard: Option<&'a LoanAssetGuard>,
    pub(in crate::transfers::loan) borrower: Option<LoanBorrowerProfile>,
}

/// The same, for the cross-border sweep, which reads a staged summary
/// rather than a listing.
pub(in crate::transfers::loan) struct LegacyForeignGate<'a> {
    pub(in crate::transfers::loan) summary: &'a PlayerSummary,
    pub(in crate::transfers::loan) borrower_rep: u16,
    pub(in crate::transfers::loan) borrower_league_rep: u16,
    pub(in crate::transfers::loan) depth: &'a BorrowerPositionDepth,
    pub(in crate::transfers::loan) borrower: Option<LoanBorrowerProfile>,
}

/// The destination verdict as HEAD produced it: a boolean, reached
/// through four conjunctive readings.
pub struct LegacyLoanGuard;

impl LegacyLoanGuard {
    /// Value ÷ borrower annual income above which the asset was
    /// untouchable …
    const W_MAX: f64 = 1.0;
    /// … and the wage share ÷ what the borrower could pay above which
    /// the wage was.
    const CARRY_MAX: f64 = 1.0;
    /// Key-player floor gap a peer borrower could sit below the parent's.
    const PEER_BAND: i16 = 10;
    /// Share of the parent's league standing a peer borrower's own
    /// competition had to reach.
    const PEER_LEAGUE_SHARE: f32 = 0.85;
    /// Standing at/above which only peers borrowed him …
    const PEER_STANDING: f32 = 1.0;
    /// … and below which the development floors owned the destination.
    const RAW_STANDING: f32 = 0.35;
    /// Multiples of the borrower's own reputation a cross-border loanee's
    /// home club could be worth …
    const FOREIGN_REP_CEILING: f32 = 2.0;
    /// … and the share of it the borrower had to reach.
    const FOREIGN_REP_FLOOR: f32 = 0.35;

    /// May this loan happen at all, to THIS borrower?
    pub fn allows(guard: &LoanAssetGuard, borrower: &LoanBorrowerProfile) -> bool {
        // Priced with no subsidy, which is the split HEAD wrote.
        let verdict = guard.assess(borrower, 1.0, 0.0);
        if guard.parent_holds() || verdict.weight > Self::W_MAX || verdict.carry > Self::CARRY_MAX {
            return false;
        }
        if verdict.standing < Self::RAW_STANDING || guard.is_development() {
            return true;
        }
        if verdict.standing >= Self::PEER_STANDING {
            return Self::clears_peer_band(guard, borrower);
        }
        true
    }

    /// A club of the parent's own standing, in a competition of the
    /// parent's own standard. An unknown competition on either side
    /// stood the division half down; the club half still spoke.
    fn clears_peer_band(guard: &LoanAssetGuard, borrower: &LoanBorrowerProfile) -> bool {
        let group = guard.group();
        let club_ok = borrower.anchor.key_floor(group)
            >= guard.parent_anchor().key_floor(group) - Self::PEER_BAND;
        let parent_league_rep = guard.parent_league_rep();
        let league_ok = parent_league_rep == 0
            || borrower.league_rep == 0
            || borrower.league_rep as f32 >= parent_league_rep as f32 * Self::PEER_LEAGUE_SHARE;
        club_ok && league_ok
    }

    /// Realism gate for the borrower side of a domestic loan: players
    /// don't drop from a giant to a minnow for a bit-part role.
    /// `borrower_rep` / `parent_rep` are main-team world reputations
    /// (0..10000).
    ///
    /// **Readiness, not age, decides how far a player may drop.** The signal is
    /// how far he sits below the parent's best at his position:
    ///   * A genuinely raw player (`very_raw` — 25+ below the best) tolerates
    ///     the biggest drop: for him ANY senior football is the point, and the
    ///     stricter `would_get_loan_minutes` gate already guarantees he plays
    ///     wherever he lands. A raw *development* youngster (the classic
    ///     teenage keeper) has the floor lifted entirely so he can drop to a
    ///     small club where he STARTS; a raw non-development player keeps a
    ///     light 0.12 floor.
    ///   * Everyone else — INCLUDING a young **displaced first-choice** who is
    ///     close to the parent's best — is held to the peer-level 0.25 floor.
    ///     He moves down a tier or two for minutes, not several tiers to a
    ///     minnow's bench.
    ///
    /// The club-standing half of [`LoanDestinationLevel`] on its own —
    /// what [`Self::loan_level_ok`] reduces to when neither competition
    /// is known, and the entry point the gate's own tests measure
    /// against.
    #[cfg(test)]
    pub(in crate::transfers::loan) fn loan_reputation_drop_ok(
        borrower_rep: u16,
        parent_rep: u16,
        player_ability: u8,
        parent_best_in_group: u8,
        is_development: bool,
    ) -> bool {
        Self::loan_level_ok(
            borrower_rep,
            parent_rep,
            player_ability,
            parent_best_in_group,
            is_development,
            0,
            0,
        )
    }

    /// Both halves of [`LoanDestinationLevel`], with the league context a
    /// caller can supply. Zero on either side stands the division gate
    /// down — that is its own "unknown competition" rule, not a
    /// suspension.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::transfers::loan) fn loan_level_ok(
        borrower_rep: u16,
        parent_rep: u16,
        player_ability: u8,
        parent_best_in_group: u8,
        is_development: bool,
        parent_league_rep: u16,
        borrower_league_rep: u16,
    ) -> bool {
        LoanDestinationLevel {
            ability: player_ability,
            parent_best_in_group,
            parent_rep,
            borrower_rep,
            parent_league_rep,
            borrower_league_rep,
            is_development,
        }
        .is_plausible()
    }

    /// One `loan` trace line per (traced player, candidate borrower):
    /// every destination gate's own reading, side by side, so the funnel
    /// can be read as a table instead of re-derived from file:line.
    ///
    /// Always returns `true` — it is a diagnostic, never a gate — so a
    /// caller folds it into its filter chain wherever it wants the reading
    /// taken. Costs one cached `OnceLock` read when disarmed.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::transfers::loan) fn trace_loan_destination(
        player_id: u32,
        borrower_name: &str,
        group: PlayerFieldPositionGroup,
        ability: u8,
        is_development: bool,
        parent_best_in_group: u8,
        level: &LoanDestinationLevel,
        depth: &BorrowerPositionDepth,
    ) -> bool {
        if !TransferTrace::is(player_id) {
            return true;
        }
        TransferTrace::line(
            player_id,
            "loan",
            format!(
                "borrower={borrower_name} dev={is_development} readiness={:.2} \
                 division_floor={:.3} rep={}/{} league={}/{} standing_ok={} division_ok={} \
                 room={} minutes={} best_here={}",
                level.readiness(),
                level.division_floor(),
                level.borrower_rep,
                level.parent_rep,
                level.borrower_league_rep,
                level.parent_league_rep,
                level.clears_club_standing(),
                level.clears_division(),
                depth.has_room_for(group, ability, is_development),
                depth.would_get_loan_minutes(group, ability, is_development, parent_best_in_group),
                depth.best_in_group(group),
            ),
        );
        true
    }

    /// The same destination verdict across a border, where the parent
    /// side is only reachable through the player's summary. Legacy-only:
    /// the default path prices the pair through the agreement.
    pub(in crate::transfers::loan) fn foreign_loan_guard_allows(
        target: &PlayerSummary,
        borrower: Option<&LoanBorrowerProfile>,
    ) -> bool {
        if MarketSwitches::loan_guard_off() {
            return true;
        }
        let Some(borrower) = borrower else {
            return true;
        };
        let guard = LoanAssetGuard::from_summary(
            target.skill_ability,
            target.age,
            target.position_group,
            target.club_world_reputation,
            target.club_best_in_group,
            target.seller_ctx.league_reputation,
            target.estimated_value,
            target.salary,
            target.is_loan_listed,
            EffectivePlayerReputation::compute(
                target.world_reputation,
                target.current_reputation,
                target.home_reputation,
                false,
            ),
        );
        if TransferTrace::is(target.player_id) {
            let verdict = guard.assess(borrower, 1.0, 0.0);
            TransferTrace::line(
                target.player_id,
                "loan",
                format!("foreign {}", guard.diagnostics(borrower, &verdict)),
            );
        }
        LegacyLoanGuard::allows(&guard, borrower)
    }

    /// The whole domestic stack: the destination trace, the two depth
    /// readings, plausibility, and the guard.
    pub(in crate::transfers::loan) fn domestic_allows(gate: &LegacyDomesticGate<'_>) -> bool {
        Self::trace_loan_destination(
            gate.player_id,
            gate.borrower_name,
            gate.group,
            gate.ability,
            gate.is_development,
            gate.parent_best_in_group,
            gate.level,
            gate.depth,
        ) && gate
            .depth
            .has_room_for(gate.group, gate.ability, gate.is_development)
            && gate.depth.would_get_loan_minutes(
                gate.group,
                gate.ability,
                gate.is_development,
                gate.parent_best_in_group,
            )
            && gate.level.is_plausible()
            && match (gate.guard, gate.borrower.as_ref()) {
                (Some(guard), Some(borrower)) => Self::allows(guard, borrower),
                _ => true,
            }
    }

    /// The cross-border stack: two reputation bars either side of the
    /// pair, the same depth readings, the level rule and the guard.
    pub(in crate::transfers::loan) fn foreign_allows(gate: &LegacyForeignGate<'_>) -> bool {
        let p = gate.summary;
        let development = ForeignUnsolicitedLoanTarget::is_development(p.age);
        p.home_reputation <= (gate.borrower_rep as f32 * Self::FOREIGN_REP_CEILING) as i16
            && gate.borrower_rep
                >= (p.home_reputation.max(0) as f32 * Self::FOREIGN_REP_FLOOR) as u16
            && gate
                .depth
                .has_room_for(p.position_group, p.skill_ability, development)
            && gate.depth.would_get_loan_minutes(
                p.position_group,
                p.skill_ability,
                development,
                p.club_best_in_group,
            )
            && Self::loan_level_ok(
                gate.borrower_rep,
                p.club_world_reputation.max(0) as u16,
                p.skill_ability,
                p.club_best_in_group,
                development,
                p.seller_ctx.league_reputation,
                gate.borrower_league_rep,
            )
            && Self::foreign_loan_guard_allows(p, gate.borrower.as_ref())
    }
}

/// Eligibility policy for an *unsolicited* domestic loan — a smaller club
/// approaching a bigger one for a player his club has NOT loan-listed. The
/// `Loa` badge is deliberately not required; loan demand should not depend
/// on the parent advertising the player. The realism is in WHO is
/// approachable, decided by the central [`SquadAssetClass`] classifier that
/// the audit and listing paths already share.
/// The level a proposed loan actually asks a player to play at: the standing
/// of the two clubs, the standard of the two competitions, and how close he
/// already is to his parent club's first team.
///
/// Both level realism gates live here so no call site can apply one and skip
/// the other. Reputation alone was never enough: a well-supported
/// second-division club and a top-flight one sit close on reputation while
/// playing in different divisions, so the loan market — which knew only club
/// reputation — happily placed top-flight regulars a tier down.
pub(in crate::transfers::loan) struct LoanDestinationLevel {
    /// Current ability of the player being loaned.
    pub(in crate::transfers::loan) ability: u8,
    /// Best current ability in his position group on the parent's main
    /// roster — the standard he is measured against at his own club.
    pub(in crate::transfers::loan) parent_best_in_group: u8,
    /// Main-team world reputations (0..10000).
    pub(in crate::transfers::loan) parent_rep: u16,
    pub(in crate::transfers::loan) borrower_rep: u16,
    /// Reputations of the competitions the two clubs play in. Zero means
    /// "no league" (a friendly-only side), which suspends the division gate
    /// rather than guessing.
    pub(in crate::transfers::loan) parent_league_rep: u16,
    pub(in crate::transfers::loan) borrower_league_rep: u16,
    /// The loan exists to buy match practice, so a bigger drop is the point
    /// of the move rather than a demotion.
    pub(in crate::transfers::loan) is_development: bool,
}

impl LoanDestinationLevel {
    /// Share of the parent's league level a raw player may drop to …
    const RAW_LEAGUE_FLOOR: f32 = 0.45;
    /// … and the much tighter share a near-ready one may.
    const READY_LEAGUE_FLOOR: f32 = 0.85;
    /// Room the drop gets for being a drop the player NEEDS, at its
    /// widest — a raw youngster playing every week two divisions down is
    /// the point of his move.
    ///
    /// Continuous in the same readiness the floor is: a raw player keeps
    /// the full width, a first-team-ready one gets none of it and is
    /// held to his parent's own level.
    const RAW_LEAGUE_ALLOWANCE: f32 = 0.25;
    /// Club-standing floor a raw non-development loanee is held to …
    const RAW_STANDING_FLOOR: f32 = 0.12;
    /// … and the extra share a fully first-team-ready one adds to it, so a
    /// ready player's destination is a peer rather than "a quarter of my
    /// club", which is what a flat 0.25 made credible.
    const READY_STANDING_SPAN: f32 = 0.63;

    /// Both gates. A destination has to be a credible club **and** a
    /// credible division.
    pub(in crate::transfers::loan) fn is_plausible(&self) -> bool {
        self.clears_club_standing() && self.clears_division()
    }

    /// Club-standing gate — see [`LoanPipeline::loan_reputation_drop_ok`],
    /// which delegates here.
    pub(in crate::transfers::loan) fn clears_club_standing(&self) -> bool {
        if self.parent_rep == 0 {
            return true;
        }
        let very_raw = self.ability.saturating_add(25) <= self.parent_best_in_group;
        let floor = if very_raw && self.is_development {
            // A development youngster genuinely years off the shirt drops
            // without a reputation floor at all — the minutes gate is the
            // realism check for him.
            0.0
        } else if MarketSwitches::loan_guard_off() {
            if very_raw { 0.12 } else { 0.25 }
        } else {
            // Continuous in how ready he already is: the raw floor at one
            // end, a peer-level club at the other. A flat 0.25 made a club
            // a quarter of the parent's standing a credible home for the
            // parent's own first-choice.
            Self::RAW_STANDING_FLOOR + Self::READY_STANDING_SPAN * self.readiness()
        };
        self.borrower_rep as f32 >= self.parent_rep as f32 * floor
    }

    /// Division gate: is the borrower's **competition** a plausible home?
    ///
    /// The floor is continuous in how ready the player already is for his
    /// parent's own first team — measured, like every other loan gate here,
    /// against the parent's best at his position. A youngster far off that
    /// standard drops a long way to play; someone already competing for the
    /// shirt only moves sideways, because a club with a first-team-standard
    /// player plays him, keeps him as cover, or sells him — it does not park
    /// him a division below. A development loan widens the allowance, so the
    /// "drop a level and play every week" pathway keeps working for the
    /// players it is meant for.
    pub(in crate::transfers::loan) fn clears_division(&self) -> bool {
        // Unknown competition on either side — the club-standing gate owns
        // the decision rather than this one guessing.
        if self.parent_league_rep == 0 || self.borrower_league_rep == 0 {
            return true;
        }
        self.borrower_league_rep as f32 >= self.parent_league_rep as f32 * self.division_floor()
    }

    /// How ready this player already is for his parent club's own first
    /// team, 0..1 — measured against the parent's best at his position.
    /// Zero (unknown parent standard) reads as fully raw, which is what
    /// stands both floors down.
    pub(in crate::transfers::loan) fn readiness(&self) -> f32 {
        LevelBand::readiness_of(self.ability, self.parent_best_in_group)
    }

    /// Share of the parent's league level this loan may drop to.
    pub(in crate::transfers::loan) fn division_floor(&self) -> f32 {
        if self.parent_best_in_group == 0 {
            return 0.0;
        }
        let readiness = self.readiness();
        let floor = Self::RAW_LEAGUE_FLOOR
            + (Self::READY_LEAGUE_FLOOR - Self::RAW_LEAGUE_FLOOR) * readiness;
        if MarketSwitches::loan_guard_off() {
            return if self.is_development {
                floor * (1.0 - Self::RAW_LEAGUE_ALLOWANCE)
            } else {
                floor
            };
        }
        // The allowance a drop earns for being a drop the player needs,
        // continuous in readiness rather than switched on by his birth
        // year: full width when he is raw, none of it when he is ready.
        floor * (1.0 - Self::RAW_LEAGUE_ALLOWANCE * (1.0 - readiness))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClubLevelAnchor, PlayerFieldPositionGroup};

    struct Fx;

    impl Fx {
        const FORWARD: PlayerFieldPositionGroup = PlayerFieldPositionGroup::Forward;
        const LA_LIGA: u16 = 9_200;
        const SEGUNDA: u16 = 6_500;

        fn peer_borrower() -> LoanBorrowerProfile {
            LoanBorrowerProfile {
                income: 400_000_000,
                wage_bill: 200_000_000,
                top_earner: 20_000_000,
                wage_headroom: 40_000_000,
                best_in_group: 170,
                anchor: ClubLevelAnchor::for_reputation(0.90),
                world_rep: 9_000,
                league_rep: Self::LA_LIGA,
                reach: 9_000,
            }
        }

        fn yamal(requested: bool) -> LoanAssetGuard {
            LoanAssetGuard::new(
                176,
                19,
                Self::FORWARD,
                ClubLevelAnchor::for_reputation(0.93),
                0,
                1,
                176,
                Self::LA_LIGA,
                189_000_000.0,
                14_600_000,
                requested,
                false,
                8_500,
                0.0,
            )
        }
    }

    /// The three readings the default path replaced with a price, pinned
    /// so the census arm can still say what HEAD refused.
    #[test]
    pub(in crate::transfers::loan) fn the_head_stack_refuses_a_first_choice_outright() {
        assert!(!LegacyLoanGuard::allows(
            &Fx::yamal(false),
            &Fx::peer_borrower()
        ));
        assert!(LegacyLoanGuard::allows(
            &Fx::yamal(true),
            &Fx::peer_borrower()
        ));
    }

    #[test]
    pub(in crate::transfers::loan) fn the_head_stack_refuses_a_peer_level_asset_two_levels_below() {
        let mut borrower = Fx::peer_borrower();
        borrower.anchor = ClubLevelAnchor::for_reputation(0.45);
        borrower.league_rep = Fx::SEGUNDA;
        assert!(!LegacyLoanGuard::allows(&Fx::yamal(true), &borrower));
    }

    #[test]
    pub(in crate::transfers::loan) fn the_head_stack_refuses_a_wage_the_borrower_cannot_carry() {
        let poor = LoanBorrowerProfile {
            income: 20_000_000,
            wage_bill: 12_000_000,
            top_earner: 900_000,
            wage_headroom: 1_500_000,
            anchor: ClubLevelAnchor::for_reputation(0.45),
            world_rep: 3_000,
            league_rep: Fx::SEGUNDA,
            reach: 3_800,
            ..Fx::peer_borrower()
        };
        assert!(!LegacyLoanGuard::allows(&Fx::yamal(true), &poor));
    }
}
