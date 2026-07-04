//! Weekly chronic-conflict pass. Reads each player's
//! [`crate::CoachPlayerBond::conflict_risk`], applies captain mediation,
//! and tracks how many consecutive weekly ticks the player has stayed
//! in the elevated band. Persistent elevation escalates: private
//! complaint → Unhappy → transfer request / public criticism leak.
//!
//! Why a separate pass: `manager_talks` fires preventive *conversations*
//! when conflict_risk crosses the soft warning band — that's a single
//! coach intervention. This pass owns the *consequences* when the
//! intervention isn't enough. A coach who keeps having morale chats but
//! never moves the underlying bond eventually loses the player; this is
//! where that ladder lives.
//!
//! Architecture: the orchestration step lives here; the actual
//! mutations on each player go through `Player::on_*` methods so
//! status / relations / events stay owned by the Player module per
//! [`crate::club`] conventions.

use super::TeamBehaviour;
use crate::club::staff::CoachPlayerBond;
use crate::club::team::CaptainMediation;
use crate::club::team::behaviour::TeamBehaviourResult;
use crate::context::GlobalContext;
use crate::{Player, PlayerCollection, PlayerStatusType, StaffCollection};
use chrono::NaiveDate;

/// Boundary thresholds for the conflict-escalation ladder. Centralised
/// constants so the tests, the orchestrator, and the per-player handler
/// agree on what "elevated" / "critical" mean.
pub(crate) struct ConflictEscalationThresholds;

impl ConflictEscalationThresholds {
    /// Below this risk, the bond is calm enough that no random conflict
    /// event should fire — the suppression band.
    pub const RANDOM_CONFLICT_SUPPRESS: f32 = 0.30;
    /// Above this, the player is a candidate for a private morale talk
    /// (handled by the talks pass; we just check, we don't fire it).
    pub const PRIVATE_TALK_CANDIDATE: f32 = 0.65;
    /// Above this, the player is in the "elevated" band — consecutive
    /// ticks here build the streak counter.
    pub const ELEVATED_RISK: f32 = 0.80;
    /// Above this, the player is in the "critical" band — transfer-
    /// request roll becomes available once the streak has held.
    pub const CRITICAL_RISK: f32 = 0.90;
    /// Public-criticism floor — distinct from the streak ladder. Fires
    /// from a single tick if professionalism is low enough.
    pub const PUBLIC_CRITICISM_FLOOR: f32 = 0.85;
    /// How many consecutive weekly ticks in the elevated band before
    /// the Unhappy / transfer-request rolls can fire. Two weeks
    /// matches the spec — a single bad week never escalates.
    pub const STREAK_TO_ESCALATE: u8 = 2;
    /// Ambition floor for the transfer-request roll. Below this the
    /// player is more likely to grumble than walk.
    pub const TRANSFER_REQUEST_AMBITION_MIN: f32 = 13.0;
    /// Controversy floor that opens the transfer-request roll for the
    /// "personality-driven" path even if ambition is moderate.
    pub const TRANSFER_REQUEST_CONTROVERSY_MIN: f32 = 12.0;
    /// Professionalism ceiling for the public-criticism / media-leak
    /// roll. A pro doesn't go to the press; a low-professionalism
    /// player will.
    pub const PUBLIC_CRITICISM_PROFESSIONALISM_MAX: f32 = 8.0;
}

