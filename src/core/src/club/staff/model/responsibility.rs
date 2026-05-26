#[derive(Debug, Clone, Default)]
pub struct StaffResponsibility {
    pub board: BoardResponsibility,
    pub recruitment: RecruitmentResponsibility,
    pub incoming_transfers: IncomingTransfersResponsibility,
    pub outgoing_transfers: OutgoingTransfersResponsibility,
    pub contract_renewal: ContractRenewalResponsibility,
    pub scouting: ScoutingResponsibility,
    pub training: TrainingResponsibility,
}

#[derive(Debug, Clone, Default)]
pub struct BoardResponsibility {
    pub hire_fire_director: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct RecruitmentResponsibility {
    pub hire_fire_head_of_youth_development: Option<u32>,
    pub hire_fire_chief_scout: Option<u32>,
    pub hire_fire_other_staff: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct IncomingTransfersResponsibility {
    pub find_and_make_offers_first_team: Option<u32>,
    pub finalize_first_team_signings: Option<u32>,

    pub find_and_make_offers_youth_team: Option<u32>,
    pub finalize_youth_team_signings: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct OutgoingTransfersResponsibility {
    pub find_clubs_for_transfers_and_loans_listed_first_team: Option<u32>,
    pub find_clubs_for_transfers_and_loans_listed_youth_team: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct ContractRenewalResponsibility {
    pub handle_first_team_contracts: Option<u32>,
    pub handle_youth_team_contracts: Option<u32>,
    pub handle_director_of_football_contract: Option<u32>,
    pub handle_other_staff_contracts: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct ScoutingResponsibility {
    pub handle_scouting_tasks: Option<u32>,
    pub updates_you_on_players_found: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct TrainingResponsibility {
    pub training_first_team: Option<u32>,
    pub training_youth_team: Option<u32>,
    pub individual_training_first_team: Option<u32>,
    pub individual_training_youth_team: Option<u32>,

    pub match_training_first_team: Option<u32>,
    pub match_training_reserve_team: Option<u32>,
    pub match_training_youth_team: Option<u32>,

    pub youth_development: Option<u32>,
}
