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

/// Centralised, FM-style classification of award recognitions for the
/// reputation-impact pipeline. One variant per award the simulator can
/// emit; mapping to `HappinessEventType` is purely descriptive — the
/// reputation effect is owned here, not on the happiness event.
///
/// Reputation deltas are computed by [`Player::apply_award_reputation_impact`]
/// from the kind's centre delta + league/headroom/breakthrough/quality
/// multipliers. The impact is profile/visibility only — it never feeds
/// back into ability or potential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwardReputationKind {
    PlayerOfTheWeek,
    YoungPlayerOfTheWeek,
    TeamOfTheWeekSelection,
    YoungTeamOfTheWeekSelection,
    PlayerOfTheMonth,
    YoungPlayerOfTheMonth,
    TeamOfTheMonthSelection,
    YoungTeamOfTheMonthSelection,
    TeamOfTheSeasonSelection,
    TeamOfTheYearSelection,
    PlayerOfTheSeason,
    YoungPlayerOfTheSeason,
    LeagueTopScorer,
    LeagueTopAssists,
    LeagueGoldenGlove,
    ContinentalPlayerOfYear,
    WorldPlayerOfYear,
}

impl AwardReputationKind {
    /// Centre deltas (current, home, world) chosen near the midpoint of
    /// the FM-style design ranges. The final delta is scaled down by
    /// reputation headroom and may be amplified by award quality (avg
    /// rating, season volume) and youth breakthrough.
    fn centre_delta(self) -> (i32, i32, i32) {
        match self {
            Self::TeamOfTheWeekSelection => (32, 25, 4),
            Self::YoungTeamOfTheWeekSelection => (40, 32, 5),
            Self::PlayerOfTheWeek => (65, 55, 12),
            Self::YoungPlayerOfTheWeek => (85, 70, 15),
            Self::PlayerOfTheMonth => (135, 110, 35),
            Self::YoungPlayerOfTheMonth => (155, 125, 40),
            Self::TeamOfTheMonthSelection => (75, 60, 18),
            Self::YoungTeamOfTheMonthSelection => (90, 72, 22),
            Self::TeamOfTheSeasonSelection => (240, 200, 80),
            Self::TeamOfTheYearSelection => (300, 240, 120),
            Self::PlayerOfTheSeason => (470, 400, 200),
            Self::YoungPlayerOfTheSeason => (510, 430, 210),
            Self::LeagueTopScorer => (300, 260, 100),
            Self::LeagueTopAssists => (220, 190, 70),
            Self::LeagueGoldenGlove => (260, 220, 80),
            Self::ContinentalPlayerOfYear => (500, 500, 250),
            Self::WorldPlayerOfYear => (900, 900, 500),
        }
    }

    /// Continental / World POY are league-agnostic. They skip the league
    /// + headroom + saturation envelopes entirely so the historic
    /// `+500/+500/+250` and `+900/+900/+500` scales are preserved
    /// regardless of where the recipient plays.
    fn is_global(self) -> bool {
        matches!(
            self,
            Self::ContinentalPlayerOfYear | Self::WorldPlayerOfYear
        )
    }

    fn is_weekly(self) -> bool {
        matches!(
            self,
            Self::PlayerOfTheWeek
                | Self::YoungPlayerOfTheWeek
                | Self::TeamOfTheWeekSelection
                | Self::YoungTeamOfTheWeekSelection
        )
    }

    /// True for season / calendar-year aggregations. They get the wider
    /// quality formula (matches-played factor + softer floor) because
    /// the input is a whole campaign of football, not a single weekend.
    fn is_season_aggregate(self) -> bool {
        matches!(
            self,
            Self::PlayerOfTheSeason
                | Self::YoungPlayerOfTheSeason
                | Self::TeamOfTheSeasonSelection
                | Self::TeamOfTheYearSelection
        )
    }
}

/// Optional emit-site context for [`Player::apply_award_reputation_impact`].
/// Each field is independently optional so callers attach only what they
/// have available. Missing inputs collapse to neutral multipliers
/// (1.0 quality, no league/world dampening) — the centre delta stays
/// usable on its own.
#[derive(Debug, Clone, Copy, Default)]
pub struct AwardReputationInput {
    pub league_reputation: Option<u16>,
    pub avg_rating: Option<f32>,
    pub matches_played: Option<u16>,
}

