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
use crate::{HappinessEventType, Person};

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
        self.happiness.add_event_with_cooldown(
            HappinessEventType::YouthBreakthrough,
            mag,
            365 * 5,
        );
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
