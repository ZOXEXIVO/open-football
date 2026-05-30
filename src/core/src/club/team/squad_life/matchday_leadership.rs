//! Matchday armband resolution.
//!
//! Separate from the persistent club captaincy (`captaincy.rs`, assigned
//! monthly by `CaptaincyAssigner` and stored on `Team.captain_id` /
//! `vice_captain_id`). That hierarchy answers "who is the club captain this
//! month"; this module answers "who actually wears the armband for *this*
//! match", which depends on who was selected.
//!
//! The rules:
//!   1. The persistent club captain wears it **if he started** (is in the
//!      selected XI).
//!   2. Otherwise the persistent vice-captain wears it, if he started.
//!   3. Otherwise the best leader in the selected XI is chosen.
//! The vice slot is filled by the same priority over the remaining XI,
//! excluding whoever took the captaincy.
//!
//! The candidate pool is *always* the on-field set passed in — so a benched
//! captain never wears the armband, and the same function re-resolves the
//! armband when the captain leaves the pitch (substitution / red card): pass
//! the still-active players and the persistent hierarchy, and the vice (if
//! still on) or the best remaining leader is returned.
//!
//! National teams have no persistent hierarchy, so they pass `None, None` and
//! fall straight through to the best-leader rule.

use crate::Player;
use crate::r#match::MatchPlayer;
use std::cmp::Ordering;

/// Resolved armband holders for a single match, by player id. Always point
/// at players in the candidate pool they were resolved from (never a benched
/// or unselected player).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MatchdayLeadership {
    pub captain_id: Option<u32>,
    pub vice_captain_id: Option<u32>,
}

/// Leadership-scored candidate drawn from a selected squad. Holds only the
/// scalars the ranking needs, so the resolver is independent of whether the
/// source was a persisted `Player` (club / national selection, post-match
/// roster) or an in-match `MatchPlayer`.
///
/// Age / contract tenure are deliberately omitted: the matchday resolver runs
/// in squad-build and post-match paths that don't carry a match date, and the
/// dominant signals (leadership and the supporting mental attributes) settle
/// the pick. The persistent `CaptaincyAssigner` already weights age, tenure
/// and loyalty for the season-long armband, which this honours via priority 1
/// and 2 before any best-leader fallback runs.
#[derive(Debug, Clone, Copy)]
pub struct LeadershipCandidate {
    pub id: u32,
    pub leadership: f32,
    pub teamwork: f32,
    pub determination: f32,
    pub composure: f32,
    pub professionalism: f32,
    pub loyalty: f32,
    /// Current reputation on the 0..~10_000 scale.
    pub reputation: f32,
    /// International appearances, used as an experience proxy.
    pub experience: f32,
}

impl LeadershipCandidate {
    /// Build a candidate from a persisted `Player` (club / national selection,
    /// post-match roster lookups).
    pub fn from_player(player: &Player) -> Self {
        LeadershipCandidate {
            id: player.id,
            leadership: player.skills.mental.leadership,
            teamwork: player.skills.mental.teamwork,
            determination: player.skills.mental.determination,
            composure: player.skills.mental.composure,
            professionalism: player.attributes.professionalism,
            loyalty: player.attributes.loyalty,
            reputation: player.player_attributes.current_reputation as f32,
            experience: player.player_attributes.international_apps as f32,
        }
    }

    /// Build a candidate from a selected `MatchPlayer`. Used by the squad
    /// builders, which only carry the match-side player view.
    pub fn from_match_player(player: &MatchPlayer) -> Self {
        LeadershipCandidate {
            id: player.id,
            leadership: player.skills.mental.leadership,
            teamwork: player.skills.mental.teamwork,
            determination: player.skills.mental.determination,
            composure: player.skills.mental.composure,
            professionalism: player.attributes.professionalism,
            loyalty: player.attributes.loyalty,
            reputation: player.player_attributes.current_reputation as f32,
            experience: player.player_attributes.international_apps as f32,
        }
    }

    /// Best-leader score. Leadership dominates, with the supporting mental
    /// attributes, professionalism, loyalty, reputation and experience acting
    /// as tie-breakers. Only consulted when neither persistent captain nor
    /// vice is in the candidate pool.
    fn score(&self) -> f32 {
        self.leadership * 1.5
            + self.teamwork * 0.4
            + self.determination * 0.4
            + self.composure * 0.3
            + self.professionalism * 0.4
            + self.loyalty * 0.25
            + self.reputation / 2500.0
            + (self.experience / 10.0).min(5.0)
    }
}

