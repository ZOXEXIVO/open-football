//! Match-day rotation scoring for national teams.
//!
//! The call-up pipeline ([`super::callup`]) decides the 23/26-man squad;
//! this module decides *who actually starts a given fixture*. Historically
//! the match XI was a raw strongest-by-ability pick with no rotation, so
//! the same eleven started every match all season while the rest of the
//! squad never earned a cap. The scoring here layers two realistic
//! rotation drivers on top of raw merit:
//!
//!   * **freshness** — a player who has recently played carries depleted
//!     match-condition, so a comparable but rested deputy overtakes them.
//!     This yields natural back-to-back rotation across an international
//!     double-header without any explicit "who played last game"
//!     bookkeeping: the depleted condition *is* that memory, and it
//!     recovers day by day exactly as a manager's fitness read would.
//!   * **experimentation** — low-stakes fixtures (group qualifiers, and
//!     especially friendlies) make room for uncapped youth, exactly as a
//!     manager bloods prospects when the result matters less. Knockouts
//!     switch experimentation off and damp freshness so the strongest fit
//!     XI takes the field.
//!
//! Both deltas are bounded so they only reorder genuinely comparable
//! players — a clearly stronger regular is never dropped for a much weaker
//! fringe player in a match that matters. Every input is deterministic
//! player state, so squads stay reproducible.

use super::NationalTeam;
use super::types::NationalMatchImportance;
use crate::{Player, PlayerFieldPositionGroup, PlayerPositionType};
use chrono::{Datelike, NaiveDate};

impl NationalTeam {
    /// Merit for fielding `player` in a specific tactical slot, adjusted
    /// for freshness and low-stakes experimentation. Preserves the legacy
    /// `pos_fit * 3 + ability` ordering as the merit base, then applies
    /// the rotation deltas on the same scale.
    pub(super) fn matchday_position_score(
        player: &Player,
        pos: PlayerPositionType,
        importance: NationalMatchImportance,
        date: NaiveDate,
    ) -> f32 {
        let pos_fit = player.positions.get_level(pos) as f32;
        let ability = player.player_attributes.current_ability as f32;
        let merit = pos_fit * 3.0 + ability;
        merit + Self::matchday_rotation_delta(player, importance, date)
    }

    /// Merit for fielding `player` at their natural best position — used
    /// for the goalkeeper pick, the "fill any remaining slot" pass, and
    /// bench ordering, where no specific slot is being contested.
    pub(super) fn matchday_overall_score(
        player: &Player,
        importance: NationalMatchImportance,
        date: NaiveDate,
    ) -> f32 {
        let ability = player.player_attributes.current_ability as f32;
        ability + Self::matchday_rotation_delta(player, importance, date)
    }

    /// Combined freshness + experimentation adjustment, in points on the
    /// same ~0..260 scale as merit. Bounded so it is never large enough on
    /// its own to promote a clearly weaker player into a fixture that
    /// matters.
    fn matchday_rotation_delta(
        player: &Player,
        importance: NationalMatchImportance,
        date: NaiveDate,
    ) -> f32 {
        Self::matchday_fatigue_delta(player, importance)
            + Self::matchday_experimentation_delta(player, importance, date)
    }

    /// Non-positive freshness term. Fully rested → 0; a player who has
    /// just played and not recovered → down to roughly −`swing`. Damped
    /// for knockouts (rest only the genuinely spent) and for goalkeepers
    /// (a #1 keeper plays through congestion in real football).
    fn matchday_fatigue_delta(player: &Player, importance: NationalMatchImportance) -> f32 {
        let freshness = Self::matchday_freshness(player); // 0..1, 1 = fresh
        let mut swing = match importance {
            NationalMatchImportance::Peak => 12.0,
            NationalMatchImportance::Competitive => 24.0,
            NationalMatchImportance::Friendly => 24.0,
        };
        if player.position().position_group() == PlayerFieldPositionGroup::Goalkeeper {
            swing *= 0.4;
        }
        (freshness - 1.0) * swing
    }

    /// 0..1 readiness from match-condition, recent-match sharpness and
    /// physical match-readiness. Condition dominates because it is what
    /// the match engine depletes and then recovers day by day, making it
    /// the honest "has this player just played" signal that separates a
    /// double-header's match-one starters from its rested bench.
    fn matchday_freshness(player: &Player) -> f32 {
        let condition = player.player_attributes.condition_percentage() as f32 / 100.0;
        let days_since = player.player_attributes.days_since_last_match as f32;
        let sharpness = if days_since <= 3.0 {
            1.0
        } else if days_since <= 7.0 {
            0.97
        } else if days_since <= 14.0 {
            0.9
        } else {
            0.8
        };
        let physical = (player.skills.physical.match_readiness / 20.0).clamp(0.0, 1.0);
        (condition * 0.7 + sharpness * 0.2 + physical * 0.1).clamp(0.0, 1.0)
    }

    /// Non-negative experimentation term: low-stakes fixtures lift
    /// uncapped / lightly-capped young players so prospects earn real
    /// minutes, and (in friendlies) gently ease well-capped veterans out.
    /// Knockouts return 0 — no experimenting when it counts.
    fn matchday_experimentation_delta(
        player: &Player,
        importance: NationalMatchImportance,
        date: NaiveDate,
    ) -> f32 {
        let caps = player.player_attributes.international_apps;
        let age = date.year() - player.birth_date.year();
        match importance {
            NationalMatchImportance::Peak => 0.0,
            NationalMatchImportance::Competitive => {
                if caps == 0 && age <= 23 {
                    8.0
                } else if caps <= 2 && age <= 23 {
                    5.0
                } else {
                    0.0
                }
            }
            NationalMatchImportance::Friendly => {
                if caps == 0 && age <= 23 {
                    22.0
                } else if caps == 0 && age <= 27 {
                    16.0
                } else if caps <= 3 && age <= 24 {
                    12.0
                } else if caps >= 40 && age >= 32 {
                    -8.0
                } else {
                    0.0
                }
            }
        }
    }
}
