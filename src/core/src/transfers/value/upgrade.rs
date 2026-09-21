//! L3 — why this player, why this fee.
//!
//! The pipeline could already decide *that* a club wanted a position filled.
//! What it had no way to say was what a particular player at a particular
//! price was WORTH to a particular club — so affordability was a single
//! comparison (`fee ≤ 1.4 × budget`) and a two-billion balance read exactly
//! like a thirty-million one.
//!
//! The whole difference between those two clubs is the marginal value of a
//! dollar. A club with years of income in the bank values its next dollar at
//! a fraction of what a break-even club does, which is the entire reason it
//! can pay a transformative fee for a marginal upgrade while a poorer club
//! cannot — and why the poorer club, offered the same fee for a player, sells
//! (L4 reads the same asymmetry from the other side).
//!
//! Everything here is belief on the buying side: believed ability, believed
//! ceiling, the club's own read of its incumbent. The gates that decide
//! whether a move is legal at all keep reading truth.

use crate::club::board::mandate::MinutesCurve;
use crate::club::player::contract::contract::ClubLevelAnchor;
use crate::club::staff::perception::AbilityEstimator;
use crate::transfers::squad::plan::{BriefTier, RoleWeight};
use crate::{Club, PlayerFieldPositionGroup};

/// The marginal value of one unit of a club's money.
///
/// Continuous in idle cash measured against the club's own annual income:
/// a club with nothing spare values a dollar at 1.0, a club sitting on
/// years of income values it at the floor. There is no tier, no "if elite"
/// — a newly-rich mid-table club and a struggling giant swap places on this
/// axis the moment their balance sheets do.
#[derive(Debug, Clone, Copy)]
pub struct MoneyUtility {
    value: f64,
}

impl MoneyUtility {
    /// Lowest a dollar is ever worth. A club that valued money at nothing
    /// would pay any fee for any improvement; a third is roughly the ratio
    /// between what a state-backed giant and a break-even club will pay for
    /// the same marginal upgrade.
    pub const FLOOR: f64 = 0.35;
    /// How fast idle cash discounts the next dollar. At `idle_cash =
    /// annual_income` the utility is `1 / 1.6 ≈ 0.63`; at three years of
    /// income it has reached the floor.
    pub const IDLE_CASH_K: f64 = 0.6;

    /// `idle_cash` is cash behind the wage cover — see
    /// [`crate::transfers::squad::plan::MoneySlack`]. A club with no revenue history
    /// values money at 1.0 (fail closed: it has no evidence it is rich).
    pub fn of(idle_cash: f64, annual_income: f64) -> Self {
        if annual_income <= 0.0 {
            return MoneyUtility { value: 1.0 };
        }
        let ratio = (idle_cash / annual_income).max(0.0);
        MoneyUtility {
            value: (1.0 / (1.0 + Self::IDLE_CASH_K * ratio)).max(Self::FLOOR),
        }
    }

    /// Neutral utility — every dollar is worth a dollar. Used where a club's
    /// finances are not available to the caller.
    pub fn neutral() -> Self {
        MoneyUtility { value: 1.0 }
    }

    pub fn value(&self) -> f64 {
        self.value
    }
}

/// How much a season of one observable-ability point of first-team
/// improvement is worth to a club, in money.
///
/// The exchange rate between quality and money is not universal — that is
/// the point. A point of improvement in a side playing for a title in front
/// of eighty thousand people is worth orders of magnitude more than the same
/// point at a club whose whole season turns over less than one week of the
/// former's wages, and the market prices exactly that difference.
#[derive(Debug, Clone, Copy)]
pub struct PointValue {
    per_season: f64,
}

impl PointValue {
    /// Share of a club's annual income one ability point of first-team
    /// improvement is worth for one season.
    ///
    /// Calibrated against the real band in Part VIII of the design: gross
    /// spend ÷ income ≈ 0.35–0.55 for a big-league club, and a
    /// transformative signing is about half of one window. Solving that
    /// shape for a tier-A signing — eight points over four usable years at
    /// a marquee wage — puts the share here, which is where the walk-away
    /// stops sitting under every plausible ask and can act as the buyer's
    /// number for EVERY deal rather than only above the seller's price.
    const INCOME_SHARE: f64 = 0.009;

