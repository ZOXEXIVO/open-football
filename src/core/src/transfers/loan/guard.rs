//! `LoanAssetGuard` — the one place a loan's **destination** is priced.
//!
//! Every other loan gate in the pipeline is *relative*: how far the player
//! sits below his parent's best at his position, how much reputation the
//! two clubs differ by, whether he would get minutes. None of them ever
//! asked the two questions a real loan turns on — **is this club's own
//! year smaller than the asset it is being handed**, and **can it pay the
//! wage that comes with him**. So a nine-figure teenager who was the joint
//! best forward at his club could be lent to a second-division side for
//! nothing: he was labelled a prospect by his birth year, read as
//! unimportant by every importance model, and the destination floors were
//! all lifted by a blanket "development" allowance that keyed on age
//! alone.
//!
//! The guard is destination-specific by construction: the same player can
//! be perfectly loanable to a peer club and untouchable at the club one
//! division down, because the two terms that decide it — `weight`
//! (his value against the borrower's annual income) and `carry` (the wage
//! share against what the borrower can actually pay) — are properties of
//! the *pair*, not of the player.
//!
//! Everything here is pure, continuous and reads TRUTH (current ability,
//! valuation, wages, reputations). Hidden potential ability is never read;
//! nothing in it is a per-club special case. It only ever **prevents** a
//! loan — no path gains a destination because the guard ran.
//!
//! Shape:
//!
//! * [`LoanAssetGuard`] is built once per player from the PARENT side.
//!   [`LoanAssetGuard::parent_holds`] is the destination-independent half
//!   ("a club does not loan out its own starter"), which the listing and
//!   loan-intent passes consult before any borrower exists.
//! * [`LoanBorrowerProfile`] is the borrower side, assembled by each call
//!   site from the club it is about to offer him to.
//! * [`LoanAssetGuard::assess`] combines them into a [`LoanGuardVerdict`]
//!   carrying the [`LoanReach`], both money terms, the soft
//!   `capacity_penalty`, the seller's `refusal_delta` and a one-line
//!   diagnostic for `OF_TRACE_PLAYER`.

use chrono::NaiveDate;

use crate::club::player::calculators::WageCalculator;
use crate::transfers::gate::{EffectivePlayerReputation, thresholds};
use crate::transfers::pipeline::PipelineProcessor;
use crate::transfers::pipeline::trace::MarketSwitches;
use crate::{
    Club, ClubLevelAnchor, Person, Player, PlayerFieldPositionGroup, PlayerStatusType,
    ReputationLevel,
};

/// How far down a loan may reach for this player — the verdict the guard
/// exists to produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanReach {
    /// No loan at all. Either the parent would not send him (he is its
    /// first choice and has not asked to go), or the borrower cannot carry
    /// the asset — its whole year is worth less than the player, or the
    /// wage is beyond it.
    Untouchable,
    /// Peer clubs only: a side at the parent's own level, in a competition
    /// of the parent's own standard.
    PeerLevel,
    /// One step down — the ordinary loan of a squad player who needs
    /// football. The existing readiness-keyed destination floors own the
    /// exact depth.
    OneStepDown,
    /// The development pathway: a genuinely below-level youngster drops as
    /// far as the minutes gate allows.
    Anywhere,
}

