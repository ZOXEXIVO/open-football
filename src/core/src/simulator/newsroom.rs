use crate::club::board::manager_market::ApproachState;
use crate::club::news::RecentEvents;
use crate::league::PlayoffRoundLabel;
use crate::continent::competitions::{
    CHAMPIONS_LEAGUE_ID, CONFERENCE_LEAGUE_ID, COPA_LIBERTADORES_ID, EUROPA_LEAGUE_ID,
};
use crate::club::news::{
    Absorbing, BoardroomDesk, ClubDugoutWatch, ClubLoanWatch, ClubTransferWeek, CupTie, DugoutDesk,
    FansDesk, IssueResult, KeeperMatchFacts, LoanDesk, LoanWatchEntry, ManagerPursuit, MarketDesk,
    MatchDesk, MatchDramaFacts, MatchStarFacts, NewsEditor, NewsStory, NewspaperIssue,
    ContinentalNight, OutfieldMatchFacts, PlayoffTie, PressMood, ResultCompetition, SquadDesk, StandingSnapshot, TableDesk,
    TownMood,
    WeeklyMatchFacts,
};
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{FieldSquad, MatchResult, SubstitutionReason};
use crate::simulator::SimulatorData;
use crate::{
    Club, Country, HappinessEventType, Person, Player, PlayerFieldPositionGroup, Team, TeamType,
};
use chrono::{Datelike, Duration, NaiveDate};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

/// Weekly press run. Every local paper in the world goes to print on the
/// same Monday morning, covering the seven days just gone.
///
/// One paper per side competing under its own brand, not one per club: a
/// club with a first team, a "{Club} 2" side in a real lower division
/// and a B team goes to press three times, and each edition is about the
/// football that side played.
///
/// The pass is deliberately laid out as gather-then-write: all the
/// reading happens in parallel over an immutable world, and only the
/// finished editions are applied under `&mut`. That keeps it off the
/// critical path of the daily tick and away from the borrow gymnastics
/// the transfer pipeline needs.
pub(crate) struct NewsroomTick;

impl NewsroomTick {
    pub(crate) fn run(data: &mut SimulatorData, week_start: NaiveDate, week_end: NaiveDate) {
        let facts = WeeklyMatchFacts::from_world(data, week_start, week_end);
        let market = WeeklyMarket::from_world(data, week_start, week_end);
        let loans = WeeklyLoanWatch::from_world(data, week_end);
        let dugout = WeeklyDugout::from_world(data);

        let editions: Vec<(u32, u32, NewspaperIssue)> = data
            .continents
            .par_iter()
            .flat_map(|continent| continent.countries.par_iter())
            .flat_map_iter(|country| {
                country.clubs.iter().flat_map(|club| {
                    ClubPressRun::compile(
                        club, country, &facts, &market, &loans, &dugout, week_start, week_end,
                    )
                    .into_iter()
                    .map(|(team_id, issue)| (club.id, team_id, issue))
                })
            })
            .collect();

        for (club_id, team_id, issue) in editions {
            if let Some(team) = data
                .club_mut(club_id)
                .and_then(|club| club.teams.find_mut(team_id))
            {
                team.newsroom.publish(issue);
            }
        }
    }
}

impl WeeklyMatchFacts {
    /// Read the week's completed matches once and keep only the things
    /// no club can work out for itself: who scored three or more in a
    /// single game, who was sent off, and which knockout ties were
    /// played (a cup result never reaches a league table, so without
    /// this the desk cannot tell a cup exit from a league defeat).
    ///
    /// Domestic fixtures live in each competition's own store; only
    /// national and continental ties reach the global one. Both are
    /// walked, or the desk would never see a hat-trick in league
    /// football — which is nearly all of it.
    fn from_world(data: &SimulatorData, week_start: NaiveDate, week_end: NaiveDate) -> Self {
        let mut facts = data
            .continents
            .par_iter()
            .flat_map(|continent| continent.countries.par_iter())
            .map(|country| {
                let mut country_facts = WeeklyMatchFacts::empty();

                let playoffs = country.playoffs.iter().map(|playoff| &playoff.league);

                for league in country
                    .leagues
                    .leagues
                    .iter()
                    .chain(playoffs)
                    .filter(|league| !league.friendly)
                {
                    country_facts.absorb(league.matches.iter_in_range(week_start, week_end), false);
                }

                // The series behind the games. A playoff's fixtures run
                // through an inner league the loop above already walks,
                // so the football was always reported — as a routine
                // league Saturday, because nothing carried the stakes.
                for playoff in country.playoffs.iter() {
                    country_facts.absorb_playoff(playoff, week_start, week_end);
                }

                if let Some(cup) = country.domestic_cup.as_ref() {
                    country_facts
                        .absorb(cup.league.matches.iter_in_range(week_start, week_end), true);
                }

                country_facts
            })
            .reduce(WeeklyMatchFacts::empty, WeeklyMatchFacts::merged);

        facts.absorb(data.match_store.iter_in_range(week_start, week_end), false);
        facts
    }

