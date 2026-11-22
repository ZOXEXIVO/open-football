use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::shared::{
    ActivityIntensity, DefenderCondition, DefenderRead,
};
use crate::r#match::events::Event;
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::defender_census::{ChallengeOutcome, DefenderCensus};
use crate::r#match::player::events::{FoulCommitted, FoulSource, PlayerEvent};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::shared::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::shared::states::{
    BlockChallenge, ChallengeKind, ClosingPoint, RecoveryChallenge, TackleDecision, TackleOutcome,
};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
#[cfg(feature = "match-logs")]
use crate::tackle_stats::DEF_TICKS_IN_STATE;
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

/// **The challenge.** Entered with the kind of challenge staged on him —
/// the block tackle from containing, the stretch from being beaten — it
/// rolls once, on entry, and leaves in the same tick: won, with the ball;
/// missed, getting up; a foul, to the whistle.
#[derive(Default, Clone)]
pub struct DefenderTacklingState {}

impl StateProcessingHandler for DefenderTacklingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        #[cfg(feature = "match-logs")]
        DEF_TICKS_IN_STATE.fetch_add(1, Ordering::Relaxed);

        let (Some(kind), Some(carrier)) = (
            ctx.player.staged_challenge,
            ctx.players().opponents().with_ball().next(),
        ) else {
            return Some(DefenderRead::change(ctx));
        };
        #[cfg(feature = "match-logs")]
        crate::tackle_stats::DEF_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
        let outcome = match kind {
            ChallengeKind::Block => self.attempt_block_tackle(ctx, &carrier),
            _ => {
                let distance = carrier.distance(ctx);
                let lead = RecoveryChallenge::lead(ctx, carrier.position);
                let minute = sc::minute_from_ms(ctx.context.total_match_time);
                RecoveryChallenge::resolve(
                    ctx,
                    sc::defensive_duel(ctx.player, minute),
                    DefenderSkillProfile::from_ctx(ctx).discipline,
                    TackleDecision::carrier_threat(ctx, &carrier),
                    distance,
                    lead,
                )
            }
        };
        #[cfg(feature = "match-logs")]
        DefenderCensus::note_challenge(
            ctx,
            carrier.id,
            carrier.position,
            kind,
            ChallengeOutcome::of(&outcome),
        );
        let mut result = self.settle(ctx, outcome, kind, carrier.id);
        result.start_tackle_cooldown = true;
        Some(result)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let carrier = ctx.players().opponents().with_ball().next()?;
        Some(
            SteeringBehavior::Pursuit {
                target: ClosingPoint::for_carrier(ctx, &carrier),
                target_velocity: Vector3::zeros(),
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tackling is explosive and very demanding physically
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl DefenderTacklingState {
    /// One resolved challenge, turned into the state change it implies.
    /// Both challenges end here so a win, a foul and a miss cannot mean
    /// different things depending on which one was made.
    fn settle(
        &self,
        ctx: &StateProcessingContext,
        outcome: TackleOutcome,
        kind: ChallengeKind,
        carrier_id: u32,
    ) -> StateChangeResult {
        match outcome {
            TackleOutcome::Won => {
                #[cfg(feature = "match-logs")]
                crate::tackle_stats::DEF_SUCCESSES.fetch_add(1, Ordering::Relaxed);
                StateChangeResult::with_defender_state_and_event(
                    DefenderState::Running,
                    Event::PlayerEvent(PlayerEvent::TacklingBall(ctx.player.id)),
                )
            }
            TackleOutcome::Foul(severity) => StateChangeResult::with_defender_state_and_event(
                DefenderState::HoldingLine,
                Event::PlayerEvent(PlayerEvent::CommitFoul(FoulCommitted {
                    fouler: ctx.player.id,
                    fouled: carrier_id,
                    severity,
                    source: FoulSource::Tackle,
                    challenge: Some(kind),
                })),
            ),
            TackleOutcome::Missed => StateChangeResult::with_missed_challenge(kind),
        }
    }

    fn attempt_block_tackle(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> TackleOutcome {
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        // The defender profile's discipline — the composure / decisions /
        // concentration blend — suppresses fouls.
        let profile = DefenderSkillProfile::from_ctx(ctx);
        BlockChallenge {
            tackle: sc::defensive_duel(ctx.player, minute),
            carry: TackleDecision::carrier_threat(ctx, opponent),
            aggression: ctx.player.skills.mental.aggression,
            discipline: profile.discipline,
            condition: profile.def_condition_mult,
            in_own_box: ctx
                .context
                .penalty_area(ctx.player.side == Some(PlayerSide::Left))
                .contains(&ctx.tick_context.positions.ball.position),
            booked: ctx.player.yellow_cards > 0,
            rush: BlockChallenge::rush_of(
                ctx.player,
                opponent.position,
                ctx.tick_context.positions.players.velocity(opponent.id),
            ),
        }
        .resolve(&ctx.context.rng, ctx.context.psychology.get(ctx.player.id))
    }
}

#[cfg(test)]
mod tests {
    use crate::PlayerSkills;
    use crate::club::player::builder::PlayerBuilder;
    use crate::r#match::MatchPlayer;
    use crate::r#match::player::strategies::players::ops::skill_composites as sc;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
    };
    use chrono::NaiveDate;

    fn defender(tackling: f32, marking: f32, positioning: f32) -> MatchPlayer {
        let attrs = PlayerAttributes {
            condition: 9000,
            ..Default::default()
        };
        let mut skills = PlayerSkills::default();
        skills.technical.tackling = tackling;
        skills.technical.marking = marking;
        skills.mental.positioning = positioning;
        skills.mental.anticipation = 12.0;
        skills.mental.concentration = 12.0;
        skills.mental.bravery = 12.0;
        skills.physical.strength = 13.0;
        skills.physical.balance = 12.0;
        skills.physical.agility = 12.0;
        skills.physical.stamina = 14.0;
        skills.physical.natural_fitness = 14.0;
        let p = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("D".into(), "Z".into()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::DefenderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(1, &p, PlayerPositionType::DefenderCenter, false, None)
    }

    fn attacker(dribbling: f32, technique: f32, agility: f32) -> MatchPlayer {
        let attrs = PlayerAttributes {
            condition: 9000,
            ..Default::default()
        };
        let mut skills = PlayerSkills::default();
        skills.technical.dribbling = dribbling;
        skills.technical.technique = technique;
        skills.mental.flair = 12.0;
        skills.mental.composure = 12.0;
        skills.mental.decisions = 12.0;
        skills.physical.agility = agility;
        skills.physical.acceleration = 14.0;
        skills.physical.balance = 12.0;
        skills.physical.strength = 11.0;
        skills.physical.stamina = 14.0;
        skills.physical.natural_fitness = 14.0;
        let p = PlayerBuilder::new()
            .id(2)
            .full_name(FullName::new("A".into(), "Z".into()))
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
        MatchPlayer::from_player(2, &p, PlayerPositionType::ForwardCenter, false, None)
    }

    #[test]
    fn strong_tackler_dominates_weak_dribbler() {
        let strong = defender(18.0, 17.0, 16.0);
        let weak_attacker = attacker(7.0, 7.0, 9.0);
        let diff = sc::defensive_duel(&strong, 30) - sc::dribble_attack(&weak_attacker, 30);
        assert!(
            diff > 0.20,
            "expected strong defender advantage, got diff={diff}"
        );
    }

    #[test]
    fn weak_tackler_loses_to_strong_dribbler() {
        let weak = defender(7.0, 7.0, 8.0);
        let elite_attacker = attacker(18.0, 17.0, 17.0);
        let diff = sc::defensive_duel(&weak, 30) - sc::dribble_attack(&elite_attacker, 30);
        assert!(diff < -0.10, "expected attacker advantage, got diff={diff}");
    }

    /// Every attribute at `level`, so the two sides of the duel are
    /// matched by construction.
    fn uniform(id: u32, level: f32, position: PlayerPositionType) -> MatchPlayer {
        let attrs = PlayerAttributes {
            condition: 10000,
            jadedness: 0,
            ..Default::default()
        };
        let mut skills = PlayerSkills::default();
        let s = &mut skills;
        s.technical.tackling = level;
        s.technical.marking = level;
        s.technical.heading = level;
        s.technical.passing = level;
        s.technical.technique = level;
        s.technical.first_touch = level;
        s.technical.crossing = level;
        s.technical.dribbling = level;
        s.mental.positioning = level;
        s.mental.anticipation = level;
        s.mental.concentration = level;
        s.mental.decisions = level;
        s.mental.composure = level;
        s.mental.bravery = level;
        s.mental.aggression = level;
        s.mental.teamwork = level;
        s.mental.work_rate = level;
        s.mental.leadership = level;
        s.mental.vision = level;
        s.mental.off_the_ball = level;
        s.mental.determination = level;
        s.mental.flair = level;
        s.physical.strength = level;
        s.physical.jumping = level;
        s.physical.pace = level;
        s.physical.acceleration = level;
        s.physical.agility = level;
        s.physical.balance = level;
        s.physical.stamina = level;
        s.physical.natural_fitness = level;
        s.physical.match_readiness = level;
        let p = PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("U".into(), "Z".into()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(id, &p, position, false, None)
    }

    /// **The production duel is level for equal quality, at every level
    /// of the pyramid.**
    ///
    /// Scoring `tackle_profile` against `dribble_attack` could not have
    /// it: the two were on different scales, so the same matched pair
    /// resolved at 0.55 in the fourth tier and 0.24 in the top flight.
    /// Both composites here are one family, so the difference is zero
    /// wherever the pair is drawn from.
    #[test]
    fn an_equal_duel_resolves_level_at_every_level() {
        let win = |level: f32| {
            let d = uniform(1, level, PlayerPositionType::DefenderCenter);
            let a = uniform(2, level, PlayerPositionType::ForwardCenter);
            let diff = sc::defensive_duel(&d, 30) - sc::dribble_attack(&a, 30);
            (1.0f32 / (1.0 + (-diff * 2.4).exp())).clamp(0.06, 0.55)
        };
        let mut lowest = f32::MAX;
        let mut highest = f32::MIN;
        for level in [4.0f32, 8.0, 12.0, 16.0, 18.0, 20.0] {
            let p = win(level);
            assert!(
                (p - 0.5).abs() <= 0.05,
                "equal quality at level {level} resolved at {p}"
            );
            lowest = lowest.min(p);
            highest = highest.max(p);
        }
        assert!(
            highest - lowest <= 0.05,
            "the duel walks up the pyramid: {lowest} to {highest}"
        );
    }

    /// …and skill advantage is still monotone through it.
    #[test]
    fn the_better_defender_still_wins_more_of_them() {
        let elite = uniform(1, 18.0, PlayerPositionType::DefenderCenter);
        let poor = uniform(3, 6.0, PlayerPositionType::DefenderCenter);
        let carrier = uniform(2, 12.0, PlayerPositionType::ForwardCenter);
        let carry = sc::dribble_attack(&carrier, 30);
        assert!(sc::defensive_duel(&elite, 30) - carry > sc::defensive_duel(&poor, 30) - carry);
    }

    #[test]
    fn marking_and_positioning_help_defender_in_duel() {
        let positional = defender(12.0, 18.0, 18.0);
        let raw_tackler = defender(15.0, 8.0, 8.0);
        // The positional defender should match or exceed a stronger
        // pure tackler thanks to marking + positioning weight (0.13 +
        // 0.17). This validates the spec's expectation that the duel
        // composite isn't dominated by raw tackling alone.
        assert!(sc::defensive_duel(&positional, 30) >= sc::defensive_duel(&raw_tackler, 30));
    }
}
