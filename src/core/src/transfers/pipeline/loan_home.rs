//! The loan home — seeing it, wanting it, preferring it.
//!
//! A foreign prospect at a stronger club, a season in, with three starts
//! and no language, goes home on loan. It happens every window: Gabigol to
//! Santos, Vitinho to Flamengo, Gerson to Flamengo, Kaio Jorge to Cruzeiro,
//! Wesley to Internacional, Lainez to Tigres. It could not happen here at
//! all, for four separate reasons, none of which was a decision anybody
//! made:
//!
//! 1. **Nationality was not on the market summary.** `country_id`,
//!    `continent_id` and `region` were all stamped from the player's CLUB,
//!    so a Brazilian at Arsenal was a Western European to every scan.
//! 2. **The parent read no wish.** `identify_loan_outs` triggered on
//!    potential, depth rank and January; homesickness, adaptation and the
//!    mind's own `GoHome` reached none of it.
//! 3. **The home club could not see him.** A Brazilian club's scouts know
//!    South America and its corridors, not Western Europe — so the league
//!    he came through had no way to remember one of its own exports.
//! 4. **And if it could, he would refuse.** Loan personal terms started at
//!    45 with no wage term and then subtracted 60–91 points of region
//!    prestige, flooring at 5 %. Going home was the single least likely
//!    destination on the board.
//!
//! This module fixes 2, 3 and the preference half of 4 (the acceptance
//! half is [`super::appraisal`]). **Nothing here is a Brazil rule.** "Home"
//! is `Player.country_id == borrower country`; "home region" is his
//! passport's [`ScoutingRegion`]. The LEVEL gates are untouched — a
//! Ghanaian at a Premier League club still cannot be loaned into a
//! 3400-reputation league, because `LoanDestinationLevel` says no, and
//! that is correct: he should be loaned inside Europe instead, and the
//! census should show home-region loans for weak-home-league players near
//! zero (memory `loan_market_argmax_predictability`: gates read truth,
//! choices read belief).

use super::LoanDestinationPreference;
use super::appraisal_inputs::PlayerStanceBuilder;
use super::trace::MarketSwitches;
use crate::Player;
use crate::club::player::mind::{GoalKind, MindSituation};
use crate::club::player::statistics::StuckCareerScan;
use crate::context::HomeLeagueTable;
use crate::transfers::ScoutingRegion;
use chrono::NaiveDate;

/// Where the squad being evaluated actually is.
///
/// Threaded into the loan-out sweep because a club carries no country of
/// its own, and "is this player a foreigner here?" cannot be answered
/// without one.
#[derive(Debug, Clone, Copy)]
pub struct SquadHomeContext<'a> {
    pub country_id: u32,
    pub country_code: &'a str,
    pub continent_id: u32,
    /// What every country's best league is worth, republished on the
    /// country each tick. Borrowed, so this stays `Copy` and the sweep
    /// costs an `Arc` deref rather than a map clone per club.
    ///
    /// The parent needs it to answer a question about the CANDIDATE's
    /// passport, not about itself: "is the league he came through strong
    /// enough to warehouse him in?" No country can answer that from inside
    /// its own borrow, which is why the world publishes the table.
    pub home_leagues: &'a HomeLeagueTable,
}

impl SquadHomeContext<'_> {
    /// The region the club plays in.
    pub fn region(&self) -> ScoutingRegion {
        ScoutingRegion::from_country(self.continent_id, self.country_code)
    }

    /// Is this player a foreigner at this club?
    pub fn is_foreign(&self, player: &Player) -> bool {
        player.country_id != 0 && player.country_id != self.country_id
    }

    /// Standard of the top flight in the country this player is FROM.
    pub fn home_league_reputation(&self, player: &Player) -> u16 {
        self.home_leagues.reputation_of(player.country_id)
    }
}

