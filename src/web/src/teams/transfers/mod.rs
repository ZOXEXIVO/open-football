pub mod routes;

use crate::leagues::address::LeagueAddress;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::shared::slug::PlayerSlug;
use crate::teams::newspaper::NewspaperCounter;
use crate::teams::{TeamLookup, TeamNeighborhood};
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use chrono::{Datelike, NaiveDate};
use core::transfers::{CompletedTransfer, TransferType};
use core::utils::{DateUtils, FormattingUtils};
use core::{Player, PlayerFieldPositionGroup, PlayerSquadStatus, SimulatorData, Team, TeamType};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamTransfersRequest {
    lang: String,
    team_slug: String,
}

#[derive(Deserialize)]
pub struct SeasonQuery {
    pub season: Option<u16>,
}

pub struct SeasonOption {
    pub year: u16,
    pub display: String,
    pub selected: bool,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/transfers/index.html")]
pub struct TeamTransfersTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub i18n: I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_flag: Option<SubTitleFlag>,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub active_tab: &'static str,
    pub show_finances_tab: bool,
    pub show_academy_tab: bool,
    /// Printed items waiting on the newspaper tab, for the tabbar badge.
    pub newspaper_count: usize,
    pub items: Vec<TransferListRow>,
    pub list_value: String,
    pub ledger: TransferLedger,
    pub seasons: Vec<SeasonOption>,
}

/// A player as every transfers table shows him. A record whose player
/// no longer exists keeps only the stored name, shown with initials.
pub struct TransferPlayerView {
    pub id: u32,
    pub slug: String,
    pub name: String,
    pub initials: String,
    pub known: bool,
    pub generated: bool,
    pub country_code: String,
    pub country_name: String,
    pub age: u8,
    pub position: &'static str,
    pub position_group: &'static str,
}

impl TransferPlayerView {
    fn of(data: &SimulatorData, player: &Player, now: NaiveDate) -> Self {
        let (country_code, country_name) = data
            .country(player.country_id)
            .map(|c| (c.code.clone(), c.name.clone()))
            .or_else(|| {
                data.country_info
                    .get(&player.country_id)
                    .map(|i| (i.code.clone(), i.name.clone()))
            })
            .unwrap_or_default();
        let first = player.full_name.display_first_name();
        let last = player.full_name.display_last_name();
        let position = player.position();

        TransferPlayerView {
            id: player.id,
            slug: player.slug(),
            name: format!("{} {}", first, last),
            initials: Self::initials(&format!("{} {}", first, last)),
            known: true,
            generated: player.is_generated(),
            country_code,
            country_name,
            age: DateUtils::age(player.birth_date, now),
            position: position.get_short_name(),
            position_group: match position.position_group() {
                PlayerFieldPositionGroup::Goalkeeper => "gk",
                PlayerFieldPositionGroup::Defender => "def",
                PlayerFieldPositionGroup::Midfielder => "mid",
                PlayerFieldPositionGroup::Forward => "fwd",
            },
        }
    }

    fn of_record(data: &SimulatorData, id: u32, stored_name: &str, now: NaiveDate) -> Self {
        match data.player(id) {
            Some(player) => Self::of(data, player, now),
            None => TransferPlayerView {
                id,
                slug: PlayerSlug::history(data, id, stored_name),
                name: stored_name.to_string(),
                initials: Self::initials(stored_name),
                known: false,
                generated: false,
                country_code: String::new(),
                country_name: String::new(),
                age: 0,
                position: "",
                position_group: "",
            },
        }
    }

    fn initials(name: &str) -> String {
        let mut words = name.split_whitespace();
        let first = words.next().and_then(|w| w.chars().next());
        let last = words.last().and_then(|w| w.chars().next());
        first.into_iter().chain(last).collect()
    }
}

/// The other side of a move: a club in its colours, or the free-agent pool.
pub struct TransferClubView {
    pub name: String,
    pub slug: String,
    pub background: String,
    pub foreground: String,
    pub free_agent: bool,
}

impl TransferClubView {
    fn of(data: &SimulatorData, club_id: u32, stored_name: &str, i18n: &I18n) -> Self {
        if club_id == 0 {
            return TransferClubView {
                name: i18n.t("free_agent").to_string(),
                slug: String::new(),
                background: String::new(),
                foreground: String::new(),
                free_agent: true,
            };
        }
        let club = data.club(club_id);
        TransferClubView {
            name: stored_name.to_string(),
            slug: club
                .and_then(|c| c.teams.teams.iter().find(|t| t.team_type == TeamType::Main))
                .map(|t| t.slug.clone())
                .unwrap_or_default(),
            background: club
                .map(|c| c.colors.background.clone())
                .unwrap_or_default(),
            foreground: club
                .map(|c| c.colors.foreground.clone())
                .unwrap_or_default(),
            free_agent: false,
        }
    }
}

