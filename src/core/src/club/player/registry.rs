//! Data-driven registries for skills and traits.
//!
//! The original audit flagged two rigidities:
//!
//! 1. **Skills as a 50-element struct-of-structs.** Adding a new skill (or
//!    iterating "all skills" for UI / serialization / scouting) means
//!    touching every consumer because there is no canonical id. Skill
//!    accesses are scattered as `player.skills.technical.passing`,
//!    `player.skills.mental.vision`, etc.
//!
//! 2. **PlayerTrait as a 28-variant enum without metadata.** Every consumer
//!    that wants to ask "what skill drives this trait?" or "which traits
//!    conflict?" has to write its own match arm — see `skill_supports_trait`
//!    in `traits.rs` for the obvious example.
//!
//! This module introduces a registry layer **on top of** the existing types.
//! No storage is migrated — `PlayerSkills` stays as four nested structs,
//! `PlayerTrait` stays as the same enum. What's new is:
//!
//! - [`SkillId`]: a flat enum with one variant per skill (50 total).
//! - [`SkillCategory`], [`SkillMetadata`], [`SKILL_REGISTRY`]: declarative
//!   metadata accessible by `id.category()`, `id.display_name()`, etc.
//! - [`PlayerSkills::get`]: O(1) accessor that maps `SkillId` → the
//!   underlying field via a single match.
//! - [`TraitCategory`], [`TraitMetadata`], [`TRAIT_REGISTRY`]: same shape
//!   for traits, plus the primary-skill-driver and conflict graph that
//!   used to be implicit in scattered match arms.
//!
//! Existing direct-field access (`player.skills.technical.passing`) keeps
//! working — this is purely additive. New code should prefer the registry
//! API; old code can migrate incrementally.

use crate::club::player::skills::PlayerSkills;
use crate::club::player::traits::PlayerTrait;

// ============================================================
// Skill registry
// ============================================================

/// A canonical id for every individual skill in the simulator. Variants
/// prefixed with `Gk` denote the goalkeeping-specific skill of the same
/// short name (e.g. `Gk*FirstTouch*` is distinct from outfield `FirstTouch`)
/// — the underlying data has separate fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkillId {
    // Technical
    Corners,
    Crossing,
    Dribbling,
    Finishing,
    FirstTouch,
    FreeKicks,
    Heading,
    LongShots,
    LongThrows,
    Marking,
    Passing,
    PenaltyTaking,
    Tackling,
    Technique,
    // Mental
    Aggression,
    Anticipation,
    Bravery,
    Composure,
    Concentration,
    Decisions,
    Determination,
    Flair,
    Leadership,
    OffTheBall,
    Positioning,
    Teamwork,
    Vision,
    WorkRate,
    // Physical
    Acceleration,
    Agility,
    Balance,
    Jumping,
    NaturalFitness,
    Pace,
    Stamina,
    Strength,
    MatchReadiness,
    // Goalkeeping
    GkAerialReach,
    GkCommandOfArea,
    GkCommunication,
    GkEccentricity,
    GkFirstTouch,
    GkHandling,
    GkKicking,
    GkOneOnOnes,
    GkPassing,
    GkPunching,
    GkReflexes,
    GkRushingOut,
    GkThrowing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkillCategory {
    Technical,
    Mental,
    Physical,
    Goalkeeping,
}

#[derive(Debug, Clone, Copy)]
pub struct SkillMetadata {
    pub id: SkillId,
    pub category: SkillCategory,
    /// Human-readable display name (UI / scout reports).
    pub display_name: &'static str,
    /// Stable field name on the underlying struct — usable as a JSON key
    /// / serialization handle. Disambiguated for goalkeeping where a name
    /// collides with an outfield skill: `gk_first_touch`, not `first_touch`.
    pub field_name: &'static str,
}

