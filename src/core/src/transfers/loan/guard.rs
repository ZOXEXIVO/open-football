//! `LoanAssetGuard` — what a loan COSTS the two clubs that would agree
//! it.
//!
//! Two questions a destination turns on that no relative reading answers:
//! **is this club's own year smaller than the asset it is being handed**,
//! and **can it pay the wage that comes with him**. Both are properties
//! of the *pair*, not of the player: the same man is comfortable at a
//! peer and ruinous at the club one division down.
//!
//! Nothing here refuses a loan. The parent's own position is priced by
//! [`ParentWillingness`] and the money by [`LoanMoney`], both of which
//! live with the agreement; the guard reads the pair and hands those two
//! the numbers only it can see. Everything it reads is observable —
//! [`AbilityEstimator::observable_level`], a valuation, a wage, a
//! reputation. Hidden ability is never read.
//!
//! Shape:
//!
//! * [`LoanAssetGuard`] is built once per player from the PARENT side.
//!   [`LoanAssetGuard::willingness_for`] is the destination-independent
//!   half, which the listing and loan-intent passes consult before any
//!   borrower exists.
//! * [`LoanBorrowerProfile`] is the borrower side, assembled by each call
//!   site from the club it is about to offer him to.
//! * [`LoanAssetGuard::assess`] combines them into a [`LoanGuardVerdict`]
//!   carrying both money terms, the affordability they come to, the
//!   seller's `refusal_delta` and a one-line diagnostic for
//!   `OF_TRACE_PLAYER`.

use crate::transfers::view::player::PlayerView;
use chrono::NaiveDate;

use crate::club::CareerRunway;
use crate::club::player::calculators::WageCalculator;
use crate::club::player::mind::{CareerPlanView, MindClock, MindSituation};
use crate::club::player::statistics::MatchExperienceBackground;
use crate::club::staff::perception::AbilityEstimator;
use crate::transfers::gate::{EffectivePlayerReputation, thresholds};
use crate::transfers::loan::agreement::{
    LoanMoney, MoneyReading, ParentReading, ParentWillingness,
};
use crate::transfers::pipeline::trace::MarketSwitches;
use crate::transfers::squad::SquadReviewPass;
use crate::{Club, ClubLevelAnchor, Person, Player, PlayerFieldPositionGroup, PlayerStatusType};

/// The borrower half of the pair, read off the club a loan is being
/// offered to. Every field is already in hand at every call site.
#[derive(Debug, Clone, Copy)]
pub struct LoanBorrowerProfile {
    /// Annualised trailing income (`ClubFinances::estimated_annual_income`).
    pub income: i64,
    /// Total annual wage bill across the club's squads.
    pub wage_bill: i64,
    /// The best-paid player at the borrower, annual.
    pub top_earner: u32,
    /// Wage budget still unspent, annual. Zero when the club is at or over
    /// its ceiling.
    pub wage_headroom: i64,
    /// Best current ability the borrower already has in the loanee's group.
    pub best_in_group: u8,
    /// What the borrower expects of a starter — the same anchor the buy
    /// side briefs against.
    pub anchor: ClubLevelAnchor,
    /// Main-team world reputation, 0..10000.
    pub world_rep: u16,
    /// Reputation of the competition the borrower plays in. Zero means
    /// "no league", which stands the division half of the peer band down
    /// rather than guessing — same rule the destination-level gate uses.
    pub league_rep: u16,
    /// Reputation reach of the borrower as the player's side reads it —
    /// world standing blended with its league's.
    pub reach: i16,
}

impl LoanBorrowerProfile {
    /// Borrower appetite score the wage split is keyed on: main-team world
    /// reputation on 0..1, the same reading the loan contract builder uses
    /// when it actually writes the split.
    pub fn wage_split_score(&self) -> f32 {
        (self.world_rep as f32 / 10_000.0).clamp(0.0, 1.0)
    }

    /// The group-specific half, folded in by the caller — every loan scan
    /// already holds a per-group depth snapshot of the borrower, so this
    /// stays a copy rather than a second walk of the roster.
    pub fn with_best_in_group(mut self, best_in_group: u8) -> Self {
        self.best_in_group = best_in_group;
        self
    }

