use crate::transfers::{CompletedTransfer, TransferType};
use crate::{HappinessEvent, HappinessEventType, Player, PlayerSquadStatus};
use rustc_hash::{FxHashMap, FxHashSet};

/// One knockout tie a club played this week. Domestic cup results never
/// reach a league table, so the match desk has no other way of telling
/// "lost 0-1 away" from "knocked out of the cup".
#[derive(Debug, Clone, Copy)]
pub struct CupTie {
    pub opponent_team_id: u32,
    pub goals_for: u8,
    pub goals_against: u8,
    /// Shoot-out tally when the tie went that far, home side first.
    pub shootout: (u8, u8),
}

impl CupTie {
    /// Who goes through. A drawn 90 minutes is settled on the shoot-out
    /// tally, which is exactly how a supporter reads the result.
    pub fn advanced(&self) -> bool {
        if self.goals_for != self.goals_against {
            return self.goals_for > self.goals_against;
        }
        self.shootout.0 > self.shootout.1
    }
}

/// A goalkeeper's afternoon, as the match engine recorded it.
///
/// The one position whose week is invisible in the numbers every other
/// desk reads: a keeper scores nothing, assists nothing, and the team's
/// scoreline says as much about the ten in front of him as about him.
/// What actually happened to him is in the per-match stat line, so the
/// press run reads it directly.
///
/// Each field is the SINGLE most newsworthy match of the week rather
/// than a total — a story is about one afternoon, and "he made nine
/// saves" is a piece where "he made nine saves across two games" is
/// not.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeeperMatchFacts {
    pub saves: u16,
    /// Goals past him. The engine defines `shots_faced` as saves plus
    /// goals conceded, so this is what is left when the saves come out.
    pub conceded: u16,
    /// Spot-kicks he kept out in a shoot-out.
    pub penalties_saved: u8,
    /// Mistakes of his own that ended in the net.
    pub errors_leading_to_goal: u16,
}

impl KeeperMatchFacts {
    /// Keep the loudest version of each figure across a week in which he
    /// played more than once.
    pub fn absorb(&mut self, other: KeeperMatchFacts) {
        self.saves = self.saves.max(other.saves);
        self.conceded = self.conceded.max(other.conceded);
        self.penalties_saved = self.penalties_saved.max(other.penalties_saved);
        self.errors_leading_to_goal = self
            .errors_leading_to_goal
            .max(other.errors_leading_to_goal);
    }
}

/// Everything the desks need to know about the week just played that
/// they cannot read off a `Club`. Built once per Monday from the world's
/// competitions and shared by every club in it.
pub struct WeeklyMatchFacts {
    /// Goals scored in a single match this week, for players who
    /// managed at least three in one game.
    pub hat_tricks: FxHashMap<u32, u8>,
    /// Players sent off this week.
    pub red_cards: FxHashSet<u32>,
    /// Knockout ties, keyed by the team that played them.
    pub cup_ties: FxHashMap<u32, CupTie>,
    /// What the week did to each goalkeeper who played in it.
    pub keepers: FxHashMap<u32, KeeperMatchFacts>,
}

impl WeeklyMatchFacts {
    pub fn empty() -> Self {
        WeeklyMatchFacts {
            hat_tricks: FxHashMap::default(),
            red_cards: FxHashSet::default(),
            cup_ties: FxHashMap::default(),
            keepers: FxHashMap::default(),
        }
    }
}

/// What kind of move a completed transfer actually was. The distinction
/// the old code missed is the one between "sold" and "left for nothing":
/// a departure with no fee is a release or an expiring contract, and a
/// paper that prints "sold for $0.00" is not a paper anybody believes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMoveKind {
    /// Money changed hands.
    Paid,
    /// No fee: a free transfer, a released player, an expiring deal.
    Free,
    /// A temporary move.
    Loan,
}

impl TransferMoveKind {
    fn read(transfer: &CompletedTransfer, fee: i64) -> Self {
        if matches!(transfer.transfer_type, TransferType::Loan(_)) {
            return TransferMoveKind::Loan;
        }
        // A `Permanent` record with nothing on it is still a free move
        // as far as a reader is concerned, whatever the type says.
        if fee <= 0 || matches!(transfer.transfer_type, TransferType::Free) {
            return TransferMoveKind::Free;
        }
        TransferMoveKind::Paid
    }
}

