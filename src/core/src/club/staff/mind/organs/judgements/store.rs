//! Where a coach keeps his opinions of people.
//!
//! Fixed capacity, allocation-free, and — the whole point — attached to
//! the man rather than the club. Forty-eight is a career's worth of
//! *firm* views, not a list of everyone he has ever watched: when it
//! fills, the view he is least sure of makes way, which is the right
//! thing to lose.
//!
//! Reads are all through [`Judgements`] so the call sites read as
//! directives rather than as store manipulation.

use super::judgement::{JudgementOutcome, PlayerJudgement};
use crate::club::mind::organs::memory::{ActorRef, EpochDay, FixedStore};

/// A coach's standing opinions. 48 slots.
pub type JudgementStore = FixedStore<PlayerJudgement, 48>;

/// Operations over the judgement store.
pub struct Judgements;

impl Judgements {
    /// What he thinks of this player, if he has a view.
    pub fn of(store: &JudgementStore, player: ActorRef) -> Option<&PlayerJudgement> {
        store.find(|view| view.player == player)
    }

    pub fn of_mut(store: &mut JudgementStore, player: ActorRef) -> Option<&mut PlayerJudgement> {
        store.find_mut(|view| view.player == player)
    }

    /// Get the view he holds, forming a tentative one if this is the
    /// first time he has looked. Returns `None` only when the store is
    /// full of views he is surer of than this one would be — a coach
    /// with forty-eight firm opinions does not form a forty-ninth about
    /// a player he has just met.
    pub fn form(
        store: &mut JudgementStore,
        player: ActorRef,
        level: f32,
        ceiling: f32,
        today: EpochDay,
    ) -> Option<&mut PlayerJudgement> {
        if Self::of(store, player).is_none() {
            let fresh = PlayerJudgement::forming(player, level, ceiling, today);
            let displaced = fresh.retention(today);
            store.push_evicting(
                fresh,
                |existing| existing.retention(today),
                // Nothing he is surer of than the new impression gives
                // way. `push_evicting` drops the incoming item when
                // nothing qualifies, which is exactly right here.
                move |existing| existing.retention(today) < displaced,
            );
        }
        Self::of_mut(store, player)
    }

    /// He watched him play.
    pub fn watched(
        store: &mut JudgementStore,
        player: ActorRef,
        rating: f32,
        big_match: bool,
        today: EpochDay,
    ) {
        if let Some(view) = Self::of_mut(store, player) {
            view.watched(rating, big_match, today);
        }
    }

    /// How sure he is about this player right now. Zero when he has no
    /// view — which a caller must distinguish from a low opinion.
    pub fn confidence_in(store: &JudgementStore, player: ActorRef, today: EpochDay) -> f32 {
        Self::of(store, player)
            .map(|view| view.confidence(today))
            .unwrap_or(0.0)
    }

    /// Settle a question the player's career has answered. Returns the
    /// verdict when there was a firm enough view to be right or wrong
    /// about.
    pub fn settle(
        store: &mut JudgementStore,
        player: ActorRef,
        true_level: f32,
    ) -> Option<JudgementOutcome> {
        Self::of_mut(store, player).and_then(|view| view.settle(true_level))
    }

    /// The players he would build a side around, strongest first.
    /// Bounded output so the caller never allocates.
    pub fn core<const N: usize>(store: &JudgementStore, today: EpochDay) -> [Option<ActorRef>; N] {
        let mut best = [(f32::NEG_INFINITY, None); N];
        if N == 0 {
            return best.map(|(_, actor)| actor);
        }

        for view in store.iter().filter(|v| v.is_worth_building_around(today)) {
            let score = view.level() * view.confidence(today);
            if score <= best[N - 1].0 {
                continue;
            }
            best[N - 1] = (score, Some(view.player));
            let mut slot = N - 1;
            while slot > 0 && best[slot].0 > best[slot - 1].0 {
                best.swap(slot, slot - 1);
                slot -= 1;
            }
        }

        best.map(|(_, actor)| actor)
    }

    /// Time passes. Nothing to do — confidence fades in the read rather
    /// than in the store, so a judgement never needs a tick. Kept as a
    /// named no-op so the absence is deliberate rather than an omission.
    pub fn no_periodic_pass_needed() {}

    /// What the store holds.
    pub fn census(store: &JudgementStore, today: EpochDay) -> JudgementCensus {
        JudgementCensus {
            held: store.len() as u16,
            firm: store.iter().filter(|v| v.confidence(today) >= 0.5).count() as u16,
            settled: store.iter().filter(|v| v.outcome.is_settled()).count() as u16,
            wrong: store
                .iter()
                .filter(|v| v.outcome == JudgementOutcome::Wrong)
                .count() as u16,
        }
    }
}

