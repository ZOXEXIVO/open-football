use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// Hint passed to [`Relations::recalculate_chemistry_with_context`] so the
/// chemistry calculation can read squad-wide signals — leadership, captain,
/// turnover — that the per-relation store can't see on its own. When the
/// caller doesn't have this context (legacy paths, isolated tests) the
/// chemistry recalculation falls back to neutral defaults for those terms.
#[derive(Debug, Clone, Default)]
pub struct ChemistryContext {
    /// Top-3 leadership scores in the dressing room, raw 0..20 scale, sorted
    /// descending. Drives the leadership_quality term. Empty fallback → 50.
    pub top_leadership_scores: Vec<f32>,
    /// Top-3 influence scores (`relation.influence` summed across the squad
    /// per player), 0..100 scale, sorted descending. Empty fallback → 50.
    pub top_influence_scores: Vec<f32>,
    /// Captain id — receives a 1.5× weight in leadership_quality.
    pub captain_id: Option<u32>,
    /// Vice-captain id — receives a 1.2× weight in leadership_quality.
    pub vice_captain_id: Option<u32>,
    /// Number of new signings in the last 90 days. Drives turnover_penalty.
    pub recent_signings_90d: u8,
    /// Inner-circle cohesion (0..1) — proxy for group_cohesion. Pulled from
    /// the GroupDynamics model so callers don't have to compute it.
    pub inner_circle_cohesion: f32,
}

/// Enhanced Relations system with complex relationship dynamics
#[derive(Debug, Clone)]
pub struct Relations {
    /// Player relationships
    players: RelationStore<PlayerRelation>,

    /// Staff relationships
    staffs: RelationStore<StaffRelation>,

    /// Group dynamics and cliques
    groups: GroupDynamics,

    /// Relationship events history
    history: RelationshipHistory,

    /// Global mood and chemistry
    chemistry: TeamChemistry,
}

impl Default for Relations {
    fn default() -> Self {
        Self::new()
    }
}

impl Relations {
    pub fn new() -> Self {
        Relations {
            players: RelationStore::new(),
            staffs: RelationStore::new(),
            groups: GroupDynamics::new(),
            history: RelationshipHistory::new(),
            chemistry: TeamChemistry::new(),
        }
    }

    /// Simple update method for backward compatibility
    /// Updates a player relationship by a simple increment value
    pub fn update(&mut self, player_id: u32, increment: f32, date: NaiveDate) {
        // Create a relationship change based on the increment
        let change = if increment >= 0.0 {
            RelationshipChange::positive(
                ChangeType::NaturalProgression,
                increment.abs(),
            )
        } else {
            RelationshipChange::negative(
                ChangeType::NaturalProgression,
                increment.abs(),
            )
        };

        self.update_player_relationship(player_id, change, date);
    }

    /// Update with a specific change type and simulation date
    pub fn update_with_type(&mut self, player_id: u32, increment: f32, change_type: ChangeType, date: NaiveDate) {
        let change = if increment >= 0.0 {
            RelationshipChange::positive(change_type, increment.abs())
        } else {
            RelationshipChange::negative(change_type, increment.abs())
        };

        self.update_player_relationship(player_id, change, date);
    }

    /// Alternative: Direct level update without full change tracking
    pub fn update_simple(&mut self, player_id: u32, increment: f32) {
        let relation = self.players.get_or_create(player_id);
        relation.level = (relation.level + increment).clamp(-100.0, 100.0);

        // Update momentum based on the change
        relation.momentum = (relation.momentum + increment.signum() * 0.1).clamp(-1.0, 1.0);

        // Update interaction frequency
        relation.interaction_frequency = (relation.interaction_frequency + 0.1).min(1.0);

        // Recalculate chemistry if significant change
        if increment.abs() > 0.1 {
            self.chemistry.recalculate(&self.players, &self.staffs);
        }
    }

    // ========== Player Relations ==========

    /// Get relationship with a specific player
    pub fn get_player(&self, id: u32) -> Option<&PlayerRelation> {
        self.players.get(id)
    }

    /// Record a mentor/mentee link on this player's side of the relation.
    /// Creates the relation entry if one didn't exist yet.
    pub fn set_mentorship(&mut self, other_player_id: u32, kind: MentorshipType) {
        let relation = self.players.get_or_create(other_player_id);
        relation.mentorship = Some(kind);
        // Mentorship implies elevated professional respect on both ends —
        // we only touch this side here; the caller mirrors on the other.
        relation.professional_respect = (relation.professional_respect + 10.0).min(100.0);
        // Mentors accumulate dressing-room influence; mentees look up to them.
        let influence_bump = match kind {
            MentorshipType::Mentor => 8.0,
            MentorshipType::Mentee => 3.0,
        };
        relation.influence = (relation.influence + influence_bump).min(100.0);
    }

    /// Clear any mentor/mentee link (e.g. when the pair is disbanded).
    pub fn clear_mentorship(&mut self, other_player_id: u32) {
        if let Some(rel) = self.players.get_mut(other_player_id) {
            rel.mentorship = None;
        }
    }