    /// Fold one competition's week of results into the tally.
    fn absorb<'a>(&mut self, results: impl Iterator<Item = &'a MatchResult>, knockout: bool) {
        let mut per_match: FxHashMap<u32, u8> = FxHashMap::default();

        for result in results {
            if result.friendly {
                continue;
            }

            per_match.clear();

            for goal in &result.score.details {
                match goal.stat_type {
                    MatchStatisticType::Goal if !goal.is_auto_goal => {
                        *per_match.entry(goal.player_id).or_insert(0) += 1;
                    }
                    MatchStatisticType::RedCard => {
                        if let Some(team_id) = Self::side_of(result, goal.player_id) {
                            self.red_cards.insert((goal.player_id, team_id));
                        }
                    }
                    _ => {}
                }
            }

            for (player_id, goals) in per_match.iter() {
                if *goals < 3 {
                    continue;
                }
                let Some(team_id) = Self::side_of(result, *player_id) else {
                    continue;
                };
                let best = self.hat_tricks.entry((*player_id, team_id)).or_insert(0);
                *best = (*best).max(*goals);
            }

            self.absorb_keepers(result);
            self.absorb_outfield(result);
            self.absorb_stars(result);
            self.absorb_drama(result);
            self.absorb_continental(result);

            if knockout {
                self.absorb_cup_tie(result);
            }
        }
    }

    /// Which of the two sides in a result fielded a player. `None` when
    /// the recorded squads cannot place him — an older save without
    /// details, or a goal feed naming somebody neither squad lists — in
    /// which case nothing about him is written: an afternoon that cannot
    /// be attributed to a side is an afternoon no paper can claim.
    fn side_of(result: &MatchResult, player_id: u32) -> Option<u32> {
        let details = result.details.as_ref()?;
        if Self::fielded(&details.left_team_players, player_id) {
            return Some(details.left_team_players.team_id);
        }
        if Self::fielded(&details.right_team_players, player_id) {
            return Some(details.right_team_players.team_id);
        }
        None
    }

    /// Read each side's man of the match report out of one result: its
    /// top scorer that afternoon, ties settled by whoever struck first.
    ///
    /// This is what lets a match report carry a name instead of only a
    /// scoreline — the same channel the keeper and hat-trick beats
    /// already use, pointed at the report itself. Goals are attributed
    /// to a side through the recorded squads, so a result stored
    /// without its details (an older save, a walkover) simply
    /// contributes no star and the report prints without one.
    fn absorb_stars(&mut self, result: &MatchResult) {
        let Some(details) = result.details.as_ref() else {
            return;
        };

        // (goals, first strike) per scorer per side, walked off the
        // same goal feed the hat-trick tally reads. Own goals belong to
        // nobody's afternoon.
        let mut home: FxHashMap<u32, (u8, u64)> = FxHashMap::default();
        let mut away: FxHashMap<u32, (u8, u64)> = FxHashMap::default();

        for goal in &result.score.details {
            if goal.stat_type != MatchStatisticType::Goal || goal.is_auto_goal {
                continue;
            }

            let side = if Self::fielded(&details.left_team_players, goal.player_id) {
                &mut home
            } else if Self::fielded(&details.right_team_players, goal.player_id) {
                &mut away
            } else {
                continue;
            };

            let entry = side.entry(goal.player_id).or_insert((0, goal.time));
            entry.0 = entry.0.saturating_add(1);
            entry.1 = entry.1.min(goal.time);
        }

        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();

        self.crown(result.home_team_id, result.away_team_id, home_goals, &home);
        self.crown(result.away_team_id, result.home_team_id, away_goals, &away);
    }

    fn fielded(squad: &FieldSquad, player_id: u32) -> bool {
        squad.main.contains(&player_id) || squad.substitutes.contains(&player_id)
    }

    /// Rebuild the running score from the goal feed and read the shape
    /// of the afternoon off it for both sides.
    ///
    /// The one thing this cannot do naively is trust the scorer's own
    /// squad. An own goal is recorded against the man who put it in,
    /// who is by definition playing for the side it counts AGAINST —
    /// so crediting goals to whoever fielded the scorer would have
    /// every own-goal match reading as a comeback that never happened.
    fn absorb_drama(&mut self, result: &MatchResult) {
        let Some(details) = result.details.as_ref() else {
            return;
        };

        // (minute stamp, whether the HOME side's tally went up)
        let mut feed: Vec<(u64, bool)> = Vec::new();
        let mut home_red = false;
        let mut away_red = false;

        for goal in &result.score.details {
            let home_player = Self::fielded(&details.left_team_players, goal.player_id);
            let away_player = Self::fielded(&details.right_team_players, goal.player_id);
            if !home_player && !away_player {
                // Same rule as everywhere else in the press run: a
                // moment that cannot be placed on a side is a moment
                // no paper may claim.
                continue;
            }

            match goal.stat_type {
                MatchStatisticType::Goal => {
                    let scored_for_home = if goal.is_auto_goal {
                        !home_player
                    } else {
                        home_player
                    };
                    feed.push((goal.time, scored_for_home));
                }
                MatchStatisticType::RedCard => {
                    if home_player {
                        home_red = true;
                    } else {
                        away_red = true;
                    }
                }
                _ => {}
            }
        }

        // The feed arrives in play order, but a stored result is not a
        // contract and the whole reading depends on the order.
        feed.sort_by_key(|(time, _)| *time);

        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();
        let total = home_goals.saturating_add(away_goals);

        self.record_drama(
            result.home_team_id,
            result.away_team_id,
            Self::read_side(&feed, true, home_goals, away_goals, total, home_red),
        );
        self.record_drama(
            result.away_team_id,
            result.home_team_id,
            Self::read_side(&feed, false, away_goals, home_goals, total, away_red),
        );
    }

    /// Walk the rebuilt feed once from one side's point of view.
    fn read_side(
        feed: &[(u64, bool)],
        mine_is_home: bool,
        goals_for: u8,
        goals_against: u8,
        total_goals: u8,
        red_card: bool,
    ) -> MatchDramaFacts {
        let mut mine = 0i32;
        let mut theirs = 0i32;
        let mut max_deficit = 0i32;
        let mut max_lead = 0i32;
        let mut early = 0u8;
        let mut lead_taken: Option<u64> = None;
        let mut behind_since: Option<u64> = None;
        let mut reply_minutes = 0u8;

        for (time, scored_for_home) in feed {
            let mine_scored = *scored_for_home == mine_is_home;
            let was_ahead = mine > theirs;
            let was_behind = mine < theirs;

            if mine_scored {
                mine += 1;
            } else {
                theirs += 1;
            }

            if mine_scored && *time < MatchDramaFacts::EARLY_WINDOW_MS {
                early = early.saturating_add(1);
            }

            // The last time it went in front is the goal that won it,
            // for a side that went on to win.
            if mine_scored && !was_ahead && mine > theirs {
                lead_taken = Some(*time);
            }

            // Falling behind starts the clock on a reply; drawing level
            // stops it.
            if !mine_scored && !was_behind && mine < theirs {
                behind_since = Some(*time);
            }
            if mine_scored && was_behind && mine == theirs {
                if let Some(since) = behind_since {
                    let gap = time.saturating_sub(since);
                    if gap <= MatchDramaFacts::REPLY_WINDOW_MS {
                        // A reply forty seconds later is "within a
                        // minute", never "within nought minutes".
                        reply_minutes = ((gap / 60_000) as u8).max(1);
                    }
                }
                behind_since = None;
            }

            max_deficit = max_deficit.max(theirs - mine);
            max_lead = max_lead.max(mine - theirs);
        }

        let won = goals_for > goals_against;

        MatchDramaFacts {
            team_goals: goals_for,
            total_goals,
            winner_minute: if won {
                lead_taken.map(|time| (time / 60_000) as u16).unwrap_or(0)
            } else {
                0
            },
            max_deficit: max_deficit.max(0) as u8,
            max_lead: max_lead.max(0) as u8,
            early_goals: early,
            reply_minutes,
            red_card,
            won,
        }
    }

    /// Note a continental night for both sides.
    ///
    /// Continental results are pushed to the same global store this
    /// gather already walks, which is why a European hat-trick has
    /// always been reported correctly — and why the match it happened
    /// in was filed as an ordinary league game. The four reserved
    /// competition ids are the whole of the fix.
    ///
    /// Filtered by id rather than by anything on the result, because
    /// the global store also carries international fixtures and a
    /// country id is not a club team id.
    fn absorb_continental(&mut self, result: &MatchResult) {
        const CONTINENTAL: [u32; 4] = [
            CHAMPIONS_LEAGUE_ID,
            EUROPA_LEAGUE_ID,
            CONFERENCE_LEAGUE_ID,
            COPA_LIBERTADORES_ID,
        ];
        if !CONTINENTAL.contains(&result.league_id) {
            return;
        }

        let home = result.score.home_team.get();
        let away = result.score.away_team.get();

        self.continental.insert(
            result.home_team_id,
            ContinentalNight {
                opponent_team_id: result.away_team_id,
                goals_for: home,
                goals_against: away,
            },
        );
        self.continental.insert(
            result.away_team_id,
            ContinentalNight {
                opponent_team_id: result.home_team_id,
                goals_for: away,
                goals_against: home,
            },
        );
    }

    /// Read the week's playoff ties off the bracket itself.
    ///
    /// Anchored on `last_game_date` rather than on the match store: the
    /// question a playoff piece has to answer is not "did they play"
    /// but "is the series over", and only the bracket knows that. A
    /// best-of-three sitting at one win each has had two games this
    /// week and decided nothing.
    fn absorb_playoff(
        &mut self,
        playoff: &crate::league::LeaguePlayoff,
        week_start: NaiveDate,
        week_end: NaiveDate,
    ) {
        for series in playoff.series.iter() {
            let Some(played) = series.last_game_date else {
                continue;
            };
            if played < week_start || played >= week_end {
                continue;
            }

            let winner = series.winner();
            // Winning this one puts a side in the final, which is a
            // bigger sentence than "through to the next round" and the
            // only round distinction a supporter needs.
            let decides_a_finalist =
                playoff.round_label(series.round + 1) == PlayoffRoundLabel::Final;

            for (team_id, opponent_team_id) in [
                (series.home_team_id, series.away_team_id),
                (series.away_team_id, series.home_team_id),
            ] {
                self.playoff.insert(
                    team_id,
                    PlayoffTie {
                        opponent_team_id,
                        advanced: winner == Some(team_id),
                        eliminated: winner.is_some() && winner != Some(team_id),
                        decides_a_finalist,
                        best_of: series.best_of,
                    },
                );
            }
        }
    }

    /// Keep the louder of two meetings, on the same rule as `crown`.
    fn record_drama(&mut self, team_id: u32, opponent_team_id: u32, candidate: MatchDramaFacts) {
        let slot = self
            .drama
            .entry((team_id, opponent_team_id))
            .or_insert(candidate);
        if candidate.loudness() > slot.loudness() {
            *slot = candidate;
        }
    }

    /// Keep one side's loudest scorer for one match. The slot is keyed
    /// by (team, opponent); when a cup replay puts two meetings in one
    /// week the bigger individual afternoon survives, and the
    /// `team_goals` pin in [`WeeklyMatchFacts::star_of`] keeps it off
    /// the other meeting's report.
    fn crown(
        &mut self,
        team_id: u32,
        opponent_team_id: u32,
        team_goals: u8,
        tally: &FxHashMap<u32, (u8, u64)>,
    ) {
        let Some((player_id, (goals, _))) =
            tally
                .iter()
                .min_by_key(|(player_id, (goals, first_strike))| {
                    (std::cmp::Reverse(*goals), *first_strike, **player_id)
                })
        else {
            return;
        };

        let candidate = MatchStarFacts {
            player_id: *player_id,
            goals: *goals,
            team_goals,
        };

        let slot = self
            .stars
            .entry((team_id, opponent_team_id))
            .or_insert(candidate);
        if (candidate.goals, candidate.team_goals) > (slot.goals, slot.team_goals) {
            *slot = candidate;
        }
    }

    /// Read the goalkeepers out of one match's stat lines.
    ///
    /// A keeper's week is invisible in the goal and card feed every
    /// other desk works from, so it comes from the per-player match
    /// stats instead — which survive into stored results, only the
    /// position replay is stripped. A result recorded without them (an
    /// older save, a walkover) simply contributes no keeper facts.
    fn absorb_keepers(&mut self, result: &MatchResult) {
        let Some(details) = result.details.as_ref() else {
            return;
        };

        // Spot-kicks kept out, tallied per keeper before the stat lines
        // are walked so a shoot-out hero is credited on the same pass.
        let mut saved: FxHashMap<u32, u8> = FxHashMap::default();
        for kick in &details.penalty_shootout {
            if kick.scored {
                continue;
            }
            if let Some(keeper_id) = kick.goalkeeper_id {
                *saved.entry(keeper_id).or_insert(0) += 1;
            }
        }

        for (player_id, stats) in &details.player_stats {
            if stats.position_group != PlayerFieldPositionGroup::Goalkeeper {
                continue;
            }
            // He has to have been on the pitch. A keeper who never came
            // off the bench carries an all-zero line, and "he conceded
            // nothing" is not a story about him.
            if stats.minutes_played == 0 {
                continue;
            }
            // …and the afternoon has to belong to a side, because that
            // is what decides whose paper may print it.
            let Some(team_id) = Self::side_of(result, *player_id) else {
                continue;
            };

            self.keepers
                .entry((*player_id, team_id))
                .or_default()
                .absorb(KeeperMatchFacts {
                    saves: stats.saves,
                    conceded: stats.shots_faced.saturating_sub(stats.saves),
                    penalties_saved: saved.get(player_id).copied().unwrap_or(0),
                    errors_leading_to_goal: stats.errors_leading_to_goal,
                });
        }
    }

    /// Read the other ten out of one match's stat lines.
    ///
    /// The outfield twin of [`Self::absorb_keepers`], and the bigger of
    /// the two gaps: a goalkeeper at least had clean sheets on his
    /// record, while an outfield player's afternoon left no trace on the
    /// page at all unless he scored three or was sent off. Everything a
    /// ratings column is written from — the mark out of ten, the shots
    /// against the expected goals, the assists, the key passes, the
    /// tackles and clearances, the error that ended in the net — is in
    /// this stat line and was being thrown away every Monday.
    fn absorb_outfield(&mut self, result: &MatchResult) {
        let Some(details) = result.details.as_ref() else {
            return;
        };

        // Spot-kicks he took and missed. The scorer of a shoot-out
        // penalty is nobody's story; the man who missed one is a back
        // page, and until now the feed only ever credited the keeper.
        let mut missed: FxHashMap<u32, u8> = FxHashMap::default();
        for kick in &details.penalty_shootout {
            if kick.scored {
                continue;
            }
            *missed.entry(kick.taker_id).or_insert(0) += 1;
        }

        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();

        // Who was taken off by CHOICE. An injury swap and a manager's
        // verdict look identical in a stat line — same early exit, same
        // half-finished rating — and printing the first as the second
        // invents a decision nobody made.
        let hooked: FxHashSet<u32> = details
            .substitutions
            .iter()
            .filter(|sub| sub.reason == SubstitutionReason::Discretionary)
            .map(|sub| sub.player_out_id)
            .collect();

        for (player_id, stats) in &details.player_stats {
            if stats.position_group == PlayerFieldPositionGroup::Goalkeeper {
                continue;
            }
            // He has to have been on the pitch. An unused substitute
            // carries an all-zero line, and a mark of nothing is not a
            // bad mark — it is no mark, which is the single failure this
            // whole page has hit twice.
            if stats.minutes_played == 0 || stats.match_rating <= 0.0 {
                continue;
            }

            // What his side conceded, so a defender's shift can be told
            // apart from a defender's shift in a hiding. A stat line has
            // no idea what happened at the other end. The side itself is
            // kept too: it is what decides whose paper may print this.
            let (team_id, conceded, started) =
                if Self::fielded(&details.left_team_players, *player_id) {
                    (
                        details.left_team_players.team_id,
                        away_goals,
                        details.left_team_players.main.contains(player_id),
                    )
                } else if Self::fielded(&details.right_team_players, *player_id) {
                    (
                        details.right_team_players.team_id,
                        home_goals,
                        details.right_team_players.main.contains(player_id),
                    )
                } else {
                    // Recorded a stat line without appearing in either
                    // squad. Nothing here can be attributed to a side,
                    // so nothing is written about him.
                    continue;
                };

            let rating = (stats.match_rating * 100.0) as i32;
            let contribution = stats.goals.saturating_add(stats.assists);

            self.outfield
                .entry((*player_id, team_id))
                .or_default()
                .absorb(OutfieldMatchFacts {
                    best_rating: rating,
                    worst_rating: rating,
                    worst_rating_minutes: stats.minutes_played,
                    worst_rating_started: started,
                    worst_rating_hooked: started && hooked.contains(player_id),
                    goals: stats.goals,
                    assists: stats.assists,
                    key_passes: stats.key_passes,
                    successful_dribbles: stats.successful_dribbles,
                    defensive_actions: stats
                        .tackles
                        .saturating_add(stats.interceptions)
                        .saturating_add(stats.blocks)
                        .saturating_add(stats.clearances),
                    shots: stats.shots_total,
                    xg_x100: (stats.xg * 100.0) as i32,
                    shut_out: conceded == 0,
                    own_goals: stats.own_goals,
                    errors_leading_to_goal: stats.errors_leading_to_goal,
                    fouls: stats.fouls,
                    yellow_cards: stats.yellow_cards,
                    penalties_missed: missed.get(player_id).copied().unwrap_or(0),
                    man_of_the_match: details.player_of_the_match_id == Some(*player_id),
                    // Only a substitute can have an impact off the
                    // bench; a starter's goal is just a goal.
                    impact_off_the_bench: if started { 0 } else { contribution },
                });
        }
    }

    /// Record a knockout tie from each side's point of view, so the
    /// match desk can tell "went through" from "went out" without
    /// re-reading the fixture. A shoot-out is carried alongside the
    /// ninety minutes because that is how the tie was actually settled.
    fn absorb_cup_tie(&mut self, result: &MatchResult) {
        let home = result.score.home_team.get();
        let away = result.score.away_team.get();
        let shootout = (result.score.home_shootout, result.score.away_shootout);

        self.cup_ties.insert(
            result.home_team_id,
            CupTie {
                opponent_team_id: result.away_team_id,
                goals_for: home,
                goals_against: away,
                shootout,
            },
        );
        self.cup_ties.insert(
            result.away_team_id,
            CupTie {
                opponent_team_id: result.home_team_id,
                goals_for: away,
                goals_against: home,
                shootout: (shootout.1, shootout.0),
            },
        );
    }

    /// Rayon fold partner: two half-worlds become one.
    fn merged(mut self, other: WeeklyMatchFacts) -> Self {
        for (key, goals) in other.hat_tricks {
            let best = self.hat_tricks.entry(key).or_insert(0);
            *best = (*best).max(goals);
        }
        self.red_cards.extend(other.red_cards);
        self.cup_ties.extend(other.cup_ties);
        for (key, keeper) in other.keepers {
            self.keepers.entry(key).or_default().absorb(keeper);
        }
        for (key, outfield) in other.outfield {
            self.outfield.entry(key).or_default().absorb(outfield);
        }
        // Same louder-afternoon rule as `crown`: two half-worlds can
        // both have seen the fixture only through the global store, so
        // a straight extend would let iteration order pick the star.
        for (key, star) in other.stars {
            let slot = self.stars.entry(key).or_insert(star);
            if (star.goals, star.team_goals) > (slot.goals, slot.team_goals) {
                *slot = star;
            }
        }
        self.continental.extend(other.continental);
        self.playoff.extend(other.playoff);
        for (key, drama) in other.drama {
            let slot = self.drama.entry(key).or_insert(drama);
            if drama.loudness() > slot.loudness() {
                *slot = drama;
            }
        }
        self
    }
}

