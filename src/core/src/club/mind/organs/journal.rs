//! The diary — what actually turned in his head, and on what day.
//!
//! The other organs hold *state*: what he remembers, what he believes,
//! what he wants. None of them holds the moment any of it turned. A
//! conviction knows the day it crystallised and a want knows the day it
//! appeared, but the day he stopped feeling it privately and said it out
//! loud is nowhere — the goal simply carries a different status
//! afterwards — and a want he gave up on is pruned from the stack
//! entirely, taking its whole history with it.
//!
//! Those turns were already being computed and thrown away. [`GoalStack::
//! review`] decides them every week and returns them in a report whose
//! own doc comment promised it to "the census harness, **the event
//! feed**, and the tests"; the caller dropped it on the floor.
//!
//! This is where they land. It is deliberately **not** a second copy of
//! the organs: nothing here decays, nothing here is read back by the
//! simulation, and nothing here decides anything. It is a bounded,
//! dated, human-readable trail that a reader — the player's event feed —
//! renders, and it exists so a mind can be *watched* rather than only
//! inspected.
//!
//! Its one discipline is that a note is written where the turn is
//! decided, never re-derived by a reader comparing two snapshots. A
//! reader that infers "he must have voiced it some time last week"
//! guesses the date, and a diary of guessed dates is worse than none.
//!
//! [`GoalStack::review`]: super::goals::GoalStack::review

use super::goals::GoalKind;
use super::memory::{ActorRef, EpochDay, FactClaim, FixedStore};

/// What kind of turn a note records.
///
/// Only turns that nothing else in the game can show. A want being
/// *fed* is not here — it happens weekly and is a graph, not an event —
/// and neither is anything the happiness feed already carries, because
/// the two would render side by side on the same day saying the same
/// thing in different words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MindNoteKind {
    #[default]
    None,
    /// A want appeared. He feels it; nobody knows, himself included, how
    /// much it will come to matter.
    WantFormed,
    /// He said it out loud. The first rung anyone outside his own head
    /// can observe.
    WantVoiced,
    /// He made it a formal demand.
    WantPressed,
    /// He got it.
    WantSatisfied,
    /// The date he had privately given it passed unmet.
    WantFrustrated,
    /// He let it go — age, resignation, or a better want displaced it.
    WantAbandoned,
    /// A belief crystallised out of enough episodes saying the same
    /// thing. The one note that outlives everything that produced it.
    ConvictionFormed,
}

impl MindNoteKind {
    /// Localisation key for the line this note leads with. Every kind
    /// must have one; the catalog parity test walks [`Self::ALL`].
    pub fn as_i18n_key(self) -> &'static str {
        match self {
            MindNoteKind::None => "mind_note_none",
            MindNoteKind::WantFormed => "mind_note_want_formed",
            MindNoteKind::WantVoiced => "mind_note_want_voiced",
            MindNoteKind::WantPressed => "mind_note_want_pressed",
            MindNoteKind::WantSatisfied => "mind_note_want_satisfied",
            MindNoteKind::WantFrustrated => "mind_note_want_frustrated",
            MindNoteKind::WantAbandoned => "mind_note_want_abandoned",
            MindNoteKind::ConvictionFormed => "mind_note_conviction_formed",
        }
    }

    /// Which way the turn reads. Drives the feed's valence rail, and it
    /// is deliberately about the *turn* rather than the want: forming a
    /// want to leave is not bad news, being unable to act on one is.
    ///
    /// [`WantFormed`] and [`WantVoiced`] are neutral on purpose. A want
    /// appearing is a fact about him, not a good or a bad day, and a feed
    /// that painted every new ambition green would read as praise.
    ///
    /// [`WantFormed`]: Self::WantFormed
    /// [`WantVoiced`]: Self::WantVoiced
    pub fn valence(self) -> i8 {
        match self {
            MindNoteKind::None => 0,
            MindNoteKind::WantFormed | MindNoteKind::WantVoiced => 0,
            MindNoteKind::WantSatisfied => 1,
            MindNoteKind::WantPressed
            | MindNoteKind::WantFrustrated
            | MindNoteKind::WantAbandoned => -1,
            // A conviction carries its own sign — see `FactClaim::
            // valence` — so the note kind stays neutral and the renderer
            // reads the claim instead.
            MindNoteKind::ConvictionFormed => 0,
        }
    }

    /// True for the turns that are a genuine milestone in a career rather
    /// than a week's weather. The feed gives these a heavier card.
    #[inline]
    pub fn is_major(self) -> bool {
        matches!(
            self,
            MindNoteKind::WantPressed
                | MindNoteKind::WantSatisfied
                | MindNoteKind::ConvictionFormed
        )
    }

    /// True when the note is about a want, so a renderer knows to read
    /// [`MindNote::goal`] rather than [`MindNote::claim`].
    #[inline]
    pub fn is_about_a_want(self) -> bool {
        !matches!(self, MindNoteKind::None | MindNoteKind::ConvictionFormed)
    }

    pub const ALL: &'static [MindNoteKind] = &[
        MindNoteKind::None,
        MindNoteKind::WantFormed,
        MindNoteKind::WantVoiced,
        MindNoteKind::WantPressed,
        MindNoteKind::WantSatisfied,
        MindNoteKind::WantFrustrated,
        MindNoteKind::WantAbandoned,
        MindNoteKind::ConvictionFormed,
    ];
}

