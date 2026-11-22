//! How one shirt's football is divided between its first choice and the men
//! behind him, and what the club has already promised out of it.

use crate::club::player::{ManagerPromiseKind, PlayerUsage};
use crate::{Player, PlayerClubContract};
use chrono::NaiveDate;

/// What the division reads besides the chart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShareClimate {
    /// Competitive fixtures a week the club expects.
    pub fixtures_per_week: f32,
    /// The coach's own tendency to rotate, 0..1.
    pub rotation: f32,
    /// How far the club exists to bring players through, 0..1: its
    /// philosophy.
    pub development: f32,
    /// How far this side of the club exists to develop players rather than
    /// to win, 0..1 ([`crate::TeamType::development_weight`]).
    pub side: f32,
}

impl ShareClimate {
    pub const ONE_COMPETITION: ShareClimate = ShareClimate {
        fixtures_per_week: 1.0,
        rotation: 0.5,
        development: 0.3,
        side: 0.0,
    };
}

/// One man planned for a shirt, as the division sees him.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShirtCandidate {
    /// His standing for the shirt in the depth chart.
    pub standing: f32,
    /// How far the staff believe he will go beyond his present level, 0..1.
    /// Nothing for a man who is what he will be.
    pub belief: f32,
}

pub struct ShirtShares;

impl ShirtShares {
    /// A first choice with nobody near him, at a club with one match a week
    /// under a coach of ordinary habits.
    const FIRST_CHOICE: f32 = 0.92;
    /// Taken off for each competitive fixture a week beyond one.
    const LOAD: f32 = 0.20;
    /// The swing between the coach who never rotates and the one who always
    /// does.
    const ROTATION: f32 = 0.12;
    /// Taken off when the next man is as good, fading with the gap.
    const GAP: f32 = 0.24;
    const GAP_SCALE: f32 = 8.0;
    /// A goalkeeper shares least: the load and the coach's habits reach
    /// him at this fraction.
    const KEEPER_SENSITIVITY: f32 = 0.25;
    const KEEPER_FIRST_CHOICE: f32 = 0.95;
    const LEAST: f32 = 0.50;
    const MOST: f32 = 0.97;
    /// What a first choice gives up at a side that exists only to develop
    /// players, and how much lower his least share goes there.
    const SIDE_SPREAD: f32 = 0.30;
    const SIDE_LEAST: f32 = 0.25;
    /// Standing behind the second man at which a man further back gets
    /// about a third of what the second man does.
    const BEHIND_SCALE: f32 = 8.0;
    /// What a first choice gives up to the prospect the staff believe in
    /// most, at a club that exists to develop.
    const PROSPECT_YIELD: f32 = 0.12;
    /// How much further a believed-in prospect reaches into what is left.
    const PROSPECT_LIFT: f32 = 1.5;
    /// Standing behind the first choice at which a prospect is half as
    /// near to him as one level with him.
    const NEARNESS_SCALE: f32 = 12.0;

    /// The share of his shirt's football a first choice is planned for.
    /// `gap` is his standing over the next man's; `None` when nobody stands
    /// behind him.
    pub fn first_choice(climate: &ShareClimate, gap: Option<f32>, keeper: bool) -> f32 {
        let Some(gap) = gap else {
            return 1.0;
        };
        let (base, sensitivity) = if keeper {
            (Self::KEEPER_FIRST_CHOICE, Self::KEEPER_SENSITIVITY)
        } else {
            (Self::FIRST_CHOICE, 1.0)
        };
        let load = Self::LOAD * (climate.fixtures_per_week - 1.0).clamp(0.0, 1.5);
        let habit = Self::ROTATION * (climate.rotation.clamp(0.0, 1.0) - 0.5);
        let rival = Self::GAP * (-gap.max(0.0) / Self::GAP_SCALE).exp();
        let side = climate.side.clamp(0.0, 1.0);
        (base - sensitivity * (load + habit + rival + Self::SIDE_SPREAD * side))
            .clamp(Self::least(climate), Self::MOST)
    }

    fn least(climate: &ShareClimate) -> f32 {
        Self::LEAST - Self::SIDE_LEAST * climate.side.clamp(0.0, 1.0)
    }

    /// How hard a prospect pulls on his shirt: the staff's belief in him,
    /// how near he already is to the man ahead, and what the club is for.
    fn prospect_pull(climate: &ShareClimate, first: &ShirtCandidate, man: &ShirtCandidate) -> f32 {
        let nearness = (-(first.standing - man.standing).max(0.0) / Self::NEARNESS_SCALE).exp();
        let lean = climate.development.max(climate.side).clamp(0.0, 1.0);
        man.belief.clamp(0.0, 1.0) * nearness * (0.4 + 1.2 * lean)
    }

