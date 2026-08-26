//! Consolidation — banking what the episodes meant before they fade.
//!
//! This is the pass that makes a whole career fit in about a kilobyte.
//! Run monthly, it does two jobs:
//!
//! 1. **Abstraction.** *n* episodes saying the same thing about the same
//!    subject collapse into one [`SemanticFact`]. Eight hostile nights
//!    from one club's support become `FansTurnedOnMe`, and the eight
//!    episodes are then free to fade without losing anything. A few
//!    events are strong enough to form a fact on their own — being sold
//!    against his will needs to happen exactly once.
//! 2. **Forgetting.** Episodes whose retention has fallen through the
//!    faint line are dropped, protected slots excepted.
//!
//! The ordering matters and is the whole trick: **meaning is banked
//! before the evidence is discarded.** Reverse the two and a player
//! would forget his career instead of learning from it.
//!
//! [`SemanticFact`]: super::semantic::SemanticFact

use super::actor::{ActorKind, ActorRef};
use super::episode::{EpisodeFlags, EpisodeKind, MindEpisode};
use super::epoch::EpochDay;
use super::forgetting::ForgettingCurve;
use super::ledger::{AttributionLedger, Ledger};
use super::semantic::{FactClaim, Semantic, SemanticStore};
use super::store::FixedStore;
use std::cmp::Ordering;

/// Who a consolidated fact ends up being *about*. Often not the
/// episode's own actor: a debut has no actor at all, but it makes the
/// club where it happened the place he broke through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactSubject {
    /// The episode's own actor.
    Actor,
    /// The club he was at when it happened.
    Club,
    /// That club's supporters.
    Fans,
    /// No subject — a piece of self-knowledge.
    Himself,
}

/// How one kind of episode turns into a belief.
#[derive(Debug, Clone, Copy)]
pub struct ConsolidationRule {
    pub claim: FactClaim,
    pub subject: FactSubject,
    /// How many such episodes it takes before the pattern registers as
    /// a conviction. One bad night is a bad night; six is what the place
    /// was like.
    pub support_needed: u8,
    /// Encoding at or above which a single episode forms the belief
    /// outright. Some things only need to happen once.
    pub instant_at: f32,
}

impl ConsolidationRule {
    /// A belief that only registers once a pattern has repeated. One bad
    /// night is a bad night; `support_needed` of them is what the place
    /// was like.
    pub const fn pattern(claim: FactClaim, subject: FactSubject, support_needed: u8) -> Self {
        ConsolidationRule {
            claim,
            subject,
            support_needed,
            // Unreachable by any encoding, so only repetition can form it.
            instant_at: f32::INFINITY,
        }
    }

    /// A belief one event is enough for. Being sold against your will
    /// needs to happen exactly once.
    pub const fn single(claim: FactClaim, subject: FactSubject) -> Self {
        ConsolidationRule {
            claim,
            subject,
            support_needed: 1,
            instant_at: 0.0,
        }
    }
}

/// Whose mind is doing the consolidating.
///
/// The same event does not mean the same thing to the two people it
/// happened to. A title won at a club is `WonEverythingHere` to the man
/// who played in it and `IBuiltSomethingThere` to the man who picked
/// the side. One catalog of episodes, two readings of it — which is
/// exactly why the reading is a parameter rather than a second catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MindHolder {
    #[default]
    Player,
    Staff,
}

impl EpisodeKind {
    /// What believing this episode repeatedly amounts to, for the mind
    /// holding it. `None` for the many episodes that are simply events
    /// and never become convictions.
    pub fn consolidates_to(self, holder: MindHolder) -> Option<ConsolidationRule> {
        match holder {
            MindHolder::Player => self.consolidates_for_player(),
            MindHolder::Staff => self.consolidates_for_staff(),
        }
    }