impl SkillId {
    /// All 50 skill ids in canonical (registry) order. Useful for iterating
    /// "every skill" without depending on the nested struct layout.
    pub const ALL: &'static [SkillId] = &[
        SkillId::Corners,
        SkillId::Crossing,
        SkillId::Dribbling,
        SkillId::Finishing,
        SkillId::FirstTouch,
        SkillId::FreeKicks,
        SkillId::Heading,
        SkillId::LongShots,
        SkillId::LongThrows,
        SkillId::Marking,
        SkillId::Passing,
        SkillId::PenaltyTaking,
        SkillId::Tackling,
        SkillId::Technique,
        SkillId::Aggression,
        SkillId::Anticipation,
        SkillId::Bravery,
        SkillId::Composure,
        SkillId::Concentration,
        SkillId::Decisions,
        SkillId::Determination,
        SkillId::Flair,
        SkillId::Leadership,
        SkillId::OffTheBall,
        SkillId::Positioning,
        SkillId::Teamwork,
        SkillId::Vision,
        SkillId::WorkRate,
        SkillId::Acceleration,
        SkillId::Agility,
        SkillId::Balance,
        SkillId::Jumping,
        SkillId::NaturalFitness,
        SkillId::Pace,
        SkillId::Stamina,
        SkillId::Strength,
        SkillId::MatchReadiness,
        SkillId::GkAerialReach,
        SkillId::GkCommandOfArea,
        SkillId::GkCommunication,
        SkillId::GkEccentricity,
        SkillId::GkFirstTouch,
        SkillId::GkHandling,
        SkillId::GkKicking,
        SkillId::GkOneOnOnes,
        SkillId::GkPassing,
        SkillId::GkPunching,
        SkillId::GkReflexes,
        SkillId::GkRushingOut,
        SkillId::GkThrowing,
    ];

    /// Category this skill belongs to. Cheaper to call than going through
    /// the full metadata lookup when only the category is needed.
    pub fn category(self) -> SkillCategory {
        self.metadata().category
    }

    /// Full metadata record — name, category, field handle.
    pub fn metadata(self) -> &'static SkillMetadata {
        // Index lookup via the position-in-ALL contract: ALL is in the
        // same order as the metadata table, and SkillId is repr-Rust so
        // we can't just use `self as usize`. Linear scan with a match
        // is what the compiler generates anyway for an exhaustive enum
        // dispatch, so this stays O(1) per call after monomorphisation.
        match self {
            SkillId::Corners => &SKILL_REGISTRY[0],
            SkillId::Crossing => &SKILL_REGISTRY[1],
            SkillId::Dribbling => &SKILL_REGISTRY[2],
            SkillId::Finishing => &SKILL_REGISTRY[3],
            SkillId::FirstTouch => &SKILL_REGISTRY[4],
            SkillId::FreeKicks => &SKILL_REGISTRY[5],
            SkillId::Heading => &SKILL_REGISTRY[6],
            SkillId::LongShots => &SKILL_REGISTRY[7],
            SkillId::LongThrows => &SKILL_REGISTRY[8],
            SkillId::Marking => &SKILL_REGISTRY[9],
            SkillId::Passing => &SKILL_REGISTRY[10],
            SkillId::PenaltyTaking => &SKILL_REGISTRY[11],
            SkillId::Tackling => &SKILL_REGISTRY[12],
            SkillId::Technique => &SKILL_REGISTRY[13],
            SkillId::Aggression => &SKILL_REGISTRY[14],
            SkillId::Anticipation => &SKILL_REGISTRY[15],
            SkillId::Bravery => &SKILL_REGISTRY[16],
            SkillId::Composure => &SKILL_REGISTRY[17],
            SkillId::Concentration => &SKILL_REGISTRY[18],
            SkillId::Decisions => &SKILL_REGISTRY[19],
            SkillId::Determination => &SKILL_REGISTRY[20],
            SkillId::Flair => &SKILL_REGISTRY[21],
            SkillId::Leadership => &SKILL_REGISTRY[22],
            SkillId::OffTheBall => &SKILL_REGISTRY[23],
            SkillId::Positioning => &SKILL_REGISTRY[24],
            SkillId::Teamwork => &SKILL_REGISTRY[25],
            SkillId::Vision => &SKILL_REGISTRY[26],
            SkillId::WorkRate => &SKILL_REGISTRY[27],
            SkillId::Acceleration => &SKILL_REGISTRY[28],
            SkillId::Agility => &SKILL_REGISTRY[29],
            SkillId::Balance => &SKILL_REGISTRY[30],
            SkillId::Jumping => &SKILL_REGISTRY[31],
            SkillId::NaturalFitness => &SKILL_REGISTRY[32],
            SkillId::Pace => &SKILL_REGISTRY[33],
            SkillId::Stamina => &SKILL_REGISTRY[34],
            SkillId::Strength => &SKILL_REGISTRY[35],
            SkillId::MatchReadiness => &SKILL_REGISTRY[36],
            SkillId::GkAerialReach => &SKILL_REGISTRY[37],
            SkillId::GkCommandOfArea => &SKILL_REGISTRY[38],
            SkillId::GkCommunication => &SKILL_REGISTRY[39],
            SkillId::GkEccentricity => &SKILL_REGISTRY[40],
            SkillId::GkFirstTouch => &SKILL_REGISTRY[41],
            SkillId::GkHandling => &SKILL_REGISTRY[42],
            SkillId::GkKicking => &SKILL_REGISTRY[43],
            SkillId::GkOneOnOnes => &SKILL_REGISTRY[44],
            SkillId::GkPassing => &SKILL_REGISTRY[45],
            SkillId::GkPunching => &SKILL_REGISTRY[46],
            SkillId::GkReflexes => &SKILL_REGISTRY[47],
            SkillId::GkRushingOut => &SKILL_REGISTRY[48],
            SkillId::GkThrowing => &SKILL_REGISTRY[49],
        }
    }

    /// Convenience: human-readable name.
    pub fn display_name(self) -> &'static str {
        self.metadata().display_name
    }
}