/// A player's own homesickness, computed once in the weekly tick and read
/// as two fields everywhere else.
///
/// The pool builder used to build a `MindSituation` and scan
/// `recent_events` for every player in the world every day, and then build
/// the situation a SECOND time for anyone posted. Neither is a per-day
/// quantity: the mind thinks weekly, and `WantsReturnHome` fires on a
/// 60-day cooldown. So it is read where the mind already builds its
/// picture, and everything downstream reads a field.
#[derive(Debug, Clone, Copy, Default)]
pub struct HomePull {
    /// 0..1 — the mind's `GoHome`, a recent `WantsReturnHome` mood, or raw
    /// cultural isolation, whichever is loudest.
    pub desire: f32,
    /// The want has formed AND he is actually playing abroad — the
    /// condition on which a parent posts him to the world.
    pub wanted: bool,
    /// When it was last read. `None` before his first weekly tick.
    pub computed_on: Option<NaiveDate>,
}

impl HomePull {
    /// A year in the side, playing this share of the matches, is a man who
    /// has settled — whatever he was saying when he arrived.
    pub const SETTLED_TENURE_DAYS: u16 = 365;
    pub const SETTLED_STARTER_SHARE: f32 = 0.4;

    /// Read him, at the point the mind has already built its picture.
    pub fn read(player: &Player, situation: &MindSituation, date: NaiveDate) -> Self {
        let desire = player
            .mind
            .pressure_of(GoalKind::GoHome)
            .max(PlayerStanceBuilder::home_mood_desire(player, situation))
            .clamp(0.0, 1.0);
        // The want clears itself. A move resets `days_at_club`, and a year
        // of regular football answers the question the posting asked — so
        // nothing has to remember to take the flag down.
        let settled = situation.days_at_club >= Self::SETTLED_TENURE_DAYS
            && situation.starter_ratio >= Self::SETTLED_STARTER_SHARE;
        HomePull {
            desire,
            wanted: situation.is_abroad && !settled && desire >= HomeLoanGates::WANTS_HOME_BAR,
            computed_on: Some(date),
        }
    }
}

/// How badly a young foreigner has failed to settle — and where he would
/// rather be.
///
/// The three axes are read as a MAX, not a sum: wanting to go home, having
/// adapted badly and not playing are three views of one problem, and
/// adding them would make a homesick benched player four times the case a
/// merely homesick one is. Scaled by how much runway he has left, because
/// this is a development decision: a 21-year-old is sent somewhere to
/// play, a 25-year-old is closer to being sold.
#[derive(Debug, Clone, Copy)]
pub struct UnsettledAbroadScan {
    /// 0..1 — how badly he wants to go home.
    pub home_desire: f32,
    /// 0..1 — how far short of the settling bar his adaptation falls.
    pub adaptation_shortfall: f32,
    /// 0..1 — how far short of a fifth of the starts he is.
    pub minutes_shortfall: f32,
    /// The blended read, 0..1.
    pub unsettled: f32,
    /// Days he has actually been this club's player.
    pub tenure_days: u16,
    pub preference: LoanDestinationPreference,
}

impl UnsettledAbroadScan {
    /// Oldest a player can be and still be *sent somewhere to settle*
    /// rather than sold. Past this the club's answer to a man who has not
    /// worked out is the market, not a loan.
    pub const MAX_AGE: u8 = 25;

    /// Below this share of starts he is not playing, whatever the reason.
    /// Part I.3's population is "foreign U23 signings with start share
    /// < 20 % after ≥ 180 days"; the shortfall ramps from a quarter so
    /// the band sits inside the ramp rather than on its edge.
    pub const STARTER_BAR: f32 = 0.25;

    /// Adaptation below this is a man who has not settled. The same 40
    /// the chronic-adaptation detector and the career-desire evidence
    /// row already use.
    pub const ADAPTATION_BAR: f32 = 40.0;

    /// Days before a bad start is a verdict rather than a settling-in
    /// period. Six months — a window and a half.
    pub const MIN_TENURE_DAYS: u16 = 180;

    /// Blended read at which the parent acts.
    pub const CANDIDATE_BAR: f32 = 0.4;

    /// Oldest a player is still being SENT SOMEWHERE TO PLAY rather than
    /// covered for. The one development band the loan market has: both
    /// unsolicited-target classifiers read it, so a boy who is a
    /// development loan to one branch is never cover to another.
    pub const DEVELOPMENT_AGE: u8 = 23;

