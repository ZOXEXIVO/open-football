use crate::league::League;

/// A country's divisions read as a pyramid: which league each one drops
/// into, and how many sides really cross each boundary at season end.
///
/// The rungs are `(tier, group level)` steps. Regional groups share their
/// tier's rung; a ranked group (Russian Division A Silver below Gold) is a
/// rung of its own inside the tier.
pub struct LeagueLadder<'a> {
    leagues: &'a [League],
}

impl<'a> LeagueLadder<'a> {
    pub fn new(leagues: &'a [League]) -> Self {
        LeagueLadder { leagues }
    }

    /// `league`'s rung: its tier and its group's level inside the tier.
    fn step(league: &League) -> (u8, u8) {
        let level = league.settings.league_group.as_ref().map_or(0, |g| g.level);
        (league.settings.tier, level)
    }

    /// The rung directly below `league`: the next ranked group of its own
    /// tier when there is one, otherwise the top of the next tier.
    fn step_below(&self, league: &League) -> (u8, u8) {
        let (tier, level) = Self::step(league);
        level
            .checked_add(1)
            .map(|next| (tier, next))
            .filter(|&next| self.leagues.iter().any(|l| Self::step(l) == next))
            .unwrap_or((tier + 1, 0))
    }

    /// Ids of the parallel groups `league` shares its rung with in its
    /// competition, itself included, in id order. Empty when ungrouped.
    fn zone_ids(&self, league: &League) -> Vec<u32> {
        let Some(group) = league.settings.league_group.as_ref() else {
            return Vec::new();
        };
        let step = Self::step(league);
        let mut zones: Vec<u32> = self
            .leagues
            .iter()
            .filter(|l| {
                Self::step(l) == step
                    && l.settings
                        .league_group
                        .as_ref()
                        .is_some_and(|g| g.competition == group.competition)
            })
            .map(|l| l.id)
            .collect();
        zones.sort_unstable();
        zones
    }

    /// The league one rung down that `league_id` relegates into.
    ///
    /// When the relegating rung and the rung below are BOTH split into
    /// groups, zones pair to groups by position (zone 0 → group 0, zone 1
    /// → group 1), so each zone relegates into a distinct group instead of
    /// every zone piling into the first one.
    pub fn lower_partner(&self, league_id: u32) -> Option<&'a League> {
        let league = self.leagues.iter().find(|l| l.id == league_id)?;
        let below = self.step_below(league);
        let mut candidates: Vec<&League> = self
            .leagues
            .iter()
            .filter(|l| {
                l.id != league_id && Self::step(l) == below && l.settings.promotion_spots > 0
            })
            .collect();
        if candidates.is_empty() {
            return None;
        }

        let zones = self.zone_ids(league);
        if !zones.is_empty() {
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
    /// A division alone on its rung above a rung split into groups of one
    /// competition (Segunda above Primera Federación's two groups, Division
    /// A Silver above Division B's zones) feeds EVERY group. Places are
    /// handed out champions-first — one to each group in turn, then each
    /// group's runner-up, and so on — until the division above has no
    /// relegation places left, so no group is starved while another
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

    /// The groups `league` relegates into, ordered by id — empty unless
    /// `league` has no parallel zone on its rung, every promoting league
    /// one rung down is a group of the same competition and there are at
    /// least two of them.
    fn lower_groups(&self, league: &League) -> Vec<&'a League> {
        if self.zone_ids(league).len() > 1 {
            return Vec::new();
        }
        let below = self.step_below(league);
        let mut lower: Vec<&League> = self
            .leagues
            .iter()
            .filter(|l| {
                l.id != league.id && Self::step(l) == below && l.settings.promotion_spots > 0
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
            .filter(|upper| self.step_below(upper) == Self::step(league))
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
