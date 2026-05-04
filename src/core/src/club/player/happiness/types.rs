use crate::club::player::behaviour_config::HappinessConfig;
use chrono::NaiveDate;

#[derive(Debug, Clone)]
pub struct PlayerHappiness {
    pub morale: f32,
    pub factors: HappinessFactors,
    pub recent_events: Vec<HappinessEvent>,
    pub last_salary_negotiation: Option<NaiveDate>,
    /// EMA of "did I start this competitive match?" — updated on every
    /// non-friendly match. Drives the WonStartingPlace / LostStartingPlace
    /// transitions instead of raw season totals so a mid-season turnaround
    /// is felt promptly. Range 0.0..=1.0; 0.5 baseline before first match.
    pub starter_ratio: f32,
    /// Rolling count of recent competitive appearances feeding `starter_ratio`.
    /// Caps at u8::MAX; only the first 5 appearances are required before role
    /// transitions can fire (avoids one good week swinging the verdict).
    pub appearances_tracked: u8,
    /// Sticky flag — true once the player has been recognised as an
    /// established starter, false until they fall back below the bench
    /// threshold. Used to emit one-shot WonStartingPlace / LostStartingPlace
    /// events on the crossing rather than every matchday in range.
    pub is_established_starter: bool,
    /// Competitive appearances since the player's last competitive goal.
    /// Drives `GoalDroughtEnded` / `ScoringDroughtConcern` without an
    /// unbounded per-match history. Saturates at u8::MAX.
    pub apps_since_last_competitive_goal: u8,
    /// Bit-ring of the last 5 competitive appearances: 1 = rating < 6.0,
    /// 0 otherwise. Bit 0 is the most recent appearance. Drives the
    /// "two of the last five" trigger for `MediaPressureMounting` as a
    /// true sliding window — bad games never fall off a block boundary.
    pub recent_low_rating_mask: u8,
    /// Number of appearances currently encoded in `recent_low_rating_mask`,
    /// capped at 5. The trigger only fires once the mask is full, so a
    /// player who has just made his second poor appearance after one good
    /// one doesn't fire on a 1-of-2 ratio.
    pub recent_low_rating_len: u8,
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

    // ── Derived "life in the team" factors ──────────────────────
    /// Does the player understand his role and how he's being used?
    /// Drops on RoleMismatch / repeated tactical-role talks; rises
    /// when the player is in his preferred position with consistent
    /// minutes. Range roughly -8..+5.
    pub role_clarity: f32,
    /// Does the player believe the coaching staff is competent enough
    /// to coach him? Reads coach attribute scores against the player's
    /// own ability. A world-class player at a club with weak coaching
    /// loses respect quickly. Range roughly -8..+6.
    pub coach_credibility: f32,
    /// Where does the player sit in the dressing room — respected,
    /// resented, isolated, or influential? Built from leadership,
    /// reputation, and relations. Range roughly -6..+8.
    pub dressing_room_status: f32,
    /// Cultural / structural fit with the club — facilities, league
    /// level, language, lifestyle, ambition match. Range roughly -8..+6.
    pub club_fit: f32,
    /// Pressure load from fans, media, board expectations relative to
    /// the player's `pressure` personality. Range roughly -8..+3.
    pub pressure_load: f32,
    /// Trust the player has in the manager's word — distinct from the
    /// general manager_relationship. Built from kept-vs-broken
    /// promises and recent broken-promise count. Range roughly -10..+6.
    pub promise_trust: f32,
}

#[derive(Debug, Clone)]
pub struct HappinessEvent {
    pub event_type: HappinessEventType,
    pub magnitude: f32,
    pub days_ago: u16,
    /// Optional teammate / partner involved in this event. Lets the UI
    /// link the event description to a specific player (e.g. who the
    /// player bonded with, who the close friend was, who the mentor was).
    /// `None` for events that don't naturally involve a specific peer.
    pub partner_player_id: Option<u32>,
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
    /// Named the league's Player of the Week — chosen Mondays based on the
    /// previous calendar week's performances. Bigger than POM (which is one
    /// match) and rarer (one player per league per week). Career-visible.
    PlayerOfTheWeek,
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

