//! The big-stage pull — one continuous model of how strongly a player is
//! drawn away from his current competition toward a bigger one.
//!
//! Football's most ordinary career story is a good player in a decent
//! league wanting to test himself in a better one. Before this model the
//! simulation had three separate, mostly-unreachable expressions of it:
//! a league-gap request whose league reputation was never wired through,
//! a European-competition mood gated on world fame a sub-elite player can
//! never earn, and a Libertadores twin of the same. All three needed the
//! player to be UNHAPPY first, which inverted the truth — the players who
//! most want a bigger stage are usually the ones doing best on their
//! current one.
//!
//! So: one score, computed for every senior player every week, from
//! observable career facts. It answers "how far above his stage is he, and
//! how much does he want more?" — and the answer feeds three tiers of
//! consequence rather than a single on/off request:
//!
//!   * **Inclination** — silent. He would listen if a bigger league
//!     called. Nothing is emitted; the market reads it when a bid arrives.
//!     This is most good players in most sub-elite leagues, which is
//!     exactly how the real market behaves.
//!   * **Mood** — visible. A recurring, cooldowned ambition event and a
//!     chronic morale drag: the player is publicly restless.
//!   * **Request** — a formal `Req` on the player's own initiative.
//!
//! Escalation from mood to request deliberately does NOT require a spell
//! of unhappiness. It requires *persistence* (the itch has lasted a
//! season) or *denial* (a concrete move was blocked). That is how these
//! requests actually happen: they follow a rejected bid or a window that
//! came and went, not a depression.
//!
//! Every axis is a continuous curve, so there is no threshold at which a
//! league suddenly starts or stops exporting players — a stronger league
//! simply sheds fewer of them.

use crate::club::player::mind::{MindClock, MindSituation};
use crate::club::player::player::Player;
use crate::club::player::statistics::StuckCareerScan;
use crate::club::staff::perception::AbilityEstimator;
use crate::transfers::squad::bands::TierBands;
use crate::utils::DateUtils;
use crate::{Person, PlayerFieldPositionGroup, TeamType};
use chrono::NaiveDate;

