// Shared event-context envelope (cause / severity / scope / evidence /
// follow-up + the HappinessEventContext that carries every payload).
mod context;
// Core happiness state + the event-type taxonomy.
mod model;
// Specialized context payloads, grouped by the domain they describe.
mod career;
mod manager;
mod matchday;
mod squad;
mod transfer;

pub use context::*;
pub use model::*;

pub use career::*;
pub use manager::*;
pub use matchday::*;
pub use squad::*;
pub use transfer::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// Total number of specialized payload `Option<…>` fields on
    /// [`HappinessEventContext`]. Kept in lockstep with the struct
    /// definition. If you add or remove a `*_context` field, update
    /// this constant AND the audit test below so the counter stays
    /// honest.
    const SPECIALIZED_PAYLOAD_FIELD_COUNT: usize = 27;

    #[test]
    fn specialized_payload_count_covers_every_field() {
        // Build a single context with every specialized payload
        // attached. The counter must equal the field constant — any
        // mismatch means a payload was added to the struct without
        // updating `specialized_payload_count`, and the renderer
        // dispatch / cooldown audits would silently miss it.
        //
        // We intentionally bypass `add_event_full`'s debug_assert by
        // calling the constructor directly and `with_*` builders;
        // this is a structural audit, not an emit-site check.
        let ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::Minor,
            HappinessEventScope::Personal,
        )
        .with_selection_context(MatchSelectionContext {
            scope: SelectionDecisionScope::DroppedToBench,
            reason: SelectionOmissionReason::PoorRecentForm,
            comparison: None,
            role: SelectionRole::Other,
            match_importance: 0.5,
            repeated: false,
            is_friendly: false,
        })
        .with_support_context(SupportEventContext::new(
            SupportSource::Manager,
            SupportSetting::PostMatch,
            SupportTrigger::HighRating,
        ))
        .with_transfer_interest_context(TransferInterestContext::new(
            TransferInterestStage::ConcreteInterest,
            TransferInterestSource::NationalPress,
            TransferInterestKind::StepUp,
            TransferInterestReaction::Flattered,
        ))
        .with_training_context(TrainingEventContext::new(
            TrainingEventReason::RoutineGoodSession,
            7.0,
            6.5,
        ))
        .with_manager_interaction_context(ManagerInteractionEventContext::new(
            ManagerInteractionTopic::Performance,
            ManagerInteractionTone::Honest,
            PlayerAcceptance::Accepted,
        ))
        .with_teammate_conflict_context(TeammateConflictContext::new(
            TeammateConflictReason::TrainingStandards,
            ConflictLocation::TrainingGround,
        ))
        .with_contract_context(ContractEventContext::new(
            ContractEventKind::OfferReceived,
        ))
        .with_injury_context(InjuryRecoveryEventContext::new(
            InjuryRecoveryStage::ReturnedToFullTraining,
            30,
            0.8,
        ))
        .with_match_performance_context(MatchPerformanceEventContext::new(
            MatchPerformanceKind::ChangedGameFromBench,
        ))
        .with_role_status_context(RoleStatusEventContext::new(
            RoleStatusKind::EstablishedStarter,
        ))
        .with_national_team_context(NationalTeamEventContext::new(
            NationalTeamEventKind::FirstCallup,
        ))
        .with_leadership_context(LeadershipEventContext::new(
            LeadershipEventKind::LeadershipEmergence,
        ))
        .with_media_fan_context(MediaFanEventContext::new(
            MediaFanEventKind::SocialMediaCriticism,
            MediaFanSource::SocialMedia,
        ))
        .with_personal_adaptation_context(PersonalAdaptationEventContext::new(
            PersonalAdaptationKind::SettlingIntoSquad,
            14,
        ))
        .with_career_desire_context(CareerDesireEventContext::new(
            CareerDesireKind::EuropeanCompetitionAmbition,
        ))
        .with_career_stage_context(CareerStageEventContext::new(
            CareerStageEventKind::RetirementConsidering,
        ))
        .with_loan_context(LoanEventContext::new(LoanEventKind::LoanRecallRequested))
        .with_recognition_context(RecognitionEventContext::new(
            RecognitionEventKind::PlayerOfTheWeek,
        ))
        .with_season_outcome_context(SeasonOutcomeContext::new(SeasonOutcomeKind::Relegated))
        .with_regulation_context(RegulationEventContext::new(
            RegulationOutcomeKind::Omitted,
            RegulationSlotKind::NonEuQuota,
        ))
        .with_life_simulation_desire_context(LifeSimulationDesireContext::new(
            LifeSimulationDesireKind::FamilyUnsettledAbroad,
            LifeSimulationSeverity::Moderate,
        ))
        .with_trophy_context(TrophyEventContext::new(TrophyKind::ContinentalCup))
        .with_private_talk_context(PrivateTalkRequestContext::new(
            PrivateTalkReason::PlayingTime,
        ))
        .with_club_direction_context(ClubDirectionContext::new(ClubDirectionKind::Concern))
        .with_big_match_selection_context(BigMatchSelectionContext::new(
            BigMatchKind::Derby,
            BigMatchDecision::StartedTrusted,
        ))
        .with_substitution_frustration_context(SubstitutionFrustrationContext::new(
            SubstitutionFrustrationKind::RepeatedEarlyHook,
        ))
        .with_new_signing_threat_context(NewSigningThreatContext::new(
            7,
            NewSigningThreatReason::SamePosition,
        ));

        assert_eq!(
            ctx.specialized_payload_count(),
            SPECIALIZED_PAYLOAD_FIELD_COUNT,
            "specialized_payload_count must count every Option<*Context> field; \
             if you added a payload, list it in the function AND bump \
             SPECIALIZED_PAYLOAD_FIELD_COUNT"
        );
    }

    #[test]
    fn fresh_context_carries_no_specialized_payload() {
        let ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::Minor,
            HappinessEventScope::Personal,
        );
        assert_eq!(ctx.specialized_payload_count(), 0);
    }

    #[test]
    fn each_with_specialized_context_yields_payload_count_one() {
        // Spot-check several builders — the renderer dispatch trusts
        // that exactly one specialized payload is set per event, so a
        // single with_*_context call must produce count == 1 (and not,
        // say, count == 2 due to a copy-paste bug in a future builder).
        let base = || {
            HappinessEventContext::new(
                HappinessEventCause::Other,
                HappinessEventSeverity::Minor,
                HappinessEventScope::Personal,
            )
        };
        let recognition = base().with_recognition_context(RecognitionEventContext::new(
            RecognitionEventKind::PlayerOfTheWeek,
        ));
        assert_eq!(recognition.specialized_payload_count(), 1);

        let season = base()
            .with_season_outcome_context(SeasonOutcomeContext::new(SeasonOutcomeKind::Relegated));
        assert_eq!(season.specialized_payload_count(), 1);

        let regulation = base().with_regulation_context(RegulationEventContext::new(
            RegulationOutcomeKind::Omitted,
            RegulationSlotKind::NonEuQuota,
        ));
        assert_eq!(regulation.specialized_payload_count(), 1);
    }

    #[test]
    fn double_attached_specialized_payload_count_exceeds_one() {
        // If a future emit site (or a careless refactor) attaches two
        // specialized contexts, `specialized_payload_count` must return
        // >1 so the debug_assert in `add_event_full` catches it. This
        // test pins that the counter actually counts each payload, so
        // the runtime guard remains effective.
        let ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::Minor,
            HappinessEventScope::Personal,
        )
        .with_recognition_context(RecognitionEventContext::new(
            RecognitionEventKind::PlayerOfTheWeek,
        ))
        .with_season_outcome_context(SeasonOutcomeContext::new(SeasonOutcomeKind::Relegated));
        assert_eq!(ctx.specialized_payload_count(), 2);
    }

    #[test]
    #[should_panic(expected = "specialized payloads")]
    fn add_event_with_double_specialized_payload_panics_in_debug() {
        // The debug_assert in add_event_full fires when an emit site
        // attaches two specialized payloads. Tests run with
        // debug_assertions enabled, so this should panic. In release
        // builds the event is still recorded (best-effort), but the
        // mutually-exclusive contract is enforced under tests.
        let mut h = PlayerHappiness::new();
        let bad_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::Media,
        )
        .with_recognition_context(RecognitionEventContext::new(
            RecognitionEventKind::PlayerOfTheMonth,
        ))
        .with_regulation_context(RegulationEventContext::new(
            RegulationOutcomeKind::Omitted,
            RegulationSlotKind::Other,
        ));
        h.add_event_with_context(HappinessEventType::PlayerOfTheMonth, 5.0, None, bad_ctx);
    }

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

    #[test]
    fn severity_thresholds_are_stable() {
        // Boundary checks — keep these in lockstep with renderer copy
        // and tests that assert the visible label.
        assert_eq!(
            HappinessEventSeverity::from_magnitude(0.5),
            HappinessEventSeverity::Minor
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(1.9),
            HappinessEventSeverity::Minor
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(2.0),
            HappinessEventSeverity::Moderate
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(-3.5),
            HappinessEventSeverity::Moderate
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(4.0),
            HappinessEventSeverity::Serious
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(-5.9),
            HappinessEventSeverity::Serious
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(6.0),
            HappinessEventSeverity::Major
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(-12.0),
            HappinessEventSeverity::Major
        );
    }

    #[test]
    fn legacy_emit_paths_carry_no_context() {
        let mut h = PlayerHappiness::new();
        h.add_event(HappinessEventType::PoorTraining, -1.0);
        let event = h.recent_events.last().unwrap();
        assert!(
            event.context.is_none(),
            "legacy emit must not synthesise a context — None means 'unknown', \
             which the renderer falls back from cleanly"
        );
    }

    #[test]
    fn add_event_with_context_round_trips() {
        let mut h = PlayerHappiness::new();
        let ctx = HappinessEventContext::new(
            HappinessEventCause::PositionalRivalry,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::DressingRoom,
        )
        .with_relationship_level(-30.0)
        .with_follow_up(HappinessEventFollowUp::DressingRoomDamageRisk);
        h.add_event_with_context(
            HappinessEventType::ConflictWithTeammate,
            -2.0,
            Some(99),
            ctx,
        );
        let event = h.recent_events.last().unwrap();
        let stored = event.context.as_ref().expect("context must round-trip");
        assert_eq!(stored.cause, HappinessEventCause::PositionalRivalry);
        assert_eq!(stored.severity, HappinessEventSeverity::Moderate);
        assert_eq!(stored.scope, HappinessEventScope::DressingRoom);
        assert_eq!(stored.relationship_level_before, Some(-30.0));
        assert_eq!(
            stored.follow_up,
            Some(HappinessEventFollowUp::DressingRoomDamageRisk)
        );
        assert_eq!(event.partner_player_id, Some(99));
    }

    #[test]
    fn partner_required_event_without_partner_is_dropped() {
        let mut h = PlayerHappiness::new();
        // debug_assertions panic in test builds — wrap in catch_unwind so
        // we can assert that the event is not silently committed.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let ctx = HappinessEventContext::new(
                HappinessEventCause::PersonalityClash,
                HappinessEventSeverity::Minor,
                HappinessEventScope::DressingRoom,
            );
            h.add_event_with_context(HappinessEventType::ConflictWithTeammate, -1.0, None, ctx);
        }));
        assert!(
            result.is_err() || h.recent_events.is_empty(),
            "partner-required event without partner_id must not land in recent_events"
        );
    }

    #[test]
    fn match_selection_context_round_trips_through_event_context() {
        let mut h = PlayerHappiness::new();
        let sel = MatchSelectionContext {
            scope: SelectionDecisionScope::DroppedToBench,
            reason: SelectionOmissionReason::TeammatePreferredOnFitness,
            comparison: Some(SelectionComparison {
                selected_player_id: 42,
                selected_was_starter: true,
                slot: Some(SelectionRole::Winger),
                selected_score: 14.5,
                omitted_score: 12.0,
                top_factors: vec![SelectionScoreFactor::MatchReadiness],
            }),
            role: SelectionRole::Winger,
            match_importance: 0.8,
            repeated: false,
            is_friendly: false,
        };
        let ctx = HappinessEventContext::new(
            HappinessEventCause::PositionalRivalry,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::MatchDay,
        )
        .with_selection_context(sel);
        h.add_event_with_context(HappinessEventType::MatchDropped, -2.0, None, ctx);

        let event = h.recent_events.last().expect("event must land");
        let stored = event
            .context
            .as_ref()
            .and_then(|c| c.selection_context.as_ref())
            .expect("selection context round-trips");
        assert_eq!(stored.scope, SelectionDecisionScope::DroppedToBench);
        assert_eq!(
            stored.reason,
            SelectionOmissionReason::TeammatePreferredOnFitness
        );
        let comp = stored.comparison.as_ref().expect("comparison present");
        assert_eq!(comp.selected_player_id, 42);
        assert!(comp.selected_was_starter);
        assert_eq!(comp.slot, Some(SelectionRole::Winger));
    }

    #[test]
    fn selection_omission_reason_keys_are_unique_and_non_empty() {
        let reasons = [
            SelectionOmissionReason::LowerMatchReadiness,
            SelectionOmissionReason::FitnessProtection,
            SelectionOmissionReason::FatigueManagement,
            SelectionOmissionReason::PoorRecentForm,
            SelectionOmissionReason::TacticalMismatch,
            SelectionOmissionReason::PositionFitIssue,
            SelectionOmissionReason::TeammatePreferredOnAbility,
            SelectionOmissionReason::TeammatePreferredOnForm,
            SelectionOmissionReason::TeammatePreferredOnFitness,
            SelectionOmissionReason::TeammatePreferredOnTrust,
            SelectionOmissionReason::TeammatePreferredForTacticalBalance,
            SelectionOmissionReason::YouthDevelopmentRotation,
            SelectionOmissionReason::CupRotation,
            SelectionOmissionReason::LowMatchImportanceRotation,
            SelectionOmissionReason::SquadStatusMismatch,
            SelectionOmissionReason::ManagerDoesNotTrustPlayer,
            SelectionOmissionReason::NewcomerStillIntegrating,
            SelectionOmissionReason::ReturningFromInjury,
            SelectionOmissionReason::DisciplinarySelection,
            SelectionOmissionReason::BenchBalance,
            SelectionOmissionReason::NoNaturalRoleInFormation,
        ];
        let mut keys: Vec<&'static str> = reasons.iter().map(|r| r.as_i18n_key()).collect();
        keys.sort();
        let unique = {
            let mut k = keys.clone();
            k.dedup();
            k.len()
        };
        assert_eq!(keys.len(), unique, "reason keys must be unique");
        for k in &keys {
            assert!(!k.is_empty(), "reason i18n key must be non-empty");
            assert!(
                k.starts_with("selection_reason_"),
                "reason key {} must follow the naming convention",
                k
            );
        }
    }

    #[test]
    fn partner_aware_cooldown_is_per_partner() {
        let mut h = PlayerHappiness::new();
        let ctx = HappinessEventContext::new(
            HappinessEventCause::TrainingFriction,
            HappinessEventSeverity::Minor,
            HappinessEventScope::TrainingGround,
        );
        let added_first = h.add_event_with_partner_context_and_cooldown(
            HappinessEventType::ConflictWithTeammate,
            -1.0,
            7,
            ctx.clone(),
            45,
        );
        assert!(added_first);
        // Same partner inside cooldown — blocked.
        let added_again = h.add_event_with_partner_context_and_cooldown(
            HappinessEventType::ConflictWithTeammate,
            -1.0,
            7,
            ctx.clone(),
            45,
        );
        assert!(!added_again, "same partner inside cooldown must be blocked");
        // Different partner — should land.
        let added_other = h.add_event_with_partner_context_and_cooldown(
            HappinessEventType::ConflictWithTeammate,
            -1.0,
            42,
            ctx,
            45,
        );
        assert!(
            added_other,
            "cooldown must be keyed per-partner so a new teammate's first incident is recorded"
        );
    }
}
