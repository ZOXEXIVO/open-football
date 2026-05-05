//! Manager ↔ Player interaction history.
//!
//! Where the old code only stored "we had a talk and morale moved by X",
//! the interaction record keeps the *why* and *what* of every coach
//! conversation: which staff member, which topic, which tone, what the
//! player's mood was going in, and what came out the other side
//! (kept/broken promise, relationship delta, morale delta, cooldown).
//!
//! Why this is "beyond FM": you can audit the dressing room — which
//! topics keep getting raised, which manager keeps overpromising, which
//! player keeps being told the same thing. The new manager-talk
//! processing reads this history to apply per-(player,topic) cooldowns,
//! detect overpromise patterns, and refuse to repeat a talk that just
//! went badly.

use chrono::NaiveDate;

/// What the talk was about. Topics are deliberately football-specific —
/// each maps to a different decision tree in `process_manager_player_talks`
/// and a different verifier path in `Player::verify_promises`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionTopic {
    /// "I want more minutes." Both player- and manager-initiated.
    PlayingTime,
    /// "Where do I fit tactically?" — focal point, mentor, rotation.
    TacticalRole,
    /// Manager addressing a slump in form.
    PoorForm,
    /// Manager praising a hot streak.
    GoodForm,
    /// "You're late again." Disciplinary action / standards talk.
    Discipline,
    /// Player asking to be loaned out for development.
    LoanRequest,
    /// Player asking to leave permanently.
    TransferRequest,
    /// Renewal terms / squad-status review.
    ContractStatus,
    /// "Are you settling in?" — typically post-transfer.
    IntegrationSupport,
    /// Captain / vice-captain / leadership-path conversations.
    Leadership,
    /// Press / fan pressure on the player.
    MediaPressure,
    /// Off-pitch / family / personal — gentle, off-the-record.
    PersonalIssue,
}

/// How the manager delivered it. Tones interact with player personality
/// the same way `team_talks::TeamTalkTone` does at half-time, but at the
/// 1:1 level. A `Demanding` talk to a low-temperament player backfires;
/// the same talk to a determined pro lands fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionTone {
    /// Even-keeled, factual. Default safe tone.
    Calm,
    /// "I expect more from you." Tough love.
    Demanding,
    /// "We've got your back." Backing the player.
    Supportive,
    /// "Here's the truth, no spin." Honest delivery — softens bad news.
    Honest,
    /// Vague answers / kicking the can. Rarely lands well.
    Evasive,
    /// "This is how it's going to be." Top-down.
    Authoritarian,
    /// Owning past mistakes / asking for trust.
    Apologetic,
}

/// What came out of the talk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionOutcome {
    /// Talk landed — player's concerns reduced or motivation up.
    Positive,
    /// Talk went badly — relationship dipped, frustration rose.
    Negative,
    /// Talk was politely had but moved nothing — neither side conceded.
    Neutral,
    /// The talk ended with a concrete promise (recorded separately on
    /// `Player::promises`). Outcome status is independent of whether
    /// the promise gets kept later.
    PromiseMade,
}

/// One row in the player's manager-interaction log. Lives on `Player`
/// behind a small ring buffer so the whole structure is bounded.
#[derive(Debug, Clone)]
pub struct ManagerInteraction {
    pub date: NaiveDate,
    pub staff_id: u32,
    pub topic: InteractionTopic,
    pub tone: InteractionTone,
    /// Player morale snapshot at the start of the talk (0..100).
    pub player_mood_before: f32,
    pub outcome: InteractionOutcome,
    /// True if this talk created a `ManagerPromise`. The matching promise
    /// kind is implied by the topic.
    pub promise_created: bool,
    /// Signed change applied to the staff relation (-100..100 axis).
    pub relationship_delta: f32,
    /// Signed change applied to morale (-50..+50ish in practice).
    pub morale_delta: f32,
    /// Earliest date this same (topic, player) pair can be raised again.
    pub cooldown_until: NaiveDate,
}

/// Bounded log of recent interactions with manager / coaching staff.
/// Drops the oldest entry past `MAX_INTERACTIONS`. Cheap O(n) scans —
/// `n` is small.
#[derive(Debug, Clone, Default)]
pub struct ManagerInteractionLog {
    pub entries: Vec<ManagerInteraction>,
}

const MAX_INTERACTIONS: usize = 32;
/// How long old entries stay before being purged. Long enough to detect
/// patterns ("manager overpromised three times in six months") without
/// growing forever.
const RETENTION_DAYS: i64 = 365;

