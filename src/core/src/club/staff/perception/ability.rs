use crate::Player;

use super::potential::PotentialEstimator;

/// Staff-formed read of *how good a player is right now*, built only from
/// signals a coach can actually observe — never from the hidden
/// `player_attributes.current_ability` (CA) digit.
///
/// A club's coaching staff never see the biological CA number. What they
/// see is: the player on the training pitch (his visible, position-weighted
/// skill level), how he has *performed* when given competitive minutes (his
/// match ratings), and how he *applies himself day to day* (his training
/// performance). This estimator fuses exactly those three observable
/// channels into one 1..200 "assessed level" the squad-role classifier can
/// rank players by, so a player is judged on results and application rather
/// than a number the coach can't see.
///
/// Mirror of [`PotentialEstimator::observable_ceiling`] — that one answers
/// "how high could he go?" from visible signals; this one answers "how good
/// is he *now*?" from visible signals. Both deliberately refuse to read the
/// hidden attributes (`current_ability` / `potential_ability`).
///
/// Deterministic and staff-free: no judging noise, no per-coach bias, no
/// date dependence. A player's assessed level is a pure function of his
/// visible skills, season stat ledger, and training EMA, so the classifier
/// stays reproducible across a load/save round-trip. When a per-coach,
/// noisy read is wanted the perception layer's `perceived_quality` provides
/// it on a different scale; this is the shared, objective baseline.
pub struct AbilityEstimator;

impl AbilityEstimator {
    /// Minimum official (league + cup) appearances before the match-results
    /// channel is allowed to move the assessment. Below it the sample says
    /// nothing — a benched player is read at his visible skill level, never
    /// penalised for minutes he was never given. Deliberately small: the
    /// rating itself is already sample-regressed (see
    /// [`crate::PlayerStatistics::combined_realistic_average_rating`]), so
    /// this only guards the zero / near-zero sample.
    const RESULTS_MIN_APPS: u16 = 3;

    /// Positional-neutral match rating — the league-average performance a
    /// squad player is measured against. Mirrors the private
    /// `statistics::neutral_rating` (which spans 6.55–6.65 by position); the
    /// 0.10 positional spread is immaterial at this overlay's resolution, so
    /// one shared constant keeps the estimator self-contained.
    const NEUTRAL_RATING: f32 = 6.60;

    /// Ability points credited per full rating point above neutral. A
    /// full-season 7.6 (a point above the ~6.6 baseline) reads as a player
    /// performing ~10 levels above his bare skill; a 5.6 reads ~10 below.
    const RATING_TO_ABILITY: f32 = 10.0;

    /// Cap on the match-results swing so a purple patch (or a slump) colours
    /// the assessment without ever swamping the visible-skill anchor. Kept
    /// below the classifier's `SURPLUS_GAP` (25) so form alone can move a
    /// player a rotation tier but never teleport him across the whole
    /// surplus gap — skill still dominates, results modulate.
    const RESULTS_CAP: f32 = 15.0;

    /// Neutral training-performance value (the `PlayerTraining` EMA starts
    /// here); deviations above/below feed the training channel.
    const NEUTRAL_TRAINING: f32 = 10.0;

    /// Ability points per training-performance point away from neutral. A
    /// player tearing training up at 18/20 reads a few levels above his bare
    /// skill; a persistent slacker at 4/20 a few below.
    const TRAIN_TO_ABILITY: f32 = 0.8;

    /// Cap on the training swing. Smaller than [`Self::RESULTS_CAP`] —
    /// training is softer evidence than competitive matches, but it is the
    /// *only* fresh signal for the fringe / reserve players the surplus
    /// judgement most often concerns, so it is never zero.
    const TRAIN_CAP: f32 = 6.0;

