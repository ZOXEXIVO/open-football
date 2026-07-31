use super::facts::{
    ClubTransferWeek, PlayerStanding, RecentEvents, SquadPulse, StandingSnapshot, TransferMoveKind,
};
use crate::club::news::types::{IssueResult, NewsStory, NewsStoryKind, ResultCompetition};
use crate::{HappinessEventType, Person, Player};
use chrono::NaiveDate;

/// What the terraces have to go on this week, beyond the dressing
/// room's own mood.
///
/// The fans desk could only ever see what individual players felt about
/// the crowd, which is the wrong way round: a supporter's week is made
/// of the table, the transfer business and how the last cup tie went,
/// and none of that reached this column. Everything here is already in
/// front of the press run — it just had no way through to the one desk
/// whose job is the town.
#[derive(Clone, Copy)]
pub struct TownMood<'a> {
    /// The board's own reading of how angry the support is, 0..100.
    pub supporter_pressure: u8,
    pub standing: Option<StandingSnapshot>,
    pub results: &'a [IssueResult],
    pub transfers: Option<&'a ClubTransferWeek>,
    /// The most valuable player on the books — the yardstick for
    /// whether a piece of business is one the town will care about.
    pub peak_value: i64,
}

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

        // A crowd taking to one of its own kids is a different affection
        // from a crowd taking to a signing, and the only one a local
        // paper genuinely owns. Gated on his age rather than on where he
        // was signed from: what the terraces have decided is that he is
        // theirs.
        if feed.happened(HappinessEventType::FanPraise) && player.age(date) <= Self::DARLING_AGE {
            out.push(
                NewsStory::new(NewsStoryKind::AcademyDarling, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0),
            );
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

    /// A crowd adopts young players; this is the age at which one stops
    /// being the local kid and starts being simply a good footballer.
    const DARLING_AGE: u8 = 23;

    /// The board's supporter gauge at which the unrest is no longer a
    /// mood but a thing happening outside the ground.
    const PROTEST_PRESSURE: u8 = 70;

    /// Below this share of the season the table means nothing to
    /// anybody, which is the same bar the table desk holds itself to.
    const LATE_SEASON: f32 = 0.55;

    /// A departure or an arrival at this share of the club's most
    /// valuable player is business the town has an opinion about.
    const NOTABLE_FRACTION: i64 = 2;

    /// A cup exit by this margin is not a defeat, it is an afternoon
    /// somebody will be answering for.
    const HUMILIATION_MARGIN: i32 = 3;

    /// The mood of the ground, which only exists in aggregate. One
    /// player having a rough week with the crowd is not a story about
    /// the crowd.
    pub fn file_club(
        out: &mut Vec<NewsStory>,
        pulse: &SquadPulse,
        mood: TownMood<'_>,
        date: NaiveDate,
    ) {
        Self::file_temperature(out, pulse, mood, date);
        Self::file_business(out, mood, date);
        Self::file_cup_fallout(out, mood, date);
        Self::file_travelling_support(out, pulse, mood, date);
    }

    /// How the ground feels, in the order a supporter would rank it:
    /// organised anger first, then where the season is going, then the
    /// week's ordinary mood.
    fn file_temperature(
        out: &mut Vec<NewsStory>,
        pulse: &SquadPulse,
        mood: TownMood<'_>,
        date: NaiveDate,
    ) {
        // Not a mood any more. The board's own gauge says the unrest has
        // organised itself, which is the point at which it stops being a
        // dressing-room matter and starts being a story about the town.
        if mood.supporter_pressure >= Self::PROTEST_PRESSURE {
            out.push(
                NewsStory::new(NewsStoryKind::ProtestBrewing, date)
                    .with_numbers(mood.supporter_pressure as i32, 0)
                    .weighted((mood.supporter_pressure as i32 - Self::PROTEST_PRESSURE as i32) * 4),
            );
            return;
        }

        // Where the season is going, told from the terraces rather than
        // from the table. Both halves need the ground to agree with the
        // standings — a top-two side whose crowd is not enjoying it is
        // not a promotion-fever story, it is a different one entirely.
        if let Some(standing) = mood.standing {
            if standing.teams > 0 && standing.progress() >= Self::LATE_SEASON {
                if standing.position <= 2 && pulse.is_widespread(pulse.fan_praise) {
                    out.push(
                        NewsStory::new(NewsStoryKind::PromotionFever, date)
                            .with_numbers(standing.position as i32, standing.points as i32)
                            .weighted((standing.progress() * 60.0) as i32),
                    );
                    return;
                }

                let drop_edge = standing.teams.saturating_sub(3);
                if standing.position > drop_edge && pulse.is_widespread(pulse.fan_criticism) {
                    out.push(
                        NewsStory::new(NewsStoryKind::RelegationDread, date)
                            .with_numbers(standing.position as i32, standing.points as i32)
                            .weighted((standing.progress() * 70.0) as i32),
                    );
                    return;
                }
            }
        }

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

    /// What the terraces make of the week's business.
    ///
    /// The market desk reports a transfer; this reports the reaction,
    /// which for a supporter is the whole of it. Both are gated on the
    /// deal being large against the club's own scale, because a town
    /// does not hold a phone-in about a squad player.
    fn file_business(out: &mut Vec<NewsStory>, mood: TownMood<'_>, date: NaiveDate) {
        let Some(week) = mood.transfers else {
            return;
        };
        if mood.peak_value <= 0 {
            return;
        }
        let bar = mood.peak_value / Self::NOTABLE_FRACTION;

        // Selling somebody the ground had decided was theirs. Louder
        // than the arrival, and reported first for that reason.
        if let Some(sale) = week
            .departures
            .iter()
            .filter(|move_| move_.kind == TransferMoveKind::Paid && move_.fee >= bar)
            .max_by_key(|move_| move_.fee)
        {
            out.push(
                NewsStory::new(NewsStoryKind::FansFurySale, date)
                    .about(sale.player_id)
                    .against(sale.other_club_id)
                    .with_money(sale.fee),
            );
        }

        if let Some(signing) = week
            .arrivals
            .iter()
            .filter(|move_| move_.kind == TransferMoveKind::Paid && move_.fee >= bar)
            .max_by_key(|move_| move_.fee)
        {
            out.push(
                NewsStory::new(NewsStoryKind::FansDreamSigning, date)
                    .about(signing.player_id)
                    .against(signing.other_club_id)
                    .with_money(signing.fee),
            );
        }
    }

    /// A cup exit is the match desk's. Being taken apart in one is the
    /// town's, and it is a different piece: nobody rings a phone-in
    /// about losing a tie narrowly.
    fn file_cup_fallout(out: &mut Vec<NewsStory>, mood: TownMood<'_>, date: NaiveDate) {
        let Some(hiding) = mood
            .results
            .iter()
            .filter(|result| {
                result.competition == ResultCompetition::Cup
                    && (result.goals_against as i32 - result.goals_for as i32)
                        >= Self::HUMILIATION_MARGIN
            })
            .max_by_key(|result| result.goals_against as i32 - result.goals_for as i32)
        else {
            return;
        };

        out.push(
            NewsStory::new(NewsStoryKind::CupHumiliationFallout, date)
                .against(hiding.opponent_team_id)
                .at_home(hiding.is_home)
                .with_numbers(hiding.goals_for as i32, hiding.goals_against as i32),
        );
    }

    /// The people who paid twice — for the ticket and for the journey —
    /// and got something back for it.
    fn file_travelling_support(
        out: &mut Vec<NewsStory>,
        pulse: &SquadPulse,
        mood: TownMood<'_>,
        date: NaiveDate,
    ) {
        if !pulse.is_widespread(pulse.fan_praise) {
            return;
        }
        let Some(win) = mood
            .results
            .iter()
            .find(|result| !result.is_home && result.goals_for > result.goals_against)
        else {
            return;
        };

        out.push(
            NewsStory::new(NewsStoryKind::TravellingSupportRewarded, date)
                .against(win.opponent_team_id)
                .with_numbers(win.goals_for as i32, win.goals_against as i32),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{FansDesk, TownMood};
    use crate::club::news::desk::facts::{SquadPulse, StandingSnapshot};
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

        /// A quiet week outside the dressing room: no unrest the board
        /// has noticed, no table worth reading, no business done. Every
        /// mood test below is about the pulse alone, so this is what
        /// "nothing else happened" looks like.
        fn quiet() -> TownMood<'static> {
            TownMood {
                supporter_pressure: 0,
                standing: None,
                results: &[],
                transfers: None,
                peak_value: 0,
            }
        }

        fn file(pulse: SquadPulse) -> Vec<NewsStoryKind> {
            Self::file_with(pulse, Self::quiet())
        }

        fn file_with(pulse: SquadPulse, mood: TownMood<'_>) -> Vec<NewsStoryKind> {
            let mut out: Vec<NewsStory> = Vec::new();
            FansDesk::file_club(&mut out, &pulse, mood, Self::day());
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

    /// Organised unrest is not a mood, and it stops the mood pieces.
    ///
    /// The board's own gauge is the only thing on this desk that knows
    /// the difference between a stand that is unhappy and a support
    /// that has started meeting about it. When it says the latter, a
    /// piece reading "the crowd are getting frustrated" underneath is
    /// the paper failing to notice its own lead.
    #[test]
    fn organised_anger_replaces_the_weeks_mood_piece() {
        let angry = TownMood {
            supporter_pressure: 82,
            ..Ground::quiet()
        };

        assert_eq!(
            Ground::file_with(Ground::mood(20, 0, 6), angry),
            vec![NewsStoryKind::ProtestBrewing],
            "the protest is the story; the mood piece is not printed beneath it"
        );
    }

    /// Where the season is going, but only when the ground agrees with
    /// the table.
    ///
    /// A side in the top two whose support is not enjoying it is not a
    /// promotion-fever story — it is a different one, and printing the
    /// happy version would be the desk reading the standings instead of
    /// the people it is supposed to be about.
    #[test]
    fn the_season_story_needs_the_ground_to_agree_with_the_table() {
        let top = |praise: u16, criticism: u16| {
            Ground::file_with(
                Ground::mood(20, praise, criticism),
                TownMood {
                    standing: Some(StandingSnapshot {
                        position: 2,
                        teams: 24,
                        points: 70,
                        played: 32,
                        total_rounds: 46,
                    }),
                    ..Ground::quiet()
                },
            )
        };

        assert_eq!(top(6, 0), vec![NewsStoryKind::PromotionFever]);
        assert_eq!(
            top(0, 6),
            vec![NewsStoryKind::FansTurnOnTeam],
            "a promotion push nobody is enjoying is not promotion fever"
        );

        // …and the same shape at the wrong end of the table.
        let bottom = Ground::file_with(
            Ground::mood(20, 0, 6),
            TownMood {
                standing: Some(StandingSnapshot {
                    position: 23,
                    teams: 24,
                    points: 24,
                    played: 32,
                    total_rounds: 46,
                }),
                ..Ground::quiet()
            },
        );
        assert_eq!(bottom, vec![NewsStoryKind::RelegationDread]);
    }

    /// A table nobody can read yet is not a story about the town. The
    /// desk holds itself to the same share-of-season bar the table desk
    /// does, so a strong start in September stays a strong start.
    #[test]
    fn an_early_table_is_not_a_mood() {
        let september = Ground::file_with(
            Ground::mood(20, 6, 0),
            TownMood {
                standing: Some(StandingSnapshot {
                    position: 1,
                    teams: 24,
                    points: 12,
                    played: 5,
                    total_rounds: 46,
                }),
                ..Ground::quiet()
            },
        );

        assert_eq!(september, vec![NewsStoryKind::FansGetBehind]);
    }
}
