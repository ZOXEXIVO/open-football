//! The manager's standing intention and outstanding commitments for a fixture.
//! The same read feeds the XI, bench, explanations and live cameo decisions.
use super::plan::PlannedRole;
use crate::club::player::{ManagerPromiseKind, PlayerUsage};
use crate::{Person, Player, Staff};
use chrono::NaiveDate;

#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerMatchIntent {
    pub planned_start: f32,
    pub planned_bench: f32,
    pub promised_start: f32,
    pub promised_bench: f32,
    pub loan_opportunity: f32,
    pub development: bool,
    /// Earliest sensible minute for the planned substitute appearance.
    pub cameo_from: u32,
    /// Protected debuts require a two-goal cushion; experienced players do not.
    pub minimum_lead: i32,
    pub succeeds: Option<u32>,
}

impl PlayerMatchIntent {
    pub fn assess(
        player: &Player,
        staff: &Staff,
        date: NaiveDate,
        importance: f32,
        domestic_cup: bool,
        friendly: bool,
    ) -> Self {
        if friendly {
            return Self::default();
        }
        let usage = PlayerUsage::of(player);
        let opportunity = ((0.82 - importance) / 0.62).clamp(0.0, 1.0);
        let mut intent = Self {
            cameo_from: 60,
            minimum_lead: if importance >= 0.82 { 2 } else { 0 },
            ..Self::default()
        };
        if let Some(entry) = staff.squad_plan.entry(player.id) {
            let progress = usage.since(entry.baseline);
            let (share, base) = match entry.role {
                PlannedRole::Cornerstone => (0.80, 0.65),
                PlannedRole::Starter => (0.65, 0.45),
                PlannedRole::Rotation => (0.35, 0.10),
                PlannedRole::CupKeeper => (
                    if domestic_cup { 0.8 } else { 0.05 },
                    if domestic_cup { 1.0 } else { -0.2 },
                ),
                PlannedRole::SuccessionHeir => (0.25, 0.0),
                PlannedRole::DevelopmentPathway => (0.15, 0.0),
                PlannedRole::Cover => (0.05, -0.15),
                PlannedRole::ShopWindow => (0.05, -0.30),
                PlannedRole::NotInPlans => (0.0, -0.65),
            };
            let deficit = ((f32::from(progress.eligible) * 90.0 * share - progress.minutes as f32)
                / 180.0)
                .clamp(-0.5, 1.0);
            intent.development = matches!(
                entry.role,
                PlannedRole::SuccessionHeir | PlannedRole::DevelopmentPathway
            );
            intent.succeeds = entry.succeeds;
            if intent.development {
                // Real senior minutes advance the introduction. Five short
                // cameos do not establish the player as five starts would.
                let readiness = (usage.minutes as f32 / 180.0).clamp(0.25, 1.0);
                let youth_support =
                    0.6 + staff.staff_attributes.coaching.working_with_youngsters as f32 / 25.0;
                intent.planned_start =
                    (0.35 + deficit.max(0.0)) * opportunity * readiness * youth_support;
                intent.planned_bench = (0.55 + deficit.max(0.0)) * opportunity * youth_support;
                intent.cameo_from = if usage.minutes < 45 { 70 } else { 60 };
                intent.minimum_lead = if usage.minutes < 180 { 2 } else { 1 };
            } else {
                intent.planned_start = base + deficit * opportunity;
                intent.planned_bench = base * 0.4 + deficit.max(0.0) * opportunity;
            }
        }

        if staff.squad_plan.entry(player.id).is_none()
            && player.age(date) <= 21
            && !player.is_being_moved_on()
        {
            // Academy call-ups are still rostered under their youth coach.
            // The senior manager starts with protected exposure until he
            // adopts a standing plan; their real minutes advance the stage.
            intent.development = true;
            intent.planned_bench = 0.35 * opportunity;
            intent.cameo_from = if usage.minutes < 45 { 70 } else { 60 };
            intent.minimum_lead = if usage.minutes < 180 { 2 } else { 1 };
        }

        let conscience = (staff.staff_attributes.mental.discipline as f32
            + staff.staff_attributes.mental.man_management as f32)
            / 40.0;
        let loan_target = player
            .contract_loan
            .as_ref()
            .and_then(|l| l.loan_min_appearances);
        for promise in &player.promises {
            if date < promise.made_on || promise.made_by_staff_id.is_some_and(|id| id != staff.id) {
                continue;
            }
            let Some((delivered, required)) = promise.involvement_progress(usage, loan_target)
            else {
                continue;
            };
            if delivered >= required {
                continue;
            }
            let window = (promise.deadline - promise.made_on).num_days().max(1) as f32;
            let urgency = ((date - promise.made_on).num_days() as f32 / window).clamp(0.15, 1.0);
            let deficit = f32::from(required - delivered) / f32::from(required.max(1));
            let pull = urgency * deficit * (0.45 + conscience * 0.75);
            if promise.kind == ManagerPromiseKind::StartingRole {
                intent.promised_start = intent.promised_start.max(pull);
            } else {
                intent.promised_start = intent.promised_start.max(pull * 0.7);
                intent.promised_bench = intent.promised_bench.max(pull * 0.9);
            }
        }

        // The borrower is accountable for the spell's agreed appearances,
        // even when no separate conversation promise has been recorded.
        if let Some(loan) = player.contract_loan.as_ref()
            && let (Some(start), Some(target)) = (loan.started, loan.loan_min_appearances)
            && date >= start
            && date <= loan.expiration
            && usage.appearances < target
        {
            let span = (loan.expiration - start).num_days().max(1) as f32;
            let elapsed = (date - start).num_days().max(0) as f32;
            let due = (f32::from(target) * ((elapsed + 7.0) / span).min(1.0))
                .min(f32::from(usage.eligible.saturating_add(1)));
            intent.loan_opportunity = ((due - f32::from(usage.appearances)) / 3.0).clamp(0.0, 0.8);
        }
        // Exit decisions suspend discretionary development, while formal
        // commitments retain their own bounded pressure and consequences.
        intent
    }

