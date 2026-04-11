//! Team talks — manager speech moments before/during/after matches
//! that nudge squad morale. FM uses team talks as a core tuning knob for
//! dressing-room dynamics; here we provide a minimal skeleton that applies
//! `DressingRoomSpeech` happiness events based on the chosen tone and the
//! manager's Man Management / Motivating attributes.
//!
//! The match engine can call `apply_pre_match_talk` at kickoff,
//! `apply_half_time_talk` at 45', and `apply_full_time_talk` at 90'.

use crate::club::player::Player;
use crate::club::HappinessEventType;
use crate::club::Staff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamTalkTone {
    /// "You lot are brilliant — go out and show them." Morale boost for
    /// mentally strong players, but soft players shrink under the pressure.
    Praise,
    /// "You owe me a performance here." Tough love; works for thick-skinned
    /// pros, upsets nervy ones.
    Criticise,
    /// "I have complete faith in you." Safe middle ground; small uplift.
    Encourage,
    /// "The fans, the shirt, the city — this is bigger than us." Passionate
    /// appeal; big reward if the players buy in, nothing if they don't.
    Passionate,
    /// No talk — tactical silence. No morale effect.
    TacticalSilent,
}

#[derive(Debug, Clone, Copy)]
pub struct TeamTalkContext {
    /// Where the match stands when the talk is given (see `MatchPhase`).
    pub phase: MatchPhase,
    /// Score delta from the talking team's perspective.
    pub score_delta: i8,
    /// Is this a "big" match (derby / cup final / continental)?
    pub big_match: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchPhase {
    PreMatch,
    HalfTime,
    FullTime,
}

/// Apply a team talk to every player on the given team. The magnitude of
/// the morale event depends on the tone, the manager's Man Management
/// attribute, and each player's personality (Pressure, Temperament,
/// ImportantMatches for big-match moments).
pub fn apply_team_talk<'a, I>(
    players: I,
    manager: Option<&Staff>,
    tone: TeamTalkTone,
    ctx: TeamTalkContext,
) where
    I: IntoIterator<Item = &'a mut Player>,
{
    if matches!(tone, TeamTalkTone::TacticalSilent) {
        return;
    }

    let man_mgmt = manager
        .map(|m| m.staff_attributes.mental.man_management as f32 / 20.0)
        .unwrap_or(0.5);
    let motivating = manager
        .map(|m| m.staff_attributes.mental.motivating as f32 / 20.0)
        .unwrap_or(0.5);
    let effectiveness = (man_mgmt * 0.6 + motivating * 0.4).clamp(0.1, 1.0);

    for player in players {
        let personality = &player.attributes;
        let pressure = personality.pressure / 20.0; // 0..1
        let temperament = personality.temperament / 20.0;
        let important_matches = personality.important_matches / 20.0;

        // Base magnitude per tone
        let base: f32 = match tone {
            TeamTalkTone::Praise => 3.0,
            TeamTalkTone::Criticise => -1.5,
            TeamTalkTone::Encourage => 1.5,
            TeamTalkTone::Passionate => 2.5,
            TeamTalkTone::TacticalSilent => 0.0,
        };

        // Personality modulation — how the player receives the tone
        let personality_mod: f32 = match tone {
            // Praise helps thick-skinned players, hurts nervy ones
            TeamTalkTone::Praise => (pressure - 0.5) * 2.0,
            // Criticism lands harder on sensitive temperaments
            TeamTalkTone::Criticise => -(temperament - 0.5) * 1.5,
            // Encourage is safe — small bump, scales a bit with pressure
            TeamTalkTone::Encourage => (pressure - 0.3) * 0.5,
            // Passionate lands for big-match performers
            TeamTalkTone::Passionate => {
                let multiplier = if ctx.big_match { 1.5 } else { 0.8 };
                ((important_matches - 0.4) * 1.5 + 0.5) * multiplier
            }
            TeamTalkTone::TacticalSilent => 0.0,
        };

        // Phase weighting — half-time talks matter more when losing,
        // full-time praise matters more when winning
        let phase_mod: f32 = match (ctx.phase, ctx.score_delta) {
            (MatchPhase::HalfTime, d) if d < 0 => 1.3,
            (MatchPhase::FullTime, d) if d > 0 => 1.2,
            (MatchPhase::FullTime, d) if d < 0 => 0.8,
            _ => 1.0,
        };

        let magnitude = (base + personality_mod) * effectiveness * phase_mod;

        if magnitude.abs() >= 0.3 {
            player
                .happiness
                .add_event(HappinessEventType::DressingRoomSpeech, magnitude);
        }
    }
}
