//! The goalkeeping department's standing plan.
//!
//! Held on a member of staff rather than on the club, for the same reason
//! the head coach's squad plan is: it is somebody's opinion, and when that
//! somebody leaves it goes with him. A new goalkeeping coach re-ranks the
//! room; he does not inherit his predecessor's pecking order.
//!
//! Like [`crate::club::staff::CoachSquadPlan`] this is a PLAN, not a
//! mechanism. Nothing here selects, promotes, loans or sells. Selection
//! reads the declared order and the live nomination, the squad plan reads
//! the tiers, the loan desk reads the recommendations — and with an empty
//! plan every one of them behaves exactly as it did before the department
//! existed.

use std::collections::HashMap;

use chrono::NaiveDate;

use super::advice::{KeeperAdvice, KeeperRecommendation, KeeperSuccession, KeeperTier};

/// One keeper's standing in the department's plan.
#[derive(Debug, Clone, Copy)]
pub struct KeeperAssignment {
    pub tier: KeeperTier,
    /// When the department last committed to this standing — so a consumer
    /// can tell a fresh decision from a settled one.
    pub set_on: NaiveDate,
    /// For a pathway keeper being groomed, the incumbent he is behind.
    pub understudy_to: Option<u32>,
    /// Share of the season's competitive matches the plan intends for him.
    pub planned_share: f32,
}

/// The keeper the department has asked the manager to start, and until when.
///
/// One at a time, deliberately. A goalkeeping coach who nominates three
/// boys at once is nominating nobody, and a selection layer that honours
/// three nominations at once has stopped picking a team.
#[derive(Debug, Clone, Copy)]
pub struct KeeperNomination {
    pub player_id: u32,
    pub opened_on: NaiveDate,
    /// The nomination stands until here; the next review renews it if the
    /// minutes still have not come.
    pub stands_until: NaiveDate,
}

impl KeeperNomination {
    /// How long a nomination stands before it has to be made again.
    pub const WINDOW_DAYS: i64 = 45;

    pub fn is_live(&self, today: NaiveDate) -> bool {
        today <= self.stands_until
    }
}

/// The department's standing view of the club's goalkeepers.
#[derive(Debug, Clone, Default)]
pub struct KeeperRoomPlan {
    assignments: HashMap<u32, KeeperAssignment>,
    /// Declared order. Kept alongside the assignments so the pecking order
    /// survives a review that could not see a squad (an injured keeper is
    /// still the number one).
    number_one: Option<u32>,
    deputy: Option<u32>,
    third: Option<u32>,
    /// The young keeper the department is building toward the shirt.
    heir: Option<u32>,
    succession: KeeperSuccession,
    nomination: Option<KeeperNomination>,
    recommendations: Vec<KeeperRecommendation>,
    /// How far the manager acts on the department's word (0..1). Written by
    /// the review so selection reads one number rather than re-deriving a
    /// staff comparison on every matchday.
    authority: f32,
    /// How the department's past nominations have turned out. Starts
    /// neutral; a nomination that produced minutes and decent ratings
    /// raises it, one the manager ignored or that went badly lowers it.
    credibility: f32,
    last_reviewed: Option<NaiveDate>,
}

impl KeeperRoomPlan {
    /// How long the plan stands before the department revisits it.
    pub const REVIEW_DAYS: i64 = 30;
    /// A department nobody has any reason to trust or distrust yet.
    pub const NEUTRAL_CREDIBILITY: f32 = 0.5;