    /// A home top flight at or above this reputation is a league a
    /// development prospect can be warehoused in and still be SEEN.
    ///
    /// 6000 is the split the design's Part VI loan-home band is written
    /// against — "30–50 % home country for men whose home top flight is
    /// ≥ 6000, and near zero for the rest" — and it is the same number the
    /// census buckets its stuck cohort by. Roughly the top fifteen
    /// competitions in the world plus the strongest South American ones,
    /// which is exactly the population that takes these loans (Brazil,
    /// Argentina, Portugal, the Netherlands, Turkey…). A Ghanaian at a
    /// Premier League club falls below it and is loaned inside Europe
    /// instead, which is correct and what the level gates already say.
    pub const HOME_LEAGUE_BAR: u16 = 6_000;

    /// Read a player. `adaptation_score` is passed in because it needs
    /// squad context the loan sweep does not hold; `None` leaves that axis
    /// silent rather than reading an unknown as a failure.
    pub fn read(
        player: &Player,
        home: &SquadHomeContext<'_>,
        adaptation_score: Option<f32>,
        date: NaiveDate,
    ) -> Option<Self> {
        if !home.is_foreign(player) {
            return None;
        }
        let tenure_days = StuckCareerScan::club_tenure_days(player, date)
            .unwrap_or(i64::from(u16::MAX))
            .clamp(0, i64::from(u16::MAX)) as u16;

        // The mind is the source of the want; the mood is the channel that
        // predates it. Taking the MAX rather than the sum keeps a player
        // who is homesick in both models from counting twice.
        let situation = player.mind_situation(date, home.country_id, home.country_code);
        let home_desire = player
            .mind
            .pressure_of(GoalKind::GoHome)
            .max(PlayerStanceBuilder::home_mood_desire(player, &situation))
            .clamp(0.0, 1.0);

        let adaptation_shortfall = adaptation_score
            .map(|s| ((Self::ADAPTATION_BAR - s) / Self::ADAPTATION_BAR).clamp(0.0, 1.0))
            .unwrap_or(0.0);

        let minutes_shortfall = ((Self::STARTER_BAR - player.happiness.starter_ratio)
            / Self::STARTER_BAR)
            .clamp(0.0, 1.0);

        let runway = situation.career_runway();
        let unsettled = (home_desire.max(adaptation_shortfall).max(minutes_shortfall)
            * (0.5 + 0.5 * runway))
            .clamp(0.0, 1.0);

        Some(UnsettledAbroadScan {
            home_desire,
            adaptation_shortfall,
            minutes_shortfall,
            unsettled,
            tenure_days,
            preference: Self::preference_for(
                home_desire,
                home.home_league_reputation(player),
                player.home_region(),
                home.region(),
                situation.age <= Self::DEVELOPMENT_AGE,
            ),
        })
    }

    /// Does the parent act on this?
    pub fn is_candidate(&self) -> bool {
        self.unsettled >= Self::CANDIDATE_BAR && self.tenure_days >= Self::MIN_TENURE_DAYS
    }

    /// Where the parent would send him.
    ///
    /// This used to read the homesickness axis alone, and the scan is a
    /// MAX of three: a foreign prospect on 10 % of starts IS a candidate,
    /// but with no mood at all he scored below the 0.25 floor, came out
    /// `Any`, was never posted, and was loaned "elsewhere". That is the
    /// 66 % cell the census printed.
    ///
    /// Part I.3's second initiator is the PARENT's decision — warehouse
    /// the asset where it will play and be seen — and where it will be
    /// seen is the league he came through, when that league is strong
    /// enough to hold him. So the passport and the home league's standing
    /// decide the destination, and the mood only sharpens it: a formed
    /// want can raise `Any` to a home preference, and it never lowers one.
    fn preference_for(
        home_desire: f32,
        home_league_reputation: u16,
        home_region: Option<ScoutingRegion>,
        club_region: ScoutingRegion,
        is_development: bool,
    ) -> LoanDestinationPreference {
        // What his situation says, before he says anything. The region
        // arm asks the question the level gate will ask anyway — is his
        // own continent within one allowance of the one he plays in? —
        // so the parent never forms a preference the market must refuse
        // (memory `loan_market_argmax_predictability`).
        let region_reachable = home_region.is_some_and(|region| {
            region.league_prestige() + HomeLoanGates::REGION_ALLOWANCE
                >= club_region.league_prestige()
        });
        let from_standing = if is_development && home_league_reputation >= Self::HOME_LEAGUE_BAR {
            LoanDestinationPreference::HomeCountry
        } else if region_reachable {
            LoanDestinationPreference::HomeRegion
        } else {
            LoanDestinationPreference::Any
        };
        // …and what he says, on the old mood ladder.
        let from_mood = if home_desire >= 0.55 {
            LoanDestinationPreference::HomeCountry
        } else if home_desire >= 0.25 {
            LoanDestinationPreference::HomeRegion
        } else {
            LoanDestinationPreference::Any
        };
        Self::stronger(from_standing, from_mood)
    }