/// Tunables for [`BigStagePull`]. Every value is a shape parameter of a
/// continuous curve rather than a cliff, so calibration moves the whole
/// distribution instead of reclassifying a band of players.
#[derive(Debug, Clone, Copy)]
pub struct BigStagePullConfig {
    /// League reputation at or above which there is no bigger stage worth
    /// chasing. Set just under the very top so the strongest leagues carry
    /// a small residual pull toward each other (a Bundesliga star can
    /// still dream of Madrid) while the top two carry none.
    pub elite_league_rep: u16,
    /// Reputation points below `elite_league_rep` at which the stage gap
    /// saturates. Beyond this the league is simply "not the big time" and
    /// getting weaker adds nothing.
    pub stage_gap_span: f32,
    /// Exponent on the stage gap. Below 1 so mid-tier leagues — the
    /// classic exporters — carry a meaningful pull rather than only the
    /// weakest ones.
    pub stage_gap_curve: f32,
    /// Ability above the league's starter baseline at which a player reads
    /// as a complete standout for that stage. Measured: the gap between a
    /// league's typical starter and its top 2% runs 10–28 across the
    /// database, clustering near 20.
    pub standout_span: f32,
    /// Ability at which a player is entirely implausible on the biggest
    /// stage, and the span from there to unquestionably good enough.
    ///
    /// Towering over your own league is only half the story: a modest
    /// player topping a very weak division is a standout *there* and
    /// nowhere near a top-five side. Without this second axis the weakest
    /// leagues dominated the pull entirely — their starter baseline is low,
    /// so standing came cheap, while their stage gap was always maximal.
    pub elite_plausibility_floor: f32,
    pub elite_plausibility_span: f32,
    /// Ambition below which the pull is inert, and the span to full drive.
    pub ambition_floor: f32,
    pub ambition_span: f32,
    /// How much maximum loyalty damps the drive.
    pub loyalty_damp: f32,
    /// Amplifier granted at full international exposure — a squad regular
    /// for his country measures himself against players in better leagues
    /// every camp.
    pub caps_amplifier: f32,
    /// Caps at which that amplifier saturates.
    pub caps_saturation: f32,
    /// Amplifier when the club's country is barred from continental
    /// competition — the league is a genuine dead end.
    pub isolation_amplifier: f32,
    /// Multiplier while the player is registered below the first team. He
    /// has a nearer problem than the size of the stage, and
    /// `WantsFirstTeamFootball` owns it.
    pub below_first_team_damp: f32,
    /// Score at/above which the player's openness to a bigger league is
    /// material enough to name.
    ///
    /// **Diagnostic, not a gate.** The market reads the raw score
    /// continuously — a bigger-league bid is weighed in proportion to how
    /// much the player wants one, with no cliff anywhere — so nothing in
    /// the simulation branches on this. It exists so the telemetry and the
    /// tests can talk about "would listen" as a population, and it is set
    /// where the personal-terms bonus reaches roughly five points, the
    /// level at which it starts swinging marginal negotiations.
    pub inclination_bar: f32,
    /// Score at/above which the restlessness becomes visible — the tier a
    /// person browsing the game actually sees, so it is calibrated on
    /// population rather than feel. At 0.45 only three players in the
    /// whole Russian top flight were ever publicly restless, which is
    /// indistinguishable from the behaviour not existing; here it is
    /// roughly half a dozen per strong sub-elite league at any moment —
    /// visible while browsing, still a clear minority of good players.
    pub mood_bar: f32,
    /// Score at/above which he is willing to formally ask out, once
    /// persistence or denial has also been satisfied.
    pub request_bar: f32,
    /// Loyalty at/above which a player at a boyhood club stays regardless.
    pub loyalty_stay_floor: f32,
    /// Days at the club before the pull engages — a new signing gets a
    /// season to find out what he has joined. The same season every
    /// other "is he stuck here" reading uses.
    pub settle_days: i64,
}

impl Default for BigStagePullConfig {
    fn default() -> Self {
        BigStagePullConfig {
            elite_league_rep: 9000,
            stage_gap_span: 3000.0,
            stage_gap_curve: 0.7,
            standout_span: 18.0,
            elite_plausibility_floor: 95.0,
            elite_plausibility_span: 45.0,
            ambition_floor: 6.0,
            ambition_span: 11.0,
            loyalty_damp: 0.40,
            caps_amplifier: 0.12,
            caps_saturation: 20.0,
            isolation_amplifier: 0.15,
            below_first_team_damp: 0.5,
            inclination_bar: 0.22,
            mood_bar: 0.40,
            request_bar: 0.68,
            loyalty_stay_floor: 17.0,
            settle_days: StuckCareerScan::TENURE_FOR_A_STUCK_STORY,
        }
    }
}

/// What the world looks like to the player when the pull is scored.
#[derive(Debug, Clone, Copy)]
pub struct BigStagePullContext {
    /// Reputation (0..10000) of the competition his club plays in. Zero
    /// means unknown, and the pull fails closed.
    pub league_reputation: u16,
    /// True when his club's country cannot enter continental competition.
    pub continentally_isolated: bool,
    /// Which squad holds his registration.
    pub squad_tier: TeamType,
    /// True when his current club is one of his boyhood favourites.
    pub at_favourite_club: bool,
}

/// A scored big-stage pull. `score` is the single number; the tier
/// predicates read it against the configured bars so callers never
/// hard-code a threshold.
#[derive(Debug, Clone, Copy)]
pub struct BigStagePull {
    pub score: f32,
    /// How far his league sits below the biggest stage, 0..1.
    pub stage_gap: f32,
    /// How far he sits above his league's own starter level, 0..1.
    pub standing: f32,
    config: BigStagePullConfig,
}

