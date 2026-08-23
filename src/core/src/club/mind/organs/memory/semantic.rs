//! Semantic memory — what the episodes *meant*.
//!
//! This is the store that answers the ten-year question. Episodes fade
//! on a power law; facts do not decay at all. A player does not carry
//! four hundred memories of his three years at a club — he carries
//! "that's where I broke through", "they sold me when I didn't want to
//! go", "that manager never trusted me", and those outlive every episode
//! that produced them.
//!
//! Facts are formed by [`consolidation`] from episodes, and they weaken
//! only when contradicted — a manager who never trusted him, then does,
//! erodes the fact rather than leaving it standing alongside its
//! opposite.
//!
//! [`consolidation`]: super::consolidation

use super::actor::{ActorKind, ActorRef};
use super::epoch::EpochDay;
use super::store::FixedStore;
use std::cmp::Ordering;

/// A distilled, timeless claim about someone or somewhere.
///
/// Deliberately coarse. A fact is not a summary of an episode — it is
/// the one sentence a player would actually say about a club or a person
/// years later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FactClaim {
    #[default]
    None,

    // ── About a club ────────────────────────────────────────────
    /// Where he became a player. The strongest positive club bond there is.
    BrokeThroughHere,
    /// Won things here.
    WonEverythingHere,
    /// Sold against his will. The grudge that outlives everything.
    WasSoldAgainstMyWill,
    /// Discarded — released, frozen out, made surplus.
    DiscardedMe,
    /// The club kept its word when it mattered.
    ClubStoodByMe,
    /// The club broke its word.
    ClubBrokeItsWord,
    /// Never got a game here.
    NeverPlayedHere,
    /// Went down with them.
    RelegatedWithThem,
    /// Long service, warmly remembered. The "spiritual home" fact.
    SpiritualHome,

    // ── About supporters ────────────────────────────────────────
    FansAdoredMe,
    FansTurnedOnMe,

    // ── About a coach ───────────────────────────────────────────
    /// He made me the player I am.
    MadeMeAPlayer,
    /// He never rated me.
    NeverTrustedMe,
    /// His word is worthless.
    HisWordIsWorthless,
    /// He backed me when nobody else did.
    HeBackedMe,
    /// We clashed.
    WeClashed,

    // ── About a teammate ────────────────────────────────────────
    CloseFriend,
    BadBlood,
    HeMentoredMe,

    // ── About a country ─────────────────────────────────────────
    CountryNeverSuitedMe,
    SettledWellThere,

    // ── About himself ───────────────────────────────────────────
    /// Self-knowledge, held with no particular subject.
    IThriveUnderPressure,
    IStruggleAbroad,
    BigGamesAreMine,
    InjuriesHaveDefinedMe,

    // ═══ What a manager carries between jobs ════════════════════

    // ── About a club he worked for ──────────────────────────────
    TheySackedMe,
    TheyNeverBackedMe,
    TheyKeptTheirWord,
    IBuiltSomethingThere,
    TheSquadWasNeverMine,
    ThatPlaceWasAGraveyard,

    // ── About a board ───────────────────────────────────────────
    TheirWordIsWorthless,
    TheyStoodByMe,

    // ── About a player he coached ───────────────────────────────
    HeRepaidMyFaith,
    HeLetMeDown,
    HeIsWorthBuildingAround,
    /// The one claim in either catalog whose subject is the holder's own
    /// past judgement. It is what lets a coach's eye improve over a
    /// career instead of staying at whatever `CoachProfile` seeded.
    IWasWrongAboutHim,

    // ── About himself ───────────────────────────────────────────
    IAmATeacherNotAWinner,
    IOnlyWorkWithMySquad,
}

