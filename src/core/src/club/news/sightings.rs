use chrono::{Duration, NaiveDate};
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// A scout's individual observation of a player who took the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoutVisit {
    pub date: NaiveDate,
    pub fixture_id: Arc<str>,
    pub team_id: u32,
    pub opponent_team_id: u32,
    pub club_id: u32,
    pub scout_id: u32,
    pub scout_name: Option<Arc<str>>,
    pub player_id: u32,
    /// Whether this visit qualifies for public coverage on its own.
    pub notable: bool,
}

#[derive(Debug, Clone)]
pub struct ScoutSighting {
    pub latest: ScoutVisit,
    pub other: Option<ScoutVisit>,
    pub returned: bool,
}

struct QualifyingVisit<'a> {
    visit: &'a ScoutVisit,
    returned: bool,
}

#[derive(Default)]
pub(crate) struct WeeklyScoutVisits<'a> {
    by_side: FxHashMap<u32, Vec<QualifyingVisit<'a>>>,
}

impl<'a> WeeklyScoutVisits<'a> {
    pub(crate) fn gather(log: &'a ScoutVisitLog, start: NaiveDate, end: NaiveDate) -> Self {
        let cutoff = start - Duration::days(ScoutVisitLog::RETURN_DAYS);
        let mut by_visitor: FxHashMap<_, Vec<_>> = FxHashMap::default();
        for visit in log
            .iter()
            .filter(|visit| (cutoff..end).contains(&visit.date))
        {
            by_visitor
                .entry((visit.player_id, visit.club_id))
                .or_default()
                .push(visit);
        }
        let mut week = Self::default();
        for visits in by_visitor.values() {
            for visit in visits.iter().filter(|visit| visit.date >= start) {
                let returned = visits.iter().any(|previous| {
                    previous.fixture_id != visit.fixture_id
                        && (1..=ScoutVisitLog::RETURN_DAYS)
                            .contains(&(visit.date - previous.date).num_days())
                });
                if visit.notable || returned {
                    week.by_side
                        .entry(visit.team_id)
                        .or_default()
                        .push(QualifyingVisit { visit, returned });
                }
            }
        }
        week
    }

    pub(crate) fn for_paper(&self, rosters: &[&crate::Team]) -> FxHashMap<u32, ScoutSighting> {
        let mut by_player: FxHashMap<_, Vec<_>> = FxHashMap::default();
        for team in rosters {
            for visit in self.by_side.get(&team.id).into_iter().flatten() {
                by_player
                    .entry(visit.visit.player_id)
                    .or_default()
                    .push(visit);
            }
        }
        by_player
            .into_iter()
            .map(|(player_id, mut visits)| {
                visits.sort_unstable_by_key(|entry| {
                    let visit = entry.visit;
                    (
                        std::cmp::Reverse(visit.date),
                        visit.club_id,
                        visit.fixture_id.as_ref(),
                    )
                });
                let latest = visits[0];
                let other = visits
                    .iter()
                    .find(|entry| entry.visit.club_id != latest.visit.club_id);
                (
                    player_id,
                    ScoutSighting {
                        latest: latest.visit.clone(),
                        other: other.map(|entry| entry.visit.clone()),
                        returned: latest.returned,
                    },
                )
            })
            .collect()
    }
}

/// Verified visits kept long enough to establish a return in any match
/// covered by the next weekly edition.
#[derive(Debug, Clone, Default)]
pub(crate) struct ScoutVisitLog {
    visits: FxHashMap<(u32, u32, Arc<str>), ScoutVisit>,
}

impl ScoutVisitLog {
    pub(crate) const RETURN_DAYS: i64 = 28;

    pub(crate) fn record(&mut self, today: NaiveDate, visits: Vec<ScoutVisit>) {
        for visit in visits {
            let key = (
                visit.club_id,
                visit.player_id,
                Arc::clone(&visit.fixture_id),
            );
            let current = self.visits.entry(key).or_insert_with(|| visit.clone());
            let preferred = (visit.scout_name.is_none(), visit.scout_id)
                < (current.scout_name.is_none(), current.scout_id);
            current.notable |= visit.notable;
            if preferred {
                current.scout_id = visit.scout_id;
                current.scout_name = visit.scout_name;
            }
        }
        let cutoff = today - Duration::days(Self::RETURN_DAYS + 7);
        self.visits.retain(|_, visit| visit.date >= cutoff);
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &ScoutVisit> {
        self.visits.values()
    }
}