impl BigStagePull {
    /// Score the pull for one player. Returns a zero-score pull whenever
    /// any hard precondition fails (unknown league, still settling, a
    /// loyal one-club man at home), so callers can treat the result
    /// uniformly instead of unwrapping an option.
    pub fn assess(player: &Player, now: NaiveDate, ctx: &BigStagePullContext) -> Self {
        Self::assess_with(player, now, ctx, BigStagePullConfig::default())
    }

    pub fn assess_with(
        player: &Player,
        now: NaiveDate,
        ctx: &BigStagePullContext,
        config: BigStagePullConfig,
    ) -> Self {
        let inert = BigStagePull {
            score: 0.0,
            stage_gap: 0.0,
            standing: 0.0,
            config,
        };

        // Unknown league: fail closed rather than read it as infinitely
        // weak. A missing context must never manufacture ambition.
        if ctx.league_reputation == 0 {
            return inert;
        }
        // A boyhood servant at his own club is not going anywhere, however
        // modest the league. This is the one categorical exemption — the
        // rest of the model is continuous.
        if ctx.at_favourite_club && player.attributes.loyalty >= config.loyalty_stay_floor {
            return inert;
        }
        // A recent arrival has not yet had the season that would tell him
        // whether this league is beneath him.
        let settled = player
            .days_since_transfer(now)
            .map(|days| days >= config.settle_days)
            .unwrap_or(true);
        if !settled {
            return inert;
        }

        let stage_gap = Self::stage_gap(ctx.league_reputation, &config);
        if stage_gap <= 0.0 {
            return inert;
        }

        let standing = Self::standing(
            player.player_attributes.current_ability as f32,
            ctx.league_reputation,
            &config,
        );
        if standing <= 0.0 {
            return inert;
        }

        let age = DateUtils::age(player.birth_date, now);
        let age_curve = Self::age_curve(age, player.position().is_goalkeeper());
        if age_curve <= 0.0 {
            return inert;
        }

        let drive = ((player.attributes.ambition - config.ambition_floor) / config.ambition_span)
            .clamp(0.0, 1.0);
        let loyalty_damp =
            1.0 - config.loyalty_damp * (player.attributes.loyalty / 20.0).clamp(0.0, 1.0);
        let personality = drive * loyalty_damp;

        let caps = player.player_attributes.international_apps as f32;
        let caps_amp =
            1.0 + config.caps_amplifier * (caps / config.caps_saturation).clamp(0.0, 1.0);
        let isolation_amp = if ctx.continentally_isolated {
            1.0 + config.isolation_amplifier
        } else {
            1.0
        };
        let squad_damp = if matches!(ctx.squad_tier, TeamType::Main) {
            1.0
        } else {
            config.below_first_team_damp
        };

        let score = (stage_gap
            * standing
            * personality
            * age_curve
            * caps_amp
            * isolation_amp
            * squad_damp)
            .clamp(0.0, 1.0);

        BigStagePull {
            score,
            stage_gap,
            standing,
            config,
        }
    }

    /// He would listen if a bigger league came calling. Silent — no mood,
    /// no request. The market reads this when a bid actually arrives.
    pub fn is_inclined(&self) -> bool {
        self.score >= self.config.inclination_bar
    }

    /// The restlessness is visible: a recurring ambition mood and a
    /// chronic drag on morale while he stays.
    pub fn shows_mood(&self) -> bool {
        self.score >= self.config.mood_bar
    }

    /// Strong enough that he is willing to formally ask out — subject to
    /// the caller also establishing persistence or denial.
    pub fn would_request(&self) -> bool {
        self.score >= self.config.request_bar
    }

    /// How far the league sits below the biggest stage, 0..1, on the
    /// configured curve. Zero for the elite leagues themselves.
    fn stage_gap(league_reputation: u16, config: &BigStagePullConfig) -> f32 {
        let deficit = config.elite_league_rep.saturating_sub(league_reputation) as f32;
        let linear = (deficit / config.stage_gap_span).clamp(0.0, 1.0);
        linear.powf(config.stage_gap_curve)
    }

