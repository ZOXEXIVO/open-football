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

use crate::club::CareerRunway;
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
    /// How much being denied those minutes costs him — the prime years he
    /// has left and how hard he pushes, [`Self::MIN_STAKE`]..
    /// [`Self::MIN_STAKE`] + [`Self::STAKE_SPAN`]. Continuous in age: a
    /// birthday never changes how much a missed match hurts.
    pub stake: f32,
}

impl CareerExpectation {
    /// Most a player's own ambition can add to the club's share. Small on
    /// purpose: this is a disagreement about role, not a delusion. Even a
    /// maximally ambitious deputy believes he should play a quarter of
    /// the matches, not that he is the first name on the team sheet.
    const MAX_AMBITION_UPLIFT: f32 = 0.08;
    /// A man winding down still minds being left out — just less.
    const MIN_STAKE: f32 = 0.7;
    /// What a young, maximally ambitious player adds on top.
    const STAKE_SPAN: f32 = 0.6;

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
            stake: Self::stake(player, today),
        }
    }

    /// Career runway weighted by ambition: the years a spectator's season
    /// costs the most, pushed hardest by the men who want most.
    fn stake(player: &Player, today: NaiveDate) -> f32 {
        let years = (today - player.birth_date).num_days() as f32 / 365.25;
        let runway = CareerRunway::at_years(years);
        let ambition01 = (player.attributes.ambition / 20.0).clamp(0.0, 1.0);
        Self::MIN_STAKE + Self::STAKE_SPAN * runway * (0.5 + 0.5 * ambition01)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2031, 11, 9).unwrap()
    }

    fn player(birth: NaiveDate, ambition: f32) -> Player {
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("St".into(), "Ake".into()))
            .birth_date(birth)
            .country_id(1)
            .attributes(PersonAttributes {
                ambition,
                ..Default::default()
            })
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Goalkeeper,
                    level: 18,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn stake(birth: NaiveDate, ambition: f32) -> f32 {
        CareerExpectation::of(&player(birth, ambition), None, today()).stake
    }

    #[test]
    fn stake_rises_with_runway_and_with_ambition() {
        let young = NaiveDate::from_ymd_opt(2008, 5, 19).unwrap();
        let prime = NaiveDate::from_ymd_opt(2003, 5, 19).unwrap();
        assert!(stake(young, 10.0) > stake(prime, 10.0));
        assert!(stake(young, 16.0) > stake(young, 4.0));
    }

    #[test]
    fn a_veteran_sits_at_the_floor() {
        let veteran = NaiveDate::from_ymd_opt(1996, 1, 1).unwrap();
        assert!((stake(veteran, 20.0) - CareerExpectation::MIN_STAKE).abs() < 1e-6);
    }

    #[test]
    fn no_birthday_cliff_at_thirty_one() {
        let day_before = NaiveDate::from_ymd_opt(2000, 11, 10).unwrap();
        let day_after = NaiveDate::from_ymd_opt(2000, 11, 9).unwrap();
        let (before, after) = (stake(day_before, 12.0), stake(day_after, 12.0));
        assert!(
            (before - after).abs() < 1e-3,
            "turning 31 moves the stake by a day's drift, not a step: {before} vs {after}"
        );
    }
}
