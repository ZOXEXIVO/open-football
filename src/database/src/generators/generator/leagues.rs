use crate::{CountryEntity, DatabaseEntity};
use core::league::{
    DayMonthPeriod, DomesticCup, League, LeagueFinancials, LeagueGroup, LeagueSettings,
};
use core::{Club, TeamType};
use std::str::FromStr;

use super::DatabaseGenerator;

/// Base for generated domestic-cup league ids: `BASE + country_id`. Country
/// ids top out in the low thousands, so the cup band is `800_000_005 ..=
/// 800_002_000` — clear of every real league id (which are either small or
/// in the ~2.0e9 Russian range) and of the continental cups (900_000_001+).
const DOMESTIC_CUP_ID_BASE: u32 = 800_000_000;

/// Lowercase, hyphenate, and strip non-alphanumerics from a string so a
/// country slug like "czech republic" yields a URL-safe "czech-republic".
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

impl DatabaseGenerator {
    pub(super) fn generate_leagues(
        country_id: u32,
        country_reputation: u16,
        data: &DatabaseEntity,
    ) -> Vec<League> {
        data.leagues
            .iter()
            .filter(|l| l.country_id == country_id)
            .map(|league| {
                let financials = LeagueFinancials::from_reputation_and_tier(
                    league.reputation,
                    league.tier,
                    country_reputation,
                );
                let settings = LeagueSettings {
                    season_starting_half: DayMonthPeriod {
                        from_day: league.settings.season_starting_half.from_day,
                        from_month: league.settings.season_starting_half.from_month,
                        to_day: league.settings.season_starting_half.to_day,
                        to_month: league.settings.season_starting_half.to_month,
                    },
                    season_ending_half: DayMonthPeriod {
                        from_day: league.settings.season_ending_half.from_day,
                        from_month: league.settings.season_ending_half.from_month,
                        to_day: league.settings.season_ending_half.to_day,
                        to_month: league.settings.season_ending_half.to_month,
                    },
                    tier: league.tier,
                    promotion_spots: league.promotion_spots,
                    relegation_spots: league.relegation_spots,
                    league_group: league.league_group.as_ref().map(|g| LeagueGroup {
                        name: g.name.clone(),
                        competition: g.competition.clone(),
                        total_groups: g.total_groups,
                    }),
                };

                let mut l = League::new(
                    league.id,
                    league.name.clone(),
                    league.slug.clone(),
                    league.country_id,
                    league.reputation,
                    settings,
                    false,
                );
                l.financials = financials;
                l
            })
            .collect()
    }

    /// Build the country's single domestic cup as a `DomesticCup` (a
    /// `League` flagged `is_cup = true`, `friendly = false`). Mirrors the
    /// primary (tier-1, else first) league's season window so the cup runs
    /// across the same calendar. Returns `None` only when the country has
    /// no leagues to feed participants.
    ///
    /// Uses the named cup from the data when configured for this country
    /// (FA Cup, Copa del Rey, …); otherwise falls back to "{Country} Cup".
    pub(super) fn generate_domestic_cup(
        country: &CountryEntity,
        leagues: &[League],
    ) -> Option<DomesticCup> {
        let primary = leagues
            .iter()
            .find(|l| l.settings.tier == 1)
            .or_else(|| leagues.first())?;

        let settings = LeagueSettings {
            season_starting_half: primary.settings.season_starting_half,
            season_ending_half: primary.settings.season_ending_half,
            tier: 0,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
        };

        let (slug, name, configured_rep) = match &country.domestic_cup {
            Some(cfg) => (cfg.slug.clone(), cfg.name.clone(), cfg.reputation),
            None => (
                format!("{}-cup", slugify(&country.slug)),
                format!("{} Cup", country.name),
                None,
            ),
        };
        let reputation = configured_rep.unwrap_or(primary.reputation);

        let mut league = League::new(
            DOMESTIC_CUP_ID_BASE + country.id,
            name,
            slug,
            country.id,
            reputation,
            settings,
            false,
        );
        league.is_cup = true;

        Some(DomesticCup::new(league))
    }