/// Static metadata table indexed in the same order as `SkillId::ALL`.
pub const SKILL_REGISTRY: [SkillMetadata; 50] = [
    // ── Technical ─────────────────────────────────────────────
    SkillMetadata { id: SkillId::Corners,       category: SkillCategory::Technical,   display_name: "Corners",        field_name: "corners" },
    SkillMetadata { id: SkillId::Crossing,      category: SkillCategory::Technical,   display_name: "Crossing",       field_name: "crossing" },
    SkillMetadata { id: SkillId::Dribbling,     category: SkillCategory::Technical,   display_name: "Dribbling",      field_name: "dribbling" },
    SkillMetadata { id: SkillId::Finishing,     category: SkillCategory::Technical,   display_name: "Finishing",      field_name: "finishing" },
    SkillMetadata { id: SkillId::FirstTouch,    category: SkillCategory::Technical,   display_name: "First Touch",    field_name: "first_touch" },
    SkillMetadata { id: SkillId::FreeKicks,     category: SkillCategory::Technical,   display_name: "Free Kicks",     field_name: "free_kicks" },
    SkillMetadata { id: SkillId::Heading,       category: SkillCategory::Technical,   display_name: "Heading",        field_name: "heading" },
    SkillMetadata { id: SkillId::LongShots,     category: SkillCategory::Technical,   display_name: "Long Shots",     field_name: "long_shots" },
    SkillMetadata { id: SkillId::LongThrows,    category: SkillCategory::Technical,   display_name: "Long Throws",    field_name: "long_throws" },
    SkillMetadata { id: SkillId::Marking,       category: SkillCategory::Technical,   display_name: "Marking",        field_name: "marking" },
    SkillMetadata { id: SkillId::Passing,       category: SkillCategory::Technical,   display_name: "Passing",        field_name: "passing" },
    SkillMetadata { id: SkillId::PenaltyTaking, category: SkillCategory::Technical,   display_name: "Penalty Taking", field_name: "penalty_taking" },
    SkillMetadata { id: SkillId::Tackling,      category: SkillCategory::Technical,   display_name: "Tackling",       field_name: "tackling" },
    SkillMetadata { id: SkillId::Technique,     category: SkillCategory::Technical,   display_name: "Technique",      field_name: "technique" },
    // ── Mental ────────────────────────────────────────────────
    SkillMetadata { id: SkillId::Aggression,    category: SkillCategory::Mental,      display_name: "Aggression",     field_name: "aggression" },
    SkillMetadata { id: SkillId::Anticipation,  category: SkillCategory::Mental,      display_name: "Anticipation",   field_name: "anticipation" },
    SkillMetadata { id: SkillId::Bravery,       category: SkillCategory::Mental,      display_name: "Bravery",        field_name: "bravery" },
    SkillMetadata { id: SkillId::Composure,     category: SkillCategory::Mental,      display_name: "Composure",      field_name: "composure" },
    SkillMetadata { id: SkillId::Concentration, category: SkillCategory::Mental,      display_name: "Concentration",  field_name: "concentration" },
    SkillMetadata { id: SkillId::Decisions,     category: SkillCategory::Mental,      display_name: "Decisions",      field_name: "decisions" },
    SkillMetadata { id: SkillId::Determination, category: SkillCategory::Mental,      display_name: "Determination",  field_name: "determination" },
    SkillMetadata { id: SkillId::Flair,         category: SkillCategory::Mental,      display_name: "Flair",          field_name: "flair" },
    SkillMetadata { id: SkillId::Leadership,    category: SkillCategory::Mental,      display_name: "Leadership",     field_name: "leadership" },
    SkillMetadata { id: SkillId::OffTheBall,    category: SkillCategory::Mental,      display_name: "Off the Ball",   field_name: "off_the_ball" },
    SkillMetadata { id: SkillId::Positioning,   category: SkillCategory::Mental,      display_name: "Positioning",    field_name: "positioning" },
    SkillMetadata { id: SkillId::Teamwork,      category: SkillCategory::Mental,      display_name: "Teamwork",       field_name: "teamwork" },
    SkillMetadata { id: SkillId::Vision,        category: SkillCategory::Mental,      display_name: "Vision",         field_name: "vision" },
    SkillMetadata { id: SkillId::WorkRate,      category: SkillCategory::Mental,      display_name: "Work Rate",      field_name: "work_rate" },
    // ── Physical ──────────────────────────────────────────────
    SkillMetadata { id: SkillId::Acceleration,    category: SkillCategory::Physical, display_name: "Acceleration",     field_name: "acceleration" },
    SkillMetadata { id: SkillId::Agility,         category: SkillCategory::Physical, display_name: "Agility",          field_name: "agility" },
    SkillMetadata { id: SkillId::Balance,         category: SkillCategory::Physical, display_name: "Balance",          field_name: "balance" },
    SkillMetadata { id: SkillId::Jumping,         category: SkillCategory::Physical, display_name: "Jumping",          field_name: "jumping" },
    SkillMetadata { id: SkillId::NaturalFitness,  category: SkillCategory::Physical, display_name: "Natural Fitness",  field_name: "natural_fitness" },
    SkillMetadata { id: SkillId::Pace,            category: SkillCategory::Physical, display_name: "Pace",             field_name: "pace" },
    SkillMetadata { id: SkillId::Stamina,         category: SkillCategory::Physical, display_name: "Stamina",          field_name: "stamina" },
    SkillMetadata { id: SkillId::Strength,        category: SkillCategory::Physical, display_name: "Strength",         field_name: "strength" },
    SkillMetadata { id: SkillId::MatchReadiness,  category: SkillCategory::Physical, display_name: "Match Readiness",  field_name: "match_readiness" },
    // ── Goalkeeping ───────────────────────────────────────────
    SkillMetadata { id: SkillId::GkAerialReach,    category: SkillCategory::Goalkeeping, display_name: "Aerial Reach",    field_name: "gk_aerial_reach" },
    SkillMetadata { id: SkillId::GkCommandOfArea,  category: SkillCategory::Goalkeeping, display_name: "Command of Area", field_name: "gk_command_of_area" },
    SkillMetadata { id: SkillId::GkCommunication,  category: SkillCategory::Goalkeeping, display_name: "Communication",   field_name: "gk_communication" },
    SkillMetadata { id: SkillId::GkEccentricity,   category: SkillCategory::Goalkeeping, display_name: "Eccentricity",    field_name: "gk_eccentricity" },
    SkillMetadata { id: SkillId::GkFirstTouch,     category: SkillCategory::Goalkeeping, display_name: "First Touch (GK)", field_name: "gk_first_touch" },
    SkillMetadata { id: SkillId::GkHandling,       category: SkillCategory::Goalkeeping, display_name: "Handling",        field_name: "gk_handling" },
    SkillMetadata { id: SkillId::GkKicking,        category: SkillCategory::Goalkeeping, display_name: "Kicking",         field_name: "gk_kicking" },
    SkillMetadata { id: SkillId::GkOneOnOnes,      category: SkillCategory::Goalkeeping, display_name: "One on Ones",     field_name: "gk_one_on_ones" },
    SkillMetadata { id: SkillId::GkPassing,        category: SkillCategory::Goalkeeping, display_name: "Passing (GK)",    field_name: "gk_passing" },
    SkillMetadata { id: SkillId::GkPunching,       category: SkillCategory::Goalkeeping, display_name: "Punching",        field_name: "gk_punching" },
    SkillMetadata { id: SkillId::GkReflexes,       category: SkillCategory::Goalkeeping, display_name: "Reflexes",        field_name: "gk_reflexes" },
    SkillMetadata { id: SkillId::GkRushingOut,     category: SkillCategory::Goalkeeping, display_name: "Rushing Out",     field_name: "gk_rushing_out" },
    SkillMetadata { id: SkillId::GkThrowing,       category: SkillCategory::Goalkeeping, display_name: "Throwing",        field_name: "gk_throwing" },
];

