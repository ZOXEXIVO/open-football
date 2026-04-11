//! Preferred Player Moves (PPMs) — signature behaviours that give players
//! identity in the match engine and scouting reports. FM calls these
//! "Player Traits" or "Preferred Player Moves".
//!
//! Traits modulate decision weights in the match-engine state machines:
//! a player with `TriesThroughBalls` will bias toward risky passes, one
//! with `HugsLine` keeps a wider average x-position, etc.

use crate::club::player::skills::PlayerSkills;
use crate::club::player::position::{PlayerPosition, PlayerFieldPositionGroup};
use crate::utils::FloatUtils;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerTrait {
    // Attacking movement
    CutsInsideFromBothWings,
    HugsLine,
    RunsWithBallOften,
    RunsWithBallRarely,
    GetsIntoOppositionArea,
    ArrivesLateInOppositionArea,
    StaysBack,
    // Passing
    TriesThroughBalls,
    LikesToSwitchPlay,
    LooksForPassRatherThanAttemptShot,
    PlaysShortPasses,
    PlaysLongPasses,
    // Shooting
    ShootsFromDistance,
    PlacesShots,
    PowersShots,
    TriesLobs,
    // Set-piece / specialism
    CurlsBall,
    KnocksBallPast,
    KillerBallOften,
    // Defensive
    DivesIntoTackles,
    StaysOnFeet,
    MarkTightly,
    // Personality on-pitch
    Playmaker,
    Argues,
    WindsUpOpponents,
    // Technical flair
    TriesTricks,
    BackheelsRegularly,
    OneClubPlayer,
}

impl PlayerTrait {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlayerTrait::CutsInsideFromBothWings => "Cuts inside from both wings",
            PlayerTrait::HugsLine => "Hugs line",
            PlayerTrait::RunsWithBallOften => "Runs with ball often",
            PlayerTrait::RunsWithBallRarely => "Runs with ball rarely",
            PlayerTrait::GetsIntoOppositionArea => "Gets into opposition area",
            PlayerTrait::ArrivesLateInOppositionArea => "Arrives late in opposition area",
            PlayerTrait::StaysBack => "Stays back at all times",
            PlayerTrait::TriesThroughBalls => "Tries killer balls often",
            PlayerTrait::LikesToSwitchPlay => "Likes to switch play",
            PlayerTrait::LooksForPassRatherThanAttemptShot => "Looks for pass rather than shot",
            PlayerTrait::PlaysShortPasses => "Plays short passes",
            PlayerTrait::PlaysLongPasses => "Plays long passes",
            PlayerTrait::ShootsFromDistance => "Shoots from distance",
            PlayerTrait::PlacesShots => "Places shots",
            PlayerTrait::PowersShots => "Powers shots",
            PlayerTrait::TriesLobs => "Tries lobs",
            PlayerTrait::CurlsBall => "Curls ball",
            PlayerTrait::KnocksBallPast => "Knocks ball past opponent",
            PlayerTrait::KillerBallOften => "Plays killer balls",
            PlayerTrait::DivesIntoTackles => "Dives into tackles",
            PlayerTrait::StaysOnFeet => "Stays on feet",
            PlayerTrait::MarkTightly => "Marks opponent tightly",
            PlayerTrait::Playmaker => "Dictates tempo",
            PlayerTrait::Argues => "Argues with officials",
            PlayerTrait::WindsUpOpponents => "Winds up opponents",
            PlayerTrait::TriesTricks => "Tries tricks",
            PlayerTrait::BackheelsRegularly => "Tries backheels",
            PlayerTrait::OneClubPlayer => "One club player",
        }
    }

    /// Traits plausibly acquired by the player's position group.
    fn candidates_for(group: PlayerFieldPositionGroup) -> &'static [PlayerTrait] {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => &[PlayerTrait::StaysBack],
            PlayerFieldPositionGroup::Defender => &[
                PlayerTrait::StaysBack,
                PlayerTrait::MarkTightly,
                PlayerTrait::StaysOnFeet,
                PlayerTrait::DivesIntoTackles,
                PlayerTrait::PlaysLongPasses,
                PlayerTrait::LikesToSwitchPlay,
            ],
            PlayerFieldPositionGroup::Midfielder => &[
                PlayerTrait::Playmaker,
                PlayerTrait::TriesThroughBalls,
                PlayerTrait::LikesToSwitchPlay,
                PlayerTrait::PlaysShortPasses,
                PlayerTrait::PlaysLongPasses,
                PlayerTrait::ShootsFromDistance,
                PlayerTrait::RunsWithBallOften,
                PlayerTrait::ArrivesLateInOppositionArea,
                PlayerTrait::CurlsBall,
                PlayerTrait::KillerBallOften,
                PlayerTrait::TriesTricks,
            ],
            PlayerFieldPositionGroup::Forward => &[
                PlayerTrait::CutsInsideFromBothWings,
                PlayerTrait::HugsLine,
                PlayerTrait::RunsWithBallOften,
                PlayerTrait::GetsIntoOppositionArea,
                PlayerTrait::ShootsFromDistance,
                PlayerTrait::PlacesShots,
                PlayerTrait::PowersShots,
                PlayerTrait::TriesLobs,
                PlayerTrait::KnocksBallPast,
                PlayerTrait::TriesTricks,
                PlayerTrait::BackheelsRegularly,
            ],
        }
    }
}

