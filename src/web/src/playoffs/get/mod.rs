pub mod routes;

use crate::leagues::address::LeagueAddress;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::shared::slug::PlayerSlug;
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use core::league::{CROSS_BRACKET, LeaguePlayoff, PlayoffRoundLabel, PlayoffSeries, ScheduleItem};
use core::r#match::player::statistics::MatchStatisticType;
use core::{Country, SimulatorData};
use serde::Deserialize;
use std::cmp::Reverse;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct PlayoffGetRequest {
    pub lang: String,
    pub playoff_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "playoffs/get/index.html")]
pub struct PlayoffGetTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_flag: Option<SubTitleFlag>,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: I18n,
    pub lang: String,
    pub playoff_slug: String,
    pub active_tab: &'static str,
    /// True once the bracket has resolved to a single champion.
    pub is_decided: bool,
    pub champion_name: String,
    pub champion_slug: String,
    pub entrants: usize,
    pub rounds_count: usize,
    /// Label of the furthest round drawn, shown in the hero while the
    /// playoff is still being contested (empty once `is_decided`, or
    /// before the bracket draw).
    pub stage_label: String,
    /// Current edition's Supporters' Shield holder (MLS format only).
    pub has_shield: bool,
    pub shield_name: String,
    pub shield_slug: String,
    /// Bracket sections: one per conference for per-conference formats
    /// (plus a closing section for the cross-conference final), a single
    /// unlabelled section for merged brackets.
    pub sections: Vec<PlayoffSection>,
    pub top_scorers: Vec<PlayoffPlayerStat>,
    pub top_assisters: Vec<PlayoffPlayerStat>,
}

pub struct PlayoffSection {
    /// Conference display name; empty for merged brackets (the template
    /// falls back to the generic bracket heading).
    pub label: String,
    /// The cross-conference final section renders its lone card centered.
    pub centered: bool,
    pub rounds: Vec<PlayoffRound>,
}

pub struct PlayoffRound {
    pub label: String,
    pub is_final: bool,
    pub series: Vec<PlayoffSeriesCard>,
}

/// One knockout tie card — a single game or a best-of-N series. Single
/// games carry the game score (and shootout tally) per row; series carry
/// the series win tally per row plus one chip per game underneath.
pub struct PlayoffSeriesCard {
    pub home_name: String,
    pub home_slug: String,
    pub away_name: String,
    pub away_slug: String,
    /// Seed rank within the team's own group (1 = group winner); empty
    /// when unknown.
    pub home_seed: String,
    pub away_seed: String,
    /// Per-row score cell: goals for single games, series wins for a
    /// best-of-N; `None` while nothing has been played.
    pub home_score: Option<String>,
    pub away_score: Option<String>,
    /// Shootout tallies; `Some` only for a single game settled on penalties.
    pub home_pens: Option<String>,
    pub away_pens: Option<String>,
    pub home_winner: bool,
    pub away_winner: bool,
    pub played: bool,
    /// Single games link their score cell straight to the match page.
    pub match_id: String,
    /// Best-of-N series only: one chip per game (G1/G2/G3), unplayed
    /// games render as "–".
    pub games: Vec<PlayoffGameChip>,
    /// Final at a nominally neutral venue — display tag only.
    pub neutral: bool,
}

pub struct PlayoffGameChip {
    pub label: String,
    pub match_id: String,
    pub score: String,
    pub played: bool,
}

pub struct PlayoffPlayerStat {
    pub player_slug: String,
    pub player_name: String,
    pub team_name: String,
    pub team_slug: String,
    pub played: u16,
    pub stat_value: String,
}

/// Home goals, away goals and the shoot-out score, series-oriented.
type GameGoals = (u8, u8, Option<(u8, u8)>);

impl PlayoffRound {
    /// I18n label for a playoff round stage.
    fn label(i18n: &I18n, label: PlayoffRoundLabel) -> String {
        match label {
            PlayoffRoundLabel::WildCard => i18n.t("wild_card").to_string(),
            PlayoffRoundLabel::RoundOne => i18n.t("round_one").to_string(),
            PlayoffRoundLabel::RoundOf16 => i18n.t("round_of_16").to_string(),
            PlayoffRoundLabel::QuarterFinal => i18n.t("quarter_finals").to_string(),
            PlayoffRoundLabel::SemiFinal => i18n.t("semi_finals").to_string(),
            PlayoffRoundLabel::ConferenceSemiFinal => i18n.t("conference_semi_finals").to_string(),
            PlayoffRoundLabel::ConferenceFinal => i18n.t("conference_finals").to_string(),
            PlayoffRoundLabel::Final => i18n.t("final").to_string(),
        }
    }
}

