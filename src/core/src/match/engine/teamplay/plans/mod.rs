//! **The per-tick team plans** — the three assignments a side refreshes
//! together, on the same tactical cadence as the state in
//! [`tactical`](super::tactical), so all four always describe the same
//! instant.
//!
//! Each plan has the same shape: a plain-POD `*Plan` / `TeamShape` every
//! player reads, a `*RefreshInputs` bundle the tick loop fills, and a
//! private builder that does the assigning. Between them they answer the
//! three questions a footballer has when the ball is somewhere else:
//!
//! * [`attack`] — who is this attack FOR, who takes which patch of the
//!   box, and who stays home ([`AttackPlan`](attack::AttackPlan)).
//! * [`defence`] — who presses, who covers, and who picks up whom
//!   ([`DefensivePlan`](defence::DefensivePlan)); the ranking that
//!   decides it is in [`duties`] and [`matcher`].
//! * [`shape`] — where the block IS, and the anchor each player holds
//!   inside it ([`TeamShape`](shape::TeamShape)); the rectangle's own
//!   geometry is in [`block`].
//!
//! They reference each other in prose because they overlap by design: a
//! man given a box slot by the attack plan is a man the rest-defence
//! count did not keep at home, and the shape catches everybody neither
//! plan named. `attack` keeps its builder inline; the other two were
//! large enough to earn their own file.

pub mod attack;
pub mod block;
pub mod defence;
pub mod duties;
pub mod matcher;
pub mod shape;
