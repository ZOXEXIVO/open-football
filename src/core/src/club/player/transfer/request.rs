//! A transfer request is a decision, not a mood. Reasons have to hold before
//! a man makes one, and once made it stands until he moves or the reasons
//! have been gone for a while, so what the market acts on is what he still
//! wants.

use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::core::player::{Player, TransferRequestReason};
use crate::club::player::transfer::processing::UNHAPPY_LISTING_MIN_DAYS;
use crate::transfers::market::window::AGREEMENT_PRE_OPEN_DAYS;
use crate::{HappinessEventType, PlayerStatusType};
use chrono::{Duration, NaiveDate};

/// When his reasons began and when they last held.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransferRequestStanding {
    reasons_since: Option<NaiveDate>,
    last_reason_on: Option<NaiveDate>,
    refused_talk: Option<ManagerGrievance>,
    fell_out: Option<ManagerGrievance>,
    /// Windows that have shut on this request with nobody coming for him.
    windows_unanswered: u8,
    /// He has let a request drop, and will not make another before this.
    let_go_until: Option<NaiveDate>,
}

/// Something his manager did that he means to leave over. Only a window
/// can answer it, so he holds it until one is near and is done with it
/// once that window has shut, or once the man himself has gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ManagerGrievance {
    against: u32,
    stage: GrievanceStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrievanceStage {
    Waiting,
    /// The window it waits for is near or open.
    Due,
}

impl ManagerGrievance {
    fn against(manager: u32) -> Self {
        ManagerGrievance {
            against: manager,
            stage: GrievanceStage::Waiting,
        }
    }

    /// `held`, read `days_to_window` before the next registration window
    /// opens. `None` once the window it was due in has shut.
    fn on_window_clock(held: Option<Self>, days_to_window: Option<u16>) -> Option<Self> {
        // He asks once clubs may agree terms for him.
        let near = days_to_window.is_some_and(|days| i64::from(days) <= AGREEMENT_PRE_OPEN_DAYS);
        let held = held?;
        match (held.stage, near) {
            (_, true) => Some(ManagerGrievance {
                stage: GrievanceStage::Due,
                ..held
            }),
            (GrievanceStage::Waiting, false) => Some(held),
            (GrievanceStage::Due, false) => None,
        }
    }

    fn is_due(held: Option<Self>) -> bool {
        held.is_some_and(|grievance| grievance.stage == GrievanceStage::Due)
    }
}

/// What today's reasons mean for his request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferRequestStep {
    /// No request, and nothing to make one with.
    Nothing,
    /// Reasons are holding, not yet for long enough to ask.
    Waiting,
    /// They have held for long enough: he asks.
    Make,
    /// He has asked, and it stands.
    Stands,
    /// His reasons have been gone for long enough: he withdraws it.
    Withdraw,
}

impl TransferRequestStanding {
    pub(crate) const DAYS_TO_MAKE: i64 = 14;
    const DAYS_TO_WITHDRAW: i64 = 30;
    /// Windows a man who is playing gives the market to answer him …
    const WINDOWS_HE_WAITS: u8 = 2;
    /// … and how long it is before he tries again.
    const DAYS_BEFORE_HE_ASKS_AGAIN: i64 = 365;

    pub(crate) fn step(
        &mut self,
        has_reason: bool,
        requested: bool,
        today: NaiveDate,
    ) -> TransferRequestStep {
        if has_reason {
            if !requested && self.let_go_until.is_some_and(|until| today < until) {
                return TransferRequestStep::Waiting;
            }
            let since = *self.reasons_since.get_or_insert(today);
            self.last_reason_on = Some(today);
            if requested {
                TransferRequestStep::Stands
            } else if (today - since).num_days() >= Self::DAYS_TO_MAKE {
                TransferRequestStep::Make
            } else {
                TransferRequestStep::Waiting
            }
        } else if requested {
            let last = *self.last_reason_on.get_or_insert(today);
            if (today - last).num_days() >= Self::DAYS_TO_WITHDRAW {
                TransferRequestStep::Withdraw
            } else {
                TransferRequestStep::Stands
            }
        } else {
            self.reasons_since = None;
            self.last_reason_on = None;
            TransferRequestStep::Nothing
        }
    }