impl FactClaim {
    /// Sign of the claim. Drives which way it pushes a decision and how
    /// it colours a recall.
    pub fn valence(self) -> f32 {
        match self {
            FactClaim::None => 0.0,
            FactClaim::BrokeThroughHere => 0.85,
            FactClaim::WonEverythingHere => 0.90,
            FactClaim::WasSoldAgainstMyWill => -0.80,
            FactClaim::DiscardedMe => -0.75,
            FactClaim::ClubStoodByMe => 0.70,
            FactClaim::ClubBrokeItsWord => -0.75,
            FactClaim::NeverPlayedHere => -0.55,
            FactClaim::RelegatedWithThem => -0.45,
            FactClaim::SpiritualHome => 0.95,
            FactClaim::FansAdoredMe => 0.80,
            FactClaim::FansTurnedOnMe => -0.75,
            FactClaim::MadeMeAPlayer => 0.90,
            FactClaim::NeverTrustedMe => -0.70,
            FactClaim::HisWordIsWorthless => -0.85,
            FactClaim::HeBackedMe => 0.75,
            FactClaim::WeClashed => -0.65,
            FactClaim::CloseFriend => 0.80,
            FactClaim::BadBlood => -0.70,
            FactClaim::HeMentoredMe => 0.75,
            FactClaim::CountryNeverSuitedMe => -0.65,
            FactClaim::SettledWellThere => 0.65,
            FactClaim::IThriveUnderPressure => 0.60,
            FactClaim::IStruggleAbroad => -0.55,
            FactClaim::BigGamesAreMine => 0.65,
            FactClaim::InjuriesHaveDefinedMe => -0.60,
            FactClaim::TheySackedMe => -0.70,
            FactClaim::TheyNeverBackedMe => -0.75,
            FactClaim::TheyKeptTheirWord => 0.70,
            FactClaim::IBuiltSomethingThere => 0.85,
            FactClaim::TheSquadWasNeverMine => -0.60,
            FactClaim::ThatPlaceWasAGraveyard => -0.80,
            FactClaim::TheirWordIsWorthless => -0.85,
            FactClaim::TheyStoodByMe => 0.80,
            FactClaim::HeRepaidMyFaith => 0.80,
            FactClaim::HeLetMeDown => -0.70,
            FactClaim::HeIsWorthBuildingAround => 0.75,
            FactClaim::IWasWrongAboutHim => -0.25,
            FactClaim::IAmATeacherNotAWinner => 0.30,
            FactClaim::IOnlyWorkWithMySquad => -0.30,
        }
    }

    /// The claim this one directly contradicts, if any. Corroborating
    /// evidence for a claim erodes its opposite — a player cannot
    /// coherently hold both "he never trusted me" and "he backed me".
    pub fn opposite(self) -> Option<FactClaim> {
        match self {
            FactClaim::ClubStoodByMe => Some(FactClaim::ClubBrokeItsWord),
            FactClaim::ClubBrokeItsWord => Some(FactClaim::ClubStoodByMe),
            FactClaim::FansAdoredMe => Some(FactClaim::FansTurnedOnMe),
            FactClaim::FansTurnedOnMe => Some(FactClaim::FansAdoredMe),
            FactClaim::NeverTrustedMe => Some(FactClaim::HeBackedMe),
            FactClaim::HeBackedMe => Some(FactClaim::NeverTrustedMe),
            FactClaim::MadeMeAPlayer => Some(FactClaim::WeClashed),
            FactClaim::CloseFriend => Some(FactClaim::BadBlood),
            FactClaim::BadBlood => Some(FactClaim::CloseFriend),
            FactClaim::CountryNeverSuitedMe => Some(FactClaim::SettledWellThere),
            FactClaim::SettledWellThere => Some(FactClaim::CountryNeverSuitedMe),
            FactClaim::NeverPlayedHere => Some(FactClaim::BrokeThroughHere),
            FactClaim::TheyNeverBackedMe => Some(FactClaim::TheyKeptTheirWord),
            FactClaim::TheyKeptTheirWord => Some(FactClaim::TheyNeverBackedMe),
            FactClaim::IBuiltSomethingThere => Some(FactClaim::ThatPlaceWasAGraveyard),
            FactClaim::ThatPlaceWasAGraveyard => Some(FactClaim::IBuiltSomethingThere),
            FactClaim::TheirWordIsWorthless => Some(FactClaim::TheyStoodByMe),
            FactClaim::TheyStoodByMe => Some(FactClaim::TheirWordIsWorthless),
            FactClaim::HeRepaidMyFaith => Some(FactClaim::HeLetMeDown),
            FactClaim::HeLetMeDown => Some(FactClaim::HeRepaidMyFaith),
            _ => None,
        }
    }

