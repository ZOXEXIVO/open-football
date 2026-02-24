use crate::club::player::player::Player;
use crate::club::{PlayerMailbox, PlayerResult, PlayerStatusType};
use crate::utils::DateUtils;
use chrono::{NaiveDate, NaiveDateTime};

impl Player {
    pub(crate) fn process_contract(&mut self, result: &mut PlayerResult, now: NaiveDateTime) {
        if let Some(ref mut contract) = self.contract {
            const HALF_YEAR_DAYS: i64 = 30 * 6;

            if contract.days_to_expiration(now) < HALF_YEAR_DAYS {
                result.contract.want_extend_contract = true;
            }
        } else {
            result.contract.no_contract = true;
        }
    }

    pub(crate) fn process_mailbox(&mut self, result: &mut PlayerResult, now: NaiveDate) {
        PlayerMailbox::process(self, result, now);
    }

    /// Transfer desire based on multiple factors, not just behaviour
    pub(crate) fn process_transfer_desire(&mut self, result: &mut PlayerResult, now: NaiveDate) {
        // Under-16 players cannot request transfers — only free release
        let age = DateUtils::age(self.birth_date, now);
        if age < 16 {
            return;
        }

        let mut wants_transfer = false;

        // Existing logic: poor behaviour
        if self.behaviour.is_poor() {
            wants_transfer = true;
        }

        // Unhappy for extended period (check if Unh status exists)
        let has_unh = self.statuses.statuses.iter().any(|s| {
            s.status == PlayerStatusType::Unh && (now - s.start_date).num_days() > 30
        });
        if has_unh {
            wants_transfer = true;
        }

        // Overall unhappiness
        if !self.happiness.is_happy() && self.behaviour.is_poor() {
            wants_transfer = true;
        }

        if wants_transfer {
            // Set Req (transfer request) status
            if !self.statuses.get().contains(&PlayerStatusType::Req) {
                self.statuses.add(now, PlayerStatusType::Req);
            }
            result.wants_to_leave = true;
            result.request_transfer(self.id);
        }
    }
}