pub struct TransferListRow {
    pub player: TransferPlayerView,
    pub squad_status: String,
    pub squad_tone: &'static str,
    pub contract_until: String,
    pub wage: String,
    pub value: String,
}

impl TransferListRow {
    /// Most valuable first, with the summed value of the whole list.
    fn collect(
        data: &SimulatorData,
        team: &Team,
        now: NaiveDate,
        i18n: &I18n,
    ) -> (Vec<TransferListRow>, String) {
        let league_rep = team
            .league_id
            .and_then(|id| data.league(id))
            .map(|l| l.reputation)
            .unwrap_or(0);
        let club_rep = team.reputation.market_value_score();
        let players = team.players();

        let mut valued: Vec<(&Player, f64)> = team
            .transfer_list
            .items()
            .iter()
            .filter_map(|item| players.iter().find(|p| p.id == item.player_id).copied())
            .map(|p| (p, p.value(now, league_rep, club_rep)))
            .collect();
        valued.sort_by(|a, b| b.1.total_cmp(&a.1));

        let total = valued.iter().map(|(_, v)| v).sum();
        let rows = valued
            .into_iter()
            .map(|(p, value)| {
                let contract = p.contract.as_ref();
                TransferListRow {
                    player: TransferPlayerView::of(data, p, now),
                    squad_status: contract
                        .map(|c| i18n.t(c.squad_status.i18n_key()).to_string())
                        .unwrap_or_default(),
                    squad_tone: match contract.map(|c| &c.squad_status) {
                        Some(
                            PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular,
                        ) => "regular",
                        Some(PlayerSquadStatus::FirstTeamSquadRotation) => "rotation",
                        _ => "backup",
                    },
                    contract_until: contract
                        .map(|c| i18n.format_month_year(c.expiration))
                        .unwrap_or_default(),
                    wage: contract
                        .map(|c| FormattingUtils::format_money(c.salary as f64))
                        .unwrap_or_default(),
                    value: FormattingUtils::format_money(value),
                }
            })
            .collect();
        (rows, TransferLedger::money(total))
    }
}

pub struct TransferHistoryRow {
    pub player: TransferPlayerView,
    pub club: TransferClubView,
    pub date: String,
    pub fee: String,
    pub free: bool,
}

pub struct LoanHistoryRow {
    pub player: TransferPlayerView,
    pub club: TransferClubView,
    pub start: String,
    pub end: String,
    pub progress: u8,
}

/// One season of a club's completed moves, newest first.
pub struct TransferLedger {
    pub incoming_transfers: Vec<TransferHistoryRow>,
    pub outgoing_transfers: Vec<TransferHistoryRow>,
    pub incoming_loans: Vec<LoanHistoryRow>,
    pub outgoing_loans: Vec<LoanHistoryRow>,
    pub spent: String,
    pub received: String,
}

impl TransferLedger {
    fn collect(
        data: &SimulatorData,
        club_id: u32,
        season: u16,
        now: NaiveDate,
        i18n: &I18n,
    ) -> Self {
        // A cross-border move is recorded only in the buying country's
        // market, so every country is scanned or foreign sales go missing.
        let mut records: Vec<&CompletedTransfer> = data
            .continents
            .iter()
            .flat_map(|continent| continent.countries.iter())
            .flat_map(|country| country.transfer_market.transfer_history.iter())
            .filter(|t| t.season_year == season)
            .filter(|t| t.to_club_id == club_id || t.from_club_id == club_id)
            .collect();
        records.sort_by(|a, b| b.transfer_date.cmp(&a.transfer_date));

        let mut ledger = TransferLedger {
            incoming_transfers: Vec::new(),
            outgoing_transfers: Vec::new(),
            incoming_loans: Vec::new(),
            outgoing_loans: Vec::new(),
            spent: String::new(),
            received: String::new(),
        };
        let (mut spent, mut received) = (0.0, 0.0);

        for t in records {
            let incoming = t.to_club_id == club_id;
            let (other_id, other_name) = if incoming {
                (t.from_club_id, &t.from_team_name)
            } else {
                (t.to_club_id, &t.to_team_name)
            };
            let player = TransferPlayerView::of_record(data, t.player_id, &t.player_name, now);
            let club = TransferClubView::of(data, other_id, other_name, i18n);

            match &t.transfer_type {
                TransferType::Loan(end) => {
                    let row = LoanHistoryRow {
                        player,
                        club,
                        start: i18n.format_date(t.transfer_date),
                        end: i18n.format_date(*end),
                        progress: Self::progress(t.transfer_date, *end, now),
                    };
                    if incoming {
                        ledger.incoming_loans.push(row);
                    } else {
                        ledger.outgoing_loans.push(row);
                    }
                }
                TransferType::Permanent | TransferType::Free => {
                    let fee = t.fee.amount;
                    let row = TransferHistoryRow {
                        player,
                        club,
                        date: i18n.format_date(t.transfer_date),
                        fee: if fee > 0.0 {
                            FormattingUtils::format_money(fee)
                        } else {
                            i18n.t("fee_free").to_string()
                        },
                        free: fee <= 0.0,
                    };
                    if incoming {
                        spent += fee;
                        ledger.incoming_transfers.push(row);
                    } else {
                        received += fee;
                        ledger.outgoing_transfers.push(row);
                    }
                }
            }
        }

        ledger.spent = Self::money(spent);
        ledger.received = Self::money(received);
        ledger
    }