    // ── Match performance ────────────────────────────────────────
    /// First competitive goal scored for this club. Career milestone —
    /// one-shot per club, lingers in memory for the season.
    FirstClubGoal,
    /// Scored or assisted a goal that decided a tight match. Bigger
    /// than a routine goal, smaller than POM unless paired with it.
    DecisiveGoal,
    /// Came on as a substitute and made a clear positive impact —
    /// scored, assisted, or finished with a high rating off the bench.
    SubstituteImpact,
    /// Defender or goalkeeper kept a clean sheet. Position-gated —
    /// strikers don't care about clean sheets.
    CleanSheetPride,
    /// Finished a match with a costly low rating, often paired with
    /// a goal conceded the player was directly responsible for.
    CostlyMistake,
    /// Sent off (direct red or two yellows). Lingers as embarrassment
    /// plus the suspension fallout.
    RedCardFallout,
    /// Standout performer in a derby win — scorer, assister, POM, or
    /// high-rated display. Reserved for players who carried the win;
    /// ordinary participants get the squad-wide [`DerbyWin`] instead.
    DerbyHero,
    /// Squad-wide moderate positive for being on the winning side of a
    /// derby. Distinct from [`DerbyHero`], which is reserved for the
    /// match's standout performers.
    DerbyWin,
    /// Lost a derby — meaningfully bigger blow than a generic defeat.
    /// Lingers; rivalry loss isn't shaken off in a week.
    DerbyDefeat,

    // ── Team season events ──────────────────────────────────────
    /// Team won a trophy (league, cup, continental). Major career moment.
    TrophyWon,
    /// Team lost a cup final. The flip side of TrophyWon — tournament
    /// runs that ended in heartbreak weigh on a squad.
    CupFinalDefeat,
    /// Team confirmed promotion to a higher division.
    PromotionCelebration,
    /// Team is in the relegation fight late in the season — ambient
    /// dread that builds with the season trajectory.
    RelegationFear,
    /// Team was relegated. Major morale hit, particularly for ambitious
    /// players who'll often want a transfer afterwards.
    Relegated,
    /// Team qualified for European competition — a real boost for
    /// ambitious squads who treat continental football as the floor.
    QualifiedForEurope,

    // ── Role / status ───────────────────────────────────────────
    /// Cemented a place in the starting XI after fighting for it. Fires
    /// once per spell — the moment the manager's trust is established.
    WonStartingPlace,
    /// Lost the starting place to a teammate / new signing. Fires once
    /// per spell on the cusp of being benched, not every dropped match.
    LostStartingPlace,
    /// Awarded the captain's armband. Big prestige and trust signal.
    CaptaincyAwarded,
    /// Stripped of the captain's armband. Wounding — rarely forgotten.
    CaptaincyRemoved,
    /// Young player promoted from academy / development squad to senior
    /// matchday duty for the first time. One-shot career milestone.
    YouthBreakthrough,
    /// Left out of the squad registration list for a competition. Frozen
    /// out of matchday minutes for the duration of that registration window.
    ///
    /// **Reserved.** No emit site exists today — the simulation has
    /// `ForeignPlayerLimits` / `YouthRequirements` placeholders in
    /// `continent::regulations::types`, but no per-club registration list
    /// is enforced and `match_squad` picks XI matchday-by-matchday with
    /// no separate roster gate. When a registration window is added
    /// (continental cup squad lists, foreign-player caps), emit this for
    /// `KeyPlayer` / `FirstTeamRegular` who were expected to be included
    /// but weren't. Do **not** infer it from match-day non-selection —
    /// that's a manager call, not a roster lockout, and a different event.
    SquadRegistrationOmitted,

