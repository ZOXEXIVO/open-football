//! Late-career lifecycle: the arc from an active player weighing
//! retirement, through the formal retirement announcement, to a veteran
//! leader signalling interest in a coaching career. The emit logic is
//! wrapped in [`CareerStageDetector`] (so call sites stay thin) and on
//! [`Player`] itself for the announcement, which mutates retirement state
//! and is therefore reusable by any future contracted-retirement path.

use crate::club::person::Person;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::utils::FloatUtils;
use crate::{
    CareerStageEventContext, CareerStageEventKind, CareerStageEvidence, HappinessEventCause,
    HappinessEventContext, HappinessEventScope, HappinessEventSeverity, HappinessEventType,
    MatchExperienceBackground, Player, PlayerSquadStatus, PlayerStatusType, RetirementReason,
    TeamInfo,
};
use chrono::NaiveDate;

/// Cooldown windows (days) for the career-stage events so the monthly
/// audits don't spam the feed.
const RETIREMENT_CONSIDERING_COOLDOWN_DAYS: u16 = 180;
const COACHING_INTEREST_COOLDOWN_DAYS: u16 = 365;

/// Season-end retirement pull of a man who barely featured, and of an
/// ever-present, relative to what his age alone implies.
const UNUSED_RETIREMENT_PULL: f32 = 2.0;
const EVER_PRESENT_RETIREMENT_PULL: f32 = 0.08;

/// The ages a career can end between. Ability sets the band — clubs keep
/// wanting a better player for longer — keepers and defenders outlast
/// forwards, and a per-player offset stops one cohort retiring together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetirementWindow {
    pub opens: u8,
    pub closes: u8,
}

impl RetirementWindow {
    pub fn of(player: &Player) -> Self {
        let (opens, closes): (i16, i16) = match player.player_attributes.current_ability {
            0..=39 => (31, 36),
            40..=69 => (32, 37),
            70..=99 => (33, 38),
            100..=129 => (34, 39),
            130..=159 => (35, 40),
            160..=179 => (36, 41),
            _ => (37, 42),
        };
        let position = player.position();
        let position_offset: i16 = if position.is_goalkeeper() {
            2
        } else if position.is_defender() {
            1
        } else if position.is_forward() {
            -1
        } else {
            0
        };
        let offset = position_offset + (player.id % 3) as i16 - 1;
        RetirementWindow {
            opens: (opens + offset).clamp(30, 44) as u8,
            closes: (closes + offset).clamp(34, 46) as u8,
        }
    }

    /// How far through the window an age (in fractional years) sits:
    /// 0 at the opening, 1 at the close.
    fn progress(&self, years: f32) -> f32 {
        let span = (self.closes - self.opens) as f32;
        ((years - self.opens as f32) / span).clamp(0.0, 1.0)
    }
}

impl Player {
    /// Season-end verdict on hanging up his boots; `None` means he plays
    /// on. The pull grows through his retirement window and every career
    /// ends at its close. Inside it, a season of real football holds the
    /// pull off and a season without it brings it forward — an
    /// ever-present at the opening of his window is almost never the man
    /// who stops.
    pub fn season_end_retirement(&self, date: NaiveDate) -> Option<RetirementReason> {
        let window = RetirementWindow::of(self);
        let age = self.age(date);
        if age < window.opens {
            return None;
        }
        let involvement = self.season_involvement();
        let retires = age >= window.closes
            || FloatUtils::random(0.0, 1.0) < self.retirement_chance(window, involvement, date);
        retires.then(|| self.retirement_reason(involvement))
    }

    /// Monthly backstop for a man past the age any career runs to, so he
    /// doesn't linger in a squad until the season-end verdict. Five games
    /// this season or last buy one more year.
    pub fn overdue_retirement(&self, date: NaiveDate) -> Option<RetirementReason> {
        let still_playing = self.statistics.total_games() >= 5
            || self
                .statistics_history
                .items
                .last()
                .map(|h| h.statistics.total_games() >= 5)
                .unwrap_or(true);
        let ceiling = RetirementWindow::of(self).closes + 1 + u8::from(still_playing);
        (self.age(date) >= ceiling).then(|| self.retirement_reason(self.season_involvement()))
    }

