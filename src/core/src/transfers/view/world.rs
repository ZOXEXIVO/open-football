//! How a market pass reaches the world it is moving players around in.

use crate::transfers::pipeline::PlayerSummary;
use crate::world::SimulatorData;
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

/// Where each player sits in the day's pool, sorted by id. Built once beside
/// the pool, so a summary resolves by id from any country.
pub struct WorldPoolIndex {
    by_id: Vec<(u32, u32)>,
}

impl WorldPoolIndex {
    pub fn build(summaries: &[PlayerSummary]) -> Self {
        let mut by_id: Vec<(u32, u32)> = summaries
            .iter()
            .enumerate()
            .map(|(slot, s)| (s.player_id, slot as u32))
            .collect();
        by_id.sort_unstable_by_key(|(id, _)| *id);
        WorldPoolIndex { by_id }
    }
}

/// The day's world pool: every country's transferable players, laid out
/// country by country, with the index that finds any of them by id.
#[derive(Clone, Copy)]
pub struct WorldPool<'a> {
    summaries: &'a [PlayerSummary],
    by_id: &'a [(u32, u32)],
}

impl<'a> WorldPool<'a> {
    pub fn new(summaries: &'a [PlayerSummary], index: &'a WorldPoolIndex) -> Self {
        WorldPool {
            summaries,
            by_id: &index.by_id,
        }
    }

    /// A pool with nobody on it.
    pub fn empty() -> Self {
        WorldPool {
            summaries: &[],
            by_id: &[],
        }
    }

    pub fn summaries(&self) -> &'a [PlayerSummary] {
        self.summaries
    }

    /// His summary, wherever he plays. A player who is not on the pool — a
    /// loanee, or a man who has left football — resolves to nothing.
    pub fn find(&self, player_id: u32) -> Option<&'a PlayerSummary> {
        self.by_id
            .binary_search_by_key(&player_id, |(id, _)| *id)
            .ok()
            .and_then(|at| self.summaries.get(self.by_id[at].1 as usize))
    }

    /// Everyone on the pool but this country's own.
    pub fn foreign(&self, country_id: u32) -> ForeignPool<'a> {
        ForeignPool::of(*self, country_id)
    }
}

/// Every summary in the world pool but this country's own. The pool is laid
/// out country by country, so that is the run before the country's block
/// and the run after it, read in place.
#[derive(Clone, Copy)]
pub struct ForeignPool<'a> {
    world: WorldPool<'a>,
    before: &'a [PlayerSummary],
    after: &'a [PlayerSummary],
}

impl<'a> ForeignPool<'a> {
    pub fn of(world: WorldPool<'a>, country_id: u32) -> Self {
        let world_pool = world.summaries;
        let start = world_pool
            .iter()
            .position(|s| s.country_id == country_id)
            .unwrap_or(world_pool.len());
        let end = start
            + world_pool[start..]
                .iter()
                .take_while(|s| s.country_id == country_id)
                .count();
        ForeignPool {
            world,
            before: &world_pool[..start],
            after: &world_pool[end..],
        }
    }

    /// The whole pool this is a view of.
    pub fn world(&self) -> WorldPool<'a> {
        self.world
    }

    pub fn iter(&self) -> impl Iterator<Item = &'a PlayerSummary> + Clone + 'a {
        self.before.iter().chain(self.after)
    }

    pub fn is_empty(&self) -> bool {
        self.before.is_empty() && self.after.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfers::tests::kit::TestSummary;

    #[test]
    fn a_player_abroad_resolves_by_id_and_a_stranger_to_nothing() {
        let summaries = vec![
            TestSummary::new(30).country(1, "en").build(),
            TestSummary::new(10).country(1, "en").build(),
            TestSummary::new(20).country(2, "es").build(),
        ];
        let index = WorldPoolIndex::build(&summaries);
        let world = WorldPool::new(&summaries, &index);

        let abroad = world
            .find(20)
            .expect("a player in another country resolves");
        assert_eq!(abroad.country_id, 2);
        assert_eq!(world.find(10).map(|s| s.player_id), Some(10));
        assert!(world.find(99).is_none());
        assert!(WorldPool::empty().find(10).is_none());

        let foreign: Vec<u32> = world.foreign(1).iter().map(|s| s.player_id).collect();
        assert_eq!(foreign, vec![20]);
        assert_eq!(
            world.foreign(1).world().find(30).map(|s| s.player_id),
            Some(30)
        );
    }
}