    fn on_window_clock(&mut self, days_to_window: Option<u16>) {
        self.refused_talk = ManagerGrievance::on_window_clock(self.refused_talk, days_to_window);
        self.fell_out = ManagerGrievance::on_window_clock(self.fell_out, days_to_window);
    }

    pub(crate) fn on_manager_left(&mut self, manager: u32) {
        self.refused_talk = self.refused_talk.filter(|g| g.against != manager);
        self.fell_out = self.fell_out.filter(|g| g.against != manager);
    }

    /// The request is over. What his manager did is not: it waits for its
    /// own window.
    fn closed(&self, let_go_until: Option<NaiveDate>) -> Self {
        TransferRequestStanding {
            refused_talk: self.refused_talk,
            fell_out: self.fell_out,
            let_go_until,
            ..TransferRequestStanding::default()
        }
    }

    fn made_at_once(&mut self, today: NaiveDate) {
        self.reasons_since.get_or_insert(today);
        self.last_reason_on = Some(today);
    }
}

impl Player {
    /// Long enough to fire once per window close, never twice inside one.
    const UNSOLD_MOOD_COOLDOWN_DAYS: u16 = 100;

    /// His manager turned down his playing-time talk. Refused to his face,
    /// he means to go, and asks when a window can answer it.
    pub fn on_playing_time_talk_refused(&mut self, manager: u32) {
        self.transfer_request
            .refused_talk
            .get_or_insert(ManagerGrievance::against(manager));
    }

    /// Friction with his manager has come to a head.
    pub fn on_fell_out_with_manager(&mut self, manager: u32) {
        self.transfer_request
            .fell_out
            .get_or_insert(ManagerGrievance::against(manager));
    }

    /// He has had it out with his manager and is waiting for a window to
    /// leave in.
    pub fn waits_on_the_window(&self) -> bool {
        self.transfer_request.refused_talk.is_some() || self.transfer_request.fell_out.is_some()
    }

    /// The reasons his grievances with the manager give him today. Each
    /// asks at once, because the wait was for the window. A refusal was
    /// about minutes, so it is a reason only while he is still starting
    /// less often than he expects to.
    pub(crate) fn answer_manager_grievances(
        &mut self,
        today: NaiveDate,
        days_to_window: Option<u16>,
    ) -> Vec<TransferRequestReason> {
        self.transfer_request.on_window_clock(days_to_window);
        let mut reasons = Vec::new();
        if ManagerGrievance::is_due(self.transfer_request.refused_talk)
            && self.is_short_of_football(today)
        {
            reasons.push(TransferRequestReason::RefusedPlayingTime);
        }
        if ManagerGrievance::is_due(self.transfer_request.fell_out) {
            reasons.push(TransferRequestReason::FellOutWithManager);
        }
        for reason in &reasons {
            self.ask_to_leave_at_once(today, *reason);
        }
        reasons
    }

    pub(crate) fn is_short_of_football(&self, today: NaiveDate) -> bool {
        let status = self.contract.as_ref().map(|c| &c.squad_status);
        self.seen_start_share() < self.own_expected_start_share(status, today)
    }

    /// Has he asked for nothing but a bigger stage? Reasons he has not
    /// asked over yet are no request.
    pub fn asks_only_for_a_bigger_stage(&self) -> bool {
        self.statuses.has(PlayerStatusType::Req)
            && !self.transfer_request_reasons.is_empty()
            && self
                .transfer_request_reasons
                .iter()
                .all(|reason| reason.asks_for_a_bigger_stage())
    }

