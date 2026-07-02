use crate::PlayerFieldPositionGroup;
use crate::club::player::skills::GoalkeeperSpeedContext;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::events::EventCollection;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer, MovementEffort};

use nalgebra::Vector3;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Result;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerState {
    Injured,
    Goalkeeper(GoalkeeperState),
    Defender(DefenderState),
    Midfielder(MidfielderState),
    Forward(ForwardState),
}

impl PlayerState {
    /// Cheap integer ID for fast dedup — avoids `to_string()` allocation.
    /// Each (outer variant, inner variant) pair maps to a unique u16.
    ///
    /// The id is embedded in replay / position records, so it must stay
    /// stable across releases. The inner `*s as u16` reads the inner
    /// enum's **explicit** discriminant (every state enum pins them), and
    /// `compact_id_snapshot` in this file's tests fails loudly if any
    /// value drifts — so it never silently depends on variant order.
    #[inline]
    pub fn compact_id(&self) -> u16 {
        match self {
            PlayerState::Injured => 0,
            PlayerState::Goalkeeper(s) => 100 + (*s as u16),
            PlayerState::Defender(s) => 200 + (*s as u16),
            PlayerState::Midfielder(s) => 300 + (*s as u16),
            PlayerState::Forward(s) => 400 + (*s as u16),
        }
    }

    /// Every state in the engine's universe, role-then-declaration order.
    /// The single source of truth for the transition-graph audit and the
    /// compact-id stability snapshot. Built from each role's `ALL`
    /// registry, so adding a state in one place flows through here.
    pub fn all() -> Vec<PlayerState> {
        let mut states = Vec::with_capacity(1 + 21 + 20 + 19 + 19);
        states.push(PlayerState::Injured);
        states.extend(GoalkeeperState::ALL.map(PlayerState::Goalkeeper));
        states.extend(DefenderState::ALL.map(PlayerState::Defender));
        states.extend(MidfielderState::ALL.map(PlayerState::Midfielder));
        states.extend(ForwardState::ALL.map(PlayerState::Forward));
        states
    }

    /// States a player may occupy with no inbound transition: the per-role
    /// kickoff defaults the match *starts* in (via `set_default_state`)
    /// before any transition fires. The transition-graph audit exempts
    /// these from the "every state has an inbound edge" rule.
    pub fn entry_states() -> [PlayerState; 4] {
        [
            PlayerState::Goalkeeper(GoalkeeperState::Standing),
            PlayerState::Defender(DefenderState::Standing),
            PlayerState::Midfielder(MidfielderState::Standing),
            PlayerState::Forward(ForwardState::Standing),
        ]
    }

    /// States reserved / not yet wired into the in-match state machine.
    ///
    /// `Injured` has a handler (`CommonInjuredState`) and a `compact_id`,
    /// but **no code path transitions a player into it during a match** —
    /// in-match injuries are intentionally future work (fitness/injury
    /// modelling lives in the development pipeline, not the engine). It is
    /// kept as a deliberate placeholder. The audit treats reserved states
    /// as both entry (no inbound required) and terminal (no outbound
    /// required) so the unwired state is not flagged.
    pub fn reserved_states() -> [PlayerState; 1] {
        [PlayerState::Injured]
    }
}

impl Display for PlayerState {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self {
            PlayerState::Injured => write!(f, "Injured"),
            PlayerState::Goalkeeper(state) => write!(f, "Goalkeeper: {}", state),
            PlayerState::Defender(state) => write!(f, "Defender: {}", state),
            PlayerState::Midfielder(state) => write!(f, "Midfielder: {}", state),
            PlayerState::Forward(state) => write!(f, "Forward: {}", state),
        }
    }
}

pub struct PlayerMatchState;