    /// The player reading.
    fn consolidates_for_player(self) -> Option<ConsolidationRule> {
        use ConsolidationRule as R;

        match self {
            // ── Club ────────────────────────────────────────────
            EpisodeKind::SeniorDebut => {
                Some(R::single(FactClaim::BrokeThroughHere, FactSubject::Club))
            }
            EpisodeKind::FirstGoalForClub => Some(R::pattern(
                FactClaim::BrokeThroughHere,
                FactSubject::Club,
                3,
            )),
            EpisodeKind::WonLeagueTitle | EpisodeKind::WonContinentalTrophy => {
                Some(R::single(FactClaim::WonEverythingHere, FactSubject::Club))
            }
            EpisodeKind::WonDomesticCup => Some(R::pattern(
                FactClaim::WonEverythingHere,
                FactSubject::Club,
                2,
            )),
            EpisodeKind::SoldAgainstWill => Some(R::single(
                FactClaim::WasSoldAgainstMyWill,
                FactSubject::Club,
            )),
            EpisodeKind::ReleasedByClub => {
                Some(R::single(FactClaim::DiscardedMe, FactSubject::Club))
            }
            EpisodeKind::ManagerFrozenOut => {
                Some(R::pattern(FactClaim::DiscardedMe, FactSubject::Club, 3))
            }
            EpisodeKind::ClubBrokeWagePromise => {
                Some(R::single(FactClaim::ClubBrokeItsWord, FactSubject::Club))
            }
            EpisodeKind::ContractRenewed => {
                Some(R::pattern(FactClaim::ClubStoodByMe, FactSubject::Club, 3))
            }
            EpisodeKind::ReturnedFromLongInjury => {
                Some(R::pattern(FactClaim::ClubStoodByMe, FactSubject::Club, 2))
            }
            EpisodeKind::Relegated => {
                Some(R::single(FactClaim::RelegatedWithThem, FactSubject::Club))
            }
            EpisodeKind::ClubServantMilestone => {
                Some(R::single(FactClaim::SpiritualHome, FactSubject::Club))
            }
            EpisodeKind::LeftOutOfBigMatch => {
                Some(R::pattern(FactClaim::NeverPlayedHere, FactSubject::Club, 8))
            }

            // ── Supporters ──────────────────────────────────────
            EpisodeKind::FansAdoration => {
                Some(R::pattern(FactClaim::FansAdoredMe, FactSubject::Fans, 4))
            }
            EpisodeKind::FansHostility => {
                Some(R::pattern(FactClaim::FansTurnedOnMe, FactSubject::Fans, 4))
            }

            // ── The manager ─────────────────────────────────────
            EpisodeKind::ManagerPromiseBroken => Some(R::pattern(
                FactClaim::HisWordIsWorthless,
                FactSubject::Actor,
                2,
            )),
            EpisodeKind::ManagerPromiseKept => {
                Some(R::pattern(FactClaim::HeBackedMe, FactSubject::Actor, 3))
            }
            EpisodeKind::ManagerPrivateBacking => {
                Some(R::pattern(FactClaim::HeBackedMe, FactSubject::Actor, 3))
            }
            EpisodeKind::ManagerPublicCriticism => {
                Some(R::pattern(FactClaim::WeClashed, FactSubject::Actor, 4))
            }
            EpisodeKind::ManagerSignedARival => {
                Some(R::pattern(FactClaim::NeverTrustedMe, FactSubject::Actor, 3))
            }
            EpisodeKind::RoleUpgraded => {
                Some(R::pattern(FactClaim::MadeMeAPlayer, FactSubject::Actor, 3))
            }

            // ── Teammates ───────────────────────────────────────
            EpisodeKind::TeammateBefriended => {
                Some(R::pattern(FactClaim::CloseFriend, FactSubject::Actor, 3))
            }
            EpisodeKind::TeammateConflict => {
                Some(R::pattern(FactClaim::BadBlood, FactSubject::Actor, 3))
            }
            EpisodeKind::MentorSupport => {
                Some(R::pattern(FactClaim::HeMentoredMe, FactSubject::Actor, 3))
            }

            // ── Himself ─────────────────────────────────────────
            EpisodeKind::DerbyWin | EpisodeKind::StartedBigMatch => Some(R::pattern(
                FactClaim::BigGamesAreMine,
                FactSubject::Himself,
                6,
            )),
            EpisodeKind::FeltIsolated => Some(R::pattern(
                FactClaim::IStruggleAbroad,
                FactSubject::Himself,
                5,
            )),
            EpisodeKind::CareerThreateningInjury => Some(R::single(
                FactClaim::InjuriesHaveDefinedMe,
                FactSubject::Himself,
            )),
            EpisodeKind::SeriousInjury => Some(R::pattern(
                FactClaim::InjuriesHaveDefinedMe,
                FactSubject::Himself,
                4,
            )),

            _ => None,
        }
    }

