use chrono::NaiveDate;

#[derive(Debug, Clone)]
pub struct PlayerHappiness {
    pub morale: f32,
    pub factors: HappinessFactors,
    pub recent_events: Vec<HappinessEvent>,
    pub last_salary_negotiation: Option<NaiveDate>,
}

#[derive(Debug, Clone, Default)]
pub struct HappinessFactors {
    pub playing_time: f32,
    pub salary_satisfaction: f32,
    pub manager_relationship: f32,
    pub ambition_fit: f32,
    pub injury_frustration: f32,
    pub recent_praise: f32,
    pub recent_discipline: f32,
}

#[derive(Debug, Clone)]
pub struct HappinessEvent {
    pub event_type: HappinessEventType,
    pub magnitude: f32,
    pub days_ago: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HappinessEventType {
    // Manager interactions
    ManagerPraise,
    ManagerDiscipline,
    ManagerPlayingTimePromise,
    ManagerCriticism,
    ManagerEncouragement,
    ManagerTacticalInstruction,
    // Training
    GoodTraining,
    PoorTraining,
    // Match selection
    MatchSelection,
    MatchDropped,
    // Contract & transfers
    ContractOffer,
    ContractRenewal,
    SquadStatusChange,
    LackOfPlayingTime,
    LoanListingAccepted,
    // Injury
    InjuryReturn,
    // Match performance
    PlayerOfTheMatch,
    // Team/squad relationship
    TeammateBonding,
    ConflictWithTeammate,
    DressingRoomSpeech,
    SettledIntoSquad,
    FeelingIsolated,
    /// Teammate signed a meaningfully bigger deal and this player noticed —
    /// drags salary_satisfaction. Typically only fires if the friendship
    /// with the newly-signed teammate is low.
    SalaryGapNoticed,
    /// Manager kept a concrete promise (e.g. more playing time).
    PromiseKept,
    /// Manager broke a concrete promise. Big morale hit, erodes trust.
    PromiseBroken,
    /// Fresh transfer landed the player at a club whose reputation sits well
    /// below what his ambition expects. Lingers while the gap exists.
    AmbitionShock,
    /// New contract is dramatically worse than the pre-transfer salary —
    /// e.g. Messi moving to a Maltese club on a 1/100th deal.
    SalaryShock,
    /// Team's primary formation has no slot for the player's preferred
    /// position. Degrades ambition_fit until a compatible role opens.
    RoleMismatch,
    /// Signed for a club well above the player's expectations — an
    /// unambiguous step up (small-club talent joining Barça / Madrid).
    DreamMove,
    /// New contract pays materially more than the previous deal — the
    /// positive counterpart to SalaryShock.
    SalaryBoost,
    /// Joined a genuinely elite club (top-tier reputation). Fires only
    /// when the move is also a step up relative to the player's own
    /// reputation, to avoid stacking with DreamMove at mid-table moves.
    JoiningElite,
    /// Club bought the player out of his contract — a mild blow to pride
    /// softened by the severance payout. Emitted on mutual termination.
    ContractTerminated,
}

impl PlayerHappiness {
    pub fn new() -> Self {
        PlayerHappiness {
            morale: 50.0,
            factors: HappinessFactors::default(),
            recent_events: Vec::new(),
            last_salary_negotiation: None,
        }
    }

    pub fn recalculate_morale(&mut self) {
        let factor_sum = self.factors.playing_time
            + self.factors.salary_satisfaction
            + self.factors.manager_relationship
            + self.factors.ambition_fit
            + self.factors.injury_frustration
            + self.factors.recent_praise
            + self.factors.recent_discipline;

        let event_sum: f32 = self
            .recent_events
            .iter()
            .map(|e| {
                let decay = 1.0 - (e.days_ago as f32 / 60.0).min(1.0);
                e.magnitude * decay
            })
            .sum();

        self.morale = (50.0 + factor_sum + event_sum).clamp(0.0, 100.0);
    }

    pub fn adjust_morale(&mut self, delta: f32) {
        self.morale = (self.morale + delta).clamp(0.0, 100.0);
    }

    pub fn decay_events(&mut self) {
        for event in &mut self.recent_events {
            event.days_ago += 7;
        }
        // Keep events for up to 365 days (1 season of history)
        self.recent_events.retain(|e| e.days_ago <= 365);

        if self.recent_events.len() > 100 {
            self.recent_events.sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
            self.recent_events.truncate(100);
        }
    }

    pub fn add_event(&mut self, event_type: HappinessEventType, magnitude: f32) {
        self.recent_events.push(HappinessEvent {
            event_type,
            magnitude,
            days_ago: 0,
        });

        if self.recent_events.len() > 100 {
            self.recent_events.sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
            self.recent_events.truncate(100);
        }
    }

    /// Reset happiness to neutral state (fresh start at a new club)
    pub fn clear(&mut self) {
        self.morale = 50.0;
        self.factors = HappinessFactors::default();
        self.recent_events.clear();
        self.last_salary_negotiation = None;
    }

    /// Backward compatible: morale >= 40 means happy (not visibly unhappy)
    pub fn is_happy(&self) -> bool {
        self.morale >= 40.0
    }

    /// Backward compatible: push a positive event
    pub fn add_positive(&mut self, _item: PositiveHappiness) {
        self.add_event(HappinessEventType::GoodTraining, 2.0);
    }

    /// Backward compatible: push a negative event
    pub fn add_negative(&mut self, _item: NegativeHappiness) {
        self.add_event(HappinessEventType::PoorTraining, -2.0);
    }
}

/// Kept for backward compatibility
#[derive(Debug, Clone)]
pub struct PositiveHappiness {
    pub description: String,
}

/// Kept for backward compatibility
#[derive(Debug, Clone)]
pub struct NegativeHappiness {
    pub description: String,
}
