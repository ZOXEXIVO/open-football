//! Who a memory is *about*.
//!
//! The gap this closes: today a `HappinessEvent` carries an optional
//! `partner_player_id` and nothing else, so the sim cannot express "this
//! manager broke his word to me" or "that club sold me against my will"
//! — only "something happened, and a player may have been involved".
//!
//! [`ActorRef`] is the subject key for all three memory stores: episodes
//! are tagged with it, semantic facts are held *about* it, and the
//! attribution ledger keeps one running account per actor. It is a
//! packed 8-byte `Copy` value so every store can hold it inline.

/// What kind of thing a memory is about. Separated from the id because
/// ids are only unique within a kind — staff 412 and club 412 are
/// different actors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ActorKind {
    /// No specific subject — a memory about a situation rather than a
    /// person or institution (an injury, a debut, a tournament).
    #[default]
    None,
    /// A coach / manager / staff member.
    Staff,
    /// Another player.
    Player,
    /// A club as an institution — distinct from its board and its fans,
    /// which a player really does judge separately.
    Club,
    /// The board specifically — the people who sanction transfers and
    /// break wage promises.
    Board,
    /// A club's supporters.
    Fans,
    /// A country, for the "this place never suited me" memories that
    /// outlive any particular club in it.
    Country,
}

impl ActorKind {
    /// True for actors that are people. People accrue trust and warmth;
    /// institutions accrue those plus a much longer memory of debt.
    #[inline]
    pub fn is_person(self) -> bool {
        matches!(self, ActorKind::Staff | ActorKind::Player)
    }

    /// True for the club-side actors that a `RecallCue::Club` should
    /// bring back together — arriving at a club also brings back its
    /// board and its supporters.
    #[inline]
    pub fn is_club_side(self) -> bool {
        matches!(self, ActorKind::Club | ActorKind::Board | ActorKind::Fans)
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            ActorKind::None => "mind_actor_none",
            ActorKind::Staff => "mind_actor_staff",
            ActorKind::Player => "mind_actor_player",
            ActorKind::Club => "mind_actor_club",
            ActorKind::Board => "mind_actor_board",
            ActorKind::Fans => "mind_actor_fans",
            ActorKind::Country => "mind_actor_country",
        }
    }
}

/// A reference to whoever a memory concerns. 8 bytes, `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ActorRef {
    pub kind: ActorKind,
    /// Id within `kind`. Meaningless (and zero by convention) when
    /// `kind` is [`ActorKind::None`].
    pub id: u32,
}

impl ActorRef {
    /// A memory with no personal subject.
    pub const NONE: ActorRef = ActorRef {
        kind: ActorKind::None,
        id: 0,
    };

    #[inline]
    pub fn staff(id: u32) -> Self {
        ActorRef {
            kind: ActorKind::Staff,
            id,
        }
    }

    #[inline]
    pub fn player(id: u32) -> Self {
        ActorRef {
            kind: ActorKind::Player,
            id,
        }
    }

    #[inline]
    pub fn club(id: u32) -> Self {
        ActorRef {
            kind: ActorKind::Club,
            id,
        }
    }

    #[inline]
    pub fn board(id: u32) -> Self {
        ActorRef {
            kind: ActorKind::Board,
            id,
        }
    }

    #[inline]
    pub fn fans(id: u32) -> Self {
        ActorRef {
            kind: ActorKind::Fans,
            id,
        }
    }

    #[inline]
    pub fn country(id: u32) -> Self {
        ActorRef {
            kind: ActorKind::Country,
            id,
        }
    }

    #[inline]
    pub fn is_none(self) -> bool {
        self.kind == ActorKind::None
    }

    #[inline]
    pub fn is_some(self) -> bool {
        !self.is_none()
    }

    /// True when this actor belongs to `club_id` — the club itself, its
    /// board, or its supporters. Staff and players are *not* matched:
    /// people move between clubs, and a player's memory of a coach
    /// follows the coach, not the badge.
    #[inline]
    pub fn belongs_to_club(self, club_id: u32) -> bool {
        self.kind.is_club_side() && self.id == club_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_id_different_kind_is_a_different_actor() {
        assert_ne!(ActorRef::staff(412), ActorRef::club(412));
    }

    #[test]
    fn club_side_actors_all_belong_to_the_club() {
        for actor in [ActorRef::club(7), ActorRef::board(7), ActorRef::fans(7)] {
            assert!(
                actor.belongs_to_club(7),
                "{actor:?} should belong to club 7"
            );
        }
    }

    #[test]
    fn people_do_not_belong_to_a_club() {
        // A coach who leaves takes the player's memory of him with him.
        assert!(!ActorRef::staff(7).belongs_to_club(7));
        assert!(!ActorRef::player(7).belongs_to_club(7));
    }

    #[test]
    fn none_is_the_default_and_reads_as_absent() {
        assert!(ActorRef::default().is_none());
        assert!(ActorRef::NONE.is_none());
        assert!(ActorRef::staff(1).is_some());
    }
}