impl PlayerSkills {
    /// Read a skill value by id. The map is a single match arm so the
    /// compiler optimises it to a constant offset; behaviour is identical
    /// to direct field access.
    pub fn get(&self, id: SkillId) -> f32 {
        match id {
            SkillId::Corners => self.technical.corners,
            SkillId::Crossing => self.technical.crossing,
            SkillId::Dribbling => self.technical.dribbling,
            SkillId::Finishing => self.technical.finishing,
            SkillId::FirstTouch => self.technical.first_touch,
            SkillId::FreeKicks => self.technical.free_kicks,
            SkillId::Heading => self.technical.heading,
            SkillId::LongShots => self.technical.long_shots,
            SkillId::LongThrows => self.technical.long_throws,
            SkillId::Marking => self.technical.marking,
            SkillId::Passing => self.technical.passing,
            SkillId::PenaltyTaking => self.technical.penalty_taking,
            SkillId::Tackling => self.technical.tackling,
            SkillId::Technique => self.technical.technique,
            SkillId::Aggression => self.mental.aggression,
            SkillId::Anticipation => self.mental.anticipation,
            SkillId::Bravery => self.mental.bravery,
            SkillId::Composure => self.mental.composure,
            SkillId::Concentration => self.mental.concentration,
            SkillId::Decisions => self.mental.decisions,
            SkillId::Determination => self.mental.determination,
            SkillId::Flair => self.mental.flair,
            SkillId::Leadership => self.mental.leadership,
            SkillId::OffTheBall => self.mental.off_the_ball,
            SkillId::Positioning => self.mental.positioning,
            SkillId::Teamwork => self.mental.teamwork,
            SkillId::Vision => self.mental.vision,
            SkillId::WorkRate => self.mental.work_rate,
            SkillId::Acceleration => self.physical.acceleration,
            SkillId::Agility => self.physical.agility,
            SkillId::Balance => self.physical.balance,
            SkillId::Jumping => self.physical.jumping,
            SkillId::NaturalFitness => self.physical.natural_fitness,
            SkillId::Pace => self.physical.pace,
            SkillId::Stamina => self.physical.stamina,
            SkillId::Strength => self.physical.strength,
            SkillId::MatchReadiness => self.physical.match_readiness,
            SkillId::GkAerialReach => self.goalkeeping.aerial_reach,
            SkillId::GkCommandOfArea => self.goalkeeping.command_of_area,
            SkillId::GkCommunication => self.goalkeeping.communication,
            SkillId::GkEccentricity => self.goalkeeping.eccentricity,
            SkillId::GkFirstTouch => self.goalkeeping.first_touch,
            SkillId::GkHandling => self.goalkeeping.handling,
            SkillId::GkKicking => self.goalkeeping.kicking,
            SkillId::GkOneOnOnes => self.goalkeeping.one_on_ones,
            SkillId::GkPassing => self.goalkeeping.passing,
            SkillId::GkPunching => self.goalkeeping.punching,
            SkillId::GkReflexes => self.goalkeeping.reflexes,
            SkillId::GkRushingOut => self.goalkeeping.rushing_out,
            SkillId::GkThrowing => self.goalkeeping.throwing,
        }
    }

    /// Iterate every (id, value) pair. Useful for serialization, UI tables,
    /// and "scout this player's strengths" sweeps without depending on the
    /// nested struct layout.
    pub fn iter_all(&self) -> impl Iterator<Item = (SkillId, f32)> + '_ {
        SkillId::ALL.iter().map(move |id| (*id, self.get(*id)))
    }
}

// ============================================================
// Trait registry
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TraitCategory {
    AttackingMovement,
    Passing,
    Shooting,
    SetPiece,
    Defensive,
    Personality,
    TechnicalFlair,
}

/// One side of the skill-gate filter: a player must have this skill at
/// or above the minimum value for the trait to be a viable acquisition.
/// Multi-skill gates compose by AND.
#[derive(Debug, Clone, Copy)]
pub struct SkillRequirement {
    pub skill: SkillId,
    pub min: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct TraitMetadata {
    pub trait_id: PlayerTrait,
    pub category: TraitCategory,
    pub display_name: &'static str,
    /// Skill that primarily drives this trait. Used by the existing
    /// `skill_supports_trait` gate during generation, and (going forward)
    /// by any "should this player attempt this PPM right now?" check in
    /// the match engine. `None` for personality traits that don't map to
    /// a single skill (e.g. `OneClubPlayer`, `Argues`).
    pub primary_skill: Option<SkillId>,
    /// Minimum skill thresholds for this trait to be plausible at
    /// generation time. AND-composed — every requirement must hold. Empty
    /// list means the trait has no skill gate (personality traits, or
    /// traits whose viability is captured by position alone).
    pub skill_requirements: &'static [SkillRequirement],
    /// Traits that are mutually exclusive with this one. Used at trait
    /// generation time to avoid contradictions (e.g. RunsWithBallOften vs
    /// RunsWithBallRarely, StaysOnFeet vs DivesIntoTackles).
    pub conflicts_with: &'static [PlayerTrait],
    /// Behavioural flag: this trait makes the player tolerate riskier
    /// passes (Playmaker, KillerBallOften, TriesThroughBalls — all
    /// chance-creation-oriented). Read by the pass evaluator's
    /// recommendation gate. Adding a new trait that should bias toward
    /// risky passing is now just `risk_tolerant_passer: true` — no edit
    /// to the evaluator.
    pub risk_tolerant_passer: bool,
}

impl PlayerTrait {
    /// All trait ids in canonical (registry) order.
    pub const ALL: &'static [PlayerTrait] = &[
        PlayerTrait::CutsInsideFromBothWings,
        PlayerTrait::HugsLine,
        PlayerTrait::RunsWithBallOften,
        PlayerTrait::RunsWithBallRarely,
        PlayerTrait::GetsIntoOppositionArea,
        PlayerTrait::ArrivesLateInOppositionArea,
        PlayerTrait::StaysBack,
        PlayerTrait::TriesThroughBalls,
        PlayerTrait::LikesToSwitchPlay,
        PlayerTrait::LooksForPassRatherThanAttemptShot,
        PlayerTrait::PlaysShortPasses,
        PlayerTrait::PlaysLongPasses,
        PlayerTrait::ShootsFromDistance,
        PlayerTrait::PlacesShots,
        PlayerTrait::PowersShots,
        PlayerTrait::TriesLobs,
        PlayerTrait::CurlsBall,
        PlayerTrait::KnocksBallPast,
        PlayerTrait::KillerBallOften,
        PlayerTrait::DivesIntoTackles,
        PlayerTrait::StaysOnFeet,
        PlayerTrait::MarkTightly,
        PlayerTrait::Playmaker,
        PlayerTrait::Argues,
        PlayerTrait::WindsUpOpponents,
        PlayerTrait::TriesTricks,
        PlayerTrait::BackheelsRegularly,
        PlayerTrait::OneClubPlayer,
    ];

