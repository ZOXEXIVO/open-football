use crate::club::player::player::Player;
use crate::club::{PlayerMailbox, PlayerResult, PlayerStatusType};
use crate::utils::DateUtils;
use chrono::{NaiveDate, NaiveDateTime};

impl Player {
    pub(crate) fn process_contract(&mut self, result: &mut PlayerResult, now: NaiveDateTime) {
        // Snapshot threshold inputs before borrowing contract mutably.
        let career_apps = (self.statistics.played as u32) + (self.statistics.played_subs as u32);
        let caps = self.player_attributes.international_apps;

        if let Some(ref mut contract) = self.contract {
            const ONE_YEAR_DAYS: i64 = 365;

            if contract.days_to_expiration(now) < ONE_YEAR_DAYS {
                // For loaned players this signals the parent club to renew remotely
                result.contract.want_extend_contract = true;
            }

            // Yearly wage-rise clause: applies on the contract anniversary.
            // The helper guards idempotency via a per-year memo so a
            // same-day double tick won't double-apply.
            let _ = contract.try_apply_yearly_wage_rise(now.date());

            // Threshold clauses fire as soon as the daily count crosses
            // the negotiated threshold. Each helper consumes its clause
            // on success so the next tick is a cheap no-op.
            let _ = contract.try_apply_wage_after_career_apps(career_apps);
            let _ = contract.try_apply_wage_after_caps(caps);

            // Final-season auto-extension: triggers once the contract is
            // inside its last 365 days and the player has crossed the
            // appearance threshold this season. Helper handles both
            // gates internally.
            let season_apps = self.statistics.played + self.statistics.played_subs;
            let _ = contract.try_apply_appearance_extension(season_apps, now.date());
        } else if !self.is_on_loan() {
            result.contract.no_contract = true;
        }
    }

    pub(crate) fn process_mailbox(&mut self, result: &mut PlayerResult, now: NaiveDate) {
        PlayerMailbox::process(self, result, now);
    }

    /// Transfer desire based on multiple factors, not just behaviour
    pub(crate) fn process_transfer_desire(&mut self, result: &mut PlayerResult, now: NaiveDate) {
        // Loaned players belong to their parent club — they cannot request
        // transfers or be unhappy with salary at the loan club
        if self.is_on_loan() {
            return;
        }

        // Under-16 players cannot request transfers — only free release
        let age = DateUtils::age(self.birth_date, now);
        if age < 16 {
            return;
        }

        // Honeymoon: newly-transferred players don't fire off a request in
        // the first 21 days regardless of shock events — they need a fair
        // look first (unless behaviour is already broken).
        let recently_transferred = self
            .days_since_transfer(now)
            .map(|d| d >= 0 && d < 21)
            .unwrap_or(false);

        let mut wants_transfer = false;

        // Poor behaviour (overrides the honeymoon — character issues surface fast)
        if self.behaviour.is_poor() {
            wants_transfer = true;
        }

        // Unhappy for extended period (Unh status > 30 days, default path)
        let has_unh_long = self.statuses.statuses.iter().any(|s| {
            s.status == PlayerStatusType::Unh && (now - s.start_date).num_days() > 30
        });
        if has_unh_long {
            wants_transfer = true;
        }

        // Structural unhappiness: a big ambition mismatch (Messi → Floriana)
        // is a permanent feature of the club, not a bad week. Fire a request
        // sooner — 14 days of Unh is enough once ambition_fit is badly red.
        if !recently_transferred && self.happiness.factors.ambition_fit <= -7.0 {
            let has_unh_short = self.statuses.statuses.iter().any(|s| {
                s.status == PlayerStatusType::Unh && (now - s.start_date).num_days() > 14
            });
            if has_unh_short {
                wants_transfer = true;
            }
        }

        // Salary unhappy for a long time with no resolution → wants to leave
        // Only in the 540–730 day window; after 730 days the player has accepted the situation
        if let Some(first_request) = self.happiness.last_salary_negotiation {
            let days = (now - first_request).num_days();
            if days > 540 && days <= 730 && self.happiness.factors.salary_satisfaction <= -5.0 {
                wants_transfer = true;
            }
        }

        if recently_transferred && !self.behaviour.is_poor() {
            wants_transfer = false;
        }

        if wants_transfer {
            // Set Req (transfer request) status
            if !self.statuses.get().contains(&PlayerStatusType::Req) {
                self.statuses.add(now, PlayerStatusType::Req);
            }
            result.wants_to_leave = true;
            result.request_transfer(self.id);
        } else {
            // Conditions improved — remove transfer request if it was set
            if self.statuses.get().contains(&PlayerStatusType::Req) {
                self.statuses.remove(PlayerStatusType::Req);
            }
        }
    }
}