impl AwardReputationInput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_league_reputation(mut self, rep: u16) -> Self {
        self.league_reputation = Some(rep);
        self
    }

    pub fn with_avg_rating(mut self, rating: f32) -> Self {
        self.avg_rating = Some(rating);
        self
    }

    pub fn with_matches_played(mut self, matches: u16) -> Self {
        self.matches_played = Some(matches);
        self
    }
}

/// One entry in the lifetime award log. Storing the date alongside the
/// kind lets the Awards-tab chart bucket totals by year / month, which
/// the per-league archives can't do once their retention windows expire.
#[derive(Debug, Clone, Copy)]
pub struct AwardTimelineEntry {
    pub date: NaiveDate,
    pub kind: AwardReputationKind,
}

/// Capped log size — comfortably fits an unusually decorated 20-year
/// career (~30 weekly TOTW + a handful of monthlies = ~35-40 awards/yr).
/// Beyond the cap, the oldest entry is dropped: per-year counters
/// continue to be correct via the dedicated `u16` fields; only the
/// chart loses depth on the deep historical tail.
const TIMELINE_MAX: usize = 1024;

/// Lifetime tally of awards won, one counter per [`AwardReputationKind`].
/// Incremented inside [`Player::apply_award_reputation_impact`] so every
/// award path (weekly, monthly, season, year, continental, world) bumps
/// exactly one counter. Unbounded by design — the per-league archives are
/// retention-bounded, so they can't be used to render a "across all
/// seasons" tally on the player page.
#[derive(Debug, Clone, Default)]
pub struct PlayerAwardsCount {
    pub player_of_the_week: u16,
    pub young_player_of_the_week: u16,
    pub team_of_the_week: u16,
    pub young_team_of_the_week: u16,
    pub player_of_the_month: u16,
    pub young_player_of_the_month: u16,
    pub team_of_the_month: u16,
    pub young_team_of_the_month: u16,
    pub team_of_the_season: u16,
    pub team_of_the_year: u16,
    pub player_of_the_season: u16,
    pub young_player_of_the_season: u16,
    pub league_top_scorer: u16,
    pub league_top_assists: u16,
    pub league_golden_glove: u16,
    pub continental_player_of_year: u16,
    pub world_player_of_year: u16,
    /// Chronological log of every award, capped at [`TIMELINE_MAX`].
    /// Read by the web layer to chart awards by year / month.
    pub timeline: Vec<AwardTimelineEntry>,
}

impl PlayerAwardsCount {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sum across every award kind — used as the tab badge total.
    pub fn total(&self) -> u32 {
        self.player_of_the_week as u32
            + self.young_player_of_the_week as u32
            + self.team_of_the_week as u32
            + self.young_team_of_the_week as u32
            + self.player_of_the_month as u32
            + self.young_player_of_the_month as u32
            + self.team_of_the_month as u32
            + self.young_team_of_the_month as u32
            + self.team_of_the_season as u32
            + self.team_of_the_year as u32
            + self.player_of_the_season as u32
            + self.young_player_of_the_season as u32
            + self.league_top_scorer as u32
            + self.league_top_assists as u32
            + self.league_golden_glove as u32
            + self.continental_player_of_year as u32
            + self.world_player_of_year as u32
    }