/// One dated turn. 16 bytes, `Copy`, no allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MindNote {
    /// Who or where it concerns. [`ActorRef::NONE`] for the notes that
    /// are purely about him — most want notes, and self-knowledge.
    pub subject: ActorRef,
    /// The day it turned, not the day it was read.
    pub day: EpochDay,
    pub kind: MindNoteKind,
    /// The want, when [`MindNoteKind::is_about_a_want`].
    pub goal: GoalKind,
    /// The belief, for [`MindNoteKind::ConvictionFormed`].
    pub claim: FactClaim,
}

impl MindNote {
    /// A turn in a want.
    pub fn want(kind: MindNoteKind, goal: GoalKind, day: EpochDay) -> Self {
        MindNote {
            subject: ActorRef::NONE,
            day,
            kind,
            goal,
            claim: FactClaim::None,
        }
    }

    /// A belief crystallising.
    pub fn conviction(claim: FactClaim, subject: ActorRef, day: EpochDay) -> Self {
        MindNote {
            subject,
            day,
            kind: MindNoteKind::ConvictionFormed,
            goal: GoalKind::None,
            claim,
        }
    }

    /// A note that says nothing. Never written; the `Default` that backs
    /// the store's empty slots.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.kind == MindNoteKind::None
    }
}

/// The trail itself.
///
/// Capacity 12 — about two seasons of turning points at the rate a
/// settled player produces them, and a hard ceiling on a structure that
/// sits inside every `Player` and every `Staff`. It is a *diary*, not a
/// log: when it fills, the oldest note goes, because the feed's job is
/// to explain the man he is now.
pub type MindNoteStore = FixedStore<MindNote, 12>;

/// Everything he has noticed turning in himself.
#[derive(Debug, Clone, Copy, Default)]
pub struct MindJournal {
    notes: MindNoteStore,
}

impl MindJournal {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Every note held, in the order written.
    pub fn iter(&self) -> impl Iterator<Item = &MindNote> {
        self.notes.iter().filter(|note| !note.is_empty())
    }

    /// Write a turn down. Oldest note out when full.
    ///
    /// A [`MindNoteKind::None`] note is refused rather than stored: it
    /// would occupy a slot, evict a real note, and render as a blank row.
    /// Callers building a note from an enum they did not check are the
    /// reason this guard is here rather than at each call site.
    pub fn record(&mut self, note: MindNote) -> bool {
        if note.is_empty() {
            return false;
        }
        // Rank is the day, so the victim is the oldest note. Everything
        // is evictable — a diary has no protected entries, unlike the
        // episode store where a flashbulb outranks a Tuesday.
        self.notes
            .push_evicting(note, |existing| existing.day as f32, |_| true);
        true
    }

    /// Drop everything. For a mind that is reset rather than aged.
    pub fn clear(&mut self) {
        self.notes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_note_is_refused() {
        let mut journal = MindJournal::new();
        assert!(!journal.record(MindNote::default()));
        assert!(journal.is_empty());
    }

    #[test]
    fn the_oldest_note_is_the_one_evicted() {
        let mut journal = MindJournal::new();
        for day in 0..20u16 {
            journal.record(MindNote::want(
                MindNoteKind::WantFormed,
                GoalKind::LeaveThisClub,
                100 + day,
            ));
        }
        assert_eq!(journal.len(), 12);
        let oldest = journal.iter().map(|n| n.day).min().expect("notes held");
        assert_eq!(
            oldest, 108,
            "a full journal must shed its oldest note, not its newest"
        );
    }

    #[test]
    fn notes_are_held_in_the_order_they_were_written() {
        let mut journal = MindJournal::new();
        for (kind, day) in [
            (MindNoteKind::WantFormed, 100),
            (MindNoteKind::WantVoiced, 300),
            (MindNoteKind::WantPressed, 200),
        ] {
            journal.record(MindNote::want(kind, GoalKind::LeaveThisClub, day));
        }

        // Insertion order, not date order. The feed merges the journal
        // with two other sources and sorts the lot itself; a store that
        // pre-sorted would be sorting twice and lying about which turn
        // was written first.
        let days: Vec<EpochDay> = journal.iter().map(|n| n.day).collect();
        assert_eq!(days, vec![100, 300, 200]);
    }

    #[test]
    fn a_want_note_carries_no_claim_and_a_conviction_carries_no_goal() {
        let want = MindNote::want(MindNoteKind::WantVoiced, GoalKind::LeaveThisClub, 10);
        assert_eq!(want.claim, FactClaim::None);
        assert!(want.kind.is_about_a_want());

        let belief = MindNote::conviction(FactClaim::DiscardedMe, ActorRef::club(7), 10);
        assert_eq!(belief.goal, GoalKind::None);
        assert!(!belief.kind.is_about_a_want());
    }

    #[test]
    fn every_kind_has_a_namespaced_key() {
        for kind in MindNoteKind::ALL {
            assert!(
                kind.as_i18n_key().starts_with("mind_note_"),
                "{kind:?} has an off-namespace key"
            );
        }
        let mut keys: Vec<&str> = MindNoteKind::ALL.iter().map(|k| k.as_i18n_key()).collect();
        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(before, keys.len(), "two note kinds share an i18n key");
    }

    #[test]
    fn a_note_stays_small() {
        assert!(
            size_of::<MindNote>() <= 16,
            "MindNote grew to {} bytes — it sits inside every Player",
            size_of::<MindNote>()
        );
        assert!(
            size_of::<MindJournal>() <= 208,
            "MindJournal grew to {} bytes",
            size_of::<MindJournal>()
        );
    }
}