    pub fn new() -> Self {
        KeeperRoomPlan {
            credibility: Self::NEUTRAL_CREDIBILITY,
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.assignments.is_empty()
    }

    pub fn due_for_review(&self, today: NaiveDate) -> bool {
        match self.last_reviewed {
            None => true,
            Some(last) => (today - last).num_days() >= Self::REVIEW_DAYS,
        }
    }

    pub fn tier_of(&self, player_id: u32) -> Option<KeeperTier> {
        self.assignments.get(&player_id).map(|a| a.tier)
    }

    pub fn assignment(&self, player_id: u32) -> Option<&KeeperAssignment> {
        self.assignments.get(&player_id)
    }

    pub fn assignments(&self) -> impl Iterator<Item = (u32, &KeeperAssignment)> {
        self.assignments.iter().map(|(id, a)| (*id, a))
    }

    pub fn number_one(&self) -> Option<u32> {
        self.number_one
    }

    pub fn deputy(&self) -> Option<u32> {
        self.deputy
    }

    pub fn third(&self) -> Option<u32> {
        self.third
    }

    pub fn heir(&self) -> Option<u32> {
        self.heir
    }

    pub fn succession(&self) -> KeeperSuccession {
        self.succession
    }

    pub fn authority(&self) -> f32 {
        self.authority
    }

    pub fn credibility(&self) -> f32 {
        self.credibility
    }

    pub fn last_reviewed(&self) -> Option<NaiveDate> {
        self.last_reviewed
    }

    pub fn recommendations(&self) -> &[KeeperRecommendation] {
        &self.recommendations
    }

    /// Recommendations about one keeper.
    pub fn advice_for(&self, player_id: u32) -> impl Iterator<Item = &KeeperRecommendation> {
        self.recommendations
            .iter()
            .filter(move |r| r.player_id == Some(player_id))
    }

    /// Whether the department is asking the club to do something about the
    /// shape of the room as a whole.
    pub fn wants(&self, advice: KeeperAdvice) -> bool {
        self.recommendations.iter().any(|r| r.advice == advice)
    }

    /// The keeper the department is currently asking the manager to start,
    /// if that request still stands today.
    pub fn nominated(&self, today: NaiveDate) -> Option<u32> {
        self.nomination
            .filter(|n| n.is_live(today))
            .map(|n| n.player_id)
    }

    pub fn nomination(&self) -> Option<&KeeperNomination> {
        self.nomination.as_ref()
    }

    /// A new goalkeeping coach inherits a group, not a pecking order.
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// Write the outcome of a review. Returns the keepers whose standing
    /// changed, so the caller can tell them.
    pub fn commit(
        &mut self,
        outcome: KeeperReviewOutcome,
        today: NaiveDate,
    ) -> Vec<(u32, KeeperTier)> {
        let mut changes = Vec::new();
        let mut next: HashMap<u32, KeeperAssignment> = HashMap::with_capacity(outcome.tiers.len());

        for (player_id, tier, understudy_to) in outcome.tiers {
            let previous = self.assignments.get(&player_id);
            let changed = previous.map(|a| a.tier) != Some(tier);
            if changed {
                changes.push((player_id, tier));
            }
            next.insert(
                player_id,
                KeeperAssignment {
                    tier,
                    set_on: if changed {
                        today
                    } else {
                        previous.map(|a| a.set_on).unwrap_or(today)
                    },
                    understudy_to,
                    planned_share: tier.planned_share(),
                },
            );
        }

        self.assignments = next;
        self.number_one = outcome.number_one;
        self.deputy = outcome.deputy;
        self.third = outcome.third;
        self.heir = outcome.heir;
        self.succession = outcome.succession;
        self.recommendations = outcome.recommendations;
        self.authority = outcome.authority;

        // A nomination is renewed rather than restarted while the same
        // keeper is still waiting for his minutes — the coach keeps asking.
        self.nomination = match (outcome.nominated, self.nomination) {
            (Some(id), Some(existing)) if existing.player_id == id => Some(KeeperNomination {
                player_id: id,
                opened_on: existing.opened_on,
                stands_until: today + chrono::Duration::days(KeeperNomination::WINDOW_DAYS),
            }),
            (Some(id), _) => Some(KeeperNomination {
                player_id: id,
                opened_on: today,
                stands_until: today + chrono::Duration::days(KeeperNomination::WINDOW_DAYS),
            }),
            (None, _) => None,
        };

        self.last_reviewed = Some(today);
        changes
    }

    /// Move the department's standing with the manager. A nomination that
    /// produced minutes and held up raises it; one that was ignored, or
    /// that went badly, lowers it. Bounded so a department never becomes
    /// either infallible or mute.
    pub fn credit(&mut self, delta: f32) {
        self.credibility = (self.credibility + delta).clamp(0.15, 0.95);
    }
}

/// What one review produced, before it is written into the plan.
///
/// Only [`super::GoalkeepingDepartment::review`] builds one; it is public
/// so the club-level pass can carry it from the read phase to the write.
pub struct KeeperReviewOutcome {
    pub tiers: Vec<(u32, KeeperTier, Option<u32>)>,
    pub number_one: Option<u32>,
    pub deputy: Option<u32>,
    pub third: Option<u32>,
    pub heir: Option<u32>,
    pub succession: KeeperSuccession,
    pub nominated: Option<u32>,
    pub recommendations: Vec<KeeperRecommendation>,
    pub authority: f32,
}
