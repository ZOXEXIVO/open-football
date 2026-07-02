pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::PerfCounters;
use serde::Deserialize;

pub fn performance_routes() -> axum::Router<GameAppData> {
    routes::routes()
}

#[derive(Deserialize)]
pub struct PerformancePageRequest {
    pub lang: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "performance/index.html")]
pub struct PerformancePageTemplate {
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
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,

    pub view: PerfView,
}

pub struct PerfView {
    pub has_run: bool,
    pub full_process_ms: String,
    pub full_process_pill: &'static str,
    pub match_day_ms: String,
    pub match_day_pill: &'static str,
    pub non_match_day_ms: String,
    pub non_match_day_pill: &'static str,
    pub tick_avg_ms: String,
    pub tick_avg_pill: &'static str,

    pub callups_ms: String,
    pub callups_pct: u32,
    pub callups_pill: &'static str,
    pub world_matches_ms: String,
    pub world_matches_pct: u32,
    pub world_matches_pill: &'static str,
    pub continents_ms: String,
    pub continents_pct: u32,
    pub continents_pill: &'static str,
    pub result_proc_ms: String,
    pub result_proc_pct: u32,
    pub result_proc_pill: &'static str,
    pub manager_market_ms: String,
    pub manager_market_pct: u32,
    pub manager_market_pill: &'static str,
    pub global_comp_ms: String,
    pub global_comp_pct: u32,
    pub global_comp_pill: &'static str,
    pub cleanup_ms: String,
    pub cleanup_pct: u32,
    pub cleanup_pill: &'static str,
    pub awards_ms: String,
    pub awards_pct: u32,
    pub awards_pill: &'static str,
    pub match_storage_ms: String,
    pub match_storage_pct: u32,
    pub match_storage_pill: &'static str,

    pub match_sim_avg_ms: String,
    pub match_result_proc_ms: String,
    pub match_pool_workers: usize,

    pub simulated_days_total: String,
    pub matches_simulated: String,
    pub countries_processed: String,
    pub leagues_processed: String,
    pub clubs_processed: String,
    pub players_touched: String,
    pub match_results_written: String,
    pub panicked_continents: String,
    pub dirty_index_rebuild: bool,
    pub recording_mode: bool,
}

fn pill_for(ms: f64, watch_threshold: f64, slow_threshold: f64) -> &'static str {
    if ms >= slow_threshold {
        "slow"
    } else if ms >= watch_threshold {
        "watch"
    } else {
        "good"
    }
}

fn fmt_ms(ns: u64) -> String {
    let ms = ns as f64 / 1_000_000.0;
    if ms < 1.0 {
        format!("{:.3}", ms)
    } else if ms < 10.0 {
        format!("{:.2}", ms)
    } else {
        format!("{:.1}", ms)
    }
}

fn fmt_us(ns: u64) -> String {
    let ms = ns as f64 / 1_000_000.0;
    format!("{:.3}", ms)
}

fn fmt_count(n: u64) -> String {
    let raw = n.to_string();
    let bytes = raw.as_bytes();
    let len = bytes.len();
    if len <= 3 {
        return raw;
    }
    let mut out = String::with_capacity(len + (len - 1) / 3);
    let head = len % 3;
    if head != 0 {
        out.push_str(&raw[..head]);
    }
    let mut idx = head;
    while idx < len {
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(&raw[idx..idx + 3]);
        idx += 3;
    }
    out
}

fn pct_of(part_ns: u64, total_ns: u64) -> u32 {
    if total_ns == 0 {
        return 0;
    }
    let p = (part_ns as f64 / total_ns as f64) * 100.0;
    p.round().clamp(0.0, 100.0) as u32
}