    pub fn of(annual_income: f64) -> Self {
        PointValue {
            per_season: annual_income.max(0.0) * Self::INCOME_SHARE,
        }
    }

    pub fn per_season(&self) -> f64 {
        self.per_season
    }
}

/// Everything the buying club believes about one deal.
///
/// Note what is absent: hidden current or potential ability. `believed_level`
/// is the club's own noisy read (`ClubOpinion` over `observable ability`),
/// `incumbent_level` is what its coach sees in the man already in the shirt,
/// and `believed_ceiling` is the potential ESTIMATE. A club can be wrong
/// about all three, which is what makes it a market.
#[derive(Debug, Clone, Copy)]
pub struct DealInputs {
    pub group: PlayerFieldPositionGroup,
    pub tier: BriefTier,
    /// Believed ability of the target, in observable points, after the
    /// buyer has discounted what it does not know about him.
    pub believed_level: f32,
    /// Believed ability of the man whose shirt this is. Zero for an empty
    /// shirt.
    pub incumbent_level: f32,
    /// What a free body gives in this shirt at this club.
    pub replacement_level: f32,
    /// Believed ceiling — drives resale, never the improvement.
    pub believed_ceiling: f32,
    /// 0..1 distance between the buyer's league and the seller's. A step-up
    /// buy is worth less on resale because the buyer may be the only club
    /// that believed the number.
    pub league_gap: f32,
    /// What the club said he would play, season by season.
    pub minutes: MinutesCurve,
    pub age: u8,
    /// Annual wage the buyer would pay him.
    pub annual_wage: f64,
    /// Buyer's trailing annual income.
    pub annual_income: f64,
    /// Buyer's idle cash behind the wage cover.
    pub idle_cash: f64,
}

/// What a deal is worth, and the fee at which it stops being worth doing.
#[derive(Debug, Clone, Copy)]
pub struct DealValue {
    /// Sporting benefit, in money, over the years the club expects to use
    /// him. Zero when he is no improvement at all.
    pub benefit: f64,
    /// Years the club expects to get out of him.
    pub years_of_use: f64,
    /// Share of a fee the club expects to get back when he leaves.
    pub resale_fraction: f64,
    /// Marginal value of the buyer's money.
    pub money_utility: f64,
    /// Highest fee at which the deal still has non-negative value. Zero
    /// when no fee works.
    pub ceiling_fee: f64,
    /// Believed improvement over the incumbent, in ability points.
    pub gain: f32,
    /// Points he brings over the man whose minutes the mandate takes.
    pub contribution: f32,
}

impl DealValue {
    /// Net value of the deal at a given fee. Positive means worth doing.
    pub fn at_fee(&self, fee: f64, annual_wage: f64) -> f64 {
        let net_cost = fee * (1.0 - self.resale_fraction)
            + annual_wage * self.years_of_use
            + UpgradeMath::agent_fee(fee);
        self.benefit - net_cost * self.money_utility
    }
}

/// What one buying club believes about one target, in the shape the
/// pipeline can actually supply at negotiation time.
///
/// Two of these fields say how WRONG the club could be rather than what it
/// thinks. `confidence` is how well its scouts know him; `league_gap` is
/// how far the football he has played sits below the football he is being
/// bought for. The board discounts the headline number by both — see
/// [`crate::club::board::mandate::doctrine::BeliefRisk`].
#[derive(Debug, Clone, Copy)]
pub struct TargetBelief {
    pub group: PlayerFieldPositionGroup,
    pub tier: BriefTier,
    pub believed_level: f32,
    /// Believed ability of the man whose shirt the mandate is about. Zero
    /// for an empty shirt.
    pub incumbent_level: f32,
    /// What a free body gives in this shirt at this club — the club's own
    /// rotation floor. The bottom of every contribution.
    pub replacement_level: f32,
    pub believed_ceiling: f32,
    /// 0..1 distance between the buyer's league and the seller's.
    pub league_gap: f32,
    /// 0..1 — how sure the club is of the headline number.
    pub confidence: f32,
    pub age: u8,
    pub annual_wage: f64,
}

