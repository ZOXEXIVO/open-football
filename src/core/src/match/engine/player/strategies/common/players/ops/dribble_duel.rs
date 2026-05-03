//! 1v1 dribble-duel resolver.
//!
//! Replaces the "ball carrier steers around opponents until timeout"
//! shape with a real attacker-vs-defender duel. Triggered when the
//! carrier enters a defender's pressure cone (within ~10u, ahead/side).
//! Each side scores a duel rating from skills + traits + context, and
//! a sigmoid converts the gap into a beat-probability in [0.12, 0.88].
//! The outcome roll then chooses the specific success / failure
//! flavour: clean beat, heavy-touch beat, foul drawn, clean tackle,
//! lost ball, or foul committed.

use crate::club::player::traits::PlayerTrait;
use crate::r#match::MatchPlayer;
use crate::r#match::engine::player::strategies::common::players::ops::effective_skill::{
    ActionContext, effective_skill,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DribbleOutcome {
    /// Attacker beats the defender cleanly — keeps the ball at feet.
    BeatManClean,
    /// Attacker beats the defender but with a heavy touch — the ball
    /// runs ahead and is briefly contested.
    BeatManButHeavyTouch,
    /// Attacker draws a foul and wins a free kick / penalty.
    WinsFoul,
    /// Defender cleanly takes the ball — possession turnover.
    TackledClean,
    /// Attacker loses control — ball runs loose, neither in possession.
    LosesBallLoose,
    /// Defender commits a foul — attacker keeps the ball or wins a set
    /// piece depending on the caller's context.
    CommitsFoul,
}

impl DribbleOutcome {
    pub fn is_attacker_win(self) -> bool {
        matches!(
            self,
            DribbleOutcome::BeatManClean
                | DribbleOutcome::BeatManButHeavyTouch
                | DribbleOutcome::WinsFoul
                | DribbleOutcome::CommitsFoul
        )
    }

    pub fn is_foul(self) -> bool {
        matches!(self, DribbleOutcome::WinsFoul | DribbleOutcome::CommitsFoul)
    }
}

