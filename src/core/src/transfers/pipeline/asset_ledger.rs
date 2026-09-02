//! L4 — the club as a business: what every player is worth to sell, and
//! which of them the club would actually sell.
//!
//! The market had a buying side and almost no selling side. A solvent club
//! never parted with a core player; asking prices were one flat multiplier
//! over market value plus a distress discount; and "available" meant a
//! public listing, which is a state real clubs almost never put a good
//! player in. What real clubs live in is the state in between: *we are not
//! selling him, but there is a number.*
//!
//! Two things live here.
//!
//! **The ledger.** Every contracted player carries an asking price built
//! from three continuous curves — what he is to the side (role premium),
//! how long the club still controls him (contract runway), and where he is
//! on his own career arc (age trajectory). It replaces the flat multiplier
//! for seller-advertised and unsolicited approaches alike.
//! [`super::super::super::country::result::transfers::negotiations`]'s
//! `SellerFeeFloor` stays the absolute floor underneath it.
//!
//! **The sell list.** A continuous score, not a flag, over the six reasons
//! clubs actually sell: a peak-value asset with decline ahead, a contract
//! running down that will not be renewed, a wage bill at its ceiling, money
//! needed for the brief, a player the plan has no room for, and a player
//! pushing to leave. A listed entry is *marketed, not listed* — it becomes a
//! Soft availability signal to buyers in the right band and nothing else: no
//! `Lst` status, no public event, no asset-protection override.

use chrono::NaiveDate;

use crate::club::team::squad::SquadAssetClass;
use crate::{ClubPhilosophy, PlayerFieldPositionGroup};

/// Why a club would sell a player. Ordered by how loudly it says so, which
/// is only used to name the strongest motive on an entry — the score itself
/// is the sum of all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SellMotive {
    /// He is at the top of his value and the curve goes down from here.
    PeakValue,
    /// The contract is running out and the club cannot or will not renew.
    ExpiringRunway,
    /// The wage bill is at its ceiling and his is the wage that frees it.
    WageRelief,
    /// The brief wants more money than the budget holds.
    CashNeed,
    /// The plan has no shirt for him.
    SurplusByPlan,
    /// He wants to go.
    PlayerPushing,
}

/// One player as the selling club sees him.
#[derive(Debug, Clone, Copy)]
pub struct AssetRow {
    pub player_id: u32,
    pub group: PlayerFieldPositionGroup,
    pub age: u8,
    /// Whole months left on his contract. `None` = no contract.
    pub contract_months_remaining: Option<i32>,
    /// Public market value — the number the asking price is built on.
    pub estimated_value: f64,
    /// Annual wage.
    pub annual_wage: f64,
    /// What he is to this side.
    pub asset_class: SquadAssetClass,
    /// Observable level, and the squad's own average, so the role premium
    /// can tell a talisman from a core journeyman.
    pub observable_level: u8,
    pub squad_average_level: u8,
    /// Believed ceiling — drives the prospect premium.
    pub believed_ceiling: u8,
    /// 0-indexed rank in his own position group.
    pub group_rank: u8,
    /// He has formally asked to leave.
    pub is_transfer_requested: bool,
    /// How strongly he is drawn to a bigger stage, 0..1.
    pub stage_pull: f32,
    /// True while a renewal has been refused or is out of reach.
    pub renewal_blocked: bool,
    /// True while a recent signing is still protected from resale.
    pub signing_protected: bool,
    /// True while the player is unavailable for reasons that make a sale
    /// impossible right now (long-term injury).
    pub unsellable: bool,
}

/// What the club is, financially and by plan, when it prices its squad.
#[derive(Debug, Clone)]
pub struct LedgerContext {
    pub philosophy: ClubPhilosophy,
    /// Annual wage bill and the board's wage mandate.
    pub annual_wages: f64,
    pub wage_budget: f64,
    /// What the recruitment brief wants, and what the club has to spend.
    pub brief_envelope: f64,
    pub available_budget: f64,
    /// Senior squad size against the board's registered cap.
    pub squad_size: usize,
    pub max_squad_size: usize,
    /// True when the club's balance is negative.
    pub in_debt: bool,
}