/// The week's completed transfer business, bucketed by club. Both sides
/// of a deal are recorded, so a club hears about its own departures even
/// though the player has already left the roster.
struct WeeklyMarket {
    by_club: FxHashMap<u32, ClubTransferWeek>,
}

impl WeeklyMarket {
    /// How far past the week the backwards scan of the transfer log
    /// keeps reading before it gives up. Bounds the work while leaving
    /// room for rows filed out of date order — see [`Self::gather`].
    const SCAN_MARGIN_DAYS: i64 = 31;

    fn from_world(data: &SimulatorData, week_start: NaiveDate, week_end: NaiveDate) -> Self {
        let mut market = WeeklyMarket {
            by_club: Self::gather(data, week_start, week_end),
        };
        market.enrich_arrivals(data, week_end);
        market
    }

    fn gather(
        data: &SimulatorData,
        week_start: NaiveDate,
        week_end: NaiveDate,
    ) -> FxHashMap<u32, ClubTransferWeek> {
        let mut by_club: FxHashMap<u32, ClubTransferWeek> = FxHashMap::default();

        // The history is append-ordered and never trimmed, so the walk
        // starts at the newest entry and stops once it is behind the
        // window — a decade of completed deals is not rescanned every
        // Monday. The stop is held a month back rather than on the
        // window's own edge: the log is appended from several points in
        // the tick (academy graduations, contract expiries, the market
        // pipeline, and cross-border deals filed under the *buying*
        // country), and a single row landing a day out of order would
        // otherwise cut the walk short and take that country's entire
        // week of transfer business off every front page in it, silently.
        let scan_floor = week_start - Duration::days(Self::SCAN_MARGIN_DAYS);

        for continent in &data.continents {
            for country in &continent.countries {
                for transfer in country
                    .transfer_market
                    .transfer_history
                    .iter()
                    .rev()
                    .take_while(|transfer| transfer.transfer_date >= scan_floor)
                {
                    if transfer.transfer_date < week_start || transfer.transfer_date >= week_end {
                        continue;
                    }
                    if transfer.to_club_id != 0 {
                        by_club
                            .entry(transfer.to_club_id)
                            .or_default()
                            .absorb(transfer.to_club_id, transfer);
                    }
                    if transfer.from_club_id != 0 {
                        by_club
                            .entry(transfer.from_club_id)
                            .or_default()
                            .absorb(transfer.from_club_id, transfer);
                    }
                }
            }
        }

        by_club
    }

