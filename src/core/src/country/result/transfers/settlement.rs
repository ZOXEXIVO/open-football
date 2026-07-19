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
//! Cross-country routing: the queue lives on the BUYER's market, so the
//! buyer is always resolvable locally, but performance add-ons from
//! cross-border deals name a foreign seller. Those credits are returned
//! to the caller as `(club_id, amount)` pairs and routed globally in
//! Phase C (installment tranches collapse to upfront for cross-country
//! deals, so in practice only add-ons take this path).

use chrono::{Datelike, NaiveDate};
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
    /// in place. Returns `(club_id, amount)` credits owed to sellers
    /// that don't live in this country (cross-border deals): the buyer
    /// was debited here, and the caller must route the credit globally
    /// once the country borrow ends, or the money is destroyed.
    pub(crate) fn settle_due(country: &mut Country, today: NaiveDate) -> Vec<(u32, f64)> {
        let mut foreign_credits = Self::settle_installments(country, today);
        foreign_credits.extend(Self::settle_performance_addons(country, today));
        foreign_credits
    }

    /// Season-end promotion add-ons. `promoted_clubs` comes from the
    /// league pipeline — the only place that knows who actually went up.
    /// Wired from `process_promotion_relegation`; before that wiring the
    /// resolver had no caller at all, so every scheduled promotion bonus
    /// silently expired and sellers were shortchanged. Locally-resolvable
    /// payouts route immediately; a fired clause owed to a foreign seller
    /// is re-queued as an immediately-due installment so the next daily
    /// settlement pass routes it through the Phase-C global drain (this
    /// walk runs inside the season-end country borrow, which cannot reach
    /// other countries).
    pub(crate) fn settle_promotions(
        country: &mut Country,
        today: NaiveDate,
        promoted_clubs: &[u32],
    ) {
        if promoted_clubs.is_empty() {
            return;
        }
        let fired = country
            .transfer_market
            .resolve_promotion_clauses(today, promoted_clubs);
        for clause in fired {
            let seller_is_local = country.clubs.iter().any(|c| c.id == clause.selling_club_id);
            if seller_is_local {
                let _ = Self::route_payout(country, &clause);
            } else {
                country
                    .transfer_market
                    .pending_clauses
                    .push(PendingTransferClause {
                        trigger: ClauseTrigger::Installment {
                            scheduled_date: today,
                        },
                        fires_so_far: 0,
                        max_fires: 1,
                        expires_on: None,
                        ..clause
                    });
            }
        }
    }

    fn settle_installments(country: &mut Country, today: NaiveDate) -> Vec<(u32, f64)> {
        let due = country.transfer_market.drain_due_installments(today);
        let mut foreign = Vec::new();
        for clause in due {
            if let Some(credit) = Self::route_payout(country, &clause) {
                foreign.push(credit);
            }
        }
        foreign
    }

    /// Resolve appearance/goal milestone bonuses. Reads counters off
    /// the buying club's roster — the player has to still be on it
    /// for the milestone to count (parent-club appearances accumulate
    /// elsewhere; this market only tracks the buyer-side total).
    fn settle_performance_addons(country: &mut Country, today: NaiveDate) -> Vec<(u32, f64)> {
        // First pass: build a snapshot of (player_id → buying_club_id)
        // → (appearances, goals) from the country roster so the
        // closures below can do O(1) lookups without re-walking the
        // world for each pending clause.
        let snapshot = PlayerCountSnapshot::build(country);
        let fired = country.transfer_market.resolve_performance_clauses(
            today,
            |player_id, buying_club_id, since| {
                snapshot.appearances(player_id, buying_club_id, since)
            },
            |player_id, buying_club_id, since| snapshot.goals(player_id, buying_club_id, since),
        );
        let mut foreign = Vec::new();
        for clause in fired {
            if let Some(credit) = Self::route_payout(country, &clause) {
                foreign.push(credit);
            }
        }
        foreign
    }

    /// Move cash from the buying club to the selling club. Routes through
    /// the pure-cash `adjust_cash` interface (debit = negative): a deferred
    /// installment / add-on tranche is an obligation settlement, NOT a
    /// player sale, so it must not perturb either club's transfer budget.
    /// The buyer always lives in this country (clauses are queued on the
    /// buyer's market); the seller may not — a cross-border deal's seller
    /// is returned as `Some((club_id, amount))` for the caller to credit
    /// globally after the country borrow ends.
    fn route_payout(
        country: &mut Country,
        clause: &PendingTransferClause,
    ) -> Option<(u32, f64)> {
        let amount = clause.amount.max(0.0);
        if amount <= 0.0 {
            return None;
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
        match seller_idx {
            Some(_) => None,
            None => Some((seller, amount)),
        }
    }
}