    // ── Transfer / media ────────────────────────────────────────
    /// Confirmed concrete interest from a club meaningfully bigger than
    /// the current one. Flattery for ambitious players, distraction for
    /// settled ones — replaces the old "manager-encouragement" misuse.
    WantedByBiggerClub,
    /// Bid for the player from another club was rejected by the selling
    /// side. Frustrating for an ambitious player who saw the move coming.
    TransferBidRejected,
    /// A transfer the player was set on collapsed at a late stage —
    /// medical, registration, or club back-out. Lingering bitterness.
    DreamMoveCollapsed,
    /// Praised by the supporters — banners, songs, fan-poll wins.
    FanPraise,
    /// Targeted by fan criticism — bad displays, off-field controversy.
    FanCriticism,
    /// Praised in the media. Reputation-boosting profile pieces, top
    /// pundit ratings.
    MediaPraise,
    /// Targeted by media criticism. Hatchet jobs, tabloid drama.
    MediaCriticism,

    // ── Social / culture ────────────────────────────────────────
    /// A close friend / mentor / linchpin teammate left the club. Players
    /// with strong relationships at the dressing-room core feel this.
    CloseFriendSold,
    /// A compatriot (same primary nationality) joined the club. Big
    /// integration boost for foreign players battling language/culture.
    CompatriotJoined,
    /// Veteran mentor on whom this young player relied departed. Hits
    /// developing players who lost an established guidance figure.
    MentorDeparted,
    /// Made meaningful progress with the local language. Self-reinforcing
    /// integration milestone, only fires for foreign players.
    LanguageProgress,

    // ── Awards / nominations ────────────────────────────────────
    PlayerOfTheMonth,
    YoungPlayerOfTheMonth,
    TeamOfTheWeekSelection,
    TeamOfTheSeasonSelection,
    PlayerOfTheSeason,
    YoungPlayerOfTheSeason,
    LeagueTopScorer,
    LeagueTopAssists,
    LeagueGoldenGlove,
    ContinentalPlayerOfYearNomination,
    ContinentalPlayerOfYear,
    WorldPlayerOfYearNomination,
    WorldPlayerOfYear,

    // ── Real-life football events ────────────────────────────────
    /// First competitive senior appearance for the current club.
    SeniorDebut,
    /// First international appearance after being capped (transitions
    /// from 0 to >0 international apps).
    NationalTeamDebut,
    /// Three or more goals in a non-friendly match.
    HatTrick,
    /// Three or more assists in a non-friendly match.
    AssistHatTrick,
    /// Returned to scoring after a long competitive drought.
    GoalDroughtEnded,
    /// Forward facing a sustained scoring drought.
    ScoringDroughtConcern,
    /// Reached a competitive appearances milestone.
    AppearanceMilestone,
    /// Reached a competitive goals milestone.
    GoalMilestone,
    /// Reached a competitive clean sheets milestone (GK only).
    CleanSheetMilestone,
    /// High-controversy / low-temperament training-ground confrontation.
    TrainingGroundBustUp,
    /// Public apology following a controversy / bust-up.
    PublicApology,
    /// Supporters chanted the player's name in a strong performance.
    FansChantPlayerName,
    /// Sustained negative media coverage at high-profile reputation.
    MediaPressureMounting,
    /// Veteran captain / senior pro stepping up as dressing-room leader.
    LeadershipEmergence,
}

impl PlayerHappiness {
    pub fn new() -> Self {
        let cfg = HappinessConfig::default();
        PlayerHappiness {
            morale: cfg.default_morale,
            factors: HappinessFactors::default(),
            recent_events: Vec::new(),
            last_salary_negotiation: None,
            starter_ratio: 0.5,
            appearances_tracked: 0,
            is_established_starter: false,
            apps_since_last_competitive_goal: 0,
            recent_low_rating_mask: 0,
            recent_low_rating_len: 0,
        }
    }