    /// Fills in the facts about each arrival that decide which story he
    /// is: his age, and whether he has worn the buying club's colours
    /// before. The desk cannot look this up itself — a `TransferMove`
    /// deliberately carries identifiers only — and without it every
    /// returning favourite and every teenage signing prints as the same
    /// anonymous "new signing" line.
    fn enrich_arrivals(&mut self, data: &SimulatorData, today: NaiveDate) {
        for (club_id, week) in self.by_club.iter_mut() {
            if week.arrivals.is_empty() {
                continue;
            }
            let Some(club) = data.club(*club_id) else {
                continue;
            };
            let slugs: Vec<&str> = club.teams.iter().map(|team| team.slug.as_str()).collect();

            for arrival in week.arrivals.iter_mut() {
                let Some(player) = data.player(arrival.player_id) else {
                    continue;
                };
                arrival.age = player.age(today);

                let ledger = &player.statistics_history.season_ledger;
                arrival.returning = ledger
                    .iter()
                    .any(|entry| slugs.contains(&entry.team_slug.as_str()));
                // "Was here on loan" means the loan is the reason the
                // reader already knows him: his newest spell at this
                // club was a loan, and a recent one — a borrowed season
                // from years ago is a homecoming, not a buy-out.
                arrival.was_loan_here = ledger
                    .iter()
                    .rev()
                    .find(|entry| slugs.contains(&entry.team_slug.as_str()))
                    .is_some_and(|entry| {
                        entry.is_loan && i32::from(entry.season_start_year) >= today.year() - 1
                    });
            }
        }
    }

    fn for_club(&self, club_id: u32) -> Option<&ClubTransferWeek> {
        self.by_club.get(&club_id)
    }
}

/// Every loaned-out player in the world, bucketed under the club that
/// still owns his contract.
///
/// A loanee is rostered at the club borrowing him, so his parent club
/// cannot see him by walking its own squads — the loan column would
/// otherwise be the one part of the paper a club could not write.
struct WeeklyLoanWatch {
    by_parent: FxHashMap<u32, ClubLoanWatch>,
}

impl WeeklyLoanWatch {
    /// A loanee this age or younger is out there to be developed, which
    /// changes what a wasted spell means.
    const PROSPECT_AGE: u8 = 23;

    fn from_world(data: &SimulatorData, today: NaiveDate) -> Self {
        let mut by_parent: FxHashMap<u32, ClubLoanWatch> = FxHashMap::default();

        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in club.teams.iter() {
                        for player in team.players.iter() {
                            let Some(entry) = Self::entry(player, club.id, today) else {
                                continue;
                            };
                            let parent = player
                                .contract_loan
                                .as_ref()
                                .and_then(|loan| loan.loan_from_club_id)
                                .unwrap_or(0);
                            if parent == 0 {
                                continue;
                            }
                            by_parent.entry(parent).or_default().players.push(entry);
                        }
                    }
                }
            }
        }

        WeeklyLoanWatch { by_parent }
    }

    /// One loanee's spell, as the parent club would read it off a
    /// scouting report: what he has played, what he has produced, how
    /// long is left, and what he has been saying about it.
    fn entry(player: &Player, loan_club_id: u32, today: NaiveDate) -> Option<LoanWatchEntry> {
        let loan = player.contract_loan.as_ref()?;
        loan.loan_from_club_id?;

        let stats = &player.statistics;
        let rating = stats.average_rating_realistic(player.position().position_group());
        let feed = RecentEvents::fortnight(player);

        Some(LoanWatchEntry {
            player_id: player.id,
            loan_club_id,
            starts: stats.played as i32,
            sub_appearances: stats.played_subs as i32,
            goals: stats.goals as i32,
            assists: stats.assists as i32,
            rating_x100: (rating * 100.0) as i32,
            days_left: (loan.expiration - today).num_days() as i32,
            days_elapsed: loan
                .started
                .map(|started| (today - started).num_days() as i32)
                .unwrap_or(0),
            recall_available: loan
                .loan_recall_available_after
                .is_some_and(|from| from <= today),
            permanent_option: loan.loan_future_fee.is_some(),
            is_prospect: player.age(today) <= Self::PROSPECT_AGE,
            wants_permanent: feed.happened(HappinessEventType::WantsLoanMadePermanent),
            wants_to_prove_himself: feed.happened(HappinessEventType::WantsToProveHimselfAtParent),
            recall_requested: feed.happened(HappinessEventType::LoanRecallRequested),
            development_concern: feed.happened(HappinessEventType::LoanDevelopmentConcern),
            form_concern: feed.happened(HappinessEventType::LoanFormConcern),
            level_mismatch: feed.happened(HappinessEventType::LoanLevelMismatch),
            // A goal big enough to have registered on the player's own
            // record. There is no plain "he scored" event to read, and
            // the season tally above already covers volume — what this
            // adds is that one of them landed *recently*.
            //
            // The event behind it fires on an ASSIST as well as a goal —
            // "decisive contribution in a one-goal win" — and the last
            // pass before a winner can come off a goalkeeper's boot. So
            // the tally is checked too: without it the column filed "he
            // keeps scoring" about a keeper who had never scored in his
            // life, which is the one thing a paper cannot come back from.
            scored_recently: feed.happened(HappinessEventType::DecisiveGoal) && stats.goals > 0,
        })
    }

    fn for_club(&self, club_id: u32) -> Option<&ClubLoanWatch> {
        self.by_parent.get(&club_id)
    }
}