    /// Update player relationship with detailed context
    pub fn update_player_relationship(
        &mut self,
        player_id: u32,
        change: RelationshipChange,
        date: NaiveDate,
    ) {
        // Store values we need before taking mutable borrows
        let is_rivalry = matches!(change.change_type, ChangeType::CompetitionRivalry);
        let (old_level, new_level, should_recalculate) = {
            // Create a scope for the mutable borrow
            let relation = self.players.get_or_create(player_id);

            // Store the old level
            let old_level = relation.level;

            // Apply the change
            relation.apply_change(&change);

            // Track rivalry with the actual target player ID
            if is_rivalry {
                relation.rivalry_with.insert(player_id);
            }

            // Store the new level
            let new_level = relation.level;

            // Determine if we should recalculate chemistry
            let should_recalculate = (new_level - old_level).abs() > 0.1;

            // Return the values we need (this ends the mutable borrow)
            (old_level, new_level, should_recalculate)
        }; // Mutable borrow of self.players ends here

        // Record the event (no borrow conflicts)
        self.history.record_event(RelationshipEvent {
            date,
            subject_id: player_id,
            subject_type: SubjectType::Player,
            change_type: change.change_type.clone(),
            old_value: old_level,
            new_value: new_level,
        });

        // Update chemistry if significant change (can now borrow immutably)
        if should_recalculate {
            self.chemistry.recalculate(&self.players, &self.staffs);
        }

        // Check for group formation/dissolution (uses new_level instead of relation.level)
        self.groups.update_from_relationship(player_id, new_level);
    }

    /// Get all favorite players
    pub fn get_favorite_players(&self) -> Vec<u32> {
        self.players.get_favorites()
    }

    /// Get all disliked players
    pub fn get_disliked_players(&self) -> Vec<u32> {
        self.players.get_disliked()
    }

    /// Check if player is a favorite
    pub fn is_favorite_player(&self, player_id: u32) -> bool {
        self.players.get(player_id)
            .map(|r| r.is_favorite())
            .unwrap_or(false)
    }

    // ========== Staff Relations ==========

    /// Get relationship with a specific staff member
    pub fn get_staff(&self, id: u32) -> Option<&StaffRelation> {
        self.staffs.get(id)
    }

    /// Update staff relationship
    pub fn update_staff_relationship(
        &mut self,
        staff_id: u32,
        change: RelationshipChange,
        date: NaiveDate,
    ) {
        let relation = self.staffs.get_or_create(staff_id);

        let old_level = relation.level;
        relation.apply_change(&change);

        self.history.record_event(RelationshipEvent {
            date,
            subject_id: staff_id,
            subject_type: SubjectType::Staff,
            change_type: change.change_type.clone(),
            old_value: old_level,
            new_value: relation.level,
        });

        self.chemistry.recalculate(&self.players, &self.staffs);
    }

    /// Get coaching receptiveness (how well player responds to coaching)
    pub fn get_coaching_receptiveness(&self, coach_id: u32) -> f32 {
        self.staffs.get(coach_id)
            .map(|r| r.calculate_coaching_multiplier())
            .unwrap_or(1.0)
    }

    // ========== Group Dynamics ==========

    /// Get all cliques/groups this entity belongs to
    pub fn get_groups(&self, entity_id: u32) -> Vec<GroupId> {
        self.groups.get_entity_groups(entity_id)
    }

    /// Get influence level in the dressing room
    pub fn get_influence_level(&self, player_id: u32) -> InfluenceLevel {
        let base_influence = self.players.get(player_id)
            .map(|r| r.influence)
            .unwrap_or(0.0);

        let group_bonus = self.groups.get_leadership_bonus(player_id);

        InfluenceLevel::from_value(base_influence + group_bonus)
    }

    /// Conflicts this entity has with others — strained or openly hostile
    /// relations. Each entry is the subject's relation with `target_id`.
    /// Caller supplies `subject_id` because a `Relations` instance doesn't
    /// know who owns it.
    pub fn get_potential_conflicts(&self, subject_id: u32) -> Vec<ConflictInfo> {
        let mut conflicts = Vec::new();

        for (target_id, rel) in self.players.iter() {
            // A relation is a conflict if it's rivalry-flagged or disliked.
            let is_rivalry = !rel.rivalry_with.is_empty();
            if !is_rivalry && !rel.is_disliked() {
                continue;
            }

            let conflict_type = if is_rivalry {
                ConflictType::PersonalRivalry
            } else if rel.level <= -50.0 {
                ConflictType::PersonalRivalry
            } else {
                // Disliked-but-not-hostile — default to rivalry type.
                ConflictType::PersonalRivalry
            };

            conflicts.push(ConflictInfo {
                party_a: subject_id,
                party_b: *target_id,
                conflict_type,
                severity: ConflictSeverity::from_relationship_level(rel.level),
            });
        }

        conflicts.extend(self.groups.get_group_conflicts());
        conflicts
    }

    // ========== Chemistry & Morale ==========

    /// Get overall team chemistry
    pub fn get_team_chemistry(&self) -> f32 {
        self.chemistry.overall
    }

    /// Get chemistry breakdown
    pub fn get_chemistry_factors(&self) -> &ChemistryFactors {
        &self.chemistry.factors
    }

