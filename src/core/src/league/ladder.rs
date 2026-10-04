use crate::league::League;

/// A country's divisions read as a pyramid: which league each one drops
/// into, and how many sides really cross each boundary at season end.
pub struct LeagueLadder<'a> {
    leagues: &'a [League],
}

impl<'a> LeagueLadder<'a> {
    pub fn new(leagues: &'a [League]) -> Self {
        LeagueLadder { leagues }
    }

    /// The tier-(T+1) league that `league_id` relegates into.
    ///
    /// When the relegating tier and the tier below are BOTH split into
    /// groups of the same competition, zones pair to groups by position
    /// (zone 0 → group 0, zone 1 → group 1), so each zone relegates into a
    /// distinct group instead of every zone piling into the first one.
    pub fn lower_partner(&self, league_id: u32) -> Option<&'a League> {
        let league = self.leagues.iter().find(|l| l.id == league_id)?;
        let tier = league.settings.tier;
        let lower_tier = tier + 1;
        let mut candidates: Vec<&League> = self
            .leagues
            .iter()
            .filter(|l| {
                l.id != league_id && l.settings.tier == lower_tier && l.settings.promotion_spots > 0
            })
            .collect();
        if candidates.is_empty() {
            return None;
        }

        if let Some(group) = league.settings.league_group.as_ref() {
            let mut zones: Vec<u32> = self
                .leagues
                .iter()
                .filter(|l| {
                    l.settings.tier == tier
                        && l.settings
                            .league_group
                            .as_ref()
                            .is_some_and(|g| g.competition == group.competition)
                })
                .map(|l| l.id)
                .collect();
            zones.sort_unstable();

            let mut grouped: Vec<&League> = candidates
                .iter()
                .copied()
                .filter(|l| l.settings.league_group.is_some())
                .collect();
            grouped.sort_unstable_by_key(|l| l.id);

            if let Some(pos) = zones.iter().position(|&id| id == league_id)
                && let Some(l) = grouped.get(pos)
            {
                return Some(l);
            }
        }

        Some(candidates.remove(0))
    }

    /// Where the sides relegated from `league`'s table go: `(lower league,
    /// sides)` per boundary, in league-id order.
    ///
    /// A single division above a tier split into groups of one competition
    /// (Segunda above Primera Federación's two groups) feeds EVERY group.
    /// Places are handed out champions-first — one to each group in turn,
    /// then each group's runner-up, and so on — until the division above
    /// has no relegation places left, so no group is starved while another
    /// promotes its third. Every other boundary is the lone `lower_partner`.
    pub fn relegation_split(&self, league: &League) -> Vec<(&'a League, usize)> {
        if league.settings.tier == 0 || league.settings.relegation_spots == 0 {
            return Vec::new();
        }
        let relegation_spots = league.settings.relegation_spots as usize;

        let groups = self.lower_groups(league);
        if groups.is_empty() {
            return self
                .lower_partner(league.id)
                .map(|lower| {
                    let places = relegation_spots.min(lower.settings.promotion_spots as usize);
                    vec![(lower, places)]
                })
                .unwrap_or_default();
        }

        let mut places = vec![0usize; groups.len()];
        let mut left = relegation_spots;
        let deepest = groups
            .iter()
            .map(|g| g.settings.promotion_spots as usize)
            .max()
            .unwrap_or(0);
        for rank in 0..deepest {
            for (i, group) in groups.iter().enumerate() {
                if left > 0 && (group.settings.promotion_spots as usize) > rank {
                    places[i] += 1;
                    left -= 1;
                }
            }
        }
        groups
            .into_iter()
            .zip(places)
            .filter(|&(_, n)| n > 0)
            .collect()
    }

    /// The groups an ungrouped `league` relegates into, ordered by id —
    /// empty unless every promoting league one tier down is a group of the
    /// same competition and there are at least two of them.
    fn lower_groups(&self, league: &League) -> Vec<&'a League> {
        if league.settings.league_group.is_some() {
            return Vec::new();
        }
        let mut lower: Vec<&League> = self
            .leagues
            .iter()
            .filter(|l| {
                l.id != league.id
                    && l.settings.tier == league.settings.tier + 1
                    && l.settings.promotion_spots > 0
            })
            .collect();
        let Some(competition) = lower
            .first()
            .and_then(|l| l.settings.league_group.as_ref())
            .map(|g| g.competition.as_str())
        else {
            return Vec::new();
        };
        let one_competition = lower.iter().all(|l| {
            l.settings
                .league_group
                .as_ref()
                .is_some_and(|g| g.competition == competition)
        });
        if lower.len() < 2 || !one_competition {
            return Vec::new();
        }
        lower.sort_unstable_by_key(|l| l.id);
        lower
    }

    /// Sides that drop out of this league's own table. Split-season
    /// leagues relegate off the annual aggregate, never a tournament table.
    pub fn relegated_from_table(&self, league: &League) -> usize {
        if league.settings.split_season {
            return 0;
        }
        self.swap_count(league)
    }

    /// Sides that drop out of a split-season competition's annual table.
    pub fn relegated_from_annual(&self, league: &League) -> usize {
        if !league.settings.split_season {
            return 0;
        }
        let zones = self.split_zones(league);
        if zones.len() < 2 {
            return self.swap_count(league);
        }
        if zones.iter().any(|z| self.lower_partner(z.id).is_none()) {
            return 0;
        }
        let spots: usize = zones
            .iter()
            .map(|z| z.settings.relegation_spots as usize)
            .sum();
        spots.min(zones.len())
    }

    /// Sides this league sends up — each boundary moves as many as the
    /// league above drops into it.
    pub fn promoted_from_table(&self, league: &League) -> usize {
        if league.settings.promotion_spots == 0 {
            return 0;
        }
        self.leagues
            .iter()
            .filter(|upper| upper.settings.tier + 1 == league.settings.tier)
            .map(|upper| {
                let zones = self.split_zones(upper);
                if zones.len() < 2 {
                    return self
                        .relegation_split(upper)
                        .into_iter()
                        .find(|(lower, _)| lower.id == league.id)
                        .map_or(0, |(_, places)| places);
                }
                if !self
                    .lower_partner(upper.id)
                    .is_some_and(|l| l.id == league.id)
                {
                    return 0;
                }
                // A split competition's k-th relegated side is replaced by
                // the k-th paired group's champion.
                let position = zones.iter().position(|z| z.id == upper.id).unwrap_or(0);
                usize::from(position < self.relegated_from_annual(upper))
            })
            .sum()
    }

    fn swap_count(&self, league: &League) -> usize {
        self.relegation_split(league)
            .iter()
            .map(|&(_, places)| places)
            .sum()
    }

    /// Zones of the split-season grouped competition `league` belongs to,
    /// ordered by id.
    fn split_zones(&self, league: &League) -> Vec<&'a League> {
        let Some(group) = league.settings.league_group.as_ref() else {
            return Vec::new();
        };
        if !league.settings.split_season || league.settings.relegation_spots == 0 {
            return Vec::new();
        }
        let mut zones: Vec<&League> = self
            .leagues
            .iter()
            .filter(|l| {
                l.settings.split_season
                    && l.settings.relegation_spots > 0
                    && l.settings
                        .league_group
                        .as_ref()
                        .is_some_and(|g| g.competition == group.competition)
            })
            .collect();
        zones.sort_by_key(|l| l.id);
        zones
    }
}
