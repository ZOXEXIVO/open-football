//! How ready a club is to replace a player it is being asked to sell.

use crate::club::staff::perception::AbilityEstimator;
use crate::{Club, PlayerPositionType};

/// The pieces of a club's readiness to replace one man, each 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplacementReadiness {
    /// The plan's next man for his shirt, as near his level as he is.
    pub heir: f32,
    /// Names the desk holds for the shirt at about his level that the fee
    /// would buy, as far as the days left allow a signing.
    pub names: f32,
}

impl ReplacementReadiness {
    /// Perceived levels behind him at which the next man is a ready heir,
    /// and the span over which readiness falls to nothing.
    const HEIR_READY_GAP: f32 = 4.0;
    const HEIR_SPAN: f32 = 16.0;
    /// Perceived levels below him a name may stand and still count.
    const NAME_GAP: u8 = 8;
    /// What one name is worth; each further one closes the same share.
    const NAME: f32 = 0.35;
    /// Days a signing takes to complete from a standing file.
    const SIGNING_DAYS: f32 = 21.0;

    pub fn overall(&self) -> f32 {
        1.0 - (1.0 - self.heir.clamp(0.0, 1.0)) * (1.0 - self.names.clamp(0.0, 1.0))
    }

    fn heir_term(level: u8, next: Option<u8>) -> f32 {
        let Some(next) = next else {
            return 0.0;
        };
        let gap = f32::from(level) - f32::from(next);
        (1.0 - (gap - Self::HEIR_READY_GAP) / Self::HEIR_SPAN).clamp(0.0, 1.0)
    }

    fn names_term(names: usize, days_left: Option<i64>) -> f32 {
        let held = 1.0 - (1.0 - Self::NAME).powi(names.min(16) as i32);
        // Outside a window there is the whole of the next one to act in.
        let time = days_left.map_or(1.0, |days| {
            (days as f32 / Self::SIGNING_DAYS).clamp(0.0, 1.0)
        });
        held * time
    }
}

/// What a seller knows about losing one man: how ready it is to replace
/// him, and whether he is a first choice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SellerCover {
    pub readiness: f32,
    pub first_choice: bool,
}

impl SellerCover {
    /// Readiness below which a first choice is not sold late in a window.
    const UNREADY: f32 = 0.3;
    /// Days left from which there is no longer time to replace him.
    const TOO_FEW_DAYS: i64 = 7;
    /// An offer this far over the asking price is one the club cannot
    /// refuse, ready or not.
    const IRRESISTIBLE: f64 = 1.5;

    /// The club holds him for this window: nobody to replace him, he is
    /// the first choice and there is no time, and nothing about the offer
    /// or the player overrules it.
    pub fn declines(&self, days_left: Option<i64>, ratio: f64, forced: bool) -> bool {
        self.first_choice
            && self.readiness < Self::UNREADY
            && days_left.is_some_and(|days| days <= Self::TOO_FEW_DAYS)
            && ratio < Self::IRRESISTIBLE
            && !forced
    }
}

impl Club {
    /// The club's cover for `player_id` against `offer` with `days_left`.
    pub fn seller_cover(&self, player_id: u32, offer: f64, days_left: Option<i64>) -> SellerCover {
        let first_choice = self.teams.main().is_some_and(|main| {
            main.staffs
                .head_coach()
                .squad_plan
                .entry(player_id)
                .is_some_and(|e| e.shirt.is_some() && e.place == 0)
        });
        SellerCover {
            readiness: self
                .replacement_readiness(player_id, offer, days_left)
                .overall(),
            first_choice,
        }
    }

    /// How ready the club is to replace `player_id` if it takes `offer`
    /// for him now, with `days_left` of its window to sign somebody: 0
    /// when it has nobody, 1 when it is covered. A man counts only in his
    /// shirt and near his level.
    pub fn replacement_readiness(
        &self,
        player_id: u32,
        offer: f64,
        days_left: Option<i64>,
    ) -> ReplacementReadiness {
        let none = ReplacementReadiness {
            heir: 0.0,
            names: 0.0,
        };
        let Some(main) = self.teams.main() else {
            return none;
        };
        let plan = &main.staffs.head_coach().squad_plan;
        let Some(player) = main.players.find(player_id) else {
            return none;
        };
        let shirt: PlayerPositionType = plan
            .entry(player_id)
            .and_then(|e| e.shirt)
            .unwrap_or_else(|| player.position());
        let level = AbilityEstimator::observable_level(player);

        let next = main
            .players
            .iter()
            .filter(|p| p.id != player_id)
            .filter(|p| plan.entry(p.id).is_some_and(|e| e.shirt == Some(shirt)))
            .map(AbilityEstimator::observable_level)
            .max();
        let names = self
            .transfer_plan
            .watchlist
            .iter()
            .filter(|e| e.slot_position == shirt)
            .filter(|e| {
                e.believed_ability
                    .saturating_add(ReplacementReadiness::NAME_GAP)
                    >= level
            })
            .filter(|e| e.estimated_value <= offer)
            .count();

        ReplacementReadiness {
            heir: ReplacementReadiness::heir_term(level, next),
            names: ReplacementReadiness::names_term(names, days_left),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_heir_at_his_level_is_full_readiness_and_one_far_behind_is_none() {
        assert_eq!(ReplacementReadiness::heir_term(150, Some(148)), 1.0);
        assert_eq!(ReplacementReadiness::heir_term(150, Some(120)), 0.0);
        assert_eq!(ReplacementReadiness::heir_term(150, None), 0.0);
        let mut previous = 1.0;
        for next in (120..=150).rev() {
            let now = ReplacementReadiness::heir_term(150, Some(next));
            assert!(now <= previous && previous - now < 0.07, "a step at {next}");
            previous = now;
        }
    }

    #[test]
    fn names_count_for_less_the_fewer_days_are_left_to_sign_one() {
        let three = |days| ReplacementReadiness::names_term(3, days);
        assert!(three(Some(30)) > three(Some(10)));
        assert!(three(Some(10)) > three(Some(0)));
        assert_eq!(three(Some(0)), 0.0);
        assert_eq!(three(None), three(Some(30)));
        assert!(ReplacementReadiness::names_term(5, Some(30)) > three(Some(30)));
        assert_eq!(ReplacementReadiness::names_term(0, Some(30)), 0.0);
    }
}
