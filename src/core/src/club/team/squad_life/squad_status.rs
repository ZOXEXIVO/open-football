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

use crate::club::player::happiness::PlayingTimeFrustrationConfig;
use crate::club::team::Team;
use crate::utils::DateUtils;
use crate::{HappinessEventType, Player, PlayerFieldPositionGroup, PlayerSquadStatus};
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

            // Honesty ceiling from actual involvement, computed before the
            // mutable contract borrow (it reads the whole player).
            let involvement_ceiling = Self::involvement_status_ceiling(player, date);

            let mut transition: Option<(u8, u8)> = None;
            if let Some(ref mut contract) = player.contract {
                let group_cas = by_group.get(&group).map(|v| v.as_slice()).unwrap_or(&[]);
                let old_rank = Self::senior_rank(&contract.squad_status);
                let mut new_status = PlayerSquadStatus::calculate(ca, age, group, group_cas);
                // Don't let a CA-strong label over-promise a role the player
                // isn't actually getting: a keeper stuck behind the number
                // one, or an out-of-favour senior, reads as a backup rather
                // than a first-team regular. Only demotes senior labels, and
                // only with enough match evidence.
                if let (Some(ca_rank), Some(ceiling)) =
                    (Self::senior_rank(&new_status), involvement_ceiling.as_ref())
                {
                    if Self::senior_rank(ceiling).unwrap_or(u8::MAX) < ca_rank {
                        new_status = ceiling.clone();
                    }
                }
                // Bidirectional: a player whose actual minutes still justify
                // his role isn't demoted just because a pricier signing now
                // outranks him on CA — a regular still starting every week is
                // not relabelled a backup (which also suppresses the unearned
                // demotion morale event). Never floats him above his prior
                // standing, so it only ever cancels an unjust demotion.
                if let Some(ceiling) = involvement_ceiling.as_ref() {
                    let ceil_rank = Self::senior_rank(ceiling).unwrap_or(0);
                    let floor_rank = ceil_rank.min(old_rank.unwrap_or(0));
                    if Self::senior_rank(&new_status).unwrap_or(0) < floor_rank {
                        new_status = if old_rank.unwrap_or(0) <= ceil_rank {
                            contract.squad_status.clone()
                        } else {
                            ceiling.clone()
                        };
                    }
                }
                // Honor an unexpired role promise as a floor: the club
                // committed to it at signing, so never recompute below it —
                // otherwise the promise the buyer paid for is wiped within a
                // month and the breach is hidden (expected_start_share reads
                // this same field). Past its expiry the promise stops binding.
                if let Some((promised, until)) = contract.promised_squad_status.clone() {
                    if date <= until {
                        if Self::senior_rank(&promised).unwrap_or(0)
                            > Self::senior_rank(&new_status).unwrap_or(0)
                        {
                            new_status = promised;
                        }
                    } else {
                        contract.promised_squad_status = None;
                    }
                }
                contract.squad_status = new_status;
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

    /// Highest senior status the player's ACTUAL match involvement justifies,
    /// or `None` when there isn't enough evidence to judge — too few eligible
    /// matches, or still settling in after a transfer. This is the honesty
    /// cap: a label may not claim a bigger role than the player is getting.
    ///
    /// Never demotes below `MainBackupPlayer` on appearances alone — a
    /// contracted senior who simply isn't picked is a backup, not surplus;
    /// genuine `NotNeeded` stays a CA-rank verdict (and is handled by the
    /// surplus-release systems, not here). The share thresholds sit a little
    /// under each status's `expected_start_share` so only a player clearly
    /// below his tier is demoted, giving hysteresis against month-to-month
    /// CA-rank wobble.
    fn involvement_status_ceiling(player: &Player, date: NaiveDate) -> Option<PlayerSquadStatus> {
        /// Enough eligible matches (~a third of a season) to trust the share.
        const MIN_ELIGIBLE_TO_JUDGE: u16 = 10;
        /// Give a player a fair chunk of a season at the club before his
        /// label can be demoted on appearances.
        const MIN_DAYS_AT_CLUB: i64 = 120;

        let opp = player.playing_time_opportunity(date);
        if opp.eligible_official_matches_since_join < MIN_ELIGIBLE_TO_JUDGE
            || opp.days_since_join < MIN_DAYS_AT_CLUB
        {
            return None;
        }

        let cfg = PlayingTimeFrustrationConfig::default();
        let eligible = opp.eligible_official_matches_since_join as f32;
        let share = (opp.actual_involvement_score(&cfg) / eligible).clamp(0.0, 1.0);

        let ceiling = if share >= 0.50 {
            PlayerSquadStatus::KeyPlayer
        } else if share >= 0.33 {
            PlayerSquadStatus::FirstTeamRegular
        } else if share >= 0.15 {
            PlayerSquadStatus::FirstTeamSquadRotation
        } else {
            PlayerSquadStatus::MainBackupPlayer
        };
        Some(ceiling)
    }
}
