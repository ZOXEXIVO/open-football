//! What an in-match injury is: how it came about and how bad it is. The
//! severity is decided on the pitch and is what the player carries out of
//! the match.

use crate::r#match::engine::officiating::injury_stoppage::InjuryStoppage;
use crate::r#match::engine::player::events::players::FoulSeverity;
use crate::r#match::player::state::PlayerState;
use crate::r#match::{MatchContext, MatchField, MatchPlayer};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjuryCause {
    /// A tackle or a collision.
    Contact,
    /// Fatigue, jadedness, age and the conditions, with nobody near.
    Load,
}

impl InjuryCause {
    pub const COUNT: usize = 2;
    pub const NAMES: [&'static str; Self::COUNT] = ["contact", "load"];

    pub fn index(self) -> usize {
        match self {
            InjuryCause::Contact => 0,
            InjuryCause::Load => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum InjuryGrade {
    /// Shaken off after treatment; nothing persists.
    Knock,
    /// Plays on diminished, or is replaced when the bench pressure says so.
    Hurt,
    /// Match over, and an injury to carry out of it.
    Serious,
}

impl InjuryGrade {
    pub const COUNT: usize = 3;
    pub const NAMES: [&'static str; Self::COUNT] = ["knock", "hurt", "serious"];

    pub fn index(self) -> usize {
        match self {
            InjuryGrade::Knock => 0,
            InjuryGrade::Hurt => 1,
            InjuryGrade::Serious => 2,
        }
    }
}

/// **How likely a player is to get hurt, and how badly.** Two ways in —
/// a contact (a tackle, a foul) and the slow accumulation of load (tired
/// legs, a jaded body, an old one, the conditions) — and one severity
/// draw for both.
pub struct InjuryRisk;

impl InjuryRisk {
    /// A tackle that took the ball cleanly still put a body into a man.
    pub const CLEAN_TACKLE_IMPULSE: f32 = 0.10;
    /// Chance of an injury per unit of impulse for an average body: a
    /// normal foul (0.25) hurts somebody about one time in four hundred.
    /// With `LOAD_EXPOSURE` it is calibrated to the season volume the
    /// world had before injuries were decided on the pitch: about 0.08
    /// hurts or worse a match, three quarters of what the post-match roll
    /// alone used to carry. At ×5.7 the in-match layer quadrupled every
    /// club's injury list.
    const CONTACT_RATE: f32 = 0.0105;
    /// Load checks run every 6-14 minutes over the whole match.
    const LOAD_EXPOSURE: f32 = 0.105;
    /// How strongly the conditions scale either channel: the environment's
    /// `injury_risk` of 0.07 (heavy rain on a muddy pitch) is a third more.
    const CONDITIONS_GAIN: f32 = 5.0;

    /// Time a player is down for, in match milliseconds: the knock he gets
    /// up from, the treatment he plays on after, and the one he is carried
    /// off from.
    pub const KNOCK_MS: u64 = 15_000;
    pub const HURT_MS: u64 = 60_000;
    pub const SERIOUS_MS: u64 = 120_000;

    pub fn treatment_ms(severity: InjuryGrade) -> u64 {
        match severity {
            InjuryGrade::Knock => Self::KNOCK_MS,
            InjuryGrade::Hurt => Self::HURT_MS,
            InjuryGrade::Serious => Self::SERIOUS_MS,
        }
    }

    pub fn foul_impulse(severity: FoulSeverity) -> f32 {
        match severity {
            FoulSeverity::Normal => 0.25,
            FoulSeverity::Reckless => 0.60,
            FoulSeverity::Violent => 0.90,
        }
    }

    /// Chance that a contact of `impulse` injures `player`.
    pub fn contact_chance(player: &MatchPlayer, impulse: f32, conditions: f32) -> f32 {
        Self::CONTACT_RATE
            * impulse.clamp(0.0, 1.0)
            * (0.6 + 0.8 * Self::fragility(player))
            * Self::conditions_factor(conditions)
    }

    /// Chance that a load check finds `player` injured.
    pub fn load_chance(player: &MatchPlayer, match_minute: u32, conditions: f32) -> f32 {
        let jaded = (player.player_attributes.jadedness as f32 / 10_000.0).clamp(0.0, 1.0);
        let cond = (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
        let nat_fit = (player.skills.physical.natural_fitness / 20.0).clamp(0.02, 1.0);
        let minutes = (match_minute as f32 / 90.0).clamp(0.0, 1.2);
        let keeper = if player.tactical_position.current_position.is_goalkeeper() {
            0.35
        } else {
            1.0
        };
        (0.0005 + jaded * 0.004 + (1.0 - cond) * 0.003 + (1.0 - nat_fit) * 0.002 + minutes * 0.001)
            * Self::LOAD_EXPOSURE
            * keeper
            * Self::conditions_factor(conditions)
    }

    /// One roll decides how bad it is. A harder contact and a more fragile
    /// body both move the odds toward the serious end; a load injury is
    /// more often a strain than a knock.
    pub fn severity(
        cause: InjuryCause,
        impulse: f32,
        player: &MatchPlayer,
        roll: f32,
    ) -> InjuryGrade {
        let shift = (Self::fragility(player) - 0.5) * 0.2;
        let (serious, hurt) = match cause {
            InjuryCause::Contact => (0.04 + 0.25 * impulse, 0.15 + 0.25 * impulse),
            InjuryCause::Load => (0.20, 0.35),
        };
        let serious = (serious + shift).clamp(0.01, 0.60);
        let hurt = (hurt + shift).clamp(0.05, 0.60);
        if roll < serious {
            InjuryGrade::Serious
        } else if roll < serious + hurt {
            InjuryGrade::Hurt
        } else {
            InjuryGrade::Knock
        }
    }

    /// 0 for a robust body, 1 for a fragile one; 0.5 is the median pro.
    fn fragility(player: &MatchPlayer) -> f32 {
        let prone = (player.player_attributes.injury_proneness as f32 / 20.0).clamp(0.0, 1.0);
        let fit = (player.skills.physical.natural_fitness / 20.0).clamp(0.0, 1.0);
        (0.6 * prone + 0.4 * (1.0 - fit)).clamp(0.0, 1.0)
    }

    fn conditions_factor(conditions: f32) -> f32 {
        1.0 + Self::CONDITIONS_GAIN * conditions.clamp(0.0, 0.1)
    }
}

/// **Something has hurt a player in this match.** The player reacts, the
/// match records it, and the referee decides whether play stops.
pub struct MatchInjury;

impl MatchInjury {
    /// Roll whether a contact of `impulse` on `victim_id` hurt him, and how
    /// badly. Nothing is applied — the caller decides when, because a foul
    /// is whistled before the physio comes on.
    pub fn roll_contact(
        field: &MatchField,
        context: &mut MatchContext,
        victim_id: u32,
        impulse: f32,
    ) -> Option<InjuryGrade> {
        let victim = field.get_player(victim_id)?;
        if victim.off_pitch || victim.state == PlayerState::Injured {
            return None;
        }
        let conditions = context.conditions.injury_risk;
        if context.rng.unit_f32() >= InjuryRisk::contact_chance(victim, impulse, conditions) {
            return None;
        }
        Some(InjuryRisk::severity(
            InjuryCause::Contact,
            impulse,
            victim,
            context.rng.unit_f32(),
        ))
    }

    /// Roll the load check across everybody on the pitch.
    pub fn roll_load(field: &mut MatchField, context: &mut MatchContext) {
        let match_minute = (context.total_match_time / 60_000) as u32;
        if match_minute < 5 {
            return;
        }
        let conditions = context.conditions.injury_risk;
        let mut victims: Vec<(u32, InjuryGrade)> = Vec::new();
        for player in field.players.iter() {
            if player.off_pitch || player.state == PlayerState::Injured {
                continue;
            }
            if context.rng.unit_f32() < InjuryRisk::load_chance(player, match_minute, conditions) {
                let severity =
                    InjuryRisk::severity(InjuryCause::Load, 0.0, player, context.rng.unit_f32());
                victims.push((player.id, severity));
            }
        }
        for (id, severity) in victims {
            Self::befall(field, context, id, InjuryCause::Load, severity);
        }
    }

    pub fn befall(
        field: &mut MatchField,
        context: &mut MatchContext,
        player_id: u32,
        cause: InjuryCause,
        severity: InjuryGrade,
    ) {
        let Some(player) = field.get_player_mut(player_id) else {
            return;
        };
        player.on_injury(severity, context.total_match_time);
        context.tally.note_injury(cause, severity);
        InjuryStoppage::call(field, context, severity);
    }
}
