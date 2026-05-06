//! Career-milestone events: youth breakthrough + team-level season
//! outcomes (trophies, relegation, promotion, continental qualification).
//!
//! Magnitude is the catalog default scaled by season participation
//! (helpers in [`super::role`]) and a personality blend chosen for the
//! event type. Cooldown gates prevent the same event firing twice when
//! emit logic stutters (e.g. season-end ticking on consecutive days).

use chrono::NaiveDate;

use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::player::Player;
use crate::{
    HappinessEventCause, HappinessEventContext, HappinessEventScope, HappinessEventSeverity,
    HappinessEventType, Person, RecognitionEventContext, RecognitionEventKind,
    RegulationEventContext, SeasonOutcomeContext,
};

impl Player {
    /// React to a promotion from a youth/reserve team to the senior side.
    /// Career milestone — emit once per spell with a long cooldown so a
    /// player who oscillates between reserves and main doesn't get a fresh
    /// "breakthrough" each bounce. Late bloomers (>21) get a softened
    /// magnitude — the moment is real but they expected it eventually.
    pub fn on_youth_breakthrough(&mut self, now: NaiveDate) {
        let age = self.age(now);
        // Skip players already past the breakthrough window — a 25-year-old
        // moving from reserve to main is a squad-depth call, not a debut.
        if age >= 26 {
            return;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.youth_breakthrough;
        let age_factor = if age <= 21 { 1.0 } else { 0.6 };
        let mag = base * age_factor;
        // 5-year cooldown ≈ one-shot per career spell.
        self.happiness
            .add_event_with_cooldown(HappinessEventType::YouthBreakthrough, mag, 365 * 5);
    }

    /// React to a team-level season / competition outcome. Magnitude is
    /// the catalog default scaled by the player's involvement in the
    /// season and a personality blend chosen for the event type. Cooldown
    /// gates prevent the same event firing twice when emit logic stutters
    /// (e.g. season-end ticking on consecutive days).
    pub fn on_team_season_event(
        &mut self,
        event: HappinessEventType,
        cooldown_days: u16,
        now: NaiveDate,
    ) -> bool {
        self.on_team_season_event_with_prestige(event, cooldown_days, 1.0, now)
    }

    /// React to being named the league's Player of the Week. Recorded as a
    /// big career-visible event with a 6-day cooldown so the same player can
    /// win consecutive weeks without the second emit being swallowed, but a
    /// double-fire on the same Monday tick is still rejected.
    pub fn on_player_of_the_week(&mut self) -> bool {
        self.on_recognition_award(
            HappinessEventType::PlayerOfTheWeek,
            RecognitionEventContext::new(RecognitionEventKind::PlayerOfTheWeek),
            6,
        )
    }

    /// Centralised entry point for award / recognition events. Wraps the
    /// catalog-default magnitude path with a structured
    /// [`RecognitionEventContext`] so the renderer can describe what was
    /// won, the season totals or vote margin behind the award, and who
    /// the closest contender was. Returns whether the event was recorded
    /// (cooldown may have suppressed it).
    pub fn on_recognition_award(
        &mut self,
        event: HappinessEventType,
        context: RecognitionEventContext,
        cooldown_days: u16,
    ) -> bool {
        let cfg = HappinessConfig::default();
        let magnitude = cfg.catalog.magnitude(event.clone());
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::Media,
        )
        .with_recognition_context(context);
        self.happiness.add_event_with_context_and_cooldown(
            event,
            magnitude,
            None,
            happiness_ctx,
            cooldown_days,
        )
    }

    /// Centralised entry point for season-outcome events (relegation,
    /// relegation-fear, survival). Wraps `on_team_season_event` to also
    /// attach a [`SeasonOutcomeContext`] so the renderer can explain
    /// position, points, and gap-to-safety instead of a bare verdict.
    pub fn on_season_outcome(
        &mut self,
        event: HappinessEventType,
        cooldown_days: u16,
        prestige: f32,
        now: NaiveDate,
        context: SeasonOutcomeContext,
    ) -> bool {
        if self.happiness.has_recent_event(&event, cooldown_days) {
            return false;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.magnitude(event.clone());
        let participation = self.season_participation_factor();
        let age = self.age(now);
        let personality = self.team_event_personality_factor(&event, age);
        let role = self.season_event_role_factor(&event, age);
        let mag = base * participation * personality * role * prestige.max(0.0);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::Serious,
            HappinessEventScope::Media,
        )
        .with_season_outcome_context(context);
        self.happiness.add_event_with_context_and_cooldown(
            event,
            mag,
            None,
            happiness_ctx,
            cooldown_days,
        )
    }

    /// Centralised entry point for squad-registration / regulation
    /// events (e.g. `SquadRegistrationOmitted`). Wraps the catalog
    /// magnitude path with a structured [`RegulationEventContext`] so
    /// the renderer can describe slot type, slot counts, and who took
    /// the slot.
    pub fn on_registration_event(
        &mut self,
        event: HappinessEventType,
        context: RegulationEventContext,
        cooldown_days: u16,
    ) -> bool {
        let cfg = HappinessConfig::default();
        let magnitude = cfg.catalog.magnitude(event.clone());
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::Media,
        )
        .with_regulation_context(context);
        self.happiness.add_event_with_context_and_cooldown(
            event,
            magnitude,
            None,
            happiness_ctx,
            cooldown_days,
        )
    }

    /// Same as [`on_team_season_event`] with an explicit prestige multiplier
    /// applied to the magnitude. Use it for cup / continental events whose
    /// magnitude depends on competition tier — e.g. `0.7` for a domestic
    /// minor cup, `1.0` for a domestic top cup, `1.4` for a continental
    /// trophy. Returned bool tracks whether the event was recorded
    /// (cooldown may have suppressed it).
    pub fn on_team_season_event_with_prestige(
        &mut self,
        event: HappinessEventType,
        cooldown_days: u16,
        prestige: f32,
        now: NaiveDate,
    ) -> bool {
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.magnitude(event.clone());
        let participation = self.season_participation_factor();
        let age = self.age(now);
        let personality = self.team_event_personality_factor(&event, age);
        let role = self.season_event_role_factor(&event, age);
        let mag = base * participation * personality * role * prestige.max(0.0);
        self.happiness
            .add_event_with_cooldown(event, mag, cooldown_days)
    }
}