    /// Retire straight out of a squad. The spell he was playing closes
    /// like any departure first, so the season he stopped in keeps its
    /// games.
    pub fn retire_from_squad(
        &mut self,
        from: &TeamInfo,
        date: NaiveDate,
        reason: RetirementReason,
    ) {
        self.on_retirement(from, date);
        self.announce_retirement(date, reason);
    }

    fn retirement_chance(
        &self,
        window: RetirementWindow,
        involvement: f32,
        date: NaiveDate,
    ) -> f32 {
        let years = (date - self.birth_date).num_days() as f32 / 365.25;
        let progress = window.progress(years);
        let age_pull = 0.05 + 0.55 * progress * progress;
        // Ambition and determination (0–20, neutral at 10) keep a man going.
        let resolve = (self.attributes.ambition - 10.0) * 0.03
            + (self.skills.mental.determination - 10.0) * 0.02;
        let temperament = (1.0 - resolve).clamp(0.5, 1.5);
        let role = EVER_PRESENT_RETIREMENT_PULL
            + (UNUSED_RETIREMENT_PULL - EVER_PRESENT_RETIREMENT_PULL) * (1.0 - involvement).powi(2);
        (age_pull * temperament * role).min(0.85)
    }

    /// Share of a full league season he played across every spell of the
    /// campaign, 0..1 — starts count whole, substitute appearances half.
    fn season_involvement(&self) -> f32 {
        let season = self
            .statistics_history
            .current_season_stats(&self.statistics);
        let apps = season.played as f32 + 0.5 * season.played_subs as f32;
        (apps / MatchExperienceBackground::SEASON_MATCHES).clamp(0.0, 1.0)
    }

    fn retirement_reason(&self, involvement: f32) -> RetirementReason {
        if involvement < 0.3 {
            RetirementReason::ReducedRole
        } else if self.player_attributes.world_reputation >= 7000 {
            RetirementReason::PlannedFarewell
        } else {
            RetirementReason::Age
        }
    }

    /// Record a formal retirement announcement and move the player into
    /// retirement state. Emits a career-visible [`RetirementAnnounced`]
    /// event *before* flipping the retirement flags so the event remains
    /// visible in history, then sets `Ret` status, clears the contract
    /// and any loan, and marks the player retired.
    ///
    /// Idempotent: a player who has already retired produces no second
    /// announcement. Magnitude is positive for a planned / legend
    /// farewell, neutral for ordinary age retirement, and negative for a
    /// forced (long-unemployment) or injury-driven early stop.
    ///
    /// [`RetirementAnnounced`]: HappinessEventType::RetirementAnnounced
    pub fn announce_retirement(&mut self, date: NaiveDate, reason: RetirementReason) {
        if self.retired {
            return;
        }

        let age = self.age(date);
        let world_rep = self.player_attributes.world_reputation.max(0) as u16;
        let magnitude = CareerStageDetector::retirement_magnitude(reason);

        let mut stage = CareerStageEventContext::new(CareerStageEventKind::RetirementAnnounced)
            .with_age(age)
            .with_world_reputation(world_rep)
            .with_retirement_reason(reason);
        for ev in CareerStageDetector::retirement_evidence(reason, world_rep) {
            stage = stage.with_evidence(ev);
        }

        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::Personal,
        )
        .with_career_stage_context(stage);

        self.happiness.add_event_with_context(
            HappinessEventType::RetirementAnnounced,
            magnitude,
            None,
            happiness_ctx,
        );

