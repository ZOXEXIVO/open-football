use crate::club::team::reputation::{Achievement, AchievementType};
use crate::{
    AwardReputationInput, AwardReputationKind, Country, HappinessEventType, Player,
    TrophyEventContext, TrophyKind,
};
use chrono::NaiveDate;
use log::debug;
use rustc_hash::{FxHashMap, FxHashSet};

struct CupAwardSnapshot {
    winner_team_id: u32,
    cup_slug: String,
    cup_name: String,
    cup_league_id: u32,
    cup_reputation: u16,
    final_appearance_ids: FxHashSet<u32>,
}

struct CupEligiblePlayer {
    player_id: u32,
    apps: u16,
    starts: u16,
    sub_apps: u16,
    goals: u16,
    assists: u16,
    clean_sheets: u16,
    avg_rating: f32,
    final_appearance: bool,
}

impl Country {
    /// Emit the domestic cup winner-trophy fan-out for the country's cup.
    ///
    /// Unlike league-title silverware (which lands on every player in the
    /// champion roster), cup medals are tied to *appearance*: a player
    /// only gets the event + award if they actually featured in the
    /// winning campaign. Eligibility is read from
    /// `cup_statistics_by_competition[cup_slug]` — which `process_local`
    /// has already populated by the time this runs in the parallel tick,
    /// including final-day starters and used substitutes. Unused
    /// substitutes, January signings who never made the bench, and
    /// youth players merely registered won't have an entry and so get
    /// nothing — matching how real-football medals are awarded.
    ///
    /// Must be invoked AFTER the day's `LeagueResult::process_local` pass
    /// — otherwise the final's appearances aren't on the player yet and
    /// the entire winning XI silently fails the eligibility check.
    ///
    /// Idempotent across ticks via the `DomesticCup::award_emitted_*`
    /// markers — the post-final daily tick re-enters this helper but
    /// `should_emit_winner_award` returns `None` on the second visit.
    pub(crate) fn on_domestic_cup_winner_awards(&mut self, date: NaiveDate) {
        let Some(snapshot) = Self::cup_award_snapshot(self) else {
            return;
        };
        let eligible = Self::eligible_cup_players(self, &snapshot);
        if eligible.is_empty() {
            // No one to award. Leave the cup marker unset so the next
            // tick (once `process_local` has caught up) can fire the
            // achievement + awards once instead of duplicating them.
            debug!(
                "Domestic cup {} resolved (winner team {}) but no eligible players this tick — \
                 deferring achievement + awards",
                snapshot.cup_name, snapshot.winner_team_id
            );
            return;
        }

        Self::award_cup_players(self, &snapshot, eligible, date);
        if let Some(cup) = self.domestic_cup.as_mut() {
            cup.mark_winner_award_emitted(snapshot.winner_team_id, date);
        }
    }

    fn cup_award_snapshot(country: &Country) -> Option<CupAwardSnapshot> {
        let cup = country.domestic_cup.as_ref()?;
        let winner_team_id = cup.should_emit_winner_award(&country.clubs)?;
        let mut final_ids = FxHashSet::default();
        if let Some((home_id, away_id)) = cup.champion_final_pairing(&country.clubs)
            && let Some(last_tour) = cup.league.schedule.tours.last()
            && let Some(item) = last_tour.items.first()
            && let Some(mr) = cup.league.matches.get(&item.id)
            && let Some(details) = mr.details.as_ref()
        {
            let winning_squad = if winner_team_id == home_id {
                Some(&details.left_team_players)
            } else if winner_team_id == away_id {
                Some(&details.right_team_players)
            } else {
                None
            };
            if let Some(squad) = winning_squad {
                final_ids.extend(squad.main.iter().copied());
                final_ids.extend(squad.substitutes_used.iter().copied());
            }
        }
        Some(CupAwardSnapshot {
            winner_team_id,
            cup_slug: cup.league.slug.clone(),
            cup_name: cup.league.name.clone(),
            cup_league_id: cup.league.id,
            cup_reputation: cup.league.reputation,
            final_appearance_ids: final_ids,
        })
    }

