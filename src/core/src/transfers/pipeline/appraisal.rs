//! The player's side of a transfer — one appraisal, three doors.
//!
//! A pay cut, a money move to a weaker league, and a young foreigner's loan
//! home are the same decision seen from three angles: a footballer weighing
//! an offer against where his life currently is. Before this module the
//! simulator scored that decision as an additive pile of hand-set points,
//! rolled once, with no notion of what the offer pays *relative to what he
//! earns*, no notion of where the club is relative to where he is from, and
//! no reading of the mind the simulation builds for him every week.
//!
//! Here it is one utility. Every axis is a signed, dimensionless quantity a
//! footballer would recognise — how much more money, how much worse a stage,
//! how much more football, how far from home, how badly he wants out, how
//! tied he is — and money sits on the SAME scale as sport, so money can buy
//! a step down and a step down can be refused at any price only when the
//! sporting weight says so.
//!
//! Three properties make it work where the point pile did not:
//!
//! * **The reservation wage falls out of the utility.** There is no separate
//!   "what he asks for" curve: the wage at which `U == 0` IS his demand.
//!   That is what makes a cut and a premium the same object, and it is what
//!   lets a buyer be told "he would go for $31M a year; we can pay $12M".
//! * **A stance, not a die.** One private disposition per negotiation
//!   ([`PlayerDisposition`]), seeded, drawn once. Wage rounds move the
//!   utility; they do not re-roll luck, so a raised offer that reaches the
//!   reservation IS accepted.
//! * **Continuous in every axis.** Age, contract, standing, happiness, home,
//!   money, league gap. No threshold that flips at a birthday, no list of
//!   countries, no "money league" flag — the Gulf, China, Russia and MLS all
//!   emerge from `balance ÷ income` and region prestige, and disappear when
//!   those normalise (memory `feedback_balance_system_not_cases`).
//!
//! The three callers — permanent personal terms, loan acceptance, and the
//! mind's own `JoinClub` / `AcceptLoan` deliberation — build an
//! [`OfferView`] and a [`PlayerStance`] and ask [`PlayerOfferAppraisal`].
//! Nothing else scores a player's willingness.

use crate::club::player::contract::agent::PlayerAgent;
use crate::club::player::language::LanguageProfile;
use crate::club::player::mind::{GoalKind, MindSituation, PlayerMind};
use crate::transfers::ScoutingRegion;
use crate::transfers::offer::PromisedSquadStatus;
use crate::transfers::pipeline::playing_time::PlayingTimeExpectation;

/// What is being offered — a move, or a season somewhere else.
///
/// A loan halves the money and the sport (it is temporary) and leaves role,
/// home and push at full weight, which is what makes the loan home the
/// natural outcome for an unsettled prospect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferKind {
    Permanent,
    Loan,
}

impl OfferKind {
    #[inline]
    pub fn is_loan(self) -> bool {
        matches!(self, OfferKind::Loan)
    }
}

/// L1 — what is on the table, as the player sees it.
///
/// Built by whoever makes the offer, permanent or loan, domestic or foreign.
/// Everything here is buyer-side and knowable at the moment an offer exists;
/// nothing in it requires the selling country's borrow.
#[derive(Debug, Clone, Copy)]
pub struct OfferView {
    pub kind: OfferKind,
    pub buyer_club_id: u32,
    pub buyer_country_id: u32,
    pub buyer_continent_id: u32,
    pub buyer_region: ScoutingRegion,
    /// Annual wage he would actually draw. On a loan this is the share he is
    /// paid at the borrower, not a permanent salary.
    pub offered_wage: f64,
    /// The shirt the buyer says it is offering. `None` = nothing promised,
    /// which reads as "he will have to earn it".
    pub promised_status: Option<PromisedSquadStatus>,
    /// Sporting distance of the move, positive when the buyer is below the
    /// seller — [`crate::transfers::pipeline::plausibility::TransferPlausibilityEvaluator::sporting_drop`].
    pub sporting_drop: f32,
    /// How far the destination's region prestige falls short of the
    /// seller's, signed. Positive is a drop.
    pub prestige_drop: f32,
    /// Does the move cross a continent?
    pub crosses_continent: bool,
    /// [`crate::club::player::language::LanguageProfile::affinity_for`] the
    /// buyer's country language mask, 0..1.
    pub language_affinity: f32,
    /// A club he grew up wanting to play for.
    pub is_favourite_club: bool,
    /// How close the window is to shutting, 0..1.
    pub deadline_urgency: f32,
    /// A release clause in his contract has been met — the escape route he
    /// negotiated for himself.
    pub release_clause_triggered: bool,
    /// The buyer is the club that sold him, and how sharply that still
    /// stings, 0..1. Zero for everybody else.
    pub returning_to_seller: f32,
}

impl OfferView {
    /// A neutral permanent offer, for tests and for callers filling in one
    /// axis at a time. Every field reads as "no view" rather than bad news.
    pub fn neutral() -> Self {
        OfferView {
            kind: OfferKind::Permanent,
            buyer_club_id: 0,
            buyer_country_id: 0,
            buyer_continent_id: 0,
            buyer_region: ScoutingRegion::WesternEurope,
            offered_wage: 0.0,
            promised_status: None,
            sporting_drop: 0.0,
            prestige_drop: 0.0,
            crosses_continent: false,
            language_affinity: 1.0,
            is_favourite_club: false,
            deadline_urgency: 0.0,
            release_clause_triggered: false,
            returning_to_seller: 0.0,
        }
    }
}

/// L1b — where the player actually is, flattened.
///
/// Every field is readable on the SELLER's side. Domestic negotiations build
/// it live from the player; cross-border ones stage it on the negotiation at
/// creation, when the seller's country is still in scope. That is what lets
/// the same appraisal run on both paths instead of the foreign one falling
/// back to a bare prestige wall.
///
/// `Copy` on purpose: it rides on a negotiation and is rebuilt per round.
#[derive(Debug, Clone, Copy)]
pub struct PlayerStance {
    // ── Money ───────────────────────────────────────────────────
    /// What he is paid today, annual.
    pub current_wage: f64,
    /// What a man of his standing is worth at the club he is at now.
    pub fair_wage_at_current: f64,

    // ── Time and temperament ────────────────────────────────────
    pub age: u8,
    /// Years of prime left, 0..1 ([`MindSituation::career_runway`]).
    pub career_runway: f32,
    /// How far into his career he is, 0..1.
    pub career_spent: f32,
    /// 0..1, nothing with two years left, total at expiry.
    pub contract_pressure: f32,
    pub ambition_drive: f32,
    pub loyalty_drive: f32,
    pub adaptability_drive: f32,
    /// How strongly he is drawn to a bigger competition, 0..1.
    pub big_stage_inclination: f32,
    /// How hard a tournament on the horizon presses on him, 0..1.
    pub nt_stake: f32,

    // ── The shirt ───────────────────────────────────────────────
    /// What he is to his current club, 0..1 — the plausibility model's
    /// `player_importance`, the ONE importance formula the appraisal reads
    /// on both paths.
    pub importance: f32,
    /// Rolling share of recent competitive matches started, 0..1.
    pub starter_ratio: f32,
    /// How far short of what his role implies he is falling, −1..+1.
    pub playing_time_gap: f32,
    /// How long the market has been declining him at his level, 0..1.
    pub market_resignation: f32,

    // ── Where his life is ───────────────────────────────────────
    pub nationality_country_id: u32,
    // No `nationality_continent_id`. It was filled on every path and read
    // on none: the home term runs off the region, and the place term's
    // `crosses_continent` is a property of the MOVE (where he plays now →
    // where he would play), not of his passport. A field nothing reads is
    // a field that will drift.
    /// Footballing region of his passport. `None` when his nationality
    /// has never been seeded — the home pull then pays nothing rather
    /// than falling back to his CLUB's region, which read every
    /// unseeded foreigner as "he is from here" and paid a home bonus for
    /// a move that goes nowhere near his home.
    pub nationality_region: Option<ScoutingRegion>,
    /// Days at the current club — the `StuckCareerScan` reading, never bare
    /// `days_since_transfer` (a loan return resets that).
    pub days_at_club: u16,
    /// 0..1 — the strongest of the mind's `GoHome`, a recent
    /// `WantsReturnHome` mood, and raw cultural isolation.
    pub return_home_desire: f32,

