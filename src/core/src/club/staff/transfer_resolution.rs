//! Transfer-pipeline staff resolution.
//!
//! Sits next to the data it queries: the per-role finders (squad
//! evaluator, director of football, scouts, negotiator) are methods on
//! [`StaffCollection`] rather than free functions on a separate service
//! struct. The aggregate result type, [`ResolvedStaff`], lives here too
//! so the whole "who handles transfers at this club?" surface is one
//! cohesive unit.
//!
//! Each finder encodes a fallback chain — clubs without a dedicated
//! Director of Football fall back to the Manager, clubs without scouts
//! fall back to the Manager acting as a limited scout, etc. The chains
//! mirror real-world staff cover: a small club's manager genuinely does
//! everything.

use super::staff::StaffCollection;
use crate::club::staff::contract::StaffPosition;
use crate::club::staff::staff::Staff;

/// Snapshot of every staff member needed to drive the transfer pipeline
/// for a single club. Borrowed for the lifetime of the underlying
/// `StaffCollection`, so callers don't pay clone costs and the resolved
/// roles stay in sync with the source data for the duration of use.
pub struct ResolvedStaff<'a> {
    pub squad_evaluator: Option<&'a Staff>,
    pub director_of_football: Option<&'a Staff>,
    pub scouts: Vec<&'a Staff>,
    pub negotiator: Option<&'a Staff>,
}

impl StaffCollection {
    /// Resolve every staff role the transfer pipeline needs in one pass.
    /// Equivalent to calling `find_squad_evaluator`, `find_director_of_football`,
    /// `find_scouts`, and `find_negotiator` and packing the results into a
    /// [`ResolvedStaff`].
    pub fn resolve_for_transfers(&self) -> ResolvedStaff<'_> {
        ResolvedStaff {
            squad_evaluator: self.find_squad_evaluator(),
            director_of_football: self.find_director_of_football(),
            scouts: self.find_scouts(),
            negotiator: self.find_negotiator(),
        }
    }

    /// Squad evaluator — who decides whether to make an offer for an
    /// incoming target. Fallback chain:
    ///   1. Staff explicitly assigned to `find_and_make_offers_first_team`
    ///   2. Manager
    ///   3. Assistant Manager
    pub fn find_squad_evaluator(&self) -> Option<&Staff> {
        if let Some(id) = self
            .responsibility
            .incoming_transfers
            .find_and_make_offers_first_team
        {
            if let Some(staff) = self.find(id) {
                return Some(staff);
            }
        }
        self.find_by_position(StaffPosition::Manager)
            .or_else(|| self.find_by_position(StaffPosition::AssistantManager))
    }

    /// Director of Football. Fallback: DoF → Manager → Assistant Manager.
    pub fn find_director_of_football(&self) -> Option<&Staff> {
        self.find_by_position(StaffPosition::DirectorOfFootball)
            .or_else(|| self.find_by_position(StaffPosition::Manager))
            .or_else(|| self.find_by_position(StaffPosition::AssistantManager))
    }

    /// Every Scout / ChiefScout on the books. Returns a single-element
    /// list containing the Manager when the club has no dedicated scouts
    /// — used by the scouting pipeline as a degraded-mode fallback.
    pub fn find_scouts(&self) -> Vec<&Staff> {
        let scouts: Vec<&Staff> = self
            .iter()
            .filter(|s| {
                matches!(
                    s.contract.as_ref().map(|c| &c.position),
                    Some(StaffPosition::Scout) | Some(StaffPosition::ChiefScout)
                )
            })
            .collect();
        if !scouts.is_empty() {
            return scouts;
        }
        match self.find_by_position(StaffPosition::Manager) {
            Some(manager) => vec![manager],
            None => Vec::new(),
        }
    }

    /// Negotiator who finalizes an incoming signing. Fallback chain:
    ///   1. Staff explicitly assigned to `finalize_first_team_signings`
    ///   2. Director of Football
    ///   3. Manager
    pub fn find_negotiator(&self) -> Option<&Staff> {
        if let Some(id) = self
            .responsibility
            .incoming_transfers
            .finalize_first_team_signings
        {
            if let Some(staff) = self.find(id) {
                return Some(staff);
            }
        }
        self.find_by_position(StaffPosition::DirectorOfFootball)
            .or_else(|| self.find_by_position(StaffPosition::Manager))
    }
}

impl<'a> ResolvedStaff<'a> {
    /// Whether the club has any real scouts — a `find_scouts` result
    /// containing only the Manager fallback returns false.
    pub fn has_dedicated_scouts(&self) -> bool {
        self.scouts.iter().any(|s| {
            matches!(
                s.contract.as_ref().map(|c| &c.position),
                Some(StaffPosition::Scout) | Some(StaffPosition::ChiefScout)
            )
        })
    }

    /// Best scout's `judging_player_ability` for error calculations.
    /// Falls back to a baseline 5 when the scout list is empty.
    pub fn best_scout_judging_ability(&self) -> u8 {
        self.scouts
            .iter()
            .map(|s| s.staff_attributes.knowledge.judging_player_ability)
            .max()
            .unwrap_or(5)
    }

    /// Best scout's `judging_player_potential` for error calculations.
    /// Falls back to a baseline 5 when the scout list is empty.
    pub fn best_scout_judging_potential(&self) -> u8 {
        self.scouts
            .iter()
            .map(|s| s.staff_attributes.knowledge.judging_player_potential)
            .max()
            .unwrap_or(5)
    }
}