    /// Standing (`current_reputation.max(home_reputation)`) at/below which a
    /// player carries no special reputation — an ordinary pro. Below it the
    /// reputation channel is silent. On the codebase's ~0..10000 reputation
    /// scale, ~4000 sits just under the "high-profile" band (5000+).
    const REP_STANDING_NEUTRAL: f32 = 4000.0;

    /// Standing at which the reputation nudge saturates — a genuine star
    /// (world-class reputation ~9000).
    const REP_STANDING_SATURATION: f32 = 9000.0;

    /// Maximum reputation nudge, in ability points. Deliberately the
    /// *softest* channel — a recognised name gets the benefit of the doubt on
    /// his current level, but it is one-sided (never a penalty: being unknown
    /// is not evidence of being worse) and smaller than actual match results.
    /// The classifier separately applies an intra-squad top-quartile
    /// "recognised name" rescue; this absolute nudge is complementary — it
    /// colours the level of notable players who aren't their squad's very top
    /// name. Tune this to change how much a reputation buys.
    const REP_CAP: f32 = 8.0;

    /// The coach-observable current level on the 1..200 ability scale.
    ///
    /// `visible_ability` (position-weighted from the player's actual skills)
    /// is the anchor — the eye-test read every observer shares. It is then
    /// nudged by:
    ///   * **match results** — his sample-regressed league+cup rating versus
    ///     the positional neutral (only once he has a real sample),
    ///   * **training performance** — his rolling training EMA versus its
    ///     neutral, and
    ///   * **reputation** — a one-sided "benefit of the doubt" for a
    ///     recognised name (the softest channel; never a penalty).
    ///
    /// Never reads `current_ability`. With no matches, neutral training and no
    /// standing it returns exactly the visible skill ability, so a squad of
    /// genuinely equal-skill players ranks by their results, application and
    /// standing — which is precisely how a coach separates them.
    pub fn observable_level(player: &Player) -> u8 {
        let base = PotentialEstimator::visible_ability(player) as f32;
        let results_adj = Self::results_adjustment(player);
        let training_adj = Self::training_adjustment(player);
        let reputation_adj = Self::reputation_adjustment(player);
        ((base + results_adj + training_adj + reputation_adj).round() as i16).clamp(1, 200) as u8
    }

    /// Match-results channel: how far his regressed league+cup rating sits
    /// above/below the positional neutral, converted to ability points and
    /// capped. Zero until he has a meaningful sample, so idle minutes never
    /// read as a demerit.
    fn results_adjustment(player: &Player) -> f32 {
        let apps = player.statistics.played
            + player.statistics.played_subs
            + player.cup_statistics.played
            + player.cup_statistics.played_subs;
        if apps < Self::RESULTS_MIN_APPS {
            return 0.0;
        }
        let group = player.position().position_group();
        let rating = player
            .statistics
            .combined_realistic_average_rating(&player.cup_statistics, group);
        if rating <= 0.0 {
            return 0.0;
        }
        ((rating - Self::NEUTRAL_RATING) * Self::RATING_TO_ABILITY)
            .clamp(-Self::RESULTS_CAP, Self::RESULTS_CAP)
    }

    /// Training channel: how far his rolling training EMA sits above/below
    /// neutral, converted to ability points and capped. Always present (the
    /// EMA always exists), so it is the fresh signal for players with no
    /// recent minutes.
    fn training_adjustment(player: &Player) -> f32 {
        ((player.training.training_performance - Self::NEUTRAL_TRAINING) * Self::TRAIN_TO_ABILITY)
            .clamp(-Self::TRAIN_CAP, Self::TRAIN_CAP)
    }

