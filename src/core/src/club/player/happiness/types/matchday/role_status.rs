use super::SelectionRole;
use crate::club::player::contract::PlayerSquadStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleStatusKind {
    RoleClarifiedByManager,
    RoleUnclear,
    DepthChartPressure,
    DirectRivalPreferred,
    TacticalRoleChanged,
    BenchedForBalance,
    RestedForWorkload,
    SquadStatusUpgrade,
    SquadStatusDowngrade,
    NoNaturalRoleInFormation,
    EstablishedStarter,
    SlippedOutOfStartingXI,
}

impl RoleStatusKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            RoleStatusKind::RoleClarifiedByManager => "role_status_kind_role_clarified",
            RoleStatusKind::RoleUnclear => "role_status_kind_role_unclear",
            RoleStatusKind::DepthChartPressure => "role_status_kind_depth_chart_pressure",
            RoleStatusKind::DirectRivalPreferred => "role_status_kind_direct_rival_preferred",
            RoleStatusKind::TacticalRoleChanged => "role_status_kind_tactical_role_changed",
            RoleStatusKind::BenchedForBalance => "role_status_kind_benched_for_balance",
            RoleStatusKind::RestedForWorkload => "role_status_kind_rested_for_workload",
            RoleStatusKind::SquadStatusUpgrade => "role_status_kind_squad_status_upgrade",
            RoleStatusKind::SquadStatusDowngrade => "role_status_kind_squad_status_downgrade",
            RoleStatusKind::NoNaturalRoleInFormation => "role_status_kind_no_natural_role",
            RoleStatusKind::EstablishedStarter => "role_status_kind_established_starter",
            RoleStatusKind::SlippedOutOfStartingXI => "role_status_kind_slipped_out_xi",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoleStatusEventContext {
    pub kind: RoleStatusKind,
    pub previous_status: Option<PlayerSquadStatus>,
    pub new_status: Option<PlayerSquadStatus>,
    pub formation_slot: Option<SelectionRole>,
    pub starter_ratio: Option<f32>,
    pub repeated_omissions: u8,
    pub direct_rival_id: Option<u32>,
}

impl RoleStatusEventContext {
    pub fn new(kind: RoleStatusKind) -> Self {
        Self {
            kind,
            previous_status: None,
            new_status: None,
            formation_slot: None,
            starter_ratio: None,
            repeated_omissions: 0,
            direct_rival_id: None,
        }
    }

    pub fn with_status_change(mut self, prev: PlayerSquadStatus, new: PlayerSquadStatus) -> Self {
        self.previous_status = Some(prev);
        self.new_status = Some(new);
        self
    }
    pub fn with_formation_slot(mut self, slot: SelectionRole) -> Self {
        self.formation_slot = Some(slot);
        self
    }
    pub fn with_starter_ratio(mut self, ratio: f32) -> Self {
        self.starter_ratio = Some(ratio);
        self
    }
    pub fn with_repeated_omissions(mut self, n: u8) -> Self {
        self.repeated_omissions = n;
        self
    }
    pub fn with_direct_rival(mut self, id: u32) -> Self {
        self.direct_rival_id = Some(id);
        self
    }
}