/// Who is being chased for whose dugout this week.
///
/// The manager market keeps its in-flight approaches in one world-level
/// registry, because a pursuit belongs to neither club: the requesting
/// side has no field saying "we have moved for him" and the source side
/// has no field saying "somebody wants ours". Both find out the same way
/// a supporter does — from the papers — so both are handed their half of
/// every live approach here.
struct WeeklyDugout {
    by_club: FxHashMap<u32, ClubDugoutWatch>,
}

impl WeeklyDugout {
    fn from_world(data: &SimulatorData) -> Self {
        let mut by_club: FxHashMap<u32, ClubDugoutWatch> = FxHashMap::default();

        for approach in &data.pending_manager_approaches {
            // A dead approach is not a link, and the registry keeps
            // rejected entries for one tick before reaping them.
            if matches!(approach.state, ApproachState::Rejected) {
                continue;
            }

            by_club
                .entry(approach.requesting_club_id)
                .or_default()
                .pursuits
                .push(ManagerPursuit {
                    staff_id: approach.staff_id,
                    other_club_id: approach.source_club_id,
                    we_are_asking: true,
                });
            by_club
                .entry(approach.source_club_id)
                .or_default()
                .pursuits
                .push(ManagerPursuit {
                    staff_id: approach.staff_id,
                    other_club_id: approach.requesting_club_id,
                    we_are_asking: false,
                });
        }

        WeeklyDugout { by_club }
    }

    fn for_club(&self, club_id: u32) -> Option<&ClubDugoutWatch> {
        self.by_club.get(&club_id)
    }
}

/// One club's week, turned into as many editions as it has branded
/// sides. Squads with no brand of their own (Reserve, U18..U23) are read
/// about in the first team's.
struct ClubPressRun;

impl ClubPressRun {
    fn compile(
        club: &Club,
        country: &Country,
        facts: &WeeklyMatchFacts,
        market: &WeeklyMarket,
        loans: &WeeklyLoanWatch,
        dugout: &WeeklyDugout,
        week_start: NaiveDate,
        week_end: NaiveDate,
    ) -> Vec<(u32, NewspaperIssue)> {
        let papers: Vec<&Team> = club
            .teams
            .iter()
            .filter(|team| team.team_type.is_own_team())
            .collect();

        // The club's page of record: the first team, or — for the odd
        // club whose data carries no Main side at all — whichever branded
        // side it does have. It carries the club-wide desks on top of its
        // own football.
        let Some(flagship) = club
            .teams
            .iter()
            .find(|team| team.team_type == TeamType::Main)
            .or_else(|| papers.first().copied())
        else {
            return Vec::new();
        };

        let week: Vec<Vec<IssueResult>> = papers
            .iter()
            .map(|team| Self::results(team, facts, week_start, week_end))
            .collect();

        // Resolving rivals means walking the country's club list, so it
        // only happens on the weeks one of the sides actually played.
        let rivals = if week.iter().any(|results| !results.is_empty()) {
            Self::rival_team_ids(club, country)
        } else {
            FxHashSet::default()
        };

        let transfers = market
            .for_club(club.id)
            .map(|business| TransferSplit::by_team(club, business, flagship.id))
            .unwrap_or_default();
        let peak_value = Self::squad_peak_value(club);
        // Every side the club owns, branded or not. The ratings page
        // reports football played by one of these and nothing else.
        let sides: FxHashSet<u32> = club.teams.iter().map(|team| team.id).collect();

        papers
            .iter()
            .zip(week)
            .filter_map(|(team, results)| {
                let is_flagship = team.id == flagship.id;

                let edition = TeamPressRun {
                    club,
                    team,
                    rosters: Self::rosters(club, team, is_flagship),
                    sides: &sides,
                    results,
                    standing: Self::standing(team.league_id, country, team.id),
                    rivals: &rivals,
                    facts,
                    transfers: transfers.get(&team.id),
                    peak_value,
                    // The loan column belongs to the page of record: a
                    // loanee's contract is held by the club, and nothing
                    // says which of its sides he would be playing for.
                    loans: if is_flagship {
                        loans.for_club(club.id)
                    } else {
                        None
                    },
                    // The dugout belongs to the club, not to one of its
                    // sides — a B team does not have its own manager
                    // market — so the pursuit column runs in the page of
                    // record alongside the rest of the boardroom.
                    dugout: if is_flagship {
                        dugout.for_club(club.id)
                    } else {
                        None
                    },
                    week_start,
                    flagship: is_flagship,
                }
                .compile(week_end)?;

                Some((team.id, edition))
            })
            .collect()
    }

    /// The squads one paper speaks for: its own side, plus — for the page
    /// of record — every squad with no brand of its own. Between them a
    /// club's papers cover each of its players exactly once, so nobody's
    /// week is reported twice and nobody's is dropped.
    fn rosters<'a>(club: &'a Club, team: &'a Team, is_flagship: bool) -> Vec<&'a Team> {
        let mut rosters = vec![team];
        if is_flagship {
            rosters.extend(
                club.teams
                    .iter()
                    .filter(|other| !other.team_type.is_own_team()),
            );
        }
        rosters
    }

    /// This side's fixtures inside the window, newest last so the results
    /// panel reads in the order they were played. A tie the knockout
    /// sweep saw is marked as one — the match log itself does not record
    /// which competition a fixture belonged to.
    fn results(
        team: &Team,
        facts: &WeeklyMatchFacts,
        week_start: NaiveDate,
        week_end: NaiveDate,
    ) -> Vec<IssueResult> {
        let cup_opponent = facts.cup_ties.get(&team.id).map(|tie| tie.opponent_team_id);
        // A European night is labelled before a cup tie is considered:
        // the two stores are separate and a club can play in both
        // inside one week, but only one of them is Wednesday.
        let continental_opponent = facts
            .continental
            .get(&team.id)
            .map(|night| night.opponent_team_id);
        let playoff_opponent = facts
            .playoff
            .get(&team.id)
            .map(|tie| tie.opponent_team_id);

        team.match_history
            .items()
            .iter()
            .filter(|item| {
                let played = item.date.date();
                played >= week_start && played < week_end
            })
            .map(|item| IssueResult {
                date: item.date.date(),
                opponent_team_id: item.rival_team_id,
                goals_for: item.score.0.get(),
                goals_against: item.score.1.get(),
                competition: if continental_opponent == Some(item.rival_team_id) {
                    ResultCompetition::Continental
                } else if playoff_opponent == Some(item.rival_team_id) {
                    ResultCompetition::Playoff
                } else if cup_opponent == Some(item.rival_team_id) {
                    ResultCompetition::Cup
                } else {
                    ResultCompetition::League
                },
                is_home: item.is_home,
            })
            .collect()
    }

    /// Rival clubs resolved down to the team ids a match report actually
    /// carries, so a derby is recognised from the opponent alone.
    fn rival_team_ids(club: &Club, country: &Country) -> FxHashSet<u32> {
        let mut ids = FxHashSet::default();
        for rival_club_id in &club.rivals {
            if let Some(rival) = country.clubs.iter().find(|c| c.id == *rival_club_id) {
                for team in rival.teams.teams.iter() {
                    ids.insert(team.id);
                }
            }
        }
        ids
    }

    fn standing(
        league_id: Option<u32>,
        country: &Country,
        team_id: u32,
    ) -> Option<StandingSnapshot> {
        let league_id = league_id?;
        let league = country
            .leagues
            .leagues
            .iter()
            .find(|league| league.id == league_id && !league.friendly)?;

        let rows = league.table.get();
        let index = rows.iter().position(|row| row.team_id == team_id)?;
        let row = &rows[index];

        Some(StandingSnapshot {
            position: (index + 1) as u8,
            teams: rows.len().min(u8::MAX as usize) as u8,
            points: row.effective_points(),
            played: row.played,
            // A round-robin double programme: every side plays each of
            // the others home and away.
            total_rounds: (rows.len().saturating_sub(1) * 2).min(u8::MAX as usize) as u8,
        })
    }

    /// The most valuable player currently on the books — the yardstick
    /// the market desk measures a fee against.
    fn squad_peak_value(club: &Club) -> i64 {
        club.teams
            .iter()
            .filter(|team| team.team_type.is_own_team())
            .flat_map(|team| team.players.iter())
            .map(|player| player.player_attributes.value as i64)
            .max()
            .unwrap_or(0)
    }
}