    /// Expected ability of a typical starter at this league's level — the
    /// yardstick a player measures himself against.
    ///
    /// Anchored on the shipped database rather than assumed: the median
    /// starting-calibre ability of every league in each reputation band.
    /// The relationship is emphatically NOT linear, and assuming it was
    /// cost the model its whole target population. It climbs steeply
    /// through the weak and modest divisions, then **flattens across the
    /// 5500–8000 range** — Russia, Turkey, Portugal, the Netherlands and
    /// Argentina field players of genuinely comparable quality, which is
    /// exactly why they trade with each other as peers — before rising
    /// again into the top five.
    ///
    /// A straight line through those endpoints demanded ~128 of a player
    /// in the 7000–8499 band whose league's real starter sits at 118, so
    /// the classic exporter leagues produced almost no ambition at all.
    pub(crate) fn league_starter_ability(
        league_reputation: u16,
        _config: &BigStagePullConfig,
    ) -> f32 {
        /// `(reputation ÷ 10000, median starter ability)`.
        const ANCHORS: [(f32, f32); 6] = [
            (0.00, 70.0),
            (0.32, 90.0),
            (0.48, 108.0),
            (0.58, 116.0),
            (0.75, 118.0),
            (0.92, 140.0),
        ];
        let rep = (league_reputation as f32 / 10_000.0).clamp(0.0, 1.0);
        if rep >= ANCHORS[ANCHORS.len() - 1].0 {
            let (r_top, ca_top) = ANCHORS[ANCHORS.len() - 1];
            // Above the top anchor keep climbing — the very best league is
            // a harder stage still than a merely elite one.
            return ca_top + (rep - r_top) * (150.0 - ca_top) / (1.0 - r_top).max(1e-6);
        }
        for window in ANCHORS.windows(2) {
            let (r0, ca0) = window[0];
            let (r1, ca1) = window[1];
            if rep >= r0 && rep <= r1 {
                let t = (rep - r0) / (r1 - r0).max(1e-6);
                return ca0 + (ca1 - ca0) * t;
            }
        }
        ANCHORS[0].1
    }

    /// How far the player stands out — on his own stage AND on the one he
    /// is chasing. Both must hold: a modest player topping a very weak
    /// division towers over his league and would still be nowhere near a
    /// top-five side, and the pull has to know the difference.
    fn standing(ability: f32, league_reputation: u16, config: &BigStagePullConfig) -> f32 {
        let baseline = Self::league_starter_ability(league_reputation, config);
        let local = ((ability - baseline) / config.standout_span).clamp(0.0, 1.0);
        let elite_plausible = ((ability - config.elite_plausibility_floor)
            / config.elite_plausibility_span)
            .clamp(0.0, 1.0);
        local * elite_plausible
    }

    /// Appetite for uprooting a career, by age. Ramps in through the early
    /// twenties, holds through the prime years when a big move is both
    /// wanted and buyable, then fades — a thirty-two-year-old who has not
    /// had his move is no longer waiting for it. Keepers peak later, as
    /// they do everywhere else in the model.
    fn age_curve(age: u8, is_goalkeeper: bool) -> f32 {
        let shift = if is_goalkeeper { 2 } else { 0 };
        let age = age as i16 - shift as i16;
        match age {
            a if a < 19 => 0.0,
            a if a < 24 => (a - 19) as f32 / 5.0,
            a if a <= 28 => 1.0,
            a if a < 32 => 1.0 - (a - 28) as f32 / 4.0,
            _ => 0.0,
        }
    }
}

/// What a player can be read as having earned — the inputs to the floor
/// under the football he will play.
#[derive(Debug, Clone, Copy)]
pub struct StandingReading {
    /// His observable level, 1..200.
    pub level: u8,
    pub group: PlayerFieldPositionGroup,
    /// Blended market reputation, 0..10000.
    pub effective_rep: i16,
    /// The division he plays in now, and how much of a regular he is
    /// there — 0 when nobody has seen enough of him to say.
    pub league_rep: u16,
    pub starter_share: f32,
    pub caps: u16,
    pub age: u8,
    /// How far months on the market have lowered his sights, 0..1.
    pub resignation: f32,
    /// How firmly his own plan is a step down, 0..1.
    pub plan_widening: f32,
}