    /// What a rotation player is planned for: the football a first choice
    /// leaves the man an ordinary step behind him.
    pub fn rotation(climate: &ShareClimate, keeper: bool) -> f32 {
        1.0 - Self::first_choice(climate, Some(Self::GAP_SCALE), keeper)
    }

    /// The shares of the men planned for one shirt, first choice first.
    /// They sum to one.
    pub fn divide(climate: &ShareClimate, men: &[ShirtCandidate], keeper: bool) -> Vec<f32> {
        let Some(first) = men.first() else {
            return Vec::new();
        };
        if men.len() == 1 {
            return vec![1.0];
        }
        let pulls: Vec<f32> = men[1..]
            .iter()
            .map(|man| Self::prospect_pull(climate, first, man))
            .collect();
        let strongest = pulls.iter().copied().fold(0.0_f32, f32::max);
        let lead = (Self::first_choice(climate, Some(first.standing - men[1].standing), keeper)
            - Self::PROSPECT_YIELD * strongest.min(1.0))
        .clamp(Self::least(climate), Self::MOST);

        // What the first choice leaves is divided by how near each man
        // stands to the second, so two men level with each other are
        // planned alike whichever the chart happens to list first.
        let weights: Vec<f32> = men[1..]
            .iter()
            .zip(&pulls)
            .map(|(man, pull)| {
                (-(men[1].standing - man.standing).max(0.0) / Self::BEHIND_SCALE).exp()
                    * (1.0 + Self::PROSPECT_LIFT * pull)
            })
            .collect();
        let total: f32 = weights.iter().sum();
        let mut shares = Vec::with_capacity(men.len());
        shares.push(lead);
        shares.extend(weights.iter().map(|w| (1.0 - lead) * w / total));
        shares
    }
}

/// What a commitment is, strongest first: when two ask for more than a shirt
/// has, the earlier one is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlanFloorKind {
    /// The appearances and the match fee a loan agreement names.
    LoanAgreement,
    /// The role the board bought him for.
    BoardMandate,
    /// A promise his manager made him.
    ManagerPromise,
    /// Football to be seen in, for a man the club is selling.
    ShopWindow,
}

/// The least football the club has committed to a player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanFloor {
    pub kind: PlanFloorKind,
    pub share: f32,
}

impl PlanFloor {
    /// What one appearance is worth in football: a start and a cameo both
    /// count toward an agreed number.
    const FOOTBALL_PER_APPEARANCE: f32 = 0.75;
    /// No commitment asks for more than this of a shirt.
    const MOST: f32 = 0.9;
    /// A match fee worth a match's wage is this much of a shirt.
    const FEE_SHARE: f32 = 0.3;
    const FEE_MOST: f32 = 0.45;
    /// Matches a season's wage is spread over when a fee is read against it.
    const FEE_REFERENCE_MATCHES: f32 = 40.0;
    /// A promise of football without a number.
    const PLAYING_TIME: f32 = 0.2;

    /// Every commitment the club holds to `player`, strongest first.
    pub fn of(
        player: &Player,
        coach_id: u32,
        climate: &ShareClimate,
        today: NaiveDate,
    ) -> Vec<PlanFloor> {
        let mut floors = Vec::new();
        if let Some(share) = player
            .contract_loan
            .as_ref()
            .map(|loan| Self::loan_share(player, loan, climate, today))
            .filter(|share| *share > 0.0)
        {
            floors.push(PlanFloor {
                kind: PlanFloorKind::LoanAgreement,
                share,
            });
        }
        if let Some(share) = Self::mandate_share(player, today) {
            floors.push(PlanFloor {
                kind: PlanFloorKind::BoardMandate,
                share,
            });
        }
        if let Some(share) = Self::promise_share(player, coach_id, today) {
            floors.push(PlanFloor {
                kind: PlanFloorKind::ManagerPromise,
                share,
            });
        }
        floors
    }

    fn loan_share(
        player: &Player,
        loan: &PlayerClubContract,
        climate: &ShareClimate,
        today: NaiveDate,
    ) -> f32 {
        let agreed = loan
            .loan_min_appearances
            .map(|target| {
                let made = PlayerUsage::of(player).appearances;
                let weeks_left = (loan.expiration - today).num_days().max(7) as f32 / 7.0;
                let fixtures_left = (weeks_left * climate.fixtures_per_week).max(1.0);
                f32::from(target.saturating_sub(made)) / fixtures_left
                    * Self::FOOTBALL_PER_APPEARANCE
            })
            .unwrap_or(0.0);
        let fee = loan.loan_fee_for_appearance(true) as f32;
        let match_wage = (loan.salary.max(1) as f32 / Self::FEE_REFERENCE_MATCHES).max(1.0);
        let paid = (fee / match_wage * Self::FEE_SHARE).min(Self::FEE_MOST);
        agreed.max(paid).min(Self::MOST)
    }

