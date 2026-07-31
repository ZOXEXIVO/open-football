use super::facts::{PlayerStanding, RecentEvents};
use crate::club::news::types::{NewsStory, NewsStoryKind};
use crate::{HappinessEventType, Player, PlayerStatusType};
use chrono::NaiveDate;

/// The rumour mill: everything printed about a player's future that has
/// not actually happened yet.
///
/// A local paper runs at most two lines per player per week — one about
/// where he stands with his own club, one about who is sniffing around —
/// and the desk is deliberately built that way. Emitting every event the
/// simulation recorded would bury the football under speculation, which
/// is the failure mode that makes a generated page read as filler.
pub struct RumourDesk;

impl RumourDesk {
    /// Freshly listed is news; listed since last spring is not.
    const LISTING_IS_NEWS_FOR_DAYS: i64 = 28;
    /// Same for a transfer request. `Req` is a status, not an event —
    /// it sits on the player until the club resolves it, which can be
    /// months. "{player} asks to leave" is a headline about something
    /// happening, so it has to expire, or the paper reports the asking
    /// again every time it looks at him.
    const REQUEST_IS_NEWS_FOR_DAYS: i64 = 21;

    pub fn file_player(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate) {
        Self::file_player_over(out, player, date, RecentEvents::FORTNIGHT);
    }

    /// The same desk run on a different press cycle.
    ///
    /// The club paper comes out weekly and reads a fortnight back; the
    /// division's own paper comes out monthly and has to read the whole
    /// month, or half of every month's transfer talk is printed nowhere.
    /// The detectors and their running order are identical — a rumour
    /// column that ranked its stories differently depending on which
    /// paper it was filed to would be two columns pretending to be one.
    pub fn file_player_over(
        out: &mut Vec<NewsStory>,
        player: &Player,
        date: NaiveDate,
        window_days: u16,
    ) {
        if player.is_on_loan() {
            // A borrowed player's future is his parent club's story, and
            // it is filed by the loan desk on that club's page.
            return;
        }

        Self::file_standing_at_club(out, player, date, window_days);
        Self::file_interest(out, player, date, window_days);
    }