/// The buyer's side of the market: what an upgrade is worth, what it may
/// cost, and what to open at.
pub struct UpgradeMath;

impl UpgradeMath {
    /// Agent and intermediary cost as a share of the fee. A real,
    /// unavoidable part of every deal, and one the buyer prices in.
    const AGENT_SHARE: f64 = 0.08;
    /// Age from which a player stops being a re-sellable asset and starts
    /// being a wage. Continuous — the resale curve fades toward it rather
    /// than stepping.
    const RESALE_END_AGE: f32 = 32.0;
    /// Age at which resale value peaks: he has proved himself and still has
    /// most of a career left.
    const RESALE_PEAK_AGE: f32 = 23.0;
    /// Most of a fee a club can expect to recover on a young player it
    /// bought well. Never 1.0 — a club that recovered everything would
    /// treat players as free.
    const RESALE_MAX: f64 = 0.75;
    /// Extra resale a clearly-unfulfilled ceiling buys, per believed point
    /// of headroom, saturating at [`Self::RESALE_HEADROOM_MAX`].
    const RESALE_HEADROOM_PER_POINT: f64 = 0.012;
    const RESALE_HEADROOM_MAX: f64 = 0.18;
    /// Longest contract, in years, a club writes for an arriving player.
    const MAX_CONTRACT_YEARS: f64 = 5.0;
    const MIN_CONTRACT_YEARS: f64 = 2.0;

    pub fn agent_fee(fee: f64) -> f64 {
        fee * Self::AGENT_SHARE
    }