    /// Read a borrowing club, once per pass. `league_rep` is the standard
    /// of the competition it plays in, resolved by the caller (which holds
    /// the country's league table); zero when it plays none.
    /// [`Self::with_best_in_group`] adds the per-group half.
    pub fn of(club: &Club, date: NaiveDate, league_rep: u16) -> Option<Self> {
        let team = club.teams.main().or_else(|| club.teams.teams.first())?;
        let wage_bill: i64 = club
            .teams
            .iter()
            .map(|t| t.get_annual_salary() as i64)
            .sum();
        let wage_budget = club
            .finance
            .wage_budget
            .as_ref()
            .map(|b| b.amount as i64)
            .unwrap_or(0);
        let top_earner = club
            .teams
            .iter()
            .flat_map(|t| t.players.iter())
            .filter_map(|p| p.contract.as_ref().map(|c| c.salary))
            .max()
            .unwrap_or(0);
        let world_rep = team.reputation.world;
        Some(LoanBorrowerProfile {
            income: club.finance.estimated_annual_income(date),
            wage_bill,
            top_earner,
            wage_headroom: (wage_budget - wage_bill).max(0),
            best_in_group: 0,
            anchor: ClubLevelAnchor::for_reputation(team.reputation.overall_score()),
            world_rep,
            league_rep,
            reach: (0.70 * world_rep as f32 + 0.30 * league_rep as f32)
                .round()
                .clamp(0.0, 10_000.0) as i16,
        })
    }
}

/// The parent half — everything about the player and the club that owns
/// him. Built once per player per pass and reused across every candidate
/// borrower.
#[derive(Debug, Clone, Copy)]
pub struct LoanAssetGuard {
    /// What he looks like from outside. Never hidden ability: the band
    /// he is placed in, his readiness and the men counted ahead of him
    /// are all readings a staff takes.
    level: u8,
    age: u8,
    group: PlayerFieldPositionGroup,
    parent_anchor: ClubLevelAnchor,
    parent_rank: u8,
    parent_best_in_group: u8,
    parent_league_rep: u16,
    /// Slots the club's own formation starts in this shirt. The
    /// difference between a first choice and a squad man is a place in
    /// the side, not a place in a table of typical depths.
    starting_slots: usize,
    value: f64,
    salary: u32,
    player_requested: bool,
    seller_advertised: bool,
    player_effective_rep: i16,
    listing_resignation: f32,
    /// How firmly he has decided he is dropping a level, 0..1 — the arcs
    /// whose whole point is playing somewhere smaller.
    plan_widening: f32,
}

