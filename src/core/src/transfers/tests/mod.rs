//! Test-only support for the transfer system.
//!
//! [`kit`] builds worlds; [`layering`] asserts the module tree stays a tree;
//! [`shape`] asserts the house rules on how code is arranged inside it. All
//! are `#[cfg(test)]` — nothing here ships.

pub mod kit;
mod layering;
mod shape;