impl MatchdayLeadership {
    /// Resolve the armband over a set of on-field candidates, honouring the
    /// persistent club hierarchy where it overlaps the candidate pool. See the
    /// module docs for the full rule set. `persistent_captain` /
    /// `persistent_vice` are the club's monthly armband holders (pass `None`
    /// for national teams or any side without a persistent hierarchy).
    pub fn resolve(
        persistent_captain: Option<u32>,
        persistent_vice: Option<u32>,
        candidates: &[LeadershipCandidate],
    ) -> Self {
        if candidates.is_empty() {
            return MatchdayLeadership::default();
        }

        // An id only counts if its holder is actually in the candidate pool.
        let present = |id: Option<u32>| -> Option<u32> {
            id.filter(|wanted| candidates.iter().any(|c| c.id == *wanted))
        };

        let captain = present(persistent_captain)
            .or_else(|| present(persistent_vice))
            .or_else(|| Self::best_leader(candidates, &[]));

        let vice = match captain {
            Some(cap) => present(persistent_vice)
                .filter(|v| *v != cap)
                .or_else(|| present(persistent_captain).filter(|c| *c != cap))
                .or_else(|| Self::best_leader(candidates, &[cap])),
            None => None,
        };

        MatchdayLeadership {
            captain_id: captain,
            vice_captain_id: vice,
        }
    }

    /// Resolve the armband over a selected `MatchPlayer` XI and return the
    /// chosen captain / vice as clones drawn from that XI, so the stored ids
    /// always point at genuinely selected starters. `persistent_*` carry the
    /// club hierarchy (pass `None` for national teams).
    pub fn from_match_squad(
        persistent_captain: Option<u32>,
        persistent_vice: Option<u32>,
        main_squad: &[MatchPlayer],
    ) -> (Option<MatchPlayer>, Option<MatchPlayer>) {
        let candidates: Vec<LeadershipCandidate> = main_squad
            .iter()
            .map(LeadershipCandidate::from_match_player)
            .collect();
        let resolved = Self::resolve(persistent_captain, persistent_vice, &candidates);
        let pick =
            |id: Option<u32>| id.and_then(|id| main_squad.iter().find(|p| p.id == id).cloned());
        (pick(resolved.captain_id), pick(resolved.vice_captain_id))
    }