impl PlayerMatchState {
    pub fn process(
        player: &mut MatchPlayer,
        context: &MatchContext,
        tick_context: &GameTickContext,
    ) -> EventCollection {
        // Decay memory every 100 ticks
        let current_tick = context.current_tick();
        if current_tick > 0 && current_tick % 100 == 0 {
            player.memory.decay(current_tick);
        }

        let player_position_group = player.tactical_position.current_position.position_group();

        let state_change_result =
            player_position_group.process(player.in_state_time, player, context, tick_context);

        if state_change_result.start_tackle_cooldown {
            player.start_tackle_cooldown();
        }

        // Stash the shot reason on the player. The Shooting state will
        // consume and clear this when it composes the Shoot event.
        if let Some(reason) = state_change_result.shot_reason {
            player.pending_shot_reason = Some(reason);
        }

        if let Some(state) = state_change_result.state {
            Self::change_state(player, state);
        } else {
            player.in_state_time += 1;
        }

        if let Some(velocity) = state_change_result.velocity {
            let mut max_speed = if player_position_group == PlayerFieldPositionGroup::Goalkeeper {
                let speed_context = match player.state {
                    PlayerState::Goalkeeper(GoalkeeperState::Diving)
                    | PlayerState::Goalkeeper(GoalkeeperState::PreparingForSave)
                    | PlayerState::Goalkeeper(GoalkeeperState::Jumping) => {
                        GoalkeeperSpeedContext::Explosive
                    }
                    PlayerState::Goalkeeper(GoalkeeperState::Catching)
                    | PlayerState::Goalkeeper(GoalkeeperState::ComingOut) => {
                        GoalkeeperSpeedContext::Active
                    }
                    PlayerState::Goalkeeper(GoalkeeperState::Standing)
                    | PlayerState::Goalkeeper(GoalkeeperState::ReturningToGoal) => {
                        GoalkeeperSpeedContext::Positioning
                    }
                    _ => GoalkeeperSpeedContext::Casual,
                };
                player
                    .skills
                    .goalkeeper_max_speed(player.player_attributes.condition, speed_context)
            } else {
                player.max_speed_with_condition_cached()
            };

            // Ball-carrier speed multiplier. Real football: carrying
            // the ball costs ~15-25% of top sprint for an average
            // player — they keep the ball in stride, look up, protect
            // it. Elite carriers (Mbappé/Messi) lose almost nothing.
            //
            // Routes through `movement_speed_with_ball` so dribbling +
            // technique + pace + acceleration + agility + balance all
            // contribute, and so fatigue/late-game effects propagate
            // through `effective_skill`. Mapping per spec:
            //
            //   carry_mult = 0.78 + composite * 0.42
            //
            // Composite floor 0.05 → 0.80 (worst carrier under fatigue);
            // composite 1.00 → 1.20 (elite carrier — no realistic
            // penalty). Capped to existing `[0.75, 1.00]` band so the
            // model stays a CARRY COST: an elite carrier matches their
            // off-ball speed but doesn't go faster than it.
            if tick_context.ball.current_owner == Some(player.id)
                && player_position_group != PlayerFieldPositionGroup::Goalkeeper
            {
                let minute = sc::minute_from_ms(context.total_match_time);
                let composite = sc::movement_speed_with_ball(player, minute);
                let raw = 0.78 + composite * 0.42;
                max_speed *= raw.clamp(0.75, 1.00);
            }

            // Off-ball effort: a player jogs to reposition and only
            // sprints to press / chase / break in behind. Scale the speed
            // cap by the effort the state itself declared this tick (the
            // same `ActivityIntensity` the fatigue model reads), so
            // off-ball movement stops pinning to a full sprint — the
            // condition diagnostic had ~77% of outfield ticks in the top
            // band. This caps the top speed only: a velocity already below
            // the ceiling (a decelerating `Arrive`, a slow walk vector) is
            // untouched. On-ball carriers keep the carry model applied
            // above; goalkeepers keep their context-managed speed.
            if player_position_group != PlayerFieldPositionGroup::Goalkeeper
                && tick_context.ball.current_owner != Some(player.id)
            {
                max_speed *= MovementEffort::speed_fraction(
                    player.last_activity_intensity,
                    player.player_attributes.condition_percentage(),
                );
            }

            // NaN/Inf guard: state velocity functions compose many
            // divisions and normalizations, and any zero-magnitude vector
            // put through `.normalize()` anywhere upstream produces a
            // NaN that propagates into player.velocity → player.position
            // → the recording → the viewer renders nothing. Catch it
            // here at the single integration point so no state has to
            // remember to self-sanitize. Non-finite → zero this tick.
            let finite = velocity.x.is_finite() && velocity.y.is_finite() && velocity.z.is_finite();
            let velocity = if finite { velocity } else { Vector3::zeros() };

            let velocity_sq = velocity.norm_squared();
            let max_speed_sq = max_speed * max_speed;

            if velocity_sq > max_speed_sq && velocity_sq > 0.0 {
                let velocity_magnitude = velocity_sq.sqrt();
                player.velocity = velocity * (max_speed / velocity_magnitude);
            } else {
                player.velocity = velocity;
            }
        }

        state_change_result.events
    }