/// One player the club would sell, at a price.
#[derive(Debug, Clone)]
pub struct SellListEntry {
    pub player_id: u32,
    /// What the club would want for him.
    pub asking: f64,
    /// 0..1 readiness. Above [`AssetLedger::MARKET_BAR`] the entry is
    /// marketed to buyers in the right band.
    pub score: f32,
    /// The single loudest reason, for the diagnostics and the UI.
    pub motive: SellMotive,
    pub marked_on: NaiveDate,
}

impl SellListEntry {
    /// True when the club is ready enough to answer a call about him.
    pub fn is_marketed(&self) -> bool {
        self.score >= AssetLedger::MARKET_BAR
    }
}

/// Can the seller replace him with the money?
///
/// The question a selling club actually asks, and the one the model had no
/// term for: a fee is only worth taking if it buys a successor. A club with a
/// ready replacement on its own board sells at its asking price; one with
/// nobody to spend the money on holds out, and holds out harder the later in
/// the window it gets — because a sale it cannot reinvest is a squad hole it
/// will carry all season.
///
/// This is also one of the two things that bound the DRAIN: without it, a
/// buyer whose marginal dollar is worth a third of the seller's could simply
/// strip an exporting league bare.
pub struct ReplacementScarcity;

impl ReplacementScarcity {
    /// Most the reservation rises when the proceeds cannot buy a successor.
    pub const MAX_LIFT: f64 = 0.20;
    /// Additional lift on the last days of the window, when there is no
    /// longer time to find one.
    pub const DEADLINE_LIFT: f64 = 0.15;

    /// The seller's reservation lift. `has_successor` is whether the club's
    /// own watchlist carries a name at the same position it could fund with
    /// this fee; `importance` is the seller-side 0..1 read of what the
    /// player is to the side; `deadline_pressure` is 0..1 from
    /// [`super::auction::DeadlineWindow`].
    ///
    /// Scaled by importance because scarcity is about the HOLE he leaves:
    /// a club needs a successor for its first-choice centre-back, not for
    /// its fifth. The first census applied the full lift to every domestic
    /// sale — a squad man the club was glad to move on got the same +0.20
    /// as its captain — and that alone priced a window's worth of ordinary
    /// sales out of their buyers' reach. The caller also skips the term
    /// entirely for a player the club has already decided to sell (listed,
    /// requested, marketed on its own ledger): a decision to sell is a
    /// decision that the hole is acceptable.
    pub fn lift(has_successor: bool, importance: f32, deadline_pressure: f32) -> f64 {
        if has_successor {
            return 0.0;
        }
        let weight = importance.clamp(0.0, 1.0) as f64;
        (Self::MAX_LIFT + Self::DEADLINE_LIFT * deadline_pressure as f64) * weight
    }
}

/// Accumulates one player's sell-list terms and remembers which of them
/// spoke loudest.
///
/// The terms ADD — a 29-year-old on a year's contract at a club that needs
/// money is more sellable than any one of those reasons alone, which is
/// exactly how real sales cluster. The loudest is kept only so the entry can
/// name a motive for the diagnostics and the UI.
struct SellScore {
    total: f32,
    loudest: f32,
    motive: SellMotive,
}

impl SellScore {
    fn new() -> Self {
        SellScore {
            total: 0.0,
            loudest: 0.0,
            motive: SellMotive::SurplusByPlan,
        }
    }

    fn add(&mut self, term: f32, motive: SellMotive) {
        if term <= 0.0 {
            return;
        }
        self.total += term;
        if term > self.loudest {
            self.loudest = term;
            self.motive = motive;
        }
    }
}

/// Prices a squad and decides who the club would sell.
pub struct AssetLedger;

