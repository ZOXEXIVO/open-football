//! What a player believes his own role should be — as distinct from the
//! role his club has assigned him.
//!
//! Before this existed, expectation was derived entirely from the squad
//! status the club had stamped on the contract, so a player could never
//! disagree with his own label: call him a backup and he expected to be
//! a backup, forever, however good he was and however badly he wanted
//! more. Every squad member was, in effect, of one mind with the coach.
//!
//! Three channels feed the belief, and all of them are things the player
//! himself can observe:
//!   * the club's label — the honest baseline, and what he was told when
//!     he signed;
//!   * his own record of official starts, level-adjusted (a man who owned
//!     a full loan season does not go back to accepting a prospect's
//!     share) — see [`MatchExperienceBackground::expected_start_share_floor`];
//!   * his ambition, weighted by where he is in his career. The years in
//!     which being a spectator costs a career rather than a season are
//!     the years he pushes hardest.
//!
//! Deliberately one-directional: the belief only ever RAISES the bar. The
//! status table already encodes patience, and a modest player should not
//! be made easier to please than his contract says — he should simply not
//! push. Ambition near zero therefore reproduces the old behaviour
//! exactly, which is what keeps the settled-squad-player calibration
//! intact.

use crate::club::person::Person;
use crate::club::player::player::Player;
use crate::club::player::statistics::StuckCareerScan;
use crate::{MatchExperienceBackground, PlayerSquadStatus};
use chrono::NaiveDate;

use super::processing::PlayingTimeFrustrationConfig;

/// A player's own view of the role he should have, and how far it
/// diverges from the club's.
#[derive(Debug, Clone, Copy)]
pub struct CareerExpectation {
    /// Share of the club's eligible matches he believes he should start.
    pub expected_start_share: f32,
    /// The club's own answer to the same question, from the squad-status
    /// table — carried so consumers can talk about the gap rather than
    /// re-deriving it.
    pub club_expected_start_share: f32,
}

impl CareerExpectation {
    /// Most a player's own ambition can add to the club's share. Small on
    /// purpose: this is a disagreement about role, not a delusion. Even a
    /// maximally ambitious deputy believes he should play a quarter of
    /// the matches, not that he is the first name on the team sheet.
    const MAX_AMBITION_UPLIFT: f32 = 0.08;

    /// Build the player's expectation of himself.
    pub fn of(player: &Player, status: Option<&PlayerSquadStatus>, today: NaiveDate) -> Self {
        let club_expected_start_share = PlayingTimeFrustrationConfig::expected_start_share(status);

        // What his own record says he is worth. Level-adjusted inside the
        // background, so a fourth-tier record does not demand top-flight
        // minutes.
        let current_team_reputation = player
            .statistics_history
            .current
            .iter()
            .rev()
            .find(|e| e.departed_date.is_none())
            .map(|e| e.team_reputation)
            .unwrap_or(0);
        let record_floor = MatchExperienceBackground::from_player(player)
            .expected_start_share_floor(current_team_reputation);

        let expected_start_share =
            club_expected_start_share.max(record_floor) + Self::ambition_uplift(player, today);

        Self {
            expected_start_share,
            club_expected_start_share,
        }
    }

    /// How much the player's own ambition raises his bar, 0..
    /// [`Self::MAX_AMBITION_UPLIFT`].
    ///
    /// Weighted by career stage on the same curve the restlessness model
    /// uses, so a player pushes hardest in exactly the years when sitting
    /// out costs him a career, and makes his peace as the fade sets in. A
    /// player who is already playing does not push at all — the uplift is
    /// about a role he is being denied, not one he holds.
    fn ambition_uplift(player: &Player, today: NaiveDate) -> f32 {
        let ambition01 = (player.attributes.ambition / 20.0).clamp(0.0, 1.0);
        if ambition01 <= 0.0 {
            return 0.0;
        }
        let age = player.age(today);
        let is_goalkeeper = player.position().is_goalkeeper();
        let (prime_start, prime_end, fade_end) = StuckCareerScan::career_phases(is_goalkeeper);
        let a = age as f32;
        // Youngsters below the prime window are still being brought
        // through and the development pathway owns their expectations;
        // past the fade there is nothing left to push for.
        let stage_weight: f32 = if a < prime_start - 3.0 {
            0.0
        } else if a < prime_start {
            ((a - (prime_start - 3.0)) / 3.0).clamp(0.0, 1.0)
        } else if a <= prime_end {
            1.0
        } else {
            (1.0f32 - (a - prime_end) / (fade_end - prime_end)).clamp(0.0, 1.0)
        };

        Self::MAX_AMBITION_UPLIFT * ambition01 * stage_weight
    }

    /// How far the player's belief outruns his club's label, in share
    /// points. Zero when they agree — which, for a contented squad
    /// player, is most of the time.
    pub fn role_disagreement(&self) -> f32 {
        (self.expected_start_share - self.club_expected_start_share).max(0.0)
    }
}
