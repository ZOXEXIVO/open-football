//! The attribution ledger — a running account with everyone who has
//! mattered.
//!
//! Episodes fade and facts are coarse. Between them sits the thing a
//! player actually carries around: a standing balance with each person
//! and club, on four axes, updated by everything that happens and
//! outliving the events that moved it.
//!
//! The design point is that an account **persists after its episodes are
//! gone**. A manager who broke a promise in 2028 still has a damaged
//! account in 2035 even though every episode behind it has long since
//! been evicted. Accounts drift slowly back toward neutral with contact
//! silence — but only down to a floor set by any [`SemanticFact`] that
//! supports them, so a grudge with a reason behind it never fully fades
//! while a passing irritation does.
//!
//! [`SemanticFact`]: super::semantic::SemanticFact

use super::actor::ActorRef;
use super::epoch::{EpochDay, MindClock};
use super::semantic::{Semantic, SemanticStore};
use super::store::FixedStore;

/// A standing balance with one actor. 16 bytes.
///
/// Four axes, because collapsing them loses real behaviour: a player can
/// respect a manager he dislikes, be fond of a club that treated him
/// badly, and owe a debt to someone he does not trust.
#[derive(Debug, Clone, Copy, Default)]
pub struct ActorAccount {
    pub actor: ActorRef,
    /// Will he keep his word? -100..=100. Moved almost exclusively by
    /// promises kept and broken.
    trust_pct: i8,
    /// Do I like him? -100..=100.
    warmth_pct: i8,
    /// Did he do something for me — or to me? -100..=100. Positive means
    /// the player feels he owes them (they backed him, they gave him his
    /// chance); negative means they owe him.
    debt_pct: i8,
    /// Do I rate him professionally? -100..=100. Independent of liking.
    respect_pct: i8,
    /// Last time anything touched this account.
    pub last_contact: EpochDay,
}

impl ActorAccount {
    /// Days for one step of drift toward neutral.
    pub const DRIFT_PERIOD_DAYS: f32 = 90.0;
    /// Fraction of the remaining balance shed per drift period. A
    /// continuous exponential relaxation rather than a fixed decrement,
    /// so a strong account fades slowly at first and a weak one is gone
    /// within a couple of years.
    pub const DRIFT_PER_PERIOD: f32 = 0.12;

    pub fn new(actor: ActorRef, today: EpochDay) -> Self {
        ActorAccount {
            actor,
            trust_pct: 0,
            warmth_pct: 0,
            debt_pct: 0,
            respect_pct: 0,
            last_contact: today,
        }
    }

    #[inline]
    pub fn trust(&self) -> f32 {
        self.trust_pct as f32 / 100.0
    }
    #[inline]
    pub fn warmth(&self) -> f32 {
        self.warmth_pct as f32 / 100.0
    }
    #[inline]
    pub fn debt(&self) -> f32 {
        self.debt_pct as f32 / 100.0
    }
    #[inline]
    pub fn respect(&self) -> f32 {
        self.respect_pct as f32 / 100.0
    }

    /// Overall standing, -1..+1. Warmth and trust dominate; respect is a
    /// smaller term (you can rate someone you would never play for
    /// again) and debt is deliberately excluded — owing someone is not
    /// the same as feeling well toward them.
    pub fn standing(&self) -> f32 {
        (self.warmth() * 0.42 + self.trust() * 0.42 + self.respect() * 0.16).clamp(-1.0, 1.0)
    }

    fn apply(field: &mut i8, delta: f32) {
        let next = (*field as f32 / 100.0 + delta).clamp(-1.0, 1.0);
        *field = (next * 100.0).round() as i8;
    }

    /// Post a change to the account and mark contact.
    pub fn post(&mut self, entry: LedgerEntry, today: EpochDay) {
        Self::apply(&mut self.trust_pct, entry.trust);
        Self::apply(&mut self.warmth_pct, entry.warmth);
        Self::apply(&mut self.debt_pct, entry.debt);
        Self::apply(&mut self.respect_pct, entry.respect);
        self.last_contact = today;
    }