    // ── Why he might go ─────────────────────────────────────────
    pub available_soft: bool,
    pub unhappy: bool,
    pub requested: bool,
    pub listed_by_club: bool,
    /// The strongest of `GoOutOnLoan` / `LeaveThisClub` /
    /// `PlayFirstTeamFootball`. The MAX, never the sum — `Unh` already
    /// feeds push directly and would otherwise be counted twice.
    pub leave_pressure: f32,

    // ── Why he might stay ───────────────────────────────────────
    pub at_favourite_club: bool,
    /// The stronger of `StayAtThisClub` / `BecomeAClubLegend`.
    pub stay_pressure: f32,
    /// `SecureMyFuture` — the man with a deal running down values money
    /// more, which is already how the goal is formed.
    pub secure_future_pressure: f32,

    // ── What he makes of THIS buyer ─────────────────────────────
    //
    // The stance is staged per NEGOTIATION, so buyer-specific reads that
    // need the player himself belong here rather than on the offer: a
    // cross-border resolver cannot reach him to ask.
    /// His memory of the buying club, −1..+1. Broke through here, won
    /// everything here, was sold against his will, was lied to.
    pub buyer_sentiment: f32,
    /// Agent bias on this move, −1..+1. Positive argues for signing.
    pub agent_bias: f32,
    /// The buyer is one of the clubs he grew up wanting to play for.
    pub buyer_is_favourite: bool,
    /// What he speaks, so the place term can price the dressing room he
    /// is walking into against any country's language mask.
    pub language_profile: LanguageProfile,

    // ── Where he is now, for the place term ─────────────────────
    pub seller_continent_id: u32,
    pub seller_country_id: u32,
    pub seller_region: ScoutingRegion,
}

impl PlayerStance {
    /// A neutral stance — a settled 26-year-old on a fair wage with nothing
    /// pulling at him. For tests and for callers with nothing to say.
    pub fn neutral() -> Self {
        PlayerStance {
            current_wage: 1_000_000.0,
            fair_wage_at_current: 1_000_000.0,
            age: 26,
            career_runway: 0.66,
            career_spent: 0.34,
            contract_pressure: 0.0,
            ambition_drive: 0.5,
            loyalty_drive: 0.5,
            adaptability_drive: 0.5,
            big_stage_inclination: 0.0,
            nt_stake: 0.0,
            importance: 0.55,
            starter_ratio: 0.5,
            playing_time_gap: 0.0,
            market_resignation: 0.0,
            nationality_country_id: 0,
            nationality_region: None,
            days_at_club: 0,
            return_home_desire: 0.0,
            available_soft: false,
            unhappy: false,
            requested: false,
            listed_by_club: false,
            leave_pressure: 0.0,
            at_favourite_club: false,
            stay_pressure: 0.0,
            secure_future_pressure: 0.0,
            buyer_sentiment: 0.0,
            agent_bias: 0.0,
            buyer_is_favourite: false,
            language_profile: LanguageProfile::default(),
            seller_continent_id: 0,
            seller_country_id: 0,
            seller_region: ScoutingRegion::WesternEurope,
        }
    }

    /// A stance built from nothing but what a negotiation carries.
    ///
    /// For the paths where the player himself is genuinely unreachable and
    /// nothing was staged — a global-pool free agent, whose "club" is the
    /// pool and whose depth chart does not exist. Everything unknowable is
    /// left at its no-view default, so the decision reduces to the money
    /// against a fair anchor plus the shirt on offer, which is exactly
    /// what a free agent is weighing.
    /// A free agent is tied to NOBODY and out of contract. Both facts have
    /// to be said explicitly, because the neutral stance is a settled
    /// player on a running deal:
    ///
    /// * `loyalty_drive = 0` — the attachment term prices a bond with a
    ///   club he does not have. At the neutral 0.5 it charged a flat
    ///   −0.125 on every free-agent appraisal.
    /// * `contract_pressure = 1.0` — total at expiry, which is exactly
    ///   where he is. (`MindSituation::contract_pressure` returns 0 for
    ///   `contract_days_left == 0`, so it cannot supply this either.)
    ///   Worth ≈ +0.25 of push and a heavier money weight, both correct.
    /// * `available_soft = true` — a man with no club is available.
    ///
    /// `importance` and `starter_ratio` stay at the neutral read: the
    /// pool holds no depth chart, and inventing a standing for a man
    /// nobody currently plays would be worse than saying nothing.
    pub fn from_terms(age: u8, ambition: f32, current_wage: f64, fair_wage: f64) -> Self {
        let runway = ((34.0 - age as f32) / 12.0).clamp(0.0, 1.0);
        PlayerStance {
            current_wage,
            fair_wage_at_current: fair_wage,
            age,
            career_runway: runway,
            career_spent: 1.0 - runway,
            contract_pressure: 1.0,
            ambition_drive: ambition.clamp(0.0, 1.0),
            loyalty_drive: 0.0,
            available_soft: true,
            ..PlayerStance::neutral()
        }
    }

    /// Fill the personality / career / standing half from the mind's own
    /// weekly picture, so the appraisal and the faculties never disagree
    /// about where the man is.
    pub fn with_situation(mut self, situation: &MindSituation) -> Self {
        self.age = situation.age;
        self.career_runway = situation.career_runway();
        self.career_spent = situation.career_spent();
        self.contract_pressure = situation.contract_pressure();
        self.ambition_drive = situation.ambition_drive();
        self.loyalty_drive = situation.loyalty_drive();
        self.adaptability_drive = (situation.adaptability / 20.0).clamp(0.0, 1.0);
        self.nt_stake = situation.tournament_pressure();
        self.starter_ratio = situation.starter_ratio;
        self.playing_time_gap = situation.playing_time_gap();
        self.days_at_club = situation.days_at_club;
        self
    }

    /// Read the wants off the mind. The appraisal never re-derives
    /// homesickness from raw fields when `GoHome` is sitting there — the
    /// mind is the source of the wants, the market is the source of the
    /// offer.
    ///
    /// `mood_desire` is the legacy mood channel (a recent `WantsReturnHome`
    /// event, or raw cultural isolation) so a player whose mind has not yet
    /// formed the want is not read as perfectly settled.
    pub fn with_mind(mut self, mind: &PlayerMind, mood_desire: f32) -> Self {
        self.return_home_desire = mind
            .pressure_of(GoalKind::GoHome)
            .max(mood_desire)
            .clamp(0.0, 1.0);
        // MAX, never sum: three names for one restlessness.
        self.leave_pressure = mind
            .pressure_of(GoalKind::GoOutOnLoan)
            .max(mind.pressure_of(GoalKind::LeaveThisClub))
            .max(mind.pressure_of(GoalKind::PlayFirstTeamFootball))
            .clamp(0.0, 1.0);
        self.stay_pressure = mind
            .pressure_of(GoalKind::StayAtThisClub)
            .max(mind.pressure_of(GoalKind::BecomeAClubLegend))
            .clamp(0.0, 1.0);
        self.secure_future_pressure = mind.pressure_of(GoalKind::SecureMyFuture).clamp(0.0, 1.0);
        self
    }

    /// The agent's lean on this move, folded to −1..+1 from the legacy
    /// point delta so it lands on the utility scale like everything else.
    pub fn with_agent(mut self, agent: &PlayerAgent, rep_diff: f32) -> Self {
        self.agent_bias = (agent.personal_terms_delta(rep_diff) / 20.0).clamp(-1.0, 1.0);
        self
    }

    /// Override the tenure the mind's picture supplies.
    ///
    /// `MindSituation::days_at_club` is `days_since_transfer`, which the
    /// loan machinery re-stamps on every return — a serial loanee would
    /// read as a brand-new signing twice a year, and a five-year servant
    /// would have no attachment at all. `StuckCareerScan::club_tenure_days`
    /// is the honest anchor (memory `loan_pipeline`).
    pub fn with_tenure(mut self, days_at_club: u16) -> Self {
        self.days_at_club = days_at_club;
        self
    }

    /// Is the destination his own country?
    #[inline]
    pub fn is_home_country(&self, offer: &OfferView) -> bool {
        self.nationality_country_id != 0 && self.nationality_country_id == offer.buyer_country_id
    }

