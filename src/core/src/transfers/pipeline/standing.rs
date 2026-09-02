//! Standing-based discovery — the "a scout in the stands measures STANDING,
//! not OUTPUT" channel.
//!
//! [`super::breakout::BreakoutPerformanceSignal`] can only read a scoreline:
//! goals, assists, a rating, a place on the scoring chart. That is the right
//! signal for a striker whose season is a headline, and it is blind to the
//! player this module exists for — the centre-half, the holding midfielder,
//! the full-back who is plainly the best man in his league at his job and
//! whose season produces no numbers at all. A Super Lig central midfielder
//! with seven straight thirty-game seasons at a 7.4, fifty-six caps and the
//! first shirt at his club scored ~38 on the breakout scale against a bar of
//! 45, so no club abroad ever opened a file on him. He is not a breakout. He
//! is a *standout*, and standing is a different measurement.
//!
//! So: a second, additive signal on the same 0..100 scale and behind the same
//! league-reputation discount, built only from what a market summary can
//! carry and a scout can see —
//!
//!   * the observable ability gap to the SELLER'S league starter baseline
//!     (the same anchor curve [`BigStagePull`] measures a player's standing
//!     with), fed `skill_ability` — never CA, never PA;
//!   * where he ranks in his own club's depth chart at his position;
//!   * international caps, saturating on the same curve the pull uses;
//!   * the multi-season record: a 7.4 held for three seasons is the evidence
//!     that a single 7.5 is not;
//!   * an age curve peaking through 22–26, the window in which this move
//!     actually happens.
//!
//! Discounted by league reputation exactly as the breakout signal is — a
//! standout in a rep-3500 league is a standout *there*, and the discount is
//! what stops the model treating him as one everywhere.

use crate::Player;
use crate::PlayerFieldPositionGroup;
use crate::PlayerStatCompetitionKind;
use crate::club::player::statistics::PlayerStatLedgerEntry;
use crate::club::player::transfer::{BigStagePull, BigStagePullConfig};

/// Observable career facts a standing read needs beyond the current
/// season's line. Built once per player when the world pool is assembled
/// (see [`CareerRecordSnapshot::read`]) and carried on the market summary,
/// because a club abroad has no way to walk a foreign player's ledger.
#[derive(Debug, Clone, Copy, Default)]
pub struct CareerRecordSnapshot {
    /// Sample-size-regressed average rating in the last COMPLETED season.
    /// 0.0 when he has no completed season on record.
    pub prior_season_rating: f32,
    /// Official appearances in that same season.
    pub prior_season_appearances: u16,
    /// How many prior seasons he played a regular's worth of football in.
    /// Capped at [`Self::MAX_TRACKED_SEASONS`] — beyond three the record is
    /// established and more of it adds nothing.
    pub seasons_as_regular: u8,
}

impl CareerRecordSnapshot {
    /// Appearances that make a season a regular's season. Same bar the
    /// squad-asset model uses for "was he a regular last year".
    const REGULAR_SEASON_APPS: u16 = 20;
    /// Ceiling on `seasons_as_regular`. Three consecutive regular seasons
    /// is already a career record; a fourth says nothing new about whether
    /// the player is real.
    const MAX_TRACKED_SEASONS: u8 = 3;
    /// Appearances below which last season's rating is a sample rather than
    /// a record, and contributes nothing.
    const CREDIBLE_PRIOR_APPS: u16 = 12;