    pub fn recalculate_morale(&mut self) {
        let cfg = HappinessConfig::default();
        let core_factor_sum = self.factors.playing_time
            + self.factors.salary_satisfaction
            + self.factors.manager_relationship
            + self.factors.ambition_fit
            + self.factors.injury_frustration
            + self.factors.recent_praise
            + self.factors.recent_discipline;

        // Derived "life in the team" factors. Weighted to 0.6× of their
        // raw range so they enrich morale without dominating the core
        // axes the audit already balances around. Each factor is
        // independently clamped at compute time.
        let derived_sum = (self.factors.role_clarity
            + self.factors.coach_credibility
            + self.factors.dressing_room_status
            + self.factors.club_fit
            + self.factors.pressure_load
            + self.factors.promise_trust)
            * 0.6;

        let event_sum: f32 = self
            .recent_events
            .iter()
            .map(|e| e.magnitude * cfg.event_decay(e.days_ago))
            .sum();

        self.morale =
            cfg.clamp_morale(cfg.default_morale + core_factor_sum + derived_sum + event_sum);
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
        self.recent_events
            .retain(|e| e.days_ago <= cfg.event_retention_days);

        if self.recent_events.len() > cfg.recent_events_cap {
            self.recent_events
                .sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
            self.recent_events.truncate(cfg.recent_events_cap);
        }
    }

    pub fn add_event(&mut self, event_type: HappinessEventType, magnitude: f32) {
        self.add_event_with_partner(event_type, magnitude, None);
    }

    /// Same as `add_event` but tags the event with a teammate / partner
    /// player id so the UI can render an inline link. Use this for events
    /// that naturally involve a specific peer (TeammateBonding,
    /// ConflictWithTeammate, CloseFriendSold, MentorDeparted,
    /// CompatriotJoined). The partner id has no effect on morale — it's
    /// purely informational.
    ///
    /// Enforcement: events listed in `requires_partner_id` MUST be emitted
    /// with a `Some(_)` partner id. Calls that pass `None` for those types
    /// are dropped here — the event would otherwise reach the UI as
    /// orphaned text ("bonded with a teammate" — which one?), be filtered
    /// out at render time, and waste a slot in `recent_events`. Failing
    /// silently at the source forces the emit-site to either supply the
    /// partner id or pick a different event type.
    pub fn add_event_with_partner(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: Option<u32>,
    ) {
        if requires_partner_id(&event_type) && partner_player_id.is_none() {
            debug_assert!(
                false,
                "{:?} requires a partner_player_id; use add_event_with_partner",
                event_type
            );
            return;
        }
        let cfg = HappinessConfig::default();
        self.recent_events.push(HappinessEvent {
            event_type,
            magnitude,
            days_ago: 0,
            partner_player_id,
        });

        if self.recent_events.len() > cfg.recent_events_cap {
            self.recent_events
                .sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
            self.recent_events.truncate(cfg.recent_events_cap);
        }
    }

    /// True if an event of `event_type` was recorded within the last
    /// `days` days (inclusive). Cheap O(n) scan — `recent_events` is
    /// capped, so this is bounded.
    pub fn has_recent_event(&self, event_type: &HappinessEventType, days: u16) -> bool {
        self.recent_events
            .iter()
            .any(|e| e.event_type == *event_type && e.days_ago <= days)
    }

    /// Same as [`Self::has_recent_event`] but filtered to events tagged
    /// with the given partner. Use this for per-pair cooldowns — e.g.
    /// to avoid emitting "ConflictWithTeammate (vs player X)" every
    /// week when the underlying friction is constant.
    pub fn has_recent_event_with_partner(
        &self,
        event_type: &HappinessEventType,
        partner_player_id: u32,
        days: u16,
    ) -> bool {
        self.recent_events.iter().any(|e| {
            e.event_type == *event_type
                && e.partner_player_id == Some(partner_player_id)
                && e.days_ago <= days
        })
    }

