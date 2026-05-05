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

    pub fn add(&mut self, start_date: NaiveDate, status: PlayerStatusType) {
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
