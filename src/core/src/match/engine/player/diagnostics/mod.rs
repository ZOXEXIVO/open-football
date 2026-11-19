/// Who takes the ball off a carrier and how, each duel, header and job a
/// defender has, which situations each state acts on, and how he moves.
#[cfg(feature = "match-logs")]
pub mod defender_census;
/// How a forward moves, the job each assignment gives him, each run in
/// behind, his duels and headers, the shots and passes his states ask for,
/// which situations each state acts on, and how long he keeps the ball.
#[cfg(feature = "match-logs")]
pub mod forward_census;
/// How each shot and high ball a keeper faces is decided, the rebound, the
/// time he spends down after a dive and his readiness phases.
#[cfg(feature = "match-logs")]
pub mod keeper_census;
/// How a midfielder moves, the job each assignment gives him, his duels,
/// headers and dropping balls, which situations each state acts on, and how
/// long he keeps a ball he receives.
#[cfg(feature = "match-logs")]
pub mod midfielder_census;
/// Runtime position-flicker / state-churn tracer. Dev diagnostic only —
/// the whole module (and every call site) compiles out without
/// `match-logs`, so production pays nothing.
#[cfg(feature = "match-logs")]
pub mod motion_diag;