    /// Add an event only if no event of this type was emitted in the
    /// last `cooldown_days`. Returns `true` if the event was recorded.
    /// Centralised cooldown gate so emit sites don't reimplement the
    /// "did we already fire this recently" pattern (the audit found
    /// inline copies in `process_contract_jealousy` and
    /// `process_periodic_wage_envy`).
    pub fn add_event_with_cooldown(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        cooldown_days: u16,
    ) -> bool {
        if self.has_recent_event(&event_type, cooldown_days) {
            return false;
        }
        self.add_event(event_type, magnitude);
        true
    }

    /// Cooldown-gated counterpart of `add_event_with_partner`. Use this
    /// for partner-required events that also want a cooldown — emitting
    /// via the partner-less variant would be silently dropped by the
    /// `requires_partner_id` guard.
    pub fn add_event_with_partner_and_cooldown(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: Option<u32>,
        cooldown_days: u16,
    ) -> bool {
        if self.has_recent_event(&event_type, cooldown_days) {
            return false;
        }
        self.add_event_with_partner(event_type, magnitude, partner_player_id);
        true
    }

    /// Catalog-default counterpart of [`add_event_with_cooldown`].
    pub fn add_event_default_with_cooldown(
        &mut self,
        event_type: HappinessEventType,
        cooldown_days: u16,
    ) -> bool {
        if self.has_recent_event(&event_type, cooldown_days) {
            return false;
        }
        self.add_event_default(event_type);
        true
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

    /// Reset happiness to neutral state (fresh start at a new club).
    /// `HappinessFactors::default()` zeroes all six derived factors —
    /// they're recomputed on the first weekly tick at the new club.
    pub fn clear(&mut self) {
        let cfg = HappinessConfig::default();
        self.morale = cfg.default_morale;
        self.factors = HappinessFactors::default();
        self.recent_events.clear();
        self.last_salary_negotiation = None;
        self.starter_ratio = 0.5;
        self.appearances_tracked = 0;
        self.is_established_starter = false;
        self.apps_since_last_competitive_goal = 0;
        self.recent_low_rating_mask = 0;
        self.recent_low_rating_len = 0;
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

/// Event types that name a specific teammate and therefore must carry a
/// `partner_player_id`. Mirrors the web layer's `is_partner_required`
/// gate — kept here as the source of truth so emit-side and render-side
/// agree. Adding a new partner-style event type means listing it both
/// here (to enforce at emit) and in the web filter (to render the link).
fn requires_partner_id(event_type: &HappinessEventType) -> bool {
    matches!(
        event_type,
        HappinessEventType::TeammateBonding
            | HappinessEventType::ConflictWithTeammate
            | HappinessEventType::CloseFriendSold
            | HappinessEventType::MentorDeparted
            | HappinessEventType::CompatriotJoined
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_blocks_duplicate_event() {
        let mut h = PlayerHappiness::new();
        let added = h.add_event_with_cooldown(HappinessEventType::DerbyHero, 5.0, 14);
        assert!(added, "first emit should land");
        let second = h.add_event_with_cooldown(HappinessEventType::DerbyHero, 5.0, 14);
        assert!(!second, "second emit inside cooldown should be skipped");
        assert_eq!(
            h.recent_events
                .iter()
                .filter(|e| e.event_type == HappinessEventType::DerbyHero)
                .count(),
            1
        );
    }

    #[test]
    fn cooldown_lapses_after_age() {
        let mut h = PlayerHappiness::new();
        h.add_event_with_cooldown(HappinessEventType::SettledIntoSquad, 2.0, 14);
        // Simulate time passing — bump days_ago past the cooldown window.
        h.recent_events[0].days_ago = 21;
        let added = h.add_event_with_cooldown(HappinessEventType::SettledIntoSquad, 2.0, 14);
        assert!(added, "emit should resume once cooldown has elapsed");
    }

    #[test]
    fn has_recent_event_distinguishes_event_types() {
        let mut h = PlayerHappiness::new();
        h.add_event_default(HappinessEventType::DerbyHero);
        assert!(h.has_recent_event(&HappinessEventType::DerbyHero, 30));
        assert!(!h.has_recent_event(&HappinessEventType::DerbyDefeat, 30));
    }
}