    /// Process weekly relationship decay and evolution
    pub fn process_weekly_update(&mut self, date: NaiveDate) {
        // Natural relationship evolution
        self.players.apply_natural_decay();
        self.staffs.apply_natural_decay();

        // Update groups
        self.groups.weekly_update();

        // Clean old history
        self.history.cleanup_old_events(date);

        // Recalculate chemistry
        self.chemistry.recalculate(&self.players, &self.staffs);
    }

    /// Recalculate chemistry with squad-wide context (captain, leadership,
    /// turnover). Use this when the caller can supply the extra signals;
    /// otherwise the inner [`Relations::process_weekly_update`] still falls
    /// back to the neutral-default version.
    pub fn recalculate_chemistry_with_context(&mut self, ctx: &ChemistryContext) {
        self.chemistry
            .recalculate_with_context(&self.players, &self.staffs, &self.groups, ctx);
    }

    /// Iterate this subject's player-side relations as `(target_id, relation)`.
    /// Used by squad-level passes (chemistry context, leadership totals) that
    /// need to roll up relations across many subjects without exposing the
    /// internal RelationStore type.
    pub fn player_relations_iter(&self) -> impl Iterator<Item = (&u32, &PlayerRelation)> {
        self.players.iter()
    }

    /// Iterate this subject's staff-side relations the same way as
    /// `player_relations_iter`.
    pub fn staff_relations_iter(&self) -> impl Iterator<Item = (&u32, &StaffRelation)> {
        self.staffs.iter()
    }

    /// Inner-circle cohesion (0..1) for the subject's perceived clique —
    /// useful when callers want to roll up many subjects' cohesion signals
    /// into a [`ChemistryContext::inner_circle_cohesion`] without exposing
    /// the GroupDynamics struct.
    pub fn inner_circle_cohesion(&self) -> f32 {
        const INNER_CIRCLE_GROUP: GroupId = 1;
        self.groups
            .groups
            .get(&INNER_CIRCLE_GROUP)
            .map(|g| g.cohesion.clamp(0.0, 1.0))
            .unwrap_or(0.0)
    }

}

/// Store for relationships of a specific type
#[derive(Debug, Clone)]
struct RelationStore<T: Relationship> {
    relations: HashMap<u32, T>,
}

impl<T: Relationship> RelationStore<T> {
    fn new() -> Self {
        RelationStore {
            relations: HashMap::new(),
        }
    }

    fn get(&self, id: u32) -> Option<&T> {
        self.relations.get(&id)
    }

    fn get_mut(&mut self, id: u32) -> Option<&mut T> {
        self.relations.get_mut(&id)
    }

    fn get_or_create(&mut self, id: u32) -> &mut T {
        self.relations.entry(id).or_insert_with(T::new_neutral)
    }

    fn get_favorites(&self) -> Vec<u32> {
        self.relations.iter()
            .filter(|(_, r)| r.is_favorite())
            .map(|(id, _)| *id)
            .collect()
    }

    fn get_disliked(&self) -> Vec<u32> {
        self.relations.iter()
            .filter(|(_, r)| r.is_disliked())
            .map(|(id, _)| *id)
            .collect()
    }

    fn apply_natural_decay(&mut self) {
        for relation in self.relations.values_mut() {
            relation.apply_decay();
        }
    }

    fn iter(&self) -> impl Iterator<Item=(&u32, &T)> {
        self.relations.iter()
    }
}

/// Trait for relationship types
trait Relationship {
    fn new_neutral() -> Self;
    fn is_favorite(&self) -> bool;
    fn is_disliked(&self) -> bool;
    fn apply_decay(&mut self);
    fn apply_change(&mut self, change: &RelationshipChange);
    /// Return a 0..100 quality summary used by chemistry's player_harmony /
    /// coach_relationship terms. Different implementors weight the inputs
    /// differently — a player relation cares about trust + respect, a staff
    /// relation cares about authority respect + trust in abilities.
    fn quality_score(&self) -> f32;
    /// Contribution to the chemistry conflict_level term. 0 for neutral or
    /// positive relations; positive for rivalries, dislikes, hostility.
    fn conflict_contribution(&self) -> f32;
}

/// Player relationship details
#[derive(Debug, Clone)]
pub struct PlayerRelation {
    /// Relationship level (-100 to 100)
    pub level: f32,

    /// Trust level (0 to 100)
    pub trust: f32,

    /// Respect level (0 to 100)
    pub respect: f32,

    /// Friendship level (0 to 100)
    pub friendship: f32,

    /// Professional respect (0 to 100)
    pub professional_respect: f32,

    /// Influence this player has
    pub influence: f32,

    /// Mentorship relationship
    pub mentorship: Option<MentorshipType>,

    /// Rivalry information
    pub rivalry_with: HashSet<u32>,

    /// Interaction frequency
    pub interaction_frequency: f32,

    /// Relationship momentum
    momentum: f32,
}

impl Relationship for PlayerRelation {
    fn new_neutral() -> Self {
        PlayerRelation {
            level: 0.0,
            trust: 50.0,
            respect: 50.0,
            friendship: 30.0,
            professional_respect: 50.0,
            influence: 0.0,
            mentorship: None,
            rivalry_with: HashSet::new(),
            interaction_frequency: 0.0,
            momentum: 0.0,
        }
    }

