use chrono::NaiveDate;

#[derive(Debug, Clone)]
pub struct ClubSponsorship {
    pub sponsorship_contracts: Vec<ClubSponsorshipContract>,
}

impl ClubSponsorship {
    pub fn new(contracts: Vec<ClubSponsorshipContract>) -> Self {
        ClubSponsorship {
            sponsorship_contracts: contracts,
        }
    }

    fn remove_expired_contracts(&mut self, date: NaiveDate) {
        self.sponsorship_contracts
            .retain(|contract| !contract.is_expired(date))
    }

    pub fn get_sponsorship_incomes<'s>(&mut self, date: NaiveDate) -> &[ClubSponsorshipContract] {
        self.remove_expired_contracts(date);

        &self.sponsorship_contracts
    }
}

#[derive(Debug, Clone)]
pub struct ClubSponsorshipContract {
    pub sponsor_name: String,
    pub wage: i32,
    expiration: NaiveDate,
}

impl ClubSponsorshipContract {
    pub fn new(sponsor_name: String, wage: i32, expiration: NaiveDate) -> Self {
        ClubSponsorshipContract {
            sponsor_name,
            wage,
            expiration,
        }
    }

    pub fn is_expired(&self, date: NaiveDate) -> bool {
        date >= self.expiration
    }
}
