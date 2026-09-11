use crate::Club;
use crate::ContractBonusType;
use chrono::{Datelike, NaiveDate};

impl Club {
    /// Pay every lump-sum bonus owed to a player on this monthly tick.
    /// Walks the club's player contracts and, for each one:
    ///   - SigningBonus pays once on the first finance pass after
    ///     acceptance. Mutates `signing_bonus_paid = true` so subsequent
    ///     passes skip it.
    ///   - LoyaltyBonus pays once per calendar year — the contract's
    ///     `last_loyalty_paid_year` memo prevents same-year double pay.
    ///   - InternationalCapFee pays per cap gained since the last pass.
    ///     Tracked via `last_intl_caps_paid` so the difference is the new
    ///     caps.
    ///
    /// Returns the total expense to charge to the club this month.
    pub(in crate::club::core) fn settle_lump_sum_bonuses(&mut self, date: NaiveDate) -> i64 {
        let year = date.year();
        let mut total: i64 = 0;
        for team in self.teams.teams.iter_mut() {
            for player in team.players.players.iter_mut() {
                // Cap-tracking baseline lives on the player; caps cumulative
                // count is `player.player_attributes.international_apps`.
                let current_caps = player.player_attributes.international_apps;
                let baseline_caps = player.last_intl_caps_paid;

                if let Some(contract) = player.contract.as_mut() {
                    for bonus in &contract.bonuses {
                        if bonus.value <= 0 {
                            continue;
                        }
                        match bonus.bonus_type {
                            ContractBonusType::SigningBonus => {
                                if !contract.signing_bonus_paid {
                                    total += bonus.value as i64;
                                }
                            }
                            ContractBonusType::LoyaltyBonus => {
                                // Pay only when the calendar date is on or
                                // past the contract's month/day anniversary
                                // for the current year. A Dec-31 signing
                                // therefore pays nothing on Jan 1 of the
                                // following year — payout falls due on the
                                // next Dec 31. Pay at most once per
                                // calendar year (last_loyalty_paid_year
                                // memo). Year of signing pays nothing —
                                // that's the signing bonus.
                                if let Some(started) = contract.started {
                                    if year <= started.year() {
                                        // Signing year — no loyalty payout.
                                    } else if contract.last_loyalty_paid_year == Some(year) {
                                        // Already paid this calendar year.
                                    } else if ContractAnniversary::reached(date, started) {
                                        total += bonus.value as i64;
                                    }
                                }
                            }
                            ContractBonusType::InternationalCapFee => {
                                let new_caps = current_caps.saturating_sub(baseline_caps) as i64;
                                if new_caps > 0 {
                                    total += bonus.value as i64 * new_caps;
                                }
                            }
                            _ => {}
                        }
                    }
                    // Memo updates AFTER the bonus value scan so a re-entrant
                    // call within the same month is a no-op.
                    if !contract.signing_bonus_paid
                        && contract
                            .bonuses
                            .iter()
                            .any(|b| matches!(b.bonus_type, ContractBonusType::SigningBonus))
                    {
                        contract.signing_bonus_paid = true;
                    }
                    if let Some(started) = contract.started {
                        if year > started.year()
                            && contract.last_loyalty_paid_year != Some(year)
                            && ContractAnniversary::reached(date, started)
                            && contract
                                .bonuses
                                .iter()
                                .any(|b| matches!(b.bonus_type, ContractBonusType::LoyaltyBonus))
                        {
                            contract.last_loyalty_paid_year = Some(year);
                        }
                    }
                }
                // Update international-caps baseline on the player so the next
                // pass only counts further caps. Done outside the contract
                // borrow.
                if current_caps > baseline_caps {
                    player.last_intl_caps_paid = current_caps;
                }
            }
        }
        total
    }
}

/// The month/day a contract was signed, as the loyalty bonus reads it.
struct ContractAnniversary;

impl ContractAnniversary {
    /// True when `today` falls on or past `started`'s month/day in the
    /// current calendar year. Gates annual loyalty payouts so a Dec-31
    /// contract doesn't accidentally pay a Jan-1 loyalty in the following
    /// year — the contract hasn't reached its anniversary yet.
    fn reached(today: NaiveDate, started: NaiveDate) -> bool {
        if today.month() > started.month() {
            return true;
        }
        if today.month() < started.month() {
            return false;
        }
        today.day() >= started.day()
    }
}

#[cfg(test)]
mod tests {
    use super::ContractAnniversary;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn dec_31_signing_does_not_pay_jan_1_next_year() {
        // The classic edge case: a Dec-31 signing must NOT pay a
        // loyalty bonus on Jan 1 — that's not the anniversary.
        let signed = d(2026, 12, 31);
        assert!(!ContractAnniversary::reached(d(2027, 1, 1), signed));
        assert!(!ContractAnniversary::reached(d(2027, 6, 1), signed));
        assert!(!ContractAnniversary::reached(d(2027, 12, 30), signed));
        // Pays on the anniversary itself.
        assert!(ContractAnniversary::reached(d(2027, 12, 31), signed));
    }

    #[test]
    fn mid_year_signing_pays_after_anniversary_in_following_year() {
        let signed = d(2026, 7, 1);
        // Same month, before the day.
        assert!(!ContractAnniversary::reached(d(2027, 6, 30), signed));
        // Anniversary day.
        assert!(ContractAnniversary::reached(d(2027, 7, 1), signed));
        // Later in the year.
        assert!(ContractAnniversary::reached(d(2027, 11, 1), signed));
    }

    #[test]
    fn signing_year_does_not_pay_anniversary() {
        // Even though the date passes the month/day check WITHIN the
        // signing year, callers must additionally gate on year >
        // started.year() — the loyalty bonus pays from the FIRST
        // anniversary onward, not from "the day after signing".
        let signed = d(2026, 7, 1);
        // The helper itself just checks month/day — the year guard is
        // upstream in `Club::settle_lump_sum_bonuses`.
        assert!(ContractAnniversary::reached(d(2026, 12, 31), signed));
    }
}