/// One side of one completed deal, as the market desk needs it.
#[derive(Debug, Clone, Copy)]
pub struct TransferMove {
    pub player_id: u32,
    /// The club at the other end. `0` when the counterparty is the free
    /// agent pool rather than a club.
    pub other_club_id: u32,
    pub fee: i64,
    pub kind: TransferMoveKind,
    /// The player's age on publication day. `0` when the newsroom could
    /// not resolve him, which reads as "age unknown" — never as a
    /// teenager.
    pub age: u8,
    /// An arrival who has worn these colours before. What separates a
    /// homecoming from an ordinary signing of the same size.
    pub returning: bool,
    /// An arrival who was already here on loan — the option exercised
    /// rather than a stranger unveiled.
    pub was_loan_here: bool,
}

/// One club's slice of the week's completed transfer business.
#[derive(Default)]
pub struct ClubTransferWeek {
    pub arrivals: Vec<TransferMove>,
    pub departures: Vec<TransferMove>,
}

impl ClubTransferWeek {
    /// Fold a completed move into the buying club's arrivals or the
    /// selling club's departures. `club_id` says which side is being
    /// filled in.
    pub fn absorb(&mut self, club_id: u32, transfer: &CompletedTransfer) {
        let fee = transfer.fee.amount.max(0.0) as i64;
        let kind = TransferMoveKind::read(transfer, fee);

        // Age and history start unresolved; the newsroom's enrichment
        // pass fills them in for arrivals, where they decide the story.
        if transfer.to_club_id == club_id {
            self.arrivals.push(TransferMove {
                player_id: transfer.player_id,
                other_club_id: transfer.from_club_id,
                fee,
                kind,
                age: 0,
                returning: false,
                was_loan_here: false,
            });
        } else if transfer.from_club_id == club_id {
            self.departures.push(TransferMove {
                player_id: transfer.player_id,
                other_club_id: transfer.to_club_id,
                fee,
                kind,
                age: 0,
                returning: false,
                was_loan_here: false,
            });
        }
    }

    pub fn is_empty(&self) -> bool {
        self.arrivals.is_empty() && self.departures.is_empty()
    }
}

/// Where the club stands in its league on publication day.
#[derive(Debug, Clone, Copy)]
pub struct StandingSnapshot {
    pub position: u8,
    pub teams: u8,
    pub points: u8,
    pub played: u8,
    pub total_rounds: u8,
}

impl StandingSnapshot {
    /// Fraction of the programme completed, 0..1. The press only starts
    /// talking about titles and relegation once there is enough season
    /// behind the table for it to mean something.
    pub fn progress(&self) -> f32 {
        if self.total_rounds == 0 {
            return 0.0;
        }
        (self.played as f32 / self.total_rounds as f32).clamp(0.0, 1.0)
    }
}

/// How one of the club's loaned-out players is getting on, gathered
/// from the borrowing club's roster. The parent club's own squad list
/// no longer holds him, so without this pass the loan column would have
/// nothing to write about — which is exactly the gap a supporter
/// notices first.
#[derive(Debug, Clone, Copy)]
pub struct LoanWatchEntry {
    pub player_id: u32,
    /// Club he is playing for.
    pub loan_club_id: u32,
    /// Starts and substitute appearances in the current spell.
    pub starts: i32,
    pub sub_appearances: i32,
    pub goals: i32,
    pub assists: i32,
    /// Season average rating × 100, sample-regressed.
    pub rating_x100: i32,
    /// Days until the agreement runs out. Negative once it has.
    pub days_left: i32,
    /// Days the player has been at the borrowing club.
    pub days_elapsed: i32,
    /// The parent club can take him back early.
    pub recall_available: bool,
    /// A permanent option or obligation was written into the deal.
    pub permanent_option: bool,
    /// He is young enough that the loan is a development posting rather
    /// than a way of shifting a wage off the books.
    pub is_prospect: bool,
    /// Feed flags read off the player's own event history.
    pub wants_permanent: bool,
    /// He has told anyone who will listen that he has been lent out
    /// enough and wants a pre-season at the club that owns him.
    pub wants_to_prove_himself: bool,
    pub recall_requested: bool,
    pub development_concern: bool,
    pub form_concern: bool,
    pub level_mismatch: bool,
    pub scored_recently: bool,
}