    pub fn start_adjustment(self) -> f32 {
        (self.planned_start + self.promised_start.max(self.loan_opportunity)).clamp(-0.8, 1.8)
    }

    pub fn bench_adjustment(self) -> f32 {
        (self.planned_bench + self.promised_bench.max(self.loan_opportunity * 0.6)).clamp(-0.4, 1.8)
    }

    /// A conditional intention, not a forced substitution. Injuries, role
    /// fit and the team's tactical needs are still checked by the live scorer.
    pub fn cameo_adjustment(
        self,
        minute: u32,
        goal_diff: i32,
        position_fit: f32,
        outgoing: u32,
    ) -> f32 {
        if minute < self.cameo_from
            || minute > 85
            || goal_diff < self.minimum_lead
            || position_fit < 0.7
        {
            return 0.0;
        }
        if self.development && self.succeeds.is_some_and(|id| id != outgoing) {
            return 0.0;
        }
        (self.bench_adjustment().max(0.0) * 0.18).min(0.25)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::staff::CoachSquadPlan;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerBuilder, PlayerClubContract, PlayerCollection,
        PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills, StaffStub,
    };
    use chrono::Duration;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()
    }
    fn player(id: u32, ability: u8, age: i32) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".into(), format!("{id}")))
            .birth_date(NaiveDate::from_ymd_opt(2026 - age, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::flat_for_ability(ability))
            .player_attributes(PlayerAttributes {
                current_ability: ability,
                potential_ability: ability,
                ..Default::default()
            })
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .contract(Some(PlayerClubContract::new(
                50_000,
                date() + Duration::days(1000),
            )))
            .build()
            .unwrap()
    }
    fn coach() -> Staff {
        let mut c = StaffStub::build();
        c.id = 7;
        c
    }
    fn read(p: &Player, c: &Staff, importance: f32) -> PlayerMatchIntent {
        PlayerMatchIntent::assess(p, c, date() + Duration::days(60), importance, false, false)
    }

    #[test]
    fn fulfilled_commitments_stop_pulling_and_loan_development_is_read() {
        let mut p = player(1, 100, 19);
        let c = coach();
        p.record_promise_full(
            ManagerPromiseKind::LoanDevelopment,
            date(),
            90,
            Some(c.id),
            Some(4),
            false,
        );
        for _ in 0..6 {
            p.on_match_overlooked(false);
        }
        let owed = read(&p, &c, 0.4);
        assert!(owed.promised_start > 0.0 && owed.promised_bench > 0.0);
        for _ in 0..4 {
            p.happiness.note_official_appearance(false);
        }
        assert_eq!(read(&p, &c, 0.4).promised_start, 0.0);
        assert_eq!(read(&p, &c, 0.4).promised_bench, 0.0);
        p.verify_promises(date() + Duration::days(91), None);
        assert!(p.promises.is_empty());
        assert!(
            p.happiness
                .recent_events
                .iter()
                .any(|e| e.event_type == crate::HappinessEventType::PromiseKept)
        );
    }

    #[test]
    fn one_start_among_omissions_does_not_fulfil_starting_role() {
        let mut p = player(1, 100, 25);
        let c = coach();
        p.record_promise_full(
            ManagerPromiseKind::StartingRole,
            date(),
            90,
            Some(c.id),
            Some(60),
            false,
        );
        p.happiness.note_official_appearance(true);
        for _ in 0..9 {
            p.on_match_overlooked(false);
        }
        assert!(read(&p, &c, 0.4).promised_start > 0.0);
        assert_eq!(read(&p, &c, 0.4).promised_bench, 0.0);
        p.verify_promises(date() + Duration::days(91), None);
        assert!(
            p.happiness
                .recent_events
                .iter()
                .any(|e| e.event_type == crate::HappinessEventType::PromiseBroken)
        );
    }

    #[test]
    fn no_eligible_fixture_keeps_promise_pending() {
        let mut p = player(1, 100, 25);
        p.record_promise_full(
            ManagerPromiseKind::StartingRole,
            date(),
            90,
            Some(7),
            Some(60),
            false,
        );
        p.verify_promises(date() + Duration::days(91), None);
        assert_eq!(p.promises.len(), 1);
    }

    #[test]
    fn promise_progress_survives_a_cold_ledger_and_counts_cup_appearances() {
        let mut p = player(1, 100, 25);
        p.statistics.played = 10;
        p.cup_statistics.played = 2;
        p.record_promise_full(
            ManagerPromiseKind::PlayingTime,
            date(),
            90,
            Some(7),
            Some(1),
            false,
        );
        p.happiness.note_official_appearance(false);
        p.happiness.official_minutes_since_join = 30;
        p.cup_statistics.played_subs += 1;
        let usage = PlayerUsage::of(&p);
        assert_eq!(usage.eligible, 1);
        assert_eq!(
            p.promises[0].involvement_progress(usage, None),
            Some((1, 1))
        );
        assert_eq!(read(&p, &coach(), 0.4).promised_bench, 0.0);
        p.happiness.note_official_non_appearance(true);
        assert_eq!(PlayerUsage::of(&p).eligible, 2);
    }

    #[test]
    fn newcomer_does_not_inherit_parent_club_minutes() {
        let mut p = player(1, 100, 19);
        p.statistics.played = 20;
        p.last_transfer_date = Some(date());
        assert_eq!(PlayerUsage::of(&p).eligible, 0);
        p.happiness.note_official_appearance(false);
        p.happiness.official_minutes_since_join = 5;
        assert_eq!(PlayerUsage::of(&p).minutes, 5);
        assert_eq!(PlayerUsage::of(&p).appearances, 1);
    }

    #[test]
    fn academy_call_up_has_a_protected_cameo_without_a_senior_roster_plan() {
        let p = player(1, 100, 19);
        let intent = read(&p, &coach(), 0.3);
        assert!(intent.development);
        assert!(intent.cameo_adjustment(75, 2, 1.0, 9) > 0.0);
        assert_eq!(intent.cameo_adjustment(75, 0, 1.0, 9), 0.0);
        assert_eq!(
            read(&p, &coach(), 0.95).cameo_adjustment(75, 2, 1.0, 9),
            0.0
        );
    }

    #[test]
    fn youth_coaching_changes_how_strongly_the_manager_pursues_his_pathway() {
        let p = player(1, 100, 19);
        let mut c = coach();
        c.squad_plan
            .force_role(p.id, PlannedRole::DevelopmentPathway, date());
        c.staff_attributes.coaching.working_with_youngsters = 1;
        let cautious = read(&p, &c, 0.3);
        c.staff_attributes.coaching.working_with_youngsters = 20;
        let supportive = read(&p, &c, 0.3);
        assert!(supportive.planned_start > cautious.planned_start);
        assert!(supportive.planned_bench > cautious.planned_bench);
    }

    #[test]
    fn new_coach_does_not_inherit_old_coachs_selection_pressure() {
        let mut p = player(1, 100, 25);
        p.record_promise_full(
            ManagerPromiseKind::PlayingTime,
            date(),
            90,
            Some(99),
            Some(6),
            false,
        );
        assert_eq!(read(&p, &coach(), 0.4).promised_start, 0.0);
    }

    #[test]
    fn loan_target_does_not_restart_with_a_new_promise() {
        let mut p = player(1, 100, 19);
        for _ in 0..8 {
            p.happiness.note_official_appearance(true);
        }
        p.record_promise_full(
            ManagerPromiseKind::LoanDevelopment,
            date(),
            90,
            Some(7),
            Some(0),
            false,
        );
        for _ in 0..2 {
            p.happiness.note_official_appearance(false);
        }
        assert_eq!(
            p.promises[0].involvement_progress(PlayerUsage::of(&p), Some(10)),
            Some((2, 2))
        );
    }

    #[test]
    fn borrowed_players_have_a_role_and_count_in_depth() {
        let mut loan = player(1, 160, 26);
        loan.contract_loan = Some(PlayerClubContract::new_loan(
            50_000,
            date() + Duration::days(180),
            10,
            11,
            20,
        ));
        let squad = PlayerCollection::new(vec![loan, player(2, 100, 27)]);
        let mut plan = CoachSquadPlan::new();
        plan.revise(&squad, date());
        assert!(matches!(
            plan.role_of(1),
            Some(PlannedRole::Cornerstone | PlannedRole::Starter)
        ));
        assert_eq!(plan.role_of(2), Some(PlannedRole::Rotation));
        let baseline = plan.entry(1).unwrap().set_on;
        plan.revise(&squad, date() + Duration::days(31));
        assert_eq!(plan.entry(1).unwrap().set_on, baseline);
    }

    #[test]
    fn loan_agreement_pressure_fades_when_actual_appearances_catch_up() {
        let mut p = player(1, 100, 19);
        p.contract_loan = Some(
            PlayerClubContract::new_loan(50_000, date() + Duration::days(120), 10, 11, 20)
                .starting_on(date())
                .with_loan_min_appearances(8),
        );
        for _ in 0..8 {
            p.on_match_overlooked(false);
        }
        assert!(read(&p, &coach(), 0.4).loan_opportunity > 0.0);
        for _ in 0..8 {
            p.happiness.note_official_appearance(false);
        }
        assert_eq!(read(&p, &coach(), 0.4).loan_opportunity, 0.0);
    }

    #[test]
    fn silent_manager_plan_guides_selection_and_real_minutes_advance_integration() {
        let mut p = player(1, 100, 19);
        let mut c = coach();
        c.staff_attributes.mental.man_management = 1;
        c.squad_plan
            .force_role(p.id, PlannedRole::DevelopmentPathway, date());
        for _ in 0..6 {
            p.on_match_overlooked(false);
        }
        let debut = read(&p, &c, 0.3);
        assert!(debut.planned_bench > debut.planned_start);
        assert_eq!(debut.cameo_adjustment(65, 3, 1.0, 2), 0.0);
        assert_eq!(debut.cameo_adjustment(75, 0, 1.0, 2), 0.0);
        assert!(debut.cameo_adjustment(75, 2, 1.0, 2) > 0.0);
        p.happiness.note_official_appearance(false);
        p.happiness.official_minutes_since_join = 5;
        assert_eq!(read(&p, &c, 0.3).minimum_lead, 2);
        p.happiness.official_minutes_since_join = 180;
        assert_eq!(read(&p, &c, 0.3).minimum_lead, 1);
        assert_eq!(read(&p, &c, 0.95).planned_start, 0.0);
        c.squad_plan
            .force_role(p.id, PlannedRole::NotInPlans, date());
        assert!(read(&p, &c, 0.3).start_adjustment() < 0.0);
    }
}