    fn bump(&mut self, kind: AwardReputationKind, date: NaiveDate) {
        let slot = match kind {
            AwardReputationKind::PlayerOfTheWeek => &mut self.player_of_the_week,
            AwardReputationKind::YoungPlayerOfTheWeek => &mut self.young_player_of_the_week,
            AwardReputationKind::TeamOfTheWeekSelection => &mut self.team_of_the_week,
            AwardReputationKind::YoungTeamOfTheWeekSelection => &mut self.young_team_of_the_week,
            AwardReputationKind::PlayerOfTheMonth => &mut self.player_of_the_month,
            AwardReputationKind::YoungPlayerOfTheMonth => &mut self.young_player_of_the_month,
            AwardReputationKind::TeamOfTheMonthSelection => &mut self.team_of_the_month,
            AwardReputationKind::YoungTeamOfTheMonthSelection => &mut self.young_team_of_the_month,
            AwardReputationKind::TeamOfTheSeasonSelection => &mut self.team_of_the_season,
            AwardReputationKind::TeamOfTheYearSelection => &mut self.team_of_the_year,
            AwardReputationKind::PlayerOfTheSeason => &mut self.player_of_the_season,
            AwardReputationKind::YoungPlayerOfTheSeason => &mut self.young_player_of_the_season,
            AwardReputationKind::LeagueTopScorer => &mut self.league_top_scorer,
            AwardReputationKind::LeagueTopAssists => &mut self.league_top_assists,
            AwardReputationKind::LeagueGoldenGlove => &mut self.league_golden_glove,
            AwardReputationKind::ContinentalPlayerOfYear => &mut self.continental_player_of_year,
            AwardReputationKind::WorldPlayerOfYear => &mut self.world_player_of_year,
        };
        *slot = slot.saturating_add(1);

        self.timeline.push(AwardTimelineEntry { date, kind });
        if self.timeline.len() > TIMELINE_MAX {
            // Drop oldest to keep memory bounded on freakishly long careers.
            self.timeline.drain(0..self.timeline.len() - TIMELINE_MAX);
        }
    }
}

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

    /// React to being named the league's Young Player of the Week. Same
    /// cooldown shape as the senior weekly award — a 6-day gap is short
    /// enough that consecutive Mondays each register, but a double tick
    /// on the same Monday is suppressed.
    pub fn on_young_player_of_the_week(&mut self) -> bool {
        self.on_recognition_award(
            HappinessEventType::YoungPlayerOfTheWeek,
            RecognitionEventContext::new(RecognitionEventKind::YoungPlayerOfTheWeek),
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

    /// Centralised, FM-style award reputation impact. Awards change a
    /// player's public profile (current / home / world reputation), not
    /// their ability or potential — winning Team of the Week doesn't
    /// make a player technically better, but media interest, transfer
    /// attention, and national-team visibility all rise.
    ///
    /// The delta is built from a per-kind centre value, scaled by:
    /// - **league multiplier** — a 0.60 → 1.40 envelope on the awarding
    ///   league's reputation (small leagues mostly move home reputation,
    ///   elite leagues feed world too);
    /// - **world-exposure curve** — `(league_rep / 10000)^1.25`, so a
    ///   continental-tier league lifts world reputation but a small
    ///   domestic award barely does;
    /// - **headroom** — diminishing gains as the player approaches the
    ///   ceiling, separately for each reputation axis;
    /// - **breakthrough** — wonderkid hype for ≤20 / ≤23 sub-elites;
    /// - **quality** — avg-rating bump plus a season-volume term for
    ///   year-aggregate awards;
    /// - **saturation** — weekly awards halve once `current` ≥ 7500, and
    ///   non-global awards barely move world once world ≥ 8000;
    /// - **stacking** — a smaller weekly/monthly emit attached to a
    ///   player who already won the larger same-period award is
    ///   capped at 25–30%.
    ///
    /// Continental / World POY skip the league + headroom + saturation
    /// envelopes so their historic `+500/+500/+250` and `+900/+900/+500`
    /// scales survive intact.
    pub fn apply_award_reputation_impact(
        &mut self,
        kind: AwardReputationKind,
        input: AwardReputationInput,
        now: NaiveDate,
    ) {
        // Lifetime tally — incremented here (the single funnel every
        // award path goes through) so the Awards tab can show "across
        // all seasons" totals beyond the per-league retention bounds.
        self.awards_count.bump(kind, now);

        let (base_cur, base_home, base_world) = kind.centre_delta();
        let cur_now = self.player_attributes.current_reputation;
        let home_now = self.player_attributes.home_reputation;
        let world_now = self.player_attributes.world_reputation;

        if kind.is_global() {
            // Top-of-the-pyramid awards: scale is fixed by design,
            // ceiling clamping in `update_reputation` handles the rest.
            self.player_attributes.update_reputation(
                base_cur as i16,
                base_home as i16,
                base_world as i16,
            );
            return;
        }

        let lr_norm = (input.league_reputation.unwrap_or(5_000) as f32 / 10_000.0).clamp(0.0, 1.0);
        let league_mul = 0.60 + 0.80 * lr_norm;
        let world_exposure = lr_norm.powf(1.25);

        let quality_mul = match input.avg_rating {
            Some(rating) if kind.is_season_aggregate() => {
                let matches = input.matches_played.unwrap_or(0) as f32;
                (0.85 + (rating - 7.0) * 0.12 + ((matches + 1.0).log10()) * 0.05).clamp(0.85, 1.30)
            }
            Some(rating) => (0.85 + (rating - 7.0) * 0.18).clamp(0.75, 1.25),
            None => 1.0,
        };

        let age = self.age(now);
        let breakthrough_mul = if age <= 20 && cur_now < 1_500 {
            1.25
        } else if age <= 23 && cur_now < 2_500 {
            1.15
        } else {
            1.0
        };

        let weekly_saturation = if kind.is_weekly() && cur_now > 7_500 {
            0.5
        } else {
            1.0
        };

        // Non-global awards barely move world reputation once the
        // player is already a globally-recognised name — only continental
        // / world recognition moves the needle from there.
        let world_lock = if world_now > 8_000 { 0.20 } else { 1.0 };

        let stacking_mul = self.award_stacking_dampener(kind);

        let common = league_mul * quality_mul * breakthrough_mul * weekly_saturation * stacking_mul;

        let head_cur = (1.0 - cur_now as f32 / 10_000.0).max(0.0).powf(0.65);
        let head_home = (1.0 - home_now as f32 / 10_000.0).max(0.0).powf(0.65);
        let head_world = (1.0 - world_now as f32 / 10_000.0).max(0.0).powf(0.75);

        let delta_cur = (base_cur as f32 * common * head_cur).round() as i16;
        let delta_home = (base_home as f32 * common * head_home).round() as i16;
        let delta_world =
            (base_world as f32 * common * world_exposure * head_world * world_lock).round() as i16;

        self.player_attributes
            .update_reputation(delta_cur, delta_home, delta_world);
    }

    /// Stacking dampener for the centralised award reputation pipeline.
    /// When a player wins multiple awards in the same period, only the
    /// most prestigious gets full reputation impact — the rest are
    /// capped at 25-30% so a single great week doesn't move three
    /// reputation axes by triple the natural amount.
    ///
    /// Lookback windows match the natural cadence of each award class:
    /// weekly awards land on the same Monday tick, monthly awards are
    /// emitted within hours of each other on the 1st of the month.
    /// Senior + young weekly are checked symmetrically — whichever
    /// fires first goes full, the other dampens.
    fn award_stacking_dampener(&self, kind: AwardReputationKind) -> f32 {
        use HappinessEventType as H;
        let recent = |evt: H, days: u16| self.happiness.has_recent_event(&evt, days);
        match kind {
            AwardReputationKind::PlayerOfTheWeek => {
                if recent(H::YoungPlayerOfTheWeek, 2) {
                    0.25
                } else {
                    1.0
                }
            }
            AwardReputationKind::YoungPlayerOfTheWeek => {
                if recent(H::PlayerOfTheWeek, 2) {
                    0.25
                } else {
                    1.0
                }
            }
            AwardReputationKind::TeamOfTheWeekSelection => {
                if recent(H::PlayerOfTheWeek, 2) || recent(H::YoungPlayerOfTheWeek, 2) {
                    0.30
                } else {
                    1.0
                }
            }
            AwardReputationKind::YoungTeamOfTheWeekSelection => {
                if recent(H::YoungPlayerOfTheWeek, 2) || recent(H::PlayerOfTheWeek, 2) {
                    0.30
                } else {
                    1.0
                }
            }
            AwardReputationKind::PlayerOfTheMonth => {
                if recent(H::YoungPlayerOfTheMonth, 3) {
                    0.25
                } else {
                    1.0
                }
            }
            AwardReputationKind::YoungPlayerOfTheMonth => {
                if recent(H::PlayerOfTheMonth, 3) {
                    0.25
                } else {
                    1.0
                }
            }
            // Monthly XI selections fire on the same first-of-month tick
            // as POM / Young POM. If the same player just won the
            // larger monthly individual award, dampen the team-XI emit
            // so a single player can't take double reputation in one
            // tick. Symmetric with the weekly TOTW/POW logic.
            AwardReputationKind::TeamOfTheMonthSelection => {
                if recent(H::PlayerOfTheMonth, 3) || recent(H::YoungPlayerOfTheMonth, 3) {
                    0.30
                } else {
                    1.0
                }
            }
            AwardReputationKind::YoungTeamOfTheMonthSelection => {
                if recent(H::YoungPlayerOfTheMonth, 3) || recent(H::PlayerOfTheMonth, 3) {
                    0.30
                } else {
                    1.0
                }
            }
            _ => 1.0,
        }
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