    /// Is it at least his own footballing region?
    ///
    /// Fails closed on an unseeded nationality: a man whose passport the
    /// world has never stamped has no home to go to, and reading his
    /// CLUB's region as his own paid a "going home" bonus to every
    /// borrower in the league he already plays in.
    #[inline]
    pub fn is_home_region(&self, offer: &OfferView) -> bool {
        !self.is_home_country(offer) && self.nationality_region == Some(offer.buyer_region)
    }

    /// Is he playing away from his own country at all?
    ///
    /// The gate on the whole home term. Without it every domestic move by a
    /// native would collect the "going home" pull — a flat bonus on the
    /// most common transfer in the game, and a systematic thumb on the
    /// scale against every foreign one. A man already at home is not going
    /// home by moving down the road.
    #[inline]
    pub fn is_playing_abroad(&self) -> bool {
        self.nationality_country_id != 0
            && self.seller_country_id != 0
            && self.nationality_country_id != self.seller_country_id
    }
}

/// Which axis killed the deal — for the story, the census, and the trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermsRefusalCause {
    /// He would go, for more money than this buyer can hold.
    WageDemand,
    /// The football is a clear step down and nothing pays for it.
    SportingStepDown,
    /// The shirt on offer is smaller than the one he has.
    Role,
    /// The country, the distance, the language, the family.
    Place,
    /// He is tied to where he is.
    Attachment,
}

impl TermsRefusalCause {
    pub fn as_i18n_key(self) -> &'static str {
        match self {
            TermsRefusalCause::WageDemand => "terms_refusal_wage_demand",
            TermsRefusalCause::SportingStepDown => "terms_refusal_sporting_step_down",
            TermsRefusalCause::Role => "terms_refusal_role",
            TermsRefusalCause::Place => "terms_refusal_place",
            TermsRefusalCause::Attachment => "terms_refusal_attachment",
        }
    }
}

/// What he made of the offer, axis by axis.
///
/// Every refusal carries this and the reservation wage, so
/// `OF_TRACE_PLAYER` can print "he would go for $31M a year; we can pay
/// $12M" instead of a bare probability.
#[derive(Debug, Clone, Copy)]
pub struct Appraisal {
    /// `U` — the sum, before the private disposition.
    pub utility: f32,
    pub money: f32,
    pub sport: f32,
    pub role: f32,
    pub place: f32,
    pub home: f32,
    pub push: f32,
    pub attachment: f32,
    pub memory: f32,
    /// The wage at which `U + ε == 0`, given everything else. His demand.
    pub reservation_wage: u32,
    /// `ε` — his private disposition on this negotiation, drawn once.
    pub disposition: f32,
    /// The money weight actually used, so a caller can reason about how
    /// steeply the reservation moves.
    pub money_weight: f32,
}

impl Appraisal {
    /// Does he sign?
    #[inline]
    pub fn accepts(&self) -> bool {
        self.utility + self.disposition > 0.0
    }

    /// Why the deal died.
    ///
    /// `wage_unreachable` is the resolver's own answer to "could this
    /// buyer have paid his number?" — the L3 step-3 case. It outranks
    /// every axis, because a man who would sign for more money than the
    /// club can hold has refused on the WAGE however his other axes read:
    /// with an offer at or above his anchor the money term is
    /// non-negative and the axis fallback could never say so.
    ///
    /// With no such verdict to hand, the axis that cost him most is what
    /// the refusal is about.
    pub fn refusal_cause(&self, wage_unreachable: bool) -> TermsRefusalCause {
        if wage_unreachable {
            return TermsRefusalCause::WageDemand;
        }
        let candidates = [
            (self.money.min(0.0).abs(), TermsRefusalCause::WageDemand),
            (
                self.sport.min(0.0).abs(),
                TermsRefusalCause::SportingStepDown,
            ),
            (self.role.min(0.0).abs(), TermsRefusalCause::Role),
            (
                self.place.min(0.0).abs() + self.home.min(0.0).abs(),
                TermsRefusalCause::Place,
            ),
            (self.attachment.max(0.0), TermsRefusalCause::Attachment),
        ];
        let mut worst = TermsRefusalCause::WageDemand;
        let mut worst_weight = f32::MIN;
        for (weight, cause) in candidates {
            if weight > worst_weight {
                worst_weight = weight;
                worst = cause;
            }
        }
        worst
    }

    /// One grepable line for `OF_TRACE_PLAYER`.
    pub fn explain(&self) -> String {
        format!(
            "U={:+.3} (M{:+.3} S{:+.3} R{:+.3} P{:+.3} H{:+.3} D{:+.3} A{:+.3} F{:+.3}) \
             eps={:+.3} w_m={:.2} reservation={}",
            self.utility,
            self.money,
            self.sport,
            self.role,
            self.place,
            self.home,
            self.push,
            self.attachment,
            self.memory,
            self.disposition,
            self.money_weight,
            self.reservation_wage,
        )
    }
}

/// Shape constants. Every one carries the population number it was set
/// against; measure before moving any (Part V of
/// `docs/transfer_player_negotiation_prompt.md`).
#[derive(Debug, Clone, Copy)]
pub struct AppraisalConfig {
    /// Money weight at 22 with three years left. Below this a prime-age
    /// starter reads as a mercenary — the failure mode the age ladder in
    /// Part V is calibrated against (< 10 % of regular starters aged 24–27
    /// take a cut on a move).
    pub money_base: f32,
    /// How much of the money weight the career clock adds. Takes `w_m` to
    /// ~1.00 at 34 on an expiring deal: roughly a third of big-five movers
    /// aged 29+ take a lower basic wage.
    pub money_career: f32,
    /// How much a contract running down adds. Leverage evaporates in the
    /// last year and the money starts to matter more than the badge.
    pub money_contract: f32,
    /// How much a formed `SecureMyFuture` adds on top.
    pub money_secure_future: f32,
    /// Ceiling on the money weight.
    pub money_cap: f32,
    /// A loan is temporary: money and sport at half weight.
    pub loan_factor: f32,

    /// Sporting weight floor and the share the career runway carries.
    pub sport_runway_base: f32,
    pub sport_runway_span: f32,
    /// Ambition floor and span.
    pub sport_ambition_base: f32,
    pub sport_ambition_span: f32,
    /// A tournament in view for a squad-edge international.
    pub sport_nt_span: f32,
    /// Importance floor and span — a fringe player resists less.
    pub sport_importance_base: f32,
    pub sport_importance_span: f32,
    /// How far unsold months erode the step-down resistance. Same clock the
    /// seller's fee-floor erosion runs on.
    pub sport_resignation_relief: f32,
    /// A step UP is worth this share to a man with no big-stage pull, and
    /// all of it to one with the full itch.
    pub sport_upside_base: f32,
    pub sport_upside_span: f32,

    /// A promotion in role is worth this much per point of standing.
    pub role_gain: f32,
    /// A demotion costs this much, scaled by what he has to lose — keeps
    /// `playing_time.rs`'s 15 : 25 asymmetry.
    pub role_demotion: f32,

    /// Western Europe → Gulf is a 0.6 prestige drop ⇒ ≈ 0.27 before
    /// adaptability and family.
    pub place_prestige: f32,
    pub place_continent: f32,
    pub place_language: f32,
    /// A natural settler pays 60 % of the place cost; a man who cannot
    /// settle pays all of it.
    pub place_adaptability_floor: f32,
    /// A settled family adds up to 30 % to the place cost.
    pub place_family_span: f32,

    /// Home country is worth this at full desire; home region a fraction.
    pub home_country: f32,
    pub home_region: f32,
    /// The rumour every player entertains, before any want has formed. Must
    /// stay small — without the mind's gate the pull turns every foreigner
    /// into a returnee (Part VIII, "the magnet").
    pub home_base: f32,

    pub push_soft: f32,
    pub push_unhappy: f32,
    pub push_requested: f32,
    pub push_listed: f32,
    pub push_contract: f32,
    pub push_bench: f32,
    pub push_goals: f32,
    pub push_deadline: f32,
    /// A requested, unhappy, benched player accepts nearly any sensible
    /// move — and still refuses one that is bad on every other axis.
    pub push_cap: f32,

    pub attachment_base: f32,
    pub attachment_favourite: f32,
    pub attachment_long_service: f32,
    pub attachment_goals: f32,
    /// Days of service at which "long service" is full. Five seasons.
    pub attachment_long_service_days: f32,