    /// What kind of actor this claim can be held about. Guards
    /// consolidation from forming "the fans never trusted me".
    pub fn subject_kind(self) -> ActorKind {
        match self {
            FactClaim::None => ActorKind::None,

            FactClaim::BrokeThroughHere
            | FactClaim::WonEverythingHere
            | FactClaim::WasSoldAgainstMyWill
            | FactClaim::DiscardedMe
            | FactClaim::ClubStoodByMe
            | FactClaim::ClubBrokeItsWord
            | FactClaim::NeverPlayedHere
            | FactClaim::RelegatedWithThem
            | FactClaim::SpiritualHome => ActorKind::Club,

            FactClaim::FansAdoredMe | FactClaim::FansTurnedOnMe => ActorKind::Fans,

            FactClaim::MadeMeAPlayer
            | FactClaim::NeverTrustedMe
            | FactClaim::HisWordIsWorthless
            | FactClaim::HeBackedMe
            | FactClaim::WeClashed => ActorKind::Staff,

            FactClaim::CloseFriend | FactClaim::BadBlood | FactClaim::HeMentoredMe => {
                ActorKind::Player
            }

            FactClaim::CountryNeverSuitedMe | FactClaim::SettledWellThere => ActorKind::Country,

            FactClaim::IThriveUnderPressure
            | FactClaim::IStruggleAbroad
            | FactClaim::BigGamesAreMine
            | FactClaim::InjuriesHaveDefinedMe
            | FactClaim::IAmATeacherNotAWinner
            | FactClaim::IOnlyWorkWithMySquad => ActorKind::None,

            FactClaim::TheySackedMe
            | FactClaim::TheyNeverBackedMe
            | FactClaim::TheyKeptTheirWord
            | FactClaim::IBuiltSomethingThere
            | FactClaim::TheSquadWasNeverMine
            | FactClaim::ThatPlaceWasAGraveyard => ActorKind::Club,

            FactClaim::TheirWordIsWorthless | FactClaim::TheyStoodByMe => ActorKind::Board,

            FactClaim::HeRepaidMyFaith
            | FactClaim::HeLetMeDown
            | FactClaim::HeIsWorthBuildingAround
            | FactClaim::IWasWrongAboutHim => ActorKind::Player,
        }
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            FactClaim::None => "mind_fact_none",
            FactClaim::BrokeThroughHere => "mind_fact_broke_through_here",
            FactClaim::WonEverythingHere => "mind_fact_won_everything_here",
            FactClaim::WasSoldAgainstMyWill => "mind_fact_sold_against_will",
            FactClaim::DiscardedMe => "mind_fact_discarded_me",
            FactClaim::ClubStoodByMe => "mind_fact_club_stood_by_me",
            FactClaim::ClubBrokeItsWord => "mind_fact_club_broke_word",
            FactClaim::NeverPlayedHere => "mind_fact_never_played_here",
            FactClaim::RelegatedWithThem => "mind_fact_relegated_with_them",
            FactClaim::SpiritualHome => "mind_fact_spiritual_home",
            FactClaim::FansAdoredMe => "mind_fact_fans_adored",
            FactClaim::FansTurnedOnMe => "mind_fact_fans_turned",
            FactClaim::MadeMeAPlayer => "mind_fact_made_me_a_player",
            FactClaim::NeverTrustedMe => "mind_fact_never_trusted_me",
            FactClaim::HisWordIsWorthless => "mind_fact_word_worthless",
            FactClaim::HeBackedMe => "mind_fact_backed_me",
            FactClaim::WeClashed => "mind_fact_we_clashed",
            FactClaim::CloseFriend => "mind_fact_close_friend",
            FactClaim::BadBlood => "mind_fact_bad_blood",
            FactClaim::HeMentoredMe => "mind_fact_mentored_me",
            FactClaim::CountryNeverSuitedMe => "mind_fact_country_never_suited",
            FactClaim::SettledWellThere => "mind_fact_settled_well",
            FactClaim::IThriveUnderPressure => "mind_fact_thrive_under_pressure",
            FactClaim::IStruggleAbroad => "mind_fact_struggle_abroad",
            FactClaim::BigGamesAreMine => "mind_fact_big_games_mine",
            FactClaim::InjuriesHaveDefinedMe => "mind_fact_injuries_defined_me",
            FactClaim::TheySackedMe => "mind_fact_they_sacked_me",
            FactClaim::TheyNeverBackedMe => "mind_fact_they_never_backed_me",
            FactClaim::TheyKeptTheirWord => "mind_fact_they_kept_their_word",
            FactClaim::IBuiltSomethingThere => "mind_fact_i_built_something_there",
            FactClaim::TheSquadWasNeverMine => "mind_fact_squad_was_never_mine",
            FactClaim::ThatPlaceWasAGraveyard => "mind_fact_place_was_a_graveyard",
            FactClaim::TheirWordIsWorthless => "mind_fact_board_word_worthless",
            FactClaim::TheyStoodByMe => "mind_fact_board_stood_by_me",
            FactClaim::HeRepaidMyFaith => "mind_fact_he_repaid_my_faith",
            FactClaim::HeLetMeDown => "mind_fact_he_let_me_down",
            FactClaim::HeIsWorthBuildingAround => "mind_fact_worth_building_around",
            FactClaim::IWasWrongAboutHim => "mind_fact_i_was_wrong_about_him",
            FactClaim::IAmATeacherNotAWinner => "mind_fact_teacher_not_a_winner",
            FactClaim::IOnlyWorkWithMySquad => "mind_fact_only_my_own_squad",
        }
    }

    pub const ALL: &'static [FactClaim] = &[
        FactClaim::BrokeThroughHere,
        FactClaim::WonEverythingHere,
        FactClaim::WasSoldAgainstMyWill,
        FactClaim::DiscardedMe,
        FactClaim::ClubStoodByMe,
        FactClaim::ClubBrokeItsWord,
        FactClaim::NeverPlayedHere,
        FactClaim::RelegatedWithThem,
        FactClaim::SpiritualHome,
        FactClaim::FansAdoredMe,
        FactClaim::FansTurnedOnMe,
        FactClaim::MadeMeAPlayer,
        FactClaim::NeverTrustedMe,
        FactClaim::HisWordIsWorthless,
        FactClaim::HeBackedMe,
        FactClaim::WeClashed,
        FactClaim::CloseFriend,
        FactClaim::BadBlood,
        FactClaim::HeMentoredMe,
        FactClaim::CountryNeverSuitedMe,
        FactClaim::SettledWellThere,
        FactClaim::IThriveUnderPressure,
        FactClaim::IStruggleAbroad,
        FactClaim::BigGamesAreMine,
        FactClaim::InjuriesHaveDefinedMe,
        FactClaim::TheySackedMe,
        FactClaim::TheyNeverBackedMe,
        FactClaim::TheyKeptTheirWord,
        FactClaim::IBuiltSomethingThere,
        FactClaim::TheSquadWasNeverMine,
        FactClaim::ThatPlaceWasAGraveyard,
        FactClaim::TheirWordIsWorthless,
        FactClaim::TheyStoodByMe,
        FactClaim::HeRepaidMyFaith,
        FactClaim::HeLetMeDown,
        FactClaim::HeIsWorthBuildingAround,
        FactClaim::IWasWrongAboutHim,
        FactClaim::IAmATeacherNotAWinner,
        FactClaim::IOnlyWorkWithMySquad,
    ];
}