impl AssetLedger {
    // ── Role premium ─────────────────────────────────────────────
    /// A core player commands a premium over his market value because the
    /// club does not want to sell him — the band widens with how far above
    /// his own squad he is, so a talisman costs more than a core journeyman.
    const CORE_PREMIUM_MIN: f64 = 1.6;
    const CORE_PREMIUM_MAX: f64 = 2.4;
    /// Ability points above the squad average at which the core premium
    /// saturates.
    const CORE_SPAN: f32 = 20.0;
    const FIRST_TEAM_PREMIUM: f64 = 1.3;
    const ROTATION_PREMIUM: f64 = 1.0;
    const SURPLUS_PREMIUM: f64 = 0.8;
    /// A prospect is priced on what he might become, so his premium is the
    /// widest of all and scales with the believed headroom.
    const PROSPECT_PREMIUM_MIN: f64 = 2.0;
    const PROSPECT_PREMIUM_MAX: f64 = 3.0;
    /// Believed headroom over current level at which the prospect premium
    /// saturates.
    const PROSPECT_SPAN: f32 = 25.0;

    // ── Contract runway ──────────────────────────────────────────
    /// Months of contract at or above which the club is in full control and
    /// asks the whole price.
    const RUNWAY_FULL_MONTHS: f64 = 36.0;
    /// Runway at which the price has halved — six months out, the buyer can
    /// simply wait for a pre-contract.
    const RUNWAY_HALF_MONTHS: f64 = 6.0;
    const RUNWAY_FLOOR: f64 = 0.45;

    // ── Sell list ────────────────────────────────────────────────
    /// Age from which the planned peak-value sale curve starts. Goalkeepers
    /// hold value years longer, which is why they are sold years later.
    pub const PEAK_SALE_AGE_OUTFIELD: u8 = 28;
    pub const PEAK_SALE_AGE_KEEPER: u8 = 31;
    /// Contract runway below which an unrenewed player is marketed rather
    /// than walked to a free transfer.
    pub const RUNWAY_SELL_MONTHS: i32 = 18;
    /// Share of the wage mandate at which a solvent club starts wanting the
    /// wage off the books.
    const WAGE_PRESSURE_BAR: f64 = 0.95;
    /// Readiness at or above which the club will answer a call.
    pub const MARKET_BAR: f32 = 0.35;
    /// Longest sell list the club carries. A window is a few decisions, and
    /// a club marketing a dozen players is in a fire sale, not a plan.
    const MAX_ENTRIES: usize = 6;

    /// What the club would want for this player.
    ///
    /// Replaces the flat `value × distress` multiplier for every path that
    /// needs a seller's number: the club's own advertised price and an
    /// unsolicited approach alike. The distress discount still applies
    /// downstream — this is what the club wants, not what it will take.
    pub fn asking_for(row: &AssetRow) -> f64 {
        row.estimated_value.max(0.0)
            * Self::role_premium(row)
            * Self::runway_curve(row.contract_months_remaining)
            * Self::age_trajectory(row.age, row.group)
    }

    /// The premium the shirt he wears puts on his price.
    fn role_premium(row: &AssetRow) -> f64 {
        match row.asset_class {
            SquadAssetClass::CorePlayer => {
                let above = (row.observable_level as f32 - row.squad_average_level as f32).max(0.0)
                    / Self::CORE_SPAN;
                Self::CORE_PREMIUM_MIN
                    + (Self::CORE_PREMIUM_MAX - Self::CORE_PREMIUM_MIN)
                        * above.clamp(0.0, 1.0) as f64
            }
            SquadAssetClass::FirstTeamUseful => Self::FIRST_TEAM_PREMIUM,
            SquadAssetClass::RotationUseful => Self::ROTATION_PREMIUM,
            SquadAssetClass::ProspectDevelopment => {
                let headroom = (row.believed_ceiling as f32 - row.observable_level as f32).max(0.0)
                    / Self::PROSPECT_SPAN;
                Self::PROSPECT_PREMIUM_MIN
                    + (Self::PROSPECT_PREMIUM_MAX - Self::PROSPECT_PREMIUM_MIN)
                        * headroom.clamp(0.0, 1.0) as f64
            }
            SquadAssetClass::TrueSurplus => Self::SURPLUS_PREMIUM,
            // Not enough signal to place him: price him as depth, which is
            // the conservative read in both directions.
            SquadAssetClass::UnknownNeedsEvaluation => Self::ROTATION_PREMIUM,
        }
    }

