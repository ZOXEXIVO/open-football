//! Recall — memory is cued, never scanned.
//!
//! Nothing outside this module iterates the episode store. Callers ask a
//! question shaped like a situation ("I am walking back into this club",
//! "I am sitting opposite this manager") and get back what that brings
//! up. Three reasons this matters:
//!
//! * **It is how memory works.** You do not review your life and filter
//!   it; a place brings things back. Cued retrieval is the difference
//!   between a database and a mind.
//! * **Rehearsal needs a hook.** Recall strengthens what it returns and
//!   resets its forgetting clock — which is exactly why the memories
//!   come flooding back when you return somewhere, and why they fade
//!   again if you never do. That only works if there is one place where
//!   remembering happens.
//! * **It is cheap.** O(32) worst case, and only when something actually
//!   prompts it.
//!
//! Two biases are applied on the way out, each worth its line:
//! **mood-congruent retrieval** (a player in a bad way genuinely does
//! reach the bad memories first) and **reconsolidation drift** (each act
//! of remembering nudges a memory toward the disposition of the man
//! doing the remembering — which is where nostalgia comes from).

use super::actor::{ActorKind, ActorRef};
use super::consolidation::EpisodeStore;
use super::episode::MindEpisode;
use super::epoch::EpochDay;
use super::forgetting::ForgettingCurve;
use super::ledger::{ActorAccount, AttributionLedger, Ledger};
use super::semantic::{Semantic, SemanticFact, SemanticStore};
use std::cmp::Ordering;

/// What prompts the remembering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallCue {
    /// Arriving at, facing, or being offered a club. Brings back the
    /// club, its board and its supporters together — this is the cue
    /// behind the ten-year return.
    Club(u32),
    /// Meeting someone again.
    Person(ActorRef),
    /// A country, for what outlives any one club in it.
    Country(u32),
    /// Everything, strongest first. For the player-profile UI and the
    /// census harness — not for decisions.
    Everything,
}

impl RecallCue {
    /// Does this cue reach `episode`?
    fn matches_episode(self, episode: &MindEpisode) -> bool {
        match self {
            RecallCue::Club(id) => episode.where_club == id || episode.who.belongs_to_club(id),
            RecallCue::Person(actor) => episode.who == actor,
            RecallCue::Country(_) => false,
            RecallCue::Everything => true,
        }
    }

    /// Does this cue reach a belief held about `subject`?
    fn matches_subject(self, subject: ActorRef) -> bool {
        match self {
            RecallCue::Club(id) => subject.belongs_to_club(id),
            RecallCue::Person(actor) => subject == actor,
            RecallCue::Country(id) => subject.kind == ActorKind::Country && subject.id == id,
            RecallCue::Everything => true,
        }
    }
}

/// One remembered episode as it comes back — the record plus how vividly.
#[derive(Debug, Clone, Copy)]
pub struct RecalledEpisode {
    pub episode: MindEpisode,
    /// Live retention at the moment of recall, 0..1. How clearly he
    /// remembers it.
    pub vividness: f32,
}

/// What a cue brought back.
///
/// The three stores answer together because that is how the question is
/// actually asked: "what do I feel about this place" is a few vivid
/// moments, the conclusions they added up to, and the standing balance.
#[derive(Debug, Clone, Default)]
pub struct RecallResult {
    /// Episodes, most vivid first.
    pub episodes: Vec<RecalledEpisode>,
    /// Convictions, most firmly held first.
    pub facts: Vec<SemanticFact>,
    /// Standing accounts with everyone the cue reached.
    pub accounts: Vec<ActorAccount>,
}