impl LoanReach {
    /// Stable label for the trace and the census.
    pub fn label(self) -> &'static str {
        match self {
            LoanReach::Untouchable => "untouchable",
            LoanReach::PeerLevel => "peer_level",
            LoanReach::OneStepDown => "one_step_down",
            LoanReach::Anywhere => "anywhere",
        }
    }

    /// Does this verdict permit a loan at all?
    pub fn allows_loan(self) -> bool {
        !matches!(self, LoanReach::Untouchable)
    }

    /// Lowest reputation tier a seller-side broadcast may cascade to for
    /// this verdict, given the parent's own tier. A listing is consent to
    /// a loan, not consent to any destination: a peer-level asset stops at
    /// the parent's tier, a one-step-down asset at the tier below, and the
    /// development pathway keeps the whole market.
    pub fn cascade_floor(self, parent_tier: ReputationLevel) -> ReputationLevel {
        match self {
            LoanReach::Untouchable | LoanReach::PeerLevel => parent_tier,
            LoanReach::OneStepDown => parent_tier.next_lower(),
            LoanReach::Anywhere => ReputationLevel::Amateur,
        }
    }
}

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
    ca: u8,
    age: u8,
    group: PlayerFieldPositionGroup,
    parent_anchor: ClubLevelAnchor,
    parent_rank: u8,
    parent_best_in_group: u8,
    parent_league_rep: u16,
    value: f64,
    salary: u32,
    player_requested: bool,
    seller_advertised: bool,
    player_effective_rep: i16,
    listing_resignation: f32,
}

impl LoanAssetGuard {
    /// Value ÷ borrower annual income at which the capacity penalty starts
    /// to bite …
    pub const W_SOFT: f64 = 0.35;
    /// … and at which no club on earth borrows the asset. A loan is a
    /// season of somebody else's money; a club whose entire year is worth
    /// less than the player is not a destination, it is a liability.
    pub const W_MAX: f64 = 1.0;
    /// Wage share ÷ what the borrower can pay. Above one the club is
    /// borrowing a wage it cannot carry, whatever the fee is.
    pub const CARRY_MAX: f64 = 1.0;
    /// Key-player floor gap a `PeerLevel` borrower may sit below the
    /// parent's. Ten points is one upgrade band — the difference between
    /// two clubs of the same standing, not between two levels.
    pub const PEER_BAND: i16 = 10;
    /// Share of the parent's league standing a `PeerLevel` borrower's own
    /// competition must reach.
    pub const PEER_LEAGUE_SHARE: f32 = 0.85;
    /// Standing at/above which the player is his club's own level — only
    /// peers borrow him.
    pub const PEER_STANDING: f32 = 1.0;
    /// Standing below which he is not a first-team asset at his parent at
    /// all, and the development floors own the destination.
    pub const RAW_STANDING: f32 = 0.35;
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
    /// Seller engagement lost when the parent will not send him there at
    /// all …
    const REFUSAL_UNTOUCHABLE: f32 = -80.0;
    /// … when the borrower sits below the verdict's own floor …
    const REFUSAL_BELOW_REACH: f32 = -40.0;
    /// … and the most a merely expensive-for-them destination costs.
    const REFUSAL_CAPACITY_SPAN: f32 = -25.0;