impl ManagerInteractionLog {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Append one interaction; trims to keep the buffer bounded.
    pub fn push(&mut self, entry: ManagerInteraction) {
        self.entries.push(entry);
        if self.entries.len() > MAX_INTERACTIONS {
            self.entries.remove(0);
        }
    }

    /// True if a (topic) cooldown is still active anywhere in the log.
    /// Doesn't filter by staff — re-asking a different coach the same
    /// thing in the cooldown window is still spam from the player's
    /// point of view.
    pub fn topic_on_cooldown(&self, topic: InteractionTopic, today: NaiveDate) -> bool {
        self.entries
            .iter()
            .any(|e| e.topic == topic && e.cooldown_until > today)
    }

    /// Most recent entry for a given topic — used to avoid re-emitting
    /// the same talk the next week.
    pub fn last_for_topic(&self, topic: InteractionTopic) -> Option<&ManagerInteraction> {
        self.entries.iter().rev().find(|e| e.topic == topic)
    }

    /// Count of broken-promise outcomes within `window_days`. Drives
    /// credibility: a manager who's broken three promises in two months
    /// can't credibly make a fourth.
    pub fn broken_promise_count(&self, today: NaiveDate, window_days: i64) -> usize {
        self.entries
            .iter()
            .filter(|e| {
                e.outcome == InteractionOutcome::Negative
                    && e.promise_created
                    && (today - e.date).num_days() <= window_days
            })
            .count()
    }

    /// Drop entries past the retention window. Called weekly alongside
    /// happiness event decay.
    pub fn decay(&mut self, today: NaiveDate) {
        self.entries
            .retain(|e| (today - e.date).num_days() <= RETENTION_DAYS);
    }
}

/// Default per-topic cooldown in days. Topics that touch a player's
/// long-term anxieties (transfer, contract, loan) reset slowly; routine
/// form talks reset quickly. Used by talk processing to set
/// `cooldown_until` when an interaction is recorded.
pub fn default_cooldown_days(topic: InteractionTopic) -> i64 {
    use InteractionTopic::*;
    match topic {
        PlayingTime => 21,
        TacticalRole => 28,
        PoorForm => 14,
        GoodForm => 14,
        Discipline => 28,
        LoanRequest => 60,
        TransferRequest => 60,
        ContractStatus => 90,
        IntegrationSupport => 14,
        Leadership => 60,
        MediaPressure => 14,
        PersonalIssue => 21,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn entry(
        topic: InteractionTopic,
        date: NaiveDate,
        cooldown_until: NaiveDate,
    ) -> ManagerInteraction {
        ManagerInteraction {
            date,
            staff_id: 1,
            topic,
            tone: InteractionTone::Calm,
            player_mood_before: 60.0,
            outcome: InteractionOutcome::Neutral,
            promise_created: false,
            relationship_delta: 0.0,
            morale_delta: 0.0,
            cooldown_until,
        }
    }

    #[test]
    fn cooldown_blocks_topic_in_window() {
        let mut log = ManagerInteractionLog::new();
        log.push(entry(
            InteractionTopic::PlayingTime,
            d(2026, 4, 1),
            d(2026, 4, 22),
        ));
        assert!(log.topic_on_cooldown(InteractionTopic::PlayingTime, d(2026, 4, 10)));
        assert!(!log.topic_on_cooldown(InteractionTopic::PlayingTime, d(2026, 4, 23)));
    }

    #[test]
    fn buffer_caps_at_max_interactions() {
        let mut log = ManagerInteractionLog::new();
        for i in 0..(MAX_INTERACTIONS + 5) {
            let day = ((i % 28) + 1) as u32;
            log.push(entry(
                InteractionTopic::GoodForm,
                d(2026, 4, day),
                d(2026, 4, day),
            ));
        }
        assert_eq!(log.entries.len(), MAX_INTERACTIONS);
    }

    #[test]
    fn decay_drops_old_entries() {
        let mut log = ManagerInteractionLog::new();
        log.push(entry(
            InteractionTopic::PlayingTime,
            d(2025, 1, 1),
            d(2025, 1, 22),
        ));
        log.push(entry(
            InteractionTopic::GoodForm,
            d(2026, 4, 1),
            d(2026, 4, 15),
        ));
        log.decay(d(2026, 4, 26));
        assert_eq!(log.entries.len(), 1);
        assert_eq!(log.entries[0].topic, InteractionTopic::GoodForm);
    }
}