impl LoanAssetGuard {
    /// Standing at/above which the player is his club's own level — the
    /// top of the readiness scale.
    pub const PEER_STANDING: f32 = 1.0;
    /// Oldest age the development pathway covers. Development now means
    /// "below his club's level", not "young" — the age band is only the
    /// outer bound on it.
    pub const DEVELOPMENT_AGE: u8 = 23;
    /// Years below [`Self::DEVELOPMENT_AGE`] over which the renown band
    /// widens to its full extra allowance.
    const RENOWN_AGE_SPAN: f32 = 7.0;
    /// Extra share of the base step-down band a boy of sixteen is granted
    /// on top of it. Renown widens with youth; it never vanishes.
    const RENOWN_YOUTH_WIDENING: f32 = 0.8;
    /// Assemble the parent side.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        level: u8,
        age: u8,
        group: PlayerFieldPositionGroup,
        parent_anchor: ClubLevelAnchor,
        parent_rank: u8,
        starting_slots: usize,
        parent_best_in_group: u8,
        parent_league_rep: u16,
        value: f64,
        salary: u32,
        player_requested: bool,
        seller_advertised: bool,
        player_effective_rep: i16,
        listing_resignation: f32,
    ) -> Self {
        LoanAssetGuard {
            level,
            age,
            group,
            parent_anchor,
            parent_rank,
            starting_slots,
            parent_best_in_group,
            parent_league_rep,
            value,
            salary,
            player_requested,
            seller_advertised,
            player_effective_rep,
            listing_resignation,
            plan_widening: 0.0,
        }
    }

    /// Build the parent side from the club that owns the player.
    ///
    /// `seller_league_rep` / `seller_club_rep` are the same pair the
    /// valuation and every sell-side reading already resolve
    /// (`PlayerValuationCalculator::seller_context`), so the guard prices
    /// him at exactly the number the rest of the market quotes. `None`
    /// when the player has no contract (a free agent on a roster) or the
    /// club has no squad to read a level from — there is no loan to price.
    pub fn for_player(
        club: &Club,
        player: &Player,
        date: NaiveDate,
        seller_league_rep: u16,
        seller_club_rep: u16,
    ) -> Option<Self> {
        let value = player.value(date, seller_league_rep, seller_club_rep);
        Self::from_parts(club, player, date, seller_league_rep, value)
    }

    /// The shared field build. `value` is passed in because the two
    /// entry points differ only there: the full guard prices him, and the
    /// destination-independent veto never reads the number and must not
    /// pay for a valuation on a per-player-per-day sweep.
    fn from_parts(
        club: &Club,
        player: &Player,
        date: NaiveDate,
        parent_league_rep: u16,
        value: f64,
    ) -> Option<Self> {
        let contract = player.contract.as_ref()?;
        let team = club.teams.main().or_else(|| club.teams.teams.first())?;
        let group = player.position().position_group();
        Some(LoanAssetGuard {
            level: AbilityEstimator::observable_level(player),
            age: player.age(date),
            group,
            parent_anchor: ClubLevelAnchor::for_reputation(team.reputation.overall_score()),
            parent_rank: PlayerView::position_group_rank(club, player.id, group),
            starting_slots: SquadReviewPass::group_min_needed(group, team.tactics().positions())
                .saturating_sub(1)
                .max(1),
            parent_best_in_group: PlayerView::best_observable_in_group(club, group),
            parent_league_rep,
            value,
            salary: contract.salary,
            player_requested: player.statuses.has(PlayerStatusType::Req),
            seller_advertised: player.statuses.has(PlayerStatusType::Loa),
            player_effective_rep: EffectivePlayerReputation::compute(
                player.player_attributes.world_reputation,
                player.player_attributes.current_reputation,
                player.player_attributes.home_reputation,
                true,
            ),
            listing_resignation: player.market_resignation(date),
            plan_widening: player
                .mind
                .career
                .plan_view(MindClock::day(date))
                .renown_widening(),
        })
    }

    /// The parent's own position on lending him out, 0..1 — the
    /// destination-independent half every loan-INTENT pass needs, and
    /// those passes run before any destination exists.
    ///
    /// Fully willing whenever the parent side cannot be read at all, and
    /// on the `OF_LOAN_GUARD_OFF` arm, so it only ever restrains an
    /// intent. The reading it is built from is the one the agreement
    /// prices with, carried whole rather than flattened to a score.
    pub fn willingness_for(club: &Club, player: &Player, date: NaiveDate) -> ParentWillingness {
        if MarketSwitches::loan_guard_off() {
            return ParentWillingness::open();
        }
        // Value and the parent's competition are money / destination
        // terms, which none of the readings below touch — so neither is
        // resolved here, and the half-built guard is used for nothing
        // else. The valuation matters: this runs per player per day on
        // the country listing pass.
        let Some(guard) = Self::from_parts(club, player, date, 0, 0.0) else {
            return ParentWillingness::open();
        };
        ParentWillingness::of(&guard.parent_reading(club, player, date))
    }

    /// Everything the parent can see about him, gathered from the club
    /// that owns him.
    fn parent_reading(&self, club: &Club, player: &Player, date: NaiveDate) -> ParentReading {
        let group = self.group;
        // The FIELDING squad, against the formation's own floor. Counting
        // every roster in the building against a flat `typical_starters`
        // made `depth_room` 1.0 for any club with two teams, so the term
        // said nothing at all — and the reserve keeper it counted is not
        // the man the first team would be short of.
        // Zero means the caller is holding a squad the club's own
        // rosters do not contain, which is no view rather than an empty
        // position group.
        let group_count = club
            .teams
            .main()
            .into_iter()
            .flat_map(|team| team.players.iter())
            .filter(|p| p.position().position_group() == group && !p.is_on_loan())
            .count();
        // A rolling start share is only worth reading once he has played
        // enough for it to mean anything. Before that the club falls back
        // on the season he did play — the ledger's own record — and only
        // then on "no view", which the minutes term reads as a full
        // reason to lend rather than as none: a man nobody has watched is
        // exactly the man a loan is for.
        let starter_share = if player.happiness.appearances_tracked >= MindSituation::TRACKED_APPS {
            player.happiness.starter_ratio
        } else {
            let record = MatchExperienceBackground::from_player(player).recent_start_share;
            if record > 0.0 {
                record
            } else {
                Self::NO_VIEW_SHARE
            }
        };
        let plan = player.plan.as_ref();
        ParentReading {
            first_choice: self.first_choice(),
            starter_share,
            runway: CareerRunway::at(self.age),
            loans_used: plan.map(|p| p.loans_used).unwrap_or(0),
            group_count,
            group_min_needed: club
                .teams
                .main()
                .map(|team| SquadReviewPass::group_min_needed(group, team.tactics().positions()))
                .unwrap_or_else(|| group.typical_starters()),
            rank: self.parent_rank,
            stage: player.pathway_stage(),
            philosophy: club.philosophy,
            plan_push: player
                .mind
                .career
                .plan_view(MindClock::day(date))
                .loan_push(),
            requested: self.player_requested,
            advertised: self.seller_advertised,
        }
    }

    /// Approximate the parent side across a border, from the facts a
    /// cross-country player summary carries.
    ///
    /// Two of them are approximations and are named as such, in the same
    /// spirit as [`super::loan_market`]'s foreign target classifier. The
    /// parent's level anchor is derived from its world reputation alone
    /// (a summary carries no home / national standing), and his rank in
    /// the group is read off whether he IS his club's best there — which
    /// errs toward letting a loan through, never toward blocking one.
    /// Both money terms are exact: value, wage and the borrower's books
    /// are all in hand.
    #[allow(clippy::too_many_arguments)]
    pub fn from_summary(
        level: u8,
        age: u8,
        group: PlayerFieldPositionGroup,
        parent_world_rep: i16,
        parent_best_in_group: u8,
        parent_league_rep: u16,
        value: f64,
        salary: u32,
        seller_advertised: bool,
        player_effective_rep: i16,
    ) -> Self {
        let parent_rank = if parent_best_in_group > 0 && level >= parent_best_in_group {
            0
        } else {
            group.typical_starters().min(u8::MAX as usize) as u8
        };
        LoanAssetGuard {
            level,
            age,
            group,
            parent_anchor: ClubLevelAnchor::for_reputation(
                (parent_world_rep.max(0) as f32 / 10_000.0).clamp(0.0, 1.0),
            ),
            parent_rank,
            // A summary carries no formation, so the typical shape of the
            // shirt stands in — named as the approximation it is, like
            // the two beside it.
            starting_slots: group.typical_starters(),
            parent_best_in_group,
            parent_league_rep,
            value,
            salary,
            player_requested: false,
            seller_advertised,
            player_effective_rep,
            listing_resignation: 0.0,
            plan_widening: 0.0,
        }
    }

    /// The summary path's own plan reading — a borrowing country cannot
    /// reach his mind, so the arc travels on the summary instead.
    pub fn with_plan(mut self, plan: CareerPlanView) -> Self {
        self.plan_widening = plan.renown_widening();
        self
    }

    /// The shirt he is judged in, the club's own level bands and the
    /// standard of its competition — the three readings the
    /// `OF_LOAN_AGREEMENT_OFF` arm rebuilds its peer band from.
    pub(crate) fn group(&self) -> PlayerFieldPositionGroup {
        self.group
    }

    pub(crate) fn parent_anchor(&self) -> ClubLevelAnchor {
        self.parent_anchor
    }

    pub(crate) fn parent_league_rep(&self) -> u16 {
        self.parent_league_rep
    }

    /// The club's own first choice in this shirt, with nobody having
    /// opened the door — his request or a loan listing is his decision
    /// rather than the club's. A reading; the price of it is the
    /// `starter_hold` term.
    pub fn parent_holds(&self) -> bool {
        self.first_choice() && !self.player_requested && !self.seller_advertised
    }

    /// Where he stands at his own club, 0..1.25: 0 at the rotation floor,
    /// 1 at the key-player floor, and above 1 for a man his club's level
    /// does not stretch to.
    pub fn standing(&self) -> f32 {
        let key = self.parent_anchor.key_floor(self.group) as f32;
        let rotation = self.parent_anchor.rotation_floor(self.group) as f32;
        let span = (key - rotation).max(1.0);
        ((self.level as f32 - rotation) / span).clamp(0.0, 1.25)
    }

    /// Start share that stands the minutes term fully up — a man nobody
    /// has watched is exactly the man a loan is for, so "no view" reads
    /// as no football rather than as a full season of it.
    const NO_VIEW_SHARE: f32 = 0.0;

    /// The club's own first choice in this shirt: at its key-player level
    /// AND inside the slots the formation actually starts.
    pub fn first_choice(&self) -> bool {
        self.level as i16 >= self.parent_anchor.key_floor(self.group)
            && (self.parent_rank as usize) < self.starting_slots
    }

    /// Would a loan of this man be a DEVELOPMENT loan — one he needs
    /// because he is below his own club's level — rather than merely a
    /// loan of somebody young?
    ///
    /// The one definition. The foreign sweep read a birth year, the
    /// domestic one read the level, and the guard read both, so the same
    /// player was a prospect on one path and a squad man on another.
    pub fn development_loan(player: &Player, club: &Club, date: NaiveDate) -> bool {
        Self::from_parts(club, player, date, 0, 0.0)
            .map(|guard| guard.is_development())
            .unwrap_or(false)
    }

    /// Development means BELOW HIS CLUB'S LEVEL, not "young". The age band
    /// bounds it — past it a below-level player is a squad player, not a
    /// prospect — but a teenager who is already a first-team regular for
    /// his club is not on a development pathway, and the floors that
    /// pathway lifts must not lift for him.
    pub fn is_development(&self) -> bool {
        self.age <= Self::DEVELOPMENT_AGE
            && (self.level as i16) < self.parent_anchor.regular_floor(self.group)
    }

    /// How ready he already is for his parent's own first team, 0..1 —
    /// measured against the club's own bands rather than against whoever
    /// happens to be the best body in the group.
    pub fn readiness(&self) -> f32 {
        (self.standing() / Self::PEER_STANDING).clamp(0.0, 1.0)
    }

    /// Reputation gap a loan destination may sit below the player's own
    /// standing before he refuses to go.
    ///
    /// Continuous in age and in how long he has been on the market. The
    /// old rule exempted every player at or below the prime-age bar
    /// outright, which is exactly how a nineteen-year-old with a
    /// nine-figure reputation could be offered around the third tier: his
    /// renown was not weighed at all. It is weighed now — a young player's
    /// band is simply wider, because a season in men's football is worth
    /// more to him than his name is.
    pub fn renown_gap_tolerated(age: u8, listing_resignation: f32) -> f32 {
        Self::renown_gap_tolerated_with(age, listing_resignation, 0.0)
    }

    /// The same band, widened by a man's own decision to drop.
    ///
    /// The gap the band measures is a statement about his NAME, and a
    /// player who has decided he is going down a level to play has
    /// already made his peace with what that says about him. Nothing
    /// else in the model could express it: resignation is what months on
    /// the market do TO him, and this is what he has chosen.
    pub fn renown_gap_tolerated_with(age: u8, listing_resignation: f32, plan_widening: f32) -> f32 {
        let youth = ((Self::DEVELOPMENT_AGE.saturating_sub(age)) as f32 / Self::RENOWN_AGE_SPAN)
            .clamp(0.0, 1.0);
        thresholds::REP_STEP_DOWN_GAP as f32 * (1.0 + Self::RENOWN_YOUTH_WIDENING * youth)
            + listing_resignation.clamp(0.0, 1.0) * thresholds::LOAN_RENOWN_RESIGNATION_SPAN
            + plan_widening.clamp(0.0, 1.0) * Self::RENOWN_PLAN_SPAN
    }

    /// How far a man's own plan to drop a level widens his renown band,
    /// at full commitment. A division's worth of reputation.
    const RENOWN_PLAN_SPAN: f32 = 1_500.0;

    /// How far he has lowered his own sights, 0..1.
    #[inline]
    pub fn listing_resignation(&self) -> f32 {
        self.listing_resignation
    }

    /// This player's own renown band, at his age, his market resignation
    /// and the arc he is living out.
    pub fn renown_band(&self) -> f32 {
        Self::renown_gap_tolerated_with(self.age, self.listing_resignation, self.plan_widening)
    }

    /// Price one destination.
    ///
    /// Reads, never refuses. `willingness` is what [`ParentWillingness`]
    /// already made of this player, `parent_subsidy` the share of his
    /// wage the parent means to keep paying — the term that decides what
    /// the borrower is actually left carrying, and the one the
    /// negotiation room has no other way to see.
    ///
    /// Both money terms read 0 — "no objection" — when the borrower's
    /// books cannot be read at all. That is not a nicety: a club has no
    /// income history on the day a world is created, and a guard that read
    /// a missing ledger as "this club earns nothing" would call every loan
    /// in the game unaffordable for the first year.
    pub fn assess(
        &self,
        borrower: &LoanBorrowerProfile,
        willingness: f32,
        parent_subsidy: f32,
    ) -> LoanGuardVerdict {
        let weight = if borrower.income > 0 {
            self.value / borrower.income as f64
        } else {
            0.0
        };
        let carry = self.carry(borrower, parent_subsidy);
        // The fee belongs to the pair the call site is pricing, not to the
        // asset: what the guard can see is what the borrower earns and
        // what it already pays.
        let money = LoanMoney::of(&MoneyReading {
            weight,
            carry,
            asking: 0.0,
            max_loan_fee: 0.0,
            development: parent_subsidy > 0.0,
        });
        let willingness = willingness.clamp(0.0, 1.0);

        LoanGuardVerdict {
            weight,
            carry,
            standing: self.standing(),
            willingness,
            affordability: money.affordability,
            refusal_delta: LoanGuardVerdict::refusal_delta(willingness, money.affordability),
            renown_band: self.renown_band(),
            renown_gap: (self.player_effective_rep - borrower.reach) as f32,
        }
    }

    /// Share of his wage the borrower would pick up, against the most it
    /// could plausibly pay: its unspent wage headroom with the usual
    /// stretch, or — for a club with no headroom at all — what it already
    /// pays its best-paid player. Above one, the deal is a wage the
    /// borrower cannot carry however free the loan is.
    fn carry(&self, borrower: &LoanBorrowerProfile, parent_subsidy: f32) -> f64 {
        // Neither a wage budget nor a single salary on the books: this
        // club's payroll is unknown, not zero. Stand the term down.
        if borrower.wage_headroom <= 0 && borrower.top_earner == 0 {
            return 0.0;
        }
        let (borrower_wage, _) = WageCalculator::loan_wage_split_v2(
            self.salary,
            borrower.wage_split_score(),
            parent_subsidy,
        );
        let ceiling = (borrower.wage_headroom.max(0) as f64 * 1.30)
            .max(borrower.top_earner as f64 * 1.50)
            .max(1.0);
        borrower_wage as f64 / ceiling
    }

    /// Total wage bill is carried for the diagnostics line only — the
    /// verdict never reads it directly, but "how big is this club's
    /// payroll" is the first question asked of a surprising `carry`.
    pub fn diagnostics(
        &self,
        borrower: &LoanBorrowerProfile,
        verdict: &LoanGuardVerdict,
    ) -> String {
        format!(
            "willingness={:.2} affordable={:.2} within={} standing={:.2} rank={}/{} ca={} \
             parent_best={} first_choice={} development={} \
             weight={:.2} (value={:.0} income={}) carry={:.2} (salary={} headroom={} \
             top_earner={} bill={}) renown_gap={:.0}/{:.0} refusal={:+.0}",
            verdict.willingness,
            verdict.affordability,
            verdict.within_reach(),
            verdict.standing,
            self.parent_rank,
            self.group.typical_starters(),
            self.level,
            self.parent_best_in_group,
            self.first_choice(),
            self.is_development(),
            verdict.weight,
            self.value,
            borrower.income,
            verdict.carry,
            self.salary,
            borrower.wage_headroom,
            borrower.top_earner,
            borrower.wage_bill,
            verdict.renown_gap,
            verdict.renown_band,
            verdict.refusal_delta,
        )
    }
}

