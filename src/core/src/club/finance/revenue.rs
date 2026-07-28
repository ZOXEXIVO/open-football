//! Club revenue model.
//!
//! Every income line here is **continuous** in the club's inputs. The
//! previous model looked revenue up from a six-bucket `ReputationLevel`
//! ladder, so slipping one tier cut broadcast income 60% and commercial
//! income 70% in a single month, with no corresponding fall in the wage
//! bill. That cliff — not any one bad season — is what turned a mid-table
//! slump into an unrecoverable spiral: revenue collapsed, the deficit
//! widened, debt interest compounded, and the club could never buy its way
//! back out.
//!
//! The three lines model how football money actually arrives:
//!
//! * **Broadcast** is a *league* pool, not a club property. Everyone in a
//!   division draws from the same pot — an equal share, a merit payment on
//!   final position, and a facility fee for being televised often. A club
//!   whose reputation dips still banks its division's money. Relegation is
//!   cushioned by parachute payments, exactly as it is in reality.
//! * **Matchday** is capacity × utilisation × price. Capacity is a physical
//!   property of the club ([`ClubFacilities::stadium_capacity`]); only
//!   utilisation responds to form and standing.
//! * **Commercial** (sponsorship and merchandising) follows a smooth cubic
//!   in reputation — the one line that *should* track prestige closely,
//!   because shirt and sponsor money genuinely does.

use crate::club::ClubFacilities;

/// Everything the revenue model needs about a club and its surroundings for
/// one monthly tick.
#[derive(Debug, Clone)]
pub struct RevenueInputs {
    /// Blended club standing, 0.0..1.0 (`TeamReputation::overall_score`).
    pub reputation_score: f32,
    /// Division the main team plays in; 1 = top flight.
    pub league_tier: u8,
    /// Current league position, 1-based. 0 = unknown.
    pub league_position: u16,
    /// Teams in the division. 0 = unknown.
    pub league_size: u16,
    /// Country broadcast market strength (`tv_revenue_multiplier`).
    pub tv_market: f32,
    /// Country commercial market strength (`sponsorship_market_strength`).
    pub sponsorship_market: f32,
    /// Country attendance propensity (`stadium_attendance_factor`).
    pub attendance_factor: f32,
    /// Local price level — England 1.5, Colombia 0.4.
    pub price_level: f32,
    /// Ground capacity.
    pub stadium_capacity: u32,
    /// Win rate across the last ~5 matches, 0.0..1.0.
    pub recent_wins_ratio: f32,
    /// Home matches actually played in the month being billed.
    pub home_matches: u32,
    /// Parachute entitlement carried down from a higher division.
    pub parachute: Option<ParachuteEntitlement>,
}

/// A club's monthly revenue, decomposed for the ledger and the UI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MonthlyRevenue {
    /// Equal share + facility fee — the part that doesn't move with results.
    pub broadcast_base: i64,
    /// Merit payment above what a bottom-of-the-table finish would earn.
    pub broadcast_merit: i64,
    /// Parachute payment from a recent relegation.
    pub parachute: i64,
    pub matchday: i64,
    pub commercial: i64,
    /// Gate that produced [`Self::matchday`], so the club's observed
    /// average attendance can be kept honest.
    pub attendance: u32,
}

impl MonthlyRevenue {
    pub fn total(&self) -> i64 {
        self.broadcast_base
            + self.broadcast_merit
            + self.parachute
            + self.matchday
            + self.commercial
    }
}

/// Parachute payments: a relegated club keeps a declining share of the
/// division it fell out of, for three seasons.
///
/// This is the single most important anti-cliff mechanism in the model.
/// Without it, relegation is a 90% revenue cut against a wage bill fixed by
/// contracts already signed — arithmetically unsurvivable. With it, the club
/// has three seasons to shed wages down to its new level, which is precisely
/// the window real parachute payments exist to provide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParachuteEntitlement {
    /// Division the club was relegated *from*; the payment is a share of
    /// that tier's pool.
    pub from_tier: u8,
    /// Seasons elapsed since relegation: 0 = the season immediately after.
    pub seasons_elapsed: u8,
}

impl ParachuteEntitlement {
    /// Share of the higher division's per-club pool paid in each of the
    /// three post-relegation seasons. Past the third, entitlement ends.
    pub fn share(&self) -> f64 {
        match self.seasons_elapsed {
            0 => 0.55,
            1 => 0.40,
            2 => 0.25,
            _ => 0.0,
        }
    }

    /// True once the entitlement has run out and can be dropped.
    pub fn is_expired(&self) -> bool {
        self.seasons_elapsed >= 3
    }