    /// A boyhood club. Replaces the flat +25 acceptance points.
    pub memory_favourite_destination: f32,
    /// How much his memory of the buying club moves him.
    pub memory_sentiment: f32,
    /// The escape route he negotiated for himself.
    pub memory_release_clause: f32,
    /// Being asked back by the club that sold him.
    pub memory_returning_to_seller: f32,
    /// The agent's lean.
    pub memory_agent: f32,

    /// Disposition σ — one draw per negotiation, ≈ 0.7 probability width
    /// across the decision.
    pub disposition_sigma: f32,
}

impl Default for AppraisalConfig {
    fn default() -> Self {
        AppraisalConfig {
            money_base: 0.30,
            money_career: 0.45,
            money_contract: 0.25,
            money_secure_future: 0.15,
            money_cap: 1.00,
            loan_factor: 0.5,

            sport_runway_base: 0.45,
            sport_runway_span: 0.55,
            sport_ambition_base: 0.55,
            sport_ambition_span: 0.45,
            sport_nt_span: 0.5,
            sport_importance_base: 0.60,
            sport_importance_span: 0.40,
            sport_resignation_relief: 0.55,
            sport_upside_base: 0.35,
            sport_upside_span: 0.65,

            role_gain: 0.35,
            role_demotion: 0.50,

            place_prestige: 0.45,
            place_continent: 0.15,
            place_language: 0.12,
            place_adaptability_floor: 0.60,
            place_family_span: 0.3,

            home_country: 1.0,
            home_region: 0.4,
            home_base: 0.25,

            push_soft: 0.20,
            push_unhappy: 0.35,
            push_requested: 0.55,
            push_listed: 0.30,
            push_contract: 0.25,
            push_bench: 0.30,
            push_goals: 0.30,
            push_deadline: 0.15,
            push_cap: 0.90,

            attachment_base: 0.25,
            attachment_favourite: 0.45,
            attachment_long_service: 0.20,
            attachment_goals: 0.30,
            attachment_long_service_days: 1825.0,

            memory_favourite_destination: 0.60,
            memory_sentiment: 0.35,
            memory_release_clause: 0.90,
            memory_returning_to_seller: 0.55,
            memory_agent: 0.25,

            disposition_sigma: 0.22,
        }
    }
}

/// The private disposition a player brings to ONE negotiation.
///
/// Drawn once per `(negotiation, player)` and re-derived identically on
/// every round, so a buyer that raises its offer to the reservation wage
/// gets the yes it paid for instead of another coin flip. A stance, not a
/// die (design principle 4).
pub struct PlayerDisposition;

impl PlayerDisposition {
    /// `ε ~ N(0, σ)`, deterministic in the whole negotiation.
    ///
    /// The id alone is not unique in the world: `next_negotiation_id`
    /// lives on each country's own `TransferMarket`, so two foreign buyers
    /// in different countries could hand the same player the same
    /// temperament on the same day. The buyer and its country make the
    /// seed the negotiation's own.
    pub fn for_negotiation(
        buying_country_id: u32,
        negotiation_id: u32,
        player_id: u32,
        buyer_club_id: u32,
        sigma: f32,
    ) -> f32 {
        let pair = Self::hash((buying_country_id as u64) << 32 | buyer_club_id as u64);
        let mixed = Self::hash(((negotiation_id as u64) << 32 | player_id as u64) ^ pair);
        // Two independent uniforms out of one 64-bit mix, Box–Muller for a
        // genuine normal rather than a triangular sum-of-uniforms.
        let u1 = (((mixed >> 32) as u32 as f64) / (u32::MAX as f64)).max(1e-9);
        let u2 = ((mixed as u32 as f64) / (u32::MAX as f64)).clamp(0.0, 1.0);
        let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
        // Three sigma is as far as a disposition ever reaches; beyond that
        // it stops being a temperament and starts being a bug.
        (z as f32 * sigma).clamp(-3.0 * sigma, 3.0 * sigma)
    }

