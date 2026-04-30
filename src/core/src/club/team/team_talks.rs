//! Team talks — manager speech moments before/during/after matches
//! that nudge squad morale. FM uses team talks as a core tuning knob for
//! dressing-room dynamics; here we provide a minimal skeleton that applies
//! `DressingRoomSpeech` happiness events based on the chosen tone and the
//! manager's Man Management / Motivating attributes.
//!
//! The match engine can call `apply_pre_match_talk` at kickoff,
//! `apply_half_time_talk` at 45', and `apply_full_time_talk` at 90'.

use crate::club::HappinessEventType;
use crate::club::Staff;
use crate::club::player::Player;
use chrono::NaiveDate;

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
///
/// Beyond the basic tone/personality blend, this version:
///   * Reads / writes coach rapport via [`PlayerRapport`]. Trusted coaches
///     land praise harder; an untrusted coach's criticism is inflammatory.
///   * Detects repeated tone within a 14-day window and dampens magnitude
///     by 35% (the dressing room tunes out a manager who keeps saying the
///     same thing).
///   * Lets determination/professionalism convert criticism into motivation
///     for thick-skinned pros, while soft-temperament players take a
///     50%-larger hit.
pub fn apply_team_talk<'a, I>(
    players: I,
    manager: Option<&Staff>,
    tone: TeamTalkTone,
    ctx: TeamTalkContext,
) where
    I: IntoIterator<Item = &'a mut Player>,
{
    apply_team_talk_dated(players, manager, tone, ctx, None);
}

/// Variant that takes the simulation date so rapport updates and "repeated
/// tone" detection can be honest. Falls back to the original behaviour when
/// `now` is `None` — keeps the old call site happy without forcing a
/// signature change everywhere.
pub fn apply_team_talk_dated<'a, I>(
    players: I,
    manager: Option<&Staff>,
    tone: TeamTalkTone,
    ctx: TeamTalkContext,
    now: Option<NaiveDate>,
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
    let coach_id = manager.map(|m| m.id);

    for player in players {
        let personality = &player.attributes;
        let pressure = personality.pressure;
        let temperament = personality.temperament;
        let important_matches = personality.important_matches;
        let determination = player.skills.mental.determination;
        let professionalism = personality.professionalism;

        // Base magnitude per tone — wider band so important talks have real
        // weight while encouragement stays a soft cushion.
        let base: f32 = match tone {
            TeamTalkTone::Praise => 2.5,
            TeamTalkTone::Criticise => -1.0,
            TeamTalkTone::Encourage => 1.5,
            TeamTalkTone::Passionate => {
                if ctx.big_match {
                    2.0
                } else {
                    0.8
                }
            }
            TeamTalkTone::TacticalSilent => 0.0,
        };

        // Personality modulation — football-specific responses to each tone.
        let personality_mod: f32 = match tone {
            TeamTalkTone::Praise => {
                let mut m = 0.0;
                // Pressure ≥ 15: soaks up praise. Pressure ≤ 7 in big match
                // shrinks under it — they fear they can't live up.
                if pressure >= 15.0 {
                    m += 0.20 * 2.5;
                }
                if pressure <= 7.0 && ctx.big_match {
                    m -= 0.20 * 2.5;
                }
                m
            }
            TeamTalkTone::Criticise => {
                // Determined / professional pros absorb criticism and
                // convert it into motivation. Soft-temperament players
                // crumble — extra 50% hit.
                if determination >= 15.0 && professionalism >= 14.0 {
                    // Net positive — we need to overcome the negative base
                    // and end up at +1.0. base is -1.0, so this adds +2.0
                    // to flip sign.
                    2.0
                } else if temperament <= 7.0 {
                    -0.5 * 1.0 // additional 50% hit relative to base
                } else {
                    0.0
                }
            }
            TeamTalkTone::Encourage => {
                if pressure >= 15.0 {
                    0.20 * 1.5
                } else {
                    0.0
                }
            }
            TeamTalkTone::Passionate => {
                if ctx.big_match && important_matches >= 15.0 {
                    0.40 * 2.0
                } else if ctx.big_match && important_matches <= 6.0 {
                    -0.25 * 2.0
                } else {
                    0.0
                }
            }
            TeamTalkTone::TacticalSilent => 0.0,
        };

        // Phase weighting — half-time talks matter more when losing,
        // full-time praise matters more when winning.
        let phase_mod: f32 = match (ctx.phase, ctx.score_delta) {
            (MatchPhase::HalfTime, d) if d < 0 => 1.3,
            (MatchPhase::FullTime, d) if d > 0 => 1.2,
            (MatchPhase::FullTime, d) if d < 0 => 0.8,
            _ => 1.0,
        };

        // Rapport multiplier — trusted coaches land praise harder; from an
        // untrusted coach, criticism becomes inflammatory.
        let raw_signal = base + personality_mod;
        let positive_tone = raw_signal >= 0.0;
        let rapport_mul = if let Some(cid) = coach_id {
            player.rapport.talk_reception_multiplier(cid, positive_tone)
        } else {
            1.0
        };

        // Repeated-same-tone decay — speeches lose impact if the manager
        // delivered the same tone in the last 14 days.
        let repetition_mul = if recently_repeated_tone(player, tone, 14) {
            0.65
        } else {
            1.0
        };

        let magnitude = raw_signal * effectiveness * phase_mod * rapport_mul * repetition_mul;

        if magnitude.abs() >= 0.3 {
            player
                .happiness
                .add_event(HappinessEventType::DressingRoomSpeech, magnitude);
        }

        // Rapport feedback — positive talk that landed lifts rapport;
        // criticism that backfired (negative magnitude on a low-rapport
        // coach) hurts it. Skip if we don't know the coach id yet.
        if let (Some(cid), Some(today)) = (coach_id, now) {
            if positive_tone && magnitude >= 1.0 {
                player.rapport.on_positive(cid, today, 1);
            } else if !positive_tone && magnitude <= -1.0 {
                player.rapport.on_negative(cid, today, 2);
            }
        }
    }
}

/// True if the player received a `DressingRoomSpeech` of approximately the
/// same tone within `window_days`. We can't see the original tone in the
/// stored event, so we approximate with magnitude sign — the +/- band is
/// what the dressing room actually tunes out.
fn recently_repeated_tone(player: &Player, tone: TeamTalkTone, window_days: u16) -> bool {
    let want_positive = matches!(
        tone,
        TeamTalkTone::Praise | TeamTalkTone::Encourage | TeamTalkTone::Passionate
    );
    player.happiness.recent_events.iter().any(|e| {
        e.event_type == HappinessEventType::DressingRoomSpeech
            && e.days_ago <= window_days
            && (e.magnitude > 0.5) == want_positive
            && e.magnitude.abs() >= 0.5
    })
}