    /// The more homeward of two preferences — the mood raises, never
    /// lowers.
    fn stronger(
        a: LoanDestinationPreference,
        b: LoanDestinationPreference,
    ) -> LoanDestinationPreference {
        let rank = |p: LoanDestinationPreference| match p {
            LoanDestinationPreference::Any => 0u8,
            LoanDestinationPreference::HomeRegion => 1,
            LoanDestinationPreference::HomeCountry => 2,
        };
        if rank(a) >= rank(b) { a } else { b }
    }
}

/// How much a destination answers where a player wants to be.
///
/// A ranking term on both sides of the loan market — the borrower's scan
/// and the parent's broadcast — and never a gate. The existing same-region
/// preference (local supply is real) stays exactly as it was, so the two
/// compete and the census decides.
pub struct HomeLoanPull;

impl HomeLoanPull {
    /// Lift for a destination in the player's own country.
    pub const COUNTRY: f32 = 0.60;
    /// …and for one merely on his own continent.
    pub const REGION: f32 = 0.25;
    /// How much a formed want sharpens the lift. At zero desire the pull
    /// is the base alone — the rumour every player entertains — so this
    /// never becomes a magnet that sends every foreigner home
    /// (Part VIII).
    pub const DESIRE: f32 = 0.50;

    /// Multiplier on a destination's appeal.
    pub fn factor(
        nationality_country_id: u32,
        nationality_region: Option<ScoutingRegion>,
        borrower_country_id: u32,
        borrower_region: ScoutingRegion,
        return_home_desire: f32,
    ) -> f32 {
        let home_country =
            nationality_country_id != 0 && nationality_country_id == borrower_country_id;
        // Unknown nationality fails closed — no home, no pull.
        let home_region = !home_country && nationality_region == Some(borrower_region);
        let geography = 1.0
            + if home_country {
                Self::COUNTRY
            } else if home_region {
                Self::REGION
            } else {
                0.0
            };
        geography * (1.0 + Self::DESIRE * return_home_desire.clamp(0.0, 1.0))
    }
}

/// The three level gates the loan market runs across a border, with the
/// one thing they were missing: a player is never a stranger in his own
/// country.
///
/// The gates themselves are unchanged — they read truth and they stay.
/// What changes is that a man's own league is not "a foreign step down"
/// to him, and his own continent is a smaller one than a stranger's.
pub struct HomeLoanGates;

impl HomeLoanGates {
    /// Extra region-prestige allowance for a loan into the player's own
    /// footballing region. A quarter of a point — enough to bring the
    /// mid-prestige regions (South America, Eastern Europe, the Middle
    /// East, North America, East Asia) inside reach of a Western European
    /// club's prospect, and nowhere near enough to open the bottom of the
    /// table to everybody.
    pub const REGION_ALLOWANCE: f32 = 0.25;

    /// Does the borrower's country clear the region-prestige step-down for
    /// this candidate? Home country always does — a Mexican going to Liga
    /// MX is not stepping into a strange ecosystem, he is going home, and
    /// under the old rule he could never do it at all.
    pub fn region_ok(
        base_ok: bool,
        player_region_prestige: f32,
        club_region_prestige: f32,
        is_home_country: bool,
        is_home_region: bool,
    ) -> bool {
        if is_home_country {
            return true;
        }
        if base_ok {
            return true;
        }
        is_home_region && player_region_prestige <= club_region_prestige + Self::REGION_ALLOWANCE
    }

    /// Country-reputation step-down. Home country always clears: a
    /// player's own federation is not "a smaller country" to him.
    pub fn country_rep_ok(base_ok: bool, is_home_country: bool) -> bool {
        base_ok || is_home_country
    }