/// One thing he believes about someone or somewhere, held indefinitely.
#[derive(Debug, Clone, Copy, Default)]
pub struct SemanticFact {
    pub claim: FactClaim,
    /// Who or what it is about. [`ActorRef::NONE`] for self-knowledge.
    pub subject: ActorRef,
    /// When it first crystallised.
    pub formed: EpochDay,
    /// How firmly held, 0..=100. Grows with corroboration, shrinks when
    /// contradicted. A fact at 0 is dropped.
    strength_pct: u8,
    /// How many episodes have fed it. Saturates — the difference between
    /// eight bad nights and eighty is not meaningful.
    pub support: u8,
}

impl SemanticFact {
    /// Strength gained by one corroborating episode, as a fraction of
    /// the gap to certainty. Diminishing, so the first evidence moves a
    /// fact far more than the tenth.
    pub const CORROBORATION_GAIN: f32 = 0.35;

    /// Strength lost by one contradicting episode, as a fraction of
    /// current strength. Deliberately *slower* than corroboration:
    /// people revise a settled judgement about someone reluctantly.
    pub const CONTRADICTION_LOSS: f32 = 0.22;

    pub fn new(claim: FactClaim, subject: ActorRef, formed: EpochDay, strength: f32) -> Self {
        SemanticFact {
            claim,
            subject,
            formed,
            strength_pct: (strength.clamp(0.0, 1.0) * 100.0).round() as u8,
            support: 1,
        }
    }