    /// The manager reading.
    ///
    /// Where a kind appears in both readings — a title, a relegation,
    /// the supporters — the claim it forms differs, because what the
    /// season taught the two men differs. Where it appears in neither,
    /// it stays an event.
    fn consolidates_for_staff(self) -> Option<ConsolidationRule> {
        use ConsolidationRule as R;

        match self {
            // ── What a club was like to work for ────────────────
            EpisodeKind::SackedByClub => {
                Some(R::single(FactClaim::TheySackedMe, FactSubject::Club))
            }
            EpisodeKind::WonLeagueTitle | EpisodeKind::Promoted => Some(R::single(
                FactClaim::IBuiltSomethingThere,
                FactSubject::Club,
            )),
            EpisodeKind::WonContinentalTrophy => Some(R::single(
                FactClaim::IBuiltSomethingThere,
                FactSubject::Club,
            )),
            EpisodeKind::WonDomesticCup | EpisodeKind::SurvivedARelegationFight => Some(
                R::pattern(FactClaim::IBuiltSomethingThere, FactSubject::Club, 2),
            ),
            EpisodeKind::WonManagerOfTheMonth => Some(R::pattern(
                FactClaim::IBuiltSomethingThere,
                FactSubject::Club,
                4,
            )),
            EpisodeKind::Relegated | EpisodeKind::FailedToSurviveIt => Some(R::pattern(
                FactClaim::ThatPlaceWasAGraveyard,
                FactSubject::Club,
                2,
            )),
            // The four-refusals rule. This is the conviction a manager
            // carries back to a club a decade later, and it is formed
            // from nothing more dramatic than being told no repeatedly.
            EpisodeKind::BoardRefusedMyTarget => Some(R::pattern(
                FactClaim::TheyNeverBackedMe,
                FactSubject::Club,
                4,
            )),
            EpisodeKind::BoardSoldMyBestPlayer => Some(R::pattern(
                FactClaim::TheyNeverBackedMe,
                FactSubject::Club,
                2,
            )),
            EpisodeKind::BoardBackedMeInTheWindow => Some(R::pattern(
                FactClaim::TheyKeptTheirWord,
                FactSubject::Club,
                3,
            )),
            EpisodeKind::LostTheDressingRoom => Some(R::single(
                FactClaim::TheSquadWasNeverMine,
                FactSubject::Club,
            )),
            EpisodeKind::SignedAPlayerIDidNotWant => Some(R::pattern(
                FactClaim::TheSquadWasNeverMine,
                FactSubject::Club,
                3,
            )),

            // ── The people above him ────────────────────────────
            EpisodeKind::BoardBrokeItsPromise | EpisodeKind::ChairmanUndercutMePublicly => Some(
                R::pattern(FactClaim::TheirWordIsWorthless, FactSubject::Actor, 2),
            ),
            EpisodeKind::BoardKeptItsPromise => {
                Some(R::pattern(FactClaim::TheyStoodByMe, FactSubject::Actor, 3))
            }

            // ── Players he coached ──────────────────────────────
            EpisodeKind::PlayerRepaidMyFaith => Some(R::pattern(
                FactClaim::HeRepaidMyFaith,
                FactSubject::Actor,
                2,
            )),
            EpisodeKind::PlayerRefusedToPlayForMe => {
                Some(R::single(FactClaim::HeLetMeDown, FactSubject::Actor))
            }

            // ── Supporters ──────────────────────────────────────
            EpisodeKind::SupportersSangMyName => {
                Some(R::pattern(FactClaim::FansAdoredMe, FactSubject::Fans, 4))
            }
            EpisodeKind::SupportersTurnedOnMe => {
                Some(R::pattern(FactClaim::FansTurnedOnMe, FactSubject::Fans, 4))
            }

            // ── Himself ─────────────────────────────────────────
            // A man whose gambles on players keep coming off decides
            // that is what he is for.
            EpisodeKind::MyGambleCameOff => Some(R::pattern(
                FactClaim::IAmATeacherNotAWinner,
                FactSubject::Himself,
                5,
            )),
            // And a man who only ever settles with his own signings
            // learns that about himself too.
            EpisodeKind::SignedAPlayerIWanted => Some(R::pattern(
                FactClaim::IOnlyWorkWithMySquad,
                FactSubject::Himself,
                6,
            )),
            EpisodeKind::Bereavement => None,

            _ => None,
        }
    }
}