impl RecallResult {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.episodes.is_empty() && self.facts.is_empty() && self.accounts.is_empty()
    }

    /// Overall sentiment the recall carries, -1..+1.
    ///
    /// Weighted toward convictions over episodes on purpose: what a man
    /// concluded about a place outlasts, and outweighs, his recollection
    /// of any particular night there. After a decade the episodes have
    /// faded and this is almost entirely the facts and the ledger —
    /// which is the correct answer.
    pub fn sentiment(&self) -> f32 {
        let episodic: f32 = self
            .episodes
            .iter()
            .map(|r| r.episode.valence() * r.vividness)
            .sum();
        let episodic = if self.episodes.is_empty() {
            0.0
        } else {
            episodic / self.episodes.len() as f32
        };

        let semantic: f32 = self.facts.iter().map(|f| f.weight()).sum();
        let semantic = if self.facts.is_empty() {
            0.0
        } else {
            semantic / self.facts.len() as f32
        };

        let standing: f32 = self.accounts.iter().map(|a| a.standing()).sum();
        let standing = if self.accounts.is_empty() {
            0.0
        } else {
            standing / self.accounts.len() as f32
        };

        (episodic * 0.25 + semantic * 0.45 + standing * 0.30).clamp(-1.0, 1.0)
    }

    /// The single most vivid thing the cue brought back, if anything.
    pub fn strongest_episode(&self) -> Option<&RecalledEpisode> {
        self.episodes.first()
    }

    /// The most firmly held conviction the cue brought back.
    pub fn strongest_fact(&self) -> Option<&SemanticFact> {
        self.facts.first()
    }
}

/// Everything the mind needs to remember well: who he is, and how he
/// feels today.
#[derive(Debug, Clone, Copy)]
pub struct RecallContext {
    pub today: EpochDay,
    /// Personality, 0–20, from [`PersonAttributes`].
    ///
    /// [`PersonAttributes`]: crate::club::person::PersonAttributes
    pub professionalism: f32,
    pub consistency: f32,
    pub temperament: f32,
    pub loyalty: f32,
    /// Current morale on the sim's 0–100 scale. Drives mood-congruent
    /// retrieval.
    pub morale: f32,
}

impl RecallContext {
    /// Mood on a -1..+1 axis, centred on the neutral morale band.
    #[inline]
    fn mood(&self) -> f32 {
        ((self.morale - 50.0) / 50.0).clamp(-1.0, 1.0)
    }

    /// Which way remembering nudges a memory. A loyal man's recollection
    /// warms over the years; a volatile one's sours.
    #[inline]
    fn disposition(&self) -> f32 {
        (((self.loyalty - 10.0) / 10.0) * 0.6 - ((self.temperament - 10.0) / 10.0) * 0.4)
            .clamp(-1.0, 1.0)
    }
}

/// The retrieval engine.
pub struct Recall;

impl Recall {
    /// How strongly mood tilts retrieval toward matching memories. Small
    /// — a bad mood colours what comes to mind, it does not rewrite the
    /// past.
    pub const MOOD_CONGRUENCE: f32 = 0.25;

    /// How far one act of remembering drags a memory toward the
    /// rememberer's disposition. Tiny per recall; over a career of
    /// returning to the same thought it is what turns a mixed spell into
    /// "the best years of my life".
    pub const RECONSOLIDATION_DRIFT: f32 = 0.02;

    /// Maximum episodes a single recall returns. A memory that surfaces
    /// forty things is not a memory.
    pub const MAX_EPISODES: usize = 8;