/// The lowest level of football a player will play at, on the league
/// reputation scale: the standing he has a claim on, less what he will
/// still go under it for.
///
/// Measured against the DIVISION he would play in, never the club. A
/// relegated name keeps a top-flight reputation for seasons, and a
/// consent read off it saw a third-tier side as a top-flight one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelFloor {
    pub standing: f32,
    pub tolerance: f32,
}

impl LevelFloor {
    /// A division's worth of reputation — the step down anybody takes
    /// before his name is in question.
    pub const STEP_DOWN: f32 = 2_000.0;
    /// Age at or below which the band is at its widest: a season in
    /// men's football is worth more to a boy than his name is. It widens
    /// with youth; it never vanishes.
    const YOUTH_AGE: u8 = 23;
    const YOUTH_SPAN: f32 = 7.0;
    const YOUTH_WIDENING: f32 = 0.8;
    /// What months unsold take off the floor at full resignation —
    /// every unsold month re-reads what level actually wants him.
    pub const RESIGNATION_SPAN: f32 = 3_000.0;
    /// … and what a plan to go down a level and play takes off it.
    const PLAN_SPAN: f32 = 1_500.0;
    /// Reputation under the floor at which a division stops being
    /// football he would play at all.
    pub const REFUSAL_SPAN: f32 = 1_000.0;
    /// Caps at which an international's name carries its full reach …
    const CAPS_ESTABLISHED: f32 = 30.0;
    /// … and how much further it carries than his club form alone.
    const CAPS_RENOWN: f32 = 0.25;
    /// Start share at which he is a regular where he plays.
    const REGULAR_SHARE: f32 = 0.45;

    /// No view: nothing to object to.
    pub fn none() -> Self {
        LevelFloor {
            standing: 0.0,
            tolerance: 0.0,
        }
    }

    /// The strongest of three claims: the level his ability starts at,
    /// the name he has made, and the division he is a regular in — and
    /// never above the division he already plays in, which is a level he
    /// has accepted whatever the other two say.
    pub fn of(reading: &StandingReading) -> Self {
        let ability = TierBands::level_score(reading.level, reading.group) * 10_000.0;
        let capped = (reading.caps as f32 / Self::CAPS_ESTABLISHED).clamp(0.0, 1.0);
        let renown = reading.effective_rep.max(0) as f32 * (1.0 + Self::CAPS_RENOWN * capped);
        let regular = (reading.starter_share / Self::REGULAR_SHARE).clamp(0.0, 1.0);
        let established = reading.league_rep as f32 * regular;
        let claim = ability.max(renown).max(established);
        LevelFloor {
            standing: if reading.league_rep > 0 {
                claim.min(reading.league_rep as f32)
            } else {
                claim
            },
            tolerance: Self::tolerance(reading.age, reading.resignation, reading.plan_widening),
        }
    }

    /// How far under his standing he will still go.
    pub fn tolerance(age: u8, resignation: f32, plan_widening: f32) -> f32 {
        let youth = (Self::YOUTH_AGE.saturating_sub(age) as f32 / Self::YOUTH_SPAN).clamp(0.0, 1.0);
        Self::STEP_DOWN * (1.0 + Self::YOUTH_WIDENING * youth)
            + resignation.clamp(0.0, 1.0) * Self::RESIGNATION_SPAN
            + plan_widening.clamp(0.0, 1.0) * Self::PLAN_SPAN
    }

    pub fn floor(&self) -> f32 {
        self.standing - self.tolerance
    }

    /// How far a division at `league_rep` sits under his floor, 0..1 —
    /// nothing at the floor, everything a full refusal span under it.
    /// No view of either side reads as nothing to object to.
    pub fn below(&self, league_rep: f32) -> f32 {
        if self.standing <= 0.0 || league_rep <= 0.0 {
            return 0.0;
        }
        ((self.floor() - league_rep) / Self::REFUSAL_SPAN).clamp(0.0, 1.0)
    }
}

