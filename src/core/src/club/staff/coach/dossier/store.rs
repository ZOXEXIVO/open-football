//! Where a coach keeps the players he has worked with.
//!
//! A sorted `Vec` rather than a `HashMap`: the store is walked whole when a
//! coach arrives somewhere and binary-searched otherwise, it is cloned with
//! every `Staff`, and at [`DossierTuning::CAPACITY`] entries the linear
//! layout is both smaller and faster than a map. Sorted by `player_id`, so
//! iteration order is stable and a census is reproducible.
//!
//! Capped, and the cap is the point: a store that grows with every player a
//! coach has ever watched is a log, not a memory. When it is full, the
//! record he would miss least makes way — which is never one with a red
//! card in a final or a captaincy in it.

use super::record::PlayerDossier;
use super::tuning::DossierTuning;
use crate::club::mind::organs::memory::EpochDay;

/// A coach's lasting records, sorted by player id.
#[derive(Debug, Clone, Default)]
pub struct CoachDossierStore {
    records: Vec<PlayerDossier>,
}

impl CoachDossierStore {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        DossierTuning::CAPACITY
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &PlayerDossier> {
        self.records.iter()
    }

    #[inline]
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut PlayerDossier> {
        self.records.iter_mut()
    }

    pub fn get(&self, player_id: u32) -> Option<&PlayerDossier> {
        self.index_of(player_id).map(|at| &self.records[at])
    }

    pub fn get_mut(&mut self, player_id: u32) -> Option<&mut PlayerDossier> {
        self.index_of(player_id).map(|at| &mut self.records[at])
    }

    fn index_of(&self, player_id: u32) -> Option<usize> {
        self.records
            .binary_search_by_key(&player_id, |record| record.player_id)
            .ok()
    }

    /// Spells currently running.
    pub fn open_spells(&self) -> usize {
        self.records.iter().filter(|record| record.open).count()
    }

    /// Insert a fresh record, making room if the store is full. Returns
    /// `false` only when every record held is worth more than the new one,
    /// which for a fresh dossier is possible and correct: a coach with a
    /// career behind him does not displace a man he knows for one he has
    /// just met.
    pub fn insert(&mut self, record: PlayerDossier, today: EpochDay) -> bool {
        match self
            .records
            .binary_search_by_key(&record.player_id, |held| held.player_id)
        {
            Ok(at) => {
                self.records[at] = record;
                true
            }
            Err(at) => {
                if self.records.len() >= DossierTuning::CAPACITY
                    && !self.make_room(record.significance(today), today)
                {
                    return false;
                }
                // `make_room` may have shifted everything left of the gap.
                let at = if self.records.len() < at { self.records.len() } else { at };
                let at = self
                    .records
                    .binary_search_by_key(&record.player_id, |held| held.player_id)
                    .unwrap_or(at);
                self.records.insert(at, record);
                true
            }
        }
    }

    /// Drop the least significant record, if any is worth less than
    /// `incoming`.
    fn make_room(&mut self, incoming: f32, today: EpochDay) -> bool {
        let mut weakest: Option<(usize, f32)> = None;
        for (at, record) in self.records.iter().enumerate() {
            let score = record.significance(today);
            if score >= incoming {
                continue;
            }
            if weakest.is_none_or(|(_, held)| score < held) {
                weakest = Some((at, score));
            }
        }
        match weakest {
            Some((at, _)) => {
                self.records.remove(at);
                true
            }
            None => false,
        }
    }

    /// Forget a player outright. Used by nothing in the ordinary run of the
    /// sim — a dossier is meant to last — and exposed for the tests and for
    /// a future "he has retired and I have stopped thinking about him" pass.
    pub fn forget(&mut self, player_id: u32) {
        if let Some(at) = self.index_of(player_id) {
            self.records.remove(at);
        }
    }

    /// What the store holds.
    pub fn census(&self, today: EpochDay) -> DossierCensus {
        let mut census = DossierCensus {
            held: self.records.len() as u16,
            ..DossierCensus::default()
        };
        for record in &self.records {
            if record.open {
                census.open = census.open.saturating_add(1);
            }
            if record.spells > 1 {
                census.reunited = census.reunited.saturating_add(1);
            }
            if !record.scars.is_empty() {
                census.scarred = census.scarred.saturating_add(1);
            }
            if !record.medals.is_empty() {
                census.decorated = census.decorated.saturating_add(1);
            }
            if record.warmth_now(today) >= 0.3 {
                census.warm = census.warm.saturating_add(1);
            } else if record.warmth_now(today) <= -0.3 {
                census.cold = census.cold.saturating_add(1);
            }
            census.matches = census
                .matches
                .saturating_add(record.matches_together as u32);
        }
        census
    }
}