    fn change_state(player: &mut MatchPlayer, state: PlayerState) {
        // Normal state-machine hand-off: a state handler returned a new
        // state via `StateChangeResult`. Routed through the single
        // transition API so the timer reset and graph audit are uniform.
        player.transition_to(state, TransitionSource::Handler);
    }
}

#[cfg(test)]
mod state_id_tests {
    use super::PlayerState;
    use crate::r#match::defenders::states::DefenderState;
    use crate::r#match::forwarders::states::ForwardState;
    use crate::r#match::goalkeepers::states::state::GoalkeeperState;
    use crate::r#match::midfielders::states::MidfielderState;

    #[test]
    fn role_discriminants_match_declaration_order() {
        // Each role enum's discriminant must equal its index in `ALL`.
        // This is what makes `compact_id` independent of variant *position*:
        // reorder a variant and its discriminant no longer matches its
        // slot, failing here before any replay could be misnumbered.
        for (i, s) in GoalkeeperState::ALL.iter().enumerate() {
            assert_eq!(*s as u16, i as u16, "GoalkeeperState::ALL[{i}]");
        }
        for (i, s) in DefenderState::ALL.iter().enumerate() {
            assert_eq!(*s as u16, i as u16, "DefenderState::ALL[{i}]");
        }
        for (i, s) in MidfielderState::ALL.iter().enumerate() {
            assert_eq!(*s as u16, i as u16, "MidfielderState::ALL[{i}]");
        }
        for (i, s) in ForwardState::ALL.iter().enumerate() {
            assert_eq!(*s as u16, i as u16, "ForwardState::ALL[{i}]");
        }
    }

    #[test]
    fn compact_id_snapshot() {
        // Pin the entire id space. If a state is added, removed, reordered
        // or renumbered, this fails — the signal to bump the replay format
        // intentionally rather than by accident.
        let all = PlayerState::all();
        assert_eq!(all.len(), 1 + 21 + 20 + 19 + 19, "state count changed");
        assert_eq!(GoalkeeperState::ALL.len(), 21);
        assert_eq!(DefenderState::ALL.len(), 20);
        assert_eq!(MidfielderState::ALL.len(), 19);
        assert_eq!(ForwardState::ALL.len(), 19);

        let mut ids: Vec<u16> = all.iter().map(|s| s.compact_id()).collect();
        let unique: std::collections::BTreeSet<u16> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len(), "compact_ids must be unique");

        ids.sort_unstable();
        let mut expected: Vec<u16> = vec![0]; // Injured
        expected.extend(100..=120u16); // 21 GK
        expected.extend(200..=219u16); // 20 DEF
        expected.extend(300..=318u16); // 19 MID
        expected.extend(400..=418u16); // 19 FWD
        assert_eq!(ids, expected, "compact_id space drifted");

        // Anchor a few named states so an intra-band reorder is caught
        // even though the band-set check above would not notice it.
        assert_eq!(PlayerState::Injured.compact_id(), 0);
        assert_eq!(
            PlayerState::Goalkeeper(GoalkeeperState::Standing).compact_id(),
            100
        );
        assert_eq!(
            PlayerState::Defender(DefenderState::AttackingCorner).compact_id(),
            219
        );
        assert_eq!(
            PlayerState::Midfielder(MidfielderState::Guarding).compact_id(),
            318
        );
        assert_eq!(
            PlayerState::Forward(ForwardState::CrossReceiving).compact_id(),
            411
        );
    }

    #[test]
    fn injured_is_reserved_and_not_an_entry_state() {
        // Documents the dead-state decision: Injured is reserved (no
        // inbound transition in the match engine), not an entry state.
        assert!(PlayerState::reserved_states().contains(&PlayerState::Injured));
        assert!(!PlayerState::entry_states().contains(&PlayerState::Injured));
        assert_eq!(PlayerState::Injured.compact_id(), 0);
    }
}
