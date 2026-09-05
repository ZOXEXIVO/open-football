use super::facts::{NextFixture, StandingSnapshot};
use crate::club::news::types::{NewsStory, NewsStoryKind};
use chrono::NaiveDate;
use rustc_hash::FxHashSet;

/// Next week, before it happens.
///
/// Every local paper looks forward as well as back: the Monday edition
/// closes on who is coming to the ground on Saturday, and until now the
/// page had no idea. The desk reads the fixture the press run found in
/// the schedules and prints the angle a town would talk about — the
/// neighbours, the leaders, the two sides fighting over the same three
/// points at the bottom, the one who used to play for them — and, when
/// there is no angle, the plain preview with the opponent's place in
/// the table on it.
///
/// One preview per edition. A week only has one Saturday in it, and a
/// club that plays twice gets the nearer game; the cup tie in midweek
/// beats the league game after it because it is played first.
pub struct PreviewDesk;

impl PreviewDesk {
    /// Both sides this high, this far in, and the meeting is the
    /// summit rather than a fixture.
    const SUMMIT_PLACES: u8 = 3;
    const SUMMIT_MIN_PROGRESS: f32 = 0.30;
    /// Both sides in the drop places with half the season gone: the
    /// six-pointer. Earlier than that the bottom of the table is noise
    /// and the phrase would be a cliché rather than a sum.
    const DROP_PLACES: u8 = 3;
    const SIX_POINTER_MIN_PROGRESS: f32 = 0.50;