    /// The share the board's mandate promises this season, while the
    /// mandate still covers it.
    fn mandate_share(player: &Player, today: NaiveDate) -> Option<f32> {
        let mandate = player.mandate().filter(|m| m.is_purchase())?;
        let season = ((today - mandate.issued).num_days().max(0) / 365) as u8;
        (season <= mandate.minutes.review_horizon())
            .then(|| mandate.minutes.share(season).min(Self::MOST))
    }

    fn promise_share(player: &Player, coach_id: u32, today: NaiveDate) -> Option<f32> {
        player
            .promises
            .iter()
            .filter(|promise| today >= promise.made_on && today <= promise.deadline)
            .filter(|promise| promise.made_by_staff_id.is_none_or(|id| id == coach_id))
            .filter_map(|promise| match promise.kind {
                ManagerPromiseKind::StartingRole => {
                    let share = if promise.target_value == 0 {
                        60
                    } else {
                        promise.target_value.min(100)
                    };
                    Some(f32::from(share) / 100.0)
                }
                ManagerPromiseKind::PlayingTime => Some(Self::PLAYING_TIME),
                _ => None,
            })
            .fold(None, |best: Option<f32>, share| {
                Some(best.map_or(share, |b| b.max(share)))
            })
            .map(|share| share.min(Self::MOST))
    }