    /// A club knows the players who came through its own league.
    ///
    /// Reach normally means "a scout of this club has that region on his
    /// map", and a Brazilian club's scouts do not have Western Europe on
    /// theirs. But it does not need one to remember a boy its own league
    /// produced — and this arm is scoped to LOANS and to candidates the
    /// parent has actually posted, so it can never become a
    /// permanent-transfer discovery channel that bypasses the springboard
    /// reach model (Part VIII, "the compatriot loophole").
    pub fn reach_ok(
        scout_reaches: bool,
        nationality_country_id: u32,
        nationality_region: Option<ScoutingRegion>,
        borrower_country_id: u32,
        borrower_region: ScoutingRegion,
        home_return_wanted: bool,
        is_development: bool,
    ) -> bool {
        if scout_reaches {
            return true;
        }
        // The A/B arm: with the compatriot reach disarmed a club sees
        // only what its own scouts cover, which is the HEAD every volume
        // guard in Part VI is written against.
        if MarketSwitches::home_reach_off() {
            return false;
        }
        if !home_return_wanted {
            return false;
        }
        if nationality_country_id != 0 && nationality_country_id == borrower_country_id {
            return true;
        }
        is_development && nationality_region == Some(borrower_region)
    }

    /// Is the parent posting him to the world as a man who would go home?
    ///
    /// Either half is enough (spec L7.5): a want that has actually formed
    /// ([`HomePull::wanted`]), or a loan-out candidacy that already carries
    /// a destination preference — the `UnsettledAbroad` case, where the
    /// club has decided even if the man has not said it out loud.
    ///
    /// ONE predicate, called by BOTH builders. The world pool required the
    /// formed want AND a loan badge while the ranked-summary builder
    /// hard-coded `false`, so the two disagreed about who was posted
    /// despite a comment claiming they could not.
    /// The loan badge is deliberately NOT a third arm: a man his club has
    /// listed for a loan has said nothing about where he wants to go.
    pub fn is_posted(wants_home: bool, has_destination_preference: bool) -> bool {
        wants_home || has_destination_preference
    }