    /// The best observable level a club already fields in one position
    /// group — the fallback incumbent when no brief names the shirt.
    ///
    /// Only a fallback: the brief's slot carries the level of the man who
    /// actually wears the shirt being shopped for, and that is the honest
    /// comparison. Against the group's BEST, a second centre-back always
    /// reads as a downgrade.
    ///
    /// Reads [`AbilityEstimator::observable_level`], never hidden ability:
    /// the buying club's own coach is judging his own players by what he
    /// sees them do, which is the same currency the scout's assessment of
    /// the target is denominated in.
    pub(in crate::transfers) fn incumbent_level(
        club: &Club,
        group: PlayerFieldPositionGroup,
    ) -> f32 {
        club.teams
            .main()
            .or_else(|| club.teams.teams.first())
            .map(|team| {
                team.players
                    .players
                    .iter()
                    .filter(|p| !p.is_on_loan() && p.position().position_group() == group)
                    .map(AbilityEstimator::observable_level)
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0) as f32
    }

    /// Share of the shirt at which the man a signing displaces stops being
    /// a spare body and becomes the incumbent.
    const STARTER_SHARE: f64 = 0.8;
    /// How far below its first choice a club's own deputy sits, in
    /// observable points. The same band the squad model already uses to
    /// separate a first-team regular from a key man: a bench is not made
    /// of replacement-level bodies, and pricing it as though it were made
    /// every cover signing look like a bargain worth a starter's fee.
    const DEPTH_DROP: f32 = ClubLevelAnchor::REGULAR_BAND as f32;

    /// Whose minutes he takes.
    ///
    /// This is the one term that used to be missing, and the reason a
    /// 41.6M heir looked exactly like a 41.6M starter. A man promised the
    /// shirt takes it off the incumbent, so what he adds is the
    /// improvement on the incumbent. A man promised a fifth of the season
    /// takes those minutes off the club's own deputy, so what he adds is
    /// the improvement on him. Everything in between is the same line,
    /// read at the share the mandate actually promised.
    fn displaced_level(inputs: &DealInputs) -> f32 {
        let top = inputs.incumbent_level.max(inputs.replacement_level);
        let deputy = (top - Self::DEPTH_DROP).max(inputs.replacement_level);
        let claim = (inputs.minutes.opening_share() as f64 / Self::STARTER_SHARE).clamp(0.0, 1.0);
        deputy + (top - deputy) * claim as f32
    }

    /// Value the deal.
    pub fn evaluate(inputs: &DealInputs) -> DealValue {
        let money_utility = MoneyUtility::of(inputs.idle_cash, inputs.annual_income).value();
        let point_value = PointValue::of(inputs.annual_income).per_season();
        let gain = inputs.believed_level - inputs.incumbent_level;
        let contribution = (inputs.believed_level - Self::displaced_level(inputs)).max(0.0);
        let years_of_use = Self::years_of_use(inputs.age);
        // A step-up buy is worth less on resale than the same player bought
        // inside his own market: the buyer may be the only club that
        // believed the number, and the next one will discount it again.
        let resale_fraction = Self::resale_fraction(
            inputs.age,
            years_of_use,
            inputs.believed_ceiling,
            inputs.believed_level,
        ) * (1.0 - 0.5 * inputs.league_gap.clamp(0.0, 1.0) as f64);

        // The sporting benefit: every season he is at the club, the points
        // he brings over the man whose minutes he is taking, weighted by
        // how much of that season the club SAID he would play, by how much
        // of it a man his age can give, and by what the shirt is worth.
        //
        // A signing who brings nothing over the man he displaces earns
        // nothing rather than costing something: a club does not get paid
        // for signing a worse player, it simply has no reason to.
        let role = RoleWeight::of(inputs.group) as f64;
        let objective = Self::objective_utility(inputs.tier);
        let mut benefit = 0.0;
        if contribution > 0.0 {
            let whole_years = years_of_use.floor() as u32;
            let tail = years_of_use - whole_years as f64;
            for year in 0..whole_years {
                let age_then = inputs.age as f32 + year as f32;
                let share = inputs.minutes.share(year.min(u8::MAX as u32) as u8) as f64;
                benefit += contribution as f64
                    * share
                    * Self::usage(age_then)
                    * role
                    * objective
                    * point_value;
            }
            if tail > 0.0 {
                let age_then = inputs.age as f32 + whole_years as f32;
                let share = inputs.minutes.share(whole_years.min(u8::MAX as u32) as u8) as f64;
                benefit += contribution as f64
                    * share
                    * Self::usage(age_then)
                    * role
                    * objective
                    * point_value
                    * tail;
            }
        }

        // The fee at which the deal is exactly break-even.
        //
        //   benefit = (fee·(1 − resale) + wage·years + agent·fee) · utility
        //
        // Every money term carries the same marginal utility — a dollar
        // recovered on resale is worth what a dollar spent on the fee is —
        // so the solve is linear and always monotone in the fee.
        let per_fee = (1.0 - resale_fraction) + Self::AGENT_SHARE;
        let wage_cost = inputs.annual_wage.max(0.0) * years_of_use;
        let ceiling_fee = if per_fee <= 0.0 {
            0.0
        } else {
            ((benefit / money_utility) - wage_cost).max(0.0) / per_fee
        };

        DealValue {
            benefit,
            years_of_use,
            resale_fraction,
            money_utility,
            ceiling_fee,
            gain,
            contribution,
        }
    }

    /// Opening offer as a share of the seller's asking price.
    ///
    /// A club opens low and closes toward the ask as the window runs out —
    /// and opens higher for the signing it actually needs. `days_left_frac`
    /// is 1.0 on the first day of the window and 0.0 on the last.
    pub fn open_ratio(tier: BriefTier, days_left_frac: f32) -> f64 {
        let t = (1.0 - days_left_frac.clamp(0.0, 1.0)) as f64;
        let (start, end) = match tier {
            BriefTier::A => (0.85, 1.00),
            BriefTier::B => (0.75, 0.90),
            // Cover is bought cheap or not at all; there is always another
            // body.
            BriefTier::C => (0.60, 0.60),
        };
        start + (end - start) * t
    }

    /// How much of a season a player of this age is expected to contribute.
    /// Peaks across the prime, ramps in through the early twenties and
    /// falls away rather than stopping.
    fn usage(age: f32) -> f64 {
        let a = age as f64;
        if a < 18.0 {
            0.25
        } else if a < 24.0 {
            0.55 + (a - 18.0) * 0.075
        } else if a <= 29.0 {
            1.0
        } else if a <= 35.0 {
            (1.0 - (a - 29.0) * 0.13).max(0.15)
        } else {
            0.15
        }
    }

    /// Years the buyer expects to get out of him: the contract it would
    /// write, bounded by how long he stays worth picking.
    fn years_of_use(age: u8) -> f64 {
        let contract = if age <= 24 {
            Self::MAX_CONTRACT_YEARS
        } else if age <= 28 {
            4.0
        } else if age <= 31 {
            3.0
        } else {
            Self::MIN_CONTRACT_YEARS
        };
        // Years before he is no longer a first-team footballer at all.
        let playing_years = (36.0 - age as f64).max(1.0);
        contract.min(playing_years)
    }

    /// Share of the fee the club expects to recover when he leaves.
    ///
    /// A 21-year-old bought for a real fee is an asset the club expects to
    /// sell on; a 30-year-old is a cost it expects to write off. The
    /// believed ceiling adds to it — the upside is exactly what a
    /// develop-and-sell club is buying.
    fn resale_fraction(
        age: u8,
        years_of_use: f64,
        believed_ceiling: f32,
        believed_level: f32,
    ) -> f64 {
        let age_at_exit = age as f32 + years_of_use as f32;
        let base = if age_at_exit <= Self::RESALE_PEAK_AGE {
            Self::RESALE_MAX
        } else {
            let fade = ((age_at_exit - Self::RESALE_PEAK_AGE)
                / (Self::RESALE_END_AGE - Self::RESALE_PEAK_AGE))
                .clamp(0.0, 1.0) as f64;
            Self::RESALE_MAX * (1.0 - fade)
        };
        let headroom = ((believed_ceiling - believed_level).max(0.0) as f64
            * Self::RESALE_HEADROOM_PER_POINT)
            .min(Self::RESALE_HEADROOM_MAX);
        (base + headroom).clamp(0.0, 0.90)
    }

    /// What the objective adds to a signing's raw sporting value. The
    /// player the club actually needs is worth more than his points say;
    /// the fourth body for the bench is worth less.
    fn objective_utility(tier: BriefTier) -> f64 {
        match tier {
            BriefTier::A => 1.15,
            BriefTier::B => 1.00,
            BriefTier::C => 0.85,
        }
    }
}

#[cfg(test)]
mod money_utility_tests {
    use super::*;