    pub fn metadata(self) -> &'static TraitMetadata {
        TRAIT_REGISTRY
            .iter()
            .find(|m| m.trait_id == self)
            .expect("every PlayerTrait variant must have a TRAIT_REGISTRY entry")
    }

    pub fn category(self) -> TraitCategory {
        self.metadata().category
    }

    pub fn primary_skill(self) -> Option<SkillId> {
        self.metadata().primary_skill
    }

    pub fn conflicts_with(self, other: PlayerTrait) -> bool {
        self.metadata().conflicts_with.contains(&other)
    }

    /// Whether the player's skills meet every requirement attached to
    /// this trait. Replaces the per-variant match arm in
    /// `traits::skill_supports_trait` — the thresholds now live on the
    /// registry alongside the rest of the trait metadata.
    pub fn skills_support(self, skills: &PlayerSkills) -> bool {
        for req in self.metadata().skill_requirements {
            if skills.get(req.skill) < req.min {
                return false;
            }
        }
        true
    }

    /// Whether this trait flags the player as tolerant of risky passes.
    pub fn is_risk_tolerant_passer(self) -> bool {
        self.metadata().risk_tolerant_passer
    }
}

/// Whether any trait in the supplied set flags the player as risk-tolerant
/// for passing decisions. Mirrors the inline check in
/// `PassEvaluator::evaluate_pass`, but driven by registry metadata so new
/// traits land without an evaluator edit.
pub fn has_risk_tolerant_passing_trait(traits: &[PlayerTrait]) -> bool {
    traits.iter().any(|t| t.is_risk_tolerant_passer())
}

// Reusable requirement constants — kept short so the registry below stays readable.
const REQ_LONG_SHOTS_12:    &[SkillRequirement] = &[SkillRequirement { skill: SkillId::LongShots,   min: 12.0 }];
const REQ_FINISHING_12:     &[SkillRequirement] = &[SkillRequirement { skill: SkillId::Finishing,   min: 12.0 }];
const REQ_FINISH_LONG_11:   &[SkillRequirement] = &[
    SkillRequirement { skill: SkillId::Finishing, min: 11.0 },
    SkillRequirement { skill: SkillId::LongShots, min: 11.0 },
];
const REQ_TECHNIQUE_12:     &[SkillRequirement] = &[SkillRequirement { skill: SkillId::Technique,   min: 12.0 }];
const REQ_TECH_13_CROSS_11: &[SkillRequirement] = &[
    SkillRequirement { skill: SkillId::Technique, min: 13.0 },
    SkillRequirement { skill: SkillId::Crossing,  min: 11.0 },
];
const REQ_PASS_VIS_13:      &[SkillRequirement] = &[
    SkillRequirement { skill: SkillId::Passing, min: 13.0 },
    SkillRequirement { skill: SkillId::Vision,  min: 13.0 },
];
const REQ_PASS_VIS_14:      &[SkillRequirement] = &[
    SkillRequirement { skill: SkillId::Passing, min: 14.0 },
    SkillRequirement { skill: SkillId::Vision,  min: 14.0 },
];
const REQ_PASSING_12:       &[SkillRequirement] = &[SkillRequirement { skill: SkillId::Passing,    min: 12.0 }];
const REQ_DRIB_12_TECH_11:  &[SkillRequirement] = &[
    SkillRequirement { skill: SkillId::Dribbling, min: 12.0 },
    SkillRequirement { skill: SkillId::Technique, min: 11.0 },
];
const REQ_TECH_14_DRIB_13:  &[SkillRequirement] = &[
    SkillRequirement { skill: SkillId::Technique, min: 14.0 },
    SkillRequirement { skill: SkillId::Dribbling, min: 13.0 },
];
const REQ_TACKLING_11:      &[SkillRequirement] = &[SkillRequirement { skill: SkillId::Tackling, min: 11.0 }];
const REQ_POS_12_TACK_11:   &[SkillRequirement] = &[
    SkillRequirement { skill: SkillId::Positioning, min: 12.0 },
    SkillRequirement { skill: SkillId::Tackling,    min: 11.0 },
];
const REQ_POS_CONC_12:      &[SkillRequirement] = &[
    SkillRequirement { skill: SkillId::Positioning,   min: 12.0 },
    SkillRequirement { skill: SkillId::Concentration, min: 12.0 },
];