    /// How much of the price the club can still command given how long it
    /// controls him. A buyer facing three years of contract pays the whole
    /// number; one facing six months knows he can wait.
    fn runway_curve(contract_months_remaining: Option<i32>) -> f64 {
        let Some(months) = contract_months_remaining else {
            // No contract at all — there is nothing to sell.
            return 0.0;
        };
        let m = (months.max(0) as f64).min(Self::RUNWAY_FULL_MONTHS);
        if m >= Self::RUNWAY_FULL_MONTHS {
            return 1.0;
        }
        if m <= Self::RUNWAY_HALF_MONTHS {
            // Below six months the price keeps falling toward the floor
            // rather than stepping — the pre-contract path is what a buyer
            // is really weighing against.
            let t = (m / Self::RUNWAY_HALF_MONTHS).clamp(0.0, 1.0);
            return Self::RUNWAY_FLOOR + (0.5 - Self::RUNWAY_FLOOR) * t;
        }
        let t =
            (m - Self::RUNWAY_HALF_MONTHS) / (Self::RUNWAY_FULL_MONTHS - Self::RUNWAY_HALF_MONTHS);
        0.5 + 0.5 * t
    }

    /// Where he is on his own arc. A 20-year-old is priced on what he will
    /// be; a 33-year-old on what is left.
    fn age_trajectory(age: u8, group: PlayerFieldPositionGroup) -> f64 {
        let peak = Self::peak_sale_age(group) as f32;
        let a = age as f32;
        if a <= 24.0 {
            // Rising: the market pays for the years ahead.
            1.0 + ((24.0 - a).clamp(0.0, 6.0) as f64) * 0.05
        } else if a <= peak {
            1.0
        } else {
            (1.0 - ((a - peak) as f64) * 0.13).max(0.25)
        }
    }

