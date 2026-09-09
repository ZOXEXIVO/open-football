//! Test-only support for the transfer system.
//!
//! [`kit`] builds worlds; [`layering`] asserts the module tree stays a
//! tree. Both are `#[cfg(test)]` — nothing here ships.

pub mod kit;
mod layering;
