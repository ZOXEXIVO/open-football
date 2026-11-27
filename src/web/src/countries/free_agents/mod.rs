pub mod routes;

use crate::countries::{CountryLookup, CountryMenu};
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::shared::potential_stars::{PotentialStarsView, StarRating};
use crate::views::{MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::PlayerPositionType;
use core::utils::DateUtils;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CountryFreeAgentsRequest {
    lang: String,
    country_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "countries/free_agents/index.html")]
pub struct CountryFreeAgentsTemplate {
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
    pub active_tab: &'static str,
    pub country_slug: String,
    pub team_suffix: &'static str,
    pub players: Vec<FreeAgentPlayerDto>,
}

pub struct FreeAgentPlayerDto {
    pub slug: String,
    pub first_name: String,
    pub last_name: String,
    pub position: String,
    pub age: u8,
    pub current_ability: StarRating,
    pub potential_ability: StarRating,
    /// Primary position used to bucket players in the squad-style sort
    /// (GK → DF → MF → FW). Not rendered — purely a sort key.
    pub position_sort: PlayerPositionType,
    /// Plain-language reason he is still unsigned, from the player's own
    /// durable market state (no world scan). Empty only if state is
    /// somehow missing.
    pub status_message: String,
}

impl FreeAgentPlayerDto {
    fn of(player: &core::Player, now: chrono::NaiveDate) -> Self {
        // Cheap, state-only "why is he still free?" explanation —
        // no world scan, so it's safe to compute per row in a list.
        let status_message = player
            .market_explanation(now)
            .map(|e| e.message)
            .unwrap_or_default();

        Self {
            slug: player.slug(),
            first_name: player.full_name.display_first_name().to_string(),
            last_name: player.full_name.display_last_name().to_string(),
            position: player.positions.display_positions_compact(),
            age: DateUtils::age(player.birth_date, now),
            current_ability: PotentialStarsView::current(player),
            potential_ability: PotentialStarsView::potential_absolute(player, now),
            position_sort: player.position(),
            status_message,
        }
    }
}

pub struct CountryFreeAgentsPage;

impl CountryFreeAgentsPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<CountryFreeAgentsRequest>,
    ) -> ApiResult<impl IntoResponse> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let world = state.world.snapshot();

        let simulator_data = world
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let (country, continent) =
            CountryLookup::by_slug(simulator_data, &route_params.country_slug)?;

        Ok(Self::template(
            simulator_data,
            country,
            continent,
            i18n,
            &route_params,
        ))
    }

    fn template(
        data: &core::SimulatorData,
        country: &core::Country,
        continent: &core::continent::Continent,
        i18n: I18n,
        route_params: &CountryFreeAgentsRequest,
    ) -> CountryFreeAgentsTemplate {
        let now = data.date.date();
        let mut players: Vec<FreeAgentPlayerDto> = data
            .free_agents
            .iter()
            .filter(|player| player.country_id == country.id)
            .map(|player| FreeAgentPlayerDto::of(player, now))
            .collect();

        players.sort_by(|a, b| {
            a.position_sort
                .partial_cmp(&b.position_sort)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.current_ability.cmp(&a.current_ability))
        });

        let current_path = format!(
            "/{}/countries/{}/free-agents",
            route_params.lang, route_params.country_slug
        );

        CountryFreeAgentsTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title: country.name.clone(),
            sub_title_prefix: String::new(),
            sub_title_suffix: String::new(),
            sub_title: continent.name.clone(),
            sub_title_link: format!("/{}/countries", route_params.lang),
            sub_title_flag: None,
            header_color: country.background_color.clone(),
            foreground_color: country.foreground_color.clone(),
            menu_sections: CountryMenu::sections(
                country,
                &i18n,
                &route_params.lang,
                &route_params.country_slug,
                &current_path,
            ),
            lang: route_params.lang.clone(),
            i18n,
            active_tab: "free_agents",
            country_slug: route_params.country_slug.clone(),
            team_suffix: "",
            players,
        }
    }
}