    /// Remember. Mutates: what comes back is rehearsed (strengthened,
    /// clock reset) and drifts a little toward how he feels about things
    /// now.
    pub fn cue(
        episodes: &mut EpisodeStore,
        semantic: &SemanticStore,
        ledger: &AttributionLedger,
        cue: RecallCue,
        ctx: &RecallContext,
    ) -> RecallResult {
        let mood = ctx.mood();
        let disposition = ctx.disposition();

        // ── Episodes ────────────────────────────────────────────
        let mut recalled: Vec<RecalledEpisode> = Vec::new();
        for episode in episodes.iter_mut() {
            if !cue.matches_episode(episode) {
                continue;
            }

            let beta = ForgettingCurve::beta(
                ctx.professionalism,
                ctx.consistency,
                ctx.temperament,
                episode.valence(),
            );
            let retention = ForgettingCurve::retention_protected(
                episode.encoding(),
                episode.last_touched,
                ctx.today,
                beta,
                episode.is_flashbulb(),
            );
            if retention < ForgettingCurve::FAINT {
                continue;
            }

            // Mood-congruent retrieval: a memory that agrees with how he
            // feels surfaces more readily than one that does not.
            let congruence = 1.0 + mood * episode.valence() * Self::MOOD_CONGRUENCE;
            let vividness = (retention * congruence).clamp(0.0, 1.0);

            recalled.push(RecalledEpisode {
                episode: *episode,
                vividness,
            });
        }

        recalled.sort_by(|a, b| {
            b.vividness
                .partial_cmp(&a.vividness)
                .unwrap_or(Ordering::Equal)
        });
        recalled.truncate(Self::MAX_EPISODES);

        // Rehearse what actually came back. Everything else stays where
        // it was — remembering some of a spell does not refresh all of it.
        for entry in &recalled {
            let Some(stored) = episodes.find_mut(|e| {
                e.kind == entry.episode.kind
                    && e.when == entry.episode.when
                    && e.who == entry.episode.who
                    && e.where_club == entry.episode.where_club
            }) else {
                continue;
            };
            stored.set_encoding(ForgettingCurve::rehearsed(stored.encoding()));
            stored.last_touched = ctx.today;
            stored.recall_count = stored.recall_count.saturating_add(1);

            // Reconsolidation: the memory comes back slightly changed.
            let valence = stored.valence();
            stored.set_valence(valence + (disposition - valence) * Self::RECONSOLIDATION_DRIFT);
        }

        // ── Convictions ─────────────────────────────────────────
        let mut facts: Vec<SemanticFact> = semantic
            .iter()
            .filter(|f| cue.matches_subject(f.subject))
            .copied()
            .collect();
        facts.sort_by(|a, b| {
            b.strength()
                .partial_cmp(&a.strength())
                .unwrap_or(Ordering::Equal)
        });

        // ── Standing accounts ───────────────────────────────────
        let accounts: Vec<ActorAccount> = ledger
            .iter()
            .filter(|a| cue.matches_subject(a.actor))
            .filter_map(|a| {
                let floor = Ledger::floor_from_sentiment(Semantic::sentiment(semantic, a.actor));
                Ledger::account(ledger, a.actor, ctx.today, floor)
            })
            .collect();

        RecallResult {
            episodes: recalled,
            facts,
            accounts,
        }
    }

    /// What a cue would bring back, **without bringing it back**.
    ///
    /// The same selection as [`Self::cue`] with the two mutations
    /// removed: nothing is rehearsed and nothing drifts. That is the
    /// difference between a man walking back into a place and somebody
    /// else looking at what he remembers about it — a profile page, a
    /// scout report, a census — and only the first should strengthen
    /// anything. A reader that used `cue` would keep a career's worth of
    /// memories alive purely by being read.
    pub fn inspect(
        episodes: &EpisodeStore,
        semantic: &SemanticStore,
        ledger: &AttributionLedger,
        cue: RecallCue,
        ctx: &RecallContext,
    ) -> RecallResult {
        let mood = ctx.mood();

        let mut recalled: Vec<RecalledEpisode> = Vec::new();
        for episode in episodes.iter() {
            if !cue.matches_episode(episode) {
                continue;
            }
            let beta = ForgettingCurve::beta(
                ctx.professionalism,
                ctx.consistency,
                ctx.temperament,
                episode.valence(),
            );
            let retention = ForgettingCurve::retention_protected(
                episode.encoding(),
                episode.last_touched,
                ctx.today,
                beta,
                episode.is_flashbulb(),
            );
            if retention < ForgettingCurve::FAINT {
                continue;
            }
            let congruence = 1.0 + mood * episode.valence() * Self::MOOD_CONGRUENCE;
            recalled.push(RecalledEpisode {
                episode: *episode,
                vividness: (retention * congruence).clamp(0.0, 1.0),
            });
        }
        recalled.sort_by(|a, b| {
            b.vividness
                .partial_cmp(&a.vividness)
                .unwrap_or(Ordering::Equal)
        });
        recalled.truncate(Self::MAX_EPISODES);

        let mut facts: Vec<SemanticFact> = semantic
            .iter()
            .filter(|f| cue.matches_subject(f.subject))
            .copied()
            .collect();
        facts.sort_by(|a, b| {
            b.strength()
                .partial_cmp(&a.strength())
                .unwrap_or(Ordering::Equal)
        });

        let accounts: Vec<ActorAccount> = ledger
            .iter()
            .filter(|a| cue.matches_subject(a.actor))
            .filter_map(|a| {
                let floor = Ledger::floor_from_sentiment(Semantic::sentiment(semantic, a.actor));
                Ledger::account(ledger, a.actor, ctx.today, floor)
            })
            .collect();

        RecallResult {
            episodes: recalled,
            facts,
            accounts,
        }
    }