/// Per-duel context that the geometry / state machine knows but the
/// resolver itself can't infer from skills alone.
#[derive(Debug, Clone, Copy, Default)]
pub struct DuelContext {
    /// Attacker is moving at sprinting pace.
    pub attacker_running_at_speed: bool,
    /// Defender is squared up rather than side-on (vulnerable).
    pub defender_squared_up: bool,
    /// Attacker is isolated 1v1 in a wide channel — attacker bonus.
    pub isolated_wide: bool,
    /// A second defender is within 8u to provide cover — defender bonus.
    pub second_defender_cover: bool,
    /// Attacker is in a crowded central zone (2+ defenders / heavy
    /// traffic). Some traits read this for risk adjustment.
    pub crowded_central: bool,
    /// Match minute — feeds the fatigue model.
    pub minute: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct DuelResolution {
    pub outcome: DribbleOutcome,
    pub beat_probability: f32,
    pub attacker_score: f32,
    pub defender_score: f32,
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn attacker_trait_bonus(player: &MatchPlayer, ctx: DuelContext) -> f32 {
    let mut bonus = 0.0;
    if player.has_trait(PlayerTrait::RunsWithBallOften) {
        bonus += 0.03;
    }
    if player.has_trait(PlayerTrait::TriesTricks) {
        bonus += 0.07;
    }
    if player.has_trait(PlayerTrait::KnocksBallPast) {
        // Strong vs slow defenders, weak in crowds.
        if ctx.crowded_central {
            bonus -= 0.06;
        } else {
            bonus += 0.08;
        }
    }
    if player.has_trait(PlayerTrait::BackheelsRegularly) {
        // Flair pass / unpredictable, slight advantage but raises
        // turnover risk handled at outcome stage.
        bonus += 0.03;
    }
    if player.has_trait(PlayerTrait::CutsInsideFromBothWings) && !ctx.crowded_central {
        bonus += 0.02;
    }
    bonus
}

fn defender_trait_bonus(player: &MatchPlayer, _ctx: DuelContext) -> f32 {
    let mut bonus = 0.0;
    if player.has_trait(PlayerTrait::DivesIntoTackles) {
        // Higher attempt rate but less stable; net +0.04 on score with
        // foul risk applied at outcome stage.
        bonus += 0.04;
    }
    if player.has_trait(PlayerTrait::StaysOnFeet) {
        // Steadier — wins fewer dramatic tackles but loses fewer fouls.
        bonus += 0.02;
    }
    if player.has_trait(PlayerTrait::MarkTightly) {
        bonus += 0.04;
    }
    bonus
}

fn attacker_score(attacker: &MatchPlayer, ctx: DuelContext) -> f32 {
    let tech_ctx = ActionContext::technical(ctx.minute);
    let mental_ctx = ActionContext::mental(ctx.minute);
    let expl_ctx = ActionContext::explosive(ctx.minute);
    let s = &attacker.skills;
    let dribbling = effective_skill(attacker, s.technical.dribbling, tech_ctx);
    let technique = effective_skill(attacker, s.technical.technique, tech_ctx);
    let flair = effective_skill(attacker, s.mental.flair, mental_ctx);
    let agility = effective_skill(attacker, s.physical.agility, expl_ctx);
    let acceleration = effective_skill(attacker, s.physical.acceleration, expl_ctx);
    let balance = effective_skill(attacker, s.physical.balance, tech_ctx);
    let composure = effective_skill(attacker, s.mental.composure, mental_ctx);
    let decisions = effective_skill(attacker, s.mental.decisions, mental_ctx);

    let base = dribbling * 0.26
        + technique * 0.18
        + flair * 0.10
        + agility * 0.14
        + acceleration * 0.10
        + balance * 0.08
        + composure * 0.06
        + decisions * 0.05;
    let mut score = base / 20.0;

    if ctx.attacker_running_at_speed {
        if acceleration >= 13.0 {
            score += 0.04;
        } else {
            score -= 0.03;
        }
    }
    if ctx.isolated_wide {
        score += 0.08;
    }
    let cond = (attacker.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
    if cond < 0.45 {
        score -= 0.08;
    }

    score += attacker_trait_bonus(attacker, ctx);
    score
}

fn defender_score(defender: &MatchPlayer, ctx: DuelContext) -> f32 {
    let tech_ctx = ActionContext::technical(ctx.minute);
    let mental_ctx = ActionContext::mental(ctx.minute);
    let expl_ctx = ActionContext::explosive(ctx.minute);
    let s = &defender.skills;
    let tackling = effective_skill(defender, s.technical.tackling, tech_ctx);
    let positioning = effective_skill(defender, s.mental.positioning, mental_ctx);
    let anticipation = effective_skill(defender, s.mental.anticipation, mental_ctx);
    let marking = effective_skill(defender, s.technical.marking, tech_ctx);
    let strength = effective_skill(defender, s.physical.strength, expl_ctx);
    let balance = effective_skill(defender, s.physical.balance, tech_ctx);
    let agility = effective_skill(defender, s.physical.agility, expl_ctx);
    let concentration = effective_skill(defender, s.mental.concentration, mental_ctx);

    let base = tackling * 0.22
        + positioning * 0.18
        + anticipation * 0.16
        + marking * 0.10
        + strength * 0.08
        + balance * 0.08
        + agility * 0.08
        + concentration * 0.06;
    let mut score = base / 20.0;

    if ctx.defender_squared_up {
        score += 0.07;
    }
    if ctx.second_defender_cover {
        score += 0.12;
    }
    let cond = (defender.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
    if cond < 0.45 {
        score -= 0.08;
    }

    score += defender_trait_bonus(defender, ctx);
    score
}

/// Resolve a 1v1 duel. `roll` is a uniform [0, 1) random used for both
/// the win/loss decision and the specific outcome category.
pub fn resolve_dribble_duel(
    attacker: &MatchPlayer,
    defender: &MatchPlayer,
    ctx: DuelContext,
    roll: f32,
) -> DuelResolution {
    let a_score = attacker_score(attacker, ctx);
    let d_score = defender_score(defender, ctx);
    let beat_prob = sigmoid((a_score - d_score) * 2.4).clamp(0.12, 0.88);

    // Foul risk bumps from traits.
    let attacker_foul_drawn_bonus: f32 = if attacker.has_trait(PlayerTrait::TriesTricks) {
        0.02
    } else {
        0.0
    };
    let defender_foul_risk: f32 = if defender.has_trait(PlayerTrait::DivesIntoTackles) {
        0.08
    } else if defender.has_trait(PlayerTrait::StaysOnFeet) {
        -0.05
    } else {
        0.0
    };
    let attacker_loose_risk: f32 = if attacker.has_trait(PlayerTrait::TriesTricks) {
        0.04
    } else if attacker.has_trait(PlayerTrait::BackheelsRegularly) {
        0.05
    } else {
        0.0
    };

    // Outcome split.
    let outcome = if roll < beat_prob {
        // Success branch — apportion among 4 outcomes: clean beat,
        // heavy-touch beat, foul drawn, foul committed (rare here).
        let r = roll / beat_prob; // re-normalize into [0, 1)
        let foul_drawn = (0.13 + attacker_foul_drawn_bonus).clamp(0.05, 0.22);
        let heavy = 0.42;
        if r < foul_drawn {
            DribbleOutcome::WinsFoul
        } else if r < foul_drawn + heavy {
            DribbleOutcome::BeatManButHeavyTouch
        } else {
            DribbleOutcome::BeatManClean
        }
    } else {
        // Failure branch — clean tackle, loose ball, defender foul.
        let r = (roll - beat_prob) / (1.0 - beat_prob).max(1e-3);
        let foul = (0.10 + defender_foul_risk).clamp(0.04, 0.20);
        let loose = (0.32 + attacker_loose_risk).clamp(0.20, 0.45);
        if r < foul {
            DribbleOutcome::CommitsFoul
        } else if r < foul + loose {
            DribbleOutcome::LosesBallLoose
        } else {
            DribbleOutcome::TackledClean
        }
    };

    DuelResolution {
        outcome,
        beat_probability: beat_prob,
        attacker_score: a_score,
        defender_score: d_score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerSkills;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
    };
    use chrono::NaiveDate;

    fn build(
        dribbling: f32,
        technique: f32,
        flair: f32,
        agility: f32,
        accel: f32,
        tackling: f32,
        positioning: f32,
        anticipation: f32,
        traits: Vec<PlayerTrait>,
    ) -> MatchPlayer {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = 9000;
        let mut skills = PlayerSkills::default();
        skills.technical.dribbling = dribbling;
        skills.technical.technique = technique;
        skills.technical.tackling = tackling;
        skills.technical.marking = 12.0;
        skills.mental.flair = flair;
        skills.mental.positioning = positioning;
        skills.mental.anticipation = anticipation;
        skills.mental.composure = 12.0;
        skills.mental.decisions = 12.0;
        skills.mental.concentration = 12.0;
        skills.physical.agility = agility;
        skills.physical.acceleration = accel;
        skills.physical.balance = 12.0;
        skills.physical.strength = 12.0;
        skills.physical.stamina = 14.0;
        skills.physical.natural_fitness = 14.0;
        let mut player = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::ForwardCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        player.traits = traits;
        MatchPlayer::from_player(1, &player, PlayerPositionType::ForwardCenter, false)
    }

    #[test]
    fn elite_dribbler_beats_average_defender_more_often() {
        let elite = build(19.0, 18.0, 16.0, 17.0, 16.0, 6.0, 8.0, 8.0, vec![]);
        let avg_def = build(8.0, 8.0, 6.0, 10.0, 10.0, 12.0, 12.0, 12.0, vec![]);
        let mut wins = 0;
        for i in 0..200 {
            let roll = (i as f32 + 0.5) / 200.0;
            let r = resolve_dribble_duel(&elite, &avg_def, DuelContext::default(), roll);
            if r.outcome.is_attacker_win() {
                wins += 1;
            }
        }
        // Elite vs avg should comfortably exceed 60% wins.
        assert!(
            wins > 120,
            "Elite vs avg only won {} / 200 — expected > 120",
            wins
        );
    }

    #[test]
    fn dives_into_tackles_raises_foul_risk() {
        let attacker = build(14.0, 14.0, 10.0, 14.0, 14.0, 6.0, 8.0, 8.0, vec![]);
        let stays = build(
            8.0,
            8.0,
            6.0,
            10.0,
            10.0,
            14.0,
            14.0,
            14.0,
            vec![PlayerTrait::StaysOnFeet],
        );
        let dives = build(
            8.0,
            8.0,
            6.0,
            10.0,
            10.0,
            14.0,
            14.0,
            14.0,
            vec![PlayerTrait::DivesIntoTackles],
        );
        let mut stays_fouls = 0;
        let mut dives_fouls = 0;
        for i in 0..400 {
            let roll = (i as f32 + 0.5) / 400.0;
            let r1 = resolve_dribble_duel(&attacker, &stays, DuelContext::default(), roll);
            let r2 = resolve_dribble_duel(&attacker, &dives, DuelContext::default(), roll);
            if matches!(r1.outcome, DribbleOutcome::CommitsFoul) {
                stays_fouls += 1;
            }
            if matches!(r2.outcome, DribbleOutcome::CommitsFoul) {
                dives_fouls += 1;
            }
        }
        assert!(
            dives_fouls > stays_fouls,
            "DivesIntoTackles should foul more often (stays={}, dives={})",
            stays_fouls,
            dives_fouls
        );
    }

    #[test]
    fn second_defender_cover_helps_defenders() {
        let attacker = build(16.0, 16.0, 14.0, 14.0, 14.0, 6.0, 8.0, 8.0, vec![]);
        let defender = build(8.0, 8.0, 6.0, 10.0, 10.0, 12.0, 12.0, 12.0, vec![]);
        let solo = DuelContext::default();
        let cover = DuelContext {
            second_defender_cover: true,
            ..Default::default()
        };
        let mut solo_wins = 0;
        let mut cover_wins = 0;
        for i in 0..200 {
            let roll = (i as f32 + 0.5) / 200.0;
            let r1 = resolve_dribble_duel(&attacker, &defender, solo, roll);
            let r2 = resolve_dribble_duel(&attacker, &defender, cover, roll);
            if r1.outcome.is_attacker_win() {
                solo_wins += 1;
            }
            if r2.outcome.is_attacker_win() {
                cover_wins += 1;
            }
        }
        assert!(solo_wins > cover_wins);
    }
}