    fn is_favorite(&self) -> bool {
        self.level >= 70.0 && self.trust >= 70.0
    }

    fn is_disliked(&self) -> bool {
        self.level <= -50.0 || self.trust <= 20.0
    }

    fn apply_decay(&mut self) {
        // Relationships naturally decay toward neutral
        if self.interaction_frequency < 0.3 {
            self.level *= 0.98;
            self.trust *= 0.99;
            self.friendship *= 0.97;
        }

        // Reset interaction frequency
        self.interaction_frequency *= 0.9;

        // Momentum decays
        self.momentum *= 0.95;
    }

    fn quality_score(&self) -> f32 {
        // level ∈ [-100, 100] → 0..100 axis
        let level_axis = (self.level + 100.0) / 2.0;
        // trust, professional_respect already on 0..100
        // Mirror the spec weighting: relation quality = level*0.4 + (trust-50)*0.3 + (prof_respect-50)*0.3
        // Centre trust/prof around 50 to keep neutral input → 50 output.
        let combined = level_axis * 0.4
            + self.trust * 0.3
            + self.professional_respect * 0.3;
        combined.clamp(0.0, 100.0)
    }

    fn conflict_contribution(&self) -> f32 {
        let mut acc = 0.0;
        let is_rivalry = !self.rivalry_with.is_empty();
        let level = self.level;
        if level <= -75.0 {
            acc += 25.0; // critical conflict
        } else if level <= -50.0 {
            acc += 15.0; // serious conflict
        } else if self.is_disliked() {
            acc += 8.0; // mild dislike
        }
        if is_rivalry {
            acc += 6.0;
        }
        acc
    }

    fn apply_change(&mut self, change: &RelationshipChange) {
        // magnitude is always positive (abs stored in both positive() and negative()).
        // The sign of the effect is determined by the change_type branch (+= or -=)
        // and by is_positive for the catch-all branch.
        let magnitude = change.magnitude.abs() * (1.0 + self.momentum * 0.5);

        match change.change_type {
            ChangeType::MatchCooperation => {
                self.level += magnitude * 2.0;
                self.trust += magnitude * 1.5;
                self.professional_respect += magnitude * 3.0;
            }
            ChangeType::TrainingBonding => {
                self.friendship += magnitude * 2.0;
                self.level += magnitude;
            }
            ChangeType::ConflictResolution => {
                self.trust += magnitude * 3.0;
                self.respect += magnitude * 2.0;
                self.level += magnitude * 2.0;
            }
            ChangeType::PersonalSupport => {
                self.friendship += magnitude * 4.0;
                self.trust += magnitude * 3.0;
                self.level += magnitude * 2.0;
            }
            ChangeType::CompetitionRivalry => {
                self.level -= magnitude * 2.0;
                self.professional_respect -= magnitude;
            }
            ChangeType::TrainingFriction => {
                self.level -= magnitude;
                self.trust -= magnitude * 0.5;
            }
            ChangeType::PersonalConflict => {
                self.level -= magnitude * 3.0;
                self.trust -= magnitude * 2.0;
                self.friendship -= magnitude * 3.0;
            }
            ChangeType::ReputationAdmiration => {
                self.respect += magnitude * 3.0;
                self.professional_respect += magnitude * 2.0;
                self.level += magnitude * 1.5;
            }
            ChangeType::ReputationTension => {
                self.level -= magnitude * 1.5;
                self.respect -= magnitude;
                self.professional_respect -= magnitude * 0.5;
            }
            _ => {
                if change.is_positive {
                    self.level += magnitude;
                } else {
                    self.level -= magnitude;
                }
            }
        }

        // Update momentum — direction tracks whether this was a positive or negative change
        let momentum_dir = if change.is_positive { 1.0 } else { -1.0 };
        self.momentum = (self.momentum + momentum_dir * 0.1).clamp(-1.0, 1.0);

        // Update interaction frequency
        self.interaction_frequency = (self.interaction_frequency + 0.1).min(1.0);

        // Clamp values
        self.level = self.level.clamp(-100.0, 100.0);
        self.trust = self.trust.clamp(0.0, 100.0);
        self.respect = self.respect.clamp(0.0, 100.0);
        self.friendship = self.friendship.clamp(0.0, 100.0);
        self.professional_respect = self.professional_respect.clamp(0.0, 100.0);
    }
}

/// Staff relationship details
#[derive(Debug, Clone)]
pub struct StaffRelation {
    /// Relationship level (-100 to 100)
    pub level: f32,

    /// Authority respect (0 to 100)
    pub authority_respect: f32,

    /// Trust in abilities (0 to 100)
    pub trust_in_abilities: f32,

    /// Personal bond (0 to 100)
    pub personal_bond: f32,

    /// Coaching receptiveness
    pub receptiveness: f32,

    /// Loyalty to staff member
    pub loyalty: f32,
}

impl StaffRelation {
    pub fn calculate_coaching_multiplier(&self) -> f32 {
        let base = 1.0;
        let respect_bonus = (self.authority_respect / 100.0) * 0.3;
        let trust_bonus = (self.trust_in_abilities / 100.0) * 0.2;
        let receptiveness_bonus = (self.receptiveness / 100.0) * 0.3;

        base + respect_bonus + trust_bonus + receptiveness_bonus
    }
}