    /// Advance one season. Returns `None` when the entitlement expires.
    pub fn advanced(self) -> Option<Self> {
        let next = ParachuteEntitlement {
            from_tier: self.from_tier,
            seasons_elapsed: self.seasons_elapsed.saturating_add(1),
        };
        if next.is_expired() { None } else { Some(next) }
    }
}

/// The revenue model itself. Stateless — every constant is a calibration
/// knob and lives here rather than being inlined at the call site.
pub struct RevenueModel;

impl RevenueModel {
    // ── Broadcast ────────────────────────────────────────────────────────

    /// Annual per-club broadcast pool for a top-flight club in a maximal
    /// market (`tv_market` = 1.0), before the equal/merit/facility split.
    ///
    /// Sized against real top-five-league deals: a Premier League club draws
    /// roughly £110–170M a season. `tv_market` is the country's
    /// `tv_revenue_multiplier`, which is the *square* of its normalised
    /// reputation, so a top nation lands near 0.85–0.90 of this figure.
    const TOP_FLIGHT_POOL: f64 = 130_000_000.0;

    /// Pool as a fraction of the top flight, by division. Real ladders drop
    /// steeply — a Championship central deal is a small fraction of the
    /// Premier League's — but not to nothing, and parachute payments carry
    /// most of the weight for the recently relegated.
    fn tier_pool_fraction(tier: u8) -> f64 {
        match tier.max(1) {
            1 => 1.00,
            2 => 0.12,
            3 => 0.035,
            4 => 0.012,
            _ => 0.005,
        }
    }

    /// Annual broadcast pool available to one club in this division.
    fn annual_pool(tier: u8, tv_market: f32) -> f64 {
        Self::TOP_FLIGHT_POOL * Self::tier_pool_fraction(tier) * tv_market.max(0.0) as f64
    }

    /// Merit multiplier on final league position: champions draw roughly
    /// 1.9× the divisional average, the bottom club roughly 0.35×. Linear
    /// in rank, which is how merit ladders are actually written.
    fn merit_multiplier(position: u16, size: u16) -> f64 {
        if position == 0 || size <= 1 {
            return 1.0;
        }
        let n = size as f64;
        let p = (position as f64).clamp(1.0, n);
        // rank_share: 1.0 for first, 0.0 for last.
        let rank_share = (n - p) / (n - 1.0);
        0.35 + rank_share * 1.55
    }

    /// Facility-fee multiplier: clubs that are televised more often earn
    /// more of the pool. Reputation is the proxy, centred so a mid-division
    /// side earns its plain share.
    fn facility_multiplier(reputation_score: f32) -> f64 {
        (0.35 + reputation_score.clamp(0.0, 1.0) as f64 * 1.30).clamp(0.35, 1.75)
    }

    /// Broadcast income for one month, split into the results-independent
    /// base and the merit slice earned above a bottom-place finish.
    pub fn broadcast(inputs: &RevenueInputs) -> (i64, i64) {
        let pool = Self::annual_pool(inputs.league_tier, inputs.tv_market) / 12.0;
        if pool <= 0.0 {
            return (0, 0);
        }

        // Split: half flat, 30% on merit, 20% on televised appearances.
        let equal_share = pool * 0.50;
        let merit_pot = pool * 0.30;
        let facility_pot = pool * 0.20;

        let facility = facility_pot * Self::facility_multiplier(inputs.reputation_score);
        let merit = merit_pot * Self::merit_multiplier(inputs.league_position, inputs.league_size);

        // The base is what the club banks regardless of where it finishes;
        // the merit line shows what the table was worth on top of that.
        let floor_merit = merit_pot * 0.35;
        let base = equal_share + facility + floor_merit;
        let merit_premium = (merit - floor_merit).max(0.0);

        (base as i64, merit_premium as i64)
    }

    /// Parachute payment for one month, or zero when there's no entitlement.
    pub fn parachute(inputs: &RevenueInputs) -> i64 {
        let Some(entitlement) = inputs.parachute else {
            return 0;
        };
        let share = entitlement.share();
        if share <= 0.0 {
            return 0;
        }
        let from_pool = Self::annual_pool(entitlement.from_tier, inputs.tv_market) / 12.0;
        (from_pool * share) as i64
    }

    // ── Matchday ─────────────────────────────────────────────────────────

    /// Baseline share of the ground that turns up before form is applied.
    /// Big clubs sell out habitually; small clubs rattle around.
    fn base_utilisation(reputation_score: f32) -> f32 {
        0.52 + reputation_score.clamp(0.0, 1.0) * 0.42
    }