/// One priced (player, borrower) pair.
#[derive(Debug, Clone, Copy)]
pub struct LoanGuardVerdict {
    /// Player value ÷ borrower annual income.
    pub weight: f64,
    /// Borrower wage share ÷ what the borrower can pay, after the
    /// parent's subsidy is written into the split.
    pub carry: f64,
    /// Standing at the parent, 0..1.25.
    pub standing: f32,
    /// The parent's own position on lending him out, 0..1.
    pub willingness: f32,
    /// What [`LoanMoney`] makes of the two terms above, 0..1.
    pub affordability: f32,
    /// Added to the seller's engagement chance at the initial approach.
    pub refusal_delta: f32,
    /// The player's own renown band, and how far this borrower falls
    /// short of his standing. Carried for the trace and for the
    /// plausibility gate that reads the same numbers.
    pub renown_band: f32,
    pub renown_gap: f32,
}

impl LoanGuardVerdict {
    /// Affordability below which the borrower is not really a
    /// destination at all.
    const AFFORDABLE: f32 = 0.2;
    /// Seller engagement a parent that will not lend him costs, at total
    /// reluctance. Measured against [`ParentWillingness::ENTERTAINS`],
    /// which is already the point at which a club will have the
    /// conversation: above the bar the destination owns the refusal,
    /// below it the parent does, and the cost ramps to the whole of this
    /// at nought.
    const REFUSAL_UNWILLING: f32 = -60.0;
    /// … and what a destination that cannot carry him costs, at total
    /// unaffordability. Both are ramps in their own term: the room and
    /// the draw price the same two readings or they price two deals.
    const REFUSAL_UNAFFORDABLE: f32 = -25.0;