    /// Assemble the parent side.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ca: u8,
        age: u8,
        group: PlayerFieldPositionGroup,
        parent_anchor: ClubLevelAnchor,
        parent_rank: u8,
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
            ca,
            age,
            group,
            parent_anchor,
            parent_rank,
            parent_best_in_group,
            parent_league_rep,
            value,
            salary,
            player_requested,
            seller_advertised,
            player_effective_rep,
            listing_resignation,
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
            ca: player.player_attributes.current_ability,
            age: player.age(date),
            group,
            parent_anchor: ClubLevelAnchor::for_reputation(team.reputation.overall_score()),
            parent_rank: PipelineProcessor::position_group_rank(club, player.id, group),
            parent_best_in_group: PipelineProcessor::best_ca_in_group(club, group),
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
        })
    }

    /// The destination-independent veto on its own: would this club
    /// entertain a loan of this player AT ALL?
    ///
    /// The money terms need a borrower and are settled in
    /// [`Self::assess`]; this is the half every loan-INTENT pass needs,
    /// and those passes run before any destination exists. False whenever
    /// the parent side cannot be read, and false on the
    /// `OF_LOAN_GUARD_OFF` arm, so it only ever prevents an intent.
    pub fn parent_holds_for(club: &Club, player: &Player, date: NaiveDate) -> bool {
        if MarketSwitches::loan_guard_off() {
            return false;
        }
        // Value and the parent's competition are money / destination
        // terms, which `parent_holds` does not read — so neither is
        // resolved here, and the half-built guard is used for nothing
        // else. The valuation matters: this runs per player per day on
        // the country listing pass.
        Self::from_parts(club, player, date, 0, 0.0)
            .map(|guard| guard.parent_holds())
            .unwrap_or(false)
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
        ca: u8,
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
        let parent_rank = if parent_best_in_group > 0 && ca >= parent_best_in_group {
            0
        } else {
            group.typical_starters().min(u8::MAX as usize) as u8
        };
        LoanAssetGuard {
            ca,
            age,
            group,
            parent_anchor: ClubLevelAnchor::for_reputation(
                (parent_world_rep.max(0) as f32 / 10_000.0).clamp(0.0, 1.0),
            ),
            parent_rank,
            parent_best_in_group,
            parent_league_rep,
            value,
            salary,
            player_requested: false,
            seller_advertised,
            player_effective_rep,
            listing_resignation: 0.0,
        }
    }

    /// Where he stands at his own club, 0..1.25: 0 at the rotation floor,
    /// 1 at the key-player floor, and above 1 for a man his club's level
    /// does not stretch to.
    pub fn standing(&self) -> f32 {
        let key = self.parent_anchor.key_floor(self.group) as f32;
        let rotation = self.parent_anchor.rotation_floor(self.group) as f32;
        let span = (key - rotation).max(1.0);
        ((self.ca as f32 - rotation) / span).clamp(0.0, 1.25)
    }

    /// The club's own first choice in this shirt: at its key-player level
    /// AND inside the slots that actually start.
    pub fn first_choice(&self) -> bool {
        self.ca as i16 >= self.parent_anchor.key_floor(self.group)
            && (self.parent_rank as usize) < self.group.typical_starters()
    }

    /// Development means BELOW HIS CLUB'S LEVEL, not "young". The age band
    /// bounds it — past it a below-level player is a squad player, not a
    /// prospect — but a teenager who is already a first-team regular for
    /// his club is not on a development pathway, and the floors that
    /// pathway lifts must not lift for him.
    pub fn is_development(&self) -> bool {
        self.age <= Self::DEVELOPMENT_AGE
            && (self.ca as i16) < self.parent_anchor.regular_floor(self.group)
    }

    /// How ready he already is for his parent's own first team, 0..1 —
    /// measured against the club's own bands rather than against whoever
    /// happens to be the best body in the group.
    pub fn readiness(&self) -> f32 {
        (self.standing() / Self::PEER_STANDING).clamp(0.0, 1.0)
    }

    /// The destination-independent half: would the parent entertain a loan
    /// AT ALL? A club does not lend its starter by its own choice — but
    /// the player's own request does open the door, because that is his
    /// decision rather than the club's.
    ///
    /// A seller-advertised loan listing is consent to a loan, so it lifts
    /// the first-choice hold too (existing doctrine: availability opens
    /// gates). It never lifts the two money terms, which are the
    /// borrower's problem and are settled in [`Self::assess`].
    pub fn parent_holds(&self) -> bool {
        self.first_choice() && !self.player_requested && !self.seller_advertised
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
        let youth = ((Self::DEVELOPMENT_AGE.saturating_sub(age)) as f32 / Self::RENOWN_AGE_SPAN)
            .clamp(0.0, 1.0);
        thresholds::REP_STEP_DOWN_GAP as f32 * (1.0 + Self::RENOWN_YOUTH_WIDENING * youth)
            + listing_resignation.clamp(0.0, 1.0) * thresholds::LOAN_RENOWN_RESIGNATION_SPAN
    }

    /// This player's own renown band, at his age and market resignation.
    pub fn renown_band(&self) -> f32 {
        Self::renown_gap_tolerated(self.age, self.listing_resignation)
    }

    /// How far a loan could reach on the PARENT side alone — his standing
    /// at his own club, with no borrower in the picture. The two money
    /// terms belong to the pair and can only ever narrow this further, so
    /// this is the reading the seller-side cascade floor is built from.
    pub fn parent_reach(&self) -> LoanReach {
        let standing = self.standing();
        if self.parent_holds() {
            LoanReach::Untouchable
        } else if standing < Self::RAW_STANDING || self.is_development() {
            LoanReach::Anywhere
        } else if standing >= Self::PEER_STANDING {
            LoanReach::PeerLevel
        } else {
            LoanReach::OneStepDown
        }
    }

    /// Price one destination.
    ///
    /// Both money terms read 0 — "no objection" — when the borrower's
    /// books cannot be read at all. That is not a nicety: a club has no
    /// income history on the day a world is created, and a guard that read
    /// a missing ledger as "this club earns nothing" would call every loan
    /// in the game untouchable for the first year. Unknown stands the gate
    /// down, exactly as an unknown competition stands the division gate
    /// down.
    pub fn assess(&self, borrower: &LoanBorrowerProfile) -> LoanGuardVerdict {
        let weight = if borrower.income > 0 {
            self.value / borrower.income as f64
        } else {
            0.0
        };
        let carry = self.carry(borrower);
        let standing = self.standing();

        let reach = if weight > Self::W_MAX || carry > Self::CARRY_MAX {
            LoanReach::Untouchable
        } else {
            self.parent_reach()
        };

        let within_reach = match reach {
            LoanReach::Untouchable => false,
            LoanReach::PeerLevel => self.clears_peer_band(borrower),
            // The readiness-keyed destination floors
            // (`LoanDestinationLevel`) own the exact depth of these two,
            // and every call site runs them beside this one.
            LoanReach::OneStepDown | LoanReach::Anywhere => true,
        };

        let capacity_penalty =
            (((weight - Self::W_SOFT) / (Self::W_MAX - Self::W_SOFT)) as f32).clamp(0.0, 1.0);
        let refusal_delta = if matches!(reach, LoanReach::Untouchable) {
            Self::REFUSAL_UNTOUCHABLE
        } else if !within_reach {
            Self::REFUSAL_BELOW_REACH
        } else {
            Self::REFUSAL_CAPACITY_SPAN * capacity_penalty
        };

        LoanGuardVerdict {
            reach,
            within_reach,
            weight,
            carry,
            standing,
            capacity_penalty,
            refusal_delta,
            renown_band: self.renown_band(),
            renown_gap: (self.player_effective_rep - borrower.reach) as f32,
        }
    }

    /// Share of his wage the borrower would pick up, against the most it
    /// could plausibly pay: its unspent wage headroom with the usual
    /// stretch, or — for a club with no headroom at all — what it already
    /// pays its best-paid player. Above one, the deal is a wage the
    /// borrower cannot carry however free the loan is.
    fn carry(&self, borrower: &LoanBorrowerProfile) -> f64 {
        // Neither a wage budget nor a single salary on the books: this
        // club's payroll is unknown, not zero. Stand the term down.
        if borrower.wage_headroom <= 0 && borrower.top_earner == 0 {
            return 0.0;
        }
        let (borrower_wage, _) =
            WageCalculator::loan_wage_split_v2(self.salary, borrower.wage_split_score(), 0.0);
        let ceiling = (borrower.wage_headroom.max(0) as f64 * 1.30)
            .max(borrower.top_earner as f64 * 1.50)
            .max(1.0);
        borrower_wage as f64 / ceiling
    }

    /// Peer band: a club of the parent's own standing, in a competition of
    /// the parent's own standard. An unknown competition on either side
    /// stands the division half down — the club half still speaks.
    fn clears_peer_band(&self, borrower: &LoanBorrowerProfile) -> bool {
        let club_ok = borrower.anchor.key_floor(self.group)
            >= self.parent_anchor.key_floor(self.group) - Self::PEER_BAND;
        let league_ok = if self.parent_league_rep == 0 || borrower.league_rep == 0 {
            true
        } else {
            borrower.league_rep as f32 >= self.parent_league_rep as f32 * Self::PEER_LEAGUE_SHARE
        };
        club_ok && league_ok
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
            "reach={} within={} standing={:.2} rank={}/{} ca={} parent_best={} \
             first_choice={} development={} \
             weight={:.2} (value={:.0} income={}) carry={:.2} (salary={} headroom={} \
             top_earner={} bill={}) renown_gap={:.0}/{:.0} refusal={:+.0}",
            verdict.reach.label(),
            verdict.within_reach,
            verdict.standing,
            self.parent_rank,
            self.group.typical_starters(),
            self.ca,
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
    pub reach: LoanReach,
    /// The borrower sits inside the verdict's own floor.
    pub within_reach: bool,
    /// Player value ÷ borrower annual income.
    pub weight: f64,
    /// Borrower wage share ÷ what the borrower can pay.
    pub carry: f64,
    /// Standing at the parent, 0..1.25.
    pub standing: f32,
    /// Soft 0..1 ramp between [`LoanAssetGuard::W_SOFT`] and
    /// [`LoanAssetGuard::W_MAX`] — the borrower can carry him, but not
    /// comfortably.
    pub capacity_penalty: f32,
    /// Added to the seller's engagement chance at the initial approach.
    pub refusal_delta: f32,
    /// The player's own renown band, and how far this borrower falls
    /// short of his standing. Carried for the trace and for the
    /// plausibility gate that reads the same numbers.
    pub renown_band: f32,
    pub renown_gap: f32,
}