/// The episodic store: 32 slots, six of which flashbulb memories hold
/// against eviction.
pub type EpisodeStore = FixedStore<MindEpisode, 32>;

/// Runs the monthly consolidation pass.
pub struct Consolidator;

/// A belief that crystallised on this pass, and who it is about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FormedFact {
    pub claim: FactClaim,
    pub subject: ActorRef,
}

/// What one pass did. Returned for the census harness and the tests;
/// callers are free to ignore it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConsolidationReport {
    /// Beliefs newly formed.
    pub facts_formed: u16,
    /// Beliefs strengthened by further evidence.
    pub facts_reinforced: u16,
    /// Episodes whose meaning was banked.
    pub episodes_consolidated: u16,
    /// Episodes dropped for having faded.
    pub episodes_forgotten: u16,
    /// Which beliefs those were.
    formed: [FormedFact; Self::MAX_LISTED],
    formed_count: u8,
}

impl ConsolidationReport {
    /// How many newly formed beliefs the report *names*. `facts_formed`
    /// stays authoritative; a pass that formed more than this counts
    /// them all and names the first few.
    ///
    /// Four is generous for a monthly pass: a mind that concluded five
    /// separate new things about its life in one month is not a mind
    /// consolidating, it is a mind being seeded.
    pub const MAX_LISTED: usize = 4;

    /// The beliefs that crystallised on this pass.
    pub fn formed(&self) -> impl Iterator<Item = FormedFact> + '_ {
        self.formed[..self.formed_count as usize].iter().copied()
    }

    fn push_formed(&mut self, claim: FactClaim, subject: ActorRef) {
        if (self.formed_count as usize) < Self::MAX_LISTED {
            self.formed[self.formed_count as usize] = FormedFact { claim, subject };
            self.formed_count += 1;
        }
    }
}

impl Consolidator {
    /// Maximum flashbulb episodes held against eviction. The rest of the
    /// store competes on live retention.
    pub const PROTECTED_SLOTS: usize = 6;

    /// Resolve the subject a rule's fact is held about, for one episode.
    fn subject_of(episode: &MindEpisode, rule: &ConsolidationRule) -> ActorRef {
        match rule.subject {
            FactSubject::Actor => episode.who,
            FactSubject::Club => {
                if episode.where_club == 0 {
                    ActorRef::NONE
                } else {
                    ActorRef::club(episode.where_club)
                }
            }
            FactSubject::Fans => {
                if episode.where_club == 0 {
                    ActorRef::NONE
                } else {
                    ActorRef::fans(episode.where_club)
                }
            }
            FactSubject::Himself => ActorRef::NONE,
        }
    }

    /// Is this episode/subject pair coherent enough to form the belief?
    /// Guards against forming, say, "he never trusted me" about a club.
    fn subject_is_valid(subject: ActorRef, claim: FactClaim) -> bool {
        let wanted = claim.subject_kind();
        if wanted == ActorKind::None {
            // Self-knowledge: no subject, and none expected.
            return subject.is_none();
        }
        subject.kind == wanted
    }

    /// Bank meaning, then forget what has faded. Returns what it did.
    pub fn run(
        episodes: &mut EpisodeStore,
        semantic: &mut SemanticStore,
        ledger: &mut AttributionLedger,
        now: EpochDay,
        holder: MindHolder,
        professionalism: f32,
        consistency: f32,
        temperament: f32,
    ) -> ConsolidationReport {
        let mut report = ConsolidationReport::default();

        Self::abstract_meaning(episodes, semantic, now, holder, &mut report);
        Self::forget_faded(
            episodes,
            now,
            professionalism,
            consistency,
            temperament,
            &mut report,
        );
        // Prune last, and with the semantic store in hand: an account a
        // conviction is holding up must survive this pass.
        Ledger::prune(ledger, semantic, now);

        report
    }