pub struct PlayoffGetPage<'a> {
    data: &'a SimulatorData,
    i18n: I18n,
    lang: String,
    country: &'a Country,
    playoff: &'a LeaguePlayoff,
}

impl<'a> PlayoffGetPage<'a> {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<PlayoffGetRequest>,
    ) -> ApiResult<Response> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let guard = state.data.read().await;
        let simulator_data = guard.as_ref().unwrap();

        let league_id = simulator_data
            .indexes
            .as_ref()
            .unwrap()
            .slug_indexes
            .get_league_by_slug(&route_params.playoff_slug)
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "Playoff with slug {} not found",
                    route_params.playoff_slug
                ))
            })?;

        let league = simulator_data.league(league_id).unwrap();

        // The playoff route only serves grouped-competition playoffs. A normal
        // league slug is bounced to the standings page.
        if !league.is_cup {
            return Ok(Redirect::to(
                &LeagueAddress::of(simulator_data, league).url(&route_params.lang),
            )
            .into_response());
        }

        let country = simulator_data.country(league.country_id).unwrap();

        // A cup slug (domestic cup, not a playoff) belongs to the cup page.
        let Some(playoff) = country.playoffs.iter().find(|p| p.league.id == league_id) else {
            return Ok(
                Redirect::to(&format!("/{}/cups/{}", route_params.lang, league.slug))
                    .into_response(),
            );
        };

        let page = PlayoffGetPage {
            data: simulator_data,
            i18n,
            lang: route_params.lang,
            country,
            playoff,
        };
        Ok(page.render().into_response())
    }

    fn render(self) -> PlayoffGetTemplate {
        let summary = PlayoffSummary::of(&self);
        let stats = PlayoffPlayerStats::collect(self.data, self.playoff);
        let title = views::Menus::league_display_name(&self.playoff.league, &self.i18n, self.data);
        let current_path = format!("/{}/playoffs/{}", self.lang, self.playoff.league.slug);
        let menu_sections = self.menu_sections(&current_path);

        PlayoffGetTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title,
            sub_title_prefix: String::new(),
            sub_title_suffix: String::new(),
            sub_title: self.country.name.clone(),
            sub_title_link: format!("/{}/countries/{}", self.lang, self.country.slug),
            sub_title_flag: Some(SubTitleFlag::of(self.country, &self.lang)),
            header_color: self.country.background_color.clone(),
            foreground_color: self.country.foreground_color.clone(),
            menu_sections,
            playoff_slug: self.playoff.league.slug.clone(),
            active_tab: "bracket",
            is_decided: summary.is_decided,
            champion_name: summary.champion_name,
            champion_slug: summary.champion_slug,
            entrants: summary.entrants,
            rounds_count: summary.rounds_count,
            stage_label: summary.stage_label,
            has_shield: summary.has_shield,
            shield_name: summary.shield_name,
            shield_slug: summary.shield_slug,
            sections: self.sections(),
            top_scorers: stats.top_scorers,
            top_assisters: stats.top_assisters,
            lang: self.lang,
            i18n: self.i18n,
        }
    }

    fn team_info(&self, team_id: u32) -> (String, String) {
        self.data
            .team(team_id)
            .map(|d| (d.name.clone(), d.slug.clone()))
            .unwrap_or_default()
    }

    /// All scheduled games of a series, in play order: the tour matching
    /// the series round holds them as `ScheduleItem`s whose pair equals the
    /// series pair in either orientation (game 2 of a best-of-3 swaps the
    /// venue).
    fn series_games(&self, s: &PlayoffSeries) -> Vec<&ScheduleItem> {
        let mut items: Vec<&ScheduleItem> = self
            .playoff
            .league
            .schedule
            .tours
            .iter()
            .filter(|t| t.num == s.round)
            .flat_map(|t| &t.items)
            .filter(|i| {
                (i.home_team_id == s.home_team_id && i.away_team_id == s.away_team_id)
                    || (i.home_team_id == s.away_team_id && i.away_team_id == s.home_team_id)
            })
            .collect();
        items.sort_by_key(|i| i.date);
        items
    }

    /// Goals of one played game, oriented to the series' home/away sides.
    /// The schedule item's ids may sit in either slot of the stored `Score`,
    /// so map through the recorded `team_id`s like the cup page's `build_tie`.
    fn game_goals(&self, s: &PlayoffSeries, item: &ScheduleItem) -> Option<GameGoals> {
        let res = item.result.as_ref()?;
        let series_home_first = s.home_team_id == res.home_team.team_id;
        let (hg, ag) = if series_home_first {
            (res.home_team.get(), res.away_team.get())
        } else {
            (res.away_team.get(), res.home_team.get())
        };
        let pens = if res.had_shootout() {
            if series_home_first {
                Some((res.home_shootout, res.away_shootout))
            } else {
                Some((res.away_shootout, res.home_shootout))
            }
        } else {
            None
        };
        Some((hg, ag, pens))
    }

    fn card(&self, s: &PlayoffSeries) -> PlayoffSeriesCard {
        let (home_name, home_slug) = self.team_info(s.home_team_id);
        let (away_name, away_slug) = self.team_info(s.away_team_id);
        let games = self.series_games(s);
        let winner = s.winner();
        let played = games.iter().any(|g| g.result.is_some());
        let seed_of = |team_id: u32| -> String {
            self.playoff
                .seed_rank
                .get(&team_id)
                .map(|r| r.to_string())
                .unwrap_or_default()
        };

        let mut card = PlayoffSeriesCard {
            home_seed: seed_of(s.home_team_id),
            away_seed: seed_of(s.away_team_id),
            home_name,
            home_slug,
            away_name,
            away_slug,
            home_score: None,
            away_score: None,
            home_pens: None,
            away_pens: None,
            home_winner: winner == Some(s.home_team_id),
            away_winner: winner == Some(s.away_team_id),
            played,
            match_id: String::new(),
            games: Vec::new(),
            neutral: s.neutral,
        };

        if s.best_of > 1 {
            self.fill_series(&mut card, s, &games, winner);
        } else {
            self.fill_single(&mut card, s, &games);
        }
        card
    }

    /// Best-of-N: rows carry the series tally, chips carry the games.
    fn fill_series(
        &self,
        card: &mut PlayoffSeriesCard,
        s: &PlayoffSeries,
        games: &[&ScheduleItem],
        winner: Option<u32>,
    ) {
        let mut chips: Vec<PlayoffGameChip> = games
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                // Chip score in the game's own home/away orientation.
                let score = match item.result.as_ref() {
                    Some(res) => {
                        let item_home_first = item.home_team_id == res.home_team.team_id;
                        let (hg, ag) = if item_home_first {
                            (res.home_team.get(), res.away_team.get())
                        } else {
                            (res.away_team.get(), res.home_team.get())
                        };
                        if res.had_shootout() {
                            let (hp, ap) = if item_home_first {
                                (res.home_shootout, res.away_shootout)
                            } else {
                                (res.away_shootout, res.home_shootout)
                            };
                            format!("{}–{} ({}–{})", hg, ag, hp, ap)
                        } else {
                            format!("{}–{}", hg, ag)
                        }
                    }
                    None => "–".to_string(),
                };
                PlayoffGameChip {
                    label: format!("G{}", idx + 1),
                    match_id: item.id.clone(),
                    score,
                    played: item.result.is_some(),
                }
            })
            .collect();
        // While the series is live, the not-yet-scheduled decider still
        // shows as an empty chip; a series closed out early drops it.
        if winner.is_none() {
            for n in games.len()..s.best_of as usize {
                chips.push(PlayoffGameChip {
                    label: format!("G{}", n + 1),
                    match_id: String::new(),
                    score: "–".to_string(),
                    played: false,
                });
            }
        }
        if card.played {
            card.home_score = Some(s.home_wins.to_string());
            card.away_score = Some(s.away_wins.to_string());
        }
        card.games = chips;
    }

    /// Single game: the row scores are the game score, penalty-aware.
    fn fill_single(
        &self,
        card: &mut PlayoffSeriesCard,
        s: &PlayoffSeries,
        games: &[&ScheduleItem],
    ) {
        let item = games.first();
        card.match_id = item.map(|i| i.id.clone()).unwrap_or_default();
        if let Some((hg, ag, pens)) = item.and_then(|i| self.game_goals(s, i)) {
            card.home_score = Some(hg.to_string());
            card.away_score = Some(ag.to_string());
            if let Some((hp, ap)) = pens {
                card.home_pens = Some(hp.to_string());
                card.away_pens = Some(ap.to_string());
            }
        }
    }

    /// Rounds of one bracket, earliest first, each with its series in slot
    /// order.
    fn bracket_rounds(&self, bracket: usize) -> Vec<PlayoffRound> {
        let mut round_nums: Vec<u8> = self
            .playoff
            .series
            .iter()
            .filter(|s| s.bracket == bracket)
            .map(|s| s.round)
            .collect();
        round_nums.sort_unstable();
        round_nums.dedup();
        round_nums
            .into_iter()
            .map(|round| {
                let mut in_round: Vec<&PlayoffSeries> = self
                    .playoff
                    .series
                    .iter()
                    .filter(|s| s.bracket == bracket && s.round == round)
                    .collect();
                in_round.sort_by_key(|s| s.slot);
                let label = self.playoff.round_label(round);
                PlayoffRound {
                    label: PlayoffRound::label(&self.i18n, label),
                    is_final: label == PlayoffRoundLabel::Final,
                    series: in_round.into_iter().map(|s| self.card(s)).collect(),
                }
            })
            .collect()
    }

    /// Per-conference formats get one labelled section per conference plus
    /// a closing section for the cross-conference final; merged brackets
    /// collapse to a single unlabelled section.
    fn sections(&self) -> Vec<PlayoffSection> {
        let has_cross_final = self
            .playoff
            .series
            .iter()
            .any(|s| s.bracket == CROSS_BRACKET);
        let per_conference = self.playoff.group_league_ids.len() > 1
            && self
                .playoff
                .series
                .iter()
                .any(|s| s.bracket != CROSS_BRACKET && s.bracket != 0);

        let mut sections: Vec<PlayoffSection> = Vec::new();
        if per_conference {
            for (bracket, group_league_id) in self.playoff.group_league_ids.iter().enumerate() {
                let rounds = self.bracket_rounds(bracket);
                if rounds.is_empty() {
                    continue;
                }
                let label = self
                    .data
                    .league(*group_league_id)
                    .map(|l| l.name.clone())
                    .unwrap_or_default();
                sections.push(PlayoffSection {
                    label,
                    centered: false,
                    rounds,
                });
            }
        } else if self.playoff.series.iter().any(|s| s.bracket == 0) {
            sections.push(PlayoffSection {
                label: String::new(),
                centered: false,
                rounds: self.bracket_rounds(0),
            });
        }
        if has_cross_final {
            sections.push(PlayoffSection {
                label: self.i18n.t("final").to_string(),
                centered: true,
                rounds: self.bracket_rounds(CROSS_BRACKET),
            });
        }
        sections
    }

    fn menu_sections(&self, current_path: &str) -> Vec<MenuSection> {
        let country_leagues: Vec<(&str, &str)> = self
            .country
            .leagues
            .leagues
            .iter()
            .filter(|l| !l.friendly)
            .map(|l| (l.name.as_str(), l.slug.as_str()))
            .collect();
        let country_playoffs: Vec<(&str, &str)> = self
            .country
            .playoffs
            .iter()
            .map(|p| (p.league.name.as_str(), p.league.slug.as_str()))
            .collect();
        let mp = views::MenuParams {
            i18n: &self.i18n,
            lang: &self.lang,
            current_path,
            country_name: &self.country.name,
            country_slug: &self.country.slug,
        };
        views::Menus::playoff_menu(
            &mp,
            &country_leagues,
            self.country
                .domestic_cup
                .as_ref()
                .map(|c| (c.league.name.as_str(), c.league.slug.as_str())),
            &country_playoffs,
            self.country.continent_id,
        )
    }
}

