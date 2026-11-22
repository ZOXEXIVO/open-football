//! The club's own money, month by month.
//!
//! [`crate::club::finance`] owns the ledger — balances, revenue curves, debt
//! classification, sponsorship. This module is the club *spending and
//! earning* on it: the monthly pass that bills wages, books income, services
//! debt, and sheds wages when the bill outruns the mandate.

mod bonuses;
mod debt;
mod monthly;
mod relief;
mod surplus;

pub use relief::WageReliefSale;