    #[test]
    fn a_club_with_no_spare_cash_values_every_dollar_fully() {
        let u = MoneyUtility::of(0.0, 100_000_000.0);
        assert!((u.value() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn years_of_income_in_the_bank_discount_the_next_dollar() {
        let poor = MoneyUtility::of(0.0, 100_000_000.0).value();
        let comfortable = MoneyUtility::of(100_000_000.0, 100_000_000.0).value();
        let hoarding = MoneyUtility::of(2_000_000_000.0, 100_000_000.0).value();
        assert!(comfortable < poor, "{comfortable} vs {poor}");
        assert!(hoarding < comfortable, "{hoarding} vs {comfortable}");
        assert!(
            hoarding >= MoneyUtility::FLOOR - 1e-9,
            "never below the floor: {hoarding}"
        );
    }

    #[test]
    fn a_club_with_no_revenue_history_fails_closed() {
        // Reading a missing denominator as "infinitely rich" would hand
        // every club in a fresh world the cheapest money in the game.
        assert!((MoneyUtility::of(1_000_000_000.0, 0.0).value() - 1.0).abs() < 1e-9);
    }
}

#[cfg(test)]
mod deal_value_tests {
    use super::*;
    use crate::club::board::mandate::MandatePurpose;
    use chrono::NaiveDate;

    /// Two clubs shopping for the same marginal upgrade: one sitting on
    /// years of income, one living hand to mouth. Same player, same wage,
    /// same improvement.
    struct Fx;

    impl Fx {
        fn curve(purpose: MandatePurpose) -> MinutesCurve {
            MinutesCurve::for_purpose(
                purpose,
                PlayerFieldPositionGroup::Midfielder,
                NaiveDate::from_ymd_opt(2028, 7, 1).unwrap(),
                25,
            )
        }

        fn deal(annual_income: f64, idle_cash: f64, gain: f32, age: u8) -> DealInputs {
            DealInputs {
                group: PlayerFieldPositionGroup::Midfielder,
                tier: BriefTier::B,
                believed_level: 140.0,
                incumbent_level: 140.0 - gain,
                replacement_level: 110.0,
                believed_ceiling: 145.0,
                league_gap: 0.0,
                minutes: Self::curve(MandatePurpose::Starter),
                age,
                annual_wage: annual_income * 0.01,
                annual_income,
                idle_cash,
            }
        }
    }

    #[test]
    fn the_richer_club_will_pay_more_for_the_same_upgrade() {
        let income = 400_000_000.0;
        let lean = UpgradeMath::evaluate(&Fx::deal(income, 0.0, 4.0, 25));
        let flush = UpgradeMath::evaluate(&Fx::deal(income, income * 3.0, 4.0, 25));
        assert!(
            flush.ceiling_fee > lean.ceiling_fee * 1.5,
            "idle cash must move the ceiling: {} vs {}",
            flush.ceiling_fee,
            lean.ceiling_fee
        );
    }

    #[test]
    fn a_bigger_improvement_is_worth_a_bigger_fee() {
        let income = 400_000_000.0;
        let small = UpgradeMath::evaluate(&Fx::deal(income, 0.0, 2.0, 25));
        let large = UpgradeMath::evaluate(&Fx::deal(income, 0.0, 10.0, 25));
        assert!(large.ceiling_fee > small.ceiling_fee);
    }

    #[test]
    fn no_improvement_and_no_minutes_is_worth_no_fee() {
        // Re-pinned from `no_improvement_is_worth_no_fee`: a zero gain is
        // no longer a zero deal, because a man bought for a bench purpose
        // is not bought to be better than the incumbent. What is still
        // worth nothing is a man who brings nothing over the body he
        // displaces.
        let mut inputs = Fx::deal(400_000_000.0, 0.0, 0.0, 25);
        inputs.replacement_level = inputs.incumbent_level;
        let deal = UpgradeMath::evaluate(&inputs);
        assert_eq!(deal.benefit, 0.0);
        assert_eq!(deal.ceiling_fee, 0.0);
    }

    #[test]
    fn a_bench_purpose_is_priced_against_the_club_s_own_deputy() {
        // Cover, an heir and a prospect are bought precisely because they
        // are NOT better than the man in the shirt — which the old model
        // answered by pricing them at nothing, leaving the seller's ask as
        // the only number in the room. They are still measured against
        // somebody: the deputy whose minutes they are taking.
        let mut inputs = Fx::deal(400_000_000.0, 0.0, -4.0, 24);
        inputs.minutes = Fx::curve(MandatePurpose::Cover);
        let cover = UpgradeMath::evaluate(&inputs);
        assert!(cover.gain < 0.0);
        assert!(cover.contribution > 0.0);
        assert!(cover.benefit > 0.0, "{}", cover.benefit);

        // The same man asked to take the shirt improves on nobody.
        inputs.minutes = Fx::curve(MandatePurpose::Starter);
        assert_eq!(UpgradeMath::evaluate(&inputs).contribution, 0.0);
    }

    #[test]
    fn the_same_man_on_a_bench_promise_is_worth_less_than_on_a_shirt() {
        let starter = UpgradeMath::evaluate(&Fx::deal(400_000_000.0, 0.0, 6.0, 25));
        let mut cover_inputs = Fx::deal(400_000_000.0, 0.0, 6.0, 25);
        cover_inputs.minutes = Fx::curve(MandatePurpose::Cover);
        let cover = UpgradeMath::evaluate(&cover_inputs);
        // Not a fraction of a fraction: a deputy who plays a quarter of
        // the season is still improving on the club's OWN deputy, not on
        // a free body. What he must not be is priced like the man in the
        // shirt.
        assert!(
            cover.ceiling_fee < starter.ceiling_fee * 0.7,
            "cover {} vs starter {}",
            cover.ceiling_fee,
            starter.ceiling_fee
        );
    }

    #[test]
    fn the_same_player_is_worth_less_the_older_he_is() {
        let income = 400_000_000.0;
        let young = UpgradeMath::evaluate(&Fx::deal(income, 0.0, 6.0, 23));
        let old = UpgradeMath::evaluate(&Fx::deal(income, 0.0, 6.0, 31));
        assert!(
            young.ceiling_fee > old.ceiling_fee * 2.0,
            "fewer usable years and no resale: {} vs {}",
            young.ceiling_fee,
            old.ceiling_fee
        );
    }

    #[test]
    fn a_poor_club_cannot_reach_a_rich_club_s_price_for_the_same_player() {
        let rich = UpgradeMath::evaluate(&Fx::deal(600_000_000.0, 1_500_000_000.0, 4.0, 24));
        let modest = UpgradeMath::evaluate(&Fx::deal(40_000_000.0, 0.0, 4.0, 24));
        assert!(
            rich.ceiling_fee > modest.ceiling_fee * 10.0,
            "the whole reason a market has ladders: {} vs {}",
            rich.ceiling_fee,
            modest.ceiling_fee
        );
    }

    #[test]
    fn a_step_up_buy_recovers_less_of_its_fee() {
        let home = UpgradeMath::evaluate(&Fx::deal(400_000_000.0, 0.0, 6.0, 23));
        let mut abroad_inputs = Fx::deal(400_000_000.0, 0.0, 6.0, 23);
        abroad_inputs.league_gap = 1.0;
        let abroad = UpgradeMath::evaluate(&abroad_inputs);
        assert!(abroad.resale_fraction < home.resale_fraction);
        assert!(abroad.ceiling_fee < home.ceiling_fee);
    }

    #[test]
    fn the_opening_offer_walks_toward_the_ask_as_the_window_closes() {
        let early = UpgradeMath::open_ratio(BriefTier::A, 1.0);
        let late = UpgradeMath::open_ratio(BriefTier::A, 0.0);
        assert!(late > early);
        assert!(UpgradeMath::open_ratio(BriefTier::B, 1.0) < early);
    }

    #[test]
    fn the_ceiling_reaches_this_market_s_fee_scale() {
        // Re-pinned from `wages_dominate_the_ceiling_at_this_market_s_fee_scale`.
        // Same fixture, new calibration: at the old INCOME_SHARE the wages
        // ate the benefit and the ceiling landed near $2.6M, below the
        // $5-15M an elite club's surplus player is actually asked for — so
        // the resolver could only let it cap the extension ABOVE the
        // seller's reach. It is now the buyer's walk-away for every deal,
        // so it has to reach that band on its own.
        let deal = UpgradeMath::evaluate(&DealInputs {
            group: PlayerFieldPositionGroup::Midfielder,
            tier: BriefTier::B,
            believed_level: 146.0,
            incumbent_level: 140.0,
            replacement_level: 120.0,
            believed_ceiling: 146.0,
            league_gap: 0.0,
            minutes: Fx::curve(MandatePurpose::Starter),
            age: 27,
            annual_wage: 3_000_000.0,
            annual_income: 100_000_000.0,
            idle_cash: 0.0,
        });
        assert!(
            deal.benefit > 18_000_000.0 && deal.benefit < 24_000_000.0,
            "{}",
            deal.benefit
        );
        assert!(
            deal.ceiling_fee > 5_000_000.0 && deal.ceiling_fee < 15_000_000.0,
            "the walk-away has to sit inside the band this market's asks do: {}",
            deal.ceiling_fee
        );
    }

    #[test]
    fn value_is_positive_below_the_ceiling_and_negative_above_it() {
        let inputs = Fx::deal(400_000_000.0, 0.0, 6.0, 25);
        let deal = UpgradeMath::evaluate(&inputs);
        assert!(deal.ceiling_fee > 0.0);
        assert!(deal.at_fee(deal.ceiling_fee * 0.5, inputs.annual_wage) > 0.0);
        assert!(deal.at_fee(deal.ceiling_fee * 1.5, inputs.annual_wage) < 0.0);
        assert!(
            deal.at_fee(deal.ceiling_fee, inputs.annual_wage).abs() < deal.benefit * 1e-6,
            "the ceiling is where the deal is exactly break-even"
        );
    }
}