    fn progress(start: NaiveDate, end: NaiveDate, now: NaiveDate) -> u8 {
        let length = (end - start).num_days();
        if length <= 0 {
            return 100;
        }
        let elapsed = (now - start).num_days().clamp(0, length);
        (elapsed * 100 / length) as u8
    }

    fn money(amount: f64) -> String {
        if amount > 0.0 {
            FormattingUtils::format_money(amount)
        } else {
            "0".to_string()
        }
    }
}

pub struct TeamTransfersPage;

impl TeamTransfersPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<TeamTransfersRequest>,
        Query(query): Query<SeasonQuery>,
    ) -> ApiResult<impl IntoResponse> {
        let guard = state.data.read().await;

        let simulator_data = guard
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let i18n = state.i18n.for_lang(&route_params.lang);
        let team = TeamLookup::by_slug(simulator_data, &route_params.team_slug)?;

        Self::template(simulator_data, team, i18n, &route_params, &query)
    }

    fn template(
        data: &SimulatorData,
        team: &Team,
        i18n: I18n,
        route_params: &TeamTransfersRequest,
        query: &SeasonQuery,
    ) -> Result<TeamTransfersTemplate, ApiError> {
        let league = team.league_id.and_then(|id| data.league(id));
        let now = data.date.date();
        let club_id = team.club_id;

        let current_season_year = if now.month() >= 8 {
            now.year() as u16
        } else {
            (now.year() - 1) as u16
        };

        let selected_season = query.season.unwrap_or(current_season_year);
        let seasons = SeasonOption::list(data, club_id, current_season_year, selected_season);

        let (items, list_value) = TransferListRow::collect(data, team, now, &i18n);
        let ledger = TransferLedger::collect(data, club_id, selected_season, now, &i18n);

        let current_path = format!("/{}/teams/{}/transfers", route_params.lang, team.slug);
        let neighborhood = TeamNeighborhood::for_club(team.club_id, data, &i18n)?;
        let menu_sections = neighborhood.menu_sections(
            data,
            team.club_id,
            &i18n,
            &route_params.lang,
            &current_path,
        );
        let title = team.name.clone();
        let league_title = league
            .map(|l| views::Menus::league_display_name(l, &i18n, data))
            .unwrap_or_default();

        Ok(TeamTransfersTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            i18n,
            lang: route_params.lang.clone(),
            title,
            sub_title_prefix: String::new(),
            sub_title_suffix: String::new(),
            sub_title: league_title,
            sub_title_link: league
                .map(|l| LeagueAddress::of(data, l).url(&route_params.lang))
                .unwrap_or_default(),
            sub_title_flag: None,
            header_color: data
                .club(team.club_id)
                .map(|c| c.colors.background.clone())
                .unwrap_or_default(),
            foreground_color: data
                .club(team.club_id)
                .map(|c| c.colors.foreground.clone())
                .unwrap_or_default(),
            menu_sections,
            team_slug: team.slug.clone(),
            active_tab: "transfers",
            show_finances_tab: team.team_type.is_own_team(),
            show_academy_tab: team.team_type == TeamType::Main || team.team_type == TeamType::U18,
            newspaper_count: NewspaperCounter::count(data, team),
            items,
            list_value,
            ledger,
            seasons,
        })
    }
}

impl SeasonOption {
    fn list(
        data: &SimulatorData,
        club_id: u32,
        current_season_year: u16,
        selected_season: u16,
    ) -> Vec<SeasonOption> {
        let min_season_year = data
            .continents
            .iter()
            .flat_map(|cont| cont.countries.iter())
            .flat_map(|c| c.transfer_market.transfer_history.iter())
            .filter(|t| t.from_club_id == club_id || t.to_club_id == club_id)
            .map(|t| t.season_year)
            .min()
            .unwrap_or(current_season_year);

        (min_season_year..=current_season_year)
            .rev()
            .map(|y| SeasonOption {
                year: y,
                display: format!("{}/{}", y, (y + 1) % 100),
                selected: y == selected_season,
            })
            .collect()
    }
}