    /// Read a player's completed-season record off the canonical ledger.
    ///
    /// The "latest completed season" is the newest **League** ledger year,
    /// never `Season::from_date` — a League row is only ever written by the
    /// season-end drain, so the newest one is by construction a finished
    /// campaign, while the calendar boundary disagrees with the freeze for
    /// half the year in every league that does not start in August (memory
    /// `current_season_boundary`). Cup rows are folded into the season they
    /// belong to but never define it: an inter-spell drain stamps them with
    /// the in-progress year.
    ///
    /// One pass over the ledger, then arithmetic. This runs for every senior
    /// player in the world on the weekly watch and again whenever the world
    /// pool is rebuilt, so the obvious filter-per-season shape
    /// (O(seasons × entries)) is not affordable here — it is a per-player
    /// career walk repeated once per season the player has had. Folding the
    /// ledger into a small per-year tally first makes it one walk plus a
    /// sort over a handful of years.
    pub fn read(player: &Player, position_group: PlayerFieldPositionGroup) -> Self {
        let ledger = &player.statistics_history.season_ledger;
        let mut per_year: Vec<SeasonTally> = Vec::with_capacity(8);
        for entry in ledger.iter() {
            // Friendlies (and youth age-group football, which is
            // friendly-classified) never enter a career record.
            if !entry.competition_kind.counts_toward_career_history() {
                continue;
            }
            let year = entry.season_start_year;
            let slot = match per_year.iter_mut().find(|t| t.year == year) {
                Some(slot) => slot,
                None => {
                    per_year.push(SeasonTally::new(year));
                    per_year.last_mut().expect("just pushed")
                }
            };
            slot.absorb(entry);
        }

        // The "latest completed season" is the newest year carrying a
        // League row. Everything above it is the in-progress campaign.
        let Some(latest_completed) = per_year
            .iter()
            .filter(|t| t.has_league_row)
            .map(|t| t.year)
            .max()
        else {
            return CareerRecordSnapshot::default();
        };

        let prior = per_year
            .iter()
            .find(|t| t.year == latest_completed)
            .copied()
            .unwrap_or_else(|| SeasonTally::new(latest_completed));

        // Same sample-size regression the current-season read applies, so a
        // nine-game 8.2 last year cannot out-argue a thirty-game 7.4.
        let prior_season_rating = match prior.mean_rating() {
            Some((raw, sample)) => Self::regressed(raw, sample, position_group),
            None => 0.0,
        };

        // Completed seasons in which he played a regular's worth of
        // football, walked newest-first — a career record, not this
        // season's.
        per_year.sort_unstable_by(|a, b| b.year.cmp(&a.year));
        let mut seasons_as_regular = 0u8;
        for tally in per_year.iter() {
            // A year with no League row is not a season he had — an
            // inter-spell cup drain stamps its rows with the in-progress
            // year, so counting cup-only years would credit him with a
            // campaign that is still being played.
            if tally.year > latest_completed || !tally.has_league_row {
                continue;
            }
            if tally.apps >= Self::REGULAR_SEASON_APPS {
                seasons_as_regular = seasons_as_regular.saturating_add(1);
                if seasons_as_regular >= Self::MAX_TRACKED_SEASONS {
                    break;
                }
            }
        }
        let prior_apps = prior.apps;

        CareerRecordSnapshot {
            prior_season_rating,
            prior_season_appearances: prior_apps,
            seasons_as_regular,
        }
    }

    /// True when last season is a big enough sample for its rating to count
    /// as evidence rather than noise.
    pub fn prior_season_is_credible(&self) -> bool {
        self.prior_season_appearances >= Self::CREDIBLE_PRIOR_APPS
    }

    /// Pull a raw mean toward the positional neutral by sample size —
    /// mirrors `PlayerStatistics::realistic_average_rating`, which cannot be
    /// reused here because a completed season's ledger rows carry sums, not
    /// a live statistics object.
    fn regressed(raw: f32, matches: u16, group: PlayerFieldPositionGroup) -> f32 {
        let neutral = StandingSignal::neutral_rating(group);
        let n = matches as f32;
        let weight = n / (n + 8.0);
        neutral + (raw - neutral) * weight
    }
}

/// One completed season, folded out of however many ledger rows it took
/// (league, domestic cup, continental cup, one row per spell).
///
/// A `Vec` of these plus a linear scan beats a hash map at career lengths
/// (~20 rows), and folding first is what keeps
/// [`CareerRecordSnapshot::read`] a single pass — it runs for every senior
/// player in the world every week.
#[derive(Debug, Clone, Copy)]
struct SeasonTally {
    year: u16,
    apps: u16,
    /// Σ(effective rating) over rated appearances, from the per-match
    /// ledger. Zero for a season imported from the database.
    rating_sum: f32,
    rating_matches: u16,
    /// Σ(season average × appearances) from the legacy scalar. This is the
    /// ONLY rating a database-imported season carries: the importer writes
    /// a season average and leaves both per-match ledgers empty
    /// (`database/generators/player.rs`, "accessors fall back to
    /// average_rating"). Without this fallback every player in the shipped
    /// world starts with no rating record at all, and the standing signal
    /// runs a term short until he has played a full simulated season —
    /// which is exactly the population the signal exists to find.
    legacy_rating_sum: f32,
    /// True once any row for this year is a League row. Cup rows are folded
    /// into the season they belong to but never define one.
    has_league_row: bool,
}