    pub fn file(
        out: &mut Vec<NewsStory>,
        fixture: Option<NextFixture>,
        standing: Option<StandingSnapshot>,
        rival_team_ids: &FxHashSet<u32>,
        date: NaiveDate,
    ) {
        let Some(fixture) = fixture else {
            return;
        };
        if fixture.opponent_team_id == 0 {
            return;
        }

        let own = standing.map(|table| table.position).unwrap_or(0);
        let teams = standing.map(|table| table.teams).unwrap_or(0);
        let progress = standing.map(|table| table.progress()).unwrap_or(0.0);
        let opponent = fixture.opponent_position;
        let is_derby = rival_team_ids.contains(&fixture.opponent_team_id);

        let in_summit = |position: u8| (1..=Self::SUMMIT_PLACES).contains(&position);
        let in_drop = |position: u8| {
            teams > Self::DROP_PLACES * 2 && position > teams.saturating_sub(Self::DROP_PLACES)
        };

        let kind = if fixture.is_cup {
            NewsStoryKind::NextUpCupTie
        } else if is_derby {
            NewsStoryKind::NextUpDerby
        } else if in_summit(own) && in_summit(opponent) && progress >= Self::SUMMIT_MIN_PROGRESS {
            NewsStoryKind::NextUpSummit
        } else if in_drop(own) && in_drop(opponent) && progress >= Self::SIX_POINTER_MIN_PROGRESS {
            NewsStoryKind::NextUpSixPointer
        } else if opponent == 1 {
            NewsStoryKind::NextUpLeaders
        } else if opponent > 0 && fixture.is_home {
            NewsStoryKind::NextUpHome
        } else if opponent > 0 {
            NewsStoryKind::NextUpAway
        } else {
            // A league opponent the table cannot place — nothing to say
            // about them that the copy could set.
            return;
        };

        // `a` is the opponent's place, `b` our own: the plain preview
        // quotes the first and the summit piece may quote either.
        out.push(
            NewsStory::new(kind, date)
                .against(fixture.opponent_team_id)
                .with_numbers(i32::from(opponent), i32::from(own)),
        );

        // The reception. Runs beside the fixture piece rather than
        // instead of it — it is about a man, not a match.
        if fixture.old_boy_player_id != 0 {
            out.push(
                NewsStory::new(NewsStoryKind::NextUpOldBoy, date)
                    .about(fixture.old_boy_player_id)
                    .against(fixture.opponent_team_id)
                    .with_numbers(i32::from(opponent), i32::from(own)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PreviewDesk;
    use crate::club::news::desk::facts::{NextFixture, StandingSnapshot};
    use crate::club::news::types::{NewsStory, NewsStoryKind};
    use chrono::NaiveDate;
    use rustc_hash::FxHashSet;

    struct Week;

    impl Week {
        const OPPONENT: u32 = 44;

        fn day() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 3, 2).unwrap()
        }

        fn table(position: u8) -> Option<StandingSnapshot> {
            Some(StandingSnapshot {
                position,
                teams: 20,
                points: 40,
                played: 26,
                total_rounds: 38,
            })
        }

        fn fixture(opponent_position: u8, is_home: bool) -> NextFixture {
            NextFixture {
                opponent_team_id: Self::OPPONENT,
                is_home,
                is_cup: false,
                opponent_position,
                old_boy_player_id: 0,
            }
        }

        fn kinds(
            fixture: NextFixture,
            own_position: u8,
            rivals: &[u32],
        ) -> Vec<NewsStoryKind> {
            let mut out: Vec<NewsStory> = Vec::new();
            let rivals: FxHashSet<u32> = rivals.iter().copied().collect();
            PreviewDesk::file(
                &mut out,
                Some(fixture),
                Self::table(own_position),
                &rivals,
                Self::day(),
            );
            out.iter().map(|story| story.kind).collect()
        }
    }

    #[test]
    fn the_neighbours_outrank_every_other_angle() {
        let kinds = Week::kinds(Week::fixture(1, true), 2, &[Week::OPPONENT]);
        assert_eq!(kinds, vec![NewsStoryKind::NextUpDerby]);
    }

    #[test]
    fn first_against_second_is_the_summit() {
        let kinds = Week::kinds(Week::fixture(1, false), 2, &[]);
        assert_eq!(kinds, vec![NewsStoryKind::NextUpSummit]);
    }

    #[test]
    fn two_sides_in_the_drop_places_play_a_six_pointer() {
        let kinds = Week::kinds(Week::fixture(19, true), 18, &[]);
        assert_eq!(kinds, vec![NewsStoryKind::NextUpSixPointer]);
    }

    #[test]
    fn a_mid_table_side_facing_the_leaders_is_told_so() {
        let kinds = Week::kinds(Week::fixture(1, true), 11, &[]);
        assert_eq!(kinds, vec![NewsStoryKind::NextUpLeaders]);
    }

    #[test]
    fn the_plain_preview_knows_which_ground_it_is_at() {
        assert_eq!(
            Week::kinds(Week::fixture(9, true), 11, &[]),
            vec![NewsStoryKind::NextUpHome]
        );
        assert_eq!(
            Week::kinds(Week::fixture(9, false), 11, &[]),
            vec![NewsStoryKind::NextUpAway]
        );
    }

    #[test]
    fn a_cup_tie_is_previewed_as_one_whoever_the_opponent_is() {
        let fixture = NextFixture {
            is_cup: true,
            opponent_position: 0,
            ..Week::fixture(0, true)
        };
        assert_eq!(
            Week::kinds(fixture, 11, &[]),
            vec![NewsStoryKind::NextUpCupTie]
        );
    }

    #[test]
    fn an_old_boy_runs_beside_the_fixture_piece() {
        let fixture = NextFixture {
            old_boy_player_id: 7,
            ..Week::fixture(9, true)
        };
        let mut out: Vec<NewsStory> = Vec::new();
        PreviewDesk::file(
            &mut out,
            Some(fixture),
            Week::table(11),
            &FxHashSet::default(),
            Week::day(),
        );

        let old_boy = out
            .iter()
            .find(|story| story.kind == NewsStoryKind::NextUpOldBoy)
            .expect("the reception is its own piece");
        assert_eq!(old_boy.player_id, 7);
        assert_eq!(old_boy.other_id, Week::OPPONENT);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn a_week_with_no_fixture_previews_nothing() {
        let mut out: Vec<NewsStory> = Vec::new();
        PreviewDesk::file(
            &mut out,
            None,
            Week::table(11),
            &FxHashSet::default(),
            Week::day(),
        );
        assert!(out.is_empty());
    }
}
