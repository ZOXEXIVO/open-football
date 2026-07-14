use chrono::NaiveDate;
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct StatusData {
    pub start_date: NaiveDate,
    pub status: PlayerStatusType,
}

impl StatusData {
    pub fn new(start_date: NaiveDate, status: PlayerStatusType) -> Self {
        StatusData { start_date, status }
    }
}

#[derive(Debug, Clone)]
pub struct PlayerStatus {
    pub statuses: Vec<StatusData>,
}

impl PlayerStatus {
    pub fn new() -> Self {
        PlayerStatus {
            statuses: Vec::new(),
        }
    }

    /// Add a status the player isn't already carrying. Statuses are
    /// single-instance flags — a player either has a status or he doesn't,
    /// there is no "transfer-listed twice" — and every reader treats them as
    /// a set (`has` / `get` / `remove` / `held_for_days` all assume one
    /// instance). So `add` is idempotent: re-affirming a status already held
    /// is a no-op, and the original `start_date` is preserved as the start of
    /// the continuous spell `held_for_days` reports.
    ///
    /// This guard is load-bearing, not just defensive. Several listing paths
    /// re-stamp `Lst` / `Loa` each period (the `contract.is_transfer_listed`
    /// flag can be cleared on delist while the status is not, so the "already
    /// listed?" guards miss it), and nothing ever `remove`s `Lst` / `Loa`.
    /// Without idempotency a player who stays listed across seasons
    /// accumulated one status row per period — the "11× Transfer Listed"
    /// display seen on long-lived players.
    pub fn add(&mut self, start_date: NaiveDate, status: PlayerStatusType) {
        if self.has(status) {
            return;
        }
        self.statuses.push(StatusData::new(start_date, status));
    }

    pub fn remove(&mut self, status: PlayerStatusType) {
        if let Some(idx) = self.statuses.iter().position(|s| s.status == status) {
            self.statuses.remove(idx);
        }
    }

    pub fn get(&self) -> Vec<PlayerStatusType> {
        self.statuses.iter().map(|s| s.status).collect()
    }

    /// Membership check without materialising the `get()` Vec. The status
    /// list is tiny (a handful of flags), but `get().contains(..)` allocates
    /// on every call and sits on per-player hot loops (market pool builds,
    /// the Wnt reconciliation walk) — this is the allocation-free path.
    pub fn has(&self, status: PlayerStatusType) -> bool {
        self.statuses.iter().any(|s| s.status == status)
    }

    /// Number of whole days the given status has been continuously held as
    /// of `now`, or `None` when the player is not currently carrying it.
    ///
    /// `add` is idempotent — it never appends a status the player already
    /// carries — and `remove` drops it, so for the single-instance statuses
    /// (`Unh`, `Req`, `Lst`, …) the stored `start_date` marks the beginning
    /// of the current continuous spell, and this returns that spell's length.
    pub fn held_for_days(&self, status: PlayerStatusType, now: NaiveDate) -> Option<i64> {
        self.statuses
            .iter()
            .find(|s| s.status == status)
            .map(|s| (now - s.start_date).num_days())
    }

