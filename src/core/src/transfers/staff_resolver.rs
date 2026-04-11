use crate::{Staff, StaffCollection, StaffPosition};

/// Resolved staff roles for the transfer pipeline.
/// Each role has a fallback chain to handle clubs that lack specific staff positions.
pub struct ResolvedStaff<'a> {
    pub squad_evaluator: Option<&'a Staff>,
    pub director_of_football: Option<&'a Staff>,
    pub scouts: Vec<&'a Staff>,
    pub negotiator: Option<&'a Staff>,
}

pub struct StaffResolver;

impl StaffResolver {
    /// Resolve all staff roles needed for the transfer pipeline.
    pub fn resolve(staffs: &StaffCollection) -> ResolvedStaff<'_> {
        ResolvedStaff {
            squad_evaluator: Self::resolve_squad_evaluator(staffs),
            director_of_football: Self::resolve_dof(staffs),
            scouts: Self::resolve_scouts(staffs),
            negotiator: Self::resolve_negotiator(staffs),
        }
    }

    /// Squad Evaluator: incoming_transfers responsibility -> Manager -> AssistantManager
    fn resolve_squad_evaluator(staffs: &StaffCollection) -> Option<&Staff> {
        // Try: staff assigned to find_and_make_offers_first_team
        if let Some(id) = staffs.responsibility.incoming_transfers.find_and_make_offers_first_team {
            if let Some(staff) = staffs.find(id) {
                return Some(staff);
            }
        }

        // Fallback 1: Manager
        if let Some(staff) = staffs.find_by_position(StaffPosition::Manager) {
            return Some(staff);
        }

        // Fallback 2: Assistant Manager
        staffs.find_by_position(StaffPosition::AssistantManager)
    }

    /// DoF: DirectorOfFootball -> Manager -> Head coach
    fn resolve_dof(staffs: &StaffCollection) -> Option<&Staff> {
        if let Some(staff) = staffs.find_by_position(StaffPosition::DirectorOfFootball) {
            return Some(staff);
        }
        if let Some(staff) = staffs.find_by_position(StaffPosition::Manager) {
            return Some(staff);
        }
        staffs.find_by_position(StaffPosition::AssistantManager)
    }

    /// Scouts: all Scout/ChiefScout positions. Falls back to Manager (limited).
    fn resolve_scouts(staffs: &StaffCollection) -> Vec<&Staff> {
        let mut scouts: Vec<&Staff> = staffs
            .iter()
            .filter(|s| {
                matches!(
                    s.contract.as_ref().map(|c| &c.position),
                    Some(StaffPosition::Scout) | Some(StaffPosition::ChiefScout)
                )
            })
            .collect();

        if scouts.is_empty() {
            // No scouts: Manager acts as a limited scout
            if let Some(manager) = staffs.find_by_position(StaffPosition::Manager) {
                scouts.push(manager);
            }
        }

        scouts
    }

    /// Negotiator: finalize_first_team_signings -> DoF -> Manager
    fn resolve_negotiator(staffs: &StaffCollection) -> Option<&Staff> {
        if let Some(id) = staffs.responsibility.incoming_transfers.finalize_first_team_signings {
            if let Some(staff) = staffs.find(id) {
                return Some(staff);
            }
        }
        if let Some(staff) = staffs.find_by_position(StaffPosition::DirectorOfFootball) {
            return Some(staff);
        }
        staffs.find_by_position(StaffPosition::Manager)
    }
}

impl<'a> ResolvedStaff<'a> {
    /// Whether the club has any real scouts (not just a manager fallback).
    pub fn has_dedicated_scouts(&self) -> bool {
        self.scouts.iter().any(|s| {
            matches!(
                s.contract.as_ref().map(|c| &c.position),
                Some(StaffPosition::Scout) | Some(StaffPosition::ChiefScout)
            )
        })
    }

    /// Get the best scout's judging ability for error calculations.
    pub fn best_scout_judging_ability(&self) -> u8 {
        self.scouts
            .iter()
            .map(|s| s.staff_attributes.knowledge.judging_player_ability)
            .max()
            .unwrap_or(5)
    }

    /// Get the best scout's judging potential for error calculations.
    pub fn best_scout_judging_potential(&self) -> u8 {
        self.scouts
            .iter()
            .map(|s| s.staff_attributes.knowledge.judging_player_potential)
            .max()
            .unwrap_or(5)
    }
}