    fn eligible_cup_players(
        country: &Country,
        snapshot: &CupAwardSnapshot,
    ) -> Vec<CupEligiblePlayer> {
        let mut eligible = Vec::new();
        for club in &country.clubs {
            for team in &club.teams.teams {
                if team.id != snapshot.winner_team_id {
                    continue;
                }
                for player in &team.players.players {
                    let Some(entry) = player
                        .cup_statistics_by_competition
                        .iter()
                        .find(|entry| entry.competition_slug == snapshot.cup_slug)
                    else {
                        continue;
                    };
                    let stats = &entry.statistics;
                    if stats.total_games() == 0 {
                        continue;
                    }
                    let realistic =
                        stats.average_rating_realistic(player.position().position_group());
                    let avg_rating = if realistic > 0.0 {
                        realistic
                    } else {
                        stats.weighted_average_rating()
                    };
                    eligible.push(CupEligiblePlayer {
                        player_id: player.id,
                        apps: stats.total_games(),
                        starts: stats.played,
                        sub_apps: stats.played_subs,
                        goals: stats.goals,
                        assists: stats.assists,
                        clean_sheets: stats.clean_sheets,
                        avg_rating,
                        final_appearance: snapshot.final_appearance_ids.contains(&player.id),
                    });
                }
            }
        }
        eligible
    }

    fn award_cup_players(
        country: &mut Country,
        snapshot: &CupAwardSnapshot,
        eligible: Vec<CupEligiblePlayer>,
        date: NaiveDate,
    ) {
        let rep_norm = (snapshot.cup_reputation as f32 / 10_000.0).clamp(0.0, 1.0);
        let base_prestige = (0.70 + 0.45 * rep_norm).clamp(0.70, 1.15);
        let eligibility: FxHashMap<u32, CupEligiblePlayer> = eligible
            .into_iter()
            .map(|player| (player.player_id, player))
            .collect();

        for club in &mut country.clubs {
            let club_id = club.id;
            let mut club_owns_winner = false;
            for team in &mut club.teams.teams {
                if team.id != snapshot.winner_team_id {
                    continue;
                }
                club_owns_winner = true;
                team.on_season_trophy(Achievement::new(AchievementType::CupWin, date, 8));

                for player in &mut team.players.players {
                    let Some(eligible) = eligibility.get(&player.id) else {
                        continue;
                    };
                    Self::award_cup_player(
                        player,
                        snapshot,
                        eligible,
                        base_prestige,
                        club_id,
                        date,
                    );
                }
            }
            if club_owns_winner {
                club.board.on_achievement(AchievementType::CupWin);
            }
        }
    }

    fn award_cup_player(
        player: &mut Player,
        snapshot: &CupAwardSnapshot,
        eligible: &CupEligiblePlayer,
        base_prestige: f32,
        club_id: u32,
        date: NaiveDate,
    ) {
        let base_involvement: f32 = match eligible.apps {
            1 => 0.65,
            2 => 0.80,
            _ => 0.95,
        };
        let final_bonus: f32 = if eligible.final_appearance { 0.10 } else { 0.0 };
        let involvement = (base_involvement + final_bonus).min(1.10_f32);
        let effective_prestige = (base_prestige * involvement).min(1.10);

        let trophy_ctx = TrophyEventContext::new(TrophyKind::DomesticCup)
            .with_competition_id(snapshot.cup_league_id)
            .with_competition_slug(snapshot.cup_slug.clone())
            .with_competition_name(snapshot.cup_name.clone())
            .with_winner_team_id(snapshot.winner_team_id)
            .with_apps(eligible.apps)
            .with_starts(eligible.starts)
            .with_used_sub_apps(eligible.sub_apps)
            .with_goals(eligible.goals)
            .with_assists(eligible.assists)
            .with_clean_sheets(eligible.clean_sheets)
            .with_final_appearance(eligible.final_appearance);
        let trophy_ctx = if eligible.avg_rating > 0.0 {
            trophy_ctx.with_avg_rating(eligible.avg_rating)
        } else {
            trophy_ctx
        };

        player.on_trophy_won_with_context(
            HappinessEventType::DomesticCupWon,
            trophy_ctx,
            365,
            effective_prestige,
            true,
            date,
        );
        player.remember_season_outcome(&HappinessEventType::DomesticCupWon, club_id, date);

        let mut input = AwardReputationInput::new()
            .with_league_id(snapshot.cup_league_id)
            .with_league_reputation(snapshot.cup_reputation)
            .with_matches_played(eligible.apps);
        if eligible.avg_rating > 0.0 {
            input = input.with_avg_rating(eligible.avg_rating);
        }
        player.apply_award_reputation_impact(AwardReputationKind::DomesticCupWinner, input, date);
    }