    pub fn peak_sale_age(group: PlayerFieldPositionGroup) -> u8 {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => Self::PEAK_SALE_AGE_KEEPER,
            _ => Self::PEAK_SALE_AGE_OUTFIELD,
        }
    }

    /// Build the club's sell list.
    ///
    /// Every score is a continuous term and they add — a 29-year-old on a
    /// year's contract at a club that needs money is more sellable than any
    /// one of those reasons alone, which is exactly how real sales cluster.
    /// Asset protection is NOT overridden here: an entry is acceptance
    /// readiness, and the auto-listing sweeps keep their own vetoes.
    pub fn build(rows: &[AssetRow], ctx: &LedgerContext, date: NaiveDate) -> Vec<SellListEntry> {
        let wage_pressure = if ctx.wage_budget > 0.0 {
            ((ctx.annual_wages / ctx.wage_budget - Self::WAGE_PRESSURE_BAR)
                / (1.0 - Self::WAGE_PRESSURE_BAR).max(0.05))
            .clamp(0.0, 1.0)
        } else {
            0.0
        };
        // The brief wants more than the budget holds — and a club running a
        // negative balance has a standing cash need whatever its brief says,
        // which is what makes a distressed seller answer the phone.
        let brief_shortfall =
            if ctx.brief_envelope > 0.0 && ctx.available_budget < ctx.brief_envelope {
                ((ctx.brief_envelope - ctx.available_budget) / ctx.brief_envelope).clamp(0.0, 1.0)
            } else {
                0.0
            };
        let cash_need = brief_shortfall.max(if ctx.in_debt { 0.5 } else { 0.0 });
        let roster_pressure = if ctx.max_squad_size > 0 && ctx.squad_size > ctx.max_squad_size {
            ((ctx.squad_size - ctx.max_squad_size) as f64 / 5.0).clamp(0.0, 1.0)
        } else {
            0.0
        };
        // A develop-and-sell club is BUILT to trade: it weights the two
        // asset motives — peak value and a running-down contract — harder
        // than a club whose plan is to keep its best players.
        let trader = matches!(ctx.philosophy, ClubPhilosophy::DevelopAndSell);
        let asset_weight = if trader { 1.35 } else { 1.0 };

        let mut entries: Vec<SellListEntry> = Vec::new();
        for row in rows {
            if row.unsellable || row.signing_protected {
                continue;
            }
            if row.contract_months_remaining.is_none() {
                continue;
            }

            let mut score = SellScore::new();

            // ── Peak value ──
            // He is at the top of his curve and the club has watched the
            // decline coming. Continuous from the peak age, so nothing
            // happens on a birthday.
            let peak = Self::peak_sale_age(row.group);
            let peak_term = if row.age >= peak {
                (((row.age - peak) as f32 + 1.0) / 4.0).clamp(0.0, 1.0) * 0.45 * asset_weight
            } else {
                0.0
            };
            score.add(peak_term, SellMotive::PeakValue);

            // ── Contract running down with no renewal ──
            // A contract the club will not renew is the loudest ordinary
            // sell signal there is: every month that passes takes value off
            // the asset and moves it closer to walking away for nothing.
            let runway_term = match row.contract_months_remaining {
                Some(months) if months <= Self::RUNWAY_SELL_MONTHS => {
                    let closeness = 1.0 - (months.max(0) as f32 / Self::RUNWAY_SELL_MONTHS as f32);
                    let blocked = if row.renewal_blocked { 1.0 } else { 0.45 };
                    closeness * blocked * 0.75 * asset_weight
                }
                _ => 0.0,
            };
            score.add(runway_term, SellMotive::ExpiringRunway);

            // ── Wage relief at a SOLVENT club ──
            // The distressed case already exists (`WageReliefSale`); this is
            // the ordinary one: the bill is at the mandate, so the highest
            // earner the plan can spare becomes gettable.
            let wage_share = if ctx.annual_wages > 0.0 {
                (row.annual_wage / ctx.annual_wages).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            let wage_term = wage_pressure as f32 * wage_share * 3.0;
            score.add(wage_term.min(0.40), SellMotive::WageRelief);

            // ── The brief needs money ──
            // Everyone on the books is a little more sellable when the club
            // is short of what its own plan asks for; the surplus end of the
            // squad much more so.
            let disposability = match row.asset_class {
                SquadAssetClass::TrueSurplus => 1.0,
                SquadAssetClass::RotationUseful => 0.7,
                SquadAssetClass::UnknownNeedsEvaluation => 0.5,
                SquadAssetClass::ProspectDevelopment => 0.35,
                SquadAssetClass::FirstTeamUseful => 0.3,
                SquadAssetClass::CorePlayer => 0.15,
            };
            score.add(
                (cash_need as f32) * disposability * 0.35,
                SellMotive::CashNeed,
            );

            // ── No shirt for him in the plan ──
            let surplus_term = match row.asset_class {
                SquadAssetClass::TrueSurplus => 0.50,
                SquadAssetClass::RotationUseful if row.group_rank >= 3 => 0.25,
                _ => 0.0,
            } + (roster_pressure as f32) * disposability * 0.30;
            score.add(surplus_term, SellMotive::SurplusByPlan);

            // ── He is pushing ──
            let push_term = if row.is_transfer_requested {
                0.60
            } else {
                (row.stage_pull.clamp(0.0, 1.0) - 0.22).max(0.0) * 0.55
            };
            score.add(push_term, SellMotive::PlayerPushing);

            if score.total <= 0.0 {
                continue;
            }
            entries.push(SellListEntry {
                player_id: row.player_id,
                asking: Self::asking_for(row),
                score: score.total.clamp(0.0, 1.0),
                motive: score.motive,
                marked_on: date,
            });
        }

        entries.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.player_id.cmp(&b.player_id))
        });
        entries.retain(|e| e.is_marketed());
        entries.truncate(Self::MAX_ENTRIES);
        entries
    }
}

#[cfg(test)]
mod asking_price_tests {
    use super::*;

    struct Fx;

