//! Monthly squad-status assignment.
//!
//! Each player gets a `PlayerSquadStatus` (KeyPlayer / FirstTeamRegular /
//! RotationPlayer / Backup / NotNeeded / etc.) derived from their CA rank
//! *within their own position group*. Ranking against the whole squad puts
//! a backup goalkeeper at the bottom of a CA-sorted list dominated by
//! outfield stars — you'd get `NotNeeded` for every 3rd/4th keeper at an
//! elite club, and every downstream code path keyed on squad status would
//! treat them as surplus.
//!
//! A genuine move on the senior ladder is no longer silent: squad status
//! drives the player's expected start share, so a promotion or demotion
//! is one of the most consequential things a coach can tell a player.
//! The updater emits a `SquadStatusChange` morale event, shaped by the
//! direction of the move, the player's recent form (demoted while
//! performing reads as an injustice), personality, and whether the head
//! coach has the man-management to deliver the news in a conversation
//! rather than via the team-sheet.

use crate::club::team::Team;
use crate::utils::DateUtils;
use crate::{HappinessEventType, PlayerFieldPositionGroup, PlayerSquadStatus};
use chrono::NaiveDate;
use std::collections::HashMap;

pub struct SquadStatusUpdater;

impl SquadStatusUpdater {
    /// Head-coach man-management at or above this delivers role changes
    /// in a conversation, halving the sting of a demotion.
    const MAN_MANAGEMENT_TO_EXPLAIN: u8 = 12;
    /// Morale points per senior-ladder step.
    const MAGNITUDE_PER_STEP: f32 = 2.2;
    /// Days before another status-change event can land on the same
    /// player — CA-rank boundaries can oscillate month to month and the
    /// ledger shouldn't churn with them.
    const EVENT_COOLDOWN_DAYS: u16 = 90;

    /// Recompute every player's `contract.squad_status` against the CA
    /// distribution of their position group, emitting a morale event
    /// for genuine senior-ladder moves.
    pub fn apply(team: &mut Team, date: NaiveDate) {
        let mut by_group: HashMap<PlayerFieldPositionGroup, Vec<u8>> = HashMap::new();
        for p in team.players.iter() {
            let g = p.position().position_group();
            by_group
                .entry(g)
                .or_default()
                .push(p.player_attributes.current_ability);
        }
        for cas in by_group.values_mut() {
            cas.sort_unstable_by(|a, b| b.cmp(a));
        }

        let explains_role_changes = team
            .staffs
            .social_head_coach()
            .map(|s| s.staff_attributes.mental.man_management >= Self::MAN_MANAGEMENT_TO_EXPLAIN)
            .unwrap_or(false);

        for player in team.players.iter_mut() {
            let group = player.position().position_group();
            let ca = player.player_attributes.current_ability;
            let age = DateUtils::age(player.birth_date, date);

            let mut transition: Option<(u8, u8)> = None;
            if let Some(ref mut contract) = player.contract {
                let group_cas = by_group.get(&group).map(|v| v.as_slice()).unwrap_or(&[]);
                let old_rank = Self::senior_rank(&contract.squad_status);
                contract.squad_status = PlayerSquadStatus::calculate(ca, age, group_cas);
                let new_rank = Self::senior_rank(&contract.squad_status);
                if let (Some(old), Some(new)) = (old_rank, new_rank) {
                    if old != new {
                        transition = Some((old, new));
                    }
                }
            }

            if let Some((old, new)) = transition {
                let steps = new as f32 - old as f32;
                let mut magnitude = Self::MAGNITUDE_PER_STEP * steps;
                if steps < 0.0 {
                    // Demoted while performing reads as an injustice;
                    // ambition amplifies the wound, professionalism
                    // absorbs some of it, and a coach who actually
                    // explains the decision takes most of the edge off.
                    let pos_group = player.position().position_group();
                    let form = player.statistics.average_rating_realistic(pos_group);
                    let apps = player.statistics.played + player.statistics.played_subs;
                    if apps >= 3 && form >= 7.0 {
                        magnitude *= 1.5;
                    }
                    let ambition = (player.attributes.ambition / 20.0).clamp(0.0, 1.0);
                    let professionalism =
                        (player.attributes.professionalism / 20.0).clamp(0.0, 1.0);
                    magnitude *= 1.0 + ambition * 0.3 - professionalism * 0.25;
                    if explains_role_changes {
                        magnitude *= 0.6;
                    }
                } else {
                    // Promotions land softer than demotions sting.
                    magnitude *= 0.7;
                }
                player.happiness.add_event_with_cooldown(
                    HappinessEventType::SquadStatusChange,
                    magnitude.clamp(-6.0, 5.0),
                    Self::EVENT_COOLDOWN_DAYS,
                );
            }
        }
    }

    /// Senior-ladder rank for promotion/demotion detection. Youth
    /// labels return `None` — a prospect's label shifting with age is
    /// re-classification, not a conversation about his role.
    fn senior_rank(status: &PlayerSquadStatus) -> Option<u8> {
        match status {
            PlayerSquadStatus::KeyPlayer => Some(5),
            PlayerSquadStatus::FirstTeamRegular => Some(4),
            PlayerSquadStatus::FirstTeamSquadRotation => Some(3),
            PlayerSquadStatus::MainBackupPlayer => Some(2),
            PlayerSquadStatus::NotNeeded => Some(0),
            _ => None,
        }
    }
}