    /// Job one: collapse repeated episodes into convictions.
    fn abstract_meaning(
        episodes: &mut EpisodeStore,
        semantic: &mut SemanticStore,
        now: EpochDay,
        holder: MindHolder,
        report: &mut ConsolidationReport,
    ) {
        // O(n²) over at most 32 slots, once a month. Cheaper than any
        // grouping structure that would allocate.
        for index in 0..episodes.len() {
            let candidate = match episodes.get(index) {
                Some(ep) if !ep.is_consolidated() && ep.kind != EpisodeKind::None => *ep,
                _ => continue,
            };
            // An outcome he is still waiting on cannot yet have taught
            // him anything.
            if candidate.flags.contains(EpisodeFlags::UNRESOLVED) {
                continue;
            }

            let Some(rule) = candidate.kind.consolidates_to(holder) else {
                continue;
            };
            let subject = Self::subject_of(&candidate, &rule);
            if !Self::subject_is_valid(subject, rule.claim) {
                continue;
            }

            // Everything unconsolidated that says the same thing about
            // the same subject.
            let mut support = 0u8;
            let mut strongest = 0.0f32;
            for other in episodes.iter() {
                if other.kind != candidate.kind || other.is_consolidated() {
                    continue;
                }
                if Self::subject_of(other, &rule) != subject {
                    continue;
                }
                support = support.saturating_add(1);
                strongest = strongest.max(other.encoding());
            }

            let by_repetition = support >= rule.support_needed;
            let by_force = strongest >= rule.instant_at;
            if !(by_repetition || by_force) {
                continue;
            }

            // Initial conviction scales with how hard the evidence
            // landed — a belief formed from vivid episodes starts firmer.
            let formed = Semantic::assert(semantic, rule.claim, subject, now, strongest.max(0.35));
            if formed {
                report.facts_formed += 1;
                report.push_formed(rule.claim, subject);
            } else {
                report.facts_reinforced += 1;
            }

            // Bank every episode that fed it.
            for other in episodes.iter_mut() {
                if other.kind == candidate.kind
                    && !other.is_consolidated()
                    && Self::subject_of(other, &rule) == subject
                {
                    other.flags.insert(EpisodeFlags::CONSOLIDATED);
                    report.episodes_consolidated += 1;
                }
            }
        }
    }

    /// Job two: drop what has genuinely faded. Flashbulb episodes are
    /// exempt up to [`Self::PROTECTED_SLOTS`]; past that even they
    /// compete, weakest first.
    fn forget_faded(
        episodes: &mut EpisodeStore,
        now: EpochDay,
        professionalism: f32,
        consistency: f32,
        temperament: f32,
        report: &mut ConsolidationReport,
    ) {
        let retention_of = |ep: &MindEpisode| -> f32 {
            let beta =
                ForgettingCurve::beta(professionalism, consistency, temperament, ep.valence());
            ForgettingCurve::retention_protected(
                ep.encoding(),
                ep.last_touched,
                now,
                beta,
                ep.is_flashbulb(),
            )
        };

        // Rank flashbulbs so only the strongest keep their protection —
        // otherwise a career of landmarks would fill the store and lock
        // out everything recent.
        let mut flashbulb_strength: Vec<f32> = episodes
            .iter()
            .filter(|ep| ep.is_flashbulb())
            .map(retention_of)
            .collect();
        flashbulb_strength.sort_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
        let protection_cutoff = flashbulb_strength
            .get(Self::PROTECTED_SLOTS - 1)
            .copied()
            .unwrap_or(0.0);

        let before = episodes.len();
        episodes.retain(|ep| {
            let retention = retention_of(ep);
            // A protected landmark is kept regardless of the faint line.
            if ep.is_flashbulb() && retention >= protection_cutoff {
                return true;
            }
            retention >= ForgettingCurve::FAINT
        });
        report.episodes_forgotten = (before - episodes.len()) as u16;
    }

