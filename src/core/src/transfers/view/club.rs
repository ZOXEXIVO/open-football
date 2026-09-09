//! What the market can read off a club without deciding anything.

use crate::Club;

/// Read-only questions the market asks about a club.
///
/// The market layer reads the world constantly — squad size, capacity, who
/// is registered where — and every one of those reads used to live wherever
/// the first caller happened to need it. `can_accept_player` in particular
/// sat in `country::result::transfers::types`, which made `transfers` import
/// `country::result` and closed a module cycle: the country pass calls the
/// pipeline, and the pipeline reached back into the country pass for one
/// predicate.
///
/// Nothing here decides anything. A view answers a question about the world
/// as it stands; the thresholds and the gates live in
/// [`crate::transfers::gate`].
pub struct ClubView;

impl ClubView {
    /// Squad cap for a club whose board has set no season targets.
    const DEFAULT_MAX_SQUAD: usize = 50;

    /// Whether the club has room on its main-team roster for one more
    /// player.
    ///
    /// The cap is the board's own `max_squad_size` season target, defaulting
    /// to [`Self::DEFAULT_MAX_SQUAD`]. Only the MAIN team counts — youth and
    /// reserve rosters are separate registrations — and the main team is
    /// resolved by type rather than by `teams[0]`, because the collection
    /// does not guarantee the main side is first and counting a B squad
    /// against the first-team cap gates the wrong roster.
    ///
    /// Read by every door into a squad: the free-agent matcher, the
    /// emergency fill, the pre-contract stage, the negotiation opener and
    /// the transfer executor, so a club that is full stops fielding
    /// approaches rather than discovering the problem at execution.
    pub fn can_accept_player(club: &Club) -> bool {
        let max_squad = club
            .board
            .season_targets
            .as_ref()
            .map(|t| t.max_squad_size as usize)
            .unwrap_or(Self::DEFAULT_MAX_SQUAD);
        let main_squad = club.teams.main().map(|t| t.players.len()).unwrap_or(0);
        main_squad < max_squad
    }
}