/// Roll traits for a new player based on their skills & position.
/// Better players get more traits and skill-biased selections.
pub fn generate_player_traits(
    skills: &PlayerSkills,
    positions: &[PlayerPosition],
    current_ability: u8,
) -> Vec<PlayerTrait> {
    // Trait count scales with ability: avg 0.4 traits at CA 40, ~2 at CA 150, up to 4 at CA 190+.
    let trait_count = if current_ability < 50 {
        if FloatUtils::random(0.0, 1.0) < 0.3 { 1 } else { 0 }
    } else if current_ability < 90 {
        1
    } else if current_ability < 140 {
        if FloatUtils::random(0.0, 1.0) < 0.4 { 2 } else { 1 }
    } else if current_ability < 170 {
        2
    } else if current_ability < 190 {
        3
    } else {
        4
    };

    if trait_count == 0 {
        return Vec::new();
    }

    let main_group = positions
        .first()
        .map(|p| p.position.position_group())
        .unwrap_or(PlayerFieldPositionGroup::Midfielder);

    let pool = PlayerTrait::candidates_for(main_group);
    if pool.is_empty() {
        return Vec::new();
    }

    let mut picked: Vec<PlayerTrait> = Vec::new();
    let mut attempts = 0;
    while picked.len() < trait_count && attempts < trait_count * 6 {
        attempts += 1;
        let idx = (FloatUtils::random(0.0, pool.len() as f32) as usize).min(pool.len() - 1);
        let candidate = pool[idx];

        if picked.contains(&candidate) {
            continue;
        }

        // Skill-gated filter: don't hand out "Shoots from distance" to a
        // midfielder with 5 Long Shots, or "Tries through balls" to a
        // 6 Passing CB.
        if !skill_supports_trait(&candidate, skills) {
            continue;
        }

        picked.push(candidate);
    }

    picked
}

fn skill_supports_trait(tr: &PlayerTrait, skills: &PlayerSkills) -> bool {
    let t = &skills.technical;
    let m = &skills.mental;
    match tr {
        PlayerTrait::ShootsFromDistance => t.long_shots >= 12.0,
        PlayerTrait::PlacesShots => t.finishing >= 12.0,
        PlayerTrait::PowersShots => t.finishing >= 11.0 && t.long_shots >= 11.0,
        PlayerTrait::TriesLobs => t.technique >= 12.0,
        PlayerTrait::CurlsBall => t.technique >= 13.0 && t.crossing >= 11.0,
        PlayerTrait::TriesThroughBalls | PlayerTrait::KillerBallOften => {
            t.passing >= 13.0 && m.vision >= 13.0
        }
        PlayerTrait::Playmaker => t.passing >= 14.0 && m.vision >= 14.0,
        PlayerTrait::LikesToSwitchPlay | PlayerTrait::PlaysLongPasses => t.passing >= 12.0,
        PlayerTrait::RunsWithBallOften | PlayerTrait::KnocksBallPast => {
            t.dribbling >= 12.0 && t.technique >= 11.0
        }
        PlayerTrait::TriesTricks | PlayerTrait::BackheelsRegularly => {
            t.technique >= 14.0 && t.dribbling >= 13.0
        }
        PlayerTrait::DivesIntoTackles => t.tackling >= 11.0,
        PlayerTrait::StaysOnFeet => m.positioning >= 12.0 && t.tackling >= 11.0,
        PlayerTrait::MarkTightly => m.positioning >= 12.0 && m.concentration >= 12.0,
        _ => true,
    }
}