    pub(super) fn create_subteams_leagues(
        country_id: u32,
        clubs: &mut [Club],
        leagues: &mut Vec<League>,
        data: &DatabaseEntity,
    ) {
        // Build a map: club_id → parent league_id (from the club's Main team)
        let club_league_map: Vec<(u32, u32)> = clubs
            .iter()
            .filter_map(|club| {
                let main_league_id = club
                    .teams
                    .teams
                    .iter()
                    .find(|t| t.team_type == TeamType::Main)
                    .and_then(|t| t.league_id)?;
                Some((club.id, main_league_id))
            })
            .collect();

        // Snapshot parent leagues to create subleagues per configured team type
        let parent_leagues: Vec<(u32, String, String, u16, LeagueSettings)> = leagues
            .iter()
            .map(|l| {
                (
                    l.id,
                    l.name.clone(),
                    l.slug.clone(),
                    l.reputation,
                    l.settings.clone(),
                )
            })
            .collect();

        for (parent_id, parent_name, parent_slug, parent_rep, parent_settings) in &parent_leagues {
            // Find sub_leagues_competitions config from the league entity
            let team_types: Vec<TeamType> = data
                .leagues
                .iter()
                .find(|l| l.id == *parent_id)
                .map(|l| {
                    l.sub_leagues_competitions
                        .iter()
                        .filter_map(|s| TeamType::from_str(s).ok())
                        .collect()
                })
                .unwrap_or_default();

            for team_type in &team_types {
                // Check if any club in this parent league has this team type
                let has_type = clubs.iter().any(|club| {
                    club_league_map
                        .iter()
                        .any(|(cid, lid)| *cid == club.id && lid == parent_id)
                        && club.teams.teams.iter().any(|t| t.team_type == *team_type)
                });

                if !has_type {
                    continue;
                }

                // Deterministic league ID offset per team type
                let type_offset = match team_type {
                    TeamType::U18 => 100000,
                    TeamType::U19 => 110000,
                    TeamType::U20 => 120000,
                    TeamType::U21 => 130000,
                    TeamType::U23 => 140000,
                    _ => continue,
                };

                let youth_league_id = parent_id + type_offset;
                let youth_reputation = (parent_rep / 10).max(100);
                let type_label = format!("{}", team_type);
                let type_slug = type_label.to_lowercase();

                let youth_settings = LeagueSettings {
                    season_starting_half: parent_settings.season_starting_half,
                    season_ending_half: parent_settings.season_ending_half,
                    tier: 99,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                };

                let youth_league = League::new(
                    youth_league_id,
                    format!("{} {}", parent_name, type_label),
                    format!("{}-{}", parent_slug, type_slug),
                    country_id,
                    youth_reputation,
                    youth_settings,
                    true,
                );

                leagues.push(youth_league);

                // Assign matching teams to this youth league
                for club in clubs.iter_mut() {
                    let is_in_parent = club_league_map
                        .iter()
                        .any(|(cid, lid)| *cid == club.id && lid == parent_id);
                    if !is_in_parent {
                        continue;
                    }
                    for team in &mut club.teams.teams {
                        if team.team_type == *team_type {
                            team.league_id = Some(youth_league_id);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DOMESTIC_CUP_ID_BASE, DatabaseGenerator, slugify};
    use crate::DomesticCupEntity;
    use crate::loaders::country::{
        CountryEntity, CountryPricingEntity, CountrySettingsEntity, SkinColorsEntity,
    };
    use core::league::{DayMonthPeriod, League, LeagueSettings};

    fn country_entity(
        id: u32,
        name: &str,
        slug: &str,
        cup: Option<DomesticCupEntity>,
    ) -> CountryEntity {
        CountryEntity {
            id,
            code: "xx".into(),
            slug: slug.into(),
            name: name.into(),
            background_color: "#000000".into(),
            foreground_color: "#ffffff".into(),
            continent_id: 1,
            reputation: 5000,
            settings: CountrySettingsEntity {
                pricing: CountryPricingEntity { price_level: 1.0 },
            },
            skin_colors: SkinColorsEntity::default(),
            domestic_cup: cup,
        }
    }

    fn tier1_league(id: u32, country_id: u32) -> League {
        League::new(
            id,
            "Top Flight".into(),
            "top-flight".into(),
            country_id,
            8000,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 30, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier: 1,
                promotion_spots: 0,
                relegation_spots: 3,
                league_group: None,
            },
            false,
        )
    }

    #[test]
    fn slugify_handles_spaces_and_punctuation() {
        assert_eq!(slugify("czech republic"), "czech-republic");
        assert_eq!(slugify("United Arab Emirates"), "united-arab-emirates");
        assert_eq!(slugify("USA"), "usa");
    }

    #[test]
    fn configured_cup_uses_named_slug_and_competitive_flags() {
        let cfg = DomesticCupEntity {
            country_slug: "england".into(),
            slug: "fa-cup".into(),
            name: "FA Cup".into(),
            reputation: None,
        };
        let country = country_entity(765, "England", "england", Some(cfg));
        let cup =
            DatabaseGenerator::generate_domestic_cup(&country, &[tier1_league(1, 765)]).unwrap();

        assert!(cup.league.is_cup, "cup must be flagged is_cup");
        assert!(
            !cup.league.friendly,
            "cup must be competitive (not friendly)"
        );
        assert_eq!(cup.league.id, DOMESTIC_CUP_ID_BASE + 765);
        assert_eq!(cup.league.name, "FA Cup");
        assert_eq!(cup.league.slug, "fa-cup");
        assert_eq!(cup.league.settings.tier, 0);
        // Season window mirrors the tier-1 league.
        assert_eq!(cup.league.settings.season_starting_half.from_month, 8);
        assert_eq!(cup.league.settings.season_ending_half.to_month, 5);
    }

    #[test]
    fn unconfigured_cup_falls_back_to_country_name() {
        let country = country_entity(763, "Czech Republic", "czech republic", None);
        let cup =
            DatabaseGenerator::generate_domestic_cup(&country, &[tier1_league(1, 763)]).unwrap();
        assert_eq!(cup.league.name, "Czech Republic Cup");
        assert_eq!(cup.league.slug, "czech-republic-cup");
        assert!(cup.league.is_cup && !cup.league.friendly);
        assert_eq!(cup.league.id, DOMESTIC_CUP_ID_BASE + 763);
    }

    #[test]
    fn no_leagues_yields_no_cup() {
        let country = country_entity(1, "Nowhere", "nowhere", None);
        assert!(DatabaseGenerator::generate_domestic_cup(&country, &[]).is_none());
    }
}
