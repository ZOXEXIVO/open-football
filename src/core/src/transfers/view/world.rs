//! How a market pass reaches the world it is moving players around in.

use crate::simulator::SimulatorData;
use crate::{Club, Country};

/// The world, as the transfer executors need to see it: countries by id,
/// readable and writable.
///
/// The market runs in two scopes. Phase A holds one `&mut Country` and
/// works inside it; Phase C holds `&mut SimulatorData` and can address
/// anywhere. Before this trait every path that could cross a border was
/// written twice — once for each scope — and the two copies drifted, which
/// is the whole of defect D1 in the refactor plan.
///
/// A single `Country` is a perfectly good world; it just contains one
/// country. Asking it for any other id yields `None`, and a cross-border
/// move attempted from a country-scoped caller therefore fails to resolve
/// its far side and reports failure — which is correct, because a caller
/// holding one country borrow could not have completed that move anyway.
///
/// Deliberately two methods. [`crate::league::result::LeagueProcessAccess`]
/// is the same idea at sixteen, and implementing that surface for `Country`
/// would mean a dozen stubs to buy nothing.
pub trait MarketWorld {
    /// The country with this id, if this world holds it.
    fn country(&self, id: u32) -> Option<&Country>;

    /// The same, for a pass that needs to write.
    fn country_mut(&mut self, id: u32) -> Option<&mut Country>;

    /// A club by id, wherever in this world it sits. The sell-on and
    /// clause payouts need it: a beneficiary is named by id and may be in
    /// any country, and the money has to reach it or it is destroyed.
    fn club_mut(&mut self, club_id: u32) -> Option<&mut Club>;
}

impl MarketWorld for Country {
    fn country(&self, id: u32) -> Option<&Country> {
        (self.id == id).then_some(self)
    }

    fn country_mut(&mut self, id: u32) -> Option<&mut Country> {
        (self.id == id).then_some(self)
    }

    fn club_mut(&mut self, club_id: u32) -> Option<&mut Club> {
        self.clubs.iter_mut().find(|c| c.id == club_id)
    }
}

impl MarketWorld for SimulatorData {
    fn country(&self, id: u32) -> Option<&Country> {
        SimulatorData::country(self, id)
    }

    fn country_mut(&mut self, id: u32) -> Option<&mut Country> {
        SimulatorData::country_mut(self, id)
    }

    fn club_mut(&mut self, club_id: u32) -> Option<&mut Club> {
        SimulatorData::club_mut(self, club_id)
    }
}
