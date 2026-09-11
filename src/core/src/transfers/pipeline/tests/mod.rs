//! Behaviour tests for the pipeline's shared readings.
//!
//! They used to sit under `pipeline/helpers/`, beside a `helpers.rs` that
//! held four unrelated kinds of thing at once. That file is gone — its
//! contents went to the module that owns each of them ([`TierBands`],
//! [`PlayerView`], [`ScoutJudgement`], [`AskingPrice`], [`MarketCadence`],
//! [`ClubView`]) — so the tests live here, named for what they cover
//! rather than for where their subject used to be parked.
//!
//! [`TierBands`]: crate::transfers::squad::bands::TierBands
//! [`PlayerView`]: crate::transfers::view::player::PlayerView
//! [`ScoutJudgement`]: crate::transfers::scouting::judgement::ScoutJudgement
//! [`AskingPrice`]: crate::transfers::value::asking::AskingPrice
//! [`MarketCadence`]: crate::transfers::market::window::MarketCadence
//! [`ClubView`]: crate::transfers::view::club::ClubView

mod breakout;
mod group;
mod role;
mod slot;
mod tier;