impl SeasonTally {
    fn new(year: u16) -> Self {
        SeasonTally {
            year,
            apps: 0,
            rating_sum: 0.0,
            rating_matches: 0,
            legacy_rating_sum: 0.0,
            has_league_row: false,
        }
    }

    fn absorb(&mut self, entry: &PlayerStatLedgerEntry) {
        let games = entry.statistics.total_games();
        self.apps = self.apps.saturating_add(games);
        self.rating_sum += entry.statistics.rating_sum;
        self.rating_matches = self
            .rating_matches
            .saturating_add(entry.statistics.rating_matches);
        self.legacy_rating_sum += entry.statistics.average_rating * games as f32;
        self.has_league_row |= entry.competition_kind == PlayerStatCompetitionKind::League;
    }

    /// `(raw mean, sample size)` for the season, preferring the per-match
    /// ledger and falling back to the imported season average. `None` when
    /// the season carries no rating evidence at all.
    fn mean_rating(&self) -> Option<(f32, u16)> {
        if self.rating_matches > 0 {
            return Some((
                self.rating_sum / self.rating_matches as f32,
                self.rating_matches,
            ));
        }
        if self.apps > 0 && self.legacy_rating_sum > 0.0 {
            return Some((self.legacy_rating_sum / self.apps as f32, self.apps));
        }
        None
    }
}

/// Everything the standing read needs about one player, in the shape a
/// market summary can supply from either side of a border.
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct StandingInputs {
    pub position_group: PlayerFieldPositionGroup,
    /// Position-weighted observable ability (`position_evaluation_ability`).
    /// Never `current_ability`, never PA — a scout reads what the player can
    /// do in the shirt he wears.
    pub skill_ability: u8,
    pub age: u8,
    /// Reputation of the competition he plays in, 0..10000.
    pub league_reputation: u16,
    /// 0-indexed rank in his club's depth chart at his position group.
    pub position_group_rank: u8,
    /// International appearances.
    pub international_apps: u16,
    /// Sample-size-regressed rating in the last COMPLETED season, and how
    /// many games it covers.
    pub prior_season_rating: f32,
    pub prior_season_appearances: u16,
    /// Completed seasons played as a regular, capped at three.
    pub seasons_as_regular: u8,
}

/// A scored standing read. Same 0..100 scale and the same
/// league-reputation discount as the breakout signal, so the admission
/// gate can take `max(breakout, standing)` against one bar.
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct StandingSignal {
    pub score: f32,
}

impl StandingSignal {
    /// Ability above the league's own starter baseline at which a player
    /// reads as a complete standout for that stage. Shared with
    /// [`BigStagePull`]'s `standout_span` — the two models must not disagree
    /// about what "towers over his league" means.
    const STANDOUT_SPAN: f32 = 18.0;
    /// Caps at which the international-exposure term saturates. Matches
    /// [`BigStagePull`]'s `caps_saturation`: a squad regular for his country
    /// is measured against better leagues every camp, and a fiftieth cap
    /// tells a scout nothing a twentieth did not.
    const CAPS_SATURATION: f32 = 20.0;

    /// Weight ceilings for the four observable axes, before the age curve
    /// and the league discount. They sum to 90, so a complete standout in a
    /// top league lands near the breakout scale's own ceiling rather than
    /// pinning at 100 and losing all resolution between good and elite.
    const ABILITY_POINTS: f32 = 38.0;
    const RANK_POINTS: f32 = 12.0;
    const CAPS_POINTS: f32 = 14.0;
    const RECORD_RATING_POINTS: f32 = 14.0;
    const RECORD_TENURE_POINTS: f32 = 12.0;

