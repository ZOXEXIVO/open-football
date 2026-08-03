use crate::{Club, Person, PlayerFieldPositionGroup, TeamType};

/// Buy-side mirror of the club's own surplus machinery.
///
/// The country listing sweep sells players who sit "well below squad
/// average" ([`crate::ReputationLevel::surplus_quality_gap`]) and the
/// weekly rebalance demotes/loan-lists players ranked beyond the
/// main-team depth cap ([`PlayerFieldPositionGroup::main_depth_cap`]).
/// A buying decision that ignores those two rules produces the churn
/// this snapshot exists to stop: the club signs a player, the signing
/// plan's patience runs out, and the very same maths lists him for
/// sale. Recruitment therefore projects the candidate into the squad
/// FIRST — with the scout's assessed numbers, never the hidden CA —
/// and walks away from anyone who would land as surplus on arrival.
///
/// Precomputed per (club, position group); `Copy` so the pure
/// recruitment evaluators can carry it in their context structs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SquadFitSnapshot {
    /// Main-squad average current ability — the listing sweep's yardstick.
    pub squad_avg_ability: u8,
    /// Wealth-scaled surplus gap under that average (see
    /// [`crate::ReputationLevel::surplus_quality_gap`]).
    pub quality_gap: i16,
    /// Main-team headcount at the candidate's position group.
    pub group_size: usize,
    /// Weekly-rebalance depth cap for the group.
    pub group_cap: usize,
    /// Ability of the cap-th best player in the group (0 when the group
    /// is smaller than the cap). A candidate strictly below this bar in
    /// a full group would rank outside the cap and be demoted within
    /// weeks of arriving.
    pub group_cap_bar: u8,
    /// The club already holds its fill of development-band prospects at
    /// this position group (counted across EVERY team, not just the
    /// main roster). When true, the promising-youth exemption below no
    /// longer applies: a high-ceiling teenager still lands as surplus
    /// when the club's prospect desk at his position is full. Keepers
    /// bind first (one plays; two groomed young keepers is already a
    /// generous succession pool) — this is what stops the elite-club
    /// loop of re-buying a teenage keeper every window while the last
    /// three sit in a league-less U20.
    pub prospect_desk_full: bool,
}

impl SquadFitSnapshot {
    /// Development-band age ceiling — mirrors the DevelopmentSigning
    /// request band (16, 21).
    const PROSPECT_BAND_AGE_MAX: u8 = 21;