impl LoanWatchEntry {
    pub fn appearances(&self) -> i32 {
        self.starts + self.sub_appearances
    }

    /// Roughly how many fixtures the borrowing club has played since he
    /// arrived — a week is one game in the leagues this simulation runs.
    pub fn expected_appearances(&self) -> i32 {
        (self.days_elapsed / 7).max(0)
    }

    /// True when he is getting a real share of the football on offer.
    pub fn is_first_choice(&self) -> bool {
        let expected = self.expected_appearances();
        expected >= 4 && self.starts * 3 >= expected * 2
    }

    /// True when the move is not doing what it was signed for.
    pub fn is_frozen_out(&self) -> bool {
        let expected = self.expected_appearances();
        expected >= 5 && self.appearances() * 3 <= expected
    }
}

/// Every loaned-out player belonging to one parent club.
#[derive(Default)]
pub struct ClubLoanWatch {
    pub players: Vec<LoanWatchEntry>,
}

/// Career totals a milestone story needs. The live season counters only
/// cover the current spell, so the ledger has to be folded in.
pub struct CareerRecord;

impl CareerRecord {
    pub fn appearances(player: &Player) -> i32 {
        let live = player.statistics.played as i32 + player.statistics.played_subs as i32;
        let historic: i32 = player
            .statistics_history
            .season_ledger
            .iter()
            .map(|entry| entry.statistics.played as i32 + entry.statistics.played_subs as i32)
            .sum();
        live + historic
    }

    pub fn goals(player: &Player) -> i32 {
        let live = player.statistics.goals as i32;
        let historic: i32 = player
            .statistics_history
            .season_ledger
            .iter()
            .map(|entry| entry.statistics.goals as i32)
            .sum();
        live + historic
    }

    /// The tally a goalkeeper's career is really measured in — the one
    /// column on his record that is his own rather than the team's.
    pub fn clean_sheets(player: &Player) -> i32 {
        let live = player.statistics.clean_sheets as i32;
        let historic: i32 = player
            .statistics_history
            .season_ledger
            .iter()
            .map(|entry| entry.statistics.clean_sheets as i32)
            .sum();
        live + historic
    }

    /// Seasons on the books at the club the player is at now — the
    /// number behind a testimonial. The ledger holds one row per
    /// competition, so the seasons are counted distinctly or a player
    /// with a cup run would age three years in one summer.
    pub fn seasons_at_current_club(player: &Player) -> i32 {
        let ledger = &player.statistics_history.season_ledger;
        let Some(latest) = ledger.last() else {
            return 0;
        };
        let slug = latest.team_slug.as_str();

        let mut seasons: Vec<u16> = ledger
            .iter()
            .filter(|entry| entry.team_slug == slug && !entry.is_loan)
            .map(|entry| entry.season_start_year)
            .collect();
        seasons.sort_unstable();
        seasons.dedup();
        seasons.len() as i32
    }
}

/// A read-only window over a player's own event feed. Every squad story
/// that is not a raw statistic starts here: the simulation already
/// records what happened to a player and why, and the paper's job is to
/// pick the handful of beats a reader would care about rather than to
/// re-derive them.
pub struct RecentEvents<'a> {
    events: &'a [HappinessEvent],
    window_days: u16,
}

impl<'a> RecentEvents<'a> {
    /// The week the edition covers.
    pub const WEEK: u16 = 7;
    /// A slightly longer memory for standing conditions — a rumour that
    /// broke ten days ago is still the story if nothing has moved since.
    pub const FORTNIGHT: u16 = 16;

    pub fn week(player: &'a Player) -> Self {
        RecentEvents {
            events: &player.happiness.recent_events,
            window_days: Self::WEEK,
        }
    }

    pub fn fortnight(player: &'a Player) -> Self {
        RecentEvents {
            events: &player.happiness.recent_events,
            window_days: Self::FORTNIGHT,
        }
    }

    pub fn happened(&self, kind: HappinessEventType) -> bool {
        self.lookup(&kind).is_some()
    }

