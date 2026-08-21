//! **Attribute → ability.** What a player can actually do *right now*.
//!
//! Nothing in here knows about the pitch. Every module takes raw
//! attributes and turns them into a 0..1 ability, after the four
//! corrections that make a 5/20 finisher behave like one:
//!
//! * [`effective_skill`] — the foundation. Fatigue, late-game mental
//!   drain, stamina mitigation, and settledness, per skill category.
//! * [`skill_composites`] — named blends (`passing_execution`,
//!   `gk_shot_stopping`, …) so a decision site never averages raw
//!   attributes itself.
//! * [`traits_bias`] — `PlayerTrait` → concrete numeric bias.
//! * the role profiles — [`defender_skill`], [`midfielder_skill`],
//!   [`goalkeeper_skill`], [`shot_skill`] — one struct per role that
//!   reads every skill it needs once and exports the selection /
//!   execution scores the state machines consume directly. These are
//!   the single source of truth for their role: a decision site that
//!   branches on a raw `vision >= 14.0` belongs here instead.

pub mod effective_skill;
pub mod skill_composites;
pub mod traits_bias;

pub mod defender_skill;
pub mod goalkeeper_skill;
pub mod midfielder_skill;
pub mod shot_skill;

pub use effective_skill::*;
pub use skill_composites::*;
pub use traits_bias::*;

pub use defender_skill::*;
pub use goalkeeper_skill::*;
pub use midfielder_skill::*;
pub use shot_skill::*;