    #[inline]
    pub fn strength(&self) -> f32 {
        self.strength_pct as f32 / 100.0
    }

    #[inline]
    pub fn set_strength(&mut self, strength: f32) {
        self.strength_pct = (strength.clamp(0.0, 1.0) * 100.0).round() as u8;
    }

    /// Another episode says the same thing. Approaches certainty
    /// asymptotically.
    pub fn corroborate(&mut self) {
        let s = self.strength();
        self.set_strength(s + (1.0 - s) * Self::CORROBORATION_GAIN);
        self.support = self.support.saturating_add(1);
    }

    /// Evidence to the contrary. Erodes rather than flips — the
    /// opposite claim has to earn its own place.
    pub fn contradict(&mut self) {
        let s = self.strength();
        self.set_strength(s * (1.0 - Self::CONTRADICTION_LOSS));
    }

    /// Below this a fact is no longer meaningfully held and is dropped.
    pub fn is_spent(&self) -> bool {
        self.strength_pct < 8
    }

    /// Signed weight this fact carries into a decision: how firmly it is
    /// held, times which way it points.
    #[inline]
    pub fn weight(&self) -> f32 {
        self.strength() * self.claim.valence()
    }
}

/// The distilled beliefs a player carries about the people and places of
/// his career. Capacity 24 — a career's worth of conclusions, not its
/// events.
pub type SemanticStore = FixedStore<SemanticFact, 24>;

/// Operations over the semantic store. Wrapped rather than free
/// functions so the call sites read as directives.
pub struct Semantic;

impl Semantic {
    /// Record corroborating evidence for `claim` about `subject`,
    /// forming the fact if it is new and eroding its opposite.
    ///
    /// Returns `true` if a new fact was formed (as opposed to an
    /// existing one being strengthened).
    pub fn assert(
        store: &mut SemanticStore,
        claim: FactClaim,
        subject: ActorRef,
        today: EpochDay,
        initial_strength: f32,
    ) -> bool {
        debug_assert_eq!(
            claim.subject_kind(),
            subject.kind,
            "{claim:?} cannot be held about a {:?}",
            subject.kind
        );

        // Erode the contradicting belief first — a man who is being shown
        // he was wrong loosens the old conviction as he forms the new one.
        if let Some(opposite) = claim.opposite() {
            if let Some(existing) = store.find_mut(|f| f.claim == opposite && f.subject == subject)
            {
                existing.contradict();
            }
        }
        store.retain(|f| !f.is_spent());

        if let Some(existing) = store.find_mut(|f| f.claim == claim && f.subject == subject) {
            existing.corroborate();
            return false;
        }

        let fact = SemanticFact::new(claim, subject, today, initial_strength);
        // A new conviction displaces the weakest one held, if the store
        // is full. Nothing is protected here: facts earn their place by
        // strength alone.
        store.push_evicting(fact, |f| f.strength(), |_| true);
        true
    }