/// Everything that turns one side's week into one printed edition.
struct TeamPressRun<'a> {
    club: &'a Club,
    team: &'a Team,
    /// The squads this paper speaks for.
    rosters: Vec<&'a Team>,
    /// Every side the club owns — the football this paper is entitled to
    /// report as its own. Shared across the club's editions because it
    /// is a property of the club, not of one of its papers.
    sides: &'a FxHashSet<u32>,
    results: Vec<IssueResult>,
    standing: Option<StandingSnapshot>,
    rivals: &'a FxHashSet<u32>,
    facts: &'a WeeklyMatchFacts,
    transfers: Option<&'a ClubTransferWeek>,
    peak_value: i64,
    loans: Option<&'a ClubLoanWatch>,
    dugout: Option<&'a ClubDugoutWatch>,
    /// First day of the window the edition covers — the cut-off the
    /// club's own diary is read back to.
    week_start: NaiveDate,
    /// This side is the club's page of record.
    flagship: bool,
}

impl TeamPressRun<'_> {
    /// Recent matches the press mood is read from — roughly a month of
    /// football, which is how far back a supporter's memory really runs.
    const FORM_WINDOW: usize = 6;

    fn compile(self, date: NaiveDate) -> Option<NewspaperIssue> {
        let played_this_week = !self.results.is_empty();
        let mut candidates: Vec<NewsStory> = Vec::new();

        if played_this_week {
            MatchDesk::file(
                &mut candidates,
                &self.results,
                self.rivals,
                self.facts,
                self.team,
            );
        }
        TableDesk::file(&mut candidates, self.standing, date);

        // One walk over this paper's players feeds the squad, dugout,
        // terraces, market-verdict and rumour desks, and comes back with
        // the moods that only exist across a whole dressing room.
        let pulse = SquadDesk::file(
            &mut candidates,
            &self.rosters,
            self.sides,
            self.facts,
            played_this_week,
            date,
        );

        if let Some(transfers) = self.transfers {
            MarketDesk::file(&mut candidates, transfers, self.peak_value, date);
        }
        if let Some(watch) = self.loans {
            LoanDesk::file(&mut candidates, watch, date);
        }
        DugoutDesk::file_club(&mut candidates, &pulse, date);
        FansDesk::file_club(
            &mut candidates,
            &pulse,
            TownMood {
                supporter_pressure: self.club.board.pressure.supporter_pressure,
                standing: self.standing,
                results: &self.results,
                transfers: self.transfers,
                peak_value: self.peak_value,
            },
            date,
        );

        // The boardroom belongs to the club, so it only ever runs in the
        // page of record — a reserve side's paper reporting the sacking
        // reads as if the reserve side had done the sacking.
        if self.flagship {
            BoardroomDesk::file(
                &mut candidates,
                self.club,
                &pulse,
                self.dugout,
                self.week_start,
                date,
            );
        }

        let stories = NewsEditor::compile(candidates, &self.team.newsroom.issues);

        // A paper with nothing at all to say does not go to print. That
        // keeps dormant sides (no fixtures, no squad churn) from filling
        // the shelf with blank sheets.
        if stories.is_empty() && self.results.is_empty() {
            return None;
        }

        let mood = self.mood();

        Some(NewspaperIssue {
            number: self.team.newsroom.next_number,
            date,
            mood,
            stories,
            results: self.results,
        })
    }

    fn mood(&self) -> PressMood {
        let week = self
            .results
            .iter()
            .fold((0u8, 0u8, 0u8), |mut tally, result| {
                if result.is_win() {
                    tally.0 += 1;
                } else if result.is_draw() {
                    tally.1 += 1;
                } else {
                    tally.2 += 1;
                }
                tally
            });

        let form = self.team.match_history.recent_wins_ratio(Self::FORM_WINDOW);

        // Board confidence read as pressure: a chairman at 100 applies
        // none, a chairman at 0 is already drafting the statement. Only
        // the page of record carries it — the board does not sack the
        // manager of the "2" side over the first team's results.
        let pressure = if !self.flagship {
            0.0
        } else if self.club.board.manager_on_final_warning {
            1.0
        } else {
            ((100 - self.club.board.confidence.level.clamp(0, 100)) as f32 / 100.0).clamp(0.0, 1.0)
        };

        PressMood::read(week, form, pressure)
    }
}

/// Splits a club's completed transfer business between its papers.
///
/// An arrival is news for the side he actually joined: a "{Club} 2"
/// paper reporting its own signing is the whole point of it having a
/// paper. Everything else — departures, and arrivals into a squad with
/// no paper of its own — goes to the page of record, which is where a
/// reader would look for a player who is no longer anywhere on the books.
struct TransferSplit;

impl TransferSplit {
    fn by_team(
        club: &Club,
        business: &ClubTransferWeek,
        flagship_id: u32,
    ) -> FxHashMap<u32, ClubTransferWeek> {
        let mut split: FxHashMap<u32, ClubTransferWeek> = FxHashMap::default();

        for arrival in &business.arrivals {
            let team_id = Self::squad_of(club, arrival.player_id).unwrap_or(flagship_id);
            split.entry(team_id).or_default().arrivals.push(*arrival);
        }

        if !business.departures.is_empty() {
            split
                .entry(flagship_id)
                .or_default()
                .departures
                .extend(business.departures.iter().copied());
        }

        split
    }

    /// Which of the club's papers holds the player now. `None` when he
    /// is in a squad that prints nothing, or has already left.
    fn squad_of(club: &Club, player_id: u32) -> Option<u32> {
        club.teams
            .iter()
            .filter(|team| team.team_type.is_own_team())
            .find(|team| team.players.iter().any(|player| player.id == player_id))
            .map(|team| team.id)
    }
}

#[cfg(test)]
mod tests {
    use super::{ClubPressRun, TransferSplit, WeeklyDugout, WeeklyLoanWatch, WeeklyMarket};
    use crate::academy::ClubAcademy;
    use crate::club::news::{
        ClubTransferWeek, TransferMotive, TransferMove, TransferMoveKind, WeeklyMatchFacts,
    };
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::fullname::FullName;
    use crate::shared::{Currency, CurrencyValue, Location};
    use crate::simulator::SimulatorData;
    use crate::transfers::{CompletedTransfer, TransferType};
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes,
        Player, PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, StaffCollection, Team, TeamBuilder, TeamCollection,
        TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::NaiveDate;
    use rustc_hash::FxHashMap;

    /// A club shaped like the ones these fixtures exist for: a first
    /// team and a "2" side playing its own football.
    struct Press;

    impl Press {
        const MAIN: u32 = 10;
        const SECOND: u32 = 11;