    /// Emit the champion fan-out for resolved grouped-competition playoffs
    /// (MLS Cup, Torneo Apertura/Clausura) plus the Supporters' Shield for
    /// MLS-format competitions. The playoff champion IS the competition's
    /// league title — the zone/conference table toppers get no title of
    /// their own (see the `playoff_crowned` guard in
    /// `process_season_awards`).
    ///
    /// Runs daily; idempotent via the playoff's own `award_emitted_*` /
    /// `shield_award_emitted_*` markers, so it is a no-op on every tick
    /// where nothing new has resolved.
    pub(crate) fn on_playoff_winner_awards(&mut self, date: NaiveDate) {
        for idx in 0..self.playoffs.len() {
            // Champion trophy — squad-wide, participation-scaled.
            let champion = {
                let pf = &self.playoffs[idx];
                pf.should_emit_winner_award().map(|winner| {
                    (
                        winner,
                        pf.league.id,
                        pf.league.slug.clone(),
                        pf.league.name.clone(),
                        pf.league.reputation,
                    )
                })
            };
            if let Some((winner_team_id, league_id, slug, name, reputation)) = champion {
                let rep_norm = (reputation as f32 / 10_000.0).clamp(0.0, 1.0);
                let prestige = (0.80 + 0.40 * rep_norm).clamp(0.80, 1.20);
                for club in self.clubs.iter_mut() {
                    let club_id = club.id;
                    let mut owns_winner = false;
                    for team in club.teams.iter_mut() {
                        if team.id != winner_team_id {
                            continue;
                        }
                        owns_winner = true;
                        team.on_season_trophy(Achievement::new(
                            AchievementType::LeagueTitle,
                            date,
                            8,
                        ));
                        for player in team.players.iter_mut() {
                            let trophy_ctx = TrophyEventContext::new(TrophyKind::LeagueTitle)
                                .with_competition_id(league_id)
                                .with_competition_slug(slug.clone())
                                .with_competition_name(name.clone())
                                .with_winner_team_id(winner_team_id);
                            player.on_trophy_won_with_context(
                                HappinessEventType::TrophyWon,
                                trophy_ctx,
                                365,
                                prestige,
                                false,
                                date,
                            );
                            player.remember_season_outcome(
                                &HappinessEventType::TrophyWon,
                                club_id,
                                date,
                            );
                        }
                    }
                    if owns_winner {
                        club.board.on_achievement(AchievementType::LeagueTitle);
                    }
                }
                self.playoffs[idx].mark_winner_award_emitted(winner_team_id, date);
            }

            // Supporters' Shield — best regular-season record, awarded the
            // moment the playoff seeds (i.e. when every group finished).
            let shield = {
                let pf = &self.playoffs[idx];
                pf.should_emit_shield_award()
                    .map(|team_id| (team_id, pf.league.id))
            };
            if let Some((shield_team_id, league_id)) = shield {
                for club in self.clubs.iter_mut() {
                    for team in club.teams.iter_mut() {
                        if team.id != shield_team_id {
                            continue;
                        }
                        for player in team.players.iter_mut() {
                            let trophy_ctx = TrophyEventContext::new(TrophyKind::LeagueTitle)
                                .with_competition_id(league_id)
                                .with_competition_name("Supporters' Shield".to_string())
                                .with_winner_team_id(shield_team_id);
                            player.on_trophy_won_with_context(
                                HappinessEventType::TrophyWon,
                                trophy_ctx,
                                365,
                                0.6,
                                false,
                                date,
                            );
                        }
                    }
                }
                self.playoffs[idx].mark_shield_award_emitted();
            }
        }
    }
}
