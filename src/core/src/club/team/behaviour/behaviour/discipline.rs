//! Club disciplinary ladder — the formal response to misconduct that
//! previously ended at the mood event. A fresh incident
//! (`ControversyIncident`, `TrainingGroundBustUp`, `RedCardFallout`)
//! now draws a club reaction sized by the player's rap sheet: a first
//! offence gets a formal warning, repeat or compounding offenders get
//! fined a slice of wages (booked to club finance via the behaviour
//! result). The player's personality decides how it lands — a
//! professional takes it on the chin, a hot-head resents it.

use super::TeamBehaviour;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::team::behaviour::{PlayerFine, TeamBehaviourResult};
use crate::context::GlobalContext;
use crate::{HappinessEventType, Player, PlayerCollection, StaffCollection};

/// The rung of the ladder the club chose for one fresh incident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DisciplinaryRung {
    FormalWarning,
    /// Fine of this many annual-salary 1/26ths (i.e. weeks of wages).
    Fine { weeks: u32 },
}

/// Pure decision calculus for one player's fresh misconduct, separated
/// from the pass so the ladder can be unit-tested without a squad.
pub(super) struct DisciplinaryCall;

impl DisciplinaryCall {
    /// Days an incident stays "fresh" (ungated by an existing sanction).
    const FRESH_DAYS: u16 = 7;
    /// Window over which prior incidents count toward the rap sheet.
    const RAP_SHEET_DAYS: u16 = 180;

    fn incident_count(player: &Player, window: u16) -> usize {
        player
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                e.days_ago <= window
                    && matches!(
                        e.event_type,
                        HappinessEventType::ControversyIncident
                            | HappinessEventType::TrainingGroundBustUp
                            | HappinessEventType::RedCardFallout
                    )
            })
            .count()
    }

    /// Decide the club's response to this player's conduct, or `None`
    /// when there is nothing fresh to respond to (or it has already
    /// been answered).
    pub(super) fn decide(player: &Player) -> Option<DisciplinaryRung> {
        if Self::incident_count(player, Self::FRESH_DAYS) == 0 {
            return None;
        }
        // One sanction per incident window — don't pile a fine on top
        // of this week's warning.
        if player
            .happiness
            .has_recent_event(&HappinessEventType::FormalWarningIssued, Self::FRESH_DAYS)
            || player
                .happiness
                .has_recent_event(&HappinessEventType::FinedByClub, Self::FRESH_DAYS)
        {
            return None;
        }
        // The rap sheet (including this week's incident) sets the rung;
        // a player already in the poor-behaviour band skips the polite
        // stage.
        let priors = Self::incident_count(player, Self::RAP_SHEET_DAYS);
        if priors <= 1 && !player.behaviour.is_poor() {
            Some(DisciplinaryRung::FormalWarning)
        } else {
            let weeks = if priors >= 3 { 2 } else { 1 };
            Some(DisciplinaryRung::Fine { weeks })
        }
    }
}

impl TeamBehaviour {
    /// Weekly disciplinary pass. Requires a head coach in post — the
    /// caretaker keeps discipline too; an empty dugout punishes no one.
    pub(super) fn process_disciplinary_actions(
        players: &mut PlayerCollection,
        staffs: &StaffCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        if !ctx.simulation.is_week_beginning() {
            return;
        }
        if staffs.social_head_coach().is_none() {
            return;
        }

        let cfg = HappinessConfig::default();
        for player in players.players.iter_mut() {
            let Some(rung) = DisciplinaryCall::decide(player) else {
                continue;
            };
            // A professional takes the sanction on the chin; a
            // hot-headed player resents the club going formal.
            let professionalism01 =
                (player.attributes.professionalism / 20.0).clamp(0.0, 1.0);
            let temperament01 = (player.attributes.temperament / 20.0).clamp(0.0, 1.0);
            let reception = (1.0 - 0.4 * professionalism01) * (1.0 + 0.5 * (1.0 - temperament01));

            match rung {
                DisciplinaryRung::FormalWarning => {
                    player.happiness.add_event_with_cooldown(
                        HappinessEventType::FormalWarningIssued,
                        cfg.catalog.formal_warning_issued * reception,
                        7,
                    );
                }
                DisciplinaryRung::Fine { weeks } => {
                    let amount = player
                        .contract
                        .as_ref()
                        .map(|c| c.salary / 26 * weeks)
                        .unwrap_or(0);
                    player.happiness.add_event_with_cooldown(
                        HappinessEventType::FinedByClub,
                        cfg.catalog.fined_by_club * reception,
                        7,
                    );
                    if amount > 0 {
                        result.fines.push(PlayerFine {
                            player_id: player.id,
                            amount,
                        });
                    }
                    // The professional takes the hit and straightens
                    // out; the hot-head just pockets the resentment.
                    if player.attributes.professionalism >= 14.0 {
                        player.behaviour.try_increase();
                    }
                }
            }
        }
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

    fn offender() -> Player {
        PlayerBuilder::new()
            .id(9)
            .full_name(FullName::new("Hot".into(), "Head".into()))
            .birth_date(NaiveDate::from_ymd_opt(1998, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    #[test]
    fn clean_record_first_offence_gets_a_warning() {
        let mut p = offender();
        p.happiness
            .add_event(HappinessEventType::TrainingGroundBustUp, -3.0);
        assert_eq!(
            DisciplinaryCall::decide(&p),
            Some(DisciplinaryRung::FormalWarning)
        );
    }

    #[test]
    fn repeat_offender_is_fined() {
        let mut p = offender();
        p.happiness
            .add_event(HappinessEventType::ControversyIncident, -3.0);
        p.happiness
            .add_event(HappinessEventType::TrainingGroundBustUp, -3.0);
        assert_eq!(
            DisciplinaryCall::decide(&p),
            Some(DisciplinaryRung::Fine { weeks: 1 })
        );
    }

    #[test]
    fn no_incident_means_no_response() {
        let p = offender();
        assert_eq!(DisciplinaryCall::decide(&p), None);
    }

    #[test]
    fn one_sanction_per_incident_window() {
        let mut p = offender();
        p.happiness
            .add_event(HappinessEventType::TrainingGroundBustUp, -3.0);
        p.happiness
            .add_event(HappinessEventType::FormalWarningIssued, -1.5);
        assert_eq!(
            DisciplinaryCall::decide(&p),
            None,
            "this week's incident has already been answered"
        );
    }
}