    /// Positional neutral rating — the level at which a season says nothing
    /// either way. Same anchors the breakout signal uses.
    pub(in crate::transfers::pipeline) fn neutral_rating(group: PlayerFieldPositionGroup) -> f32 {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 6.65,
            PlayerFieldPositionGroup::Defender => 6.55,
            PlayerFieldPositionGroup::Midfielder => 6.60,
            PlayerFieldPositionGroup::Forward => 6.55,
        }
    }

    pub(in crate::transfers::pipeline) fn compute(inp: &StandingInputs) -> StandingSignal {
        // ── How far he stands above his own league's starter ──
        // The baseline is emphatically not linear in reputation (see
        // `BigStagePull::league_starter_ability`): the classic exporter
        // leagues field genuinely comparable players, which is why they
        // trade with each other as peers.
        let baseline = BigStagePull::league_starter_ability(
            inp.league_reputation,
            &BigStagePullConfig::default(),
        );
        let gap = ((inp.skill_ability as f32 - baseline) / Self::STANDOUT_SPAN).clamp(0.0, 1.0);
        let ability_points = gap * Self::ABILITY_POINTS;

        // ── Where he sits in his own club's depth chart ──
        // First choice is the whole signal; a deputy carries half of it;
        // below that a scout is watching somebody else.
        let rank_points = match inp.position_group_rank {
            0 => Self::RANK_POINTS,
            1 => Self::RANK_POINTS * 0.5,
            2 => Self::RANK_POINTS * 0.17,
            _ => 0.0,
        };

        // ── International exposure ──
        let caps_points = (inp.international_apps as f32 / Self::CAPS_SATURATION).clamp(0.0, 1.0)
            * Self::CAPS_POINTS;

        // ── The multi-season record ──
        // A rating held across completed seasons is evidence; one season is
        // a sample. The rating term only counts once the season it came from
        // is itself long enough to read.
        let neutral = Self::neutral_rating(inp.position_group);
        let prior_credible =
            inp.prior_season_appearances >= CareerRecordSnapshot::CREDIBLE_PRIOR_APPS;
        let rating_points = if prior_credible {
            ((inp.prior_season_rating - neutral).max(0.0) * 18.0)
                .clamp(0.0, Self::RECORD_RATING_POINTS)
        } else {
            0.0
        };
        let tenure_points =
            (inp.seasons_as_regular.min(3) as f32 / 3.0) * Self::RECORD_TENURE_POINTS;

        let raw = ability_points + rank_points + caps_points + rating_points + tenure_points;

        // ── Age ──
        // The step-up move is bought in the prime window; a 31-year-old
        // standout in a sub-elite league is a real footballer and not a
        // market event.
        let raw = raw * Self::age_curve(inp.age);

        // ── League-reputation discount ──
        // Identical to the breakout signal's, deliberately: a standout in a
        // weak league is a standout THERE, and the two signals have to be
        // comparable against one bar.
        let rep_frac = (inp.league_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let discount = 0.45 + 0.55 * rep_frac;

        StandingSignal {
            score: (raw * discount).clamp(0.0, 100.0),
        }
    }

    /// Appetite of the market for buying this player, by age. Peaks across
    /// 22–26 — the ages at which the standout-abroad move actually happens
    /// (Ünder 20, Yazıcı 22, Kadıoğlu 24, Aktürkoğlu 25, Tosun 26) — ramps
    /// in from 17 and fades from 27 rather than stopping.
    fn age_curve(age: u8) -> f32 {
        match age {
            a if a < 17 => 0.0,
            17 => 0.45,
            18 => 0.60,
            19 => 0.72,
            20 => 0.85,
            21 => 0.95,
            22..=26 => 1.0,
            27 => 0.88,
            28 => 0.74,
            29 => 0.58,
            30 => 0.42,
            31 => 0.28,
            32 => 0.16,
            _ => 0.08,
        }
    }
}

