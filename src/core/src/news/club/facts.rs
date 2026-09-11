use crate::PlayerFieldPositionGroup;
use crate::club::news::{
    Absorbing, ContinentalNight, CupTie, KeeperMatchFacts, MatchDramaFacts, MatchStarFacts,
    OutfieldMatchFacts, PlayoffTie, WeeklyMatchFacts,
};
use crate::continent::competitions::{
    CHAMPIONS_LEAGUE_ID, CONFERENCE_LEAGUE_ID, COPA_LIBERTADORES_ID, EUROPA_LEAGUE_ID,
};
use crate::league::PlayoffRoundLabel;
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{FieldSquad, MatchResult, SubstitutionReason};
use crate::world::SimulatorData;
use chrono::NaiveDate;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

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
    pub(super) fn from_world(
        data: &SimulatorData,
        week_start: NaiveDate,
        week_end: NaiveDate,
    ) -> Self {
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

        // A match that ended level had its last goal level it, by
        // definition — so the equaliser is the last entry in the feed,
        // and whose it was is the whole of the story.
        let (equaliser_minute, equaliser_ours) = match feed.last() {
            Some((time, scored_for_home)) if goals_for == goals_against && total_goals > 0 => (
                ((time / 60_000) as u16).max(1),
                *scored_for_home == mine_is_home,
            ),
            _ => (0, false),
        };

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
            equaliser_minute,
            equaliser_ours,
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
