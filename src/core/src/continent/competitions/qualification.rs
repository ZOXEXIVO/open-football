use crate::Country;
use crate::continent::{CompetitionTier, Continent};
use crate::league::League;
use std::cmp::Reverse;

/// A run of final-table places that feeds one continental competition:
/// positions `skip + 1 ..= skip + take`.
#[derive(Debug, Clone, PartialEq)]
pub struct QualificationBand {
    pub tier: CompetitionTier,
    pub skip: usize,
    pub take: usize,
}

impl QualificationBand {
    pub fn contains(&self, index: usize) -> bool {
        index >= self.skip && index < self.skip + self.take
    }
}

/// Which domestic table places a country's top flight sends into the
/// continent's club competitions. The draw and the standings pages both
/// read it, so a legend can never promise a place the draw won't honour.
pub struct ContinentalQualification;

const UEFA_TIERS: [CompetitionTier; 3] = [
    CompetitionTier::ChampionsLeague,
    CompetitionTier::EuropaLeague,
    CompetitionTier::ConferenceLeague,
];
const CONMEBOL_TIERS: [CompetitionTier; 1] = [CompetitionTier::CopaLibertadores];

impl ContinentalQualification {
    pub fn tiers(continent: &Continent) -> &'static [CompetitionTier] {
        if continent.is_europe() {
            &UEFA_TIERS
        } else if continent.is_south_america() {
            &CONMEBOL_TIERS
        } else {
            &[]
        }
    }

    /// Countries ordered by reputation, strongest first — the coefficient
    /// ranking every allocation below is keyed on.
    pub fn ranked_countries(continent: &Continent) -> Vec<&Country> {
        let mut countries: Vec<&Country> = continent.countries.iter().collect();
        countries.sort_by_key(|c| Reverse(c.reputation));
        countries
    }

    pub fn qualifying_league(country: &Country) -> Option<&League> {
        country
            .leagues
            .leagues
            .iter()
            .find(|l| l.settings.tier == 1 && !l.friendly)
    }

    /// `(skip, take)` for a country at `rank` in the coefficient ranking.
    pub fn band(tier: &CompetitionTier, rank: usize) -> Option<(usize, usize)> {
        match tier {
            CompetitionTier::ChampionsLeague => Some(match rank {
                0..4 => (0, 4),
                4..6 => (0, 2),
                _ => (0, 1),
            }),
            CompetitionTier::EuropaLeague => match rank {
                0..4 => Some((4, 3)),
                4..8 => Some((2, 2)),
                8..20 => Some((1, 1)),
                _ => None,
            },
            CompetitionTier::ConferenceLeague => Some(match rank {
                0..4 => (7, 1),
                4..8 => (4, 2),
                8..20 => (2, 1),
                _ => (1, 1),
            }),
            CompetitionTier::CopaLibertadores => Some(match rank {
                0..2 => (0, 5),
                2..4 => (0, 4),
                4..8 => (0, 3),
                _ => (0, 1),
            }),
        }
    }

    /// Group-stage size the allocation is trimmed to, when the competition has one.
    pub fn field_cap(tier: &CompetitionTier) -> Option<usize> {
        match tier {
            CompetitionTier::CopaLibertadores => Some(32),
            _ => None,
        }
    }

    /// Every band the given league's table carries. Empty unless the
    /// league is its country's qualifying top flight on a continent that
    /// runs club competitions.
    pub fn bands_for_league(continent: &Continent, league: &League) -> Vec<QualificationBand> {
        let ranked = Self::ranked_countries(continent);
        let Some((rank, country)) = ranked
            .iter()
            .enumerate()
            .find(|(_, c)| c.id == league.country_id)
        else {
            return Vec::new();
        };
        if Self::qualifying_league(country).map(|l| l.id) != Some(league.id) {
            return Vec::new();
        }

        Self::tiers(continent)
            .iter()
            .filter_map(|tier| {
                Self::band(tier, rank).map(|(skip, take)| QualificationBand {
                    tier: tier.clone(),
                    skip,
                    take,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copa_spots_follow_canonical_5_5_4_4_3x4_then_1() {
        let expected: Vec<usize> = vec![5, 5, 4, 4, 3, 3, 3, 3, 1, 1, 1];
        let actual: Vec<usize> = (0..expected.len())
            .map(|rank| {
                ContinentalQualification::band(&CompetitionTier::CopaLibertadores, rank)
                    .unwrap()
                    .1
            })
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn copa_spot_allocation_fills_the_group_stage_with_ten_nations() {
        let total: usize = (0..10)
            .map(|rank| {
                ContinentalQualification::band(&CompetitionTier::CopaLibertadores, rank)
                    .unwrap()
                    .1
            })
            .sum();
        assert_eq!(
            Some(total),
            ContinentalQualification::field_cap(&CompetitionTier::CopaLibertadores)
        );
    }

    #[test]
    fn europa_league_skips_countries_below_twentieth() {
        assert!(ContinentalQualification::band(&CompetitionTier::EuropaLeague, 20).is_none());
        assert_eq!(
            ContinentalQualification::band(&CompetitionTier::ConferenceLeague, 20),
            Some((1, 1))
        );
    }
}