/// Static trait metadata table. One entry per `PlayerTrait` variant —
/// this is a compile-time guarantee enforced by the test
/// `every_trait_has_a_registry_entry`. Skill-gate thresholds (the values
/// the old `skill_supports_trait` match arm hardcoded) now live as the
/// `skill_requirements` field. Empty `&[]` means the trait has no
/// skill gate.
pub const TRAIT_REGISTRY: &[TraitMetadata] = &[
    TraitMetadata { trait_id: PlayerTrait::CutsInsideFromBothWings,         category: TraitCategory::AttackingMovement, display_name: "Cuts inside from both wings",         primary_skill: Some(SkillId::Dribbling),    skill_requirements: &[],                risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::HugsLine] },
    TraitMetadata { trait_id: PlayerTrait::HugsLine,                        category: TraitCategory::AttackingMovement, display_name: "Hugs line",                            primary_skill: Some(SkillId::Crossing),     skill_requirements: &[],                risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::CutsInsideFromBothWings] },
    TraitMetadata { trait_id: PlayerTrait::RunsWithBallOften,               category: TraitCategory::AttackingMovement, display_name: "Runs with ball often",                 primary_skill: Some(SkillId::Dribbling),    skill_requirements: REQ_DRIB_12_TECH_11, risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::RunsWithBallRarely] },
    TraitMetadata { trait_id: PlayerTrait::RunsWithBallRarely,              category: TraitCategory::AttackingMovement, display_name: "Runs with ball rarely",                primary_skill: None,                        skill_requirements: &[],                risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::RunsWithBallOften, PlayerTrait::KnocksBallPast] },
    TraitMetadata { trait_id: PlayerTrait::GetsIntoOppositionArea,          category: TraitCategory::AttackingMovement, display_name: "Gets into opposition area",            primary_skill: Some(SkillId::OffTheBall),   skill_requirements: &[],                risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::StaysBack] },
    TraitMetadata { trait_id: PlayerTrait::ArrivesLateInOppositionArea,     category: TraitCategory::AttackingMovement, display_name: "Arrives late in opposition area",      primary_skill: Some(SkillId::OffTheBall),   skill_requirements: &[],                risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::StaysBack] },
    TraitMetadata { trait_id: PlayerTrait::StaysBack,                       category: TraitCategory::AttackingMovement, display_name: "Stays back at all times",              primary_skill: None,                        skill_requirements: &[],                risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::GetsIntoOppositionArea, PlayerTrait::ArrivesLateInOppositionArea] },
    TraitMetadata { trait_id: PlayerTrait::TriesThroughBalls,               category: TraitCategory::Passing,           display_name: "Tries killer balls often",             primary_skill: Some(SkillId::Passing),      skill_requirements: REQ_PASS_VIS_13,    risk_tolerant_passer: true,  conflicts_with: &[] },
    TraitMetadata { trait_id: PlayerTrait::LikesToSwitchPlay,               category: TraitCategory::Passing,           display_name: "Likes to switch play",                 primary_skill: Some(SkillId::Vision),       skill_requirements: REQ_PASSING_12,     risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::PlaysShortPasses] },
    TraitMetadata { trait_id: PlayerTrait::LooksForPassRatherThanAttemptShot, category: TraitCategory::Passing,         display_name: "Looks for pass rather than shot",      primary_skill: Some(SkillId::Decisions),    skill_requirements: &[],                risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::ShootsFromDistance] },
    TraitMetadata { trait_id: PlayerTrait::PlaysShortPasses,                category: TraitCategory::Passing,           display_name: "Plays short passes",                   primary_skill: Some(SkillId::Passing),      skill_requirements: &[],                risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::PlaysLongPasses, PlayerTrait::LikesToSwitchPlay] },
    TraitMetadata { trait_id: PlayerTrait::PlaysLongPasses,                 category: TraitCategory::Passing,           display_name: "Plays long passes",                    primary_skill: Some(SkillId::Passing),      skill_requirements: REQ_PASSING_12,     risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::PlaysShortPasses] },
    TraitMetadata { trait_id: PlayerTrait::ShootsFromDistance,              category: TraitCategory::Shooting,          display_name: "Shoots from distance",                 primary_skill: Some(SkillId::LongShots),    skill_requirements: REQ_LONG_SHOTS_12,  risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::LooksForPassRatherThanAttemptShot] },
    TraitMetadata { trait_id: PlayerTrait::PlacesShots,                     category: TraitCategory::Shooting,          display_name: "Places shots",                         primary_skill: Some(SkillId::Finishing),    skill_requirements: REQ_FINISHING_12,   risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::PowersShots] },
    TraitMetadata { trait_id: PlayerTrait::PowersShots,                     category: TraitCategory::Shooting,          display_name: "Powers shots",                         primary_skill: Some(SkillId::Finishing),    skill_requirements: REQ_FINISH_LONG_11, risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::PlacesShots] },
    TraitMetadata { trait_id: PlayerTrait::TriesLobs,                       category: TraitCategory::Shooting,          display_name: "Tries lobs",                           primary_skill: Some(SkillId::Technique),    skill_requirements: REQ_TECHNIQUE_12,   risk_tolerant_passer: false, conflicts_with: &[] },
    TraitMetadata { trait_id: PlayerTrait::CurlsBall,                       category: TraitCategory::SetPiece,          display_name: "Curls ball",                           primary_skill: Some(SkillId::Technique),    skill_requirements: REQ_TECH_13_CROSS_11, risk_tolerant_passer: false, conflicts_with: &[] },
    TraitMetadata { trait_id: PlayerTrait::KnocksBallPast,                  category: TraitCategory::AttackingMovement, display_name: "Knocks ball past opponent",            primary_skill: Some(SkillId::Pace),         skill_requirements: REQ_DRIB_12_TECH_11, risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::RunsWithBallRarely] },
    TraitMetadata { trait_id: PlayerTrait::KillerBallOften,                 category: TraitCategory::Passing,           display_name: "Plays killer balls",                   primary_skill: Some(SkillId::Vision),       skill_requirements: REQ_PASS_VIS_13,    risk_tolerant_passer: true,  conflicts_with: &[] },
    TraitMetadata { trait_id: PlayerTrait::DivesIntoTackles,                category: TraitCategory::Defensive,         display_name: "Dives into tackles",                   primary_skill: Some(SkillId::Tackling),     skill_requirements: REQ_TACKLING_11,    risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::StaysOnFeet] },
    TraitMetadata { trait_id: PlayerTrait::StaysOnFeet,                     category: TraitCategory::Defensive,         display_name: "Stays on feet",                        primary_skill: Some(SkillId::Positioning),  skill_requirements: REQ_POS_12_TACK_11, risk_tolerant_passer: false, conflicts_with: &[PlayerTrait::DivesIntoTackles] },
    TraitMetadata { trait_id: PlayerTrait::MarkTightly,                     category: TraitCategory::Defensive,         display_name: "Marks opponent tightly",               primary_skill: Some(SkillId::Marking),      skill_requirements: REQ_POS_CONC_12,    risk_tolerant_passer: false, conflicts_with: &[] },
    TraitMetadata { trait_id: PlayerTrait::Playmaker,                       category: TraitCategory::Personality,       display_name: "Dictates tempo",                       primary_skill: Some(SkillId::Vision),       skill_requirements: REQ_PASS_VIS_14,    risk_tolerant_passer: true,  conflicts_with: &[] },
    TraitMetadata { trait_id: PlayerTrait::Argues,                          category: TraitCategory::Personality,       display_name: "Argues with officials",                primary_skill: None,                        skill_requirements: &[],                risk_tolerant_passer: false, conflicts_with: &[] },
    TraitMetadata { trait_id: PlayerTrait::WindsUpOpponents,                category: TraitCategory::Personality,       display_name: "Winds up opponents",                   primary_skill: None,                        skill_requirements: &[],                risk_tolerant_passer: false, conflicts_with: &[] },
    TraitMetadata { trait_id: PlayerTrait::TriesTricks,                     category: TraitCategory::TechnicalFlair,    display_name: "Tries tricks",                         primary_skill: Some(SkillId::Technique),    skill_requirements: REQ_TECH_14_DRIB_13, risk_tolerant_passer: false, conflicts_with: &[] },
    TraitMetadata { trait_id: PlayerTrait::BackheelsRegularly,              category: TraitCategory::TechnicalFlair,    display_name: "Tries backheels",                      primary_skill: Some(SkillId::Flair),        skill_requirements: REQ_TECH_14_DRIB_13, risk_tolerant_passer: false, conflicts_with: &[] },
    TraitMetadata { trait_id: PlayerTrait::OneClubPlayer,                   category: TraitCategory::Personality,       display_name: "One club player",                      primary_skill: None,                        skill_requirements: &[],                risk_tolerant_passer: false, conflicts_with: &[] },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::skills::{Goalkeeping, Mental, Physical, PlayerSkills, Technical};

    fn skills_with_known_values() -> PlayerSkills {
        let mut s = PlayerSkills::default();
        s.technical = Technical {
            corners: 1.0, crossing: 2.0, dribbling: 3.0, finishing: 4.0, first_touch: 5.0,
            free_kicks: 6.0, heading: 7.0, long_shots: 8.0, long_throws: 9.0, marking: 10.0,
            passing: 11.0, penalty_taking: 12.0, tackling: 13.0, technique: 14.0,
        };
        s.mental = Mental {
            aggression: 15.0, anticipation: 16.0, bravery: 17.0, composure: 18.0, concentration: 19.0,
            decisions: 20.0, determination: 21.0, flair: 22.0, leadership: 23.0, off_the_ball: 24.0,
            positioning: 25.0, teamwork: 26.0, vision: 27.0, work_rate: 28.0,
        };
        s.physical = Physical {
            acceleration: 29.0, agility: 30.0, balance: 31.0, jumping: 32.0, natural_fitness: 33.0,
            pace: 34.0, stamina: 35.0, strength: 36.0, match_readiness: 37.0,
        };
        s.goalkeeping = Goalkeeping {
            aerial_reach: 38.0, command_of_area: 39.0, communication: 40.0, eccentricity: 41.0,
            first_touch: 42.0, handling: 43.0, kicking: 44.0, one_on_ones: 45.0, passing: 46.0,
            punching: 47.0, reflexes: 48.0, rushing_out: 49.0, throwing: 50.0,
        };
        s
    }

    #[test]
    fn skill_registry_has_50_entries() {
        assert_eq!(SKILL_REGISTRY.len(), 50);
        assert_eq!(SkillId::ALL.len(), 50);
    }

    #[test]
    fn every_skill_id_resolves_metadata() {
        for id in SkillId::ALL {
            let meta = id.metadata();
            assert_eq!(meta.id, *id, "registry table out of order at {:?}", id);
        }
    }

    #[test]
    fn registry_categories_match_struct_membership() {
        // First 14 = Technical, next 14 = Mental, next 9 = Physical, last 13 = Goalkeeping.
        for (i, m) in SKILL_REGISTRY.iter().enumerate() {
            let expected = if i < 14 {
                SkillCategory::Technical
            } else if i < 28 {
                SkillCategory::Mental
            } else if i < 37 {
                SkillCategory::Physical
            } else {
                SkillCategory::Goalkeeping
            };
            assert_eq!(m.category, expected, "wrong category at index {}", i);
        }
    }

    #[test]
    fn skills_get_round_trips_against_direct_field_access() {
        let s = skills_with_known_values();
        // Every SkillId::get must equal the corresponding nested-struct field.
        assert_eq!(s.get(SkillId::Corners), s.technical.corners);
        assert_eq!(s.get(SkillId::Passing), s.technical.passing);
        assert_eq!(s.get(SkillId::Vision), s.mental.vision);
        assert_eq!(s.get(SkillId::Pace), s.physical.pace);
        assert_eq!(s.get(SkillId::MatchReadiness), s.physical.match_readiness);
        // Goalkeeping must not collide with technical despite name clashes.
        assert_eq!(s.get(SkillId::FirstTouch), s.technical.first_touch);
        assert_eq!(s.get(SkillId::GkFirstTouch), s.goalkeeping.first_touch);
        assert_eq!(s.get(SkillId::Passing), s.technical.passing);
        assert_eq!(s.get(SkillId::GkPassing), s.goalkeeping.passing);
    }

    #[test]
    fn iter_all_yields_every_skill_with_value() {
        let s = skills_with_known_values();
        let collected: Vec<(SkillId, f32)> = s.iter_all().collect();
        assert_eq!(collected.len(), 50);
        // Sample: the first should be (Corners, 1.0).
        assert_eq!(collected[0], (SkillId::Corners, 1.0));
    }

    #[test]
    fn every_trait_has_a_registry_entry() {
        for t in PlayerTrait::ALL {
            // Will panic with a clear message if any variant is missing.
            let _ = t.metadata();
        }
        assert_eq!(TRAIT_REGISTRY.len(), PlayerTrait::ALL.len());
    }

    #[test]
    fn trait_conflicts_are_symmetric() {
        for entry in TRAIT_REGISTRY {
            for &other in entry.conflicts_with {
                let other_meta = other.metadata();
                assert!(
                    other_meta.conflicts_with.contains(&entry.trait_id),
                    "{:?} lists {:?} as a conflict, but the reverse is not declared",
                    entry.trait_id,
                    other
                );
            }
        }
    }

    #[test]
    fn passing_traits_route_back_to_passing_or_vision() {
        // Sanity: any passing trait's primary skill should be a passing
        // or vision driver. Catches typos in the registry table.
        let valid = [SkillId::Passing, SkillId::Vision, SkillId::Decisions];
        for entry in TRAIT_REGISTRY {
            if entry.category == TraitCategory::Passing {
                if let Some(p) = entry.primary_skill {
                    assert!(
                        valid.contains(&p),
                        "passing trait {:?} routes to non-passing skill {:?}",
                        entry.trait_id,
                        p
                    );
                }
            }
        }
    }

    /// Regression test pinned to the legacy `skill_supports_trait` match
    /// arm in `traits.rs`. When the function was registry-driven, every
    /// threshold moved into `TRAIT_REGISTRY::skill_requirements`. A
    /// typo there would silently change game balance — this table
    /// reproduces every original branch and asserts the new path returns
    /// the same answer.
    #[test]
    fn skills_support_matches_legacy_thresholds() {
        // Build a skills set where every individual field equals `value`.
        fn skills_at(value: f32) -> PlayerSkills {
            let mut s = skills_with_known_values();
            // Overwrite every field to the requested constant. Easier
            // than enumerating each field — `iter_all` confirms shape.
            s.technical = crate::club::player::skills::Technical {
                corners: value, crossing: value, dribbling: value, finishing: value,
                first_touch: value, free_kicks: value, heading: value, long_shots: value,
                long_throws: value, marking: value, passing: value, penalty_taking: value,
                tackling: value, technique: value,
            };
            s.mental = crate::club::player::skills::Mental {
                aggression: value, anticipation: value, bravery: value, composure: value,
                concentration: value, decisions: value, determination: value, flair: value,
                leadership: value, off_the_ball: value, positioning: value, teamwork: value,
                vision: value, work_rate: value,
            };
            s.physical = crate::club::player::skills::Physical {
                acceleration: value, agility: value, balance: value, jumping: value,
                natural_fitness: value, pace: value, stamina: value, strength: value,
                match_readiness: value,
            };
            s.goalkeeping = crate::club::player::skills::Goalkeeping {
                aerial_reach: value, command_of_area: value, communication: value,
                eccentricity: value, first_touch: value, handling: value, kicking: value,
                one_on_ones: value, passing: value, punching: value, reflexes: value,
                rushing_out: value, throwing: value,
            };
            s
        }

        // Each row: (trait, value-just-below-threshold, value-just-at-threshold).
        // For traits with no requirement, both rows must be `true`.
        let cases: &[(PlayerTrait, f32, f32)] = &[
            (PlayerTrait::ShootsFromDistance, 11.99, 12.0),
            (PlayerTrait::PlacesShots,        11.99, 12.0),
            (PlayerTrait::PowersShots,        10.99, 11.0),
            (PlayerTrait::TriesLobs,          11.99, 12.0),
            (PlayerTrait::CurlsBall,          12.99, 13.0),
            (PlayerTrait::TriesThroughBalls,  12.99, 13.0),
            (PlayerTrait::KillerBallOften,    12.99, 13.0),
            (PlayerTrait::Playmaker,          13.99, 14.0),
            (PlayerTrait::LikesToSwitchPlay,  11.99, 12.0),
            (PlayerTrait::PlaysLongPasses,    11.99, 12.0),
            (PlayerTrait::RunsWithBallOften,  11.99, 12.0),
            (PlayerTrait::KnocksBallPast,     11.99, 12.0),
            (PlayerTrait::TriesTricks,        13.99, 14.0),
            (PlayerTrait::BackheelsRegularly, 13.99, 14.0),
            (PlayerTrait::DivesIntoTackles,   10.99, 11.0),
            (PlayerTrait::StaysOnFeet,        11.99, 12.0),
            (PlayerTrait::MarkTightly,        11.99, 12.0),
        ];
        for (tr, below, at) in cases {
            assert!(
                !tr.skills_support(&skills_at(*below)),
                "{:?} should reject skills < threshold (value={})",
                tr, below
            );
            assert!(
                tr.skills_support(&skills_at(*at)),
                "{:?} should accept skills >= threshold (value={})",
                tr, at
            );
        }

        // No-requirement traits must always pass.
        let no_req = [
            PlayerTrait::CutsInsideFromBothWings,
            PlayerTrait::HugsLine,
            PlayerTrait::RunsWithBallRarely,
            PlayerTrait::GetsIntoOppositionArea,
            PlayerTrait::ArrivesLateInOppositionArea,
            PlayerTrait::StaysBack,
            PlayerTrait::LooksForPassRatherThanAttemptShot,
            PlayerTrait::PlaysShortPasses,
            PlayerTrait::Argues,
            PlayerTrait::WindsUpOpponents,
            PlayerTrait::OneClubPlayer,
        ];
        let zeros = skills_at(0.0);
        for tr in no_req {
            assert!(tr.skills_support(&zeros), "{:?} should always pass", tr);
        }
    }

    /// Regression test pinning the trio of traits that the legacy pass
    /// evaluator hardcoded as "risk-tolerant." Now driven by the registry
    /// `risk_tolerant_passer` flag — this test makes sure we didn't drop
    /// or miscategorise any of them, and that no other traits accidentally
    /// got the flag set.
    #[test]
    fn risk_tolerant_passer_flag_matches_legacy_set() {
        let expected = [
            PlayerTrait::TriesThroughBalls,
            PlayerTrait::KillerBallOften,
            PlayerTrait::Playmaker,
        ];
        for t in PlayerTrait::ALL {
            let want = expected.contains(t);
            assert_eq!(
                t.is_risk_tolerant_passer(),
                want,
                "{:?} risk_tolerant_passer flag mismatch (expected {})",
                t,
                want
            );
        }
        // Helper sanity: an empty slice is not risk-tolerant.
        assert!(!has_risk_tolerant_passing_trait(&[]));
        // Helper sanity: any single risk-tolerant trait flips the result.
        for t in expected {
            assert!(has_risk_tolerant_passing_trait(&[t]));
        }
    }
}