    /// How he feels about a club, without the full recall payload. The
    /// common question, and the one the transfer path asks.
    ///
    /// Read-only — this does *not* rehearse. A club being mentioned in a
    /// list of options is not the same as walking back into it; only
    /// [`Recall::cue`] counts as remembering.
    pub fn club_sentiment(
        episodes: &EpisodeStore,
        semantic: &SemanticStore,
        ledger: &AttributionLedger,
        club_id: u32,
        ctx: &RecallContext,
    ) -> f32 {
        let cue = RecallCue::Club(club_id);

        let mut episodic_sum = 0.0f32;
        let mut episodic_count = 0u32;
        for episode in episodes.iter() {
            if !cue.matches_episode(episode) {
                continue;
            }
            let beta = ForgettingCurve::beta(
                ctx.professionalism,
                ctx.consistency,
                ctx.temperament,
                episode.valence(),
            );
            let retention = ForgettingCurve::retention_protected(
                episode.encoding(),
                episode.last_touched,
                ctx.today,
                beta,
                episode.is_flashbulb(),
            );
            if retention < ForgettingCurve::FAINT {
                continue;
            }
            episodic_sum += episode.valence() * retention;
            episodic_count += 1;
        }
        let episodic = if episodic_count == 0 {
            0.0
        } else {
            episodic_sum / episodic_count as f32
        };

        let club = ActorRef::club(club_id);
        let fans = ActorRef::fans(club_id);
        let semantic_part =
            (Semantic::sentiment(semantic, club) + Semantic::sentiment(semantic, fans)) / 2.0;

        let standing = [club, fans, ActorRef::board(club_id)]
            .iter()
            .filter_map(|actor| {
                let floor = Ledger::floor_from_sentiment(Semantic::sentiment(semantic, *actor));
                Ledger::account(ledger, *actor, ctx.today, floor)
            })
            .map(|a| a.standing())
            .fold((0.0f32, 0u32), |(sum, n), s| (sum + s, n + 1));
        let standing = if standing.1 == 0 {
            0.0
        } else {
            standing.0 / standing.1 as f32
        };

        (episodic * 0.25 + semantic_part * 0.45 + standing * 0.30).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::super::consolidation::{Consolidator, MindHolder};
    use super::super::episode::{EncodingInputs, EpisodeKind};
    use super::super::ledger::LedgerEntry;
    use super::*;

    const YEAR: EpochDay = 365;

    fn ctx(today: EpochDay) -> RecallContext {
        RecallContext {
            today,
            professionalism: 12.0,
            consistency: 12.0,
            temperament: 10.0,
            loyalty: 12.0,
            morale: 50.0,
        }
    }

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

    #[test]
    fn a_club_cue_reaches_the_club_its_board_and_its_fans() {
        let mut episodes = EpisodeStore::new();
        let semantic = SemanticStore::new();
        let ledger = AttributionLedger::new();

        episodes.push(episode(EpisodeKind::SeniorDebut, ActorRef::NONE, 7, 10));
        episodes.push(episode(
            EpisodeKind::FansAdoration,
            ActorRef::fans(7),
            7,
            20,
        ));
        episodes.push(episode(EpisodeKind::DerbyWin, ActorRef::NONE, 9, 30));

        let result = Recall::cue(
            &mut episodes,
            &semantic,
            &ledger,
            RecallCue::Club(7),
            &ctx(100),
        );
        assert_eq!(
            result.episodes.len(),
            2,
            "the other club's night is not cued"
        );
    }

    #[test]
    fn recall_rehearses_what_it_returns_and_only_that() {
        let mut episodes = EpisodeStore::new();
        let semantic = SemanticStore::new();
        let ledger = AttributionLedger::new();

        episodes.push(episode(EpisodeKind::DerbyWin, ActorRef::NONE, 7, 10));
        episodes.push(episode(EpisodeKind::DerbyWin, ActorRef::NONE, 9, 10));

        Recall::cue(
            &mut episodes,
            &semantic,
            &ledger,
            RecallCue::Club(7),
            &ctx(1000),
        );

        let cued = episodes.find(|e| e.where_club == 7).unwrap();
        let uncued = episodes.find(|e| e.where_club == 9).unwrap();
        assert_eq!(cued.last_touched, 1000, "the cued memory is refreshed");
        assert_eq!(cued.recall_count, 1);
        assert_eq!(uncued.last_touched, 10, "the other one is untouched");
        assert_eq!(uncued.recall_count, 0);
    }

    #[test]
    fn returning_after_ten_years_still_brings_the_place_back() {
        // The headline requirement, end to end.
        let mut episodes = EpisodeStore::new();
        let mut semantic = SemanticStore::new();
        let mut ledger = AttributionLedger::new();

        // Three years at club 7: a debut, a title, warmth from the fans.
        episodes.push(episode(EpisodeKind::SeniorDebut, ActorRef::NONE, 7, 0));
        episodes.push(episode(EpisodeKind::WonLeagueTitle, ActorRef::NONE, 7, 400));
        for i in 0..4 {
            episodes.push(episode(
                EpisodeKind::FansAdoration,
                ActorRef::fans(7),
                7,
                500 + i * 30,
            ));
        }
        Ledger::post(
            &mut ledger,
            ActorRef::club(7),
            LedgerEntry::warmth(0.7),
            600,
        );
        Consolidator::run(
            &mut episodes,
            &mut semantic,
            &mut ledger,
            700,
            MindHolder::Player,
            12.0,
            12.0,
            10.0,
        );

        // Ten years elsewhere.
        let much_later = 700 + YEAR * 10;
        let result = Recall::cue(
            &mut episodes,
            &semantic,
            &ledger,
            RecallCue::Club(7),
            &ctx(much_later),
        );

        assert!(
            !result.is_empty(),
            "a decade later the club must still mean something"
        );
        assert!(
            result.strongest_fact().is_some(),
            "the conclusions he drew there survive"
        );
        assert!(
            result.sentiment() > 0.3,
            "and he remembers it fondly, got {}",
            result.sentiment()
        );
        assert!(
            result
                .episodes
                .iter()
                .any(|r| r.episode.kind == EpisodeKind::SeniorDebut),
            "he still remembers his debut"
        );
    }

    #[test]
    fn a_club_he_has_no_history_with_brings_back_nothing() {
        let mut episodes = EpisodeStore::new();
        let semantic = SemanticStore::new();
        let ledger = AttributionLedger::new();
        episodes.push(episode(EpisodeKind::SeniorDebut, ActorRef::NONE, 7, 10));

        let result = Recall::cue(
            &mut episodes,
            &semantic,
            &ledger,
            RecallCue::Club(99),
            &ctx(100),
        );
        assert!(result.is_empty());
        assert_eq!(result.sentiment(), 0.0);
    }

    #[test]
    fn mood_colours_what_comes_to_mind() {
        let build = || {
            let mut episodes = EpisodeStore::new();
            episodes.push(episode(EpisodeKind::DerbyWin, ActorRef::NONE, 7, 100));
            episodes.push(episode(EpisodeKind::DerbyDefeat, ActorRef::NONE, 7, 100));
            episodes
        };
        let semantic = SemanticStore::new();
        let ledger = AttributionLedger::new();

        let mut low = build();
        let mut low_ctx = ctx(200);
        low_ctx.morale = 15.0;
        let miserable = Recall::cue(&mut low, &semantic, &ledger, RecallCue::Club(7), &low_ctx);

        let mut high = build();
        let mut high_ctx = ctx(200);
        high_ctx.morale = 90.0;
        let content = Recall::cue(&mut high, &semantic, &ledger, RecallCue::Club(7), &high_ctx);

        assert!(
            miserable.sentiment() < content.sentiment(),
            "the same past reads worse to a man in a bad way: {} vs {}",
            miserable.sentiment(),
            content.sentiment()
        );
    }

    #[test]
    fn a_loyal_man_remembers_it_more_warmly_each_time() {
        let mut episodes = EpisodeStore::new();
        let semantic = SemanticStore::new();
        let ledger = AttributionLedger::new();
        episodes.push(episode(EpisodeKind::SignedForClub, ActorRef::NONE, 7, 0));

        let before = episodes.get(0).unwrap().valence();

        let mut loyal = ctx(0);
        loyal.loyalty = 19.0;
        loyal.temperament = 6.0;
        for day in 1..40 {
            loyal.today = day * 10;
            Recall::cue(
                &mut episodes,
                &semantic,
                &ledger,
                RecallCue::Club(7),
                &loyal,
            );
        }

        let after = episodes.get(0).unwrap().valence();
        assert!(
            after > before,
            "nostalgia: {before} → {after} over forty recollections"
        );
    }

    #[test]
    fn a_volatile_man_sours_on_the_same_memory() {
        let mut episodes = EpisodeStore::new();
        let semantic = SemanticStore::new();
        let ledger = AttributionLedger::new();
        episodes.push(episode(EpisodeKind::SignedForClub, ActorRef::NONE, 7, 0));
        let before = episodes.get(0).unwrap().valence();

        let mut volatile = ctx(0);
        volatile.loyalty = 4.0;
        volatile.temperament = 18.0;
        for day in 1..40 {
            volatile.today = day * 10;
            Recall::cue(
                &mut episodes,
                &semantic,
                &ledger,
                RecallCue::Club(7),
                &volatile,
            );
        }

        let after = episodes.get(0).unwrap().valence();
        assert!(after < before, "the same spell curdles: {before} → {after}");
    }

    #[test]
    fn a_person_cue_follows_the_person_not_the_badge() {
        let mut episodes = EpisodeStore::new();
        let semantic = SemanticStore::new();
        let ledger = AttributionLedger::new();
        let coach = ActorRef::staff(412);

        episodes.push(episode(EpisodeKind::ManagerPromiseBroken, coach, 7, 10));

        // Same coach, different club years later — the memory follows him.
        let result = Recall::cue(
            &mut episodes,
            &semantic,
            &ledger,
            RecallCue::Person(coach),
            &ctx(YEAR * 5),
        );
        assert_eq!(result.episodes.len(), 1);
        assert!(result.sentiment() < 0.0);
    }

    #[test]
    fn recall_returns_at_most_a_handful() {
        let mut episodes = EpisodeStore::new();
        let semantic = SemanticStore::new();
        let ledger = AttributionLedger::new();
        for i in 0..20 {
            episodes.push(episode(EpisodeKind::DerbyWin, ActorRef::NONE, 7, i));
        }
        let result = Recall::cue(
            &mut episodes,
            &semantic,
            &ledger,
            RecallCue::Club(7),
            &ctx(100),
        );
        assert!(result.episodes.len() <= Recall::MAX_EPISODES);
    }

    #[test]
    fn club_sentiment_does_not_rehearse() {
        let mut episodes = EpisodeStore::new();
        let semantic = SemanticStore::new();
        let ledger = AttributionLedger::new();
        episodes.push(episode(EpisodeKind::SeniorDebut, ActorRef::NONE, 7, 10));

        let _ = Recall::club_sentiment(&episodes, &semantic, &ledger, 7, &ctx(1000));
        assert_eq!(
            episodes.get(0).unwrap().last_touched,
            10,
            "being listed as an option is not the same as walking back in"
        );
    }

    #[test]
    fn the_grudge_and_the_fondness_can_point_opposite_ways() {
        // He loves the club and cannot stand the manager who is now
        // there. Both must survive the same cue.
        let mut episodes = EpisodeStore::new();
        let mut semantic = SemanticStore::new();
        let mut ledger = AttributionLedger::new();
        let coach = ActorRef::staff(412);

        episodes.push(episode(EpisodeKind::SeniorDebut, ActorRef::NONE, 7, 0));
        episodes.push(episode(EpisodeKind::ManagerPromiseBroken, coach, 7, 100));
        Ledger::post(
            &mut ledger,
            ActorRef::club(7),
            LedgerEntry::warmth(0.8),
            100,
        );
        Ledger::post(&mut ledger, coach, LedgerEntry::trust(-0.9), 100);
        Consolidator::run(
            &mut episodes,
            &mut semantic,
            &mut ledger,
            200,
            MindHolder::Player,
            12.0,
            12.0,
            10.0,
        );

        let club_feeling = Recall::club_sentiment(&episodes, &semantic, &ledger, 7, &ctx(YEAR * 8));
        let coach_standing = Ledger::standing(&ledger, coach, YEAR * 8, 0.0);

        assert!(club_feeling > 0.0, "the club is still home: {club_feeling}");
        assert!(
            coach_standing < 0.0,
            "the man is still the man: {coach_standing}"
        );
    }
}
