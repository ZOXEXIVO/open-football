use super::dugout::DugoutDesk;
use super::facts::{CareerRecord, PlayerStanding, RecentEvents, SquadPulse, WeeklyMatchFacts};
use super::fans::FansDesk;
use super::market::MarketDesk;
use super::rumour::RumourDesk;
use crate::club::news::types::{NewsStory, NewsStoryKind};
use crate::{HappinessEventType, Person, Player, PlayerFieldPositionGroup, Team};
use chrono::{Duration, NaiveDate};

/// Squad news: form, fitness, discipline, milestones, contracts — plus
/// everything a paper's own players say and do off the pitch.
///
/// This desk owns the single walk over the rosters one edition covers.
/// The rumour desk hangs off the same walk rather than repeating it: a
/// world with thousands of clubs cannot afford to iterate every squad
/// twice a week just to keep two detectors in separate files.
pub struct SquadDesk;

impl SquadDesk {
    /// Career appearances are reported at each round hundred, goals at
    /// each round fifty — the numbers a club shop prints on a
    /// commemorative shirt.
    const APPS_STEP: i32 = 100;
    const GOALS_STEP: i32 = 50;
    /// A player is only worth a milestone piece once he has been around
    /// long enough for the number to mean something locally.
    const SERVANT_SEASONS: i32 = 8;

    /// A player this age or younger is a prospect, and the same form
    /// reads as a breakthrough rather than as a good season.
    const PROSPECT_AGE: u8 = 21;
    /// The bar a young player has to clear before the paper calls him
    /// the real thing. Lower than the senior bar on purpose — holding a
    /// place in a senior side at nineteen IS the achievement — but he
    /// still has to have played enough for the number to mean anything.
    const RISING_STAR_RATING: f32 = 7.00;
    /// Senior appearances a prospect needs before there is anything to
    /// write, even when the club has already called it a breakthrough.
    const BREAKTHROUGH_MIN_APPS: i32 = 3;

    /// The single pass over the rosters one edition covers. Every desk
    /// that needs to look at players hangs off this one walk — the
    /// rumour mill, the dugout, the terraces, and the verdict on last
    /// summer's business. Splitting them into their own passes would
    /// multiply the weekly cost across every squad in the world for no
    /// editorial gain, and the squad-wide moods could not be tallied at
    /// all.
    ///
    /// `rosters` is the paper's own side plus any squad with no paper of
    /// its own that it speaks for, so every player in the world is
    /// walked exactly once a week however many editions a club prints.
    ///
    /// Returns what the week did to the dressing room as a whole, for
    /// the desks whose stories only exist in aggregate.
    pub fn file(
        out: &mut Vec<NewsStory>,
        rosters: &[&Team],
        facts: &WeeklyMatchFacts,
        played_this_week: bool,
        date: NaiveDate,
    ) -> SquadPulse {
        let mut pulse = SquadPulse::default();

        for team in rosters {
            let senior = team.team_type.is_own_team();

            for player in team.players.iter() {
                if player.is_retired() {
                    continue;
                }

                let feed = RecentEvents::week(player);

                Self::file_match_deeds(out, player.id, facts, date);
                Self::file_fitness(out, player, &feed, date);
                Self::file_awards(out, player, &feed, date);
                Self::file_life_events(out, player, &feed, date);

                if senior {
                    pulse.seniors = pulse.seniors.saturating_add(1);

                    Self::file_form(out, player, played_this_week, date);
                    Self::file_breakthrough(out, player, &feed, date);
                    Self::file_discipline(out, player, &feed, date);
                    Self::file_dressing_room(out, player, &feed, date);
                    Self::file_career_life(out, player, date);
                    Self::file_competition_for_places(out, player, &feed, date);
                    Self::file_contract(out, player, &feed, date);
                    DugoutDesk::file_player(out, player, &mut pulse, date);
                    FansDesk::file_player(out, player, &mut pulse, date);
                    MarketDesk::file_verdict(out, player, date);
                    RumourDesk::file_player(out, player, date);
                }
            }
        }

        pulse
    }

    /// Saves that turn a keeper's afternoon into a piece. Below this he
    /// had a busy game; at it, he is the reason there was a point.
    const KEEPER_MASTERCLASS_SAVES: u16 = 6;
    /// Goals past him before the paper stops calling it one of those
    /// days. Four is the scoreline a supporter argues about all week.
    const KEEPER_OVERRUN_CONCEDED: u16 = 4;