    impl Fx {
        fn row(class: SquadAssetClass, age: u8, months: i32) -> AssetRow {
            AssetRow {
                player_id: 1,
                group: PlayerFieldPositionGroup::Midfielder,
                age,
                contract_months_remaining: Some(months),
                estimated_value: 10_000_000.0,
                annual_wage: 1_000_000.0,
                asset_class: class,
                observable_level: 140,
                squad_average_level: 130,
                believed_ceiling: 145,
                group_rank: 0,
                is_transfer_requested: false,
                stage_pull: 0.0,
                renewal_blocked: false,
                signing_protected: false,
                unsellable: false,
            }
        }
    }

    #[test]
    fn a_core_player_costs_more_than_a_surplus_one() {
        let core = AssetLedger::asking_for(&Fx::row(SquadAssetClass::CorePlayer, 26, 48));
        let surplus = AssetLedger::asking_for(&Fx::row(SquadAssetClass::TrueSurplus, 26, 48));
        assert!(core > surplus * 2.0, "{core} vs {surplus}");
    }

    #[test]
    fn a_running_down_contract_halves_the_price() {
        let long = AssetLedger::asking_for(&Fx::row(SquadAssetClass::FirstTeamUseful, 26, 48));
        let short = AssetLedger::asking_for(&Fx::row(SquadAssetClass::FirstTeamUseful, 26, 6));
        assert!(
            (short / long - 0.5).abs() < 0.02,
            "six months out is half the price: {}",
            short / long
        );
    }

    #[test]
    fn the_price_keeps_falling_below_six_months_rather_than_stepping() {
        let six = AssetLedger::asking_for(&Fx::row(SquadAssetClass::FirstTeamUseful, 26, 6));
        let two = AssetLedger::asking_for(&Fx::row(SquadAssetClass::FirstTeamUseful, 26, 2));
        assert!(two < six);
        assert!(two > 0.0);
    }

    #[test]
    fn a_prospect_is_priced_on_what_he_might_become() {
        let mut raw = Fx::row(SquadAssetClass::ProspectDevelopment, 19, 48);
        raw.observable_level = 110;
        raw.believed_ceiling = 160;
        let mut capped = raw;
        capped.believed_ceiling = 112;
        assert!(AssetLedger::asking_for(&raw) > AssetLedger::asking_for(&capped));
    }

    #[test]
    fn the_arc_rises_before_the_prime_and_falls_after_it() {
        let young = AssetLedger::asking_for(&Fx::row(SquadAssetClass::FirstTeamUseful, 20, 48));
        let prime = AssetLedger::asking_for(&Fx::row(SquadAssetClass::FirstTeamUseful, 26, 48));
        let old = AssetLedger::asking_for(&Fx::row(SquadAssetClass::FirstTeamUseful, 33, 48));
        assert!(young > prime);
        assert!(old < prime);
    }

    #[test]
    fn a_keeper_holds_his_price_years_longer_than_an_outfielder() {
        let mut keeper = Fx::row(SquadAssetClass::FirstTeamUseful, 30, 48);
        keeper.group = PlayerFieldPositionGroup::Goalkeeper;
        let outfield = Fx::row(SquadAssetClass::FirstTeamUseful, 30, 48);
        assert!(AssetLedger::asking_for(&keeper) > AssetLedger::asking_for(&outfield));
    }
}

