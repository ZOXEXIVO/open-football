//! **One tick of the simulation**, at the three cadences the engine runs
//! it on.
//!
//! * [`driver`] — every tick: `game_tick` and the light / full tick
//!   bodies it dispatches to. This is the running order the rest of the
//!   engine is called in.
//! * [`positions`] — the sub-phases that running order invokes (ball,
//!   outfield players, keepers), the level-of-detail cadence rule, and
//!   the replay position recorder.
//! * [`shape`] — the interval-gated plan refresh: situational shape,
//!   coach evaluation, the rolling 15-minute metrics, and the tactical
//!   state every player's decision reads.
//!
//! The work the driver does *to the ball* between these phases is not
//! here — it is in [`resolve`](super::resolve), because it needs the ball
//! and all 22 players at once and belongs to neither side.

pub mod driver;
pub mod positions;
pub mod shape;
