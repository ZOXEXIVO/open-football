//! Pure helpers for single-leg knockout (cup) scheduling.
//!
//! These functions are deliberately free of simulation state so they can
//! be unit-tested in isolation: pairing a round, working out who advanced
//! (penalty-aware), and placing rounds on the calendar. The orchestration
//! that ties them to a live `DomesticCup` lives in
//! `crate::league::domestic_cup`.
//!
//! Cup ties are one-leg. A level score after extra time is resolved on the
//! penalty-shootout tally, which `Score::outcome` already encodes — so
//! `tie_winner` just reads `outcome()`.

use crate::league::{ScheduleItem, ScheduleTour};
use crate::r#match::MatchResultOutcome;
use chrono::{Datelike, Duration, NaiveDate, Weekday};
use std::collections::HashSet;

/// Largest power of two `<= n`. Returns 0 for `n == 0`.
pub fn largest_pow2_le(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let mut p = 1;
    while p * 2 <= n {
        p *= 2;
    }
    p
}

/// `(matches, byes)` for a knockout round of `n` seeded teams.
///
/// A power-of-two field plays a full round (`n/2` matches, no byes).
/// Otherwise only enough ties are played to reduce the field to the next
/// lower power of two; the surplus (top seeds) sit the round out on byes.
/// `matches*2 + byes == n` always holds, and `matches + byes` is a power
/// of two.
pub fn round_shape(n: usize) -> (usize, usize) {
    if n < 2 {
        return (0, 0);
    }
    let p = largest_pow2_le(n);
    if p == n {
        (n / 2, 0)
    } else {
        (n - p, 2 * p - n)
    }
}

/// Total knockout rounds needed to resolve `n` entrants (`ceil(log2 n)`).
pub fn total_rounds(n: usize) -> u8 {
    if n < 2 {
        return 0;
    }
    let mut rounds = 0u8;
    let mut size = 1usize;
    while size < n {
        size *= 2;
        rounds += 1;
    }
    rounds
}

/// Build the pairings and byes for one knockout round from teams given in
/// seed order (best seed first). Byes are awarded to the top seeds; the
/// remaining teams are drawn strongest-vs-weakest. Returns
/// `(pairings, byes)` where each pairing is `(home, away)`.
///
/// With fewer than two teams there are no ties and everyone "advances"
/// (returned as byes) — the caller treats a single survivor as champion.
pub fn pair_knockout_round(seeded_teams: &[u32]) -> (Vec<(u32, u32)>, Vec<u32>) {
    let (num_matches, num_byes) = round_shape(seeded_teams.len());
    if num_matches == 0 {
        return (Vec::new(), seeded_teams.to_vec());
    }
    let byes = seeded_teams[..num_byes].to_vec();
    let playing = &seeded_teams[num_byes..];
    let m = playing.len();
    let mut pairings = Vec::with_capacity(num_matches);
    for i in 0..num_matches {
        pairings.push((playing[i], playing[m - 1 - i]));
    }
    (pairings, byes)
}

/// Winner of a played knockout tie. Uses `Score::outcome`, which resolves
/// a level regulation+extra-time score on the penalty-shootout tally.
/// `None` if the tie has not been played.
pub fn tie_winner(item: &ScheduleItem) -> Option<u32> {
    let score = item.result.as_ref()?;
    Some(match score.outcome() {
        MatchResultOutcome::HomeWin => item.home_team_id,
        MatchResultOutcome::AwayWin => item.away_team_id,
        // A knockout tie can't truly end level (penalties decide); the
        // home side is an arbitrary but deterministic guard.
        MatchResultOutcome::Draw => item.home_team_id,
    })
}

/// Teams still alive after `tours`, given the full set that entered round
/// one (in seed order). Each round contributes its tie winners plus any
/// entrant who didn't appear that round (a bye). Returns `None` if any
/// tie in the supplied tours is missing a result — i.e. a round is still
/// in progress and the field is not yet resolved.
pub fn advancing_teams(tours: &[ScheduleTour], round_one_field: &[u32]) -> Option<Vec<u32>> {
    let mut entering: Vec<u32> = round_one_field.to_vec();
    for tour in tours {
        let mut played: HashSet<u32> = HashSet::with_capacity(tour.items.len() * 2);
        let mut winners: Vec<u32> = Vec::with_capacity(tour.items.len());
        for item in &tour.items {
            played.insert(item.home_team_id);
            played.insert(item.away_team_id);
            winners.push(tie_winner(item)?);
        }
        let byes = entering.into_iter().filter(|t| !played.contains(t));
        entering = winners.into_iter().chain(byes).collect();
    }
    Some(entering)
}