impl TeamBehaviour {
    /// Weekly pass. For every active squad member, read their bond with
    /// the head coach, apply captain mediation, update the streak
    /// counter, and roll the escalation events when conditions hold.
    pub(super) fn process_conflict_escalation(
        players: &mut PlayerCollection,
        staffs: &StaffCollection,
        _result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        let Some(manager) = staffs.social_head_coach() else {
            return;
        };
        let today = ctx.simulation.date.date();

        // Snapshot captain mediation inputs once — the captain doesn't
        // change inside the loop and we need to apply it to every
        // candidate's effective risk.
        let mediation = CaptainMediation::for_squad(players);
        let coach_id = manager.id;

        let _ = coach_id;

        // Collect (player_id, effective_risk) so we can mutate each
        // player without an outstanding borrow on `players`. The bond
        // read is read-only so it's safe to interleave with the borrow.
        let candidates: Vec<(u32, f32)> = players
            .iter()
            .filter(|p| !p.is_on_loan())
            .map(|p| {
                let bond = CoachPlayerBond::build(p, manager, today);
                let raw = bond.conflict_risk;
                let effective = mediation.effective_risk(raw, p);
                (p.id, effective)
            })
            .collect();

        for (player_id, effective_risk) in candidates {
            if let Some(player) = players.iter_mut().find(|p| p.id == player_id) {
                player.on_weekly_conflict_risk(effective_risk);
                player.roll_conflict_escalation(effective_risk, today, rand::random::<f32>);
            }
        }
    }
}

/// Per-player handler invoked from the orchestrator. Lives on Player
/// so the status / event / streak mutations are owned by the same
/// module that owns the underlying state.
impl Player {
    /// Weekly tick from the conflict-escalation pass. Pure
    /// bookkeeping: bumps the streak counter when the bond stays in
    /// the elevated band, resets it otherwise. The probabilistic
    /// escalation rolls live in [`Player::roll_conflict_escalation`]
    /// so tests can exercise the streak update independently of the
    /// random source.
    ///
    /// `effective_risk` is the captain-mediated value, not the raw
    /// `CoachPlayerBond::conflict_risk` read.
    pub fn on_weekly_conflict_risk(&mut self, effective_risk: f32) {
        if effective_risk > ConflictEscalationThresholds::ELEVATED_RISK {
            self.happiness.conflict_risk_streak =
                self.happiness.conflict_risk_streak.saturating_add(1);
        } else {
            self.happiness.conflict_risk_streak = 0;
        }
    }

    /// Probabilistic escalation rolls — private complaint, Unhappy,
    /// transfer request, public criticism. Takes a `dice` closure so
    /// production callers pass `rand::random` and tests can pin every
    /// roll deterministically. Streak is read but never mutated here
    /// unless an escalation actually fires (in which case it resets
    /// so we don't re-escalate next week on the same evidence).
    pub fn roll_conflict_escalation(
        &mut self,
        effective_risk: f32,
        today: NaiveDate,
        mut dice: impl FnMut() -> f32,
    ) {
        // Suppress all rolls when the bond is calm.
        if effective_risk < ConflictEscalationThresholds::RANDOM_CONFLICT_SUPPRESS {
            return;
        }

        let statuses = self.statuses.get();
        let already_unhappy = statuses.contains(&PlayerStatusType::Unh);
        let already_requested = statuses.contains(&PlayerStatusType::Req);

        // ── Private complaint roll — single-tick, lightest escalation.
        if effective_risk > ConflictEscalationThresholds::PRIVATE_TALK_CANDIDATE
            && !already_unhappy
            && !already_requested
            && dice() < 0.20 * effective_risk
        {
            self.happiness.adjust_morale(-3.0);
        }

        let streak_held =
            self.happiness.conflict_risk_streak >= ConflictEscalationThresholds::STREAK_TO_ESCALATE;

        // ── Unhappy roll — requires persistent elevation.
        if streak_held
            && effective_risk > ConflictEscalationThresholds::ELEVATED_RISK
            && !already_unhappy
            && !already_requested
            && dice() < 0.12 * effective_risk
        {
            self.statuses.add(today, PlayerStatusType::Unh);
            self.happiness.adjust_morale(-6.0);
            self.happiness.conflict_risk_streak = 0;
            return;
        }

        // ── Transfer-request roll — requires streak + ambition or
        //    controversy and a personality-driven dice.
        if streak_held && effective_risk > ConflictEscalationThresholds::CRITICAL_RISK {
            let ambition = self.attributes.ambition;
            let controversy = self.attributes.controversy;
            let qualifies = ambition >= ConflictEscalationThresholds::TRANSFER_REQUEST_AMBITION_MIN
                || controversy >= ConflictEscalationThresholds::TRANSFER_REQUEST_CONTROVERSY_MIN;
            if qualifies && !already_requested {
                let personality_pressure = Self::personality_pressure_for(self);
                if dice() < 0.06 * effective_risk * personality_pressure {
                    if !already_unhappy {
                        self.statuses.add(today, PlayerStatusType::Unh);
                    }
                    self.statuses.add(today, PlayerStatusType::Req);
                    self.happiness.adjust_morale(-8.0);
                    self.happiness.conflict_risk_streak = 0;
                    return;
                }
            }
        }

        // ── Public criticism / media leak roll — single-tick gate,
        //    unprofessional player needed.
        if effective_risk > ConflictEscalationThresholds::PUBLIC_CRITICISM_FLOOR
            && self.attributes.professionalism
                <= ConflictEscalationThresholds::PUBLIC_CRITICISM_PROFESSIONALISM_MAX
        {
            let controversy_factor = (self.attributes.controversy / 20.0).clamp(0.0, 1.0);
            if dice() < 0.05 * effective_risk * controversy_factor {
                self.happiness.adjust_morale(-4.0);
                if !self.statuses.get().contains(&PlayerStatusType::PR) {
                    self.statuses.add(today, PlayerStatusType::PR);
                }
            }
        }
    }