    /// Average revenue per attendee: ticket face value blended with
    /// hospitality and corporate, which is why a big club's per-head figure
    /// runs well above its cheapest seat.
    fn revenue_per_attendee(reputation_score: f32, price_level: f32) -> f64 {
        let base = 9.0 + reputation_score.clamp(0.0, 1.0) as f64 * 62.0;
        base * price_level.max(0.1) as f64
    }

    /// Matchday income for the month, plus the gate that produced it.
    pub fn matchday(inputs: &RevenueInputs, facilities_multiplier: f32) -> (i64, u32) {
        if inputs.home_matches == 0 || inputs.stadium_capacity == 0 {
            return (0, 0);
        }

        let utilisation = (Self::base_utilisation(inputs.reputation_score)
            * facilities_multiplier
            * inputs.attendance_factor.clamp(0.3, 1.2))
        .clamp(0.20, 1.0);

        let attendance = (inputs.stadium_capacity as f32 * utilisation).round() as u32;
        let per_head = Self::revenue_per_attendee(inputs.reputation_score, inputs.price_level);
        let revenue = attendance as f64 * per_head * inputs.home_matches as f64;

        (revenue as i64, attendance)
    }

    // ── Commercial ───────────────────────────────────────────────────────

    /// Annual commercial ceiling — merchandising, retail, and the
    /// non-contracted commercial book — for a maximal-reputation club in a
    /// maximal market. Named contracts in [`crate::ClubSponsorship`] are
    /// billed separately and sit on top of this.
    const COMMERCIAL_SCALE: f64 = 480_000_000.0;

    /// Commercial income for one month. Cubic in reputation: prestige
    /// genuinely does drive shirt sales super-linearly, and a smooth curve
    /// means a club sliding down the table loses this income gradually
    /// rather than in one 70% step.
    pub fn commercial(inputs: &RevenueInputs) -> i64 {
        let rep = inputs.reputation_score.clamp(0.0, 1.0) as f64;
        let annual = Self::COMMERCIAL_SCALE
            * rep.powi(3)
            * inputs.sponsorship_market.max(0.0) as f64
            // Commercial income is far less sensitive to local price levels
            // than a matchday ticket is — a shirt sells globally.
            * (inputs.price_level.max(0.1) as f64).sqrt();
        (annual / 12.0) as i64
    }

    // ── Aggregate ────────────────────────────────────────────────────────

    /// Full monthly revenue for a club.
    pub fn monthly(inputs: &RevenueInputs, facilities_multiplier: f32) -> MonthlyRevenue {
        let (broadcast_base, broadcast_merit) = Self::broadcast(inputs);
        let (matchday, attendance) = Self::matchday(inputs, facilities_multiplier);
        MonthlyRevenue {
            broadcast_base,
            broadcast_merit,
            parachute: Self::parachute(inputs),
            matchday,
            commercial: Self::commercial(inputs),
            attendance,
        }
    }

    /// Operating overhead: administration, community, marketing and
    /// infrastructure. A share of revenue plus a floor that scales with the
    /// club's own standing rather than a tier lookup.
    pub fn operating_overhead(monthly_income: i64, reputation_score: f32) -> i64 {
        let variable = (monthly_income.max(0) as f64) * 0.12;
        let rep = reputation_score.clamp(0.0, 1.0) as f64;
        let fixed = 6_000.0 + rep * rep * 900_000.0;
        (variable + fixed) as i64
    }
}

