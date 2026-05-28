//! Domestic-cup rotation coefficients.
//!
//! Every magnitude the cup opportunity bias and goalkeeper adjustment add is
//! named here, so the rotation gradient (rotate hard in early rounds, revert
//! to the strongest XI by the final) is tunable in one place rather than
//! buried in `match` arms. Stage-indexed coefficients hang off [`CupStage`] as
//! methods; the stage-independent thresholds and the squad-status predicate
//! live on the [`CupRotation`] namespace struct. The scoring methods in
//! `scoring.rs` read off these and own only the surrounding logic.

use super::CupStage;
use crate::Player;
use crate::club::PlayerSquadStatus;

impl CupStage {
    /// Index into the `[Early, Quarter, Semi, Final]` coefficient tables.
    fn index(self) -> usize {
        match self {
            CupStage::Early => 0,
            CupStage::Quarter => 1,
            CupStage::Semi => 2,
            CupStage::Final => 3,
        }
    }

    /// Squad-status base, by stage. Established players are eased out of the
    /// early XI; fringe players, backups and prospects are pulled in.
    /// Everything converges on 0.0 at the final.
    pub(crate) fn status_base(self, status: &PlayerSquadStatus) -> f32 {
        // Each table is [Early, Quarter, Semi, Final].
        const KEY_PLAYER: [f32; 4] = [-3.2, -1.4, -0.3, 0.0];
        const FIRST_TEAM_REGULAR: [f32; 4] = [-2.3, -0.9, -0.1, 0.0];
        const FIRST_TEAM_SQUAD_ROTATION: [f32; 4] = [3.0, 1.7, 0.6, 0.0];
        const MAIN_BACKUP: [f32; 4] = [3.6, 2.2, 0.7, 0.0];
        const HOT_PROSPECT: [f32; 4] = [3.2, 1.6, 0.3, 0.0];
        const DECENT_YOUNGSTER: [f32; 4] = [2.2, 0.9, 0.1, 0.0];
        // NotNeeded stays a touch negative at every stage — the cup isn't a
        // reason to start a player the club has frozen out.
        const NOT_NEEDED: f32 = -0.8;

        let i = self.index();
        match status {
            PlayerSquadStatus::KeyPlayer => KEY_PLAYER[i],
            PlayerSquadStatus::FirstTeamRegular => FIRST_TEAM_REGULAR[i],
            PlayerSquadStatus::FirstTeamSquadRotation => FIRST_TEAM_SQUAD_ROTATION[i],
            PlayerSquadStatus::MainBackupPlayer => MAIN_BACKUP[i],
            PlayerSquadStatus::HotProspectForTheFuture => HOT_PROSPECT[i],
            PlayerSquadStatus::DecentYoungster => DECENT_YOUNGSTER[i],
            PlayerSquadStatus::NotNeeded => NOT_NEEDED,
            _ => 0.0,
        }
    }

    /// Youth bonus by age band. Young players get extra rope to play their way
    /// in, but only in the rounds where rotation is on the table.
    pub(crate) fn youth_bonus(self, age: u8) -> f32 {
        match self {
            CupStage::Early => match age {
                17..=18 => 0.9,
                19..=21 => 0.7,
                22..=23 => 0.25,
                _ => 0.0,
            },
            CupStage::Quarter => match age {
                17..=18 => 0.5,
                19..=21 => 0.4,
                _ => 0.0,
            },
            _ => 0.0,
        }
    }

    /// Weight on the underplayed-minutes (days-idle) signal.
    pub(crate) fn idle_weight(self) -> f32 {
        const IDLE_WEIGHT: [f32; 4] = [1.2, 0.7, 0.25, 0.0];
        IDLE_WEIGHT[self.index()]
    }

    /// Weight on the short-of-appearances signal.
    pub(crate) fn appearance_weight(self) -> f32 {
        const APPEARANCE_WEIGHT: [f32; 4] = [1.0, 0.5, 0.0, 0.0];
        APPEARANCE_WEIGHT[self.index()]
    }

    /// Extra protection that pulls an overloaded established player out of the
    /// XI in the rounds where rotation is on the table.
    pub(crate) fn overload_protection(self) -> f32 {
        const OVERLOAD_PROTECTION: [f32; 4] = [-0.8, -0.4, 0.0, 0.0];
        OVERLOAD_PROTECTION[self.index()]
    }