impl Player {
    /// The floor under the football he will play, read as he stands
    /// today in the division at `league_rep`.
    pub fn level_floor(&self, date: NaiveDate, league_rep: u16) -> LevelFloor {
        let seen = self.happiness.appearances_tracked >= MindSituation::TRACKED_APPS;
        LevelFloor::of(&StandingReading {
            level: AbilityEstimator::observable_level(self),
            group: self.position().position_group(),
            effective_rep: self.player_attributes.effective_reputation(true),
            league_rep,
            starter_share: if seen {
                self.happiness.starter_ratio
            } else {
                0.0
            },
            caps: self.player_attributes.international_apps,
            age: self.age(date),
            resignation: self.market_resignation(date),
            plan_widening: self
                .mind
                .career
                .plan_view(MindClock::day(date))
                .renown_widening(),
        })
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

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn player(ca: u8, ambition: f32, loyalty: f32, age: u8, caps: u16) -> Player {
        let today = d(2026, 8, 1);
        let birth = today
            .checked_sub_signed(chrono::Duration::days(age as i64 * 365))
            .unwrap();
        let attrs = PlayerAttributes {
            current_ability: ca,
            potential_ability: ca,
            international_apps: caps,
            ..Default::default()
        };
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".into(), "Player".into()))
            .birth_date(birth)
            .country_id(1)
            .attributes(PersonAttributes {
                adaptability: 10.0,
                ambition,
                controversy: 10.0,
                loyalty,
                pressure: 10.0,
                professionalism: 10.0,
                sportsmanship: 10.0,
                temperament: 10.0,
                consistency: 10.0,
                important_matches: 10.0,
                dirtiness: 10.0,
            })
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    fn ctx(league_reputation: u16) -> BigStagePullContext {
        BigStagePullContext {
            league_reputation,
            continentally_isolated: false,
            squad_tier: TeamType::Main,
            at_favourite_club: false,
        }
    }

    #[test]
    fn a_star_in_a_strong_but_sub_elite_league_feels_the_pull() {
        // Russian Premier League reputation, a genuine standout.
        let p = player(144, 17.0, 10.0, 25, 15);
        let mut c = ctx(6500);
        c.continentally_isolated = true;
        let pull = BigStagePull::assess(&p, d(2026, 8, 1), &c);
        assert!(pull.is_inclined(), "score was {}", pull.score);
        assert!(pull.shows_mood(), "score was {}", pull.score);
    }

    #[test]
    fn an_ordinary_starter_in_the_same_league_does_not() {
        let p = player(115, 16.0, 8.0, 25, 0);
        let pull = BigStagePull::assess(&p, d(2026, 8, 1), &ctx(6500));
        assert!(!pull.is_inclined(), "score was {}", pull.score);
    }

    #[test]
    fn nobody_in_the_strongest_league_is_pulled_higher() {
        let p = player(180, 20.0, 4.0, 26, 60);
        let pull = BigStagePull::assess(&p, d(2026, 8, 1), &ctx(9500));
        assert_eq!(pull.score, 0.0);
    }

    #[test]
    fn a_feeder_league_standout_is_inclined_without_agitating() {
        // Eredivisie-class league, a very good but not generational player.
        let p = player(150, 18.0, 8.0, 24, 20);
        let pull = BigStagePull::assess(&p, d(2026, 8, 1), &ctx(7600));
        assert!(pull.is_inclined(), "score was {}", pull.score);
        assert!(!pull.would_request(), "score was {}", pull.score);
    }

    #[test]
    fn a_loyal_boyhood_servant_stays() {
        let p = player(150, 18.0, 18.0, 25, 20);
        let mut c = ctx(6000);
        c.at_favourite_club = true;
        assert_eq!(BigStagePull::assess(&p, d(2026, 8, 1), &c).score, 0.0);
    }

    #[test]
    fn an_unknown_league_never_manufactures_ambition() {
        let p = player(170, 20.0, 2.0, 25, 40);
        assert_eq!(BigStagePull::assess(&p, d(2026, 8, 1), &ctx(0)).score, 0.0);
    }

    #[test]
    fn the_pull_fades_with_age() {
        let prime = player(150, 18.0, 8.0, 26, 20);
        let veteran = player(150, 18.0, 8.0, 33, 20);
        let c = ctx(6500);
        let today = d(2026, 8, 1);
        assert!(BigStagePull::assess(&prime, today, &c).score > 0.0);
        assert_eq!(BigStagePull::assess(&veteran, today, &c).score, 0.0);
    }

    #[test]
    fn being_parked_below_the_first_team_damps_the_stage_pull() {
        let p = player(150, 18.0, 8.0, 25, 20);
        let today = d(2026, 8, 1);
        let first_team = BigStagePull::assess(&p, today, &ctx(6500)).score;
        let mut parked = ctx(6500);
        parked.squad_tier = TeamType::B;
        let reserve = BigStagePull::assess(&p, today, &parked).score;
        assert!(reserve < first_team);
        assert!(reserve > 0.0);
    }

    /// The failure the first calibration run exposed: a merely-decent
    /// player topping a very weak division outscored a genuinely good one
    /// in a strong league, because his starter baseline was low and his
    /// stage gap was maximal. Standing over your own league is not the
    /// same as being good enough for the one you are chasing.
    #[test]
    fn a_modest_standout_in_a_weak_league_is_not_pulled_past_a_real_talent() {
        let today = d(2026, 8, 1);
        let big_fish_small_pond = player(105, 16.0, 10.0, 25, 0);
        let genuine_talent = player(132, 16.0, 10.0, 25, 0);
        let weak = BigStagePull::assess(&big_fish_small_pond, today, &ctx(3_200)).score;
        let strong = BigStagePull::assess(&genuine_talent, today, &ctx(6_500)).score;
        assert!(
            strong > weak,
            "a real talent in a strong league must out-pull a small-pond standout: {strong} vs {weak}"
        );
    }

    /// The classic exporter leagues — Portugal, the Netherlands, Turkey,
    /// Argentina — must produce ambition. A straight-line starter baseline
    /// demanded ~128 of them when their real starter sits at 118, and they
    /// produced almost none.
    #[test]
    fn the_exporter_leagues_produce_ambition() {
        let today = d(2026, 8, 1);
        let star = player(136, 18.0, 8.0, 25, 12);
        for (rep, name) in [
            (7_800u16, "portugal"),
            (7_600, "netherlands"),
            (7_000, "turkey"),
        ] {
            let pull = BigStagePull::assess(&star, today, &ctx(rep));
            assert!(
                pull.is_inclined(),
                "{name} (rep {rep}) should produce ambition, score was {}",
                pull.score
            );
        }
    }

    /// The measured starter baselines the anchor curve is fitted to.
    #[test]
    fn the_starter_baseline_tracks_the_measured_leagues() {
        let cfg = BigStagePullConfig::default();
        for (rep, measured) in [
            (3_200u16, 90.0),
            (4_800, 108.0),
            (5_800, 116.0),
            (7_500, 118.0),
        ] {
            let derived = BigStagePull::league_starter_ability(rep, &cfg);
            assert!(
                (derived - measured).abs() <= 3.0,
                "rep {rep}: derived {derived} strays from the measured {measured}"
            );
        }
    }

    #[test]
    fn a_weaker_league_pulls_harder_than_a_stronger_one() {
        let p = player(150, 18.0, 8.0, 25, 10);
        let today = d(2026, 8, 1);
        let strong = BigStagePull::assess(&p, today, &ctx(8200)).score;
        let mid = BigStagePull::assess(&p, today, &ctx(7000)).score;
        let weak = BigStagePull::assess(&p, today, &ctx(5000)).score;
        assert!(strong < mid, "{strong} !< {mid}");
        assert!(mid < weak, "{mid} !< {weak}");
    }

    // ── The floor ───────────────────────────────────────────────

    /// Italy, from `league.json`: Serie A, Serie B, Serie C.
    const SERIE_A: f32 = 8_750.0;
    const SERIE_B: f32 = 5_500.0;
    const SERIE_C: f32 = 3_500.0;

    /// The case the floor was written for: a fifty-cap international,
    /// level 128, just signed by a giant and not yet picked — so no
    /// division he is a regular in, and a name his club form alone
    /// would read as a fifth-tier reputation.
    fn international(age: u8, resignation: f32) -> LevelFloor {
        LevelFloor::of(&StandingReading {
            level: 128,
            group: PlayerFieldPositionGroup::Midfielder,
            effective_rep: 5_400,
            league_rep: SERIE_A as u16,
            starter_share: 0.0,
            caps: 51,
            age,
            resignation,
            plan_widening: 0.0,
        })
    }

    #[test]
    fn an_international_does_not_play_in_the_third_tier() {
        let floor = international(27, 0.0);
        assert_eq!(floor.below(SERIE_A), 0.0);
        assert_eq!(floor.below(SERIE_B), 0.0, "{}", floor.floor());
        assert_eq!(floor.below(SERIE_C), 1.0, "{}", floor.floor());
    }

    /// His ability puts him a division under his name's reach; the
    /// strongest claim is the one that stands.
    #[test]
    fn the_standing_is_the_strongest_of_his_claims() {
        let by_ability = international(27, 0.0).standing;
        let by_name = LevelFloor::of(&StandingReading {
            level: 95,
            effective_rep: 7_000,
            caps: 0,
            ..international_reading(27)
        })
        .standing;
        let by_division = LevelFloor::of(&StandingReading {
            level: 95,
            effective_rep: 3_000,
            starter_share: 0.8,
            ..international_reading(27)
        })
        .standing;
        assert!(by_ability > 6_500.0, "{by_ability}");
        assert_eq!(by_name, 7_000.0);
        assert_eq!(by_division, SERIE_A);
    }

    fn international_reading(age: u8) -> StandingReading {
        StandingReading {
            level: 128,
            group: PlayerFieldPositionGroup::Midfielder,
            effective_rep: 5_400,
            league_rep: SERIE_A as u16,
            starter_share: 0.0,
            caps: 51,
            age,
            resignation: 0.0,
            plan_widening: 0.0,
        }
    }

    /// Caps carry a name further than club form does.
    #[test]
    fn caps_widen_what_his_name_is_worth() {
        let capped = LevelFloor::of(&StandingReading {
            level: 100,
            ..international_reading(30)
        })
        .standing;
        let uncapped = LevelFloor::of(&StandingReading {
            level: 100,
            caps: 0,
            ..international_reading(30)
        })
        .standing;
        assert!(capped > uncapped, "{capped} vs {uncapped}");
    }

    /// The band widens with youth and never vanishes; months unsold and
    /// a plan to step down widen it further.
    #[test]
    fn the_tolerance_widens_with_youth_resignation_and_a_plan() {
        let at_16 = LevelFloor::tolerance(16, 0.0, 0.0);
        let at_19 = LevelFloor::tolerance(19, 0.0, 0.0);
        let at_23 = LevelFloor::tolerance(23, 0.0, 0.0);
        assert!(at_16 > at_19 && at_19 > at_23, "{at_16} {at_19} {at_23}");
        assert_eq!(at_23, LevelFloor::STEP_DOWN);
        assert_eq!(LevelFloor::tolerance(33, 0.0, 0.0), LevelFloor::STEP_DOWN);
        assert!(LevelFloor::tolerance(27, 1.0, 0.0) > at_23);
        assert!(LevelFloor::tolerance(27, 0.0, 1.0) > at_23);
        assert!(
            international(27, 1.0).below(SERIE_C) < 1.0,
            "a season unsold re-reads what level wants him"
        );
    }

    #[test]
    fn no_view_objects_to_nothing() {
        assert_eq!(LevelFloor::none().below(SERIE_C), 0.0);
        assert_eq!(international(27, 0.0).below(0.0), 0.0);
    }
}