/// Convenience: the utilisation swing a club's form and standing produce,
/// reusing the existing supporter-mood curve on [`ClubFacilities`].
pub fn attendance_form_multiplier(
    facilities: &ClubFacilities,
    recent_wins_ratio: f32,
    league_position: u16,
    league_size: u16,
) -> f32 {
    facilities.dynamic_attendance_multiplier(recent_wins_ratio, league_position, league_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elite_inputs() -> RevenueInputs {
        RevenueInputs {
            reputation_score: 0.85,
            league_tier: 1,
            league_position: 2,
            league_size: 20,
            tv_market: 0.87,
            sponsorship_market: 0.87,
            attendance_factor: 0.9,
            price_level: 1.2,
            stadium_capacity: 75_000,
            recent_wins_ratio: 0.7,
            home_matches: 3,
            parachute: None,
        }
    }

    #[test]
    fn elite_club_annual_revenue_is_realistic() {
        let inputs = elite_inputs();
        let monthly = RevenueModel::monthly(&inputs, 1.05);
        // 3 home matches a month is the in-season cadence; annualising the
        // in-season month overstates slightly, so compare the shape not the
        // exact total. A European giant should clear half a billion a year.
        let annual = monthly.total() * 12;
        assert!(
            annual > 450_000_000 && annual < 1_100_000_000,
            "elite annual revenue out of band: {annual}"
        );
    }

    #[test]
    fn revenue_degrades_smoothly_not_in_cliffs() {
        // The whole point of the rewrite: walking reputation down must not
        // produce a step change. Every 0.05 drop should cost a bounded
        // fraction of revenue, never the 60-90% the tier ladder charged.
        let mut previous: Option<i64> = None;
        let mut rep = 0.90f32;
        while rep >= 0.35 {
            let mut inputs = elite_inputs();
            inputs.reputation_score = rep;
            let total = RevenueModel::monthly(&inputs, 1.0).total();
            if let Some(prev) = previous {
                let drop = (prev - total) as f64 / prev as f64;
                assert!(
                    drop < 0.20,
                    "revenue fell {:.0}% in one 0.05 reputation step at rep {rep}",
                    drop * 100.0
                );
            }
            previous = Some(total);
            rep -= 0.05;
        }
    }

    #[test]
    fn relegation_is_cushioned_by_parachute_payments() {
        let top_flight = elite_inputs();
        let (base, merit) = RevenueModel::broadcast(&top_flight);
        let top_broadcast = base + merit;

        // Same club, relegated, with a first-season parachute entitlement.
        let mut relegated = elite_inputs();
        relegated.league_tier = 2;
        relegated.league_position = 8;
        relegated.parachute = Some(ParachuteEntitlement {
            from_tier: 1,
            seasons_elapsed: 0,
        });
        let (rb, rm) = RevenueModel::broadcast(&relegated);
        let relegated_broadcast = rb + rm + RevenueModel::parachute(&relegated);

        let retained = relegated_broadcast as f64 / top_broadcast as f64;
        assert!(
            retained > 0.50,
            "a freshly relegated club kept only {:.0}% of its broadcast income",
            retained * 100.0
        );
    }

    #[test]
    fn parachute_entitlement_expires_after_three_seasons() {
        let mut ent = Some(ParachuteEntitlement {
            from_tier: 1,
            seasons_elapsed: 0,
        });
        let mut seasons = 0;
        while let Some(e) = ent {
            assert!(e.share() > 0.0);
            ent = e.advanced();
            seasons += 1;
        }
        assert_eq!(
            seasons, 3,
            "parachutes should run for exactly three seasons"
        );
    }

    #[test]
    fn stadium_capacity_is_independent_of_reputation() {
        // The regression that shrank Anfield to 22,000 seats: matchday
        // revenue must fall only through utilisation, never through a
        // capacity that tracks reputation.
        let mut strong = elite_inputs();
        let mut weak = elite_inputs();
        weak.reputation_score = 0.45;

        let (strong_gate, strong_att) = RevenueModel::matchday(&strong, 1.0);
        let (_, weak_att) = RevenueModel::matchday(&weak, 1.0);
        assert!(strong_att > weak_att);
        // Utilisation floor means even a slumping club fills a third of the
        // ground — a far cry from losing 60% of capacity outright.
        assert!(
            weak_att as f64 > strong_att as f64 * 0.6,
            "utilisation swing too violent: {strong_att} → {weak_att}"
        );

        strong.home_matches = 0;
        assert_eq!(RevenueModel::matchday(&strong, 1.0).0, 0);
        let _ = strong_gate;
    }

    #[test]
    fn broadcast_is_a_league_pool_not_a_club_property() {
        // Two clubs in the same division with very different standing must
        // draw comparable broadcast money — that's what a central deal is.
        let mut giant = elite_inputs();
        giant.league_position = 1;
        let mut minnow = elite_inputs();
        minnow.reputation_score = 0.42;
        minnow.league_position = 17;

        let (gb, gm) = RevenueModel::broadcast(&giant);
        let (mb, mm) = RevenueModel::broadcast(&minnow);
        let ratio = (gb + gm) as f64 / (mb + mm) as f64;
        assert!(
            ratio < 2.5,
            "same-division broadcast spread of {ratio:.1}x is not a central deal"
        );
    }

    #[test]
    fn lower_divisions_earn_less_but_not_nothing() {
        let mut tier2 = elite_inputs();
        tier2.league_tier = 2;
        tier2.reputation_score = 0.40;
        let (b, m) = RevenueModel::broadcast(&tier2);
        assert!(
            b + m > 0,
            "a second-tier club must still earn broadcast money"
        );

        let mut tier1 = tier2.clone();
        tier1.league_tier = 1;
        let (b1, m1) = RevenueModel::broadcast(&tier1);
        assert!(b1 + m1 > (b + m) * 4);
    }

    /// Annualised revenue for an archetypal club, including the named
    /// sponsorship book that is billed separately from `commercial`.
    fn annual_revenue(inputs: &RevenueInputs, sponsorship_book: i64) -> i64 {
        // A season runs ~19 home matches across ~10 active months.
        let per_month = RevenueModel::monthly(inputs, 1.0);
        let non_matchday =
            (per_month.broadcast_base + per_month.broadcast_merit + per_month.commercial) * 12;
        let matchday_per_match = if inputs.home_matches > 0 {
            per_month.matchday / inputs.home_matches as i64
        } else {
            0
        };
        non_matchday + matchday_per_match * 19 + sponsorship_book
    }

    #[test]
    fn wage_to_revenue_lands_in_a_survivable_band() {
        // The structural failure the whole rewrite exists to fix: under the
        // old model even Bayern — the healthiest club in the world — ran a
        // wage bill at 94% of revenue and lost £240M a year before a penny
        // of debt interest. Real clubs run 50-70%.
        //
        // Wage bills below are the figures the live simulation was actually
        // producing, so this asserts the revenue side can now carry them.
        struct Case {
            name: &'static str,
            inputs: RevenueInputs,
            sponsorship_book: i64,
            annual_wages: i64,
        }

        let cases = vec![
            Case {
                name: "european giant",
                inputs: RevenueInputs {
                    reputation_score: 0.86,
                    league_tier: 1,
                    league_position: 1,
                    league_size: 20,
                    tv_market: 0.87,
                    sponsorship_market: 0.87,
                    attendance_factor: 0.9,
                    price_level: 1.2,
                    stadium_capacity: 80_000,
                    recent_wins_ratio: 0.75,
                    home_matches: 3,
                    parachute: None,
                },
                sponsorship_book: 90_000_000,
                annual_wages: 430_000_000,
            },
            Case {
                name: "established top-flight club",
                inputs: RevenueInputs {
                    reputation_score: 0.60,
                    league_tier: 1,
                    league_position: 9,
                    league_size: 20,
                    tv_market: 0.87,
                    sponsorship_market: 0.87,
                    attendance_factor: 0.9,
                    price_level: 1.5,
                    stadium_capacity: 32_000,
                    recent_wins_ratio: 0.45,
                    home_matches: 3,
                    parachute: None,
                },
                sponsorship_book: 14_000_000,
                annual_wages: 95_000_000,
            },
        ];

        for case in cases {
            let revenue = annual_revenue(&case.inputs, case.sponsorship_book);
            let ratio = case.annual_wages as f64 / revenue as f64;
            assert!(
                ratio < 0.85,
                "{}: wages are {:.0}% of revenue ({} vs {}) — structurally unprofitable",
                case.name,
                ratio * 100.0,
                case.annual_wages,
                revenue
            );
        }
    }

    #[test]
    fn a_relegated_club_can_survive_its_first_season_down() {
        // Burnley's real numbers from the broken build: $240K/month income
        // against a $52.7M wage bill. With parachute payments and a ground
        // that doesn't shrink, the first season down must be survivable
        // while the wage bill unwinds.
        let inputs = RevenueInputs {
            reputation_score: 0.34,
            league_tier: 2,
            league_position: 8,
            league_size: 24,
            tv_market: 0.87,
            sponsorship_market: 0.87,
            attendance_factor: 0.9,
            price_level: 1.5,
            stadium_capacity: 22_000,
            recent_wins_ratio: 0.4,
            home_matches: 3,
            parachute: Some(ParachuteEntitlement {
                from_tier: 1,
                seasons_elapsed: 0,
            }),
        };
        let per_month = RevenueModel::monthly(&inputs, 1.0);
        let annual = (per_month.broadcast_base
            + per_month.broadcast_merit
            + per_month.parachute
            + per_month.commercial)
            * 12
            + (per_month.matchday / 3) * 23;

        assert!(
            annual > 55_000_000,
            "a freshly relegated club earned only {annual} a year — it cannot service any \
             realistic inherited wage bill"
        );
    }

    #[test]
    fn overhead_scales_with_revenue_not_wealth() {
        let small = RevenueModel::operating_overhead(200_000, 0.2);
        let large = RevenueModel::operating_overhead(30_000_000, 0.85);
        assert!(large > small * 10);
        // Overhead must never swallow revenue whole.
        assert!(large < 30_000_000 / 2);
    }
}