impl Relationship for StaffRelation {
    fn new_neutral() -> Self {
        StaffRelation {
            level: 0.0,
            authority_respect: 50.0,
            trust_in_abilities: 50.0,
            personal_bond: 30.0,
            receptiveness: 50.0,
            loyalty: 30.0,
        }
    }

    fn is_favorite(&self) -> bool {
        self.level >= 70.0 && self.loyalty >= 70.0
    }

    fn is_disliked(&self) -> bool {
        self.level <= -50.0 || self.authority_respect <= 20.0
    }

    fn apply_decay(&mut self) {
        // Authority respect decays if not reinforced
        self.authority_respect *= 0.99;

        // Personal bonds decay without interaction
        self.personal_bond *= 0.98;

        // Level trends toward neutral
        self.level *= 0.99;
    }

    fn quality_score(&self) -> f32 {
        let level_axis = (self.level + 100.0) / 2.0;
        // Mix in authority respect + trust in abilities (already 0..100).
        (level_axis * 0.4 + self.authority_respect * 0.3 + self.trust_in_abilities * 0.3)
            .clamp(0.0, 100.0)
    }

    fn conflict_contribution(&self) -> f32 {
        if self.level <= -75.0 {
            18.0
        } else if self.level <= -50.0 {
            10.0
        } else if self.is_disliked() {
            5.0
        } else {
            0.0
        }
    }

    fn apply_change(&mut self, change: &RelationshipChange) {
        let magnitude = change.magnitude.abs();

        match change.change_type {
            ChangeType::CoachingSuccess => {
                self.trust_in_abilities += magnitude * 3.0;
                self.receptiveness += magnitude * 2.0;
                self.level += magnitude * 2.0;
            }
            ChangeType::TacticalDisagreement => {
                self.authority_respect -= magnitude * 2.0;
                self.receptiveness -= magnitude * 3.0;
                self.level -= magnitude;
            }
            ChangeType::PersonalSupport => {
                self.personal_bond += magnitude * 4.0;
                self.loyalty += magnitude * 3.0;
                self.level += magnitude * 2.0;
            }
            ChangeType::DisciplinaryAction => {
                self.authority_respect += magnitude;  // Can increase if fair
                self.personal_bond -= magnitude * 2.0;
                self.level -= magnitude;
            }
            _ => {
                if change.is_positive {
                    self.level += magnitude;
                } else {
                    self.level -= magnitude;
                }
            }
        }

        // Clamp values
        self.level = self.level.clamp(-100.0, 100.0);
        self.authority_respect = self.authority_respect.clamp(0.0, 100.0);
        self.trust_in_abilities = self.trust_in_abilities.clamp(0.0, 100.0);
        self.personal_bond = self.personal_bond.clamp(0.0, 100.0);
        self.receptiveness = self.receptiveness.clamp(0.0, 100.0);
        self.loyalty = self.loyalty.clamp(0.0, 100.0);
    }
}

/// Group dynamics and cliques
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct GroupDynamics {
    groups: HashMap<GroupId, Group>,
    entity_groups: HashMap<u32, HashSet<GroupId>>,
    next_group_id: GroupId,
}

impl GroupDynamics {
    fn new() -> Self {
        GroupDynamics {
            groups: HashMap::new(),
            entity_groups: HashMap::new(),
            next_group_id: 0,
        }
    }

    /// Track the owning player's "inner circle" — members they have a
    /// strong bond with (level ≥ 60). Single Social group per subject.
    /// A hostile relationship (level ≤ -50) removes the other party
    /// from the circle. Cohesion = average bond across members / 100.
    ///
    /// Per-player scope means the Group is effectively "the subject's
    /// perceived clique": who they trust, feel close to, would back up.
    /// `get_leadership_bonus` consumes the cohesion number as a proxy
    /// for dressing-room standing.
    fn update_from_relationship(&mut self, entity_id: u32, relationship_level: f32) {
        const INNER_CIRCLE_GROUP: GroupId = 1; // well-known id for the subject's own social group
        const JOIN_THRESHOLD: f32 = 60.0;
        const LEAVE_THRESHOLD: f32 = 40.0;

        // Find or seed the inner-circle group for this subject.
        let group = self.groups.entry(INNER_CIRCLE_GROUP).or_insert_with(|| Group {
            id: INNER_CIRCLE_GROUP,
            members: HashSet::new(),
            leader_id: None,      // subject-owned: leader = the subject, but we don't know their id here
            cohesion: 0.0,
            group_type: GroupType::Social,
            rival_group: None,
        });

        let was_member = group.members.contains(&entity_id);

        if relationship_level >= JOIN_THRESHOLD {
            group.members.insert(entity_id);
        } else if relationship_level < LEAVE_THRESHOLD || relationship_level <= -50.0 {
            // Bond cooled or turned hostile — drop them.
            group.members.remove(&entity_id);
        }

        // Cohesion is the normalised mean bond level across members,
        // scaled [0..1]. Empty groups → 0 so decay eventually dissolves them.
        if group.members.is_empty() {
            group.cohesion = 0.0;
        } else {
            // We only have this one data point per call; use a cheap
            // exponential moving average so repeated updates converge on
            // the actual mean without iterating every member.
            let bond_unit = (relationship_level / 100.0).clamp(0.0, 1.0);
            if was_member {
                group.cohesion = group.cohesion * 0.9 + bond_unit * 0.1;
            } else {
                // New member — give their bond a stronger weight.
                group.cohesion = group.cohesion * 0.7 + bond_unit * 0.3;
            }
        }

        // Reverse index so get_entity_groups works.
        self.entity_groups
            .entry(entity_id)
            .or_default()
            .insert(INNER_CIRCLE_GROUP);
    }