    /// Every fact held about `subject`, strongest first.
    pub fn about(store: &SemanticStore, subject: ActorRef) -> Vec<SemanticFact> {
        let mut facts: Vec<SemanticFact> = store
            .iter()
            .filter(|f| f.subject == subject)
            .copied()
            .collect();
        facts.sort_by(|a, b| {
            b.strength()
                .partial_cmp(&a.strength())
                .unwrap_or(Ordering::Equal)
        });
        facts
    }

    /// Is `claim` held about `subject`, and how firmly? 0.0 when absent.
    pub fn strength_of(store: &SemanticStore, claim: FactClaim, subject: ActorRef) -> f32 {
        store
            .find(|f| f.claim == claim && f.subject == subject)
            .map(|f| f.strength())
            .unwrap_or(0.0)
    }

    /// Net sentiment toward `subject` from held facts alone, -1..+1.
    /// This is the part of an opinion that survives after every episode
    /// behind it has faded.
    pub fn sentiment(store: &SemanticStore, subject: ActorRef) -> f32 {
        let (sum, count) = store
            .iter()
            .filter(|f| f.subject == subject)
            .fold((0.0f32, 0u32), |(sum, count), f| {
                (sum + f.weight(), count + 1)
            });
        if count == 0 {
            return 0.0;
        }
        // Mean rather than sum: a player with five convictions about a
        // club does not feel five times as strongly as one with a single
        // conviction of the same force.
        (sum / count as f32).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn club(id: u32) -> ActorRef {
        ActorRef::club(id)
    }

    #[test]
    fn every_claim_has_a_unique_key_and_a_sane_valence() {
        let mut keys: Vec<&str> = FactClaim::ALL.iter().map(|c| c.as_i18n_key()).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len());

        for claim in FactClaim::ALL {
            assert!(claim.valence().abs() > 0.0, "{claim:?} has no direction");
            assert!((-1.0..=1.0).contains(&claim.valence()));
        }
    }

    #[test]
    fn opposites_are_symmetric_where_declared() {
        for claim in FactClaim::ALL {
            if let Some(opposite) = claim.opposite() {
                assert!(
                    claim.valence() * opposite.valence() < 0.0,
                    "{claim:?} and its opposite {opposite:?} must point opposite ways"
                );
            }
        }
    }

    #[test]
    fn asserting_forms_then_strengthens() {
        let mut store = SemanticStore::new();
        assert!(Semantic::assert(
            &mut store,
            FactClaim::BrokeThroughHere,
            club(7),
            100,
            0.5
        ));
        let first = Semantic::strength_of(&store, FactClaim::BrokeThroughHere, club(7));

        assert!(!Semantic::assert(
            &mut store,
            FactClaim::BrokeThroughHere,
            club(7),
            110,
            0.5
        ));
        let second = Semantic::strength_of(&store, FactClaim::BrokeThroughHere, club(7));

        assert!(
            second > first,
            "corroboration strengthens: {first} → {second}"
        );
        assert_eq!(store.len(), 1, "corroboration must not duplicate the fact");
    }

    #[test]
    fn corroboration_has_diminishing_returns() {
        let mut store = SemanticStore::new();
        Semantic::assert(
            &mut store,
            FactClaim::FansTurnedOnMe,
            ActorRef::fans(7),
            0,
            0.4,
        );
        let mut prior = Semantic::strength_of(&store, FactClaim::FansTurnedOnMe, ActorRef::fans(7));
        let mut first_gain = 0.0;
        for i in 0..6 {
            Semantic::assert(
                &mut store,
                FactClaim::FansTurnedOnMe,
                ActorRef::fans(7),
                0,
                0.4,
            );
            let now = Semantic::strength_of(&store, FactClaim::FansTurnedOnMe, ActorRef::fans(7));
            let gain = now - prior;
            if i == 0 {
                first_gain = gain;
            } else {
                assert!(gain <= first_gain, "gains must not grow");
            }
            prior = now;
        }
        assert!(prior < 1.001);
    }

