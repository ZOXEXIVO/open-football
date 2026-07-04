//! Late-career lifecycle: the arc from an active player weighing
//! retirement, through the formal retirement announcement, to a veteran
//! leader signalling interest in a coaching career. The emit logic is
//! wrapped in [`CareerStageDetector`] (so call sites stay thin) and on
//! [`Player`] itself for the announcement, which mutates retirement state
//! and is therefore reusable by any future contracted-retirement path.

use crate::club::person::Person;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::{
    CareerStageEventContext, CareerStageEventKind, CareerStageEvidence, HappinessEventCause,
    HappinessEventContext, HappinessEventScope, HappinessEventSeverity, HappinessEventType, Player,
    PlayerFieldPositionGroup, PlayerSquadStatus, PlayerStatusType, RetirementReason,
};
use chrono::NaiveDate;

/// Cooldown windows (days) for the career-stage events so the monthly
/// audits don't spam the feed.
const RETIREMENT_CONSIDERING_COOLDOWN_DAYS: u16 = 180;
const COACHING_INTEREST_COOLDOWN_DAYS: u16 = 365;

impl Player {
    /// Record a formal retirement announcement and move the player into
    /// retirement state. Emits a career-visible [`RetirementAnnounced`]
    /// event *before* flipping the retirement flags so the event remains
    /// visible in history, then sets `Ret` status, clears the contract,
    /// and marks the player retired.
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

    /// Outfield players are in the retirement-age window from 34, keepers
    /// (who play on longer) from 37.
    fn is_in_retirement_age_window(player: &Player, age: u8) -> bool {
        let is_keeper = player.position().position_group() == PlayerFieldPositionGroup::Goalkeeper;
        if is_keeper { age >= 37 } else { age >= 34 }
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

        // A high-professionalism regular starter is still enjoying his
        // football — he isn't weighing retirement yet.
        let is_regular_starter = matches!(
            status,
            PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
        ) && player.happiness.starter_ratio >= 0.6;
        if is_regular_starter {
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
        let apps = (player.statistics.played + player.statistics.played_subs) as u16;
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
}