    /// Eviction rank used when a new episode arrives at a full store:
    /// live retention, with consolidated episodes discounted because
    /// their meaning is already banked elsewhere.
    pub fn eviction_rank(
        ep: &MindEpisode,
        now: EpochDay,
        professionalism: f32,
        consistency: f32,
        temperament: f32,
    ) -> f32 {
        let beta = ForgettingCurve::beta(professionalism, consistency, temperament, ep.valence());
        let retention = ForgettingCurve::retention_protected(
            ep.encoding(),
            ep.last_touched,
            now,
            beta,
            ep.is_flashbulb(),
        );
        if ep.is_consolidated() {
            retention * 0.5
        } else {
            retention
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::episode::EncodingInputs;
    use super::*;

    const YEAR: EpochDay = 365;

    fn episode(kind: EpisodeKind, who: ActorRef, club: u32, when: EpochDay) -> MindEpisode {
        let spec = kind.spec();
        MindEpisode::new(
            kind,
            who,
            club,
            when,
            spec.valence,
            EncodingInputs::neutral(spec.intensity).strength(),
        )
    }

    /// Same, with an explicit encoding — for the cases that turn on how
    /// deeply something landed rather than what it was.
    fn faint_episode(
        kind: EpisodeKind,
        who: ActorRef,
        club: u32,
        when: EpochDay,
        encoding: f32,
    ) -> MindEpisode {
        MindEpisode::new(kind, who, club, when, kind.spec().valence, encoding)
    }

    fn run(
        episodes: &mut EpisodeStore,
        semantic: &mut SemanticStore,
        ledger: &mut AttributionLedger,
        now: EpochDay,
    ) -> ConsolidationReport {
        Consolidator::run(
            episodes,
            semantic,
            ledger,
            now,
            MindHolder::Player,
            12.0,
            12.0,
            10.0,
        )
    }

    #[test]
    fn a_single_forceful_event_forms_a_belief_at_once() {
        let mut episodes = EpisodeStore::new();
        let mut semantic = SemanticStore::new();
        let mut ledger = AttributionLedger::new();

        episodes.push(episode(
            EpisodeKind::SoldAgainstWill,
            ActorRef::club(7),
            7,
            100,
        ));
        let report = run(&mut episodes, &mut semantic, &mut ledger, 130);

        assert_eq!(report.facts_formed, 1);
        assert!(
            Semantic::strength_of(
                &semantic,
                FactClaim::WasSoldAgainstMyWill,
                ActorRef::club(7)
            ) > 0.0,
            "being sold against your will needs to happen exactly once"
        );
    }

    #[test]
    fn repetition_is_required_for_a_pattern() {
        let mut episodes = EpisodeStore::new();
        let mut semantic = SemanticStore::new();
        let mut ledger = AttributionLedger::new();
        let fans = ActorRef::fans(7);

        for day in 0..3 {
            episodes.push(episode(EpisodeKind::FansHostility, fans, 7, day * 10));
        }
        run(&mut episodes, &mut semantic, &mut ledger, 100);
        assert_eq!(
            Semantic::strength_of(&semantic, FactClaim::FansTurnedOnMe, fans),
            0.0,
            "three bad nights is three bad nights"
        );

        episodes.push(episode(EpisodeKind::FansHostility, fans, 7, 40));
        run(&mut episodes, &mut semantic, &mut ledger, 100);
        assert!(
            Semantic::strength_of(&semantic, FactClaim::FansTurnedOnMe, fans) > 0.0,
            "four is what the place was like"
        );
    }

    #[test]
    fn meaning_is_banked_before_the_evidence_is_discarded() {
        // The ordering guarantee, asserted directly. Episodes old enough
        // to be forgotten this pass must still have taught him something.
        let mut episodes = EpisodeStore::new();
        let mut semantic = SemanticStore::new();
        let mut ledger = AttributionLedger::new();
        let fans = ActorRef::fans(7);

        // Five nights that barely registered at the time.
        for i in 0..5 {
            episodes.push(faint_episode(EpisodeKind::FansHostility, fans, 7, i, 0.30));
        }
        // A decade on they are past the faint line and dropped.
        let report = run(&mut episodes, &mut semantic, &mut ledger, YEAR * 10);

        assert!(report.episodes_forgotten > 0, "they should have faded");
        assert!(
            Semantic::strength_of(&semantic, FactClaim::FansTurnedOnMe, fans) > 0.0,
            "but the conviction they produced survives them"
        );
    }

    #[test]
    fn consolidation_is_idempotent() {
        let mut episodes = EpisodeStore::new();
        let mut semantic = SemanticStore::new();
        let mut ledger = AttributionLedger::new();

        episodes.push(episode(EpisodeKind::SeniorDebut, ActorRef::NONE, 7, 10));
        let first = run(&mut episodes, &mut semantic, &mut ledger, 20);
        let strength_after_first =
            Semantic::strength_of(&semantic, FactClaim::BrokeThroughHere, ActorRef::club(7));

        let second = run(&mut episodes, &mut semantic, &mut ledger, 21);
        let strength_after_second =
            Semantic::strength_of(&semantic, FactClaim::BrokeThroughHere, ActorRef::club(7));

        assert_eq!(first.facts_formed, 1);
        assert_eq!(second.facts_formed, 0, "a banked episode is not re-banked");
        assert_eq!(second.facts_reinforced, 0);
        assert_eq!(strength_after_first, strength_after_second);
    }

    #[test]
    fn an_unresolved_episode_teaches_nothing_yet() {
        let mut episodes = EpisodeStore::new();
        let mut semantic = SemanticStore::new();
        let mut ledger = AttributionLedger::new();

        let mut pending = episode(EpisodeKind::SoldAgainstWill, ActorRef::club(7), 7, 10);
        pending.flags.insert(EpisodeFlags::UNRESOLVED);
        episodes.push(pending);

        let report = run(&mut episodes, &mut semantic, &mut ledger, 20);
        assert_eq!(report.facts_formed, 0);
    }

    #[test]
    fn a_belief_is_never_formed_about_the_wrong_kind_of_subject() {
        // A debut with no club recorded has nowhere to hang the fact.
        let mut episodes = EpisodeStore::new();
        let mut semantic = SemanticStore::new();
        let mut ledger = AttributionLedger::new();

        episodes.push(episode(EpisodeKind::SeniorDebut, ActorRef::NONE, 0, 10));
        let report = run(&mut episodes, &mut semantic, &mut ledger, 20);
        assert_eq!(report.facts_formed, 0);
        assert!(semantic.is_empty());
    }

    #[test]
    fn every_rule_targets_a_coherent_subject() {
        // Structural audit: a rule that names a subject the claim cannot
        // be held about would silently never fire.
        for kind in EpisodeKind::ALL {
            for holder in [MindHolder::Player, MindHolder::Staff] {
                let Some(rule) = kind.consolidates_to(holder) else {
                    continue;
                };
                let wanted = rule.claim.subject_kind();
                let produced = match rule.subject {
                    FactSubject::Actor => wanted, // emit site supplies it; checked at runtime
                    FactSubject::Club => ActorKind::Club,
                    FactSubject::Fans => ActorKind::Fans,
                    FactSubject::Himself => ActorKind::None,
                };
                assert_eq!(
                    produced, wanted,
                    "{kind:?} consolidates to {:?} for {holder:?}, which cannot be held about {:?}",
                    rule.claim, rule.subject
                );
            }
        }
    }

    #[test]
    fn flashbulb_landmarks_outlive_ordinary_episodes() {
        let mut episodes = EpisodeStore::new();
        let mut semantic = SemanticStore::new();
        let mut ledger = AttributionLedger::new();

        episodes.push(episode(EpisodeKind::SeniorDebut, ActorRef::NONE, 7, 10));
        episodes.push(episode(EpisodeKind::MediaPraise, ActorRef::NONE, 7, 10));

        run(&mut episodes, &mut semantic, &mut ledger, YEAR * 10);

        assert!(
            episodes
                .find(|e| e.kind == EpisodeKind::SeniorDebut)
                .is_some(),
            "he remembers his debut a decade on"
        );
        assert!(
            episodes
                .find(|e| e.kind == EpisodeKind::MediaPraise)
                .is_none(),
            "he does not remember a newspaper writing something nice"
        );
    }

    #[test]
    fn consolidated_episodes_are_cheaper_to_evict() {
        let banked = {
            let mut ep = episode(EpisodeKind::FansHostility, ActorRef::fans(7), 7, 100);
            ep.flags.insert(EpisodeFlags::CONSOLIDATED);
            ep
        };
        let fresh = episode(EpisodeKind::FansHostility, ActorRef::fans(7), 7, 100);

        let banked_rank = Consolidator::eviction_rank(&banked, 200, 12.0, 12.0, 10.0);
        let fresh_rank = Consolidator::eviction_rank(&fresh, 200, 12.0, 12.0, 10.0);
        assert!(
            banked_rank < fresh_rank,
            "an episode whose meaning is already banked is the better thing to lose"
        );
    }
}