#[cfg(test)]
mod career_record_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::statistics::{PlayerStatLedgerEntry, PlayerStatistics};
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };
    use chrono::NaiveDate;

    /// A career built one ledger row at a time, so a test can say "three
    /// regular seasons then an in-progress one" without hand-rolling a
    /// world.
    struct Career {
        player: Player,
        seq: u32,
    }

    impl Career {
        fn new() -> Self {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = 145;
            attrs.potential_ability = 145;
            let player = PlayerBuilder::new()
                .id(1)
                .full_name(FullName::new("Test".into(), "Player".into()))
                .birth_date(NaiveDate::from_ymd_opt(2002, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 20,
                    }],
                })
                .player_attributes(attrs)
                .build()
                .unwrap();
            Career { player, seq: 0 }
        }

        fn season(
            mut self,
            year: u16,
            kind: PlayerStatCompetitionKind,
            apps: u16,
            rating: f32,
        ) -> Self {
            let mut statistics = PlayerStatistics::default();
            statistics.played = apps;
            statistics.rating_matches = apps;
            statistics.rating_sum = rating * apps as f32;
            self.seq += 1;
            self.player
                .statistics_history
                .season_ledger
                .push(PlayerStatLedgerEntry {
                    seq_id: self.seq,
                    season_start_year: year,
                    team_slug: "club".into(),
                    team_name: "Club".into(),
                    team_reputation: 7000,
                    league_slug: "league".into(),
                    league_name: "League".into(),
                    competition_kind: kind,
                    competition_slug: String::new(),
                    is_loan: false,
                    transfer_fee: None,
                    coverage_days: None,
                    statistics,
                });
            self
        }

        fn read(&self) -> CareerRecordSnapshot {
            CareerRecordSnapshot::read(&self.player, PlayerFieldPositionGroup::Midfielder)
        }
    }

    #[test]
    fn an_empty_career_reads_as_nothing() {
        let record = Career::new().read();
        assert_eq!(record.prior_season_appearances, 0);
        assert_eq!(record.seasons_as_regular, 0);
        assert!(!record.prior_season_is_credible());
    }

    #[test]
    fn the_newest_league_season_is_the_completed_one() {
        // Three full seasons plus a cup slice stamped with the same year.
        // The cup games count toward that season's total; they never define
        // which season is "latest completed".
        let record = Career::new()
            .season(2024, PlayerStatCompetitionKind::League, 30, 7.2)
            .season(2025, PlayerStatCompetitionKind::League, 32, 7.4)
            .season(2026, PlayerStatCompetitionKind::League, 31, 7.5)
            .season(2026, PlayerStatCompetitionKind::DomesticCup, 5, 7.5)
            .read();
        assert_eq!(record.prior_season_appearances, 36, "31 league + 5 cup");
        assert!(record.prior_season_is_credible());
        assert!(
            record.prior_season_rating > 7.3,
            "rating {}",
            record.prior_season_rating
        );
        assert_eq!(record.seasons_as_regular, 3);
    }

    #[test]
    fn friendly_football_never_enters_a_career_record() {
        // Youth age-group football is friendly-classified. A dominant
        // academy season is not three seasons of senior evidence.
        let record = Career::new()
            .season(2026, PlayerStatCompetitionKind::League, 22, 7.0)
            .season(2025, PlayerStatCompetitionKind::Friendly, 28, 7.6)
            .season(2024, PlayerStatCompetitionKind::Friendly, 26, 7.6)
            .read();
        assert_eq!(record.seasons_as_regular, 1);
        assert_eq!(record.prior_season_appearances, 22);
    }

    #[test]
    fn a_bit_part_season_is_not_a_regular_one() {
        let record = Career::new()
            .season(2025, PlayerStatCompetitionKind::League, 30, 7.2)
            .season(2026, PlayerStatCompetitionKind::League, 6, 6.8)
            .read();
        assert_eq!(record.prior_season_appearances, 6);
        assert!(!record.prior_season_is_credible());
        assert_eq!(
            record.seasons_as_regular, 1,
            "only 2025 was a regular season"
        );
    }

    /// Every player in the SHIPPED database arrives this way: the importer
    /// writes a season average and leaves both per-match ledgers empty. Read
    /// without the fallback, the entire starting world has no rating record
    /// at all until it has played a full simulated season — which is exactly
    /// the population the standing signal exists to find.
    #[test]
    fn an_imported_season_average_is_still_a_rating() {
        let mut career = Career::new();
        let mut statistics = PlayerStatistics::default();
        statistics.played = 30;
        // Per-match ledgers empty, season average present.
        statistics.average_rating = 7.4;
        career.seq += 1;
        career
            .player
            .statistics_history
            .season_ledger
            .push(PlayerStatLedgerEntry {
                seq_id: career.seq,
                season_start_year: 2026,
                team_slug: "club".into(),
                team_name: "Club".into(),
                team_reputation: 7000,
                league_slug: "league".into(),
                league_name: "League".into(),
                competition_kind: PlayerStatCompetitionKind::League,
                competition_slug: String::new(),
                is_loan: false,
                transfer_fee: None,
                coverage_days: None,
                statistics,
            });

        let record = career.read();
        assert_eq!(record.prior_season_appearances, 30);
        assert!(
            record.prior_season_rating > 7.2,
            "an imported 7.4 over 30 games must survive the read, got {}",
            record.prior_season_rating
        );
    }

    #[test]
    fn a_short_prior_season_leaves_the_rating_out_of_the_signal() {
        // Four games at 8.5 is a sample, not a record — `StandingSignal`
        // must not read it, which it decides via `prior_season_appearances`.
        let record = Career::new()
            .season(2026, PlayerStatCompetitionKind::League, 4, 8.5)
            .read();
        assert!(!record.prior_season_is_credible());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-built standing reads. The named fixtures are the population the
    /// signal exists to separate: a genuine sub-elite standout, an ordinary
    /// starter in the same league, and the same standout past the age at
    /// which anyone would buy him.
    struct StandingFixtures;

    impl StandingFixtures {
        /// The reported case: a 24-year-old central midfielder, first choice
        /// at a Super Lig club, 56 caps, three straight regular seasons at a
        /// 7.44. Scores ~38 on the breakout scale — invisible — and is one
        /// of the better midfielders in the world.
        fn super_lig_standout() -> StandingInputs {
            StandingInputs {
                position_group: PlayerFieldPositionGroup::Midfielder,
                skill_ability: 145,
                age: 24,
                league_reputation: 7300,
                position_group_rank: 0,
                international_apps: 56,
                prior_season_rating: 7.44,
                prior_season_appearances: 32,
                seasons_as_regular: 3,
            }
        }

        /// A perfectly good Super Lig starter who is not a standout: at the
        /// league's own starter level, uncapped, a neutral rating.
        fn super_lig_starter() -> StandingInputs {
            StandingInputs {
                skill_ability: 119,
                international_apps: 0,
                prior_season_rating: 6.7,
                seasons_as_regular: 2,
                ..Self::super_lig_standout()
            }
        }
    }

    #[test]
    fn a_sub_elite_standout_clears_the_breakout_bar_on_standing_alone() {
        // The whole point: no goals in the input at all, and he is still
        // plainly worth a file abroad.
        let signal = StandingSignal::compute(&StandingFixtures::super_lig_standout());
        assert!(
            signal.score >= 45.0,
            "a capped, first-choice standout in a rep-7300 league must clear the bar (score {})",
            signal.score
        );
    }

    #[test]
    fn an_ordinary_starter_in_the_same_league_does_not() {
        let signal = StandingSignal::compute(&StandingFixtures::super_lig_starter());
        assert!(
            signal.score < 45.0,
            "an at-baseline starter must not read as a standout (score {})",
            signal.score
        );
    }

    #[test]
    fn the_same_player_fades_once_he_is_too_old_to_buy() {
        let prime = StandingSignal::compute(&StandingFixtures::super_lig_standout()).score;
        let veteran = StandingSignal::compute(&StandingInputs {
            age: 31,
            ..StandingFixtures::super_lig_standout()
        })
        .score;
        assert!(
            veteran < prime * 0.5,
            "a 31-year-old must read far weaker than the same man at 24: {veteran} vs {prime}"
        );
    }

    #[test]
    fn a_deputy_reads_weaker_than_the_first_choice() {
        let first = StandingSignal::compute(&StandingFixtures::super_lig_standout()).score;
        let deputy = StandingSignal::compute(&StandingInputs {
            position_group_rank: 1,
            ..StandingFixtures::super_lig_standout()
        })
        .score;
        assert!(deputy < first, "{deputy} !< {first}");
        assert!(deputy > 0.0);
    }

    #[test]
    fn a_weaker_league_is_discounted_for_identical_standing() {
        // Same man, same gap over his own league's baseline, weaker league:
        // the discount must bite, and must not erase him.
        let strong = StandingSignal::compute(&StandingFixtures::super_lig_standout()).score;
        let weak = StandingSignal::compute(&StandingInputs {
            league_reputation: 3500,
            skill_ability: 112, // same gap over a much lower baseline
            ..StandingFixtures::super_lig_standout()
        })
        .score;
        assert!(weak < strong, "{weak} !< {strong}");
        assert!(weak > 0.0);
    }

    #[test]
    fn one_good_season_is_not_a_record() {
        let established = StandingSignal::compute(&StandingFixtures::super_lig_standout()).score;
        let one_season = StandingSignal::compute(&StandingInputs {
            seasons_as_regular: 1,
            prior_season_appearances: 4,
            ..StandingFixtures::super_lig_standout()
        })
        .score;
        assert!(
            one_season < established,
            "a single season must be worth less than three: {one_season} vs {established}"
        );
    }

    #[test]
    fn a_keeper_can_read_as_a_standout_without_any_output() {
        // The signal never looks at goals, so the goalkeeper path is not a
        // special case — it is the ordinary one.
        let keeper = StandingSignal::compute(&StandingInputs {
            position_group: PlayerFieldPositionGroup::Goalkeeper,
            ..StandingFixtures::super_lig_standout()
        });
        assert!(keeper.score >= 45.0, "score {}", keeper.score);
    }
}