    /// Relax toward neutral for time elapsed since last contact, but
    /// never past `floor` — the magnitude a supporting semantic fact
    /// holds the account at.
    ///
    /// Applied at read time rather than on a tick, so an account costs
    /// nothing while nothing is happening.
    pub fn drifted(&self, now: EpochDay, floor: f32) -> ActorAccount {
        let days = MindClock::elapsed_f32(self.last_contact, now);
        if days <= 0.0 {
            return *self;
        }
        let periods = days / Self::DRIFT_PERIOD_DAYS;
        let retained = (1.0 - Self::DRIFT_PER_PERIOD).powf(periods);

        let relax = |value: f32| -> f32 {
            let drifted = value * retained;
            // The floor pins magnitude, not sign: a fact supporting a
            // grudge stops the grudge fading past it, and never turns it
            // into fondness.
            if value.abs() <= floor {
                value
            } else if value >= 0.0 {
                drifted.max(floor)
            } else {
                drifted.min(-floor)
            }
        };

        let mut out = *self;
        out.trust_pct = (relax(self.trust()) * 100.0).round() as i8;
        out.warmth_pct = (relax(self.warmth()) * 100.0).round() as i8;
        out.debt_pct = (relax(self.debt()) * 100.0).round() as i8;
        out.respect_pct = (relax(self.respect()) * 100.0).round() as i8;
        out
    }

    /// True when every axis has relaxed into the noise — the account is
    /// carrying nothing and can be evicted.
    pub fn is_neutral(&self) -> bool {
        self.trust_pct.abs() < 3
            && self.warmth_pct.abs() < 3
            && self.debt_pct.abs() < 3
            && self.respect_pct.abs() < 3
    }

    /// How much this account is worth keeping. Eviction rank.
    pub fn weight(&self) -> f32 {
        (self.trust().abs() + self.warmth().abs() + self.debt().abs() + self.respect().abs()) / 4.0
    }
}

/// A signed change to post against an account. All fields -1..+1; a zero
/// field leaves that axis alone.
#[derive(Debug, Clone, Copy, Default)]
pub struct LedgerEntry {
    pub trust: f32,
    pub warmth: f32,
    pub debt: f32,
    pub respect: f32,
}

impl LedgerEntry {
    pub fn trust(delta: f32) -> Self {
        LedgerEntry {
            trust: delta,
            ..Default::default()
        }
    }

    pub fn warmth(delta: f32) -> Self {
        LedgerEntry {
            warmth: delta,
            ..Default::default()
        }
    }

    pub fn debt(delta: f32) -> Self {
        LedgerEntry {
            debt: delta,
            ..Default::default()
        }
    }

    pub fn respect(delta: f32) -> Self {
        LedgerEntry {
            respect: delta,
            ..Default::default()
        }
    }

    pub fn with_trust(mut self, delta: f32) -> Self {
        self.trust = delta;
        self
    }
    pub fn with_warmth(mut self, delta: f32) -> Self {
        self.warmth = delta;
        self
    }
    pub fn with_debt(mut self, delta: f32) -> Self {
        self.debt = delta;
        self
    }
    pub fn with_respect(mut self, delta: f32) -> Self {
        self.respect = delta;
        self
    }

    /// The entry an episode produces. Betrayals hit `trust` hard —
    /// that is what distinguishes a broken promise from mere bad news.
    /// Everything else moves `warmth` in proportion to how strongly it
    /// landed.
    pub fn from_episode(valence: f32, encoding: f32, betrayal: bool) -> Self {
        let magnitude = (valence.abs() * encoding).clamp(0.0, 1.0);
        let signed = magnitude * valence.signum();

        if betrayal {
            LedgerEntry {
                trust: signed * 0.85,
                warmth: signed * 0.35,
                debt: 0.0,
                respect: signed * 0.20,
            }
        } else {
            LedgerEntry {
                trust: signed * 0.12,
                warmth: signed * 0.55,
                debt: 0.0,
                respect: 0.0,
            }
        }
    }
}

/// One account per actor who has mattered. 32 slots — sixteen people and
/// sixteen institutions is more than any career sustains meaningfully.
pub type AttributionLedger = FixedStore<ActorAccount, 32>;

/// Operations over the ledger.
pub struct Ledger;

impl Ledger {
    /// Post `entry` against `actor`, opening the account if new. A full
    /// ledger displaces its lightest account.
    pub fn post(
        ledger: &mut AttributionLedger,
        actor: ActorRef,
        entry: LedgerEntry,
        today: EpochDay,
    ) {
        if actor.is_none() {
            return;
        }

        if let Some(account) = ledger.find_mut(|a| a.actor == actor) {
            account.post(entry, today);
            return;
        }

        let mut account = ActorAccount::new(actor, today);
        account.post(entry, today);
        ledger.push_evicting(account, |a| a.weight(), |_| true);
    }

    /// The account with `actor` as it stands today, with drift applied
    /// and pinned by `fact_floor` (the magnitude any supporting semantic
    /// fact holds it at — see [`Ledger::floor_from_sentiment`]).
    /// `None` when there is no account.
    pub fn account(
        ledger: &AttributionLedger,
        actor: ActorRef,
        now: EpochDay,
        fact_floor: f32,
    ) -> Option<ActorAccount> {
        ledger
            .find(|a| a.actor == actor)
            .map(|a| a.drifted(now, fact_floor))
    }

