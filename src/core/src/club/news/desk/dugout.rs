use super::facts::{PlayerStanding, RecentEvents, SquadPulse};
use crate::club::news::types::{NewsStory, NewsStoryKind};
use crate::{HappinessEventType, Player};
use chrono::NaiveDate;

/// The manager and his players.
///
/// Two thirds of what a football paper actually prints is somebody's
/// relationship with the man picking the team: who he backed, who he
/// left out, who was fined, and who has asked for a word in private.
/// The simulation records all of it on the players themselves — this
/// desk is the part that decides which of it a reader would care about.
pub struct DugoutDesk;

impl DugoutDesk {
    /// Per-player beats, filed from the squad walk.
    ///
    /// At most one line per player: a paper that ran "manager praises
    /// X" and "manager criticises X" in the same edition would be
    /// reporting the noise rather than the relationship. The sourest
    /// version that applies wins, because that is the one a reader
    /// remembers.
    pub fn file_player(
        out: &mut Vec<NewsStory>,
        player: &Player,
        pulse: &mut SquadPulse,
        date: NaiveDate,
    ) {
        let feed = RecentEvents::week(player);
        let importance = PlayerStanding::importance(player);

        // Squad-wide beats are tallied whether or not this player's own
        // line makes the page.
        pulse.inquest |= feed.happened(HappinessEventType::DressingRoomInquest);
        pulse.rallied |= feed
            .any_of(&[
                HappinessEventType::ResiliencePraised,
                HappinessEventType::RalliesBehindManager,
            ])
            .is_some();
        pulse.investment |= feed.happened(HappinessEventType::EncouragedBySquadInvestment);
        // A manager change is NOT read from here any more. The squad's
        // `ManagerDeparture` / `NewManagerBounce` pair fires on every
        // move of the head-coach seat — including the caretaker stepping
        // up — so it fired twice per sacking and could not say whether
        // the man had been dismissed, poached or promoted. The club's
        // own dated diary carries all of that; see
        // [`crate::club::news::affairs`].
        pulse.trophy |= feed
            .any_of(&[
                HappinessEventType::TrophyWon,
                HappinessEventType::DomesticCupWon,
            ])
            .is_some();
        // The season's verdicts, each its own front page rather than a
        // shared "trophy" stamp: going up, going down, a final lost,
        // continental football secured.
        pulse.promotion |= feed.happened(HappinessEventType::PromotionCelebration);
        pulse.relegated |= feed.happened(HappinessEventType::Relegated);
        pulse.cup_final_lost |= feed.happened(HappinessEventType::CupFinalDefeat);
        pulse.europe |= feed.happened(HappinessEventType::QualifiedForEurope);

        // A promise the club has gone back on. The loudest thing a
        // player can say about his manager, and the only one of these
        // the paper sets as a quote.
        if feed.happened(HappinessEventType::PromiseBroken) {
            out.push(
                NewsStory::new(NewsStoryKind::PromiseBroken, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        // Money off his wages is a club decision with a paper trail —
        // it outranks a public word either way.
        if feed
            .any_of(&[
                HappinessEventType::FinedByClub,
                HappinessEventType::FormalWarningIssued,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::PlayerFined, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        // Left out of the one that mattered.
        if feed
            .any_of(&[
                HappinessEventType::BenchedForBigMatch,
                HappinessEventType::FeelsSelectionFavouritism,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::DroppedForBigMatch, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        if feed
            .any_of(&[
                HappinessEventType::ManagerCriticism,
                HappinessEventType::ManagerDiscipline,
                HappinessEventType::ManagerTrustEroding,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::ManagerCallsOutPlayer, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        // How he is being used rather than whether he is being picked:
        // shoved into a role that is not his, or taken off once too
        // often. Quieter than a public word from the manager, and it is
        // the complaint that most often comes before one.
        if feed
            .any_of(&[
                HappinessEventType::UnhappyWithTacticalRole,
                HappinessEventType::SubstitutionFrustration,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::RoleFrustration, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
            return;
        }

        // He has asked for the conversation nobody wants to have.
        if feed.happened(HappinessEventType::AskedForPrivateTalk) {
            out.push(
                NewsStory::new(NewsStoryKind::ClearTheAir, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        if feed
            .any_of(&[
                HappinessEventType::ManagerPraise,
                HappinessEventType::ManagerEncouragement,
                HappinessEventType::ManagerTrustGrowing,
                HappinessEventType::TrustedInBigMatch,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::ManagerBacksPlayer, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
        }
    }

    /// The squad-wide beats, once the walk has seen everybody.
    pub fn file_club(out: &mut Vec<NewsStory>, pulse: &SquadPulse, date: NaiveDate) {
        if pulse.inquest {
            out.push(NewsStory::new(NewsStoryKind::DressingRoomInquest, date));
        }
        // Never both: a squad that has just been hauled in and a squad
        // being praised for its response are the same week seen from
        // two ends, and the paper picks the fresher one.
        else if pulse.rallied {
            out.push(NewsStory::new(NewsStoryKind::SquadRallies, date));
        }

        if pulse.investment {
            out.push(NewsStory::new(NewsStoryKind::BoardInvests, date));
        }
    }
}