/// Per-(player, club) appearance/goal counters captured once at
/// settlement time so the trigger closures don't re-walk the world for
/// each clause. Built fresh each tick — cheap relative to a full
/// country sim.
///
/// The at-club total is the LIVE season counter plus the drained
/// `season_ledger` rows at the club's teams. The live counter alone
/// resets every July (and on every move), which silently turned "N
/// appearances at the club" into "N appearances in a single season" —
/// cross-season milestones could never fire. Loan-spell ledger rows are
/// excluded: a loan-then-buy player's borrowed apps belong to the loan
/// deal, not the purchase the clause was written on.
struct PlayerCountSnapshot {
    entries: Vec<PlayerCountEntry>,
}

struct PlayerCountEntry {
    player_id: u32,
    club_id: u32,
    live_apps: u32,
    live_goals: u32,
    /// Drained per-season rows at this club: (season_start_year, apps, goals).
    ledger: Vec<(u16, u32, u32)>,
}

impl PlayerCountEntry {
    /// Ledger rows that belong to the deal a clause created on `since`
    /// tracks. A European season crossing the new year has
    /// `season_start_year` one below the January calendar year, so the
    /// season containing `since` always satisfies
    /// `season_start_year + 1 >= since.year()`. `None` (legacy clause)
    /// counts the full at-club record.
    fn counts_since(&self, since: Option<NaiveDate>) -> (u32, u32) {
        let mut apps = self.live_apps;
        let mut goals = self.live_goals;
        for (season_start_year, row_apps, row_goals) in &self.ledger {
            let in_scope = match since {
                Some(created) => u32::from(*season_start_year) + 1 >= created.year() as u32,
                None => true,
            };
            if in_scope {
                apps = apps.saturating_add(*row_apps);
                goals = goals.saturating_add(*row_goals);
            }
        }
        (apps, goals)
    }
}

impl PlayerCountSnapshot {
    fn build(country: &Country) -> Self {
        let mut entries: Vec<PlayerCountEntry> = Vec::new();
        for club in &country.clubs {
            let club_slugs: Vec<&str> = club.teams.teams.iter().map(|t| t.slug.as_str()).collect();
            for team in &club.teams.teams {
                for player in &team.players.players {
                    let live_apps = (player.statistics.played as u32)
                        .saturating_add(player.statistics.played_subs as u32);
                    let live_goals = player.statistics.goals as u32;
                    let ledger: Vec<(u16, u32, u32)> = player
                        .statistics_history
                        .season_ledger
                        .iter()
                        .filter(|row| !row.is_loan)
                        .filter(|row| club_slugs.contains(&row.team_slug.as_str()))
                        .map(|row| {
                            (
                                row.season_start_year,
                                (row.statistics.played as u32)
                                    .saturating_add(row.statistics.played_subs as u32),
                                row.statistics.goals as u32,
                            )
                        })
                        .collect();
                    entries.push(PlayerCountEntry {
                        player_id: player.id,
                        club_id: club.id,
                        live_apps,
                        live_goals,
                        ledger,
                    });
                }
            }
        }
        PlayerCountSnapshot { entries }
    }

    fn appearances(
        &self,
        player_id: u32,
        buying_club_id: u32,
        since: Option<NaiveDate>,
    ) -> Option<u32> {
        self.entries
            .iter()
            .find(|e| e.player_id == player_id && e.club_id == buying_club_id)
            .map(|e| e.counts_since(since).0)
    }

    fn goals(&self, player_id: u32, buying_club_id: u32, since: Option<NaiveDate>) -> Option<u32> {
        self.entries
            .iter()
            .find(|e| e.player_id == player_id && e.club_id == buying_club_id)
            .map(|e| e.counts_since(since).1)
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
                created_on: None,
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
                created_on: None,
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
                created_on: None,
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
                created_on: None,
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
                created_on: None,
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