        fn week_start() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 2, 23).unwrap()
        }

        fn week_end() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 3, 2).unwrap()
        }

        /// A ninety-minute stat line with nothing in it, for tests that
        /// only care about one or two counters.
        fn stat_line(
            position_group: crate::PlayerFieldPositionGroup,
        ) -> crate::r#match::PlayerMatchEndStats {
            crate::r#match::PlayerMatchEndStats {
                shots_on_target: 0,
                shots_total: 0,
                passes_attempted: 0,
                passes_completed: 0,
                tackles: 0,
                interceptions: 0,
                saves: 0,
                shots_faced: 0,
                goals: 0,
                assists: 0,
                match_rating: 0.0,
                raw_match_rating: 0.0,
                xg: 0.0,
                position_group,
                fouls: 0,
                yellow_cards: 0,
                red_cards: 0,
                minutes_played: 90,
                key_passes: 0,
                progressive_passes: 0,
                progressive_carries: 0,
                successful_dribbles: 0,
                attempted_dribbles: 0,
                successful_pressures: 0,
                pressures: 0,
                blocks: 0,
                clearances: 0,
                passes_into_box: 0,
                crosses_attempted: 0,
                crosses_completed: 0,
                xg_chain: 0.0,
                xg_buildup: 0.0,
                miscontrols: 0,
                heavy_touches: 0,
                carry_distance: 0,
                errors_leading_to_shot: 0,
                errors_leading_to_goal: 0,
                xg_prevented: 0.0,
                offsides: 0,
                own_goals: 0,
                zone_stats: Default::default(),
            }
        }

        fn player(id: u32) -> Player {
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("P".to_string(), format!("Player{}", id)))
                .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 15,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap()
        }

        fn team(id: u32, team_type: TeamType, players: Vec<Player>) -> Team {
            TeamBuilder::new()
                .id(id)
                .club_id(1)
                .name(format!("Team{}", id))
                .slug(format!("team-{}", id))
                .team_type(team_type)
                .league_id(Some(1))
                .reputation(TeamReputation::new(500, 500, 500))
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .training_schedule(TrainingSchedule::new(
                    chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                    chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn club() -> Club {
            Club::new(
                1,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(1_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![
                    Self::team(Self::MAIN, TeamType::Main, vec![Self::player(101)]),
                    Self::team(Self::SECOND, TeamType::Second, vec![Self::player(201)]),
                ]),
                ClubFacilities::default(),
            )
        }

        /// The same club under a chosen id, for the fixtures that need
        /// two of them in two different countries.
        fn club_with_id(club_id: u32, main_team_id: u32) -> Club {
            let mut club = Self::club();
            club.id = club_id;
            if let Some(main) = club.teams.teams.first_mut() {
                main.id = main_team_id;
            }
            for team in club.teams.teams.iter_mut() {
                team.club_id = club_id;
            }
            club
        }

        fn country(id: u32, club: Club) -> Country {
            let league = League::new(
                1,
                "L".to_string(),
                "l".to_string(),
                1,
                500,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                    split_season: false,
                },
                false,
            );

            Country::builder()
                .id(id)
                .code("EN".to_string())
                .slug("en".to_string())
                .name("England".to_string())
                .continent_id(1)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(vec![club])
                .build()
                .unwrap()
        }

        fn world(countries: Vec<Country>) -> SimulatorData {
            SimulatorData::new(
                Self::week_end().and_hms_opt(12, 0, 0).unwrap(),
                vec![Continent::new(
                    1,
                    "Europe".to_string(),
                    countries,
                    Vec::new(),
                )],
                GlobalCompetitions::new(Vec::new()),
            )
        }

        fn sale(player_id: u32, from_club_id: u32, to_club_id: u32, day: u32) -> CompletedTransfer {
            CompletedTransfer::new(
                player_id,
                "Player".to_string(),
                from_club_id,
                0,
                "From".to_string(),
                to_club_id,
                "To".to_string(),
                NaiveDate::from_ymd_opt(2026, 2, day).unwrap(),
                CurrencyValue::new(9_000_000.0, Currency::Usd),
                TransferType::Permanent,
            )
        }

        fn moved(player_id: u32, kind: TransferMoveKind) -> TransferMove {
            TransferMove {
                player_id,
                other_club_id: 90,
                fee: 4_000_000,
                kind,
                age: 26,
                returning: false,
                was_loan_here: false,
                motive: TransferMotive::Unknown,
                scout_confidence_pct: 0,
                scout_urged_it: false,
                rival: false,
            }
        }
    }

    /// A sale abroad is two stories, and the record of it lives in only
    /// one of the two countries — whichever ran the negotiation. Both
    /// clubs have to find it there: the seller's readers want to know he
    /// is gone, and the buyer's want to know he is coming.
    #[test]
    fn a_transfer_between_two_countries_reaches_both_clubs() {
        const SELLER: u32 = 1;
        const BUYER: u32 = 2;

        let seller_country = Press::country(1, Press::club_with_id(SELLER, 100));
        let mut buyer_country = Press::country(2, Press::club_with_id(BUYER, 200));

        // The deal is filed where the buying club's country runs it —
        // the seller's own country never sees the row.
        buyer_country
            .transfer_market
            .transfer_history
            .push(Press::sale(77, SELLER, BUYER, 25));

        let data = Press::world(vec![seller_country, buyer_country]);
        let market = WeeklyMarket::from_world(&data, Press::week_start(), Press::week_end());

        let sold = market
            .for_club(SELLER)
            .expect("the selling club must hear about its own sale");
        assert_eq!(sold.departures.len(), 1);
        assert_eq!(sold.departures[0].player_id, 77);

        let signed = market
            .for_club(BUYER)
            .expect("the buying club must hear about its own signing");
        assert_eq!(signed.arrivals.len(), 1);
        assert_eq!(signed.arrivals[0].player_id, 77);
    }

    /// The history is walked backwards and stopped once it is behind the
    /// week. That is only sound while the log is in date order — one row
    /// filed out of order used to hide every deal behind it, and a
    /// country's whole week of business disappeared from its papers with
    /// nothing to show anything had gone wrong.
    #[test]
    fn one_out_of_order_row_must_not_hide_the_weeks_business() {
        let mut country = Press::country(1, Press::club_with_id(1, 100));

        // This week's sale, and behind it a row stamped with an older
        // date — a pre-contract agreed months ago, an academy promotion
        // written at a different point in the tick.
        country
            .transfer_market
            .transfer_history
            .push(Press::sale(77, 1, 90, 25));
        country
            .transfer_market
            .transfer_history
            .push(Press::sale(88, 1, 90, 2));

        let data = Press::world(vec![country]);
        let market = WeeklyMarket::from_world(&data, Press::week_start(), Press::week_end());

        let business = market
            .for_club(1)
            .expect("the week's sale must survive an older row filed after it");
        assert!(
            business.departures.iter().any(|d| d.player_id == 77),
            "the sale was hidden by a row stamped before the week began"
        );
    }

    /// One paper per side competing under its own brand, and no others:
    /// squads with no brand of their own are read about in the first
    /// team's.
    #[test]
    fn only_branded_sides_print_a_paper() {
        let country = Press::country(1, Press::club());
        let editions = ClubPressRun::compile(
            &country.clubs[0],
            &country,
            &WeeklyMatchFacts::empty(),
            &WeeklyMarket {
                by_club: FxHashMap::default(),
            },
            &WeeklyLoanWatch {
                by_parent: FxHashMap::default(),
            },
            &WeeklyDugout {
                by_club: FxHashMap::default(),
            },
            Press::week_start(),
            Press::week_end(),
        );

        let sides: Vec<u32> = editions.iter().map(|(team_id, _)| *team_id).collect();
        assert!(
            sides
                .iter()
                .all(|id| *id == Press::MAIN || *id == Press::SECOND),
            "only branded sides may print: {:?}",
            sides
        );
    }

    /// An arrival is news for the side he actually joined; a departure
    /// belongs to the page of record, because the player it is about is
    /// no longer in any squad to look him up in.
    #[test]
    fn arrivals_follow_the_player_and_departures_go_to_the_page_of_record() {
        let club = Press::club();
        let business = ClubTransferWeek {
            arrivals: vec![
                Press::moved(101, TransferMoveKind::Paid),
                Press::moved(201, TransferMoveKind::Paid),
            ],
            departures: vec![Press::moved(555, TransferMoveKind::Paid)],
        };

        let split = TransferSplit::by_team(&club, &business, Press::MAIN);

        let main = split.get(&Press::MAIN).expect("the first team prints");
        let second = split.get(&Press::SECOND).expect("the 2 side prints");

        assert!(main.arrivals.iter().any(|a| a.player_id == 101));
        assert!(second.arrivals.iter().any(|a| a.player_id == 201));
        assert_eq!(main.departures.len(), 1);
        assert!(second.departures.is_empty());
    }

    /// The star channel: each side's top scorer is read off one match's
    /// goal feed, ties settled by who struck first, own goals belong to
    /// nobody, and the entry remembers the team's tally so the desk can
    /// pin it to the right result.
    #[test]
    fn the_weeks_stars_are_read_off_the_goal_feed() {
        use crate::r#match::player::statistics::MatchStatisticType;
        use crate::r#match::{FieldSquad, GoalDetail, MatchResult, MatchResultRaw, Score};

        let mut left = FieldSquad::new();
        left.team_id = 10;
        left.main = vec![101, 102];
        let mut right = FieldSquad::new();
        right.team_id = 20;
        right.main = vec![201, 202];

        let mut raw = MatchResultRaw::with_match_time(90 * 60 * 1000);
        raw.left_team_players = left;
        raw.right_team_players = right;

        let mut score = Score::new(10, 20);
        let goal = |player_id: u32, time: u64, own: bool| GoalDetail {
            player_id,
            stat_type: MatchStatisticType::Goal,
            is_auto_goal: own,
            time,
        };
        // Home 3-1: 101 opens, 102 scores twice; the third home goal is
        // 202 putting through his own net, which crowns nobody.
        score.add_goal_detail(goal(101, 10, false));
        score.add_goal_detail(goal(102, 30, false));
        score.add_goal_detail(goal(102, 60, false));
        score.add_goal_detail(goal(202, 70, true));
        score.add_goal_detail(goal(201, 80, false));
        for _ in 0..3 {
            score.increment_home_goals();
        }
        score.increment_away_goals();

        let result = MatchResult {
            id: "m".to_string(),
            league_id: 1,
            league_slug: "l".to_string(),
            home_team_id: 10,
            away_team_id: 20,
            details: Some(raw),
            score,
            friendly: false,
        };

        let mut facts = WeeklyMatchFacts::empty();
        facts.absorb(std::iter::once(&result), false);

        let home = facts.stars.get(&(10, 20)).expect("the home side scored");
        assert_eq!(
            (home.player_id, home.goals, home.team_goals),
            (102, 2, 3),
            "the brace outranks the opener, and the tally is the team's"
        );

        let away = facts.stars.get(&(20, 10)).expect("the away side scored");
        assert_eq!(
            (away.player_id, away.goals, away.team_goals),
            (201, 1, 1),
            "an own goal must never crown the man who scored it"
        );

        assert_eq!(facts.star_of(10, 20, 3), 102);
        assert_eq!(
            facts.star_of(10, 20, 2),
            0,
            "a tally from a different afternoon must not borrow this one's star"
        );
    }

    /// The shape of an afternoon, rebuilt from the minute each goal was
    /// scored in.
    ///
    /// The trap this pins down is the own goal. It is recorded against
    /// the man who put it in — who plays for the side it counts
    /// AGAINST — so a running score built from "whoever fielded the
    /// scorer" credits it to the wrong end, and every own-goal match
    /// comes out of the gather as a comeback that never happened.
    #[test]
    fn an_afternoons_shape_is_rebuilt_from_the_goal_minutes() {
        use crate::r#match::player::statistics::MatchStatisticType;
        use crate::r#match::{FieldSquad, GoalDetail, MatchResult, MatchResultRaw, Score};

        const HOME: u32 = 10;
        const AWAY: u32 = 20;
        let minute = |m: u64| m * 60 * 1000;

        let mut left = FieldSquad::new();
        left.team_id = HOME;
        left.main = vec![101, 102];
        let mut right = FieldSquad::new();
        right.team_id = AWAY;
        right.main = vec![201, 202];

        let mut raw = MatchResultRaw::with_match_time(94 * 60 * 1000);
        raw.left_team_players = left;
        raw.right_team_players = right;

        let mut score = Score::new(HOME, AWAY);
        let goal = |player_id: u32, time: u64, own: bool| GoalDetail {
            player_id,
            stat_type: MatchStatisticType::Goal,
            is_auto_goal: own,
            time,
        };

        // Away score inside ten minutes and are pegged back three
        // minutes later, go two clear before the twentieth, are pulled
        // back to level — one of them through their own net — and lose
        // it to a winner in the 91st.
        score.add_goal_detail(goal(201, minute(8), false));
        score.add_goal_detail(goal(101, minute(11), false));
        score.add_goal_detail(goal(202, minute(15), false));
        score.add_goal_detail(goal(201, minute(20), false));
        score.add_goal_detail(goal(202, minute(50), true));
        score.add_goal_detail(goal(102, minute(63), false));
        score.add_goal_detail(goal(102, minute(91), false));
        for _ in 0..4 {
            score.increment_home_goals();
        }
        for _ in 0..3 {
            score.increment_away_goals();
        }

        let result = MatchResult {
            id: "m".to_string(),
            league_id: 1,
            league_slug: "l".to_string(),
            home_team_id: HOME,
            away_team_id: AWAY,
            details: Some(raw),
            score,
            friendly: false,
        };

        let mut facts = WeeklyMatchFacts::empty();
        facts.absorb(std::iter::once(&result), false);

        let home = facts
            .drama_of(HOME, AWAY, 4)
            .expect("the home side's afternoon was read");
        assert!(home.won);
        assert_eq!(
            home.max_deficit, 2,
            "the own goal counted for the home side, so they were never three down"
        );
        assert_eq!(home.winner_minute, 91, "the winner went in after the 90th");
        assert_eq!(home.total_goals, 7);
        assert_eq!(
            home.early_goals, 1,
            "only the eleventh-minute equaliser landed inside the opening twenty"
        );
        assert_eq!(
            home.reply_minutes, 3,
            "level again three minutes after first falling behind"
        );

        let away = facts
            .drama_of(AWAY, HOME, 3)
            .expect("the away side's afternoon was read too");
        assert!(!away.won);
        assert_eq!(
            away.max_lead, 2,
            "two clear was the most the visitors were ever ahead by"
        );
        assert_eq!(
            away.early_goals, 2,
            "the twentieth-minute goal falls outside the opening-twenty window"
        );
        assert_eq!(
            away.winner_minute, 0,
            "a side that lost never took a lead it kept"
        );

        assert!(
            facts.drama_of(HOME, AWAY, 2).is_none(),
            "another afternoon's tally must not borrow this one's drama"
        );
    }

    /// Every per-player line the week records is filed under the side
    /// that played it.
    ///
    /// The gather walks the whole world, which includes internationals —
    /// where the "side" is a country and not a club team at all. Filed
    /// under the player alone, that line was indistinguishable from his
    /// club football, and the club press printed a goalkeeper's caps as
    /// its own. Recording the side is what lets a paper answer the only
    /// question that matters: was this ours.
    #[test]
    fn every_players_week_is_filed_under_the_side_that_played_it() {
        use crate::PlayerFieldPositionGroup;
        use crate::r#match::player::statistics::MatchStatisticType;
        use crate::r#match::{FieldSquad, GoalDetail, MatchResult, MatchResultRaw, Score};

        const ITALY: u32 = 380;
        const SPAIN: u32 = 724;
        const KEEPER: u32 = 101;
        const SCORER: u32 = 102;

        let mut left = FieldSquad::new();
        left.team_id = ITALY;
        left.main = vec![KEEPER, SCORER];
        let mut right = FieldSquad::new();
        right.team_id = SPAIN;
        right.main = vec![201];

        let mut raw = MatchResultRaw::with_match_time(90 * 60 * 1000);
        raw.left_team_players = left;
        raw.right_team_players = right;

        let mut keeper_line = Press::stat_line(PlayerFieldPositionGroup::Goalkeeper);
        keeper_line.saves = 8;
        keeper_line.shots_faced = 8;
        raw.player_stats.insert(KEEPER, keeper_line);

        let mut scorer_line = Press::stat_line(PlayerFieldPositionGroup::Forward);
        scorer_line.match_rating = 8.6;
        scorer_line.goals = 3;
        raw.player_stats.insert(SCORER, scorer_line);

        let mut score = Score::new(ITALY, SPAIN);
        for time in [10u64, 40, 70] {
            score.add_goal_detail(GoalDetail {
                player_id: SCORER,
                stat_type: MatchStatisticType::Goal,
                is_auto_goal: false,
                time,
            });
            score.increment_home_goals();
        }
        score.add_goal_detail(GoalDetail {
            player_id: 201,
            stat_type: MatchStatisticType::RedCard,
            is_auto_goal: false,
            time: 80,
        });

        let result = MatchResult {
            id: "int-1".to_string(),
            league_id: 0,
            league_slug: "international".to_string(),
            home_team_id: ITALY,
            away_team_id: SPAIN,
            details: Some(raw),
            score,
            friendly: false,
        };

        let mut facts = WeeklyMatchFacts::empty();
        facts.absorb(std::iter::once(&result), false);

        assert!(facts.keepers.contains_key(&(KEEPER, ITALY)));
        assert!(facts.outfield.contains_key(&(SCORER, ITALY)));
        assert_eq!(facts.hat_tricks.get(&(SCORER, ITALY)), Some(&3));
        assert!(facts.red_cards.contains(&(201, SPAIN)));

        // …and none of it is reachable from a club that happens to
        // employ them, which is the whole point of the pairing.
        let juventus: rustc_hash::FxHashSet<u32> = [1139, 98036631].into_iter().collect();
        assert!(facts.keeper_of(KEEPER, &juventus).is_none());
        assert!(facts.outfield_of(SCORER, &juventus).is_none());
        assert!(facts.hat_trick_of(SCORER, &juventus).is_none());
        assert!(!facts.was_sent_off(201, &juventus));
    }
}