    /// Raise each floored man of one shirt to what he was promised, the
    /// strongest commitments first, taking the football from the others of
    /// the same shirt. Returns the men whose floor could not be met.
    pub fn apply(shares: &mut [f32], floors: &[Option<PlanFloor>]) -> Vec<usize> {
        let mut order: Vec<usize> = (0..shares.len()).filter(|i| floors[*i].is_some()).collect();
        order.sort_by_key(|i| floors[*i].map(|f| f.kind));
        let mut held = vec![false; shares.len()];
        let mut unmet = Vec::new();
        for i in order {
            let Some(floor) = floors[i] else {
                continue;
            };
            let free: f32 = (0..shares.len())
                .filter(|j| *j != i && !held[*j])
                .map(|j| shares[j])
                .sum();
            let wanted = (floor.share - shares[i]).max(0.0);
            let taken = wanted.min(free);
            if taken > 0.0 {
                let keep = (free - taken) / free;
                for j in (0..shares.len()).filter(|j| *j != i && !held[*j]) {
                    shares[j] *= keep;
                }
                shares[i] += taken;
            }
            if taken + 1e-4 < wanted {
                unmet.push(i);
            }
            held[i] = true;
        }
        unmet
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn man(standing: f32) -> ShirtCandidate {
        ShirtCandidate {
            standing,
            belief: 0.0,
        }
    }

    fn prospect(standing: f32, belief: f32) -> ShirtCandidate {
        ShirtCandidate { standing, belief }
    }

    fn busy() -> ShareClimate {
        ShareClimate {
            fixtures_per_week: 1.6,
            ..ShareClimate::ONE_COMPETITION
        }
    }

    #[test]
    fn a_club_with_one_competition_plans_its_first_choices_for_most_of_the_football() {
        let shares = ShirtShares::divide(
            &ShareClimate::ONE_COMPETITION,
            &[man(140.0), man(130.0), man(120.0)],
            false,
        );
        assert!(shares[0] > 0.8, "{shares:?}");
        assert!(shares[2] < 0.05, "cover is planned for little: {shares:?}");
    }

    #[test]
    fn a_club_in_europe_spreads_the_same_shirt_wider() {
        let men = [man(140.0), man(130.0), man(120.0)];
        let quiet = ShirtShares::divide(&ShareClimate::ONE_COMPETITION, &men, false);
        let crowded = ShirtShares::divide(&busy(), &men, false);
        assert!(
            crowded[0] < quiet[0] - 0.08,
            "{quiet:?} against {crowded:?}"
        );
        assert!(crowded[1] > quiet[1] && crowded[2] > quiet[2]);
    }

    #[test]
    fn a_shirts_shares_always_add_up_to_the_shirt() {
        for count in 1..6 {
            let men: Vec<ShirtCandidate> = (0..count)
                .map(|i| prospect(140.0 - 7.0 * i as f32, 0.2 * i as f32))
                .collect();
            for keeper in [false, true] {
                let total: f32 = ShirtShares::divide(&busy(), &men, keeper).iter().sum();
                assert!((total - 1.0).abs() < 1e-5, "{count} men: {total}");
            }
        }
    }

    #[test]
    fn a_clear_gap_leaves_the_first_choice_nearly_everything() {
        let clear = ShirtShares::divide(
            &ShareClimate::ONE_COMPETITION,
            &[man(160.0), man(110.0)],
            false,
        );
        let close = ShirtShares::divide(
            &ShareClimate::ONE_COMPETITION,
            &[man(160.0), man(159.0)],
            false,
        );
        assert!(clear[0] > 0.9, "{clear:?}");
        assert!(close[0] < clear[0] - 0.1, "{close:?}");
    }

    #[test]
    fn the_share_moves_smoothly_with_everything_it_reads() {
        let mut last = ShirtShares::first_choice(&ShareClimate::ONE_COMPETITION, Some(6.0), false);
        for step in 0..=30 {
            let climate = ShareClimate {
                fixtures_per_week: 1.0 + step as f32 / 20.0,
                ..ShareClimate::ONE_COMPETITION
            };
            let now = ShirtShares::first_choice(&climate, Some(6.0), false);
            assert!(now <= last && last - now < 0.02, "step {step}");
            last = now;
        }
        let steady = ShareClimate {
            rotation: 0.1,
            ..ShareClimate::ONE_COMPETITION
        };
        let restless = ShareClimate {
            rotation: 0.9,
            ..ShareClimate::ONE_COMPETITION
        };
        assert!(
            ShirtShares::first_choice(&steady, Some(6.0), false)
                > ShirtShares::first_choice(&restless, Some(6.0), false)
        );
    }

    #[test]
    fn a_goalkeeper_shares_least() {
        let keeper = ShirtShares::first_choice(&busy(), Some(3.0), true);
        let outfield = ShirtShares::first_choice(&busy(), Some(3.0), false);
        assert!(
            keeper > outfield + 0.1 && keeper > 0.85,
            "{keeper} against {outfield}"
        );
    }

    #[test]
    fn a_prospects_share_follows_belief_and_what_the_club_is_for() {
        let developing = ShareClimate {
            development: 1.0,
            ..ShareClimate::ONE_COMPETITION
        };
        let buying = ShareClimate {
            development: 0.0,
            ..ShareClimate::ONE_COMPETITION
        };
        let shirt = |belief: f32| [man(140.0), prospect(128.0, belief)];
        let believed = ShirtShares::divide(&developing, &shirt(0.9), false)[1];
        let doubted = ShirtShares::divide(&developing, &shirt(0.1), false)[1];
        let elsewhere = ShirtShares::divide(&buying, &shirt(0.9), false)[1];
        assert!(believed > doubted, "{believed} against {doubted}");
        assert!(believed > elsewhere, "{believed} against {elsewhere}");
    }

    fn floor(kind: PlanFloorKind, share: f32) -> Option<PlanFloor> {
        Some(PlanFloor { kind, share })
    }

    #[test]
    fn two_prospects_equally_far_behind_and_equally_believed_in_are_planned_alike() {
        // The division is given a standing and a belief for each man and
        // nothing else: a 20-year-old and a 22-year-old the staff rate and
        // believe in equally are one case.
        let shares = ShirtShares::divide(
            &ShareClimate::ONE_COMPETITION,
            &[man(140.0), prospect(128.0, 0.6), prospect(128.0, 0.6)],
            false,
        );
        assert!((shares[1] - shares[2]).abs() < 1e-6, "{shares:?}");
        assert!(shares[1] > 0.0);

        let further = ShirtShares::divide(
            &ShareClimate::ONE_COMPETITION,
            &[man(140.0), prospect(128.0, 0.6), prospect(120.0, 0.6)],
            false,
        );
        assert!(further[2] < further[1], "{further:?}");
    }

    #[test]
    fn a_floor_is_met_from_the_other_men_of_the_shirt() {
        let mut shares = vec![0.8, 0.15, 0.05];
        let unmet = PlanFloor::apply(
            &mut shares,
            &[None, floor(PlanFloorKind::BoardMandate, 0.8), None],
        );
        assert!(unmet.is_empty());
        assert!((shares[1] - 0.8).abs() < 1e-5, "{shares:?}");
        assert!((shares.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert!(shares[0] > shares[2], "the others keep their order");
    }

    #[test]
    fn two_floors_for_one_shirt_are_kept_in_order_and_the_later_one_recorded() {
        let mut shares = vec![0.8, 0.15, 0.05];
        let unmet = PlanFloor::apply(
            &mut shares,
            &[
                None,
                floor(PlanFloorKind::ManagerPromise, 0.6),
                floor(PlanFloorKind::LoanAgreement, 0.7),
            ],
        );
        assert!(
            (shares[2] - 0.7).abs() < 1e-5,
            "the loan agreement is kept: {shares:?}"
        );
        assert!(shares[1] < 0.6, "the promise cannot be: {shares:?}");
        assert_eq!(unmet, vec![1]);
        assert!((shares.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }
}