impl LoanGuardVerdict {
    /// The one predicate every call site consults: may this loan happen at
    /// all, to THIS borrower?
    pub fn allows(&self) -> bool {
        self.reach.allows_loan() && self.within_reach
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
    fn a_nine_figure_first_choice_is_untouchable_on_all_three_terms() {
        let guard = Fx::yamal(false, false);
        assert!(guard.first_choice(), "co-best forward, rank 0");
        assert!(
            !guard.is_development(),
            "a first-team regular is not on a development pathway"
        );
        let verdict = guard.assess(&Fx::cordoba_borrower());
        assert_eq!(verdict.reach, LoanReach::Untouchable);
        assert!(verdict.weight > LoanAssetGuard::W_MAX, "{verdict:?}");
        assert!(verdict.carry > LoanAssetGuard::CARRY_MAX, "{verdict:?}");
        assert!(!verdict.allows());
    }

    #[test]
    fn a_request_lifts_the_first_choice_hold_but_never_the_money() {
        let guard = Fx::yamal(true, false);
        assert!(!guard.parent_holds(), "his own request opens the door");
        let verdict = guard.assess(&Fx::cordoba_borrower());
        assert_eq!(
            verdict.reach,
            LoanReach::Untouchable,
            "the borrower still cannot carry him"
        );
    }

    #[test]
    fn a_seller_advertised_listing_lifts_the_hold_but_not_the_weight() {
        let guard = Fx::yamal(false, true);
        assert!(!guard.parent_holds());
        assert!(Fx::yamal(false, false).parent_holds());
        assert_eq!(
            guard.assess(&Fx::cordoba_borrower()).reach,
            LoanReach::Untouchable
        );
    }

    #[test]
    fn the_same_player_reaches_a_peer_when_his_club_consents() {
        let guard = Fx::yamal(true, false);
        let verdict = guard.assess(&Fx::peer_borrower());
        assert_eq!(verdict.reach, LoanReach::PeerLevel);
        assert!(verdict.allows(), "{verdict:?}");
        assert!(verdict.weight <= LoanAssetGuard::W_MAX);
        assert!(verdict.carry <= LoanAssetGuard::CARRY_MAX);
    }

    #[test]
    fn a_raw_seventeen_year_old_under_the_same_best_goes_anywhere() {
        let guard = LoanAssetGuard::new(
            120,
            17,
            Fx::FORWARD,
            Fx::barcelona(),
            4,
            176,
            Fx::LA_LIGA,
            2_000_000.0,
            120_000,
            false,
            true,
            1_200,
            0.0,
        );
        assert!(guard.is_development());
        let verdict = guard.assess(&Fx::cordoba_borrower());
        assert_eq!(verdict.reach, LoanReach::Anywhere);
        assert!(verdict.allows());
    }

    #[test]
    fn a_rotation_player_is_one_step_down_and_stops_at_the_tier_below() {
        // A 22-year-old at his club's regular level but short of its key
        // floor: a squad player, not a prospect and not a starter.
        let anchor = Fx::barcelona();
        let ca = (anchor.key_floor(Fx::FORWARD) - 4) as u8;
        let guard = LoanAssetGuard::new(
            ca,
            22,
            Fx::FORWARD,
            anchor,
            3,
            176,
            Fx::LA_LIGA,
            8_000_000.0,
            900_000,
            false,
            true,
            3_000,
            0.0,
        );
        let verdict = guard.assess(&Fx::cordoba_borrower());
        assert_eq!(verdict.reach, LoanReach::OneStepDown, "{verdict:?}");
        assert!(verdict.allows());
        assert_eq!(
            LoanReach::OneStepDown.cascade_floor(ReputationLevel::Elite),
            ReputationLevel::Continental
        );
        assert_eq!(
            LoanReach::PeerLevel.cascade_floor(ReputationLevel::Elite),
            ReputationLevel::Elite
        );
    }

    #[test]
    fn carry_fails_on_a_big_wage_at_a_small_club_even_at_a_zero_fee() {
        // Value scaled down so `weight` clears; only the wage is the
        // problem, which is the term a free development loan hides.
        let guard = LoanAssetGuard::new(
            140,
            22,
            Fx::FORWARD,
            Fx::barcelona(),
            3,
            176,
            Fx::LA_LIGA,
            5_000_000.0,
            14_600_000,
            false,
            true,
            3_000,
            0.0,
        );
        let verdict = guard.assess(&Fx::cordoba_borrower());
        assert!(verdict.weight <= LoanAssetGuard::W_MAX, "{verdict:?}");
        assert!(verdict.carry > LoanAssetGuard::CARRY_MAX, "{verdict:?}");
        assert_eq!(verdict.reach, LoanReach::Untouchable);
    }

    #[test]
    fn the_peer_band_refuses_a_club_two_levels_below() {
        let guard = Fx::yamal(true, false);
        // Money terms neutralised: the club half of the band is what is
        // under test.
        let mut borrower = Fx::peer_borrower();
        borrower.anchor = Fx::cordoba();
        borrower.league_rep = Fx::SEGUNDA;
        let verdict = guard.assess(&borrower);
        assert_eq!(verdict.reach, LoanReach::PeerLevel);
        assert!(!verdict.within_reach, "{verdict:?}");
        assert!(!verdict.allows());
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

    #[test]
    fn capacity_penalty_ramps_between_the_two_weight_bars() {
        let guard = LoanAssetGuard::new(
            140,
            22,
            Fx::FORWARD,
            Fx::barcelona(),
            3,
            176,
            Fx::LA_LIGA,
            // Half-way between W_SOFT and W_MAX of a 20M income.
            13_500_000.0,
            400_000,
            false,
            true,
            3_000,
            0.0,
        );
        let verdict = guard.assess(&Fx::cordoba_borrower());
        assert!(
            verdict.capacity_penalty > 0.3 && verdict.capacity_penalty < 0.7,
            "{verdict:?}"
        );
        assert!(verdict.refusal_delta < 0.0 && verdict.refusal_delta > -25.0);
    }
}