        self.statuses.add(date, PlayerStatusType::Ret);
        self.contract = None;
        self.contract_loan = None;
        self.retired = true;
    }

    /// Emit a [`RetirementConsidering`] mood for a long-term free agent
    /// who realistically faces the end of his career — old enough, still
    /// without a club, and not retiring this tick. Returns `true` if the
    /// event landed. Mostly informational; never sets `retired`.
    ///
    /// [`RetirementConsidering`]: HappinessEventType::RetirementConsidering
    pub fn consider_retirement_as_free_agent(
        &mut self,
        date: NaiveDate,
        months_without_club: u16,
    ) -> bool {
        if self.retired {
            return false;
        }
        if self.happiness.has_recent_event(
            &HappinessEventType::RetirementConsidering,
            RETIREMENT_CONSIDERING_COOLDOWN_DAYS,
        ) {
            return false;
        }
        let age = self.age(date);
        if !CareerStageDetector::is_in_retirement_age_window(self, age) {
            return false;
        }

        let world_rep = self.player_attributes.world_reputation.max(0) as u16;
        let mut stage = CareerStageEventContext::new(CareerStageEventKind::RetirementConsidering)
            .with_age(age)
            .with_world_reputation(world_rep)
            .with_months_without_club(months_without_club)
            .with_evidence(CareerStageEvidence::LateCareer);
        if months_without_club >= 9 {
            stage = stage.with_evidence(CareerStageEvidence::LongFreeAgency);
        }

        CareerStageDetector::emit_considering(self, stage);
        true
    }
}

/// Detector cluster for the late-career arc. Wrapping the per-player gates
/// here keeps the monthly team-behaviour audit thin and the thresholds in
/// one place. All methods take `&mut Player` and return whether an event
/// was emitted.
pub struct CareerStageDetector;

impl CareerStageDetector {
    /// Magnitude for a retirement announcement by reason — positive for a
    /// chosen / earned send-off, negative for a forced or injury-driven
    /// end. See the [`MoraleEventCatalog`] base (`retirement_announced`)
    /// for the planned-farewell anchor.
    ///
    /// [`MoraleEventCatalog`]: crate::club::player::behaviour_config::MoraleEventCatalog
    pub fn retirement_magnitude(reason: RetirementReason) -> f32 {
        match reason {
            RetirementReason::ClubLegendFarewell => 2.0,
            RetirementReason::PlannedFarewell => 1.0,
            RetirementReason::Age => 0.0,
            RetirementReason::ReducedRole => -1.0,
            RetirementReason::LongFreeAgency => -2.0,
            RetirementReason::Injury => -3.0,
        }
    }

    fn retirement_evidence(reason: RetirementReason, world_rep: u16) -> Vec<CareerStageEvidence> {
        let mut evidence = vec![CareerStageEvidence::LateCareer];
        match reason {
            RetirementReason::LongFreeAgency => evidence.push(CareerStageEvidence::LongFreeAgency),
            RetirementReason::Injury => evidence.push(CareerStageEvidence::RepeatedInjuries),
            RetirementReason::ReducedRole => evidence.push(CareerStageEvidence::ReducedRole),
            _ => {}
        }
        if world_rep >= 6000 {
            evidence.push(CareerStageEvidence::HighReputation);
        }
        evidence
    }

    /// A player weighs stopping from the day his [`RetirementWindow`]
    /// opens — the same window the season-end verdict is taken in, so no
    /// one retires before he could have thought about it.
    fn is_in_retirement_age_window(player: &Player, age: u8) -> bool {
        age >= RetirementWindow::of(player).opens
    }

    /// The last years of a career, where even an ever-present starter
    /// openly weighs stopping. Three years past the window opening.
    fn is_in_late_career_tail(player: &Player, age: u8) -> bool {
        age >= RetirementWindow::of(player).opens + 3
    }

