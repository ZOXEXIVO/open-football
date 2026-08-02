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

use crate::club::player::player::Player;
use crate::utils::DateUtils;
use crate::{PlayerAttributes, TeamType};
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
    /// season to find out what he has joined.
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
            settle_days: 365,
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
    fn league_starter_ability(league_reputation: u16, _config: &BigStagePullConfig) -> f32 {
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

impl PlayerAttributes {
    /// Convenience for the market side: the ability figure the big-stage
    /// model measures a player by. Kept here so the pull and the market
    /// never disagree about what "how good is he" means.
    pub fn stage_ability(&self) -> f32 {
        self.current_ability as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills,
    };

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn player(ca: u8, ambition: f32, loyalty: f32, age: u8, caps: u16) -> Player {
        let today = d(2026, 8, 1);
        let birth = today
            .checked_sub_signed(chrono::Duration::days(age as i64 * 365))
            .unwrap();
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ca;
        attrs.potential_ability = ca;
        attrs.international_apps = caps;
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
}