impl PerfView {
    fn from_snapshot(snap: core::PerfSnapshot, pool_workers: usize) -> Self {
        let total = snap.total_ns.max(1);
        let full_ms = snap.total_ns as f64 / 1_000_000.0;
        let match_day_ms_v = snap.match_day_ns as f64 / 1_000_000.0;
        let non_match_ms_v = snap.non_match_day_ns as f64 / 1_000_000.0;
        let tick_avg_ms_v = snap.match_tick_avg_ns as f64 / 1_000_000.0;

        PerfView {
            has_run: snap.has_run,
            full_process_ms: fmt_ms(snap.total_ns),
            full_process_pill: pill_for(full_ms, 80.0, 200.0),
            match_day_ms: fmt_ms(snap.match_day_ns),
            match_day_pill: pill_for(match_day_ms_v, 80.0, 200.0),
            non_match_day_ms: fmt_ms(snap.non_match_day_ns),
            non_match_day_pill: pill_for(non_match_ms_v, 30.0, 80.0),
            tick_avg_ms: fmt_us(snap.match_tick_avg_ns),
            tick_avg_pill: pill_for(tick_avg_ms_v, 0.05, 0.2),

            callups_ms: fmt_ms(snap.callups_ns),
            callups_pct: pct_of(snap.callups_ns, total),
            callups_pill: pill_for(snap.callups_ns as f64 / 1_000_000.0, 30.0, 80.0),
            world_matches_ms: fmt_ms(snap.world_matches_ns),
            world_matches_pct: pct_of(snap.world_matches_ns, total),
            world_matches_pill: pill_for(snap.world_matches_ns as f64 / 1_000_000.0, 30.0, 80.0),
            continents_ms: fmt_ms(snap.continents_ns),
            continents_pct: pct_of(snap.continents_ns, total),
            continents_pill: pill_for(snap.continents_ns as f64 / 1_000_000.0, 50.0, 150.0),
            result_proc_ms: fmt_ms(snap.result_proc_ns),
            result_proc_pct: pct_of(snap.result_proc_ns, total),
            result_proc_pill: pill_for(snap.result_proc_ns as f64 / 1_000_000.0, 30.0, 80.0),
            manager_market_ms: fmt_ms(snap.manager_market_ns),
            manager_market_pct: pct_of(snap.manager_market_ns, total),
            manager_market_pill: pill_for(snap.manager_market_ns as f64 / 1_000_000.0, 20.0, 60.0),
            global_comp_ms: fmt_ms(snap.global_comp_ns),
            global_comp_pct: pct_of(snap.global_comp_ns, total),
            global_comp_pill: pill_for(snap.global_comp_ns as f64 / 1_000_000.0, 20.0, 60.0),
            cleanup_ms: fmt_ms(snap.cleanup_ns),
            cleanup_pct: pct_of(snap.cleanup_ns, total),
            cleanup_pill: pill_for(snap.cleanup_ns as f64 / 1_000_000.0, 20.0, 60.0),
            awards_ms: fmt_ms(snap.awards_ns),
            awards_pct: pct_of(snap.awards_ns, total),
            awards_pill: pill_for(snap.awards_ns as f64 / 1_000_000.0, 30.0, 100.0),
            match_storage_ms: fmt_ms(snap.match_storage_ns),
            match_storage_pct: pct_of(snap.match_storage_ns, total),
            match_storage_pill: pill_for(snap.match_storage_ns as f64 / 1_000_000.0, 30.0, 100.0),

            match_sim_avg_ms: fmt_ms(snap.match_sim_avg_ns),
            match_result_proc_ms: fmt_ms(snap.match_result_proc_ns),
            match_pool_workers: pool_workers,

            simulated_days_total: fmt_count(snap.simulated_days_total),
            matches_simulated: fmt_count(snap.match_count),
            countries_processed: fmt_count(snap.countries_processed),
            leagues_processed: fmt_count(snap.leagues_processed),
            clubs_processed: fmt_count(snap.clubs_processed),
            players_touched: fmt_count(snap.players_touched),
            match_results_written: fmt_count(snap.match_results_written),
            panicked_continents: fmt_count(snap.panicked_continents),
            dirty_index_rebuild: snap.dirty_index_rebuild,
            recording_mode: snap.recording_mode,
        }
    }
}

pub async fn performance_page_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PerformancePageRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let current_path = format!("/{}/performance", &route_params.lang);
    let menu_sections = views::search_menu(&i18n, &route_params.lang, &current_path);

    let snap = PerfCounters::instance().snapshot();
    let view = PerfView::from_snapshot(snap, *CPU_CORES);

    Ok(PerformancePageTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        i18n,
        lang: route_params.lang.clone(),
        title: "Performance".to_string(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: String::new(),
        sub_title_link: String::new(),
        sub_title_country_code: String::new(),
        header_color: String::new(),
        foreground_color: String::new(),
        menu_sections,
        view,
    })
}