    fn emit_considering(player: &mut Player, stage: CareerStageEventContext) {
        let magnitude = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::RetirementConsidering);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::Personal,
        )
        .with_career_stage_context(stage);
        player.happiness.add_event_with_context(
            HappinessEventType::RetirementConsidering,
            magnitude,
            None,
            happiness_ctx,
        );
    }

    /// Monthly retirement-thought audit for a contracted veteran. Fires
    /// when an older player's role has clearly faded — a reduced squad
    /// status, a bench EMA, low morale, or a near-expiry deal with no
    /// renewal — while regular starters are suppressed entirely. Returns
    /// `true` if the event landed.
    pub fn maybe_consider_retirement(player: &mut Player, today: NaiveDate) -> bool {
        if player.is_retired() {
            return false;
        }
        if player.happiness.has_recent_event(
            &HappinessEventType::RetirementConsidering,
            RETIREMENT_CONSIDERING_COOLDOWN_DAYS,
        ) {
            return false;
        }
        let age = player.age(today);
        if !Self::is_in_retirement_age_window(player, age) {
            return false;
        }

        let status = player
            .contract
            .as_ref()
            .map(|c| c.squad_status.clone())
            .unwrap_or(PlayerSquadStatus::FirstTeamRegular);

        // A regular starter is still enjoying his football — he isn't
        // weighing retirement yet. But only up to a point: an ever-present
        // deep into the tail of his career is precisely the man whose
        // club needs to hear him thinking about it, and suppressing the
        // thought unconditionally meant a forty-year-old first choice
        // gave his club no warning at all — it learned he was gone the
        // season he went.
        let is_regular_starter = matches!(
            status,
            PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
        ) && player.happiness.starter_ratio >= 0.6;
        if is_regular_starter && !Self::is_in_late_career_tail(player, age) {
            return false;
        }

        let mut score = 0i32;
        let mut evidence = vec![CareerStageEvidence::LateCareer];

        if matches!(
            status,
            PlayerSquadStatus::NotNeeded
                | PlayerSquadStatus::MainBackupPlayer
                | PlayerSquadStatus::DecentYoungster
        ) {
            score += 2;
            evidence.push(CareerStageEvidence::ReducedRole);
        }
        if player.happiness.starter_ratio < 0.3 {
            score += 1;
            if !evidence.contains(&CareerStageEvidence::ReducedRole) {
                evidence.push(CareerStageEvidence::ReducedRole);
            }
        }
        if player.happiness.morale < 35.0 {
            score += 1;
        }
        if let Some(contract) = player.contract.as_ref() {
            let days_to_expiry = (contract.expiration - today).num_days();
            if days_to_expiry > 0 && days_to_expiry <= 365 {
                score += 1;
            }
        }
        // Low determination makes hanging on less likely.
        if player.skills.mental.determination < 8.0 {
            score += 1;
        }

        if score < 2 {
            return false;
        }

        let world_rep = player.player_attributes.world_reputation.max(0) as u16;
        let apps = player.statistics.played + player.statistics.played_subs;
        let mut stage = CareerStageEventContext::new(CareerStageEventKind::RetirementConsidering)
            .with_age(age)
            .with_world_reputation(world_rep)
            .with_appearances_this_season(apps);
        for ev in evidence {
            stage = stage.with_evidence(ev);
        }
        Self::emit_considering(player, stage);
        true
    }

    /// A decade of unbroken service at one club — the club-servant
    /// milestone. Tenure is anchored on the player's LAST transfer
    /// (nothing to celebrate mid-carousel); players whose whole career
    /// predates the sim (no transfer anchor) accrue the milestone from
    /// their first in-sim move onward. The ten-year cooldown makes the
    /// twenty-year celebration land too.
    pub fn maybe_celebrate_club_service(player: &mut Player, today: NaiveDate) {
        const DECADE_DAYS: i64 = 3652;
        if player.is_retired() || player.contract.is_none() {
            return;
        }
        let Some(joined) = player.last_transfer_date else {
            return;
        };
        if (today - joined).num_days() < DECADE_DAYS {
            return;
        }
        let mag = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::ClubServantMilestone);
        player.happiness.add_event_with_cooldown(
            HappinessEventType::ClubServantMilestone,
            mag,
            3650,
        );
    }

    /// A capped veteran steps away from international football to
    /// prolong his club career. One-way and announced once; the callup
    /// pass stops considering him the same day. Goalkeeper careers (and
    /// so their international careers) run ~2 years longer.
    pub fn maybe_retire_from_international_football(player: &mut Player, today: NaiveDate) -> bool {
        if player.international_retired || player.is_retired() {
            return false;
        }
        // Only a real international career gets an announcement — the
        // never-capped simply stop being scouted for the shirt.
        if player.player_attributes.international_apps < 20 {
            return false;
        }
        let is_goalkeeper = player.position().is_goalkeeper();
        let line = if is_goalkeeper { 36 } else { 34 };
        if player.age(today) < line {
            return false;
        }
        player.international_retired = true;
        let mag = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::InternationalRetirement);
        player
            .happiness
            .add_event(HappinessEventType::InternationalRetirement, mag);
        true
    }

    /// Monthly coaching-interest audit for a veteran leader. Surfaces
    /// players with the temperament (professionalism / determination) and
    /// standing (leadership, captaincy, mentorship) to make future
    /// coaches. Positive event; never advances retirement. Returns `true`
    /// if the event landed. First implementation emits the event only —
    /// staff conversion is left to a follow-up.
    pub fn maybe_show_coaching_interest(player: &mut Player, today: NaiveDate) -> bool {
        if player.is_retired() {
            return false;
        }
        if player.happiness.has_recent_event(
            &HappinessEventType::CoachingCareerInterest,
            COACHING_INTEREST_COOLDOWN_DAYS,
        ) {
            return false;
        }
        let age = player.age(today);
        if age < 31 {
            return false;
        }

        let professionalism = player.attributes.professionalism;
        let determination = player.skills.mental.determination;
        let leadership = player.skills.mental.leadership;
        if determination < 12.0 {
            return false;
        }

        let leader_signal = leadership >= 14.0
            || player
                .happiness
                .has_recent_event(&HappinessEventType::LeadershipEmergence, 365)
            || player
                .happiness
                .has_recent_event(&HappinessEventType::CaptaincyAwarded, 3650);
        if !(professionalism >= 14.0 || leader_signal) {
            return false;
        }

        // Suppress: controversial low-professionalism characters, and
        // prime-age stars still chasing trophies as players.
        if player.attributes.controversy > 14.0 && professionalism < 10.0 {
            return false;
        }
        if age < 33 && player.attributes.ambition >= 16.0 {
            return false;
        }

        let world_rep = player.player_attributes.world_reputation.max(0) as u16;
        let mut stage = CareerStageEventContext::new(CareerStageEventKind::CoachingCareerInterest)
            .with_age(age)
            .with_world_reputation(world_rep)
            .with_evidence(CareerStageEvidence::LateCareer);
        if leadership >= 14.0 {
            stage = stage.with_evidence(CareerStageEvidence::LeadershipEmergence);
        }
        if professionalism >= 14.0 {
            stage = stage.with_evidence(CareerStageEvidence::HighProfessionalism);
        }
        if player
            .happiness
            .has_recent_event(&HappinessEventType::CaptaincyAwarded, 3650)
        {
            stage = stage.with_evidence(CareerStageEvidence::Captaincy);
        }
        if player
            .happiness
            .has_recent_event(&HappinessEventType::RetirementConsidering, 365)
        {
            stage = stage.with_evidence(CareerStageEvidence::RecentRetirementConsidering);
        }

        let magnitude = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::CoachingCareerInterest);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::Personal,
        )
        .with_career_stage_context(stage);
        player.happiness.add_event_with_context(
            HappinessEventType::CoachingCareerInterest,
            magnitude,
            None,
            happiness_ctx,
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, PlayerSquadStatus,
    };
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn count_event(player: &Player, kind: HappinessEventType) -> usize {
        player
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == kind)
            .count()
    }

    /// Build a player at the given position with explicit personality /
    /// mental attributes and an optional contract squad status.
    fn build(
        birth: NaiveDate,
        pos: PlayerPositionType,
        attrs: PersonAttributes,
        determination: f32,
        leadership: f32,
        status: Option<PlayerSquadStatus>,
    ) -> Player {
        let mut skills = PlayerSkills::default();
        skills.mental.determination = determination;
        skills.mental.leadership = leadership;

        let contract = status.map(|s| {
            let mut c = PlayerClubContract::new(50_000, d(2028, 6, 30));
            c.squad_status = s;
            c
        });

        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".into(), "Player".into()))
            .birth_date(birth)
            .country_id(1)
            .attributes(attrs)
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: pos,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .contract(contract)
            .build()
            .unwrap()
    }

    fn neutral_attrs() -> PersonAttributes {
        PersonAttributes {
            adaptability: 12.0,
            ambition: 12.0,
            controversy: 5.0,
            loyalty: 12.0,
            pressure: 12.0,
            professionalism: 12.0,
            sportsmanship: 12.0,
            temperament: 12.0,
            consistency: 12.0,
            important_matches: 12.0,
            dirtiness: 5.0,
        }
    }

    // ── RetirementAnnounced ─────────────────────────────────────

    #[test]
    fn announce_retirement_records_event_and_sets_state() {
        let mut p = build(
            d(1990, 1, 1),
            PlayerPositionType::Striker,
            neutral_attrs(),
            10.0,
            8.0,
            Some(PlayerSquadStatus::FirstTeamRegular),
        );
        p.announce_retirement(d(2026, 5, 30), RetirementReason::LongFreeAgency);

        assert!(p.is_retired(), "player must be marked retired");
        assert!(
            p.contract.is_none(),
            "contract must be cleared on retirement"
        );
        assert_eq!(
            count_event(&p, HappinessEventType::RetirementAnnounced),
            1,
            "exactly one announcement event"
        );
        let ev = p
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::RetirementAnnounced)
            .unwrap();
        assert!(
            ev.magnitude < 0.0,
            "long-unemployment retirement reads negative"
        );
        assert!(
            ev.context
                .as_ref()
                .and_then(|c| c.career_stage_context.as_ref())
                .is_some(),
            "announcement carries a career-stage context"
        );
    }

    #[test]
    fn planned_farewell_reads_positive_injury_reads_worse() {
        assert!(CareerStageDetector::retirement_magnitude(RetirementReason::PlannedFarewell) > 0.0);
        assert!(
            CareerStageDetector::retirement_magnitude(RetirementReason::ClubLegendFarewell) > 0.0
        );
        assert!(
            CareerStageDetector::retirement_magnitude(RetirementReason::Injury)
                < CareerStageDetector::retirement_magnitude(RetirementReason::LongFreeAgency),
            "injury-forced retirement is the deepest cut"
        );
    }

    #[test]
    fn announce_retirement_is_idempotent() {
        let mut p = build(
            d(1990, 1, 1),
            PlayerPositionType::Striker,
            neutral_attrs(),
            10.0,
            8.0,
            Some(PlayerSquadStatus::FirstTeamRegular),
        );
        p.announce_retirement(d(2026, 5, 30), RetirementReason::Age);
        p.announce_retirement(d(2026, 5, 30), RetirementReason::Age);
        assert_eq!(
            count_event(&p, HappinessEventType::RetirementAnnounced),
            1,
            "already-retired player must not announce twice"
        );
    }

    // ── RetirementConsidering (free agent) ──────────────────────

    #[test]
    fn old_free_agent_emits_considering_young_does_not() {
        let mut old = build(
            d(1990, 1, 1),
            PlayerPositionType::Striker,
            neutral_attrs(),
            10.0,
            8.0,
            None,
        );
        assert!(old.consider_retirement_as_free_agent(d(2026, 5, 30), 14));
        assert_eq!(
            count_event(&old, HappinessEventType::RetirementConsidering),
            1
        );

        let mut young = build(
            d(2002, 1, 1),
            PlayerPositionType::Striker,
            neutral_attrs(),
            10.0,
            8.0,
            None,
        );
        assert!(!young.consider_retirement_as_free_agent(d(2026, 5, 30), 14));
        assert_eq!(
            count_event(&young, HappinessEventType::RetirementConsidering),
            0
        );
    }

    #[test]
    fn considering_respects_cooldown() {
        let mut p = build(
            d(1990, 1, 1),
            PlayerPositionType::Striker,
            neutral_attrs(),
            10.0,
            8.0,
            None,
        );
        assert!(p.consider_retirement_as_free_agent(d(2026, 5, 30), 14));
        assert!(
            !p.consider_retirement_as_free_agent(d(2026, 5, 30), 14),
            "second emit inside the 180-day cooldown is suppressed"
        );
        assert_eq!(
            count_event(&p, HappinessEventType::RetirementConsidering),
            1
        );
    }

    // ── RetirementConsidering (contracted veteran) ──────────────

    #[test]
    fn faded_veteran_considers_retirement() {
        let mut p = build(
            d(1990, 1, 1),
            PlayerPositionType::Striker,
            neutral_attrs(),
            6.0,
            8.0,
            Some(PlayerSquadStatus::NotNeeded),
        );
        p.happiness.starter_ratio = 0.1;
        p.happiness.morale = 30.0;
        assert!(CareerStageDetector::maybe_consider_retirement(
            &mut p,
            d(2026, 5, 30)
        ));
        assert_eq!(
            count_event(&p, HappinessEventType::RetirementConsidering),
            1
        );
    }

    #[test]
    fn regular_starting_veteran_is_suppressed() {
        let mut p = build(
            d(1990, 1, 1),
            PlayerPositionType::Striker,
            neutral_attrs(),
            14.0,
            8.0,
            Some(PlayerSquadStatus::KeyPlayer),
        );
        p.happiness.starter_ratio = 0.9;
        assert!(
            !CareerStageDetector::maybe_consider_retirement(&mut p, d(2026, 5, 30)),
            "a regular starter is not weighing retirement"
        );
    }

    // ── CoachingCareerInterest ──────────────────────────────────

    #[test]
    fn veteran_leader_shows_coaching_interest() {
        let mut attrs = neutral_attrs();
        attrs.professionalism = 16.0;
        let mut p = build(
            d(1992, 1, 1), // 34 on the test date
            PlayerPositionType::Striker,
            attrs,
            15.0,
            16.0,
            Some(PlayerSquadStatus::FirstTeamRegular),
        );
        assert!(CareerStageDetector::maybe_show_coaching_interest(
            &mut p,
            d(2026, 5, 30)
        ));
        assert_eq!(
            count_event(&p, HappinessEventType::CoachingCareerInterest),
            1
        );
    }

    #[test]
    fn young_player_no_coaching_interest() {
        let mut attrs = neutral_attrs();
        attrs.professionalism = 16.0;
        let mut p = build(
            d(2002, 1, 1),
            PlayerPositionType::Striker,
            attrs,
            15.0,
            16.0,
            Some(PlayerSquadStatus::FirstTeamRegular),
        );
        assert!(!CareerStageDetector::maybe_show_coaching_interest(
            &mut p,
            d(2026, 5, 30)
        ));
    }

    #[test]
    fn controversial_low_professional_suppresses_coaching() {
        let mut attrs = neutral_attrs();
        attrs.professionalism = 8.0;
        attrs.controversy = 16.0;
        let mut p = build(
            d(1992, 1, 1),
            PlayerPositionType::Striker,
            attrs,
            15.0,
            16.0,
            Some(PlayerSquadStatus::FirstTeamRegular),
        );
        assert!(!CareerStageDetector::maybe_show_coaching_interest(
            &mut p,
            d(2026, 5, 30)
        ));
    }

    // ── Season-end retirement ───────────────────────────────────

    /// The reported striker: CA 130, id ≡ 0 (mod 3), ambitious and
    /// determined, 33 at Portugal's 2029/30 season end.
    fn window_opening_striker(starts: u16) -> Player {
        let mut attrs = neutral_attrs();
        attrs.ambition = 15.0;
        let mut p = PlayerBuilder::new()
            .id(3)
            .full_name(FullName::new("Test".into(), "Striker".into()))
            .birth_date(d(1997, 3, 7))
            .country_id(1)
            .attributes(attrs)
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes {
                current_ability: 130,
                potential_ability: 134,
                ..Default::default()
            })
            .build()
            .unwrap();
        p.skills.mental.determination = 16.0;
        p.statistics.played = starts;
        p
    }

    fn chance(p: &Player, date: NaiveDate) -> f32 {
        p.retirement_chance(RetirementWindow::of(p), p.season_involvement(), date)
    }

    #[test]
    fn reported_striker_window_opens_at_33() {
        let p = window_opening_striker(34);
        assert_eq!(
            RetirementWindow::of(&p),
            RetirementWindow {
                opens: 33,
                closes: 38
            }
        );
    }

    #[test]
    fn ever_present_barely_weighs_retiring_at_window_opening() {
        let p = window_opening_striker(34);
        let c = chance(&p, d(2030, 5, 19));
        assert!(
            c < 0.005,
            "a 34-start season at the window opening must not read as a retirement, got {c}"
        );
    }

    #[test]
    fn a_season_without_football_brings_retirement_forward() {
        let date = d(2030, 5, 19);
        let starter = chance(&window_opening_striker(34), date);
        let unused = chance(&window_opening_striker(0), date);
        assert!(unused > 0.05, "unused veteran got {unused}");
        assert!(
            unused > starter * 20.0,
            "playing time must dominate: unused {unused} vs starter {starter}"
        );
    }

    #[test]
    fn retirement_pull_grows_through_the_window() {
        let p = window_opening_striker(17);
        let early = chance(&p, d(2030, 5, 19));
        let late = chance(&p, d(2034, 5, 19));
        assert!(late > early * 5.0, "early {early}, late {late}");
    }

    #[test]
    fn no_season_end_retirement_before_the_window_opens() {
        let p = window_opening_striker(0);
        assert_eq!(p.season_end_retirement(d(2029, 5, 19)), None);
    }

    #[test]
    fn window_close_ends_even_an_ever_present_career() {
        let p = window_opening_striker(34);
        assert_eq!(
            p.season_end_retirement(d(2035, 5, 19)),
            Some(RetirementReason::Age)
        );
    }

    #[test]
    fn overdue_backstop_gives_a_playing_man_one_more_year() {
        let p = window_opening_striker(20);
        assert_eq!(p.overdue_retirement(d(2036, 5, 19)), None);
        assert_eq!(
            p.overdue_retirement(d(2037, 5, 19)),
            Some(RetirementReason::Age)
        );
    }

    #[test]
    fn considering_opens_with_the_retirement_window() {
        let mut p = window_opening_striker(0);
        assert!(!p.consider_retirement_as_free_agent(d(2029, 5, 19), 14));
        assert!(p.consider_retirement_as_free_agent(d(2030, 5, 19), 14));
    }

    #[test]
    fn retiring_loanee_closes_his_loan_spell_with_its_games() {
        let gil = TeamInfo {
            name: "Gil Vicente".into(),
            slug: "gil-vicente".into(),
            reputation: 100,
            league_name: "Primeira Liga".into(),
            league_slug: "primeira-liga".into(),
        };
        let mut p = window_opening_striker(34);
        p.statistics.goals = 23;
        p.contract = Some(PlayerClubContract::new(50_000, d(2030, 6, 30)));
        p.contract_loan = Some(PlayerClubContract::new_loan(
            50_000,
            d(2030, 6, 30),
            99,
            0,
            100,
        ));
        p.statistics_history
            .seed_initial_team(&gil, d(2029, 6, 12), true);

        p.retire_from_squad(&gil, d(2030, 5, 19), RetirementReason::Age);

        assert!(p.is_retired());
        assert!(p.contract.is_none() && p.contract_loan.is_none());
        assert_eq!(p.statistics.total_games(), 0, "live season drained");
        let spell = p
            .statistics_history
            .current
            .iter()
            .find(|e| e.team_slug == "gil-vicente")
            .unwrap();
        assert!(spell.is_loan && spell.departed_date.is_some());
        assert_eq!((spell.statistics.played, spell.statistics.goals), (34, 23));
        assert_eq!(count_event(&p, HappinessEventType::RetirementAnnounced), 1);
    }
}