    #[test]
    fn contradiction_erodes_the_opposite_belief() {
        let mut store = SemanticStore::new();
        let coach = ActorRef::staff(412);
        for _ in 0..4 {
            Semantic::assert(&mut store, FactClaim::NeverTrustedMe, coach, 0, 0.5);
        }
        let entrenched = Semantic::strength_of(&store, FactClaim::NeverTrustedMe, coach);

        Semantic::assert(&mut store, FactClaim::HeBackedMe, coach, 100, 0.5);
        let after = Semantic::strength_of(&store, FactClaim::NeverTrustedMe, coach);

        assert!(after < entrenched, "the old conviction loosens");
        assert!(
            after > 0.0,
            "but one contrary act does not erase years of it"
        );
    }

    #[test]
    fn a_settled_judgement_takes_repeated_evidence_to_overturn() {
        let mut store = SemanticStore::new();
        let coach = ActorRef::staff(9);
        for _ in 0..6 {
            Semantic::assert(&mut store, FactClaim::NeverTrustedMe, coach, 0, 0.5);
        }
        for _ in 0..12 {
            Semantic::assert(&mut store, FactClaim::HeBackedMe, coach, 100, 0.5);
        }
        assert!(
            Semantic::strength_of(&store, FactClaim::HeBackedMe, coach)
                > Semantic::strength_of(&store, FactClaim::NeverTrustedMe, coach),
            "sustained contrary evidence eventually wins"
        );
    }

    #[test]
    fn sentiment_survives_with_no_episodes_at_all() {
        // The ten-year property in miniature: the facts alone carry an
        // opinion about a club.
        let mut store = SemanticStore::new();
        Semantic::assert(&mut store, FactClaim::BrokeThroughHere, club(7), 0, 0.9);
        Semantic::assert(&mut store, FactClaim::WonEverythingHere, club(7), 0, 0.8);
        assert!(Semantic::sentiment(&store, club(7)) > 0.5);

        Semantic::assert(&mut store, FactClaim::WasSoldAgainstMyWill, club(9), 0, 0.9);
        assert!(Semantic::sentiment(&store, club(9)) < -0.5);

        assert_eq!(
            Semantic::sentiment(&store, club(11)),
            0.0,
            "a club he has no history with is neutral"
        );
    }

    #[test]
    fn facts_do_not_decay_with_time() {
        // Explicitly asserted because it is the whole design: nothing in
        // this module takes `now` for the purpose of weakening a fact.
        let mut store = SemanticStore::new();
        Semantic::assert(&mut store, FactClaim::SpiritualHome, club(7), 0, 0.9);
        let strength = Semantic::strength_of(&store, FactClaim::SpiritualHome, club(7));
        // No tick, no decay pass, no time-based call exists to make.
        assert_eq!(
            Semantic::strength_of(&store, FactClaim::SpiritualHome, club(7)),
            strength
        );
    }

    #[test]
    fn store_evicts_the_weakest_conviction_when_full() {
        let mut store = SemanticStore::new();
        // Fill with weakly-held facts about distinct clubs.
        for id in 0..store.capacity() as u32 {
            Semantic::assert(&mut store, FactClaim::NeverPlayedHere, club(id), 0, 0.2);
        }
        assert!(store.is_full());

        Semantic::assert(&mut store, FactClaim::SpiritualHome, club(999), 0, 0.95);
        assert!(
            Semantic::strength_of(&store, FactClaim::SpiritualHome, club(999)) > 0.9,
            "a strong new conviction displaces a weak old one"
        );
        assert_eq!(store.len(), store.capacity());
    }
}