    /// How many development-band prospects a club plausibly grooms per
    /// group before one more is warehouse stock, not a pipeline. Outfield
    /// numbers are deliberately roomy (big clubs really do hold six-plus
    /// young defenders); the keeper number is the binding one.
    fn prospect_stock_allowance(group: PlayerFieldPositionGroup) -> usize {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 2,
            PlayerFieldPositionGroup::Defender => 6,
            PlayerFieldPositionGroup::Midfielder => 6,
            PlayerFieldPositionGroup::Forward => 4,
        }
    }

    pub fn build(club: &Club, group: PlayerFieldPositionGroup, date: chrono::NaiveDate) -> Self {
        let main = club.teams.iter().find(|t| t.team_type == TeamType::Main);
        let (squad_avg_ability, quality_gap) = match main {
            Some(team) => (
                team.players.current_ability_avg(),
                team.reputation.level().surplus_quality_gap(),
            ),
            None => (0, 0),
        };

        let group_cap = group.main_depth_cap();
        let mut group_abilities: Vec<u8> = main
            .map(|team| {
                team.players
                    .iter()
                    .filter(|p| p.position().position_group() == group)
                    .map(|p| p.player_attributes.current_ability)
                    .collect()
            })
            .unwrap_or_default();
        group_abilities.sort_unstable_by(|a, b| b.cmp(a));

        let group_size = group_abilities.len();
        let group_cap_bar = if group_size >= group_cap {
            group_abilities
                .get(group_cap.saturating_sub(1))
                .copied()
                .unwrap_or(0)
        } else {
            0
        };

        // Development-band bodies at this group anywhere in the club —
        // main, reserve or youth rosters alike. Loan-ins belong to another
        // club and don't stock the desk.
        let prospect_stock = club
            .teams
            .teams
            .iter()
            .flat_map(|t| t.players.players.iter())
            .filter(|p| {
                !p.is_on_loan()
                    && p.position().position_group() == group
                    && p.age(date) <= Self::PROSPECT_BAND_AGE_MAX
            })
            .count();
        let prospect_desk_full = prospect_stock >= Self::prospect_stock_allowance(group);

        SquadFitSnapshot {
            squad_avg_ability,
            quality_gap,
            group_size,
            group_cap,
            group_cap_bar,
            prospect_desk_full,
        }
    }

    /// Neutral snapshot for callers (and tests) that opt out of the fit
    /// gate — every field zeroed, so [`Self::would_be_surplus`] never fires.
    pub fn disabled() -> Self {
        SquadFitSnapshot {
            squad_avg_ability: 0,
            quality_gap: 0,
            group_size: 0,
            group_cap: usize::MAX,
            group_cap_bar: 0,
            prospect_desk_full: false,
        }
    }

    /// Would this candidate — at his scout-ASSESSED ability/potential —
    /// classify as surplus the day he arrives?
    ///
    /// Mirrors the two automatic surplus rules exactly:
    /// - the listing sweep's "well below squad average" trigger (with the
    ///   same promising-youth exemption: `age <= 23 && pa > ca + 10`);
    /// - the weekly rebalance's depth-cap demotion — a full group where
    ///   the candidate would rank strictly outside the cap. Displacing an
    ///   incumbent (equal or better rank) is normal squad upgrading and
    ///   stays allowed; the incumbent becomes the surplus body instead.
    pub fn would_be_surplus(&self, assessed_ability: u8, assessed_potential: u8, age: u8) -> bool {
        // A high-ceiling youngster is exempt from both surplus rules —
        // but only while the club still has room on its prospect desk at
        // his position. Once the desk is stocked, one more teenager is
        // warehouse inventory: he arrives, the rebalance parks him in a
        // youth squad, and the club is straight back in the market.
        let promising_youth = age <= 23 && assessed_potential > assessed_ability.saturating_add(10);
        if promising_youth && !self.prospect_desk_full {
            return false;
        }

        let well_below_avg = self.squad_avg_ability > 15
            && (assessed_ability as i16) < self.squad_avg_ability as i16 - self.quality_gap;

        let outside_depth_cap =
            self.group_size >= self.group_cap && assessed_ability < self.group_cap_bar;

        well_below_avg || outside_depth_cap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(avg: u8, gap: i16, size: usize, cap: usize, bar: u8) -> SquadFitSnapshot {
        SquadFitSnapshot {
            squad_avg_ability: avg,
            quality_gap: gap,
            group_size: size,
            group_cap: cap,
            group_cap_bar: bar,
            prospect_desk_full: false,
        }
    }

    #[test]
    fn well_below_average_candidate_is_surplus_on_arrival() {
        let fit = snapshot(120, 15, 5, 9, 0);
        assert!(fit.would_be_surplus(100, 105, 27));
        assert!(!fit.would_be_surplus(110, 112, 27));
    }

    #[test]
    fn promising_youth_is_exempt_from_the_average_rule() {
        let fit = snapshot(120, 15, 5, 9, 0);
        assert!(!fit.would_be_surplus(100, 130, 20));
        // Same numbers past the youth cutoff → surplus.
        assert!(fit.would_be_surplus(100, 130, 24));
    }

    #[test]
    fn full_prospect_desk_ends_the_promising_youth_exemption() {
        let mut fit = snapshot(120, 15, 5, 9, 0);
        fit.prospect_desk_full = true;
        // Identical promising-youth profile as above, but the club already
        // grooms its fill at this position → surplus on arrival after all.
        assert!(fit.would_be_surplus(100, 130, 20));
        // A youngster who clears the squad bars on his own numbers is
        // still fine — the desk gate only removes the exemption, it never
        // vetoes on its own.
        assert!(!fit.would_be_surplus(118, 140, 19));
    }

    #[test]
    fn full_group_rejects_candidates_ranked_outside_the_cap() {
        // Group already at cap; the cap-th best sits at 118.
        let fit = snapshot(110, 15, 9, 9, 118);
        // Below the bar → the weekly rebalance would demote him.
        assert!(fit.would_be_surplus(112, 115, 27));
        // At/above the bar → he displaces an incumbent instead.
        assert!(!fit.would_be_surplus(118, 120, 27));
    }

    /// The ageing back-up keeper a big club is offered off another big
    /// club's transfer list. Both surplus rules catch him independently,
    /// which is the point: whichever way the buying club's squad happens
    /// to be shaped, its own maths already says he would not last — so
    /// recruitment must not sign him and then discover that a week later.
    #[test]
    fn a_veteran_reserve_keeper_reads_as_surplus_at_a_stronger_club() {
        // Keeper line already at its cap of three, third-choice on 118.
        let full_line = snapshot(135, 15, 3, 3, 118);
        assert!(full_line.would_be_surplus(115, 115, 36));

        // Same club a keeper short: the depth rule can't fire, but he is
        // still well below what this squad fields.
        let short_line = snapshot(135, 15, 2, 3, 0);
        assert!(short_line.would_be_surplus(115, 115, 36));

        // A keeper who would actually be first choice there is a normal
        // signing — the gate rejects churn, not business.
        assert!(!short_line.would_be_surplus(138, 138, 29));
    }

    #[test]
    fn short_group_never_trips_the_depth_rule() {
        let fit = snapshot(110, 15, 4, 9, 0);
        assert!(!fit.would_be_surplus(104, 106, 27));
    }

    #[test]
    fn disabled_snapshot_never_fires() {
        let fit = SquadFitSnapshot::disabled();
        assert!(!fit.would_be_surplus(1, 1, 35));
    }
}
