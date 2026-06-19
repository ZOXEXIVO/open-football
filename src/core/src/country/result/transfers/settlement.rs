//! Daily settler for deferred transfer obligations. Reads the
//! buying country's `TransferMarket::pending_clauses` queue and
//! resolves any clause whose trigger has fired today:
//!
//!   * **Installment** — calendar date has passed: buyer pays
//!     seller the tranche.
//!   * **Appearance** / **goal** milestone — player has crossed the
//!     count at the buying club: buyer pays seller the bonus.
//!   * **Promotion** — handled separately at end-of-season by the
//!     league pipeline (it knows who got promoted); this module
//!     leaves promotion clauses on the queue.
//!
//! Cross-country obligations route via direct lookup in the country
//! (the buying side always settles to a same-country seller for the
//! domestic execution path; cross-country sell-ons are settled
//! upfront in `execution.rs`, so the queue only ever holds same-
//! country obligations).

use chrono::NaiveDate;
use log::debug;

use crate::Country;
use crate::transfers::market::{ClauseTrigger, PendingTransferClause};

/// Stateless namespace for the daily clause settler. Wrapped on a unit
/// struct rather than free `fn`s so the settlement surface reads like
/// an API and stays a single, discoverable entry point.
pub(crate) struct TransferClauseSettler;

impl TransferClauseSettler {
    /// Resolve every clause whose calendar/counter trigger has fired
    /// today. Cash routing happens inline — buyer's finance debit,
    /// seller's finance credit — and the market's queue is rewritten
    /// in place.
    pub(crate) fn settle_due(country: &mut Country, today: NaiveDate) {
        Self::settle_installments(country, today);
        Self::settle_performance_addons(country, today);
    }

    fn settle_installments(country: &mut Country, today: NaiveDate) {
        let due = country.transfer_market.drain_due_installments(today);
        for clause in due {
            Self::route_payout(country, &clause);
        }
    }

    /// Resolve appearance/goal milestone bonuses. Reads counters off
    /// the buying club's roster — the player has to still be on it
    /// for the milestone to count (parent-club appearances accumulate
    /// elsewhere; this market only tracks the buyer-side total).
    fn settle_performance_addons(country: &mut Country, today: NaiveDate) {
        // First pass: build a snapshot of (player_id → buying_club_id)
        // → (appearances, goals) from the country roster so the
        // closures below can do O(1) lookups without re-walking the
        // world for each pending clause.
        let snapshot = PlayerCountSnapshot::build(country);
        let fired = country.transfer_market.resolve_performance_clauses(
            today,
            |player_id, buying_club_id| snapshot.appearances(player_id, buying_club_id),
            |player_id, buying_club_id| snapshot.goals(player_id, buying_club_id),
        );
        for clause in fired {
            Self::route_payout(country, &clause);
        }
    }

    /// Move cash from the buying club to the selling club. Routes through
    /// the pure-cash `adjust_cash` interface (debit = negative): a deferred
    /// installment / add-on tranche is an obligation settlement, NOT a
    /// player sale, so it must not perturb either club's transfer budget.
    fn route_payout(country: &mut Country, clause: &PendingTransferClause) {
        let amount = clause.amount.max(0.0);
        if amount <= 0.0 {
            return;
        }
        let buyer = clause.buying_club_id;
        let seller = clause.selling_club_id;
        let (buyer_idx, seller_idx) = {
            let mut b = None;
            let mut s = None;
            for (i, club) in country.clubs.iter().enumerate() {
                if club.id == buyer {
                    b = Some(i);
                }
                if club.id == seller {
                    s = Some(i);
                }
            }
            (b, s)
        };
        if let Some(i) = buyer_idx {
            country.clubs[i].finance.adjust_cash(-amount);
        }
        if let Some(i) = seller_idx {
            country.clubs[i].finance.adjust_cash(amount);
        }
        let label = match clause.trigger {
            ClauseTrigger::Installment { .. } => "installment",
            ClauseTrigger::AppearanceMilestone { .. } => "appearance-milestone",
            ClauseTrigger::GoalMilestone { .. } => "goal-milestone",
            ClauseTrigger::Promotion => "promotion-bonus",
        };
        debug!(
            "Clause settled: player {} club {} -> {} ({}): {}",
            clause.player_id, buyer, seller, label, amount
        );
    }
}