    /// Whether he is asking to go: a formal request, or an unhappiness that
    /// has hardened for as long as the club had to fix it.
    pub fn is_asking_to_leave(&self, today: NaiveDate) -> bool {
        self.statuses.has(PlayerStatusType::Req)
            || self
                .statuses
                .held_for_days(PlayerStatusType::Unh, today)
                .is_some_and(|days| days >= UNHAPPY_LISTING_MIN_DAYS)
    }

    /// The listing he asked for has come down because he stopped asking. He
    /// goes back to being a player the club has not put on the market: no
    /// badge, no sale flag, nothing a buyer or a renewal would read.
    pub fn on_listing_withdrawn(&mut self, today: NaiveDate) {
        self.come_off_the_list();
        self.decision_history.add(
            today,
            "dec_listing_withdrawn".to_string(),
            "dec_reason_no_longer_wants_to_leave".to_string(),
            String::new(),
        );
    }

    /// The board looked at a listing the market had refused, at a man it
    /// has gone on picking, and thought again. He is a renewal like anybody
    /// else from here.
    pub fn on_kept_by_club(&mut self, today: NaiveDate) {
        self.come_off_the_list();
        if let Some(plan) = self.plan.as_mut() {
            plan.asking_multiple = None;
        }
        self.decision_history.add(
            today,
            "dec_listing_withdrawn".to_string(),
            "dec_reason_kept_in_the_side".to_string(),
            "dec_decided_board".to_string(),
        );
    }

    fn come_off_the_list(&mut self) {
        self.statuses.remove(PlayerStatusType::Lst);
        self.clear_availability_state();
        if let Some(contract) = self.contract.as_mut() {
            contract.is_transfer_listed = false;
        }
    }

    /// A window has shut with him still for sale. A man who is playing every
    /// week and has given the market its chance lets his request drop: he
    /// still wants what he wanted, and a buyer can still come, but nobody is
    /// made to read him as leaving, and he can sign a new deal.
    pub(crate) fn on_window_closed_unsold(&mut self, today: NaiveDate) {
        self.happiness.add_event_with_cooldown(
            HappinessEventType::UnsoldWindowClosed,
            HappinessConfig::default().catalog.unsold_window_closed,
            Self::UNSOLD_MOOD_COOLDOWN_DAYS,
        );
        if !self.statuses.has(PlayerStatusType::Req) {
            return;
        }
        self.transfer_request.windows_unanswered =
            self.transfer_request.windows_unanswered.saturating_add(1);
        if self.transfer_request.windows_unanswered < TransferRequestStanding::WINDOWS_HE_WAITS
            || !self.is_picked(today)
        {
            return;
        }
        self.statuses.remove(PlayerStatusType::Req);
        self.transfer_request_reasons.clear();
        self.transfer_request = self.transfer_request.closed(Some(
            today + Duration::days(TransferRequestStanding::DAYS_BEFORE_HE_ASKS_AGAIN),
        ));
        self.decision_history.add(
            today,
            "dec_transfer_request_withdrawn".to_string(),
            "dec_reason_nobody_came_for_him".to_string(),
            String::new(),
        );
    }

    fn ask_to_leave_at_once(&mut self, today: NaiveDate, reason: TransferRequestReason) {
        self.transfer_request.made_at_once(today);
        if !self.transfer_request_reasons.contains(&reason) {
            self.transfer_request_reasons.push(reason);
        }
        if !self.statuses.has(PlayerStatusType::Req) {
            self.statuses.add(today, PlayerStatusType::Req);
        }
    }