    /// True iff the player is currently away on international duty at any
    /// level — senior (`Int`) or under-21 (`IntU21`). Club match-day
    /// selection treats both the same: the player is unavailable.
    pub fn is_on_international_duty(&self) -> bool {
        self.statuses
            .iter()
            .any(|s| matches!(s.status, PlayerStatusType::Int | PlayerStatusType::IntU21))
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Serialize)]
pub enum PlayerStatusType {
    //When a player is absent from the club without permission
    Abs,
    //The player has had a bid from another club accepted
    Bid,
    //An out-of-contract player still with a club
    Ctr,
    //The player is cup-tied, having played in the same competition in a previous round but for another club
    Cup,
    //The player is on an MLS developmental contract
    Dev,
    //The player has been selected in the MLS Draft
    Dft,
    //Another club has made a transfer enquiry about the player
    Enq,
    //A player who counts as a foreign player in a competition
    Fgn,
    //A player who wants to leave the club on a free transfer at the end of the season
    Frt,
    //The player is concerned about his future at the club
    Fut,
    //The player counts towards the Home Grown quota necessary for a competition
    HG,
    //A player currently on holiday
    Hol,
    //Ineligible for the next match.
    Ine,
    //When it has a red background, this means a player is injured and cannot be selected. If the background is orange, he has resumed light training, but he may not be fully fit. Check his condition indicator
    Inj,
    //The player is away on international duty
    Int,
    //The player is away on under-21 international duty
    IntU21,
    //When a player is short on match fitness (perhaps after a long spell on the sidelines), and needs perhaps to play with the reserves in order to regain full fitness
    Lmp,
    //Player is available for loan
    Loa,
    //The player is learning from a team-mate (see Tut below).
    Lrn,
    //The player is transfer listed
    Lst,
    //The player has reacted to a media comment made by you
    PR,
    //The player has requested to leave the club
    Req,
    //The player is retiring at the end of the season
    Ret,
    //The player is jaded and in need of a rest
    Rst,
    //The player is being scouted by your scouts
    Sct,
    //The player is an MLS Senior International - a non domestic player aged 25+
    SI,
    //The player has some slight concerns
    Slt,
    //The player is suspended
    Sus,
    //The player has agreed a transfer with another club and will go there when the transfer window opens.
    Trn,
    //The player is travelling to/from international duty with his squad
    Trv,
    //The player is tutoring a team-mate
    Tut,
    //The player is unfit, and shouldn't be selected unless in case of an emergency
    Unf,
    //A player is unhappy with his role or an event/action
    Unh,
    //The player is unregistered for a competition
    Unr,
    //The player has been withdrawn from international duty by his club manager
    Wdn,
    //The player is wanted by another club
    Wnt,
    //The player has no work permit and is unable to play
    Wp,
    //The player is one yellow card away from a suspension
    Yel,
    //The player is an MLS Youth International - a non domestic player aged 24 or under.
    YI,
    //The player is on a youth contract and is not yet on professional terms
    Yth,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// Re-affirming a status each period must not stack duplicate rows — the
    /// "11× Transfer Listed" display seen on long-lived listed players.
    #[test]
    fn add_is_idempotent_single_instance_per_status() {
        let mut s = PlayerStatus::new();
        for _ in 0..11 {
            s.add(d(2026, 6, 1), PlayerStatusType::Lst);
        }
        assert_eq!(
            s.get()
                .iter()
                .filter(|&&x| x == PlayerStatusType::Lst)
                .count(),
            1,
            "a status must appear at most once",
        );
    }

    /// The first add wins: `held_for_days` measures the continuous spell from
    /// when the status was first taken, not the last re-stamp.
    #[test]
    fn add_preserves_original_spell_start() {
        let mut s = PlayerStatus::new();
        s.add(d(2026, 6, 1), PlayerStatusType::Lst);
        s.add(d(2029, 1, 1), PlayerStatusType::Lst); // re-stamped years later
        assert_eq!(
            s.held_for_days(PlayerStatusType::Lst, d(2026, 6, 11)),
            Some(10),
            "spell start must remain the first add",
        );
    }

    /// Distinct statuses coexist, and with single-instance storage one
    /// `remove` fully clears a status (no lingering duplicate).
    #[test]
    fn distinct_statuses_coexist_and_remove_clears_fully() {
        let mut s = PlayerStatus::new();
        s.add(d(2026, 6, 1), PlayerStatusType::Lst);
        s.add(d(2026, 6, 1), PlayerStatusType::Loa);
        s.add(d(2026, 6, 1), PlayerStatusType::Lst); // duplicate ignored
        assert!(s.has(PlayerStatusType::Lst));
        assert!(s.has(PlayerStatusType::Loa));
        s.remove(PlayerStatusType::Lst);
        assert!(!s.has(PlayerStatusType::Lst));
        assert!(s.has(PlayerStatusType::Loa));
    }
}