/// What a coach's dossier store currently holds. For the `.dev/mind` census
/// and the staff profile page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DossierCensus {
    pub held: u16,
    /// Spells running right now — roughly his current squad.
    pub open: u16,
    /// Players he has worked with more than once.
    pub reunited: u16,
    pub scarred: u16,
    pub decorated: u16,
    /// Men he would have back.
    pub warm: u16,
    /// Men he would not.
    pub cold: u16,
    /// Total matches watched across every record.
    pub matches: u32,
}

/// Operations over a coach's dossiers, so call sites read as directives
/// rather than as store manipulation.
pub struct Dossiers;

impl Dossiers {
    /// What he has on this player, if anything.
    pub fn of(store: &CoachDossierStore, player_id: u32) -> Option<&PlayerDossier> {
        store.get(player_id)
    }

    pub fn of_mut(store: &mut CoachDossierStore, player_id: u32) -> Option<&mut PlayerDossier> {
        store.get_mut(player_id)
    }

    /// Is there an open spell with this player?
    pub fn is_working_with(store: &CoachDossierStore, player_id: u32) -> bool {
        store.get(player_id).is_some_and(|record| record.open)
    }

    /// Start working together. Returns what kind of meeting this is.
    pub fn open(
        store: &mut CoachDossierStore,
        player_id: u32,
        club_id: u32,
        today: EpochDay,
    ) -> SpellOpening {
        if let Some(record) = store.get_mut(player_id) {
            if record.open {
                return SpellOpening::AlreadyOpen;
            }
            record.reopen(club_id, today);
            return SpellOpening::Reunion;
        }
        let record = PlayerDossier::opened(player_id, club_id, today);
        if store.insert(record, today) {
            SpellOpening::Fresh
        } else {
            // The store is full of men he is surer of. He still works with
            // the player; he simply will not be carrying a record of it.
            SpellOpening::Unrecorded
        }
    }

    /// Men he would have back, warmest first. Bounded output.
    pub fn warmest<const N: usize>(
        store: &CoachDossierStore,
        today: EpochDay,
    ) -> [Option<u32>; N] {
        let mut best = [(f32::NEG_INFINITY, None); N];
        if N == 0 {
            return best.map(|(_, id)| id);
        }
        for record in store.iter().filter(|record| !record.open) {
            let score = record.warmth_now(today);
            if score <= best[N - 1].0 {
                continue;
            }
            best[N - 1] = (score, Some(record.player_id));
            let mut slot = N - 1;
            while slot > 0 && best[slot].0 > best[slot - 1].0 {
                best.swap(slot, slot - 1);
                slot -= 1;
            }
        }
        best.map(|(_, id)| id)
    }

    /// Every spell still open, for the pass that closes them all when a
    /// coach leaves a club.
    pub fn open_player_ids(store: &CoachDossierStore) -> Vec<u32> {
        store
            .iter()
            .filter(|record| record.open)
            .map(|record| record.player_id)
            .collect()
    }
}

/// What happened when a coach and a player started working together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpellOpening {
    /// He has never worked with this player.
    Fresh,
    /// He has, and he remembers.
    Reunion,
    /// The spell was already running.
    AlreadyOpen,
    /// His store is full of men he knows better; this one goes unrecorded.
    Unrecorded,
}