    /// Reputation channel: a recognised name gets the benefit of the doubt on
    /// his current level. Reads his standing (`current_reputation` or
    /// `home_reputation`, whichever is higher — the same figure the classifier
    /// uses for its "recognised name" rescue), and lifts the level from zero
    /// at [`Self::REP_STANDING_NEUTRAL`] up to [`Self::REP_CAP`] at
    /// [`Self::REP_STANDING_SATURATION`]. One-sided: below the neutral there
    /// is no adjustment (an unknown player is judged purely on skill / results
    /// / training, never docked for having no name).
    fn reputation_adjustment(player: &Player) -> f32 {
        let standing = player
            .player_attributes
            .current_reputation
            .max(player.player_attributes.home_reputation) as f32;
        let norm = ((standing - Self::REP_STANDING_NEUTRAL)
            / (Self::REP_STANDING_SATURATION - Self::REP_STANDING_NEUTRAL))
            .clamp(0.0, 1.0);
        norm * Self::REP_CAP
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };
    use chrono::NaiveDate;

    /// Fixtures for the ability estimator. Player quality is expressed the
    /// way the estimator reads it — through *skills* — not by poking the
    /// hidden CA digit, which the estimator deliberately ignores.
    struct Fx;

    impl Fx {
        /// Flat skills (every technical / mental / physical / goalkeeping
        /// attribute equal) tuned so the position-weighted visible ability
        /// lands on `target`. Inverts `PlayerSkills::skill_to_ability`:
        /// ability = ((v-1)/19) * 199 + 1, so v = 1 + (target-1)/199*19.
        fn skills_for(target: u8) -> PlayerSkills {
            PlayerSkills::flat_for_ability(target)
        }

        /// A contracted player whose *visible* ability is `visible` (set via
        /// skills). `hidden_ca` is stamped onto `current_ability` purely to
        /// prove the estimator ignores it.
        fn player(id: u32, visible: u8, hidden_ca: u8) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = hidden_ca;
            attrs.potential_ability = 200;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Test".to_string(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(Fx::skills_for(visible))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(None)
                .build()
                .unwrap()
        }