    /// What the two readings take off the seller's engagement roll.
    fn refusal_delta(willingness: f32, affordability: f32) -> f32 {
        Self::REFUSAL_UNWILLING
            * (1.0 - willingness / ParentWillingness::ENTERTAINS).clamp(0.0, 1.0)
            + Self::REFUSAL_UNAFFORDABLE * (1.0 - affordability)
    }

    /// The same verdict, re-answered about ONE destination.
    ///
    /// The parent's willingness is staged before any borrower exists, so a
    /// cross-border approach re-states it against the place the boy would
    /// actually go — and the refusal it implies moves with it, or the room
    /// reads a club refusing a destination it was never asked about.
    pub fn about(mut self, willingness: f32, affordability: f32) -> Self {
        self.willingness = willingness.clamp(0.0, 1.0);
        self.affordability = affordability.clamp(0.0, 1.0);
        self.refusal_delta = Self::refusal_delta(self.willingness, self.affordability);
        self
    }

    /// The parent would entertain it and the borrower could carry it.
    /// Not a veto — what it decides is whether an approach starts from
    /// the listing's own "he is available" base or from a cold one.
    pub fn within_reach(&self) -> bool {
        self.willingness >= ParentWillingness::ENTERTAINS && self.affordability >= Self::AFFORDABLE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The case the guard was written for, in the model's own numbers:
    /// Lamine Yamal (CA 176, value $189M, salary $14.6M) at Barcelona,
    /// offered to Córdoba (Segunda, ~$20M a year).
    struct Fx;

    impl Fx {
        const FORWARD: PlayerFieldPositionGroup = PlayerFieldPositionGroup::Forward;
        /// La Liga / Segunda, from `league.json`.
        const LA_LIGA: u16 = 9_200;
        const SEGUNDA: u16 = 6_500;

        fn barcelona() -> ClubLevelAnchor {
            ClubLevelAnchor::for_reputation(0.93)
        }

        fn cordoba() -> ClubLevelAnchor {
            ClubLevelAnchor::for_reputation(0.45)
        }

        /// A peer: another La Liga club of Barcelona's own standing.
        fn peer_borrower() -> LoanBorrowerProfile {
            LoanBorrowerProfile {
                income: 400_000_000,
                wage_bill: 200_000_000,
                top_earner: 20_000_000,
                wage_headroom: 40_000_000,
                best_in_group: 170,
                anchor: ClubLevelAnchor::for_reputation(0.90),
                world_rep: 9_000,
                league_rep: Self::LA_LIGA,
                reach: 9_000,
            }
        }

        fn cordoba_borrower() -> LoanBorrowerProfile {
            LoanBorrowerProfile {
                income: 20_000_000,
                wage_bill: 12_000_000,
                top_earner: 900_000,
                wage_headroom: 1_500_000,
                best_in_group: 120,
                anchor: Self::cordoba(),
                world_rep: 3_000,
                league_rep: Self::SEGUNDA,
                reach: 3_800,
            }
        }

        fn yamal(requested: bool, advertised: bool) -> LoanAssetGuard {
            LoanAssetGuard::new(
                176,
                19,
                Self::FORWARD,
                Self::barcelona(),
                0,
                1,
                176,
                Self::LA_LIGA,
                189_000_000.0,
                14_600_000,
                requested,
                advertised,
                8_500,
                0.0,
            )
        }
    }

    #[test]
    fn a_nine_figure_asset_is_priced_out_of_a_small_club_rather_than_barred() {
        let guard = Fx::yamal(false, false);
        assert!(guard.first_choice(), "co-best forward, rank 0");
        assert!(
            !guard.is_development(),
            "a first-team regular is not on a development pathway"
        );
        // A club that would send him anywhere; the destination is still
        // the problem.
        let verdict = guard.assess(&Fx::cordoba_borrower(), 1.0, 0.0);
        assert!(verdict.weight > 1.0, "{verdict:?}");
        assert!(verdict.carry > 1.0, "{verdict:?}");
        assert!(verdict.affordability < 0.1, "{verdict:?}");
        assert!(!verdict.within_reach());
        assert!(verdict.refusal_delta < -20.0, "{verdict:?}");
    }

    #[test]
    fn the_same_player_is_affordable_at_a_peer() {
        let verdict = Fx::yamal(true, false).assess(&Fx::peer_borrower(), 1.0, 0.0);
        assert!(verdict.affordability > 0.5, "{verdict:?}");
        assert!(verdict.within_reach(), "{verdict:?}");
    }

    /// The parent's own reluctance and the borrower's own poverty are two
    /// separate readings, and each costs the approach in proportion to
    /// itself. Nothing here is a veto.
    #[test]
    fn the_refusal_is_a_ramp_on_each_reading_and_zero_when_both_are_clear() {
        let guard = Fx::yamal(true, false);
        let borrower = Fx::peer_borrower();
        let content = guard.assess(&borrower, 1.0, 0.0);
        assert!(content.refusal_delta > -15.0, "{content:?}");

        let reluctant = guard.assess(&borrower, 0.0, 0.0);
        assert!(
            reluctant.refusal_delta < content.refusal_delta - 40.0,
            "{reluctant:?} vs {content:?}"
        );
        assert!(!reluctant.within_reach(), "a club that will not send him");
    }

    /// The case the old `Untouchable` verdict killed: a borrower that can
    /// only cover a fraction of the wage, and a parent that means to keep
    /// paying the rest.
    #[test]
    fn a_parent_paying_the_wage_changes_what_the_borrower_carries() {
        let guard = LoanAssetGuard::new(
            140,
            22,
            Fx::FORWARD,
            Fx::barcelona(),
            3,
            1,
            176,
            Fx::LA_LIGA,
            5_000_000.0,
            3_000_000,
            false,
            true,
            3_000,
            0.0,
        );
        let unpaid = guard.assess(&Fx::cordoba_borrower(), 1.0, 0.0);
        let subsidised = guard.assess(&Fx::cordoba_borrower(), 1.0, 1.0);
        assert!(subsidised.carry < unpaid.carry, "{subsidised:?} {unpaid:?}");
        assert!(
            subsidised.affordability > unpaid.affordability,
            "{subsidised:?} {unpaid:?}"
        );
    }

    #[test]
    fn a_borrower_whose_books_cannot_be_read_objects_to_nothing() {
        let guard = Fx::yamal(false, false);
        let unknown = LoanBorrowerProfile {
            income: 0,
            wage_headroom: 0,
            top_earner: 0,
            ..Fx::cordoba_borrower()
        };
        let verdict = guard.assess(&unknown, 1.0, 0.0);
        assert_eq!(verdict.weight, 0.0);
        assert_eq!(verdict.carry, 0.0);
        assert!(verdict.within_reach(), "{verdict:?}");
    }

    #[test]
    fn affordability_falls_continuously_as_the_asset_outgrows_the_borrower() {
        let at = |value: f64| {
            LoanAssetGuard::new(
                140,
                22,
                Fx::FORWARD,
                Fx::barcelona(),
                3,
                1,
                176,
                Fx::LA_LIGA,
                value,
                400_000,
                false,
                true,
                3_000,
                0.0,
            )
            .assess(&Fx::cordoba_borrower(), 1.0, 0.0)
            .affordability
        };
        assert!(at(4_000_000.0) > at(13_500_000.0));
        assert!(at(13_500_000.0) > at(20_000_000.0));
        assert!(at(20_000_000.0) > 0.0, "dear, not impossible");
    }

    #[test]
    fn the_renown_band_widens_with_youth_and_never_vanishes() {
        let at_16 = LoanAssetGuard::renown_gap_tolerated(16, 0.0);
        let at_19 = LoanAssetGuard::renown_gap_tolerated(19, 0.0);
        let at_23 = LoanAssetGuard::renown_gap_tolerated(23, 0.0);
        assert!(at_16 > at_19 && at_19 > at_23, "{at_16} {at_19} {at_23}");
        assert!(
            (at_23 - thresholds::REP_STEP_DOWN_GAP as f32).abs() < 1.0,
            "at the development age the band is the ordinary step-down band"
        );
        assert!(
            at_23 > 0.0,
            "renown widens with youth; it never vanishes at any age"
        );
        assert!(
            LoanAssetGuard::renown_gap_tolerated(19, 1.0)
                > LoanAssetGuard::renown_gap_tolerated(19, 0.0),
            "months unsold widen what he will listen to"
        );
    }
}