    /// Where the player stands with his own club: has he asked to go,
    /// has he been told to go, is he on the list, have talks broken
    /// down. One line, the sharpest version that applies.
    fn file_standing_at_club(
        out: &mut Vec<NewsStory>,
        player: &Player,
        date: NaiveDate,
        window_days: u16,
    ) {
        let feed = RecentEvents::within(player, window_days);
        let importance = PlayerStanding::importance(player);
        let statuses = &player.statuses;

        // The request itself is dated by the feed; the `Req` status only
        // proves one is outstanding, so it counts as news while it is
        // still recent. A status held since the winter is the reason
        // the paper writes about him at all, not the story.
        let just_asked = feed.happened(HappinessEventType::AskedClubToArrangeTransfer)
            || statuses
                .held_for_days(PlayerStatusType::Req, date)
                .is_some_and(|days| days <= Self::REQUEST_IS_NEWS_FOR_DAYS);

        if just_asked {
            out.push(
                NewsStory::new(NewsStoryKind::TransferRequestFiled, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        // The window shut with him still on the list: a club that wants
        // him gone, a player nobody came for, and months of both of them
        // living with it. The quietest kind of bad news, and the sort a
        // supporter works out for himself from the team sheet anyway.
        if feed.happened(HappinessEventType::UnsoldWindowClosed) {
            out.push(
                NewsStory::new(NewsStoryKind::UnsoldStillHere, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
            return;
        }

        // He still wants out, but the asking is no longer the news. The
        // standing version of the story runs instead, and much quieter.
        if statuses.has(PlayerStatusType::Req) {
            out.push(
                NewsStory::new(NewsStoryKind::TransferListed, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
            return;
        }

        if feed.happened(HappinessEventType::ToldNotInPlans) {
            out.push(
                NewsStory::new(NewsStoryKind::ToldNotInPlans, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        if statuses.has(PlayerStatusType::Lst) {
            if statuses
                .held_for_days(PlayerStatusType::Lst, date)
                .is_some_and(|days| days <= Self::LISTING_IS_NEWS_FOR_DAYS)
            {
                out.push(
                    NewsStory::new(NewsStoryKind::TransferListed, date)
                        .about(player.id)
                        .weighted(importance),
                );
            }
            return;
        }

        // He has not asked to go anywhere, and nobody has told him to —
        // he simply wants more than the club is currently able to put in
        // front of him. The stage before a transfer request, and the one
        // a manager still has a chance to answer.
        if feed
            .any_of(&[
                HappinessEventType::WantsToLeaveAfterRelegation,
                HappinessEventType::WantsTitleChallenge,
                HappinessEventType::WantsStrongerLeague,
                HappinessEventType::WantsStrongerSquad,
                HappinessEventType::ConcernedByClubDirection,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::AmbitionWarning, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        // The same complaint with an address on it. "He wants more" is
        // the general version; "he wants European nights" names the
        // thing the club either can or cannot provide, and a supporter
        // knows immediately which of the two his club is.
        if feed
            .any_of(&[
                HappinessEventType::WantsEuropeanCompetition,
                HappinessEventType::WantsCopaLibertadores,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::ContinentalAmbition, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        // Interest from elsewhere, cashed in here. The most cynical
        // thing in football, done constantly, and never once printed.
        if feed.happened(HappinessEventType::UsedInterestForContractLeverage) {
            out.push(
                NewsStory::new(NewsStoryKind::LeverageUsed, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        if feed
            .any_of(&[
                HappinessEventType::ContractTalksStalled,
                HappinessEventType::RejectedContractOffer,
                HappinessEventType::ReleaseClauseDemanded,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::ContractTalksStalled, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        // Final year and nothing from upstairs — no offer, no stalled
        // negotiation, just silence and a calendar. Quieter than talks
        // breaking down, which is why it only runs when they haven't.
        // The copy quotes the months remaining, so a player somehow
        // carrying the mood without a contract cannot be written up.
        if feed.happened(HappinessEventType::ContractExpiryAnxiety) {
            if let Some(contract) = player.contract.as_ref() {
                let months_left = ((contract.expiration - date).num_days() as f32 / 30.0)
                    .round()
                    .max(1.0) as i32;
                out.push(
                    NewsStory::new(NewsStoryKind::ContractRunningDown, date)
                        .about(player.id)
                        .with_numbers(months_left, 0)
                        .weighted(importance),
                );
            }
            return;
        }

        // He has answered the speculation himself. The rarest line in
        // the rumour column, and the only positive one it ever carries.
        if feed.happened(HappinessEventType::TransferInterestDismissed) {
            out.push(
                NewsStory::new(NewsStoryKind::CommitsToClub, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
        }
    }

    /// Who wants him. Ordered by how far down the funnel the interest
    /// has travelled, because that is exactly how a back page ranks it:
    /// a rejected bid is a headline, a scout in the stand is a sentence.
    ///
    /// Every rung down to the agent's briefing names the club doing the
    /// chasing, and the copy for those kinds is written around the name.
    /// So they are read through `suitor`, which yields the beat only
    /// when the paper can say whose interest it was: a rung the
    /// simulation recorded anonymously is not a quieter version of the
    /// same story, it is a mood the player is carrying and no approach
    /// at all. Those fall through to the generic speculation line at the
    /// bottom, which was written for exactly that — his name keeps
    /// coming up, and the paper is not saying whose idea it was.
    fn file_interest(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate, window_days: u16) {
        let feed = RecentEvents::within(player, window_days);
        let importance = PlayerStanding::importance(player);

        // The move that did not happen. It outranks every live link on
        // the page: a deal that collapsed at the last stage is the only
        // transfer story with an ending, and the one the player is still
        // carrying around the training ground.
        if let Some(club) = feed.suitor(HappinessEventType::DreamMoveCollapsed) {
            out.push(
                NewsStory::new(NewsStoryKind::MoveCollapsed, date)
                    .about(player.id)
                    .against(club)
                    .weighted(importance),
            );
            return;
        }

        // A concrete bid the club turned down.
        if let Some((beat, club)) = feed.suitor_of(&[
            HappinessEventType::MoveVetoedByClub,
            HappinessEventType::TransferBidRejected,
        ]) {
            let vetoed = beat == HappinessEventType::MoveVetoedByClub;
            out.push(
                NewsStory::new(NewsStoryKind::BidRejected, date)
                    .about(player.id)
                    .against(club)
                    .weighted(importance + if vetoed { 60 } else { 0 }),
            );
            return;
        }

        if let Some(club) = feed.suitor(HappinessEventType::TransferTalksExpected) {
            out.push(
                NewsStory::new(NewsStoryKind::TalksExpected, date)
                    .about(player.id)
                    .against(club)
                    .weighted(importance),
            );
            return;
        }

        // The neighbours. Always the loudest version of an interest
        // story, whatever the size of the club involved.
        if let Some(club) = feed.suitor(HappinessEventType::InterestFromRival) {
            out.push(
                NewsStory::new(NewsStoryKind::RumourRival, date)
                    .about(player.id)
                    .against(club)
                    .weighted(importance),
            );
            return;
        }

        // `TopClubOpportunity` is deliberately not on this list. It
        // fires when a player has just WALKED INTO a big club, not when
        // one has come asking — it carries no suitor because there is
        // none, and reading it as interest printed "another club join
        // the queue" about a man nobody had enquired for.
        if let Some((_, club)) = feed.suitor_of(&[
            HappinessEventType::InterestFromBiggerClub,
            HappinessEventType::WantedByBiggerClub,
            HappinessEventType::FavoriteClubInterest,
        ]) {
            out.push(
                NewsStory::new(NewsStoryKind::RumourInterest, date)
                    .about(player.id)
                    .against(club)
                    .weighted(importance),
            );
            return;
        }

        // `HomeReturnOpportunity` is off this list for the same reason:
        // it is the player wanting to go home, not a club at home
        // wanting him, and a homecoming story with no home in it is not
        // a story.
        if let Some((_, club)) = feed.suitor_of(&[
            HappinessEventType::HomecomingRumour,
            HappinessEventType::FormerClubInterest,
        ]) {
            out.push(
                NewsStory::new(NewsStoryKind::HomecomingLink, date)
                    .about(player.id)
                    .against(club)
                    .weighted(importance / 2),
            );
            return;
        }

        // The agent's briefing names nobody by design — he is touting,
        // not reporting an approach — so this rung reads the beat
        // directly and passes on whatever club the feed happens to hold.
        if feed.happened(HappinessEventType::AgentStirsInterest) {
            out.push(
                NewsStory::new(NewsStoryKind::AgentTouting, date)
                    .about(player.id)
                    .against(feed.interested_club(HappinessEventType::AgentStirsInterest))
                    .weighted(importance / 2),
            );
            return;
        }

        // The man in the stand belongs to somebody. Every phrasing of
        // this one says whose notebook it was, so an unattributed
        // watching brief is not printed at all.
        if let Some(club) = feed.suitor(HappinessEventType::ScoutedByClub) {
            out.push(
                NewsStory::new(NewsStoryKind::ScoutsWatching, date)
                    .about(player.id)
                    .against(club)
                    .weighted(importance / 2),
            );
            return;
        }

        // The link that died. A paper that never closes its own stories
        // reads as noise, so the ending gets a line of its own — quiet,
        // and only when no live interest outranks it.
        if feed.happened(HappinessEventType::InterestCooled) {
            out.push(
                NewsStory::new(NewsStoryKind::RumourCools, date)
                    .about(player.id)
                    .against(feed.interested_club(HappinessEventType::InterestCooled))
                    .weighted(importance / 3),
            );
            return;
        }

        Self::file_status_speculation(out, player, date, importance, window_days);
    }

    /// The fallback: no event fired this fortnight, but the player is
    /// carrying a market status. The further down the funnel the status
    /// sits, the louder the paper runs it.
    fn file_status_speculation(
        out: &mut Vec<NewsStory>,
        player: &Player,
        date: NaiveDate,
        importance: i32,
        window_days: u16,
    ) {
        let statuses = &player.statuses;

        // `Trn` means the clubs have shaken hands: the deal is agreed
        // and only the formalities remain. That is not speculation any
        // more, and the paper stops hedging.
        if statuses.has(PlayerStatusType::Trn) {
            out.push(
                NewsStory::new(NewsStoryKind::TransferAgreed, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        let heat = if statuses.has(PlayerStatusType::Bid) {
            120
        } else if statuses.has(PlayerStatusType::Enq) {
            60
        } else if statuses.has(PlayerStatusType::Wnt)
            || statuses.has(PlayerStatusType::Sct)
            || Self::name_keeps_coming_up(player, window_days)
        {
            0
        } else {
            return;
        };

        out.push(
            NewsStory::new(NewsStoryKind::TransferSpeculation, date)
                .about(player.id)
                .weighted(heat + importance),
        );
    }

    /// The generic "his name keeps appearing" beat, with no club
    /// attached — the shape most transfer talk actually takes in print.
    fn name_keeps_coming_up(player: &Player, window_days: u16) -> bool {
        RecentEvents::within(player, window_days)
            .any_of(&[
                HappinessEventType::TransferRumour,
                HappinessEventType::TransferSpeculationDistracts,
                HappinessEventType::FansReactToTransferRumour,
            ])
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::RumourDesk;
    use crate::club::news::types::{NewsStory, NewsStoryKind};
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::club::player::happiness::{
        HappinessEventCause, HappinessEventContext, HappinessEventScope, HappinessEventSeverity,
        TransferInterestContext, TransferInterestKind, TransferInterestReaction,
        TransferInterestSource, TransferInterestStage,
    };
    use crate::shared::fullname::FullName;
    use crate::{
        HappinessEventType, PersonAttributes, Player, PlayerAttributes, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills,
    };
    use chrono::NaiveDate;

    struct Mill;

    impl Mill {
        /// The club doing the chasing in every fixture that has one.
        const SUITOR: u32 = 44;

        fn day() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 3, 2).unwrap()
        }

        /// A squad player carrying the listed moods, each attributed to
        /// a club the way the transfer pipeline attributes them.
        ///
        /// The attribution is not decoration in a fixture either: the
        /// desk will not print an approach it cannot put a name to, so a
        /// context-free mood here would be testing the anonymous path
        /// while claiming to test the ranking. That path has its own
        /// test — `an_approach_with_nobody_behind_it_is_not_printed`.
        fn player(moods: &[HappinessEventType]) -> Player {
            Self::build(moods, true)
        }

        /// The same player, with every mood recorded the way a desk that
        /// never knew who was asking records it.
        fn anonymous_player(moods: &[HappinessEventType]) -> Player {
            Self::build(moods, false)
        }

        fn build(moods: &[HappinessEventType], attributed: bool) -> Player {
            let mut player = PlayerBuilder::new()
                .id(7)
                .full_name(FullName::new("R".to_string(), "Mour".to_string()))
                .birth_date(NaiveDate::from_ymd_opt(1999, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 15,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap();
            for mood in moods {
                if attributed {
                    player.happiness.add_event_with_context(
                        mood.clone(),
                        -3.0,
                        None,
                        Self::approach_from(Self::SUITOR),
                    );
                } else {
                    player.happiness.add_event(mood.clone(), -3.0);
                }
            }
            player
        }

        /// The context the transfer pipeline hangs on an interest beat.
        fn approach_from(club_id: u32) -> HappinessEventContext {
            HappinessEventContext::new(
                HappinessEventCause::MediaPressure,
                HappinessEventSeverity::Moderate,
                HappinessEventScope::Media,
            )
            .with_transfer_interest_context(
                TransferInterestContext::new(
                    TransferInterestStage::ConcreteInterest,
                    TransferInterestSource::NationalPress,
                    TransferInterestKind::StepUp,
                    TransferInterestReaction::Flattered,
                )
                .with_interested_club(club_id),
            )
        }

        fn kinds(moods: &[HappinessEventType]) -> Vec<NewsStoryKind> {
            Self::kinds_of(&Self::player(moods))
        }

        fn kinds_of(player: &Player) -> Vec<NewsStoryKind> {
            let mut out: Vec<NewsStory> = Vec::new();
            RumourDesk::file_player(&mut out, player, Self::day());
            out.iter().map(|story| story.kind).collect()
        }
    }

    /// A player who has told the club it is not ambitious enough has
    /// said something; a player whose renewal has stalled has merely
    /// failed to agree terms. The paper leads on the first.
    #[test]
    fn wanting_more_than_the_club_offers_outranks_a_stalled_renewal() {
        let kinds = Mill::kinds(&[
            HappinessEventType::ContractTalksStalled,
            HappinessEventType::WantsTitleChallenge,
        ]);

        assert!(kinds.contains(&NewsStoryKind::AmbitionWarning));
        assert!(
            !kinds.contains(&NewsStoryKind::ContractTalksStalled),
            "one line per player about where he stands: {:?}",
            kinds
        );
    }

    /// The only transfer story with an ending beats every live link.
    #[test]
    fn a_collapsed_move_outranks_the_interest_that_is_still_alive() {
        let kinds = Mill::kinds(&[
            HappinessEventType::InterestFromBiggerClub,
            HappinessEventType::DreamMoveCollapsed,
        ]);

        assert!(kinds.contains(&NewsStoryKind::MoveCollapsed));
        assert!(!kinds.contains(&NewsStoryKind::RumourInterest));
    }

    /// Every phrasing of an interest story names the club doing the
    /// chasing, so a beat that arrived without one is not that story.
    /// The paper drops to the anonymous speculation line instead of
    /// printing "another club watching him", which names nobody while
    /// pretending to.
    #[test]
    fn an_approach_with_nobody_behind_it_is_not_printed() {
        let player = Mill::anonymous_player(&[
            HappinessEventType::InterestFromBiggerClub,
            HappinessEventType::TransferRumour,
        ]);
        let kinds = Mill::kinds_of(&player);

        assert!(
            !kinds.contains(&NewsStoryKind::RumourInterest),
            "an unattributed approach reached the page: {:?}",
            kinds
        );
        assert!(
            kinds.contains(&NewsStoryKind::TransferSpeculation),
            "the anonymous line is what carries it instead: {:?}",
            kinds
        );
    }

    /// A named suitor further down the ladder beats an anonymous rung
    /// above it. A rejected bid the paper can attribute is a story; a
    /// veto it cannot is a sentence about nobody.
    #[test]
    fn a_named_suitor_outranks_an_anonymous_rung_above_it() {
        let mut player = Mill::anonymous_player(&[HappinessEventType::MoveVetoedByClub]);
        player.happiness.add_event_with_context(
            HappinessEventType::TransferBidRejected,
            -3.0,
            None,
            Mill::approach_from(Mill::SUITOR),
        );

        let mut out: Vec<NewsStory> = Vec::new();
        RumourDesk::file_player(&mut out, &player, Mill::day());

        let bid = out
            .iter()
            .find(|story| story.kind == NewsStoryKind::BidRejected)
            .expect("the attributable rejection is the story");
        assert_eq!(bid.other_id, Mill::SUITOR);
    }

    /// Walking into a big club is not the same as one coming for you.
    /// `TopClubOpportunity` fires on arrival and names nobody, and
    /// reading it as an approach is what put "another club join the
    /// queue" on a page about a player nobody had enquired for.
    #[test]
    fn arriving_at_a_big_club_is_not_transfer_interest() {
        let kinds = Mill::kinds(&[HappinessEventType::TopClubOpportunity]);

        assert!(
            !kinds.contains(&NewsStoryKind::RumourInterest),
            "an arrival was printed as an approach: {:?}",
            kinds
        );
    }

    /// A window that shut on an unsold player is its own quiet story,
    /// and it replaces the standing "he is on the list" line rather
    /// than running alongside it.
    #[test]
    fn an_unsold_listing_reports_the_window_rather_than_the_listing() {
        let kinds = Mill::kinds(&[HappinessEventType::UnsoldWindowClosed]);

        assert_eq!(kinds, vec![NewsStoryKind::UnsoldStillHere]);
    }

    /// `Trn` means the clubs have shaken hands. That is a done deal in
    /// all but paperwork, and the paper stops calling it speculation.
    #[test]
    fn an_agreed_deal_stops_being_speculation() {
        use crate::PlayerStatusType;

        let mut player = Mill::player(&[]);
        player.statuses.add(Mill::day(), PlayerStatusType::Trn);

        let mut out: Vec<NewsStory> = Vec::new();
        RumourDesk::file_player(&mut out, &player, Mill::day());
        let kinds: Vec<NewsStoryKind> = out.iter().map(|story| story.kind).collect();

        assert!(kinds.contains(&NewsStoryKind::TransferAgreed));
        assert!(!kinds.contains(&NewsStoryKind::TransferSpeculation));
    }

    /// The mill needs endings as much as beginnings: interest that
    /// cooled closes the story, and a player who publicly commits gets
    /// the one positive line the rumour column ever prints.
    #[test]
    fn the_mill_prints_its_own_endings() {
        assert_eq!(
            Mill::kinds(&[HappinessEventType::InterestCooled]),
            vec![NewsStoryKind::RumourCools]
        );
        assert_eq!(
            Mill::kinds(&[HappinessEventType::TransferInterestDismissed]),
            vec![NewsStoryKind::CommitsToClub]
        );
    }

    /// Live interest always outranks the obituary of a dead link.
    #[test]
    fn a_live_link_outranks_a_cooled_one() {
        let kinds = Mill::kinds(&[
            HappinessEventType::InterestCooled,
            HappinessEventType::InterestFromBiggerClub,
        ]);

        assert!(kinds.contains(&NewsStoryKind::RumourInterest));
        assert!(!kinds.contains(&NewsStoryKind::RumourCools));
    }

    /// Final-year silence is a story about a contract, so it needs one
    /// behind it: with a contract the months are quoted, without one
    /// the desk stays quiet rather than print "0 months".
    #[test]
    fn contract_silence_needs_a_contract_behind_it() {
        use crate::PlayerClubContract;

        assert!(Mill::kinds(&[HappinessEventType::ContractExpiryAnxiety]).is_empty());

        let mut player = Mill::player(&[HappinessEventType::ContractExpiryAnxiety]);
        player.contract = Some(PlayerClubContract::new(
            10_000,
            NaiveDate::from_ymd_opt(2026, 11, 30).unwrap(),
        ));

        let mut out: Vec<NewsStory> = Vec::new();
        RumourDesk::file_player(&mut out, &player, Mill::day());

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, NewsStoryKind::ContractRunningDown);
        assert_eq!(
            out[0].a, 9,
            "November from March is nine months on the clock"
        );
    }
}
