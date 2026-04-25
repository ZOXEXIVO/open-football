use crate::club::player::behaviour_config::HappinessConfig;
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
    /// Head coach was replaced. Fires per-player: strongly negative for
    /// players who had a close bond with the outgoing manager, mildly
    /// positive for players whose relationship had soured.
    ManagerDeparture,
    /// Called up to the senior national team. Big prestige moment for
    /// younger players, routine for established internationals.
    NationalTeamCallup,
    /// Dropped from the national team squad after previous caps — hurts
    /// pride more than a non-selection would.
    NationalTeamDropped,
    /// Promoted to a prestigious shirt number (1-11, esp. #10 / #7 / #9).
    /// Small ongoing pride boost while the number holds.
    ShirtNumberPromotion,
    /// Had a controversial incident (media or dressing room) — fallout
    /// tied to `controversy` personality attribute.
    ControversyIncident,
}

impl PlayerHappiness {
    pub fn new() -> Self {
        let cfg = HappinessConfig::default();
        PlayerHappiness {
            morale: cfg.default_morale,
            factors: HappinessFactors::default(),
            recent_events: Vec::new(),
            last_salary_negotiation: None,
        }
    }

    pub fn recalculate_morale(&mut self) {
        let cfg = HappinessConfig::default();
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
            .map(|e| e.magnitude * cfg.event_decay(e.days_ago))
            .sum();

        self.morale = cfg.clamp_morale(cfg.default_morale + factor_sum + event_sum);
    }

    pub fn adjust_morale(&mut self, delta: f32) {
        let cfg = HappinessConfig::default();
        self.morale = cfg.clamp_morale(self.morale + delta);
    }

    pub fn decay_events(&mut self) {
        let cfg = HappinessConfig::default();
        for event in &mut self.recent_events {
            event.days_ago += cfg.decay_step_days;
        }
        self.recent_events.retain(|e| e.days_ago <= cfg.event_retention_days);

        if self.recent_events.len() > cfg.recent_events_cap {
            self.recent_events.sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
            self.recent_events.truncate(cfg.recent_events_cap);
        }
    }

    pub fn add_event(&mut self, event_type: HappinessEventType, magnitude: f32) {
        let cfg = HappinessConfig::default();
        self.recent_events.push(HappinessEvent {
            event_type,
            magnitude,
            days_ago: 0,
        });

        if self.recent_events.len() > cfg.recent_events_cap {
            self.recent_events.sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
            self.recent_events.truncate(cfg.recent_events_cap);
        }
    }

    /// Record an event using the catalog's default magnitude. Equivalent
    /// to `add_event(event_type, catalog.magnitude(event_type))`. Use this
    /// for emit sites whose magnitude is the canonical default — single-
    /// magnitude events that don't depend on context.
    pub fn add_event_default(&mut self, event_type: HappinessEventType) {
        let cfg = HappinessConfig::default();
        let magnitude = cfg.catalog.magnitude(event_type.clone());
        self.add_event(event_type, magnitude);
    }

    /// Record an event with a magnitude scaled relative to the catalog
    /// default. `factor=1.0` is equivalent to `add_event_default`. Use
    /// this for emit sites where the magnitude varies by context (severity,
    /// loan damp, etc.) but the *base* should still come from the catalog.
    pub fn add_event_scaled(&mut self, event_type: HappinessEventType, factor: f32) {
        let cfg = HappinessConfig::default();
        let magnitude = cfg.catalog.magnitude(event_type.clone()) * factor;
        self.add_event(event_type, magnitude);
    }

    /// Reset happiness to neutral state (fresh start at a new club)
    pub fn clear(&mut self) {
        let cfg = HappinessConfig::default();
        self.morale = cfg.default_morale;
        self.factors = HappinessFactors::default();
        self.recent_events.clear();
        self.last_salary_negotiation = None;
    }

    /// Backward compatible: morale >= happy_threshold means happy.
    pub fn is_happy(&self) -> bool {
        self.morale >= HappinessConfig::default().happy_threshold
    }

    /// Backward compatible: push a positive event
    pub fn add_positive(&mut self, _item: PositiveHappiness) {
        self.add_event_default(HappinessEventType::GoodTraining);
    }

    /// Backward compatible: push a negative event
    pub fn add_negative(&mut self, _item: NegativeHappiness) {
        self.add_event_default(HappinessEventType::PoorTraining);
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