    /// Standing with `actor`, -1..+1, or 0.0 with no account.
    pub fn standing(
        ledger: &AttributionLedger,
        actor: ActorRef,
        now: EpochDay,
        fact_floor: f32,
    ) -> f32 {
        Self::account(ledger, actor, now, fact_floor)
            .map(|a| a.standing())
            .unwrap_or(0.0)
    }

    /// Convert a semantic sentiment (-1..+1) into the drift floor it
    /// implies. A firmly-held conviction about someone stops the ledger
    /// forgetting them; a weak one barely slows it.
    #[inline]
    pub fn floor_from_sentiment(sentiment: f32) -> f32 {
        (sentiment.abs() * 0.75).clamp(0.0, 1.0)
    }

    /// Drop accounts that have relaxed into the noise. Called from
    /// consolidation, not per tick.
    ///
    /// Takes the semantic store because pruning must respect the same
    /// floor that reads do: an account held up by a conviction is not
    /// noise, however long ago it was last touched. Without this, a
    /// grudge with a firm reason behind it would be silently deleted by
    /// a housekeeping pass while [`Self::standing`] still promised to
    /// return it — the account would survive every read and then vanish
    /// on the next month boundary.
    pub fn prune(ledger: &mut AttributionLedger, semantic: &SemanticStore, now: EpochDay) {
        ledger.retain(|account| {
            let floor = Self::floor_from_sentiment(Semantic::sentiment(semantic, account.actor));
            !account.drifted(now, floor).is_neutral()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::semantic::FactClaim;
    use super::*;

    const YEAR: EpochDay = 365;

    fn coach() -> ActorRef {
        ActorRef::staff(412)
    }

    #[test]
    fn a_broken_promise_hits_trust_not_just_warmth() {
        let betrayal = LedgerEntry::from_episode(-0.85, 0.8, true);
        let ordinary = LedgerEntry::from_episode(-0.85, 0.8, false);
        assert!(
            betrayal.trust.abs() > ordinary.trust.abs() * 3.0,
            "a betrayal must be a trust event, not a mood event"
        );
        assert!(
            ordinary.warmth.abs() > ordinary.trust.abs(),
            "ordinary bad news is mostly warmth"
        );
    }

    #[test]
    fn posting_opens_the_account() {
        let mut ledger = AttributionLedger::new();
        Ledger::post(&mut ledger, coach(), LedgerEntry::trust(-0.7), 100);
        let account = Ledger::account(&ledger, coach(), 100, 0.0).unwrap();
        assert!((account.trust() + 0.7).abs() < 0.02);
        assert_eq!(ledger.len(), 1);
    }

    #[test]
    fn a_none_actor_is_never_recorded() {
        let mut ledger = AttributionLedger::new();
        Ledger::post(&mut ledger, ActorRef::NONE, LedgerEntry::warmth(0.9), 0);
        assert!(
            ledger.is_empty(),
            "situational memories have no counterparty"
        );
    }

    #[test]
    fn an_unsupported_grudge_fades_over_years() {
        let mut ledger = AttributionLedger::new();
        Ledger::post(&mut ledger, coach(), LedgerEntry::trust(-0.7), 0);

        let after_1y = Ledger::account(&ledger, coach(), YEAR, 0.0).unwrap();
        let after_7y = Ledger::account(&ledger, coach(), YEAR * 7, 0.0).unwrap();

        assert!(after_1y.trust() > -0.7, "it starts to fade within a year");
        assert!(
            after_7y.trust() > after_1y.trust(),
            "and keeps fading: {} → {}",
            after_1y.trust(),
            after_7y.trust()
        );
        assert!(
            after_7y.trust().abs() < 0.2,
            "a grudge with nothing behind it is largely gone after seven years"
        );
    }

    #[test]
    fn a_supported_grudge_survives_a_decade() {
        // This is the ten-year requirement at the ledger level.
        let mut ledger = AttributionLedger::new();
        Ledger::post(&mut ledger, coach(), LedgerEntry::trust(-0.7), 0);

        // A firmly-held "his word is worthless" pins the account.
        let floor = Ledger::floor_from_sentiment(-0.85);
        let after_10y = Ledger::account(&ledger, coach(), YEAR * 10, floor).unwrap();

        assert!(
            after_10y.trust() <= -0.6,
            "a conviction holds the account against drift, got {}",
            after_10y.trust()
        );
    }

    #[test]
    fn the_floor_pins_magnitude_and_never_flips_sign() {
        let mut ledger = AttributionLedger::new();
        Ledger::post(&mut ledger, coach(), LedgerEntry::warmth(0.8), 0);
        let after = Ledger::account(&ledger, coach(), YEAR * 20, 0.6).unwrap();
        assert!(
            after.warmth() >= 0.6,
            "positive stays positive at the floor"
        );

        let mut bitter = AttributionLedger::new();
        Ledger::post(&mut bitter, coach(), LedgerEntry::warmth(-0.8), 0);
        let after = Ledger::account(&bitter, coach(), YEAR * 20, 0.6).unwrap();
        assert!(
            after.warmth() <= -0.6,
            "negative stays negative at the floor"
        );
    }

    #[test]
    fn drift_never_pushes_a_small_balance_past_the_floor() {
        let mut ledger = AttributionLedger::new();
        Ledger::post(&mut ledger, coach(), LedgerEntry::warmth(0.2), 0);
        let after = Ledger::account(&ledger, coach(), YEAR * 5, 0.6).unwrap();
        assert!(
            (after.warmth() - 0.2).abs() < 0.02,
            "a balance already inside the floor is left alone, got {}",
            after.warmth()
        );
    }

    #[test]
    fn standing_ignores_debt() {
        let mut a = ActorAccount::new(coach(), 0);
        a.post(LedgerEntry::debt(1.0), 0);
        assert_eq!(a.standing(), 0.0, "owing someone is not liking them");
    }

    #[test]
    fn respect_and_warmth_are_independent() {
        let mut a = ActorAccount::new(coach(), 0);
        a.post(
            LedgerEntry::default().with_warmth(-0.8).with_respect(0.9),
            0,
        );
        assert!(a.warmth() < 0.0 && a.respect() > 0.0);
    }

    #[test]
    fn a_full_ledger_displaces_its_lightest_account() {
        let mut ledger = AttributionLedger::new();
        for id in 0..ledger.capacity() as u32 {
            Ledger::post(
                &mut ledger,
                ActorRef::player(id),
                LedgerEntry::warmth(0.05),
                0,
            );
        }
        assert!(ledger.is_full());

        Ledger::post(&mut ledger, coach(), LedgerEntry::trust(-0.9), 0);
        assert!(
            Ledger::account(&ledger, coach(), 0, 0.0).is_some(),
            "a heavy new account gets in"
        );
        assert_eq!(ledger.len(), ledger.capacity());
    }

    #[test]
    fn pruning_clears_only_the_spent_accounts() {
        let mut ledger = AttributionLedger::new();
        Ledger::post(
            &mut ledger,
            ActorRef::player(1),
            LedgerEntry::warmth(0.04),
            0,
        );
        Ledger::post(&mut ledger, coach(), LedgerEntry::trust(-0.9), 0);

        let semantic = SemanticStore::new();
        Ledger::prune(&mut ledger, &semantic, YEAR * 3);
        assert!(Ledger::account(&ledger, coach(), 0, 0.0).is_some());
        assert!(Ledger::account(&ledger, ActorRef::player(1), 0, 0.0).is_none());
    }

    #[test]
    fn pruning_never_deletes_an_account_a_conviction_is_holding_up() {
        // Regression guard. Reads apply the conviction floor, so a
        // supported grudge survives every `standing` call — but pruning
        // used to run with no floor at all, quietly deleting on a month
        // boundary the very account the reads had promised to keep.
        let mut ledger = AttributionLedger::new();
        Ledger::post(&mut ledger, coach(), LedgerEntry::trust(-0.9), 0);

        let mut semantic = SemanticStore::new();
        Semantic::assert(
            &mut semantic,
            FactClaim::HisWordIsWorthless,
            coach(),
            0,
            0.85,
        );

        Ledger::prune(&mut ledger, &semantic, YEAR * 12);

        let floor = Ledger::floor_from_sentiment(Semantic::sentiment(&semantic, coach()));
        let account = Ledger::account(&ledger, coach(), YEAR * 12, floor)
            .expect("a supported grudge must survive the prune");
        assert!(
            account.trust() <= -0.5,
            "twelve years on, the distrust is still there: {}",
            account.trust()
        );
        assert!(Ledger::standing(&ledger, coach(), YEAR * 12, floor) < 0.0);
    }

    #[test]
    fn pruning_still_clears_a_grudge_with_nothing_behind_it() {
        let mut ledger = AttributionLedger::new();
        Ledger::post(&mut ledger, coach(), LedgerEntry::trust(-0.9), 0);

        let semantic = SemanticStore::new();
        Ledger::prune(&mut ledger, &semantic, YEAR * 12);

        assert!(
            Ledger::account(&ledger, coach(), YEAR * 12, 0.0).is_none(),
            "an unexplained irritation does not survive twelve years"
        );
    }

    #[test]
    fn an_account_is_sixteen_bytes() {
        assert!(
            size_of::<ActorAccount>() <= 16,
            "ActorAccount grew to {}",
            size_of::<ActorAccount>()
        );
    }
}