#[cfg(test)]
mod sell_list_tests {
    use super::*;

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap()
        }

        fn ctx() -> LedgerContext {
            LedgerContext {
                philosophy: ClubPhilosophy::Balanced,
                annual_wages: 50_000_000.0,
                wage_budget: 100_000_000.0,
                brief_envelope: 0.0,
                available_budget: 100_000_000.0,
                squad_size: 25,
                max_squad_size: 30,
                in_debt: false,
            }
        }

        fn settled_core(age: u8) -> AssetRow {
            AssetRow {
                player_id: 1,
                group: PlayerFieldPositionGroup::Midfielder,
                age,
                contract_months_remaining: Some(48),
                estimated_value: 30_000_000.0,
                annual_wage: 3_000_000.0,
                asset_class: SquadAssetClass::CorePlayer,
                observable_level: 150,
                squad_average_level: 135,
                believed_ceiling: 152,
                group_rank: 0,
                is_transfer_requested: false,
                stage_pull: 0.0,
                renewal_blocked: false,
                signing_protected: false,
                unsellable: false,
            }
        }
    }

    #[test]
    fn a_settled_core_player_in_his_prime_is_not_marketed() {
        let list = AssetLedger::build(&[Fx::settled_core(25)], &Fx::ctx(), Fx::date());
        assert!(
            list.is_empty(),
            "nothing about him says the club would sell: {list:?}"
        );
    }

    #[test]
    fn the_same_player_past_his_peak_is() {
        let list = AssetLedger::build(&[Fx::settled_core(31)], &Fx::ctx(), Fx::date());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].motive, SellMotive::PeakValue);
    }

    #[test]
    fn a_running_down_contract_the_club_will_not_renew_markets_him() {
        let mut row = Fx::settled_core(27);
        row.contract_months_remaining = Some(9);
        row.renewal_blocked = true;
        let list = AssetLedger::build(&[row], &Fx::ctx(), Fx::date());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].motive, SellMotive::ExpiringRunway);
    }

    #[test]
    fn a_player_asking_to_leave_is_marketed_whatever_else_is_true() {
        let mut row = Fx::settled_core(24);
        row.is_transfer_requested = true;
        let list = AssetLedger::build(&[row], &Fx::ctx(), Fx::date());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].motive, SellMotive::PlayerPushing);
    }

    #[test]
    fn a_recent_signing_is_never_marketed() {
        let mut row = Fx::settled_core(31);
        row.signing_protected = true;
        assert!(AssetLedger::build(&[row], &Fx::ctx(), Fx::date()).is_empty());
    }

    #[test]
    fn a_wage_bill_at_its_ceiling_markets_the_biggest_earner() {
        let mut ctx = Fx::ctx();
        ctx.annual_wages = 100_000_000.0;
        ctx.wage_budget = 100_000_000.0;
        let mut top = Fx::settled_core(26);
        top.annual_wage = 20_000_000.0;
        let mut modest = Fx::settled_core(26);
        modest.player_id = 2;
        modest.annual_wage = 500_000.0;
        let list = AssetLedger::build(&[top, modest], &ctx, Fx::date());
        assert_eq!(list.first().map(|e| e.player_id), Some(1));
        assert_eq!(list[0].motive, SellMotive::WageRelief);
    }

    #[test]
    fn a_develop_and_sell_club_trades_its_peak_assets_harder() {
        let mut trader = Fx::ctx();
        trader.philosophy = ClubPhilosophy::DevelopAndSell;
        let row = Fx::settled_core(31);
        let keeper_club = AssetLedger::build(&[row], &Fx::ctx(), Fx::date());
        let trading_club = AssetLedger::build(&[row], &trader, Fx::date());
        let keeper_score = keeper_club.first().map(|e| e.score).unwrap_or(0.0);
        let trader_score = trading_club.first().map(|e| e.score).unwrap_or(0.0);
        assert!(
            trader_score > keeper_score,
            "{trader_score} vs {keeper_score}"
        );
    }

    #[test]
    fn a_club_short_of_its_own_brief_markets_its_surplus_first() {
        let mut ctx = Fx::ctx();
        ctx.brief_envelope = 60_000_000.0;
        ctx.available_budget = 5_000_000.0;
        let mut surplus = Fx::settled_core(26);
        surplus.asset_class = SquadAssetClass::TrueSurplus;
        surplus.observable_level = 110;
        let mut core = Fx::settled_core(26);
        core.player_id = 2;
        let list = AssetLedger::build(&[surplus, core], &ctx, Fx::date());
        assert_eq!(
            list.first().map(|e| e.player_id),
            Some(1),
            "the surplus man goes before the core one: {list:?}"
        );
    }

    #[test]
    fn a_sell_list_never_grows_into_a_fire_sale() {
        let rows: Vec<AssetRow> = (1..=20)
            .map(|i| {
                let mut r = Fx::settled_core(33);
                r.player_id = i;
                r
            })
            .collect();
        let list = AssetLedger::build(&rows, &Fx::ctx(), Fx::date());
        assert!(list.len() <= AssetLedger::MAX_ENTRIES);
    }
}
