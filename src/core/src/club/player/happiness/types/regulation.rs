#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegulationSlotKind {
    HomegrownQuota,
    NonEuQuota,
    SeniorSquadCap,
    YouthSlot,
    InternationalRegistration,
    Other,
}

impl RegulationSlotKind {
    pub fn as_token(&self) -> &'static str {
        match self {
            RegulationSlotKind::HomegrownQuota => "homegrown_quota",
            RegulationSlotKind::NonEuQuota => "non_eu_quota",
            RegulationSlotKind::SeniorSquadCap => "senior_squad_cap",
            RegulationSlotKind::YouthSlot => "youth_slot",
            RegulationSlotKind::InternationalRegistration => "international_registration",
            RegulationSlotKind::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegulationOutcomeKind {
    Omitted,
    Registered,
    DowngradedToReserves,
}

impl RegulationOutcomeKind {
    pub fn as_token(&self) -> &'static str {
        match self {
            RegulationOutcomeKind::Omitted => "omitted",
            RegulationOutcomeKind::Registered => "registered",
            RegulationOutcomeKind::DowngradedToReserves => "downgraded_reserves",
        }
    }
}

/// Regulation / squad-registration explanation payload. Captures which
/// slot type was contested, who took it, and why this player was the
/// odd one out — so the renderer can say "left out of the senior 25
/// to free a non-EU slot for the new signing" rather than "Squad
/// registration omitted".
#[derive(Debug, Clone)]
pub struct RegulationEventContext {
    pub outcome: RegulationOutcomeKind,
    pub slot_kind: RegulationSlotKind,
    pub competition_name_key: Option<String>,
    pub replacement_player_id: Option<u32>,
    /// How many roster slots were available; renderers may say "1 of 4".
    pub slots_total: Option<u8>,
    pub slots_used: Option<u8>,
}

impl RegulationEventContext {
    pub fn new(outcome: RegulationOutcomeKind, slot_kind: RegulationSlotKind) -> Self {
        Self {
            outcome,
            slot_kind,
            competition_name_key: None,
            replacement_player_id: None,
            slots_total: None,
            slots_used: None,
        }
    }

    pub fn with_competition(mut self, key: impl Into<String>) -> Self {
        self.competition_name_key = Some(key.into());
        self
    }
    pub fn with_replacement(mut self, id: u32) -> Self {
        self.replacement_player_id = Some(id);
        self
    }
    pub fn with_slots(mut self, used: u8, total: u8) -> Self {
        self.slots_used = Some(used);
        self.slots_total = Some(total);
        self
    }
}
