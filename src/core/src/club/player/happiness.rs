#[derive(Debug)]
pub struct PlayerHappiness {
    pub morale: f32,
    pub factors: HappinessFactors,
    pub recent_events: Vec<HappinessEvent>,
}

#[derive(Debug, Default)]
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
    ManagerPraise,
    ManagerDiscipline,
    ManagerPlayingTimePromise,
    GoodTraining,
    PoorTraining,
    MatchSelection,
    MatchDropped,
    ContractOffer,
    WageIncrease,
    InjuryReturn,
    SquadStatusChange,
    LackOfPlayingTime,
    LoanListingAccepted,
}

impl PlayerHappiness {
    pub fn new() -> Self {
        PlayerHappiness {
            morale: 50.0,
            factors: HappinessFactors::default(),
            recent_events: Vec::new(),
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
        self.recent_events.retain(|e| e.days_ago <= 60);

        // Keep at most 10 recent events
        if self.recent_events.len() > 10 {
            self.recent_events.sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
            self.recent_events.truncate(10);
        }
    }

    pub fn add_event(&mut self, event_type: HappinessEventType, magnitude: f32) {
        self.recent_events.push(HappinessEvent {
            event_type,
            magnitude,
            days_ago: 0,
        });

        // Keep at most 10
        if self.recent_events.len() > 10 {
            self.recent_events.sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
            self.recent_events.truncate(10);
        }
    }

    /// Backward compatible: morale > 50 means happy
    pub fn is_happy(&self) -> bool {
        self.morale > 50.0
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
#[derive(Debug)]
pub struct PositiveHappiness {
    pub description: String,
}

/// Kept for backward compatibility
#[derive(Debug)]
pub struct NegativeHappiness {
    pub description: String,
}