    /// What today's reasons do to his request.
    pub(crate) fn answer_transfer_reasons(
        &mut self,
        reasons: Vec<TransferRequestReason>,
        today: NaiveDate,
    ) {
        let requested = self.statuses.has(PlayerStatusType::Req);
        let step = self
            .transfer_request
            .step(!reasons.is_empty(), requested, today);
        match step {
            TransferRequestStep::Make => self.statuses.add(today, PlayerStatusType::Req),
            TransferRequestStep::Withdraw => {
                self.statuses.remove(PlayerStatusType::Req);
                self.transfer_request = self.transfer_request.closed(None);
                self.decision_history.add(
                    today,
                    "dec_transfer_request_withdrawn".to_string(),
                    "dec_reason_no_longer_wants_to_leave".to_string(),
                    String::new(),
                );
            }
            TransferRequestStep::Nothing
            | TransferRequestStep::Waiting
            | TransferRequestStep::Stands => {}
        }
        // A standing request keeps the last reasons he gave for it.
        if !reasons.is_empty() || !self.statuses.has(PlayerStatusType::Req) {
            self.transfer_request_reasons = reasons;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };

    const COACH: u32 = 40;

    fn day(month: u32, date: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2030, month, date).unwrap()
    }

    fn player() -> Player {
        PlayerBuilder::new()
            .id(7)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2003, 3, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .player_attributes(PlayerAttributes::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 15,
                }],
            })
            .build()
            .unwrap()
    }

    /// `reason` on the days `fires` says, one call a day from `from`.
    fn run(p: &mut Player, from: NaiveDate, days: i64, fires: impl Fn(i64) -> bool) -> usize {
        let mut requests_made = 0;
        for offset in 0..days {
            let had = p.statuses.has(PlayerStatusType::Req);
            let reasons = if fires(offset) {
                vec![TransferRequestReason::OutgrownClub]
            } else {
                Vec::new()
            };
            p.answer_transfer_reasons(reasons, from + Duration::days(offset));
            if !had && p.statuses.has(PlayerStatusType::Req) {
                requests_made += 1;
            }
        }
        requests_made
    }

    /// The daily tick from `from`, with the next window `days_to_window`
    /// days off on each day.
    fn live_through(
        p: &mut Player,
        from: NaiveDate,
        days: i64,
        days_to_window: impl Fn(i64) -> i64,
    ) {
        for offset in 0..days {
            let today = from + Duration::days(offset);
            let reasons = p.answer_manager_grievances(today, Some(days_to_window(offset) as u16));
            p.answer_transfer_reasons(reasons, today);
        }
    }

    fn asking_and_playing(seen: f32) -> Player {
        let mut p = player();
        p.statuses.add(day(1, 1), PlayerStatusType::Req);
        p.transfer_request_reasons = vec![TransferRequestReason::OutgrownClub];
        p.happiness.starter_ratio = seen;
        p.happiness.appearances_tracked = 40;
        p
    }

    #[test]
    fn a_man_in_the_side_lets_his_request_drop_after_two_windows() {
        let mut p = asking_and_playing(0.9);
        p.on_window_closed_unsold(day(2, 1));
        assert!(
            p.statuses.has(PlayerStatusType::Req),
            "one window is no answer"
        );
        p.on_window_closed_unsold(day(9, 1));
        assert!(!p.statuses.has(PlayerStatusType::Req));
        assert!(p.transfer_request_reasons.is_empty());
    }

    #[test]
    fn a_man_out_of_the_side_goes_on_asking() {
        let mut p = asking_and_playing(0.1);
        for month in 2..8 {
            p.on_window_closed_unsold(day(month, 1));
        }
        assert!(p.statuses.has(PlayerStatusType::Req));
    }

    #[test]
    fn he_does_not_ask_again_the_day_after_he_let_it_go() {
        let mut p = asking_and_playing(0.9);
        p.on_window_closed_unsold(day(2, 1));
        p.on_window_closed_unsold(day(3, 1));
        assert!(!p.statuses.has(PlayerStatusType::Req));

        let made = run(&mut p, day(3, 2), 300, |_| true);
        assert_eq!(made, 0, "the same reasons, inside the year");

        let made = run(&mut p, day(3, 2) + Duration::days(370), 30, |_| true);
        assert_eq!(made, 1, "and the year after he tries again");
    }

    #[test]
    fn falling_out_with_his_manager_does_not_wait_for_the_year() {
        let mut p = asking_and_playing(0.9);
        p.on_window_closed_unsold(day(2, 1));
        p.on_window_closed_unsold(day(3, 1));
        p.on_fell_out_with_manager(COACH);
        live_through(&mut p, day(4, 1), 1, |_| 0);
        assert!(p.statuses.has(PlayerStatusType::Req));
    }

    #[test]
    fn a_man_the_board_keeps_is_a_renewal_like_anybody_else() {
        let mut p = player();
        p.contract = Some(PlayerClubContract::new(
            1_000,
            day(1, 1) + Duration::days(900),
        ));
        p.on_listed_by_club("dec_reason_surplus_squad", day(1, 1));
        p.on_kept_by_club(day(2, 1));
        assert!(!p.statuses.has(PlayerStatusType::Lst));
        assert!(!p.contract.as_ref().unwrap().is_transfer_listed);
        assert_eq!(
            p.decision_history.items.last().unwrap().decision,
            "dec_reason_kept_in_the_side"
        );
    }

    #[test]
    fn a_one_day_reason_makes_no_request() {
        let mut p = player();
        let made = run(&mut p, day(1, 1), 60, |offset| offset == 0);
        assert_eq!(made, 0);
        assert!(!p.statuses.has(PlayerStatusType::Req));
        assert!(p.contract.is_none() || !p.contract.as_ref().unwrap().is_transfer_listed);
    }

    #[test]
    fn a_reason_has_to_hold_without_a_break() {
        let mut p = player();
        let made = run(&mut p, day(1, 1), 60, |offset| offset % 10 != 9);
        assert_eq!(made, 0, "ten days on, a day off, never a fortnight");
    }

    #[test]
    fn a_fortnight_of_reasons_makes_a_request() {
        let mut p = player();
        assert_eq!(run(&mut p, day(1, 1), 14, |_| true), 0, "not yet");
        assert_eq!(run(&mut p, day(1, 15), 1, |_| true), 1);
    }

    #[test]
    fn a_flickering_reason_is_one_request() {
        let mut p = player();
        let held = 15;
        let made = run(&mut p, day(1, 1), 120, |offset| {
            offset < held || (offset - held) % 25 < 2
        });
        assert_eq!(made, 1, "never gone for thirty days, so it never lapsed");
        assert!(p.statuses.has(PlayerStatusType::Req));
    }

    #[test]
    fn a_request_lapses_with_a_record() {
        let mut p = player();
        run(&mut p, day(1, 1), 15, |_| true);
        assert!(p.statuses.has(PlayerStatusType::Req));

        run(&mut p, day(1, 16), 29, |_| false);
        assert!(p.statuses.has(PlayerStatusType::Req), "twenty-nine days on");
        assert!(!p.transfer_request_reasons.is_empty(), "and still says why");

        run(&mut p, day(2, 14), 2, |_| false);
        assert!(!p.statuses.has(PlayerStatusType::Req));
        assert!(p.transfer_request_reasons.is_empty());
        let record = p.decision_history.items.last().unwrap();
        assert_eq!(record.movement, "dec_transfer_request_withdrawn");
        assert_eq!(record.date, day(2, 14));
    }

    /// Refused on 17 February, with the summer window 104 days off.
    #[test]
    fn a_refusal_waits_for_the_window() {
        let mut p = player();
        p.on_playing_time_talk_refused(COACH);
        let opens = 104;
        live_through(&mut p, day(2, 17), opens - 15, |offset| opens - offset);
        assert!(
            !p.statuses.has(PlayerStatusType::Req),
            "nothing could answer it yet"
        );
        assert!(p.waits_on_the_window());

        live_through(&mut p, day(2, 17) + Duration::days(opens - 15), 1, |_| 14);
        assert!(p.statuses.has(PlayerStatusType::Req));
        assert_eq!(
            p.transfer_request_reasons,
            vec![TransferRequestReason::RefusedPlayingTime]
        );
    }

    #[test]
    fn a_refusal_is_spent_once_its_window_shuts() {
        let mut p = player();
        p.on_playing_time_talk_refused(COACH);
        live_through(&mut p, day(1, 1), 31, |_| 0);
        assert!(
            p.statuses.has(PlayerStatusType::Req),
            "it stands in the window"
        );

        live_through(&mut p, day(2, 1), 29, |offset| 130 - offset);
        assert!(!p.waits_on_the_window());
        assert!(p.statuses.has(PlayerStatusType::Req), "twenty-nine days on");

        live_through(&mut p, day(3, 2), 1, |_| 101);
        assert!(!p.statuses.has(PlayerStatusType::Req));
        assert_eq!(
            p.decision_history.items.last().unwrap().movement,
            "dec_transfer_request_withdrawn"
        );
    }

    #[test]
    fn a_man_back_in_the_side_by_the_window_does_not_ask() {
        let mut p = player();
        p.on_playing_time_talk_refused(COACH);
        p.happiness.starter_ratio = 0.9;
        p.happiness.appearances_tracked = 40;
        live_through(&mut p, day(1, 1), 31, |_| 0);
        assert!(!p.statuses.has(PlayerStatusType::Req));

        live_through(&mut p, day(2, 1), 1, |_| 130);
        assert!(!p.waits_on_the_window(), "the window took it with it");
    }

    #[test]
    fn a_falling_out_is_a_request_with_a_reason() {
        let mut p = player();
        p.on_fell_out_with_manager(COACH);
        p.happiness.starter_ratio = 0.9;
        p.happiness.appearances_tracked = 40;
        assert_eq!(
            p.answer_manager_grievances(day(1, 5), Some(0)),
            vec![TransferRequestReason::FellOutWithManager],
            "it is about the manager, not his minutes"
        );
        assert!(p.statuses.has(PlayerStatusType::Req));
        assert!(p.answer_manager_grievances(day(2, 5), Some(120)).is_empty());
    }

    #[test]
    fn a_grievance_goes_with_the_manager() {
        let mut p = player();
        p.on_fell_out_with_manager(COACH);
        p.on_manager_left(COACH + 1, 1, day(1, 1));
        assert!(p.waits_on_the_window(), "somebody else left");

        p.on_manager_left(COACH, 1, day(1, 2));
        assert!(!p.waits_on_the_window());
        live_through(&mut p, day(1, 3), 1, |_| 0);
        assert!(!p.statuses.has(PlayerStatusType::Req));
    }

    #[test]
    fn a_lapsed_request_leaves_a_refusal_waiting() {
        let mut p = player();
        run(&mut p, day(1, 1), 15, |_| true);
        p.on_playing_time_talk_refused(COACH);
        run(&mut p, day(1, 16), 31, |_| false);
        assert!(!p.statuses.has(PlayerStatusType::Req), "the request lapsed");
        assert!(p.waits_on_the_window(), "the refusal did not");

        live_through(&mut p, day(3, 1), 1, |_| 14);
        assert_eq!(
            p.transfer_request_reasons,
            vec![TransferRequestReason::RefusedPlayingTime]
        );
    }

    #[test]
    fn reasons_he_has_not_asked_over_are_no_request_for_a_bigger_stage() {
        let mut p = player();
        run(&mut p, day(1, 1), 5, |_| true);
        assert!(!p.statuses.has(PlayerStatusType::Req));
        assert!(!p.asks_only_for_a_bigger_stage());

        run(&mut p, day(1, 6), 10, |_| true);
        assert!(p.statuses.has(PlayerStatusType::Req));
        assert!(p.asks_only_for_a_bigger_stage());
    }

    #[test]
    fn every_request_says_why() {
        let mut p = player();
        p.on_playing_time_talk_refused(COACH);
        live_through(&mut p, day(1, 1), 1, |_| 0);
        let mut q = player();
        run(&mut q, day(1, 1), 16, |_| true);
        run(&mut q, day(1, 17), 10, |_| false);
        for man in [&p, &q] {
            assert!(man.statuses.has(PlayerStatusType::Req));
            assert!(!man.transfer_request_reasons.is_empty());
        }
    }
}