impl SpellOpening {
    #[inline]
    pub fn is_reunion(self) -> bool {
        matches!(self, SpellOpening::Reunion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::record::{MedalFlags, ScarFlags, SeparationCause};

    const TODAY: EpochDay = 10_000;
    const YEAR: EpochDay = 365;

    /// Fixture builders, grouped so the tests read as sentences.
    struct Fx;

    impl Fx {
        /// A closed record with the given depth and marks.
        fn parted(
            player_id: u32,
            matches: u16,
            warmth: f32,
            scars: u16,
            medals: u16,
            years_ago: u16,
        ) -> PlayerDossier {
            let parted_on = TODAY - years_ago * YEAR;
            let mut record = PlayerDossier::opened(player_id, 1, parted_on);
            record.add_matches(matches);
            record.set_warmth(warmth);
            if scars != 0 {
                record.scars.insert(scars);
            }
            if medals != 0 {
                record.medals.insert(medals);
            }
            record.refresh_scar_strength(10.0);
            record.close(SeparationCause::IMovedOn, 1, 27, parted_on);
            record
        }

        fn full_of_servants() -> CoachDossierStore {
            let mut store = CoachDossierStore::new();
            for id in 0..DossierTuning::CAPACITY as u32 {
                store.insert(Fx::parted(id, 80, 0.5, 0, MedalFlags::CORNERSTONE, 1), TODAY);
            }
            store
        }
    }

    #[test]
    fn a_record_is_found_by_binary_search_whatever_order_it_arrived_in() {
        let mut store = CoachDossierStore::new();
        for id in [7u32, 2, 19, 4] {
            store.insert(Fx::parted(id, 10, 0.0, 0, 0, 1), TODAY);
        }
        for id in [7u32, 2, 19, 4] {
            assert!(store.get(id).is_some(), "missing {id}");
        }
        assert!(store.get(5).is_none());
        let ids: Vec<u32> = store.iter().map(|record| record.player_id).collect();
        assert_eq!(ids, vec![2, 4, 7, 19], "the store stays sorted");
    }

    #[test]
    fn a_full_store_forgets_the_least_significant_man_first() {
        let mut store = CoachDossierStore::new();
        for id in 1..DossierTuning::CAPACITY as u32 {
            store.insert(Fx::parted(id, 60, 0.4, 0, 0, 1), TODAY);
        }
        // One man he barely knew, long ago.
        store.insert(Fx::parted(999, 2, 0.0, 0, 0, 9), TODAY);
        assert_eq!(store.len(), DossierTuning::CAPACITY);

        // A new man, worth more than the passing acquaintance.
        assert!(store.insert(Fx::parted(1000, 40, 0.3, 0, 0, 0), TODAY));
        assert!(
            store.get(999).is_none(),
            "the one he was least sure of made way"
        );
        assert!(store.get(1000).is_some());
        assert_eq!(store.len(), DossierTuning::CAPACITY);
    }

    #[test]
    fn a_protected_scar_is_never_evicted_for_a_passing_acquaintance() {
        let mut store = CoachDossierStore::new();
        for id in 1..DossierTuning::CAPACITY as u32 {
            store.insert(Fx::parted(id, 60, 0.4, 0, 0, 1), TODAY);
        }
        store.insert(
            Fx::parted(999, 3, -0.4, ScarFlags::COST_US_THE_OCCASION, 0, 12),
            TODAY,
        );

        // Somebody he watched twice last season.
        let accepted = store.insert(Fx::parted(1000, 2, 0.0, 0, 0, 0), TODAY);
        assert!(
            store.get(999).is_some(),
            "the red card in the final stayed, whoever else had to go"
        );
        assert!(accepted || store.get(1000).is_none());
    }

    #[test]
    fn a_store_of_men_he_knows_refuses_a_stranger() {
        let mut store = Fx::full_of_servants();
        let accepted = store.insert(Fx::parted(9_999, 1, 0.0, 0, 0, 0), TODAY);
        assert!(!accepted, "he does not drop a servant for a stranger");
        assert!(store.get(9_999).is_none());
        assert_eq!(store.len(), DossierTuning::CAPACITY);
    }

    #[test]
    fn opening_a_spell_twice_is_one_spell() {
        let mut store = CoachDossierStore::new();
        assert_eq!(Dossiers::open(&mut store, 4, 1, TODAY), SpellOpening::Fresh);
        assert_eq!(
            Dossiers::open(&mut store, 4, 1, TODAY),
            SpellOpening::AlreadyOpen
        );
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(4).unwrap().spells, 1);
    }

    #[test]
    fn working_with_him_again_is_a_reunion_not_a_new_record() {
        let mut store = CoachDossierStore::new();
        store.insert(Fx::parted(4, 50, 0.5, 0, 0, 3), TODAY);

        assert_eq!(
            Dossiers::open(&mut store, 4, 9, TODAY),
            SpellOpening::Reunion
        );
        let record = store.get(4).unwrap();
        assert_eq!(record.spells, 2);
        assert_eq!(record.last_club, 9);
        assert_eq!(
            record.matches_together, 50,
            "the years together are not forgotten by meeting again"
        );
        assert!(record.open);
    }

    #[test]
    fn the_warmest_are_the_ones_he_would_ring_first() {
        let mut store = CoachDossierStore::new();
        store.insert(Fx::parted(1, 40, 0.2, 0, 0, 1), TODAY);
        store.insert(Fx::parted(2, 40, 0.9, 0, 0, 1), TODAY);
        store.insert(Fx::parted(3, 40, -0.5, 0, 0, 1), TODAY);
        store.insert(Fx::parted(4, 40, 0.6, 0, 0, 1), TODAY);

        let warmest: [Option<u32>; 2] = Dossiers::warmest(&store, TODAY);
        assert_eq!(warmest[0], Some(2));
        assert_eq!(warmest[1], Some(4));
    }

    #[test]
    fn the_census_counts_who_he_would_have_back() {
        let mut store = CoachDossierStore::new();
        store.insert(Fx::parted(1, 40, 0.8, 0, MedalFlags::CORNERSTONE, 1), TODAY);
        store.insert(
            Fx::parted(2, 10, -0.7, ScarFlags::REFUSED_TO_PLAY, 0, 1),
            TODAY,
        );
        Dossiers::open(&mut store, 3, 5, TODAY);

        let census = store.census(TODAY);
        assert_eq!(census.held, 3);
        assert_eq!(census.open, 1);
        assert_eq!(census.warm, 1);
        assert_eq!(census.cold, 1);
        assert_eq!(census.scarred, 1);
        assert_eq!(census.decorated, 1);
        assert_eq!(census.matches, 50);
    }
}