    /// Protection penalty for deep tiredness (recovery debt over threshold).
    pub(crate) fn recovery_debt_penalty(self) -> f32 {
        const RECOVERY_DEBT_PENALTY: [f32; 4] = [-0.7, -0.7, 0.0, 0.0];
        RECOVERY_DEBT_PENALTY[self.index()]
    }

    /// Nudge for a rested non-first-choice keeper to start.
    pub(crate) fn gk_backup(self) -> f32 {
        const GK_BACKUP: [f32; 4] = [2.5, 1.2, 0.3, 0.0];
        GK_BACKUP[self.index()]
    }
}

/// Stage-independent tuning constants and predicates for cup rotation. A
/// namespace struct (no instances) so the thresholds aren't loose module
/// globals.
pub(crate) struct CupRotation;

impl CupRotation {
    /// Days idle is normalised against this window before the idle weight
    /// applies.
    pub(crate) const IDLE_FULL_DAYS: f32 = 21.0;
    /// Season-appearance target below which a player is judged short of
    /// minutes.
    pub(crate) const APPEARANCE_TARGET: f32 = 8.0;

    /// Weekly physical load / minutes at or above which an established player
    /// counts as overloaded.
    pub(crate) const OVERLOAD_PHYSICAL_LOAD: f32 = 300.0;
    pub(crate) const OVERLOAD_MINUTES: f32 = 270.0;
    /// Recovery-debt level above which deep tiredness adds a protection
    /// penalty.
    pub(crate) const RECOVERY_DEBT_THRESHOLD: f32 = 120.0;

    /// Penalty for starting a player still in the post-injury recovery phase,
    /// applied at every stage before the final (the final defers to the
    /// injury-risk penalty and squad depth instead).
    pub(crate) const RECOVERY_STARTING_PENALTY: f32 = -3.0;
    /// Small bench nudge for a recovering player fit enough for cameo minutes.
    pub(crate) const RECOVERING_BENCH_CAMEO: f32 = 0.4;
    pub(crate) const CAMEO_MIN_CONDITION: f32 = 75.0;
    pub(crate) const CAMEO_MIN_IDLE_DAYS: u16 = 14;

    /// A rested non-first-choice keeper needs at least this many idle days
    /// before the backup nudge applies.
    pub(crate) const GK_BACKUP_MIN_IDLE_DAYS: u16 = 14;
    /// The first-choice keeper only steps aside in early rounds, and only when
    /// the opponent is no stronger than this reputation ratio.
    pub(crate) const GK_FIRST_CHOICE_EARLY_PENALTY: f32 = -0.8;
    pub(crate) const GK_FIRST_CHOICE_OPPONENT_RATIO_CAP: f32 = 1.15;

    /// Whether a player's squad status is KeyPlayer or FirstTeamRegular — the
    /// "established XI" group that fitness protection and the GK adjustment
    /// treat specially.
    pub(crate) fn is_established(player: &Player) -> bool {
        player
            .contract
            .as_ref()
            .map(|c| {
                matches!(
                    c.squad_status,
                    PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
                )
            })
            .unwrap_or(false)
    }

    /// Opponent-strength scaling for cup rotation. A heavy underdog cup tie
    /// is exactly the moment a manager goes deepest into rotation — a
    /// drawn-out final is the moment they don't.
    ///
    /// The multiplier is applied to *positive* opportunity bonuses and to
    /// star-rest penalties (i.e. magnitudes that push rotation harder).
    /// Final reverts to 1.0 (no scaling) — the manager picks the best XI
    /// regardless of opponent. Semi only mildly dampens.
    pub(crate) fn rotation_multiplier(stage: super::CupStage, opponent_ratio: f32) -> f32 {
        use super::CupStage;
        match stage {
            CupStage::Final => 1.0,
            CupStage::Semi => {
                // Semi: only mild dampening when opponent is stronger; never
                // amplifies rotation above baseline.
                if opponent_ratio > 1.50 {
                    0.85
                } else if opponent_ratio > 1.15 {
                    0.92
                } else {
                    1.0
                }
            }
            _ => {
                if opponent_ratio <= 0.45 {
                    1.35
                } else if opponent_ratio <= 0.70 {
                    1.20
                } else if opponent_ratio <= 1.15 {
                    1.00
                } else if opponent_ratio <= 1.50 {
                    0.75
                } else {
                    0.55
                }
            }
        }
    }
}