    /// Highest-scoring candidate whose id is not in `exclude`.
    fn best_leader(candidates: &[LeadershipCandidate], exclude: &[u32]) -> Option<u32> {
        candidates
            .iter()
            .filter(|c| !exclude.contains(&c.id))
            .max_by(|a, b| a.score().partial_cmp(&b.score()).unwrap_or(Ordering::Equal))
            .map(|c| c.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Candidate with the given id and leadership; every other attribute held
    /// at a neutral 10.0 so leadership (and the explicit exclude rules) decide
    /// the ranking.
    fn cand(id: u32, leadership: f32) -> LeadershipCandidate {
        LeadershipCandidate {
            id,
            leadership,
            teamwork: 10.0,
            determination: 10.0,
            composure: 10.0,
            professionalism: 10.0,
            loyalty: 10.0,
            reputation: 1000.0,
            experience: 0.0,
        }
    }

    #[test]
    fn matchday_captain_is_starting_club_captain_when_selected() {
        let xi = vec![cand(1, 8.0), cand(2, 18.0), cand(3, 12.0)];
        // Club captain 1 started — he keeps the armband even though 2 is the
        // stronger raw leader.
        let r = MatchdayLeadership::resolve(Some(1), Some(3), &xi);
        assert_eq!(r.captain_id, Some(1));
        assert_eq!(r.vice_captain_id, Some(3));
    }

    #[test]
    fn matchday_captain_uses_vice_when_club_captain_benched() {
        let xi = vec![cand(2, 9.0), cand(3, 12.0)];
        // Club captain 1 was rotated out; vice 3 started → vice wears it.
        let r = MatchdayLeadership::resolve(Some(1), Some(3), &xi);
        assert_eq!(r.captain_id, Some(3));
        // Vice slot falls to the next best leader on the pitch.
        assert_eq!(r.vice_captain_id, Some(2));
    }

    #[test]
    fn matchday_captain_falls_back_to_best_xi_leader() {
        let xi = vec![cand(5, 11.0), cand(6, 17.0), cand(7, 9.0)];
        // Neither persistent captain nor vice on the pitch.
        let r = MatchdayLeadership::resolve(Some(1), Some(2), &xi);
        assert_eq!(r.captain_id, Some(6)); // strongest leader
        assert_eq!(r.vice_captain_id, Some(5)); // second strongest
    }

    #[test]
    fn matchday_captain_is_never_unselected_player() {
        let xi = vec![cand(5, 11.0), cand(6, 17.0)];
        // Persistent hierarchy points at players who didn't make the XI.
        let r = MatchdayLeadership::resolve(Some(99), Some(98), &xi);
        assert!(xi.iter().any(|c| Some(c.id) == r.captain_id));
        assert!(xi.iter().any(|c| Some(c.id) == r.vice_captain_id));
    }

    #[test]
    fn matchday_vice_excludes_captain() {
        let xi = vec![cand(1, 18.0), cand(2, 16.0)];
        // Degenerate input: same id as captain and vice. Vice must still be a
        // different player.
        let r = MatchdayLeadership::resolve(Some(1), Some(1), &xi);
        assert_eq!(r.captain_id, Some(1));
        assert_ne!(r.vice_captain_id, r.captain_id);
        assert_eq!(r.vice_captain_id, Some(2));
    }

    #[test]
    fn subbed_off_captain_transfers_armband_to_active_vice() {
        // Captain 1 has left the pitch; the still-active set is re-resolved.
        // Vice 2 is on → the armband transfers to him.
        let active = vec![cand(2, 12.0), cand(3, 16.0)];
        let r = MatchdayLeadership::resolve(Some(1), Some(2), &active);
        assert_eq!(r.captain_id, Some(2));
    }

    #[test]
    fn sent_off_captain_transfers_armband_to_best_active_leader() {
        // Captain 1 sent off and vice 2 already subbed off — neither is in the
        // active set, so the best remaining leader takes the armband.
        let active = vec![cand(3, 10.0), cand(4, 15.0), cand(5, 13.0)];
        let r = MatchdayLeadership::resolve(Some(1), Some(2), &active);
        assert_eq!(r.captain_id, Some(4));
    }

    #[test]
    fn match_events_use_actual_matchday_captain_not_team_captain() {
        // Mirrors the post-match path: the persistent club captain (1) was
        // benched, so leadership events must attach to the player who actually
        // started and led — never the stale club captain.
        let started_xi = vec![cand(2, 14.0), cand(3, 11.0)];
        let r = MatchdayLeadership::resolve(Some(1), None, &started_xi);
        assert_ne!(r.captain_id, Some(1));
        assert_eq!(r.captain_id, Some(2));
        assert!(started_xi.iter().any(|c| Some(c.id) == r.captain_id));
    }

    #[test]
    fn national_team_assigns_captain_from_selected_xi() {
        // National teams carry no persistent hierarchy: best leader in the
        // selected XI wears the armband.
        let xi = vec![cand(10, 13.0), cand(11, 17.0), cand(12, 9.0)];
        let r = MatchdayLeadership::resolve(None, None, &xi);
        assert_eq!(r.captain_id, Some(11));
        assert_eq!(r.vice_captain_id, Some(10));
        assert!(xi.iter().any(|c| Some(c.id) == r.captain_id));
    }

    #[test]
    fn bench_player_does_not_count_as_captain_at_kickoff() {
        // A strong leader (id 9) exists at the club but isn't in the XI pool,
        // so he can never be picked.
        let xi = vec![cand(1, 10.0), cand(2, 11.0)];
        let r = MatchdayLeadership::resolve(None, None, &xi);
        assert_ne!(r.captain_id, Some(9));
        assert!(xi.iter().any(|c| Some(c.id) == r.captain_id));
    }

    #[test]
    fn empty_xi_yields_no_armband() {
        let r = MatchdayLeadership::resolve(Some(1), Some(2), &[]);
        assert_eq!(r.captain_id, None);
        assert_eq!(r.vice_captain_id, None);
    }
}