        /// Seed a full-season league sample at a fixed average rating.
        fn seed_rating(player: &mut Player, games: u16, rating: f32) {
            player.statistics.played = games;
            // Drive the weighted ledger directly so the realistic average
            // regresses off a real sample rather than the legacy fallback.
            player.statistics.rating_points = rating * games as f32;
            player.statistics.rating_weight = games as f32;
            player.statistics.average_rating = rating;
        }
    }

    /// The headline invariant: the estimate depends on visible skills, NOT
    /// the hidden CA digit. Two players with identical skills but wildly
    /// different `current_ability` must read the same level.
    #[test]
    fn ignores_hidden_current_ability() {
        let inflated = Fx::player(1, 110, 200);
        let honest = Fx::player(1, 110, 40);
        assert_eq!(
            AbilityEstimator::observable_level(&inflated),
            AbilityEstimator::observable_level(&honest),
            "assessed level must come from visible skills, not the CA digit",
        );
    }

    /// With no matches and neutral training, the assessed level is exactly
    /// the visible skill ability.
    #[test]
    fn falls_back_to_visible_ability_without_signal() {
        let p = Fx::player(1, 120, 120);
        let visible = PotentialEstimator::visible_ability(&p);
        assert_eq!(AbilityEstimator::observable_level(&p), visible);
    }

    /// A benched player (zero appearances) is never penalised by the results
    /// channel — idle minutes are not evidence of being worse.
    #[test]
    fn zero_appearances_do_not_penalise() {
        let p = Fx::player(1, 120, 120);
        assert!(
            AbilityEstimator::observable_level(&p) >= PotentialEstimator::visible_ability(&p),
            "no-sample player must not drop below his visible ability",
        );
    }

    /// Two identically-skilled players separated only by results: the strong
    /// performer is assessed clearly higher than the poor performer.
    #[test]
    fn strong_results_lift_over_poor_results() {
        let mut strong = Fx::player(1, 110, 110);
        Fx::seed_rating(&mut strong, 30, 7.6);
        let mut poor = Fx::player(1, 110, 110);
        Fx::seed_rating(&mut poor, 30, 5.6);

        let strong_level = AbilityEstimator::observable_level(&strong);
        let poor_level = AbilityEstimator::observable_level(&poor);
        assert!(
            strong_level > poor_level + 10,
            "strong performer ({strong_level}) should out-assess poor performer ({poor_level})",
        );
    }

    /// Training performance separates two otherwise-identical fringe players
    /// with no match minutes — the only fresh signal a coach has for them.
    #[test]
    fn training_performance_separates_fringe_players() {
        let mut grafter = Fx::player(1, 100, 100);
        grafter.training.training_performance = 18.0;
        let mut slacker = Fx::player(1, 100, 100);
        slacker.training.training_performance = 4.0;

        assert!(
            AbilityEstimator::observable_level(&grafter)
                > AbilityEstimator::observable_level(&slacker),
            "the hard trainer should be assessed above the slacker",
        );
    }

    /// The results swing is capped — a freak 9.9 season cannot swamp the
    /// visible-skill anchor.
    #[test]
    fn results_swing_is_bounded() {
        let mut p = Fx::player(1, 100, 100);
        Fx::seed_rating(&mut p, 30, 9.9);
        let visible = PotentialEstimator::visible_ability(&p) as i16;
        let level = AbilityEstimator::observable_level(&p) as i16;
        let max_overlay = (AbilityEstimator::RESULTS_CAP
            + AbilityEstimator::TRAIN_CAP
            + AbilityEstimator::REP_CAP) as i16;
        assert!(
            level - visible <= max_overlay,
            "combined overlay must stay within its caps",
        );
    }

    /// A recognised name gets the benefit of the doubt: identical skill, but
    /// the high-reputation player is assessed above the unknown.
    #[test]
    fn reputation_lifts_a_recognised_name() {
        let mut star = Fx::player(1, 110, 110);
        star.player_attributes.current_reputation = 9000;
        star.player_attributes.home_reputation = 9000;
        let unknown = Fx::player(1, 110, 110); // default reputation 0
        assert!(
            AbilityEstimator::observable_level(&star)
                > AbilityEstimator::observable_level(&unknown),
            "a recognised name should out-assess an unknown of identical skill",
        );
    }

    /// The reputation channel is one-sided: a low standing is never a penalty
    /// — being unknown is not evidence of being worse.
    #[test]
    fn sub_neutral_reputation_is_not_a_penalty() {
        let mut p = Fx::player(1, 120, 120);
        p.player_attributes.current_reputation = 500;
        p.player_attributes.home_reputation = 500;
        assert_eq!(
            AbilityEstimator::observable_level(&p),
            PotentialEstimator::visible_ability(&p),
            "sub-neutral reputation must not move the assessed level",
        );
    }

    /// The reputation nudge saturates at its cap — no reputation, however
    /// stratospheric, can lift the level beyond `REP_CAP`.
    #[test]
    fn reputation_boost_saturates_at_the_cap() {
        let mut modest = Fx::player(1, 100, 100);
        modest.player_attributes.current_reputation = 6000;
        let mut superstar = Fx::player(1, 100, 100);
        superstar.player_attributes.current_reputation = 20_000;
        let visible = PotentialEstimator::visible_ability(&modest) as i16;
        let modest_lift = AbilityEstimator::observable_level(&modest) as i16 - visible;
        let star_lift = AbilityEstimator::observable_level(&superstar) as i16 - visible;
        assert!(star_lift >= modest_lift);
        assert!(star_lift <= AbilityEstimator::REP_CAP as i16);
    }

    /// Deterministic: identical inputs always produce the identical level.
    #[test]
    fn deterministic_for_fixed_inputs() {
        let mut p = Fx::player(7, 115, 115);
        Fx::seed_rating(&mut p, 20, 7.1);
        p.training.training_performance = 12.0;
        assert_eq!(
            AbilityEstimator::observable_level(&p),
            AbilityEstimator::observable_level(&p),
        );
    }
}