    /// Any of the listed beats, in the order given — the first match
    /// wins, so callers list the loudest version of a story first.
    pub fn any_of(&self, kinds: &[HappinessEventType]) -> Option<HappinessEventType> {
        kinds
            .iter()
            .find(|kind| self.lookup(kind).is_some())
            .cloned()
    }

    pub fn find(&self, kind: HappinessEventType) -> Option<&'a HappinessEvent> {
        self.lookup(&kind)
    }

    fn lookup(&self, kind: &HappinessEventType) -> Option<&'a HappinessEvent> {
        self.events
            .iter()
            .find(|event| &event.event_type == kind && event.days_ago <= self.window_days)
    }

    /// The club named by a transfer-interest event, when the emit site
    /// knew it. `0` means "somebody, and the paper isn't saying who" —
    /// which is how most of these stories genuinely appear in print.
    pub fn interested_club(&self, kind: HappinessEventType) -> u32 {
        self.find(kind)
            .and_then(|event| event.context.as_ref())
            .and_then(|context| context.transfer_interest_context.as_ref())
            .and_then(|interest| interest.interested_club_id)
            .unwrap_or(0)
    }
}

/// What the week did to the dressing room as a whole, tallied during
/// the single walk over the club's players.
///
/// Several stories are only true in aggregate — one unhappy player is
/// not "the terraces have turned", and one grateful one is not "the
/// supporters are behind them". Counting them during the walk that was
/// happening anyway is what makes those stories affordable: the
/// alternative is a second pass over every squad in the world.
#[derive(Default)]
pub struct SquadPulse {
    /// Senior players seen — the denominator every share is read against.
    pub seniors: u16,
    /// The manager has gone, and a new one has walked in.
    pub manager_left: bool,
    pub new_manager: bool,
    /// Silverware — a league title, a cup, a continental prize.
    pub trophy: bool,
    /// Promotion confirmed. Split from `trophy` because "going up" is
    /// its own story, and a bigger one at most clubs.
    pub promotion: bool,
    /// The drop confirmed.
    pub relegated: bool,
    /// A cup final reached and lost.
    pub cup_final_lost: bool,
    /// Continental qualification secured.
    pub europe: bool,
    /// The manager held an inquest after a humiliation.
    pub inquest: bool,
    /// The squad has answered a bad run, or closed ranks around the
    /// manager while the board sharpened its knives.
    pub rallied: bool,
    /// The board put money into the team and the squad noticed.
    pub investment: bool,
    /// Players hearing it from the stands, and from the press box.
    pub fan_praise: u16,
    pub fan_criticism: u16,
    pub media_criticism: u16,
}

impl SquadPulse {
    /// A mood is only the crowd's mood once it is more than one or two
    /// players' bad week. Roughly a fifth of a senior squad, and never
    /// fewer than three voices.
    const SHARE: u16 = 5;
    const FLOOR: u16 = 3;

    pub fn is_widespread(&self, count: u16) -> bool {
        count >= Self::FLOOR && count.saturating_mul(Self::SHARE) >= self.seniors
    }

    /// Everything the boardroom desk used to walk the squad for itself
    /// to learn.
    pub fn is_dugout_settled(&self) -> bool {
        !self.manager_left && !self.new_manager
    }
}

/// How much the local paper cares about a given player, from squad role
/// and standing in the game. Used as a priority nudge so the same story
/// about a key player outranks one about a fringe squad man.
pub struct PlayerStanding;

impl PlayerStanding {
    pub fn importance(player: &Player) -> i32 {
        let role = player
            .contract
            .as_ref()
            .map(|contract| match contract.squad_status {
                PlayerSquadStatus::KeyPlayer => 90,
                PlayerSquadStatus::FirstTeamRegular => 65,
                PlayerSquadStatus::FirstTeamSquadRotation => 35,
                PlayerSquadStatus::HotProspectForTheFuture => 30,
                PlayerSquadStatus::MainBackupPlayer => 15,
                PlayerSquadStatus::DecentYoungster => 10,
                _ => 0,
            })
            .unwrap_or(0);

        role + (player.player_attributes.world_reputation as i32 / 250)
    }
}
