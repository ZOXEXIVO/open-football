/// Runtime position-flicker / state-churn tracer. Dev diagnostic only —
/// the whole module (and every call site) compiles out without
/// `match-logs`, so production pays nothing.
#[cfg(feature = "match-logs")]
pub mod motion_diag;