    fn get_entity_groups(&self, entity_id: u32) -> Vec<GroupId> {
        self.entity_groups.get(&entity_id)
            .map(|groups| groups.iter().copied().collect())
            .unwrap_or_default()
    }

    fn get_leadership_bonus(&self, entity_id: u32) -> f32 {
        self.entity_groups.get(&entity_id)
            .map(|groups| {
                groups.iter()
                    .filter_map(|gid| self.groups.get(gid))
                    .filter(|g| g.leader_id == Some(entity_id))
                    .map(|g| 0.2 * g.cohesion)
                    .sum()
            })
            .unwrap_or(0.0)
    }

    fn get_group_conflicts(&self) -> Vec<ConflictInfo> {
        let mut conflicts = Vec::new();

        for group in self.groups.values() {
            if let Some(rival_group) = group.rival_group {
                if let Some(rival) = self.groups.get(&rival_group) {
                    conflicts.push(ConflictInfo {
                        party_a: group.id,
                        party_b: rival.id,
                        conflict_type: ConflictType::GroupRivalry,
                        severity: ConflictSeverity::Medium,
                    });
                }
            }
        }

        conflicts
    }

    fn weekly_update(&mut self) {
        // Update group cohesion and dynamics
        for group in self.groups.values_mut() {
            group.weekly_update();
        }

        // Remove dissolved groups
        self.groups.retain(|_, g| g.cohesion > 0.1);
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct Group {
    id: GroupId,
    members: HashSet<u32>,
    leader_id: Option<u32>,
    cohesion: f32,
    group_type: GroupType,
    rival_group: Option<GroupId>,
}

impl Group {
    fn weekly_update(&mut self) {
        // Natural cohesion decay
        self.cohesion *= 0.98;
    }
}

type GroupId = u32;

#[allow(dead_code)]
#[derive(Debug, Clone)]
enum GroupType {
    Nationality,
    AgeGroup,
    PlayingPosition,
    Social,
    Professional,
}

/// Team chemistry calculator. Six-factor model (player_harmony, leadership,
/// coach_relationship, group_cohesion, conflict_level, turnover_penalty)
/// blended into a single 0..100 chemistry score that downstream systems
/// (training, match rating, selection) can read.
#[derive(Debug, Clone)]
struct TeamChemistry {
    overall: f32,
    factors: ChemistryFactors,
    /// Last-known turnover penalty (raw points subtracted in the formula).
    /// Surfaced via [`ChemistryFactors`] for UI/diagnostics.
    turnover_penalty: f32,
}

impl TeamChemistry {
    fn new() -> Self {
        TeamChemistry {
            overall: 50.0,
            factors: ChemistryFactors::default(),
            turnover_penalty: 0.0,
        }
    }

    /// Legacy recalculate — used by per-relation update paths that don't
    /// have squad-wide context. Falls back to neutral defaults for the
    /// signals it can't compute (leadership, turnover, captain weighting).
    fn recalculate<T: Relationship, S: Relationship>(
        &mut self,
        players: &RelationStore<T>,
        staffs: &RelationStore<S>,
    ) {
        let ctx = ChemistryContext::default();
        let player_harmony = calculate_player_harmony(players);
        let coach_relationship = calculate_coach_relationship(staffs);
        let leadership_quality = calculate_leadership_quality_default(&ctx);
        let conflict_level = calculate_conflict_level(players);
        let group_cohesion = (ctx.inner_circle_cohesion * 100.0).clamp(0.0, 100.0).max(50.0);
        let turnover = turnover_penalty_for(ctx.recent_signings_90d);

        self.factors = ChemistryFactors {
            player_harmony,
            leadership_quality,
            coach_relationship,
            group_cohesion,
            conflict_level,
        };
        self.turnover_penalty = turnover;
        self.overall = blend_chemistry(
            player_harmony,
            leadership_quality,
            coach_relationship,
            group_cohesion,
            conflict_level,
            turnover,
        );
    }

    /// Context-aware recalculate — used by the team's weekly tick where the
    /// caller can supply captain, top influencers, recent signings.
    fn recalculate_with_context<T: Relationship, S: Relationship>(
        &mut self,
        players: &RelationStore<T>,
        staffs: &RelationStore<S>,
        groups: &GroupDynamics,
        ctx: &ChemistryContext,
    ) {
        let player_harmony = calculate_player_harmony(players);
        let coach_relationship = calculate_coach_relationship(staffs);
        let leadership_quality = calculate_leadership_quality_default(ctx);
        let conflict_level = calculate_conflict_level(players);

        // Group cohesion: prefer caller-supplied inner_circle_cohesion (the
        // squad-wide aggregate); fall back to the per-store group's average.
        let circle = if ctx.inner_circle_cohesion > 0.0 {
            ctx.inner_circle_cohesion
        } else {
            groups
                .groups
                .values()
                .map(|g| g.cohesion)
                .sum::<f32>()
                / groups.groups.len().max(1) as f32
        };
        let group_cohesion = (circle * 100.0).clamp(0.0, 100.0).max(35.0);

        let turnover = turnover_penalty_for(ctx.recent_signings_90d);

        self.factors = ChemistryFactors {
            player_harmony,
            leadership_quality,
            coach_relationship,
            group_cohesion,
            conflict_level,
        };
        self.turnover_penalty = turnover;
        self.overall = blend_chemistry(
            player_harmony,
            leadership_quality,
            coach_relationship,
            group_cohesion,
            conflict_level,
            turnover,
        );
    }
}

/// Player harmony — weighted average across all stored player relations of
/// (level, trust, professional_respect). Maps the natural -100..100 / 0..100
/// inputs onto a single 0..100 axis. Empty store → 50 (neutral).
fn calculate_player_harmony<T: Relationship>(players: &RelationStore<T>) -> f32 {
    if players.relations.is_empty() {
        return 50.0;
    }
    let mut acc = 0.0f32;
    let mut count = 0.0f32;
    for rel in players.relations.values() {
        // Ask the relation for its quality summary (0..100) — defined on
        // the Relationship trait so PlayerRelation and StaffRelation can
        // share the harmony pipeline.
        acc += rel.quality_score();
        count += 1.0;
    }
    (acc / count).clamp(0.0, 100.0)
}

/// Leadership quality — average of the top 3 influence/leadership scores
/// (with captain ×1.5, vice ×1.2 weighting). Empty list → 50, *not* 60.
fn calculate_leadership_quality_default(ctx: &ChemistryContext) -> f32 {
    if ctx.top_leadership_scores.is_empty() && ctx.top_influence_scores.is_empty() {
        return 50.0;
    }

    // Combine raw 0..20 leadership and 0..100 influence into a single 0..100
    // axis. Leadership is our primary signal; influence pulls the score
    // toward the dressing-room reality (a quiet pro can still command
    // respect).
    let leadership_norm: Vec<f32> = ctx
        .top_leadership_scores
        .iter()
        .take(3)
        .map(|v| (v / 20.0 * 100.0).clamp(0.0, 100.0))
        .collect();
    let influence_norm: Vec<f32> = ctx
        .top_influence_scores
        .iter()
        .take(3)
        .map(|v| v.clamp(0.0, 100.0))
        .collect();

    let mut score = 0.0f32;
    let mut weight = 0.0f32;
    for (i, v) in leadership_norm.iter().enumerate() {
        let w = if i == 0 { 1.5 } else if i == 1 { 1.2 } else { 1.0 };
        score += v * w;
        weight += w;
    }
    for (i, v) in influence_norm.iter().enumerate() {
        let w = if i == 0 { 0.6 } else if i == 1 { 0.5 } else { 0.4 };
        score += v * w;
        weight += w;
    }
    if weight <= 0.0 {
        return 50.0;
    }
    (score / weight).clamp(0.0, 100.0)
}

/// Coach relationship — weighted average across all staff relations of
/// (level, authority_respect, trust_in_abilities). Empty store → 50.
fn calculate_coach_relationship<S: Relationship>(staffs: &RelationStore<S>) -> f32 {
    if staffs.relations.is_empty() {
        return 50.0;
    }
    let mut acc = 0.0f32;
    let mut count = 0.0f32;
    for rel in staffs.relations.values() {
        acc += rel.quality_score();
        count += 1.0;
    }
    (acc / count).clamp(0.0, 100.0)
}

/// Conflict level — sum of explicit conflict signals capped at 100. Mild
/// dislikes contribute small amounts; rivalries and outright hostility
/// contribute more. Caller-provided rivalry/conflict severity flags would
/// further tune this; the relation store itself only sees per-pair levels.
fn calculate_conflict_level<T: Relationship>(players: &RelationStore<T>) -> f32 {
    let mut total = 0.0f32;
    for rel in players.relations.values() {
        total += rel.conflict_contribution();
    }
    total.min(100.0)
}

/// Turnover penalty — recent signings unsettle dressing rooms. Step
/// function: 1 signing = mild, 7+ = severe.
fn turnover_penalty_for(recent_signings: u8) -> f32 {
    match recent_signings {
        0 => 0.0,
        1 => 2.0,
        2..=3 => 5.0,
        4..=6 => 10.0,
        _ => 18.0,
    }
}

/// Final blend — clamped to 0..100. Centred at 50 with linear contributions
/// from each factor's deviation from neutral.
fn blend_chemistry(
    player_harmony: f32,
    leadership_quality: f32,
    coach_relationship: f32,
    group_cohesion: f32,
    conflict_level: f32,
    turnover_penalty: f32,
) -> f32 {
    let raw = 50.0
        + (player_harmony - 50.0) * 0.35
        + (leadership_quality - 50.0) * 0.20
        + (coach_relationship - 50.0) * 0.20
        + (group_cohesion - 50.0) * 0.15
        - conflict_level * 0.35
        - turnover_penalty;
    raw.clamp(0.0, 100.0)
}

#[derive(Debug, Clone, Default)]
pub struct ChemistryFactors {
    pub player_harmony: f32,
    pub leadership_quality: f32,
    pub coach_relationship: f32,
    pub group_cohesion: f32,
    pub conflict_level: f32,
}

/// Relationship history tracking
#[derive(Debug, Clone)]
struct RelationshipHistory {
    events: VecDeque<RelationshipEvent>,
    max_events: usize,
}

impl RelationshipHistory {
    fn new() -> Self {
        RelationshipHistory {
            events: VecDeque::with_capacity(100),
            max_events: 100,
        }
    }

    fn record_event(&mut self, event: RelationshipEvent) {
        self.events.push_back(event);
        if self.events.len() > self.max_events {
            self.events.pop_front();
        }
    }

    fn cleanup_old_events(&mut self, current_date: NaiveDate) {
        let cutoff = current_date - chrono::Duration::days(365);
        self.events.retain(|e| e.date > cutoff);
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct RelationshipEvent {
    date: NaiveDate,
    subject_id: u32,
    subject_type: SubjectType,
    change_type: ChangeType,
    old_value: f32,
    new_value: f32,
}

#[derive(Debug, Clone)]
enum SubjectType {
    Player,
    Staff,
}

/// Types of relationship changes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeType {
    // Positive
    MatchCooperation,
    TrainingBonding,
    ConflictResolution,
    PersonalSupport,
    CoachingSuccess,
    TeamSuccess,
    MentorshipBond,

    // Negative
    CompetitionRivalry,
    TrainingFriction,
    PersonalConflict,
    TacticalDisagreement,
    DisciplinaryAction,
    TeamFailure,

    // Reputation-based
    ReputationAdmiration,
    ReputationTension,

    // Neutral
    NaturalProgression,
}

/// Relationship change event
#[derive(Debug, Clone)]
pub struct RelationshipChange {
    pub change_type: ChangeType,
    pub magnitude: f32,
    pub is_positive: bool,
}

impl RelationshipChange {
    pub fn positive(change_type: ChangeType, magnitude: f32) -> Self {
        RelationshipChange {
            change_type,
            magnitude: magnitude.abs(),
            is_positive: true,
        }
    }

    pub fn negative(change_type: ChangeType, magnitude: f32) -> Self {
        RelationshipChange {
            change_type,
            magnitude: magnitude.abs(),
            is_positive: false,
        }
    }
}

/// Mentorship types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MentorshipType {
    Mentor,
    Mentee,
}

/// Influence levels in the dressing room
#[derive(Debug, Clone, PartialEq)]
pub enum InfluenceLevel {
    KeyPlayer,
    Influential,
    Regular,
    Peripheral,
}

impl InfluenceLevel {
    fn from_value(value: f32) -> Self {
        match value {
            v if v >= 80.0 => InfluenceLevel::KeyPlayer,
            v if v >= 60.0 => InfluenceLevel::Influential,
            v if v >= 30.0 => InfluenceLevel::Regular,
            _ => InfluenceLevel::Peripheral,
        }
    }
}

/// Conflict information
#[derive(Debug, Clone)]
pub struct ConflictInfo {
    pub party_a: u32,
    pub party_b: u32,
    pub conflict_type: ConflictType,
    pub severity: ConflictSeverity,
}

#[derive(Debug, Clone)]
pub enum ConflictType {
    PersonalRivalry,
    GroupRivalry,
    AuthorityChallenge,
    PlayingTimeDispute,
}

#[derive(Debug, Clone)]
pub enum ConflictSeverity {
    Minor,
    Medium,
    Serious,
    Critical,
}

impl ConflictSeverity {
    pub fn from_relationship_level(level: f32) -> Self {
        match level {
            v if v <= -75.0 => ConflictSeverity::Critical,
            v if v <= -50.0 => ConflictSeverity::Serious,
            v if v <= -25.0 => ConflictSeverity::Medium,
            _ => ConflictSeverity::Minor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_player_relationship_updates() {
        let mut relations = Relations::new();

        let change = RelationshipChange::positive(
            ChangeType::TrainingBonding,
            0.5,
        );

        relations.update_player_relationship(
            1,
            change,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        );

        let rel = relations.get_player(1).unwrap();
        assert!(rel.friendship > 30.0);
    }

    #[test]
    fn test_coaching_receptiveness() {
        let mut relations = Relations::new();

        let change = RelationshipChange::positive(
            ChangeType::CoachingSuccess,
            0.8,
        );

        relations.update_staff_relationship(
            1,
            change,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        );

        let receptiveness = relations.get_coaching_receptiveness(1);
        assert!(receptiveness > 1.0);
    }

    #[test]
    fn test_team_chemistry_calculation() {
        let mut relations = Relations::new();

        // Add some positive relationships
        for i in 1..5 {
            let change = RelationshipChange::positive(
                ChangeType::TeamSuccess,
                0.5,
            );
            relations.update_player_relationship(
                i,
                change,
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            );
        }

        let chemistry = relations.get_team_chemistry();
        assert!(chemistry > 50.0);
    }
}