/// What a coach's judgement store currently holds. For the `.dev/mind`
/// census and the staff profile page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JudgementCensus {
    pub held: u16,
    /// Views he is actually sure of.
    pub firm: u16,
    /// Questions his players' careers have since answered.
    pub settled: u16,
    /// Of those, the ones he got wrong.
    pub wrong: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: EpochDay = 5_000;

    fn player(id: u32) -> ActorRef {
        ActorRef::player(id)
    }

    #[test]
    fn forming_a_view_is_idempotent() {
        let mut store = JudgementStore::new();
        Judgements::form(&mut store, player(1), 0.6, 0.8, TODAY);
        Judgements::form(&mut store, player(1), 0.2, 0.3, TODAY);
        assert_eq!(store.len(), 1, "a second look is not a second player");
        assert!(
            Judgements::of(&store, player(1)).unwrap().level() > 0.5,
            "and it does not overwrite the view he already had"
        );
    }

    #[test]
    fn no_view_reads_as_no_view_rather_than_a_low_one() {
        let store = JudgementStore::new();
        assert_eq!(Judgements::confidence_in(&store, player(1), TODAY), 0.0);
        assert!(Judgements::of(&store, player(1)).is_none());
    }

    #[test]
    fn a_full_store_gives_up_the_view_he_is_least_sure_of() {
        let mut store = JudgementStore::new();
        for id in 0..store.capacity() as u32 {
            Judgements::form(&mut store, player(id), 0.5, 0.6, TODAY);
            // Everyone but player 7 gets watched into a firm view.
            if id != 7 {
                for week in 0..20 {
                    Judgements::watched(&mut store, player(id), 6.5, false, TODAY + week * 7);
                }
            }
        }
        assert!(store.is_full());

        let later = TODAY + 200;
        Judgements::form(&mut store, player(9_999), 0.7, 0.9, later);

        assert!(
            Judgements::of(&store, player(9_999)).is_some(),
            "the new impression got in"
        );
        assert!(
            Judgements::of(&store, player(7)).is_none(),
            "and the one he was least sure of made way"
        );
    }

    #[test]
    fn a_store_of_firm_views_refuses_a_passing_impression() {
        let mut store = JudgementStore::new();
        for id in 0..store.capacity() as u32 {
            Judgements::form(&mut store, player(id), 0.5, 0.6, TODAY);
            for week in 0..20 {
                Judgements::watched(&mut store, player(id), 6.5, false, TODAY + week * 7);
            }
        }
        let later = TODAY + 200;
        Judgements::form(&mut store, player(9_999), 0.7, 0.9, later);

        assert!(
            Judgements::of(&store, player(9_999)).is_none(),
            "he does not drop a man he knows for a man he has just seen"
        );
        assert_eq!(store.len(), store.capacity());
    }

    #[test]
    fn the_core_ranks_the_players_he_would_build_around() {
        let mut store = JudgementStore::new();
        for (id, level) in [(1u32, 0.92f32), (2, 0.74), (3, 0.40), (4, 0.85)] {
            Judgements::form(&mut store, player(id), level, level, TODAY);
            for week in 0..20 {
                Judgements::watched(&mut store, player(id), 7.0, false, TODAY + week * 7);
            }
        }

        let later = TODAY + 200;
        let core: [Option<ActorRef>; 3] = Judgements::core(&store, later);
        assert_eq!(core[0], Some(player(1)));
        assert_eq!(core[1], Some(player(4)));
        assert_eq!(core[2], Some(player(2)));
    }

    #[test]
    fn the_census_counts_what_he_got_wrong() {
        let mut store = JudgementStore::new();
        for id in 1..=3u32 {
            Judgements::form(&mut store, player(id), 0.35, 0.40, TODAY);
            for week in 0..16 {
                Judgements::watched(&mut store, player(id), 5.5, false, TODAY + week * 7);
            }
        }
        let later = TODAY + 150;
        Judgements::settle(&mut store, player(1), 0.95);
        Judgements::settle(&mut store, player(2), 0.42);

        let census = Judgements::census(&store, later);
        assert_eq!(census.held, 3);
        assert_eq!(census.settled, 2);
        assert_eq!(census.wrong, 1);
    }

    #[test]
    fn the_store_stays_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<JudgementStore>();
    }
}