struct PlayoffSummary {
    is_decided: bool,
    champion_name: String,
    champion_slug: String,
    entrants: usize,
    rounds_count: usize,
    stage_label: String,
    has_shield: bool,
    shield_name: String,
    shield_slug: String,
}

impl PlayoffSummary {
    fn of(page: &PlayoffGetPage<'_>) -> PlayoffSummary {
        let champion_id = page.playoff.champion();
        let is_decided = champion_id.is_some();
        let (champion_name, champion_slug) =
            champion_id.map(|id| page.team_info(id)).unwrap_or_default();

        // While the playoff is live, the furthest round drawn is its current
        // stage; blanked once a champion is known or before the draw.
        let stage_label = if is_decided {
            String::new()
        } else {
            page.playoff
                .series
                .iter()
                .map(|s| s.round)
                .max()
                .map(|round| PlayoffRound::label(&page.i18n, page.playoff.round_label(round)))
                .unwrap_or_default()
        };

        let entrants = if page.playoff.seed_rank.is_empty() {
            page.playoff.qualifiers_per_group as usize * page.playoff.group_league_ids.len()
        } else {
            page.playoff.seed_rank.len()
        };
        let rounds_count = {
            let mut rounds: Vec<u8> = page.playoff.series.iter().map(|s| s.round).collect();
            rounds.sort_unstable();
            rounds.dedup();
            rounds.len()
        };

        let (shield_name, shield_slug) = page
            .playoff
            .shield_team_id
            .map(|id| page.team_info(id))
            .unwrap_or_default();
        let has_shield = !shield_name.is_empty();

        PlayoffSummary {
            is_decided,
            champion_name,
            champion_slug,
            entrants,
            rounds_count,
            stage_label,
            has_shield,
            shield_name,
            shield_slug,
        }
    }
}