    /// How formed the want has to be before the parent posts it to the
    /// world. Below this it is a mood, not an instruction.
    pub const WANTS_HOME_BAR: f32 = 0.4;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_pull_is_a_lift_and_not_a_magnet() {
        let brazil = 55;
        let sa = ScoutingRegion::SouthAmerica;
        // Home country, want fully formed.
        let full = HomeLoanPull::factor(brazil, Some(sa), brazil, sa, 1.0);
        assert!((full - 1.6 * 1.5).abs() < 1e-5, "{full}");
        // Home country, no want at all — still a lift, and a modest one.
        let quiet = HomeLoanPull::factor(brazil, Some(sa), brazil, sa, 0.0);
        assert!((quiet - 1.6).abs() < 1e-5, "{quiet}");
        // A neighbour is worth less than home.
        let neighbour = HomeLoanPull::factor(brazil, Some(sa), 54, sa, 1.0);
        assert!(neighbour < full && neighbour > 1.0);
        // Somewhere else entirely is worth nothing extra beyond the want.
        let elsewhere =
            HomeLoanPull::factor(brazil, Some(sa), 1, ScoutingRegion::WesternEurope, 0.0);
        assert!((elsewhere - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_player_is_never_a_stranger_in_his_own_country() {
        // Western Europe (1.0) → South America (0.45): the base gate says
        // no for a non-development case…
        assert!(!HomeLoanGates::region_ok(false, 1.0, 0.45, false, false));
        // …and yes the moment it is his own country.
        assert!(HomeLoanGates::region_ok(false, 1.0, 0.45, true, false));
        assert!(HomeLoanGates::country_rep_ok(false, true));
        assert!(!HomeLoanGates::country_rep_ok(false, false));
    }

    #[test]
    fn the_home_region_allowance_reaches_the_mid_regions_and_no_further() {
        // Western Europe → South America for a South American: 1.0 vs
        // 0.45 + 0.25 is still short, so the region arm alone does NOT
        // open it — the home-COUNTRY arm is what carries a return home,
        // and a neighbour needs the ordinary development allowance.
        assert!(!HomeLoanGates::region_ok(false, 1.0, 0.45, false, true));
        // Eastern Europe (0.50) → Middle East (0.40) for a Middle
        // Easterner: inside the allowance.
        assert!(HomeLoanGates::region_ok(false, 0.50, 0.40, false, true));
        // …and the bottom of the table stays shut.
        assert!(!HomeLoanGates::region_ok(false, 0.50, 0.10, false, true));
    }

    #[test]
    fn a_home_club_sees_its_own_exports_and_nobody_elses() {
        let sa = ScoutingRegion::SouthAmerica;
        let we = ScoutingRegion::WesternEurope;
        // A Brazilian club with no Western-European scout, looking at a
        // Brazilian posted as wanting home: it knows him.
        assert!(HomeLoanGates::reach_ok(
            false,
            55,
            Some(sa),
            55,
            sa,
            true,
            true
        ));
        // The same club, the same lack of a scout, a Frenchman at the
        // same English club: it does not.
        assert!(!HomeLoanGates::reach_ok(
            false,
            1,
            Some(we),
            55,
            sa,
            true,
            true
        ));
        // And a Brazilian who has NOT been posted stays invisible — the
        // arm is scoped to the parent's own broadcast.
        assert!(!HomeLoanGates::reach_ok(
            false,
            55,
            Some(sa),
            55,
            sa,
            false,
            true
        ));
        // A continental neighbour reaches him only on the development
        // profile.
        assert!(HomeLoanGates::reach_ok(
            false,
            55,
            Some(sa),
            54,
            sa,
            true,
            true
        ));
        assert!(!HomeLoanGates::reach_ok(
            false,
            55,
            Some(sa),
            54,
            sa,
            true,
            false
        ));
    }

    /// B2 — the parent decides where he goes, and the mood only sharpens
    /// it. A prospect who is merely not playing scores ≈ 0 on the
    /// homesickness axis; reading that axis alone made him `Any`, so he
    /// was never posted and was loaned "elsewhere".
    #[test]
    fn a_strong_home_league_is_a_destination_even_with_no_mood_at_all() {
        let sa = ScoutingRegion::SouthAmerica;
        let we = ScoutingRegion::WesternEurope;
        // A Brazilian development candidate at a Western European club,
        // saying nothing: home is where he will be seen.
        assert_eq!(
            UnsettledAbroadScan::preference_for(0.0, 6_400, Some(sa), we, true),
            LoanDestinationPreference::HomeCountry
        );
        // A Ghanaian in the same seat: his own top flight is nowhere near
        // the bar, and his region is a long way below the one he plays
        // in, so the answer is "wherever you will play".
        assert_eq!(
            UnsettledAbroadScan::preference_for(
                0.0,
                2_800,
                Some(ScoutingRegion::WestAfrica),
                we,
                true
            ),
            LoanDestinationPreference::Any
        );
        // Past the development band the parent is selling, not
        // warehousing — the country preference is his own to voice.
        assert_eq!(
            UnsettledAbroadScan::preference_for(0.0, 6_400, Some(sa), sa, false),
            LoanDestinationPreference::HomeRegion
        );
    }

    #[test]
    fn the_mood_raises_the_preference_and_never_lowers_it() {
        let we = ScoutingRegion::WesternEurope;
        let wa = ScoutingRegion::WestAfrica;
        // A weak home league says `Any`; a formed want overrides it —
        // the man himself has spoken.
        assert_eq!(
            UnsettledAbroadScan::preference_for(0.8, 2_800, Some(wa), we, true),
            LoanDestinationPreference::HomeCountry
        );
        assert_eq!(
            UnsettledAbroadScan::preference_for(0.3, 2_800, Some(wa), we, true),
            LoanDestinationPreference::HomeRegion
        );
        // …and a silent man with a strong home league keeps the
        // standing's answer rather than the mood's.
        assert_eq!(
            UnsettledAbroadScan::preference_for(
                0.0,
                6_400,
                Some(ScoutingRegion::SouthAmerica),
                we,
                true
            ),
            LoanDestinationPreference::HomeCountry
        );
    }

    /// C5 — ONE posting predicate, and either half is enough. The world
    /// pool used to require the formed want AND a loan badge while the
    /// ranked-summary builder hard-coded `false`.
    #[test]
    fn a_posting_is_the_want_or_the_clubs_own_decision() {
        // The want has formed: he is posted, badge or no badge.
        assert!(HomeLoanGates::is_posted(true, false));
        // The club has decided where he should go: also a posting, even
        // though he has not said it out loud.
        assert!(HomeLoanGates::is_posted(false, true));
        // Neither: nothing to broadcast.
        assert!(!HomeLoanGates::is_posted(false, false));
    }

    /// C10 / C5 — the want clears itself. A year of regular football
    /// answers the question the posting asked, so nothing has to remember
    /// to take the flag down.
    #[test]
    fn a_settled_year_takes_the_posting_down() {
        let mut situation = MindSituation::neutral();
        situation.is_abroad = true;
        situation.days_at_club = HomePull::SETTLED_TENURE_DAYS;
        situation.starter_ratio = HomePull::SETTLED_STARTER_SHARE;
        let settled = HomePull {
            desire: 0.9,
            wanted: situation.is_abroad
                && !(situation.days_at_club >= HomePull::SETTLED_TENURE_DAYS
                    && situation.starter_ratio >= HomePull::SETTLED_STARTER_SHARE)
                && 0.9 >= HomeLoanGates::WANTS_HOME_BAR,
            computed_on: None,
        };
        assert!(!settled.wanted, "a year of starts is an answer");
    }
}

/// B1 — "abroad" is a PASSPORT test, and it never was.
///
/// `MindSituation::is_abroad` read "does not speak the local language",
/// so a Brazilian at Benfica, an Argentine at Sevilla, a Colombian in
/// Mexico and a Uruguayan in Argentina were all at home: never homesick,
/// never posted, never seen by a club in the league they came through.
/// Those are the largest home-loan populations in world football, and the
/// census read 1 % home-country loans because of this one line.
#[cfg(test)]
mod passport_tests {
    use super::*;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::club::player::personality::language::{Language, PlayerLanguage};
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };

    const BRAZIL: u32 = 55;
    const PORTUGAL: u32 = 11;

    fn brazilian() -> crate::Player {
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("A".into(), "B".into()))
            .birth_date(NaiveDate::from_ymd_opt(2004, 1, 1).unwrap())
            .country_id(BRAZIL)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .player_attributes(PlayerAttributes::default())
            .languages(vec![PlayerLanguage::native(Language::Portuguese)])
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .build()
            .unwrap()
    }

    #[test]
    fn a_brazilian_in_portugal_is_abroad_and_speaks_the_language() {
        let player = brazilian();
        let date = NaiveDate::from_ymd_opt(2026, 9, 3).unwrap();
        let situation = player.mind_situation(date, PORTUGAL, "pt");
        assert!(
            situation.is_abroad,
            "he is outside his own federation, whatever he speaks"
        );
        assert!(
            situation.speaks_local_language,
            "…and the language is a SEPARATE fact, not the same one"
        );
        assert!(
            !situation.is_culturally_isolated(),
            "a man who speaks the language is not isolated by it"
        );
    }

    #[test]
    fn the_same_man_at_home_is_not_abroad() {
        let player = brazilian();
        let date = NaiveDate::from_ymd_opt(2026, 9, 3).unwrap();
        let situation = player.mind_situation(date, BRAZIL, "br");
        assert!(!situation.is_abroad);
        assert!(situation.speaks_local_language);
    }

    /// …and the consequence: the parent can now post him. Under the
    /// language test `HomePull::wanted` was unreachable for exactly this
    /// population, so `HomeLoanGates::is_posted` had nobody to say yes
    /// about and the compatriot sweep swept an empty pool.
    #[test]
    fn a_homesick_brazilian_in_portugal_can_be_posted() {
        let mut player = brazilian();
        player.happiness.starter_ratio = 0.05;
        let date = NaiveDate::from_ymd_opt(2026, 9, 3).unwrap();
        let mut situation = player.mind_situation(date, PORTUGAL, "pt");
        situation.days_at_club = 200;
        // A formed want, whatever channel raised it.
        let pull = HomePull {
            desire: 0.6,
            wanted: situation.is_abroad
                && !(situation.days_at_club >= HomePull::SETTLED_TENURE_DAYS
                    && situation.starter_ratio >= HomePull::SETTLED_STARTER_SHARE)
                && 0.6 >= HomeLoanGates::WANTS_HOME_BAR,
            computed_on: Some(date),
        };
        assert!(pull.wanted, "the passport is what opens the posting");
        assert!(HomeLoanGates::is_posted(pull.wanted, false));
    }
}