/// The cup champion, if the competition has resolved to a single team.
pub fn cup_champion(tours: &[ScheduleTour], round_one_field: &[u32]) -> Option<u32> {
    if tours.is_empty() {
        return None;
    }
    match advancing_teams(tours, round_one_field) {
        Some(alive) if alive.len() == 1 => Some(alive[0]),
        _ => None,
    }
}

/// Snap `date` forward to the next Wednesday. Cup ties are midweek so they
/// never collide with the Saturday league programme (the round-robin
/// scheduler always lands on Saturdays), which keeps a team off two
/// fixtures on the same day without any cross-competition date bookkeeping.
pub fn next_midweek(date: NaiveDate) -> NaiveDate {
    let mut d = date;
    while d.weekday() != Weekday::Wed {
        d = d.succ_opt().unwrap();
    }
    d
}

/// Calendar date for knockout `round` (1-based) of a competition with
/// `total` rounds, spread across the season window `[season_start,
/// season_end]`. Generic over any season shape (European autumn-spring or
/// calendar-year); falls back to fortnightly spacing when the window is too
/// small to divide. Always returns a midweek (Wednesday) date.
pub fn cup_round_date(
    season_start: NaiveDate,
    season_end: NaiveDate,
    round: u8,
    total: u8,
) -> NaiveDate {
    // Keep round one out of pre-season and the final just shy of the
    // season close.
    let window_start = season_start + Duration::days(30);
    let window_end = season_end - Duration::days(7);
    let offset = round.saturating_sub(1) as i64;
    let date = if total <= 1 || window_end <= window_start {
        window_start + Duration::days(14 * offset)
    } else {
        let span = (window_end - window_start).num_days();
        window_start + Duration::days(span * offset / (total - 1) as i64)
    };
    next_midweek(date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::{Score, TeamScore};
    use chrono::{NaiveDateTime, NaiveTime};

    fn dt() -> NaiveDateTime {
        NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 9, 2).unwrap(),
            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        )
    }

    /// Build a played tie from a pairing with the given regulation score
    /// and optional shootout tally.
    fn played(home: u32, away: u32, hg: u8, ag: u8, hs: u8, as_: u8) -> ScheduleItem {
        let mut item = ScheduleItem::new(1, "cup".into(), home, away, dt(), None);
        item.result = Some(Score {
            home_team: TeamScore::new_with_score(home, hg),
            away_team: TeamScore::new_with_score(away, ag),
            details: Vec::new(),
            home_shootout: hs,
            away_shootout: as_,
        });
        item
    }

    fn tour(num: u8, items: Vec<ScheduleItem>) -> ScheduleTour {
        ScheduleTour { num, items }
    }

    #[test]
    fn power_of_two_field_has_no_byes() {
        assert_eq!(round_shape(8), (4, 0));
        assert_eq!(round_shape(16), (8, 0));
        let (pairings, byes) = pair_knockout_round(&[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(pairings.len(), 4);
        assert!(byes.is_empty());
    }

    #[test]
    fn non_power_of_two_byes_go_to_top_seeds() {
        // 6 teams: reduce to 4 -> 2 ties, 2 byes. Top two seeds rest.
        assert_eq!(round_shape(6), (2, 2));
        let seeded = [10, 9, 8, 7, 6, 5]; // already best-first
        let (pairings, byes) = pair_knockout_round(&seeded);
        assert_eq!(byes, vec![10, 9]);
        assert_eq!(pairings.len(), 2);
        // matches*2 + byes == field
        assert_eq!(pairings.len() * 2 + byes.len(), seeded.len());
    }

    #[test]
    fn ten_teams_reduce_to_eight() {
        // P=8, matches = 2, byes = 6 -> survivors 8 (a power of two).
        assert_eq!(round_shape(10), (2, 6));
        let seeded: Vec<u32> = (1..=10).collect();
        let (pairings, byes) = pair_knockout_round(&seeded);
        assert_eq!(pairings.len() + byes.len(), 8);
    }

    #[test]
    fn no_team_appears_twice_in_a_round() {
        let seeded: Vec<u32> = (1..=13).collect();
        let (pairings, byes) = pair_knockout_round(&seeded);
        let mut seen = HashSet::new();
        for (h, a) in &pairings {
            assert!(seen.insert(*h), "team {} twice", h);
            assert!(seen.insert(*a), "team {} twice", a);
        }
        for b in &byes {
            assert!(seen.insert(*b), "bye team {} also played", b);
        }
        // Every entrant accounted for exactly once.
        assert_eq!(seen.len(), 13);
    }

    #[test]
    fn fewer_than_two_teams_yields_no_ties() {
        let (pairings, byes) = pair_knockout_round(&[42]);
        assert!(pairings.is_empty());
        assert_eq!(byes, vec![42]);
        let (pairings, byes) = pair_knockout_round(&[]);
        assert!(pairings.is_empty());
        assert!(byes.is_empty());
    }

    #[test]
    fn winner_advances_on_penalties_when_level() {
        // Regulation 1-1, away wins the shootout 4-2.
        let item = played(7, 8, 1, 1, 2, 4);
        assert_eq!(tie_winner(&item), Some(8));
        // Regulation decided outright.
        let item = played(7, 8, 2, 0, 0, 0);
        assert_eq!(tie_winner(&item), Some(7));
        // Unplayed.
        let item = ScheduleItem::new(1, "cup".into(), 7, 8, dt(), None);
        assert_eq!(tie_winner(&item), None);
    }

    #[test]
    fn advancing_combines_winners_and_byes() {
        // 6-team field: round 1 has 2 ties + 2 byes (seeds 1,2).
        let field = vec![1, 2, 3, 4, 5, 6];
        let (pairings, byes) = pair_knockout_round(&field);
        assert_eq!(byes, vec![1, 2]);
        // Play round 1: lower seed wins each tie (away on penalties once).
        let r1 = tour(
            1,
            vec![
                played(pairings[0].0, pairings[0].1, 0, 1, 0, 0),
                played(pairings[1].0, pairings[1].1, 1, 1, 2, 4),
            ],
        );
        let alive = advancing_teams(&[r1.clone()], &field).unwrap();
        // 2 winners + 2 byes = 4 alive (a power of two).
        assert_eq!(alive.len(), 4);
        assert!(alive.contains(&1) && alive.contains(&2));
        // No champion yet.
        assert_eq!(cup_champion(&[r1], &field), None);
    }

    #[test]
    fn pending_round_returns_none() {
        let field = vec![1, 2, 3, 4];
        let r1 = tour(
            1,
            vec![
                played(1, 4, 2, 0, 0, 0),
                ScheduleItem::new(1, "cup".into(), 2, 3, dt(), None), // unplayed
            ],
        );
        assert_eq!(advancing_teams(&[r1], &field), None);
    }

    #[test]
    fn final_leaves_exactly_one_champion() {
        let field = vec![1, 2, 3, 4];
        // Semi-finals.
        let (p, byes) = pair_knockout_round(&field);
        assert!(byes.is_empty());
        let r1 = tour(
            1,
            vec![
                played(p[0].0, p[0].1, 3, 1, 0, 0),
                played(p[1].0, p[1].1, 0, 2, 0, 0),
            ],
        );
        let semis_winners = advancing_teams(&[r1.clone()], &field).unwrap();
        assert_eq!(semis_winners.len(), 2);
        // Final.
        let (pf, _) = pair_knockout_round(&semis_winners);
        assert_eq!(pf.len(), 1);
        let r2 = tour(2, vec![played(pf[0].0, pf[0].1, 1, 1, 5, 4)]);
        let champ = cup_champion(&[r1, r2], &field);
        assert_eq!(champ, Some(pf[0].0));
    }

    #[test]
    fn round_date_is_midweek_and_ordered() {
        let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2027, 5, 30).unwrap();
        let r1 = cup_round_date(start, end, 1, 5);
        let r3 = cup_round_date(start, end, 3, 5);
        let r5 = cup_round_date(start, end, 5, 5);
        assert_eq!(r1.weekday(), Weekday::Wed);
        assert_eq!(r5.weekday(), Weekday::Wed);
        assert!(r1 < r3 && r3 < r5, "rounds must move forward in time");
        assert!(r1 >= start && r5 <= end);
    }
}