    fn hash(mut x: u64) -> u64 {
        // SplitMix64 finaliser — cheap, deterministic, well mixed.
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// L2 — the decision.
///
/// Stateless. Everything it needs is in the [`OfferView`] and the
/// [`PlayerStance`]; nothing here walks the world.
pub struct PlayerOfferAppraisal;

impl PlayerOfferAppraisal {
    /// The smallest wage any anchor is allowed to be, so a player with no
    /// contract cannot divide by zero into an infinite raise.
    const WAGE_FLOOR: f64 = 500.0;

    /// Weigh the offer.
    pub fn appraise(
        stance: &PlayerStance,
        offer: &OfferView,
        disposition: f32,
        cfg: &AppraisalConfig,
    ) -> Appraisal {
        let loan = offer.kind.is_loan();

        // ── M · money ───────────────────────────────────────────
        //
        // The anchor is the GEOMETRIC MEAN of what he is paid and what he
        // is worth where he is. An overpaid backup (6M on a 2.5M standing)
        // anchors at 3.9M, so a 3M offer reads as a modest cut rather than
        // a halving; an underpaid starter anchors above his wage, so a
        // market offer reads as a raise. That one line replaces both the
        // flat "< 0.8 → −10" and the old reservation's blindness to his
        // current deal.
        //
        // Log, so doubling and halving are symmetric and 4× is worth twice
        // 2× — the veteran payday and the prime refusal become the same
        // term with a different weight.
        let anchor = Self::anchor(stance);
        let mut money_weight = (cfg.money_base
            + cfg.money_career * stance.career_spent
            + cfg.money_contract * stance.contract_pressure
            + cfg.money_secure_future * stance.secure_future_pressure)
            .clamp(cfg.money_base, cfg.money_cap);
        if loan {
            money_weight *= cfg.loan_factor;
        }
        let offered = offer.offered_wage.max(Self::WAGE_FLOOR);
        let money = money_weight * (offered / anchor).ln() as f32;

        // ── S · sport ───────────────────────────────────────────
        let sport_weight = Self::sport_weight(stance, cfg, loan);
        let drop = offer.sporting_drop;
        let sport = if drop > 0.0 {
            -sport_weight * drop
        } else {
            let up = -drop;
            let appetite = cfg.sport_upside_base
                + cfg.sport_upside_span * stance.big_stage_inclination.clamp(0.0, 1.0);
            let temporary = if loan { cfg.loan_factor } else { 1.0 };
            up * appetite * temporary
        };

        // ── R · role ────────────────────────────────────────────
        //
        // Will I play? The question a footballer asks first. The buyer
        // already says what shirt it is offering; the seller side already
        // says what he currently is. Compensable — a bigger stage or a much
        // bigger wage can still buy a bench seat, because they are separate
        // positive terms on the same scale rather than a discount here.
        let promised = PlayingTimeExpectation::promised_standing(offer.promised_status.as_ref());
        let importance = stance.importance.clamp(0.0, 1.0);
        let role = cfg.role_gain * (promised - stance.starter_ratio.clamp(0.0, 1.0))
            - cfg.role_demotion * importance * (importance - promised).max(0.0);

        // ── P · place ───────────────────────────────────────────
        //
        // Priced, not walled. The old −110-points-per-prestige-point block
        // refused a Gulf move at any wage; here a 0.6 drop costs ≈ 0.27
        // before adaptability and family, which is payable by ≈ 2.5× wage
        // at 30 and ≈ 5× at 25 — the population that actually moves.
        let place = if stance.is_home_country(offer) {
            0.0
        } else {
            // The dressing room only changes language when the COUNTRY
            // does. Arsenal → Chelsea was costing a Brazilian
            // 0.12 × (1 − affinity) for a move that changes nothing about
            // what is spoken around him.
            let changes_country = stance.seller_country_id != offer.buyer_country_id;
            let raw = cfg.place_prestige * offer.prestige_drop.max(0.0)
                + cfg.place_continent * if offer.crosses_continent { 1.0 } else { 0.0 }
                + if changes_country {
                    cfg.place_language * (1.0 - offer.language_affinity.clamp(0.0, 1.0))
                } else {
                    0.0
                };
            let settles = cfg.place_adaptability_floor
                + (1.0 - cfg.place_adaptability_floor) * (1.0 - stance.adaptability_drive);
            -raw * settles * Self::family(stance, cfg)
        };

        // ── H · home ────────────────────────────────────────────
        //
        // A pull, gated by the mind. With no `GoHome` and no
        // `WantsReturnHome` the home country is worth the base alone — the
        // rumour every player entertains. With the want fully formed it is
        // enough on its own to carry a loan and a sideways move, and not
        // enough to carry a large step down against a starter's `w_s`.
        //
        // And it pays only to a man who is actually away: a native moving
        // domestically is not going home, and paying him for it would put
        // a flat bonus on the most common transfer in the game.
        let home_weight = if !stance.is_playing_abroad() {
            0.0
        } else if stance.is_home_country(offer) {
            cfg.home_country
        } else if stance.is_home_region(offer) {
            cfg.home_region
        } else {
            0.0
        };
        let home = home_weight
            * (cfg.home_base + (1.0 - cfg.home_base) * stance.return_home_desire.clamp(0.0, 1.0));

        // ── D · push ────────────────────────────────────────────
        let push = (cfg.push_soft * f32::from(stance.available_soft)
            + cfg.push_unhappy * f32::from(stance.unhappy)
            + cfg.push_requested * f32::from(stance.requested)
            + cfg.push_listed * f32::from(stance.listed_by_club)
            + cfg.push_contract * stance.contract_pressure
            + cfg.push_bench * (-stance.playing_time_gap).max(0.0)
            + cfg.push_goals * stance.leave_pressure
            + cfg.push_deadline * offer.deadline_urgency.clamp(0.0, 1.0))
        .clamp(0.0, cfg.push_cap);

        // ── A · attachment ──────────────────────────────────────
        let long_service =
            (stance.days_at_club as f32 / cfg.attachment_long_service_days).clamp(0.0, 1.0);
        let attachment = stance.loyalty_drive
            * (cfg.attachment_base
                + cfg.attachment_favourite * f32::from(stance.at_favourite_club)
                + cfg.attachment_long_service * long_service)
            + cfg.attachment_goals * stance.stay_pressure;

        // ── F · memory and circumstance ─────────────────────────
        let memory = cfg.memory_favourite_destination * f32::from(offer.is_favourite_club)
            + cfg.memory_sentiment * stance.buyer_sentiment.clamp(-1.0, 1.0)
            + cfg.memory_release_clause * f32::from(offer.release_clause_triggered)
            - cfg.memory_returning_to_seller * offer.returning_to_seller.clamp(0.0, 1.0)
            + cfg.memory_agent * stance.agent_bias.clamp(-1.0, 1.0);

        let utility = money + sport + role + place + home + push - attachment + memory;

        // The wage that makes `U + ε == 0`, given everything else — his
        // demand, and the number the buyer's wage power is compared with.
        let without_money = utility - money + disposition;
        let reservation = (anchor * (-(without_money / money_weight) as f64).exp())
            .clamp(Self::WAGE_FLOOR, u32::MAX as f64);

        Appraisal {
            utility,
            money,
            sport,
            role,
            place,
            home,
            push,
            attachment,
            memory,
            reservation_wage: reservation as u32,
            disposition,
            money_weight,
        }
    }

    /// `sqrt(current × fair)` — a legacy contract drifts toward what the
    /// man is actually worth at the level he plays at.
    ///
    /// A man on NO contract has no current wage to blend, and blending
    /// the floor into it produced an anchor of ≈ 22k against a $1M fair
    /// wage: every offer then read as a three- or four-unit raise. With
    /// nothing to anchor on, what he is worth IS the anchor.
    pub fn anchor(stance: &PlayerStance) -> f64 {
        let fair = stance.fair_wage_at_current.max(Self::WAGE_FLOOR);
        if stance.current_wage <= 0.0 {
            return fair;
        }
        let current = stance.current_wage.max(Self::WAGE_FLOOR);
        (current * fair).sqrt().max(Self::WAGE_FLOOR)
    }

    /// `w_s` — how much the football matters to him. Carries age, ambition,
    /// the national team and what he currently is, and fades with the
    /// months he has spent watching the market decline him.
    pub fn sport_weight(stance: &PlayerStance, cfg: &AppraisalConfig, loan: bool) -> f32 {
        let runway =
            cfg.sport_runway_base + cfg.sport_runway_span * stance.career_runway.clamp(0.0, 1.0);
        let ambition = cfg.sport_ambition_base
            + cfg.sport_ambition_span * stance.ambition_drive.clamp(0.0, 1.0);
        let nt = 1.0 + cfg.sport_nt_span * stance.nt_stake.clamp(0.0, 1.0);
        let importance = cfg.sport_importance_base
            + cfg.sport_importance_span * stance.importance.clamp(0.0, 1.0);
        let resignation =
            1.0 - cfg.sport_resignation_relief * stance.market_resignation.clamp(0.0, 1.0);
        let temporary = if loan { cfg.loan_factor } else { 1.0 };
        runway * ambition * nt * importance * resignation * temporary
    }

    /// A settled family raises the cost of moving — up to +30 % for a man
    /// past his late twenties who has been somewhere three years.
    fn family(stance: &PlayerStance, cfg: &AppraisalConfig) -> f32 {
        let settled_age = ((stance.age as f32 - 26.0) / 6.0).clamp(0.0, 1.0);
        let settled_time = (stance.days_at_club as f32 / 1095.0).clamp(0.0, 1.0);
        1.0 + cfg.place_family_span * settled_age * settled_time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Part V money-move profile: a Premier League key player weighing
    /// a Gulf offer. Drop 0.40, prestige drop 0.6, English bridge language,
    /// no compatriots, no push.
    fn gulf_offer(wage_multiple: f64, current_wage: f64) -> OfferView {
        OfferView {
            offered_wage: current_wage * wage_multiple,
            sporting_drop: 0.40,
            prestige_drop: 0.60,
            crosses_continent: true,
            language_affinity: 0.45,
            buyer_country_id: 900,
            buyer_region: ScoutingRegion::MiddleEast,
            promised_status: Some(PromisedSquadStatus::KeyPlayer),
            ..OfferView::neutral()
        }
    }

    fn premier_league_star(age: u8) -> PlayerStance {
        let runway = ((34.0 - age as f32) / 12.0).clamp(0.0, 1.0);
        PlayerStance {
            current_wage: 8_900_000.0,
            fair_wage_at_current: 8_900_000.0,
            age,
            career_runway: runway,
            career_spent: 1.0 - runway,
            ambition_drive: 0.75,
            loyalty_drive: 0.50,
            adaptability_drive: 0.50,
            importance: 0.85,
            starter_ratio: 0.85,
            nationality_country_id: 1,
            nationality_region: Some(ScoutingRegion::WesternEurope),
            seller_region: ScoutingRegion::WesternEurope,
            ..PlayerStance::neutral()
        }
    }

    fn appraise(stance: &PlayerStance, offer: &OfferView) -> Appraisal {
        PlayerOfferAppraisal::appraise(stance, offer, 0.0, &AppraisalConfig::default())
    }

    #[test]
    fn the_anchor_is_the_geometric_mean_of_paid_and_worth() {
        let overpaid = PlayerStance {
            current_wage: 6_000_000.0,
            fair_wage_at_current: 2_500_000.0,
            ..PlayerStance::neutral()
        };
        let anchor = PlayerOfferAppraisal::anchor(&overpaid);
        assert!(
            (anchor - 3_872_983.0).abs() < 2_000.0,
            "sqrt(6M x 2.5M) ~ 3.87M, got {anchor}"
        );
        // …and a 3M offer therefore reads as a modest cut, not a halving.
        let offer = OfferView {
            offered_wage: 3_000_000.0,
            ..OfferView::neutral()
        };
        let a = appraise(&overpaid, &offer);
        assert!(a.money > -0.15, "modest cut, got M={:+.3}", a.money);
    }

    #[test]
    fn money_is_logarithmic_so_doubling_and_halving_are_symmetric() {
        let stance = PlayerStance::neutral();
        let double = appraise(
            &stance,
            &OfferView {
                offered_wage: 2_000_000.0,
                ..OfferView::neutral()
            },
        );
        let half = appraise(
            &stance,
            &OfferView {
                offered_wage: 500_000.0,
                ..OfferView::neutral()
            },
        );
        assert!((double.money + half.money).abs() < 1e-4);
        let quadruple = appraise(
            &stance,
            &OfferView {
                offered_wage: 4_000_000.0,
                ..OfferView::neutral()
            },
        );
        assert!(
            (quadruple.money - 2.0 * double.money).abs() < 1e-4,
            "4x is worth exactly twice 2x"
        );
    }

    #[test]
    fn the_money_weight_climbs_with_the_career_clock() {
        let cfg = AppraisalConfig::default();
        let young = PlayerStance {
            age: 22,
            career_spent: 0.0,
            ..PlayerStance::neutral()
        };
        let old = PlayerStance {
            age: 34,
            career_spent: 1.0,
            contract_pressure: 1.0,
            ..PlayerStance::neutral()
        };
        let offer = OfferView {
            offered_wage: 2_000_000.0,
            ..OfferView::neutral()
        };
        let a = PlayerOfferAppraisal::appraise(&young, &offer, 0.0, &cfg);
        let b = PlayerOfferAppraisal::appraise(&old, &offer, 0.0, &cfg);
        assert!((a.money_weight - 0.30).abs() < 1e-5, "{}", a.money_weight);
        assert!((b.money_weight - 1.00).abs() < 1e-5, "{}", b.money_weight);
        assert!(b.money > a.money);
    }

    #[test]
    fn the_reservation_for_a_gulf_move_falls_with_age() {
        let ratio = |age: u8| {
            let stance = premier_league_star(age);
            let a = appraise(&stance, &gulf_offer(1.0, stance.current_wage));
            a.reservation_wage as f64 / stance.current_wage
        };
        let at25 = ratio(25);
        let at29 = ratio(29);
        let at31 = ratio(31);
        let at33 = ratio(33);
        assert!(
            at25 > at29 && at29 > at31 && at31 > at33,
            "{at25} {at29} {at31} {at33}"
        );
        // Part V: 5.7x at 25 down to 2.4x at 33. Bands, not point values —
        // the census is the truth.
        assert!((3.5..8.0).contains(&at25), "25yo asks {at25:.2}x");
        assert!((2.4..4.8).contains(&at29), "29yo asks {at29:.2}x");
        assert!((1.8..3.6).contains(&at33), "33yo asks {at33:.2}x");
    }

    #[test]
    fn a_twenty_nine_year_old_takes_four_times_his_wage_and_a_prime_starter_does_not() {
        let veteran = premier_league_star(29);
        let at4x = appraise(&veteran, &gulf_offer(4.0, veteran.current_wage));
        assert!(at4x.utility > 0.0, "{}", at4x.explain());

        let prime = premier_league_star(25);
        let same = appraise(&prime, &gulf_offer(2.0, prime.current_wage));
        assert!(same.utility < 0.0, "{}", same.explain());
    }

    #[test]
    fn a_surplus_star_takes_a_small_cut_where_a_settled_one_refuses_a_fortune() {
        // Same man, same offer — one of them has been told he is not wanted.
        let mut surplus = premier_league_star(29);
        surplus.unhappy = true;
        surplus.importance = 0.5;
        surplus.starter_ratio = 0.12;
        surplus.playing_time_gap = -0.6;
        surplus.leave_pressure = 0.6;
        let a = appraise(&surplus, &gulf_offer(2.0, surplus.current_wage));
        assert!(a.utility > 0.0, "{}", a.explain());
        assert!(
            (a.reservation_wage as f64) < surplus.current_wage,
            "he would take a cut: {}",
            a.reservation_wage
        );
    }

    #[test]
    fn a_settled_starter_refuses_a_sideways_move_at_half_wage() {
        let stance = PlayerStance {
            current_wage: 8_900_000.0,
            fair_wage_at_current: 8_900_000.0,
            age: 27,
            career_runway: 0.58,
            career_spent: 0.42,
            ambition_drive: 0.7,
            importance: 0.9,
            starter_ratio: 0.85,
            ..PlayerStance::neutral()
        };
        let offer = OfferView {
            offered_wage: 4_450_000.0,
            sporting_drop: 0.18,
            promised_status: Some(PromisedSquadStatus::FirstTeamRegular),
            ..OfferView::neutral()
        };
        let a = appraise(&stance, &offer);
        assert!(a.utility < -0.5, "{}", a.explain());
        assert!(
            a.reservation_wage as f64 > stance.current_wage,
            "he asks for a raise to move sideways: {}",
            a.reservation_wage
        );
    }

    #[test]
    fn the_benched_backup_takes_the_fair_value_cut() {
        // Part V row 1: MainBackup, 30, wage 6M on a 2.5M standing, Unh,
        // starter share 0.15, offered a FirstTeamRegular shirt at a
        // mid-table club.
        let stance = PlayerStance {
            current_wage: 6_000_000.0,
            fair_wage_at_current: 2_500_000.0,
            age: 30,
            career_runway: (34.0 - 30.0) / 12.0,
            career_spent: 1.0 - (34.0 - 30.0) / 12.0,
            ambition_drive: 0.5,
            importance: 0.35,
            starter_ratio: 0.15,
            playing_time_gap: -0.35,
            unhappy: true,
            available_soft: true,
            ..PlayerStance::neutral()
        };
        let offer = OfferView {
            offered_wage: 2_500_000.0,
            sporting_drop: 0.18,
            promised_status: Some(PromisedSquadStatus::FirstTeamRegular),
            ..OfferView::neutral()
        };
        let a = appraise(&stance, &offer);
        assert!(a.utility > 0.0, "{}", a.explain());
        assert!(
            a.reservation_wage < 3_000_000,
            "he asks for far less than the 6M he is on: {}",
            a.reservation_wage
        );
    }

    #[test]
    fn the_club_man_would_rather_fight_for_his_place() {
        // Part V row 3: backup, 27, benched, loyalty 17 at his boyhood club.
        let stance = PlayerStance {
            current_wage: 6_000_000.0,
            fair_wage_at_current: 2_500_000.0,
            age: 27,
            career_runway: (34.0 - 27.0) / 12.0,
            career_spent: 1.0 - (34.0 - 27.0) / 12.0,
            loyalty_drive: 0.85,
            at_favourite_club: true,
            days_at_club: 2000,
            importance: 0.35,
            starter_ratio: 0.15,
            playing_time_gap: -0.35,
            stay_pressure: 0.5,
            ..PlayerStance::neutral()
        };
        let offer = OfferView {
            offered_wage: 2_500_000.0,
            sporting_drop: 0.18,
            promised_status: Some(PromisedSquadStatus::FirstTeamRegular),
            ..OfferView::neutral()
        };
        let a = appraise(&stance, &offer);
        assert!(a.utility < 0.0, "{}", a.explain());
        assert_eq!(a.refusal_cause(false), TermsRefusalCause::Attachment);
    }

    #[test]
    fn home_costs_nothing_in_place_and_pays_in_home() {
        let brazilian = PlayerStance {
            nationality_country_id: 55,
            nationality_region: Some(ScoutingRegion::SouthAmerica),
            seller_country_id: 1,
            seller_continent_id: 1,
            seller_region: ScoutingRegion::WesternEurope,
            return_home_desire: 0.6,
            ..PlayerStance::neutral()
        };
        let home = OfferView {
            kind: OfferKind::Loan,
            buyer_country_id: 55,
            buyer_region: ScoutingRegion::SouthAmerica,
            prestige_drop: 0.55,
            crosses_continent: true,
            language_affinity: 1.0,
            offered_wage: brazilian.current_wage,
            ..OfferView::neutral()
        };
        let a = appraise(&brazilian, &home);
        assert_eq!(a.place, 0.0, "his own country is not a foreign posting");
        assert!(a.home > 0.6, "H={:+.3}", a.home);

        // A neighbour in his own region pays a fraction of it.
        let regional = OfferView {
            buyer_country_id: 54,
            ..home
        };
        let b = appraise(&brazilian, &regional);
        assert!(b.home > 0.0 && b.home < a.home);
        assert!(b.place < 0.0, "still abroad: P={:+.3}", b.place);
    }

    #[test]
    fn the_home_pull_is_not_a_magnet_without_the_want() {
        let settled = PlayerStance {
            nationality_country_id: 55,
            nationality_region: Some(ScoutingRegion::SouthAmerica),
            seller_country_id: 1,
            seller_continent_id: 1,
            seller_region: ScoutingRegion::WesternEurope,
            return_home_desire: 0.0,
            ..PlayerStance::neutral()
        };
        let home = OfferView {
            buyer_country_id: 55,
            buyer_region: ScoutingRegion::SouthAmerica,
            offered_wage: settled.current_wage,
            ..OfferView::neutral()
        };
        let a = appraise(&settled, &home);
        assert!(
            (a.home - 0.25).abs() < 1e-5,
            "the rumour every player entertains, and no more: {:+.3}",
            a.home
        );
    }

    #[test]
    fn push_is_capped_so_a_bad_move_is_still_refused() {
        let desperate = PlayerStance {
            available_soft: true,
            unhappy: true,
            requested: true,
            listed_by_club: true,
            contract_pressure: 1.0,
            playing_time_gap: -1.0,
            leave_pressure: 1.0,
            ..PlayerStance::neutral()
        };
        let terrible = OfferView {
            offered_wage: desperate.current_wage * 0.2,
            sporting_drop: 0.8,
            prestige_drop: 0.9,
            crosses_continent: true,
            language_affinity: 0.0,
            ..OfferView::neutral()
        };
        let a = appraise(&desperate, &terrible);
        assert!((a.push - 0.90).abs() < 1e-5, "D={:+.3}", a.push);
        assert!(a.utility < 0.0, "{}", a.explain());
    }

    #[test]
    fn a_loan_halves_the_money_and_the_sport() {
        let stance = PlayerStance::neutral();
        let permanent = OfferView {
            offered_wage: 2_000_000.0,
            sporting_drop: 0.3,
            ..OfferView::neutral()
        };
        let loan = OfferView {
            kind: OfferKind::Loan,
            ..permanent
        };
        let p = appraise(&stance, &permanent);
        let l = appraise(&stance, &loan);
        assert!((l.money - p.money * 0.5).abs() < 1e-4);
        assert!((l.sport - p.sport * 0.5).abs() < 1e-4);
    }

    #[test]
    fn the_disposition_is_drawn_once_and_stays_drawn() {
        let cfg = AppraisalConfig::default();
        let a = PlayerDisposition::for_negotiation(1, 17, 4242, 900, cfg.disposition_sigma);
        let b = PlayerDisposition::for_negotiation(1, 17, 4242, 900, cfg.disposition_sigma);
        assert_eq!(a, b, "a stance, not a die");
        let other = PlayerDisposition::for_negotiation(1, 18, 4242, 900, cfg.disposition_sigma);
        assert!(
            (a - other).abs() > f32::EPSILON,
            "a different deal is a different mood"
        );
    }

    #[test]
    fn the_disposition_spreads_but_stays_bounded() {
        let cfg = AppraisalConfig::default();
        let draws: Vec<f32> = (0..4000)
            .map(|i| {
                PlayerDisposition::for_negotiation(1, i, i * 7 + 1, 900, cfg.disposition_sigma)
            })
            .collect();
        let mean = draws.iter().sum::<f32>() / draws.len() as f32;
        let var = draws.iter().map(|d| (d - mean) * (d - mean)).sum::<f32>() / draws.len() as f32;
        assert!(mean.abs() < 0.03, "mean {mean}");
        assert!((0.15..0.26).contains(&var.sqrt()), "sigma {}", var.sqrt());
        assert!(
            draws
                .iter()
                .all(|d| d.abs() <= 3.0 * cfg.disposition_sigma + 1e-6)
        );
    }

    #[test]
    fn raising_the_offer_to_the_reservation_wins_the_yes() {
        let stance = premier_league_star(31);
        let eps = PlayerDisposition::for_negotiation(1, 5, 99, 900, 0.22);
        let cfg = AppraisalConfig::default();
        let opener = gulf_offer(1.2, stance.current_wage);
        let first = PlayerOfferAppraisal::appraise(&stance, &opener, eps, &cfg);
        assert!(!first.accepts());

        let improved = OfferView {
            offered_wage: first.reservation_wage as f64 * 1.02,
            ..opener
        };
        let second = PlayerOfferAppraisal::appraise(&stance, &improved, eps, &cfg);
        assert!(
            second.accepts(),
            "the reservation IS the demand, no re-roll: {}",
            second.explain()
        );
    }

    #[test]
    fn a_step_up_is_a_pull_and_a_big_stage_man_feels_it_most() {
        let plain = PlayerStance {
            big_stage_inclination: 0.0,
            ..PlayerStance::neutral()
        };
        let hungry = PlayerStance {
            big_stage_inclination: 1.0,
            ..plain
        };
        let step_up = OfferView {
            offered_wage: plain.current_wage,
            sporting_drop: -0.3,
            ..OfferView::neutral()
        };
        let a = appraise(&plain, &step_up);
        let b = appraise(&hungry, &step_up);
        assert!(a.sport > 0.0 && b.sport > a.sport);
    }

    #[test]
    fn unsold_months_soften_the_step_down_resistance() {
        let fresh = premier_league_star(28);
        let resigned = PlayerStance {
            market_resignation: 1.0,
            ..fresh
        };
        let offer = gulf_offer(1.0, fresh.current_wage);
        let a = appraise(&fresh, &offer);
        let b = appraise(&resigned, &offer);
        assert!(b.sport > a.sport, "{:+.3} vs {:+.3}", a.sport, b.sport);
    }

    /// Probability of acceptance across the disposition, `Φ(U / σ)`.
    /// The Part V tables are written in these terms, so the regressions
    /// are too — a single ε draw would only ever test one temperament.
    fn p_accept(utility: f32) -> f32 {
        let z = utility / AppraisalConfig::default().disposition_sigma;
        // Abramowitz & Stegun 7.1.26 error function, plenty for a band
        // assertion.
        let x = z / std::f32::consts::SQRT_2;
        let sign = if x < 0.0 { -1.0 } else { 1.0 };
        let x = x.abs();
        let t = 1.0 / (1.0 + 0.3275911 * x);
        let y = 1.0
            - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
                + 0.254829592)
                * t
                * (-x * x).exp();
        0.5 * (1.0 + sign * y)
    }

    #[test]
    fn the_money_move_odds_follow_the_age_ladder() {
        // Part II's worked example, in the other direction: the Gulf club
        // that offered a Premier League star HALF his wage and was refused
        // now offers four times it, and at 29 he goes about as often as
        // not. The old model clamped him to 5 % and rolled three times.
        let p = |age: u8, multiple: f64| {
            let stance = premier_league_star(age);
            p_accept(appraise(&stance, &gulf_offer(multiple, stance.current_wage)).utility)
        };

        // Prime age refuses at a multiple a veteran takes.
        assert!(p(25, 2.0) < 0.15, "25yo at 2x: {:.2}", p(25, 2.0));
        assert!(p(25, 3.0) < 0.35, "25yo at 3x: {:.2}", p(25, 3.0));
        // The veteran payday.
        assert!(p(29, 4.0) > 0.45, "29yo at 4x: {:.2}", p(29, 4.0));
        assert!(p(33, 3.0) > 0.40, "33yo at 3x: {:.2}", p(33, 3.0));
        // Monotone in both axes, everywhere — no birthday flips it.
        for age in 24..34u8 {
            assert!(p(age, 4.0) >= p(age, 2.0));
            assert!(p(age + 1, 3.0) >= p(age, 3.0) - 1e-4, "age {age}");
        }
    }

    #[test]
    fn a_tournament_in_view_raises_what_a_squad_edge_international_asks() {
        let base = premier_league_star(29);
        let chasing_a_squad_place = PlayerStance {
            nt_stake: 0.8,
            ..base
        };
        let offer = gulf_offer(3.0, base.current_wage);
        let quiet = appraise(&base, &offer);
        let pressed = appraise(&chasing_a_squad_place, &offer);
        assert!(pressed.utility < quiet.utility);
        assert!(pressed.reservation_wage > quiet.reservation_wage);
    }

    #[test]
    fn the_stuck_brazilian_takes_the_loan_home_and_the_settled_one_does_not() {
        // Part V's loan-home row: 21, three starts in eight months,
        // `GoHome` 0.6, `GoOutOnLoan` formed, loan-listed by his club.
        let stuck = PlayerStance {
            current_wage: 900_000.0,
            fair_wage_at_current: 900_000.0,
            age: 21,
            career_runway: 1.0,
            career_spent: 0.0,
            importance: 0.25,
            starter_ratio: 0.12,
            playing_time_gap: -0.3,
            nationality_country_id: 55,
            nationality_region: Some(ScoutingRegion::SouthAmerica),
            seller_region: ScoutingRegion::WesternEurope,
            seller_country_id: 1,
            seller_continent_id: 1,
            return_home_desire: 0.6,
            leave_pressure: 0.59,
            listed_by_club: true,
            ..PlayerStance::neutral()
        };
        let home = OfferView {
            kind: OfferKind::Loan,
            buyer_country_id: 55,
            buyer_continent_id: 3,
            buyer_region: ScoutingRegion::SouthAmerica,
            offered_wage: stuck.current_wage,
            promised_status: Some(PromisedSquadStatus::FirstTeamRegular),
            sporting_drop: 0.14,
            prestige_drop: 0.55,
            crosses_continent: true,
            language_affinity: 1.0,
            ..OfferView::neutral()
        };
        let a = appraise(&stuck, &home);
        assert!(a.utility > 0.9, "{}", a.explain());

        // A loan inside Europe is still a yes — the pull is a preference,
        // not a precondition. The market decides which one he gets.
        let belgium = OfferView {
            buyer_country_id: 32,
            buyer_continent_id: 1,
            buyer_region: ScoutingRegion::WesternEurope,
            prestige_drop: 0.0,
            crosses_continent: false,
            language_affinity: 0.45,
            ..home
        };
        assert!(appraise(&stuck, &belgium).utility > 0.3);

        // …and the same boy, settled and starting, is not sent anywhere:
        // the home pull alone barely moves him.
        let settled = PlayerStance {
            starter_ratio: 0.6,
            playing_time_gap: 0.1,
            return_home_desire: 0.0,
            leave_pressure: 0.0,
            listed_by_club: false,
            importance: 0.6,
            ..stuck
        };
        let b = appraise(&settled, &home);
        assert!(b.utility < a.utility * 0.5, "{}", b.explain());
    }

    #[test]
    fn the_refusal_names_the_axis_that_cost_him_most() {
        let stance = premier_league_star(24);
        let far_away = OfferView {
            offered_wage: stance.current_wage * 6.0,
            sporting_drop: 0.0,
            prestige_drop: 0.9,
            crosses_continent: true,
            language_affinity: 0.0,
            ..OfferView::neutral()
        };
        assert_eq!(
            appraise(&stance, &far_away).refusal_cause(false),
            TermsRefusalCause::Place
        );

        let bench = OfferView {
            offered_wage: stance.current_wage,
            promised_status: Some(PromisedSquadStatus::FirstTeamSquadRotation),
            ..OfferView::neutral()
        };
        assert_eq!(
            appraise(&stance, &bench).refusal_cause(false),
            TermsRefusalCause::Role
        );
    }

    /// B7 — a man who would sign for more money than the buyer can hold
    /// refused on the WAGE, whatever the axes say. With an offer at or
    /// above his anchor the money term is non-negative, so the axis
    /// fallback could never reach that label on its own.
    #[test]
    fn an_unreachable_demand_is_a_wage_refusal_whatever_the_axes_say() {
        let stance = PlayerStance {
            current_wage: 4_000_000.0,
            fair_wage_at_current: 4_000_000.0,
            importance: 0.9,
            loyalty_drive: 0.9,
            at_favourite_club: true,
            ..PlayerStance::neutral()
        };
        let generous = OfferView {
            offered_wage: 6_000_000.0,
            sporting_drop: 0.3,
            ..OfferView::neutral()
        };
        let appraisal = appraise(&stance, &generous);
        assert!(appraisal.money > 0.0, "the offer is a raise");
        assert_eq!(
            appraisal.refusal_cause(false),
            TermsRefusalCause::Attachment,
            "with no verdict from the buyer, the worst axis is the story"
        );
        assert_eq!(
            appraisal.refusal_cause(true),
            TermsRefusalCause::WageDemand,
            "but a club that cannot reach his number lost him on money"
        );
    }

    /// B3 — a loan pays the deal he already has, so the money axis is
    /// silent by construction on both paths.
    #[test]
    fn a_loan_offer_at_the_anchor_reads_no_money_at_all() {
        let stance = PlayerStance {
            current_wage: 900_000.0,
            fair_wage_at_current: 1_600_000.0,
            ..PlayerStance::neutral()
        };
        let loan = OfferView {
            kind: OfferKind::Loan,
            offered_wage: PlayerOfferAppraisal::anchor(&stance),
            ..OfferView::neutral()
        };
        assert!(appraise(&stance, &loan).money.abs() < 1e-6);
    }

    /// B12 — a man with no contract has no wage to blend, and blending the
    /// floor into it made every offer read as a three- or four-unit raise.
    #[test]
    fn a_man_with_no_contract_anchors_on_what_he_is_worth() {
        let free_agent = PlayerStance {
            current_wage: 0.0,
            fair_wage_at_current: 1_000_000.0,
            ..PlayerStance::neutral()
        };
        assert_eq!(PlayerOfferAppraisal::anchor(&free_agent), 1_000_000.0);
        let market_offer = OfferView {
            offered_wage: 1_000_000.0,
            ..OfferView::neutral()
        };
        assert!(appraise(&free_agent, &market_offer).money.abs() < 1e-6);
    }

    /// B6 — a free agent is tied to nobody and out of contract, and both
    /// facts have to be said: the neutral stance is a settled player on a
    /// running deal.
    #[test]
    fn a_free_agent_pays_no_attachment_and_carries_full_contract_pressure() {
        let fa = PlayerStance::from_terms(28, 0.6, 0.0, 1_000_000.0);
        assert_eq!(fa.loyalty_drive, 0.0);
        assert_eq!(fa.contract_pressure, 1.0);
        assert!(fa.available_soft);
        let offer = OfferView {
            offered_wage: 1_000_000.0,
            ..OfferView::neutral()
        };
        let appraisal = appraise(&fa, &offer);
        assert_eq!(appraisal.attachment, 0.0, "he is tied to nobody");
        assert!(appraisal.push > 0.0, "his deal has run out");
    }

    /// B9 — an unstamped passport has no home, and must not borrow the
    /// club's region for one.
    #[test]
    fn an_unknown_nationality_is_never_at_home() {
        let unknown = PlayerStance {
            nationality_country_id: 0,
            nationality_region: None,
            seller_country_id: 10,
            ..PlayerStance::neutral()
        };
        let offer = OfferView {
            buyer_country_id: 55,
            buyer_region: ScoutingRegion::SouthAmerica,
            ..OfferView::neutral()
        };
        assert!(!unknown.is_home_country(&offer));
        assert!(!unknown.is_home_region(&offer));
        assert_eq!(appraise(&unknown, &offer).home, 0.0);
    }

    /// B10 — the dressing room only changes language when the country
    /// does. Arsenal → Chelsea cost a Brazilian 0.12·(1 − affinity).
    #[test]
    fn language_is_charged_across_a_border_and_not_down_the_road() {
        let brazilian_in_england = PlayerStance {
            nationality_country_id: 55,
            nationality_region: Some(ScoutingRegion::SouthAmerica),
            seller_country_id: 1,
            seller_region: ScoutingRegion::WesternEurope,
            ..PlayerStance::neutral()
        };
        let domestic = OfferView {
            offered_wage: brazilian_in_england.current_wage,
            buyer_country_id: 1,
            buyer_region: ScoutingRegion::WesternEurope,
            language_affinity: 0.2,
            ..OfferView::neutral()
        };
        assert_eq!(appraise(&brazilian_in_england, &domestic).place, 0.0);

        let abroad = OfferView {
            buyer_country_id: 2,
            ..domestic
        };
        assert!(appraise(&brazilian_in_england, &abroad).place < 0.0);
    }

    /// B8 — two countries allocate negotiation ids from their own
    /// counters, so the id alone is not a seed.
    #[test]
    fn two_buyers_in_two_countries_do_not_share_a_temperament() {
        let a = PlayerDisposition::for_negotiation(1, 17, 4242, 900, 0.22);
        let b = PlayerDisposition::for_negotiation(2, 17, 4242, 901, 0.22);
        assert!((a - b).abs() > 1e-6, "{a} vs {b}");
        // …and it is still deterministic for the same negotiation.
        assert_eq!(
            a,
            PlayerDisposition::for_negotiation(1, 17, 4242, 900, 0.22)
        );
    }
}