    fn file_match_deeds(
        out: &mut Vec<NewsStory>,
        player_id: u32,
        facts: &WeeklyMatchFacts,
        date: NaiveDate,
    ) {
        if let Some(goals) = facts.hat_tricks.get(&player_id) {
            out.push(
                NewsStory::new(NewsStoryKind::HatTrick, date)
                    .about(player_id)
                    .with_numbers(*goals as i32, 0)
                    // Four and five-goal hauls are a different story again.
                    .weighted((*goals as i32 - 3) * 60),
            );
        }

        if facts.red_cards.contains(&player_id) {
            out.push(NewsStory::new(NewsStoryKind::RedCard, date).about(player_id));
        }

        Self::file_keeper_deeds(out, player_id, facts, date);
    }

    /// The goalkeeper's week.
    ///
    /// Every other beat on this desk is read off goals, cards and
    /// ratings —
    /// the currency of the ten outfield players. A keeper earns none of
    /// it: his best afternoon and his worst leave the same mark on a
    /// scoreline, and until the press run read the match stat lines
    /// directly, the only thing a paper could say about him was how many
    /// clean sheets he had. These four are what actually happens to a
    /// goalkeeper.
    ///
    /// At most one per keeper per week, sourest first — a paper does not
    /// run "he was magnificent" and "he was at fault" about the same man
    /// on the same page, and the error is the one a reader remembers.
    pub(super) fn file_keeper_deeds(
        out: &mut Vec<NewsStory>,
        player_id: u32,
        facts: &WeeklyMatchFacts,
        date: NaiveDate,
    ) {
        let Some(keeper) = facts.keepers.get(&player_id) else {
            return;
        };

        // The save the tie turned on outranks everything, including his
        // own mistakes — a shoot-out is remembered by its ending.
        if keeper.penalties_saved > 0 {
            out.push(
                NewsStory::new(NewsStoryKind::KeeperPenaltySave, date)
                    .about(player_id)
                    .with_numbers(keeper.penalties_saved as i32, keeper.saves as i32)
                    .weighted((keeper.penalties_saved as i32 - 1) * 70),
            );
            return;
        }

        if keeper.errors_leading_to_goal > 0 {
            out.push(
                NewsStory::new(NewsStoryKind::KeeperBlunder, date)
                    .about(player_id)
                    .with_numbers(keeper.errors_leading_to_goal as i32, keeper.conceded as i32)
                    .weighted((keeper.errors_leading_to_goal as i32 - 1) * 60),
            );
            return;
        }

        // A shot-stopping display and a hiding are not mutually
        // exclusive — a keeper can make eight saves and still pick the
        // ball out four times, and on that afternoon the saves are the
        // story rather than the scoreline.
        if keeper.saves >= Self::KEEPER_MASTERCLASS_SAVES {
            out.push(
                NewsStory::new(NewsStoryKind::KeeperMasterclass, date)
                    .about(player_id)
                    .with_numbers(keeper.saves as i32, keeper.conceded as i32)
                    .weighted((keeper.saves as i32 - Self::KEEPER_MASTERCLASS_SAVES as i32) * 25),
            );
            return;
        }

        if keeper.conceded >= Self::KEEPER_OVERRUN_CONCEDED {
            out.push(
                NewsStory::new(NewsStoryKind::KeeperOverrun, date)
                    .about(player_id)
                    .with_numbers(keeper.conceded as i32, keeper.saves as i32)
                    .weighted((keeper.conceded as i32 - Self::KEEPER_OVERRUN_CONCEDED as i32) * 30),
            );
        }
    }