struct PlayoffPlayerStats {
    top_scorers: Vec<PlayoffPlayerStat>,
    top_assisters: Vec<PlayoffPlayerStat>,
}

impl PlayoffPlayerStats {
    fn collect(data: &SimulatorData, playoff: &LeaguePlayoff) -> PlayoffPlayerStats {
        let tally = PlayoffTally::collect(playoff);
        PlayoffPlayerStats {
            top_scorers: PlayoffPlayerStat::rows(data, &tally.apps, &tally.goals),
            top_assisters: PlayoffPlayerStat::rows(data, &tally.apps, &tally.assists),
        }
    }
}

struct PlayoffTally {
    goals: HashMap<u32, u16>,
    assists: HashMap<u32, u16>,
    apps: HashMap<u32, u16>,
}

impl PlayoffTally {
    /// Playoff-scoped player tallies, read from this competition's own
    /// match records (same source the cup page uses).
    fn collect(playoff: &LeaguePlayoff) -> PlayoffTally {
        let mut goals: HashMap<u32, u16> = HashMap::new();
        let mut assists: HashMap<u32, u16> = HashMap::new();
        let mut apps: HashMap<u32, u16> = HashMap::new();

        for mr in playoff.league.matches.iter() {
            for d in &mr.score.details {
                match d.stat_type {
                    MatchStatisticType::Goal if !d.is_auto_goal => {
                        *goals.entry(d.player_id).or_insert(0) += 1;
                    }
                    MatchStatisticType::Assist => {
                        *assists.entry(d.player_id).or_insert(0) += 1;
                    }
                    _ => {}
                }
            }
            if let Some(details) = &mr.details {
                for side in [&details.left_team_players, &details.right_team_players] {
                    for &pid in side.main.iter().chain(side.substitutes_used.iter()) {
                        *apps.entry(pid).or_insert(0) += 1;
                    }
                }
            }
        }
        PlayoffTally {
            goals,
            assists,
            apps,
        }
    }
}

impl PlayoffPlayerStat {
    fn rows(
        data: &SimulatorData,
        apps: &HashMap<u32, u16>,
        tally: &HashMap<u32, u16>,
    ) -> Vec<PlayoffPlayerStat> {
        let mut rows: Vec<(u32, String, String, String, u16, u16)> = Vec::new();
        for (&pid, &value) in tally {
            let Some(player) = data.player(pid) else {
                continue;
            };
            let Some((_p, team)) = data.player_with_team(pid) else {
                continue;
            };
            let played = apps.get(&pid).copied().unwrap_or(0);
            rows.push((
                pid,
                player.full_name.to_string(),
                team.name.clone(),
                team.slug.clone(),
                played,
                value,
            ));
        }
        rows.sort_by_key(|r| Reverse(r.5));
        rows.into_iter()
            .take(10)
            .map(
                |(pid, player_name, team_name, team_slug, played, value)| PlayoffPlayerStat {
                    player_slug: PlayerSlug::history(data, pid, &player_name),
                    player_name,
                    team_name,
                    team_slug,
                    played,
                    stat_value: value.to_string(),
                },
            )
            .collect()
    }
}
