//! Flat-array skill registry: `SkillKey`, indexes, category lookup, and
//! the round-trip helpers between `Player.skills` and `[f32; SKILL_COUNT]`.
//!
//! Internally the development tick operates on a flat array for speed.
//! To keep the index constants in lockstep with the actual `PlayerSkills`
//! fields, they are defined relative to a single source of truth: the
//! `SkillKey` enum. Adding or reordering a variant in `SkillKey`
//! automatically shifts the `SK_*` constants. The round-trip test in
//! `tests.rs` proves the `skills_to_array` / `write_skills_back` mapping
//! covers every variant.

use crate::club::player::player::Player;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillKey {
    // Technical 0..14
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
    // Mental 14..28
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
    // Physical 28..37 (MatchReadiness sits at the end of the band — it's
    // managed by the training/match system, not the development tick)
    Acceleration,
    Agility,
    Balance,
    Jumping,
    NaturalFitness,
    Pace,
    Stamina,
    Strength,
    MatchReadiness,
    // Goalkeeping 37..50
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

impl SkillKey {
    pub const fn idx(self) -> usize {
        self as usize
    }
}

pub(super) const SKILL_COUNT: usize = 50;

pub(super) const SK_CORNERS: usize = SkillKey::Corners.idx();
pub(super) const SK_CROSSING: usize = SkillKey::Crossing.idx();
pub(super) const SK_DRIBBLING: usize = SkillKey::Dribbling.idx();
pub(super) const SK_FINISHING: usize = SkillKey::Finishing.idx();
pub(super) const SK_FIRST_TOUCH: usize = SkillKey::FirstTouch.idx();
pub(super) const SK_FREE_KICKS: usize = SkillKey::FreeKicks.idx();
pub(super) const SK_HEADING: usize = SkillKey::Heading.idx();
pub(super) const SK_LONG_SHOTS: usize = SkillKey::LongShots.idx();
pub(super) const SK_LONG_THROWS: usize = SkillKey::LongThrows.idx();
pub(super) const SK_MARKING: usize = SkillKey::Marking.idx();
pub(super) const SK_PASSING: usize = SkillKey::Passing.idx();
pub(super) const SK_PENALTY_TAKING: usize = SkillKey::PenaltyTaking.idx();
pub(super) const SK_TACKLING: usize = SkillKey::Tackling.idx();
pub(super) const SK_TECHNIQUE: usize = SkillKey::Technique.idx();
pub(super) const SK_AGGRESSION: usize = SkillKey::Aggression.idx();
pub(super) const SK_ANTICIPATION: usize = SkillKey::Anticipation.idx();
pub(super) const SK_BRAVERY: usize = SkillKey::Bravery.idx();
pub(super) const SK_COMPOSURE: usize = SkillKey::Composure.idx();
pub(super) const SK_CONCENTRATION: usize = SkillKey::Concentration.idx();
pub(super) const SK_DECISIONS: usize = SkillKey::Decisions.idx();
pub(super) const SK_DETERMINATION: usize = SkillKey::Determination.idx();
pub(super) const SK_FLAIR: usize = SkillKey::Flair.idx();
pub(super) const SK_LEADERSHIP: usize = SkillKey::Leadership.idx();
pub(super) const SK_OFF_THE_BALL: usize = SkillKey::OffTheBall.idx();
pub(super) const SK_POSITIONING: usize = SkillKey::Positioning.idx();
pub(super) const SK_TEAMWORK: usize = SkillKey::Teamwork.idx();
pub(super) const SK_VISION: usize = SkillKey::Vision.idx();
pub(super) const SK_WORK_RATE: usize = SkillKey::WorkRate.idx();
pub(super) const SK_ACCELERATION: usize = SkillKey::Acceleration.idx();
pub(super) const SK_AGILITY: usize = SkillKey::Agility.idx();
pub(super) const SK_BALANCE: usize = SkillKey::Balance.idx();
pub(super) const SK_JUMPING: usize = SkillKey::Jumping.idx();
pub(super) const SK_NATURAL_FITNESS: usize = SkillKey::NaturalFitness.idx();
pub(super) const SK_PACE: usize = SkillKey::Pace.idx();
pub(super) const SK_STAMINA: usize = SkillKey::Stamina.idx();
pub(super) const SK_STRENGTH: usize = SkillKey::Strength.idx();
pub(super) const SK_MATCH_READINESS: usize = SkillKey::MatchReadiness.idx();
pub(super) const SK_GK_AERIAL_REACH: usize = SkillKey::GkAerialReach.idx();
pub(super) const SK_GK_COMMAND_OF_AREA: usize = SkillKey::GkCommandOfArea.idx();
pub(super) const SK_GK_COMMUNICATION: usize = SkillKey::GkCommunication.idx();
pub(super) const SK_GK_ECCENTRICITY: usize = SkillKey::GkEccentricity.idx();
pub(super) const SK_GK_FIRST_TOUCH: usize = SkillKey::GkFirstTouch.idx();
pub(super) const SK_GK_HANDLING: usize = SkillKey::GkHandling.idx();
pub(super) const SK_GK_KICKING: usize = SkillKey::GkKicking.idx();
pub(super) const SK_GK_ONE_ON_ONES: usize = SkillKey::GkOneOnOnes.idx();
pub(super) const SK_GK_PASSING: usize = SkillKey::GkPassing.idx();
pub(super) const SK_GK_PUNCHING: usize = SkillKey::GkPunching.idx();
pub(super) const SK_GK_REFLEXES: usize = SkillKey::GkReflexes.idx();
pub(super) const SK_GK_RUSHING_OUT: usize = SkillKey::GkRushingOut.idx();
pub(super) const SK_GK_THROWING: usize = SkillKey::GkThrowing.idx();

// Compile-time invariant: the enum must have exactly SKILL_COUNT variants
// and the GK band must end at SKILL_COUNT - 1.
const _: () = {
    assert!(SK_GK_THROWING == SKILL_COUNT - 1);
};

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum SkillCategory {
    Technical,
    Mental,
    Physical,
    /// GK-specific skills: peak later (28-33), decline slowly — GKs have
    /// long careers.
    Goalkeeping,
}

pub(super) fn skill_category(idx: usize) -> SkillCategory {
    match idx {
        SK_ACCELERATION | SK_AGILITY | SK_BALANCE | SK_JUMPING | SK_NATURAL_FITNESS | SK_PACE
        | SK_STAMINA | SK_STRENGTH | SK_MATCH_READINESS => SkillCategory::Physical,
        SK_AGGRESSION | SK_ANTICIPATION | SK_BRAVERY | SK_COMPOSURE | SK_CONCENTRATION
        | SK_DECISIONS | SK_DETERMINATION | SK_FLAIR | SK_LEADERSHIP | SK_OFF_THE_BALL
        | SK_POSITIONING | SK_TEAMWORK | SK_VISION | SK_WORK_RATE => SkillCategory::Mental,
        SK_GK_AERIAL_REACH..=SK_GK_THROWING => SkillCategory::Goalkeeping,
        _ => SkillCategory::Technical,
    }
}

pub(super) fn skills_to_array(player: &Player) -> [f32; SKILL_COUNT] {
    let t = &player.skills.technical;
    let m = &player.skills.mental;
    let p = &player.skills.physical;
    let g = &player.skills.goalkeeping;
    [
        t.corners,
        t.crossing,
        t.dribbling,
        t.finishing,
        t.first_touch,
        t.free_kicks,
        t.heading,
        t.long_shots,
        t.long_throws,
        t.marking,
        t.passing,
        t.penalty_taking,
        t.tackling,
        t.technique,
        m.aggression,
        m.anticipation,
        m.bravery,
        m.composure,
        m.concentration,
        m.decisions,
        m.determination,
        m.flair,
        m.leadership,
        m.off_the_ball,
        m.positioning,
        m.teamwork,
        m.vision,
        m.work_rate,
        p.acceleration,
        p.agility,
        p.balance,
        p.jumping,
        p.natural_fitness,
        p.pace,
        p.stamina,
        p.strength,
        p.match_readiness,
        g.aerial_reach,
        g.command_of_area,
        g.communication,
        g.eccentricity,
        g.first_touch,
        g.handling,
        g.kicking,
        g.one_on_ones,
        g.passing,
        g.punching,
        g.reflexes,
        g.rushing_out,
        g.throwing,
    ]
}

pub(super) fn write_skills_back(player: &mut Player, arr: &[f32; SKILL_COUNT]) {
    let t = &mut player.skills.technical;
    t.corners = arr[SK_CORNERS];
    t.crossing = arr[SK_CROSSING];
    t.dribbling = arr[SK_DRIBBLING];
    t.finishing = arr[SK_FINISHING];
    t.first_touch = arr[SK_FIRST_TOUCH];
    t.free_kicks = arr[SK_FREE_KICKS];
    t.heading = arr[SK_HEADING];
    t.long_shots = arr[SK_LONG_SHOTS];
    t.long_throws = arr[SK_LONG_THROWS];
    t.marking = arr[SK_MARKING];
    t.passing = arr[SK_PASSING];
    t.penalty_taking = arr[SK_PENALTY_TAKING];
    t.tackling = arr[SK_TACKLING];
    t.technique = arr[SK_TECHNIQUE];

    let m = &mut player.skills.mental;
    m.aggression = arr[SK_AGGRESSION];
    m.anticipation = arr[SK_ANTICIPATION];
    m.bravery = arr[SK_BRAVERY];
    m.composure = arr[SK_COMPOSURE];
    m.concentration = arr[SK_CONCENTRATION];
    m.decisions = arr[SK_DECISIONS];
    m.determination = arr[SK_DETERMINATION];
    m.flair = arr[SK_FLAIR];
    m.leadership = arr[SK_LEADERSHIP];
    m.off_the_ball = arr[SK_OFF_THE_BALL];
    m.positioning = arr[SK_POSITIONING];
    m.teamwork = arr[SK_TEAMWORK];
    m.vision = arr[SK_VISION];
    m.work_rate = arr[SK_WORK_RATE];

    let p = &mut player.skills.physical;
    p.acceleration = arr[SK_ACCELERATION];
    p.agility = arr[SK_AGILITY];
    p.balance = arr[SK_BALANCE];
    p.jumping = arr[SK_JUMPING];
    p.natural_fitness = arr[SK_NATURAL_FITNESS];
    p.pace = arr[SK_PACE];
    p.stamina = arr[SK_STAMINA];
    p.strength = arr[SK_STRENGTH];
    p.match_readiness = arr[SK_MATCH_READINESS];

    let g = &mut player.skills.goalkeeping;
    g.aerial_reach = arr[SK_GK_AERIAL_REACH];
    g.command_of_area = arr[SK_GK_COMMAND_OF_AREA];
    g.communication = arr[SK_GK_COMMUNICATION];
    g.eccentricity = arr[SK_GK_ECCENTRICITY];
    g.first_touch = arr[SK_GK_FIRST_TOUCH];
    g.handling = arr[SK_GK_HANDLING];
    g.kicking = arr[SK_GK_KICKING];
    g.one_on_ones = arr[SK_GK_ONE_ON_ONES];
    g.passing = arr[SK_GK_PASSING];
    g.punching = arr[SK_GK_PUNCHING];
    g.reflexes = arr[SK_GK_REFLEXES];
    g.rushing_out = arr[SK_GK_RUSHING_OUT];
    g.throwing = arr[SK_GK_THROWING];
}