    fn file_fitness(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        let days_out = player.player_attributes.injury_days_remaining as i32;

        // Two weeks or more is a selection problem worth printing; a
        // knock that clears by Saturday is not news.
        if player.player_attributes.is_injured && days_out >= 14 {
            out.push(
                NewsStory::new(NewsStoryKind::InjuryBlow, date)
                    .about(player.id)
                    .with_numbers(days_out, 0)
                    .weighted(PlayerStanding::importance(player) + (days_out.min(180) / 3)),
            );
        }

        // A setback is a separate, sourer story: the club had a date in
        // mind and has just lost it.
        if feed.happened(HappinessEventType::InjurySetback) {
            out.push(
                NewsStory::new(NewsStoryKind::InjurySetback, date)
                    .about(player.id)
                    .with_numbers(days_out, 0)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
        }
    }

    fn file_awards(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        let week_start = date - Duration::days(7);
        // The timeline is chronological and can run to a thousand entries
        // for a decorated career, so walk back from the newest and stop
        // the moment the week closes rather than reading a whole career
        // for every player in the world, every week.
        let won_pom = player
            .awards_count
            .timeline
            .iter()
            .rev()
            .take_while(|entry| entry.date >= week_start)
            .any(|entry| {
                entry.date <= date
                    && matches!(
                        entry.kind,
                        crate::AwardReputationKind::PlayerOfTheMonth
                            | crate::AwardReputationKind::YoungPlayerOfTheMonth
                    )
            });

        if won_pom {
            out.push(NewsStory::new(NewsStoryKind::PlayerOfMonth, date).about(player.id));
        }

        // The two honours only a goalkeeper can collect. The glove is a
        // season's verdict; the shut-out milestone is the number his
        // career is actually counted in, and neither has ever had
        // anywhere to appear on this page.
        // The award is handed out at the turn of the season, which is
        // also when the live counters are drained — so the figure its
        // copy quotes has to be checked rather than assumed. "On 0
        // clean sheets" under a golden-glove headline is the same
        // mistake as a season average of 0.00.
        if feed.happened(HappinessEventType::LeagueGoldenGlove) {
            let clean_sheets = player.statistics.clean_sheets as i32;
            if clean_sheets > 0 {
                out.push(
                    NewsStory::new(NewsStoryKind::KeeperGoldenGlove, date)
                        .about(player.id)
                        .with_numbers(clean_sheets, 0)
                        .weighted(PlayerStanding::importance(player) / 2),
                );
            }
        }

        if feed.happened(HappinessEventType::CleanSheetMilestone) {
            let clean_sheets = CareerRecord::clean_sheets(player);
            if clean_sheets > 0 {
                out.push(
                    NewsStory::new(NewsStoryKind::KeeperShutoutMilestone, date)
                        .about(player.id)
                        .with_numbers(clean_sheets, CareerRecord::appearances(player))
                        .weighted(clean_sheets / 2),
                );
            }
        }

        if feed
            .any_of(&[
                HappinessEventType::TeamOfTheWeekSelection,
                HappinessEventType::YoungTeamOfTheWeekSelection,
            ])
            .is_some()
        {
            // The rating lands in `b`, which is the slot the `{rating}`
            // placeholder reads from.
            let rating = player
                .statistics
                .average_rating_realistic(player.position().position_group());
            out.push(
                NewsStory::new(NewsStoryKind::TeamOfTheWeek, date)
                    .about(player.id)
                    .with_numbers(0, (rating * 100.0) as i32),
            );
        }
    }

    /// Beats the player's own event feed records: a debut, a recovery, a
    /// career milestone. The feed is the only place these are marked, so
    /// the desk reads it for the trigger and then sources the real number
    /// from the player's career record.
    fn file_life_events(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        if feed.happened(HappinessEventType::SeniorDebut) {
            out.push(
                NewsStory::new(NewsStoryKind::YouthDebut, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0),
            );
        }

        if feed.happened(HappinessEventType::InjuryReturn) && !player.player_attributes.is_injured {
            out.push(NewsStory::new(NewsStoryKind::InjuryReturn, date).about(player.id));
        }

        if feed.happened(HappinessEventType::AppearanceMilestone) {
            let apps = CareerRecord::appearances(player);
            let milestone = apps - (apps % Self::APPS_STEP);
            if milestone >= Self::APPS_STEP {
                out.push(
                    NewsStory::new(NewsStoryKind::MilestoneApps, date)
                        .about(player.id)
                        .with_numbers(milestone, 0)
                        .weighted(milestone / 10),
                );
            }
        }

        if feed.happened(HappinessEventType::GoalMilestone) {
            let goals = CareerRecord::goals(player);
            let milestone = goals - (goals % Self::GOALS_STEP);
            if milestone >= Self::GOALS_STEP {
                out.push(
                    NewsStory::new(NewsStoryKind::MilestoneGoals, date)
                        .about(player.id)
                        .with_numbers(milestone, 0)
                        .weighted(milestone / 2),
                );
            }
        }

        if feed.happened(HappinessEventType::ClubServantMilestone) {
            let seasons = CareerRecord::seasons_at_current_club(player).max(Self::SERVANT_SEASONS);
            out.push(
                NewsStory::new(NewsStoryKind::ClubServant, date)
                    .about(player.id)
                    .with_numbers(seasons, CareerRecord::appearances(player))
                    .weighted(seasons * 8),
            );
        }

        if feed.happened(HappinessEventType::RetirementAnnounced) {
            out.push(
                NewsStory::new(NewsStoryKind::RetirementAnnounced, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, CareerRecord::appearances(player))
                    .weighted(PlayerStanding::importance(player)),
            );
        }

        if feed
            .any_of(&[
                HappinessEventType::NationalTeamDebut,
                HappinessEventType::NationalTeamCallup,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::NationalCallUp, date)
                    .about(player.id)
                    .with_numbers(player.player_attributes.international_apps as i32, 0),
            );
        }

        if feed.happened(HappinessEventType::CaptaincyAwarded) {
            out.push(NewsStory::new(NewsStoryKind::CaptainNamed, date).about(player.id));
        }

        // The armband going the other way. A paper that only ever
        // reports the naming of a captain is reporting half of the one
        // job in a dressing room everybody has an opinion about.
        if feed.happened(HappinessEventType::CaptaincyRemoved) {
            out.push(
                NewsStory::new(NewsStoryKind::CaptaincyLost, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
        }

        // Whether there is another season in him. The announcement is
        // its own story; this is the months of speculation before it,
        // which is what a town actually spends the spring talking about.
        if feed.happened(HappinessEventType::RetirementConsidering)
            && !feed.happened(HappinessEventType::RetirementAnnounced)
        {
            out.push(
                NewsStory::new(NewsStoryKind::CareerTwilight, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, CareerRecord::appearances(player))
                    .weighted(PlayerStanding::importance(player) / 2),
            );
        }

        if feed.happened(HappinessEventType::ScoredAgainstFormerClub) {
            out.push(
                NewsStory::new(NewsStoryKind::FormerClubGoal, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 3),
            );
        }

        // The goal every signing is asked about at every press
        // conference until he scores it. Distinct from the milestone
        // beats: this one is not a round number, it is the first.
        if feed.happened(HappinessEventType::FirstClubGoal) {
            out.push(
                NewsStory::new(NewsStoryKind::FirstClubGoal, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 3),
            );
        }

        // Finished with his country, carrying on at his club. A career
        // decision with a date on it, and the one retirement a player
        // gets to make twice.
        if feed.happened(HappinessEventType::InternationalRetirement) {
            out.push(
                NewsStory::new(NewsStoryKind::InternationalRetirement, date)
                    .about(player.id)
                    .with_numbers(
                        player.player_attributes.international_apps as i32,
                        player.age(date) as i32,
                    )
                    .weighted(player.player_attributes.international_apps as i32),
            );
        }

        // The returnee: a spell away has ended and he is back in the
        // building. Which of the four ways it ended decides how loudly
        // the paper says so.
        if let Some(homecoming) = feed.any_of(&[
            HappinessEventType::BackedAfterLoanReturn,
            HappinessEventType::ReturnedFromLoanProven,
            HappinessEventType::UnsettledAfterLoanReturn,
            HappinessEventType::ReturnedFromLoanDeflated,
        ]) {
            let weight = match homecoming {
                HappinessEventType::BackedAfterLoanReturn => 60,
                HappinessEventType::ReturnedFromLoanProven => 40,
                HappinessEventType::UnsettledAfterLoanReturn => 10,
                _ => 0,
            };
            out.push(
                NewsStory::new(NewsStoryKind::LoanReturn, date)
                    .about(player.id)
                    .with_numbers(
                        player.statistics.played as i32 + player.statistics.played_subs as i32,
                        player.statistics.goals as i32,
                    )
                    .weighted(weight),
            );
        }
    }

    fn file_form(
        out: &mut Vec<NewsStory>,
        player: &Player,
        played_this_week: bool,
        date: NaiveDate,
    ) {
        if !played_this_week {
            return;
        }

        let stats = &player.statistics;
        let apps = stats.played as i32 + stats.played_subs as i32;
        if apps < 6 {
            return;
        }

        let group = player.position().position_group();
        let rating = stats.average_rating_realistic(group);

        if matches!(group, PlayerFieldPositionGroup::Goalkeeper) {
            // A goalkeeper's season is measured in shut-outs, not ratings.
            let clean_sheets = stats.clean_sheets as i32;
            if clean_sheets >= 4 && rating >= 6.9 {
                out.push(
                    NewsStory::new(NewsStoryKind::KeeperWall, date)
                        .about(player.id)
                        .with_numbers(clean_sheets, (rating * 100.0) as i32)
                        .weighted(clean_sheets * 12),
                );
            }
            return;
        }

        // A nineteen-year-old holding down a senior place gets the
        // breakthrough piece, not the same "carrying the team" line the
        // paper runs about a 29-year-old. `file_breakthrough` owns him.
        if player.age(date) <= Self::PROSPECT_AGE {
            return;
        }

        if rating >= 7.20 {
            let goals = stats.goals as i32;
            out.push(
                NewsStory::new(NewsStoryKind::StarForm, date)
                    .about(player.id)
                    .with_numbers(goals, (rating * 100.0) as i32)
                    .weighted(((rating - 7.20) * 200.0) as i32 + goals * 8),
            );
        }
    }

    /// The kid who has arrived. Two ways in: the club's own machinery
    /// has already marked the breakthrough, or the numbers speak for
    /// themselves — a real run of senior football at an age when most
    /// of his year group are still in the youth team.
    ///
    /// Deliberately not gated on ability or potential. Clubs cannot read
    /// a player's ceiling and neither can a newspaper; what a reporter
    /// has is his age, his minutes and his ratings.
    fn file_breakthrough(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        let age = player.age(date);
        if age > Self::PROSPECT_AGE {
            return;
        }

        let stats = &player.statistics;
        let apps = stats.played as i32 + stats.played_subs as i32;
        let rating = stats.average_rating_realistic(player.position().position_group());

        // The piece is built on a rating, so there has to be one. A
        // fifteen-year-old moved onto the senior roster fires the club's
        // own breakthrough event with no minutes behind it at all, and
        // `average_rating_realistic` answers 0.0 for a player who has
        // never been rated — which is how "a season average of 0.00"
        // reached a front page. Being promoted is `YouthDebut`'s story;
        // this one needs a record.
        if rating <= 0.0 || apps < Self::BREAKTHROUGH_MIN_APPS {
            return;
        }

        // The club marking the breakthrough is evidence in itself, so it
        // lowers the bar — but only the form bar, never the "has he
        // actually played" one.
        let announced = feed.happened(HappinessEventType::YouthBreakthrough);
        let earned = apps >= 6 && stats.played as i32 >= 3 && rating >= Self::RISING_STAR_RATING;

        if !announced && !earned {
            return;
        }

        // The younger he is and the better he has played, the louder it
        // runs — a seventeen-year-old at 7.4 is a back-page lead, a
        // twenty-one-year-old at 7.0 is a column.
        let youth_bonus = (Self::PROSPECT_AGE as i32 - age as i32) * 18;
        let form_bonus = ((rating - Self::RISING_STAR_RATING) * 120.0).max(0.0) as i32;

        out.push(
            NewsStory::new(NewsStoryKind::RisingStar, date)
                .about(player.id)
                .with_numbers(age as i32, (rating * 100.0) as i32)
                .weighted(youth_bonus + form_bonus + if announced { 40 } else { 0 }),
        );
    }

    /// Bans, and the goals that will not come. Both are selection
    /// problems the manager has to answer for, which is what makes them
    /// print.
    fn file_discipline(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        if feed.happened(HappinessEventType::BannedThroughAccumulation) {
            out.push(
                NewsStory::new(NewsStoryKind::Suspension, date)
                    .about(player.id)
                    .with_numbers(player.statistics.yellow_cards as i32, 0)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
        }

        if feed.happened(HappinessEventType::ScoringDroughtConcern) {
            out.push(
                NewsStory::new(NewsStoryKind::GoalDrought, date)
                    .about(player.id)
                    .with_numbers(
                        player.statistics.goals as i32,
                        player.statistics.played as i32 + player.statistics.played_subs as i32,
                    )
                    .weighted(PlayerStanding::importance(player) / 3),
            );
        }

        if feed.happened(HappinessEventType::GoalDroughtEnded) {
            out.push(
                NewsStory::new(NewsStoryKind::DroughtEnded, date)
                    .about(player.id)
                    .with_numbers(player.statistics.goals as i32, 0),
            );
        }
    }

    /// What the training ground and the terraces are saying. Colour
    /// pieces, deliberately low-priority: they fill the briefs column on
    /// a quiet week and never crowd out real football.
    fn file_dressing_room(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        if feed
            .any_of(&[
                HappinessEventType::TrainingGroundBustUp,
                HappinessEventType::PublicApology,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::TrainingBustUp, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
            return;
        }

        // A voice where there was not one. The dressing room's quiet
        // hierarchy changing is a story a paper only ever gets to write
        // about the armband — this is the version that happens first,
        // and it is how a club finds its next captain.
        if feed.happened(HappinessEventType::LeadershipEmergence) {
            out.push(
                NewsStory::new(NewsStoryKind::LeaderEmerging, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0)
                    .weighted(PlayerStanding::importance(player) / 3),
            );
            return;
        }

        // Somebody he was close to has gone. The transfer itself was
        // reported as the buying club's business; this is what it did to
        // the man left behind, which is the half a local paper cares
        // about.
        if feed
            .any_of(&[
                HappinessEventType::CloseFriendSold,
                HappinessEventType::MentorDeparted,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::TeammateFarewell, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 3),
            );
        }
    }

    /// The slower half of a foreign player's life: the language coming,
    /// somebody he can speak to walking through the door, and — at the
    /// other end of a career — the first look at the coaching badges.
    ///
    /// Both are conditions rather than days, so both read the longer
    /// window and are `Standing`: settling in is not something that
    /// happened on a Tuesday.
    fn file_career_life(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate) {
        let feed = RecentEvents::fortnight(player);

        if feed
            .any_of(&[
                HappinessEventType::CompatriotJoined,
                HappinessEventType::LanguageProgress,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::SettlingIn, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 4),
            );
        }

        if feed.happened(HappinessEventType::CoachingCareerInterest) {
            out.push(
                NewsStory::new(NewsStoryKind::CoachingAmbition, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0)
                    .weighted(PlayerStanding::importance(player) / 4),
            );
        }
    }

    /// Who is after whose shirt. Competition for places is the thing a
    /// dressing room is actually made of, and until now the paper only
    /// ever saw its outcome on a team sheet — never the argument.
    ///
    /// Two versions, and the sourer one wins: the club's own kid kept
    /// out by a borrowed player of the same level (the grievance a local
    /// readership takes personally, and the reason a homegrown core
    /// mutters about the message it sends), or the incumbent who can
    /// feel somebody coming for his place.
    fn file_competition_for_places(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        if feed
            .any_of(&[
                HappinessEventType::PathwayBlockedByLoanSigning,
                HappinessEventType::UnhappyAboutBlockedHomegrown,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::PathwayBlocked, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
            return;
        }

        if feed
            .any_of(&[
                HappinessEventType::ThreatenedByReturningLoanee,
                HappinessEventType::ThreatenedByNewSigning,
                HappinessEventType::LostStartingPlace,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::ShirtUnderThreat, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
            return;
        }

        // The winning half of the same argument. The paper has always
        // run "he has lost his place" and never "he has taken one",
        // which reads as though shirts are only ever surrendered.
        if feed.happened(HappinessEventType::WonStartingPlace) {
            out.push(
                NewsStory::new(NewsStoryKind::WonStartingPlace, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0)
                    .weighted(PlayerStanding::importance(player) / 3),
            );
        }
    }

    /// The contract column: a deal signed, and a deal running out.
    fn file_contract(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        let Some(contract) = player.contract.as_ref() else {
            return;
        };
        if player.is_on_loan() {
            return;
        }

        let importance = PlayerStanding::importance(player);

        if feed.happened(HappinessEventType::ContractRenewal) {
            let years = ((contract.expiration - date).num_days() as f32 / 365.0)
                .round()
                .max(1.0) as i32;
            out.push(
                NewsStory::new(NewsStoryKind::ContractRenewed, date)
                    .about(player.id)
                    .with_numbers(years, 0)
                    .with_money(contract.salary as i64)
                    .weighted(importance),
            );
            return;
        }

        let days_left = (contract.expiration - date).num_days();
        if !(0..=210).contains(&days_left) {
            return;
        }

        // Only a player the club would miss makes this a story.
        if importance < 30 {
            return;
        }

        let months_left = ((days_left as f32) / 30.0).round().max(1.0) as i32;
        out.push(
            NewsStory::new(NewsStoryKind::ContractStandoff, date)
                .about(player.id)
                .with_numbers(months_left, 0)
                .weighted(importance - (months_left * 6)),
        );
    }
}