    /// Spec composite of "how much will personality push this player
    /// toward a transfer request when their bond is broken".
    fn personality_pressure_for(player: &Player) -> f32 {
        let ambition = player.attributes.ambition / 20.0;
        let controversy = player.attributes.controversy / 20.0;
        let temperament_inverse = (20.0 - player.attributes.temperament) / 20.0;
        let loyalty_inverse = (20.0 - player.attributes.loyalty) / 20.0;
        (0.40 * ambition + 0.30 * controversy + 0.20 * temperament_inverse + 0.10 * loyalty_inverse)
            .clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };
    use chrono::NaiveDate;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
    }

    fn pro() -> PersonAttributes {
        PersonAttributes {
            adaptability: 12.0,
            ambition: 12.0,
            controversy: 5.0,
            loyalty: 14.0,
            pressure: 12.0,
            professionalism: 14.0,
            sportsmanship: 12.0,
            temperament: 12.0,
            consistency: 12.0,
            important_matches: 12.0,
            dirtiness: 5.0,
        }
    }

    fn build_player(id: u32, leadership: f32) -> Player {
        let mut skills = PlayerSkills::default();
        skills.mental.leadership = leadership;
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("CE".into(), id.to_string()))
            .birth_date(NaiveDate::from_ymd_opt(1998, 1, 1).unwrap())
            .country_id(1)
            .attributes(pro())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    #[test]
    fn streak_increments_only_in_elevated_band() {
        let mut p = build_player(1, 10.0);
        p.on_weekly_conflict_risk(0.5);
        assert_eq!(p.happiness.conflict_risk_streak, 0);
        p.on_weekly_conflict_risk(0.85);
        assert_eq!(p.happiness.conflict_risk_streak, 1);
        p.on_weekly_conflict_risk(0.85);
        assert_eq!(p.happiness.conflict_risk_streak, 2);
        // Drop below the band — streak resets.
        p.on_weekly_conflict_risk(0.50);
        assert_eq!(p.happiness.conflict_risk_streak, 0);
    }

    #[test]
    fn roll_conflict_escalation_high_risk_with_pinned_dice_fires_unhappy() {
        // With dice pinned to 0.0 and a held streak, the Unhappy roll
        // fires deterministically and the streak resets.
        let mut p = build_player(1, 10.0);
        p.on_weekly_conflict_risk(0.85);
        p.on_weekly_conflict_risk(0.85);
        assert_eq!(p.happiness.conflict_risk_streak, 2);
        p.roll_conflict_escalation(0.85, today(), || 0.0);
        assert!(p.statuses.get().contains(&PlayerStatusType::Unh));
        assert_eq!(p.happiness.conflict_risk_streak, 0);
    }

    #[test]
    fn roll_conflict_escalation_suppressed_below_threshold() {
        // Below the suppression band the rolls don't fire even with
        // dice pinned to 0.0.
        let mut p = build_player(1, 10.0);
        p.roll_conflict_escalation(0.20, today(), || 0.0);
        assert!(!p.statuses.get().contains(&PlayerStatusType::Unh));
        assert!(!p.statuses.get().contains(&PlayerStatusType::Req));
    }

    #[test]
    fn personality_pressure_uses_spec_weights() {
        let mut p = build_player(1, 10.0);
        p.attributes.ambition = 20.0;
        p.attributes.controversy = 20.0;
        p.attributes.temperament = 0.0;
        p.attributes.loyalty = 0.0;
        // Maxed: 0.4 + 0.3 + 0.2 + 0.1 = 1.0
        assert!((Player::personality_pressure_for(&p) - 1.0).abs() < 1e-3);

        p.attributes.ambition = 0.0;
        p.attributes.controversy = 0.0;
        p.attributes.temperament = 20.0;
        p.attributes.loyalty = 20.0;
        // Inverted — only temperament_inverse / loyalty_inverse fire,
        // both zero. Expect 0.
        assert!(Player::personality_pressure_for(&p) < 1e-3);
    }

    #[test]
    fn captain_mediation_reduces_high_conflict_risk() {
        // Improvement task #7: a strong captain reduces effective
        // conflict risk for a player who otherwise has no relation
        // with them.
        let captain = build_player(1, 19.0); // peak leadership
        let player = build_player(2, 10.0);
        let no_captain = build_player(2, 10.0);
        let players_with = PlayerCollection::new(vec![captain, player]);
        let players_without = PlayerCollection::new(vec![no_captain]);

        let med_with = CaptainMediation::for_captain(&players_with, Some(1));
        let med_without = CaptainMediation::for_captain(&players_without, None);

        let raw_risk = 0.90;
        let p = players_with.find(2).unwrap();
        let with_risk = med_with.effective_risk(raw_risk, p);
        let p2 = players_without.find(2).unwrap();
        let without_risk = med_without.effective_risk(raw_risk, p2);

        assert!(
            with_risk < without_risk,
            "captain mediation must lower risk ({} → {})",
            without_risk,
            with_risk
        );
    }

    #[test]
    fn disliked_captain_fails_to_mediate() {
        // Improvement task #7: when the player has a strongly
        // negative relation with the captain, mediation backfires
        // and the effective risk *rises* by 0.10. The result must
        // exceed the same risk read for a player who simply has no
        // captain relation.
        use crate::ChangeType;
        let captain = build_player(1, 19.0);
        let mut player = build_player(2, 10.0);
        // Drive the captain relation deeply hostile.
        // ChangeType::PersonalConflict subtracts magnitude*3 from level,
        // so a few sizable updates push level below the -25 threshold.
        for _ in 0..10 {
            player
                .relations
                .update_with_type(1, 20.0, ChangeType::PersonalConflict, today());
        }
        // Sanity-check the precondition before evaluating mediation.
        let rel_level = player
            .relations
            .get_player(1)
            .map(|r| r.level)
            .unwrap_or(0.0);
        assert!(
            rel_level <= -25.0,
            "test precondition: captain relation level must be ≤ -25 (got {})",
            rel_level
        );

        let neutral_player = build_player(3, 10.0);
        let players = PlayerCollection::new(vec![captain, player, neutral_player]);

        let med = CaptainMediation::for_captain(&players, Some(1));
        let raw = 0.80;
        let hostile = med.effective_risk(raw, players.find(2).unwrap());
        let neutral = med.effective_risk(raw, players.find(3).unwrap());

        assert!(
            hostile > neutral,
            "disliked captain must produce higher effective risk ({} vs {})",
            hostile,
            neutral
        );
    }
}
