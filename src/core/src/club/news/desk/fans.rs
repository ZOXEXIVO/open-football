use super::facts::{PlayerStanding, RecentEvents, SquadPulse};
use crate::club::news::types::{NewsStory, NewsStoryKind};
use crate::{HappinessEventType, Player};
use chrono::NaiveDate;

/// The terraces and the press box.
///
/// Everything else on the page is what the club did; this is what it
/// felt like to the people it was done to. A local paper without that
/// column reads as a results service, which is the difference between
/// a newspaper and a fixture list.
pub struct FansDesk;

impl FansDesk {
    /// Per-player beats, filed from the squad walk.
    pub fn file_player(
        out: &mut Vec<NewsStory>,
        player: &Player,
        pulse: &mut SquadPulse,
        date: NaiveDate,
    ) {
        let feed = RecentEvents::week(player);
        let importance = PlayerStanding::importance(player);

        // Tallies first: these feed the club-level mood whether or not
        // the individual beat is worth its own line.
        if feed.happened(HappinessEventType::FanPraise) {
            pulse.fan_praise = pulse.fan_praise.saturating_add(1);
        }
        if feed.happened(HappinessEventType::FanCriticism) {
            pulse.fan_criticism = pulse.fan_criticism.saturating_add(1);
        }
        if feed
            .any_of(&[
                HappinessEventType::MediaCriticism,
                HappinessEventType::MediaPressureMounting,
            ])
            .is_some()
        {
            pulse.media_criticism = pulse.media_criticism.saturating_add(1);
        }

        if feed.happened(HappinessEventType::FansChantPlayerName) {
            out.push(NewsStory::new(NewsStoryKind::FansChant, date).about(player.id));
        }

        // The press turning on a player is a different story from the
        // crowd doing it, and it lands hardest on the names carrying
        // the expectation.
        if feed
            .any_of(&[
                HappinessEventType::MediaPressureMounting,
                HappinessEventType::FanExpectationBurden,
                HappinessEventType::MediaSpotlightPressure,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::MediaPressure, date)
                    .about(player.id)
                    .weighted(importance),
            );
        } else if feed.happened(HappinessEventType::MediaPraise) {
            out.push(
                NewsStory::new(NewsStoryKind::MediaDarling, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
        }

        if feed.happened(HappinessEventType::FansReactToTransferRumour) {
            out.push(
                NewsStory::new(NewsStoryKind::FansAngryAtRumour, date)
                    .about(player.id)
                    .weighted(importance),
            );
        }
    }

    /// The mood of the ground, which only exists in aggregate. One
    /// player having a rough week with the crowd is not a story about
    /// the crowd.
    pub fn file_club(out: &mut Vec<NewsStory>, pulse: &SquadPulse, date: NaiveDate) {
        if pulse.is_widespread(pulse.fan_criticism) {
            out.push(
                NewsStory::new(NewsStoryKind::FansTurnOnTeam, date)
                    .with_numbers(pulse.fan_criticism as i32, pulse.seniors as i32)
                    .weighted(pulse.fan_criticism as i32 * 8),
            );
            return;
        }

        if pulse.is_widespread(pulse.fan_praise) {
            out.push(
                NewsStory::new(NewsStoryKind::FansGetBehind, date)
                    .with_numbers(pulse.fan_praise as i32, pulse.seniors as i32)
                    .weighted(pulse.fan_praise as i32 * 6),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FansDesk;
    use crate::club::news::desk::facts::SquadPulse;
    use crate::club::news::types::{NewsStory, NewsStoryKind};
    use chrono::NaiveDate;

    struct Ground;

    impl Ground {
        fn day() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 3, 2).unwrap()
        }

        fn mood(seniors: u16, praise: u16, criticism: u16) -> SquadPulse {
            SquadPulse {
                seniors,
                fan_praise: praise,
                fan_criticism: criticism,
                ..SquadPulse::default()
            }
        }

        fn file(pulse: SquadPulse) -> Vec<NewsStoryKind> {
            let mut out: Vec<NewsStory> = Vec::new();
            FansDesk::file_club(&mut out, &pulse, Self::day());
            out.iter().map(|story| story.kind).collect()
        }
    }

    #[test]
    fn one_unhappy_player_is_not_the_mood_of_a_stadium() {
        assert!(
            Ground::file(Ground::mood(24, 0, 2)).is_empty(),
            "two players having a bad week with the crowd is not a crowd story"
        );
    }

    #[test]
    fn a_stand_that_has_had_enough_says_so() {
        assert_eq!(
            Ground::file(Ground::mood(24, 0, 6)),
            vec![NewsStoryKind::FansTurnOnTeam]
        );
    }

    #[test]
    fn a_ground_behind_its_team_reads_the_other_way() {
        assert_eq!(
            Ground::file(Ground::mood(20, 5, 0)),
            vec![NewsStoryKind::FansGetBehind]
        );
    }

    /// A week that produced both is a week the paper reports as unrest.
    /// Supporters who are split are not supporters who are happy.
    #[test]
    fn anger_outranks_approval_when_the_ground_is_split() {
        assert_eq!(
            Ground::file(Ground::mood(20, 4, 4)),
            vec![NewsStoryKind::FansTurnOnTeam]
        );
    }
}