/// Per-(player, club) appearance/goal counters captured once at
/// settlement time so the trigger closures don't re-walk the world for
/// each clause. Built fresh each tick — cheap relative to a full
/// country sim.
struct PlayerCountSnapshot {
    entries: Vec<(u32, u32, u32, u32)>, // (player_id, club_id, apps, goals)
}

impl PlayerCountSnapshot {
    fn build(country: &Country) -> Self {
        let mut entries: Vec<(u32, u32, u32, u32)> = Vec::new();
        for club in &country.clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    let apps = (player.statistics.played as u32)
                        .saturating_add(player.statistics.played_subs as u32);
                    let goals = player.statistics.goals as u32;
                    entries.push((player.id, club.id, apps, goals));
                }
            }
        }
        PlayerCountSnapshot { entries }
    }

    fn appearances(&self, player_id: u32, buying_club_id: u32) -> Option<u32> {
        self.entries
            .iter()
            .find(|(pid, cid, _, _)| *pid == player_id && *cid == buying_club_id)
            .map(|(_, _, apps, _)| *apps)
    }

    fn goals(&self, player_id: u32, buying_club_id: u32) -> Option<u32> {
        self.entries
            .iter()
            .find(|(pid, cid, _, _)| *pid == player_id && *cid == buying_club_id)
            .map(|(_, _, _, goals)| *goals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::league::LeagueCollection;
    use crate::shared::{Currency, CurrencyValue, Location};
    use crate::transfers::market::{ClauseTrigger, PendingTransferClause};
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, TeamCollection,
    };
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn club(id: u32, name: &str) -> Club {
        Club::new(
            id,
            name.to_string(),
            Location::new(1),
            ClubFinances::new(50_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(Vec::new()),
            ClubFacilities::default(),
        )
    }

    fn country(buyer_id: u32, seller_id: u32) -> Country {
        Country::builder()
            .id(1)
            .code("ru".to_string())
            .slug("russia".to_string())
            .name("Russia".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(Vec::new()))
            .clubs(vec![club(buyer_id, "Buyer"), club(seller_id, "Seller")])
            .build()
            .unwrap()
    }

    #[test]
    fn installment_due_today_routes_buyer_to_seller() {
        let mut c = country(101, 202);
        let today = d(2026, 7, 1);
        // Buyer's market: schedule a tranche that pays *today*.
        c.transfer_market
            .pending_clauses
            .push(PendingTransferClause {
                id: 1,
                buying_club_id: 101,
                selling_club_id: 202,
                player_id: 9999,
                trigger: ClauseTrigger::Installment {
                    scheduled_date: today,
                },
                amount: 1_000_000.0,
                max_fires: 1,
                fires_so_far: 0,
                expires_on: None,
            });
        let buyer_before = c.clubs[0].finance.balance.balance;
        let seller_before = c.clubs[1].finance.balance.balance;

        TransferClauseSettler::settle_due(&mut c, today);

        let buyer_after = c.clubs[0].finance.balance.balance;
        let seller_after = c.clubs[1].finance.balance.balance;
        assert_eq!(buyer_before - buyer_after, 1_000_000);
        assert_eq!(seller_after - seller_before, 1_000_000);
        // Clause is retired after firing.
        assert!(c.transfer_market.pending_clauses.is_empty());
    }

    #[test]
    fn installment_settlement_does_not_perturb_transfer_budgets() {
        let mut c = country(101, 202);
        // Give both clubs an explicit transfer budget.
        for club in c.clubs.iter_mut() {
            club.finance.transfer_budget = Some(CurrencyValue::new(10_000_000.0, Currency::Usd));
        }
        let today = d(2026, 7, 1);
        c.transfer_market
            .pending_clauses
            .push(PendingTransferClause {
                id: 1,
                buying_club_id: 101,
                selling_club_id: 202,
                player_id: 9999,
                trigger: ClauseTrigger::Installment {
                    scheduled_date: today,
                },
                amount: 1_000_000.0,
                max_fires: 1,
                fires_so_far: 0,
                expires_on: None,
            });

        TransferClauseSettler::settle_due(&mut c, today);

        // A deferred tranche is a pure cash settlement: the budgets must be
        // exactly unchanged. The old `add_transfer_income` routing would
        // have shifted them by ±500k (half the tranche).
        assert_eq!(
            c.clubs[0].finance.transfer_budget.as_ref().unwrap().amount,
            10_000_000.0
        );
        assert_eq!(
            c.clubs[1].finance.transfer_budget.as_ref().unwrap().amount,
            10_000_000.0
        );
    }

    #[test]
    fn future_installment_not_yet_due_stays_queued() {
        let mut c = country(101, 202);
        let today = d(2026, 7, 1);
        let later = d(2027, 7, 1);
        c.transfer_market
            .pending_clauses
            .push(PendingTransferClause {
                id: 1,
                buying_club_id: 101,
                selling_club_id: 202,
                player_id: 9999,
                trigger: ClauseTrigger::Installment {
                    scheduled_date: later,
                },
                amount: 1_000_000.0,
                max_fires: 1,
                fires_so_far: 0,
                expires_on: None,
            });

        TransferClauseSettler::settle_due(&mut c, today);

        // Not yet due — still in queue, no cash moved.
        assert_eq!(c.transfer_market.pending_clauses.len(), 1);
    }

    #[test]
    fn appearance_milestone_below_threshold_does_not_fire() {
        let mut c = country(101, 202);
        let today = d(2026, 7, 1);
        // No player on the roster, so `appearance_count` returns None.
        c.transfer_market
            .pending_clauses
            .push(PendingTransferClause {
                id: 1,
                buying_club_id: 101,
                selling_club_id: 202,
                player_id: 555,
                trigger: ClauseTrigger::AppearanceMilestone {
                    target_appearances: 10,
                },
                amount: 500_000.0,
                max_fires: 1,
                fires_so_far: 0,
                expires_on: None,
            });

        TransferClauseSettler::settle_due(&mut c, today);

        // Roster lookup misses → clause stays queued (until expiry or
        // a future tick where the player has been added).
        assert_eq!(c.transfer_market.pending_clauses.len(), 1);
    }

    #[test]
    fn expired_clause_drops_off_queue() {
        let mut c = country(101, 202);
        let today = d(2026, 7, 1);
        let yesterday = d(2026, 6, 30);
        c.transfer_market
            .pending_clauses
            .push(PendingTransferClause {
                id: 1,
                buying_club_id: 101,
                selling_club_id: 202,
                player_id: 9999,
                trigger: ClauseTrigger::AppearanceMilestone {
                    target_appearances: 10,
                },
                amount: 500_000.0,
                max_fires: 1,
                fires_so_far: 0,
                expires_on: Some(yesterday),
            });

        TransferClauseSettler::settle_due(&mut c, today);

        // Past deadline — clause is dropped silently.
        assert!(c.transfer_market.pending_clauses.is_empty());
    }

    // Silence the unused-imports warning for currency/value when the
    // tests above don't directly construct one. (Keeping the import
    // here documents that the settler operates on monetary values.)
    #[allow(dead_code)]
    fn _money_marker() -> CurrencyValue {
        CurrencyValue::new(0.0, Currency::Usd)
    }
}
