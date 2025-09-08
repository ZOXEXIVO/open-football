use crate::club::staff::staff_stub::StaffStub;
use crate::club::{
    PersonBehaviour, StaffClubContract, StaffPosition, StaffResponsibility, StaffResult,
};
use crate::context::GlobalContext;
use crate::shared::fullname::FullName;
use crate::utils::{DateUtils, Logging};
use crate::{
    CoachFocus, Person, PersonAttributes, Relations, StaffAttributes, StaffCollectionResult,
    TeamType,
};
use chrono::{NaiveDate, NaiveDateTime};
use std::fmt::{Display, Formatter, Result};

#[derive(Debug)]
pub struct Staff {
    pub id: u32,
    pub full_name: FullName,
    pub country_id: u32,

    pub birth_date: NaiveDate,
    pub attributes: PersonAttributes,
    pub behaviour: PersonBehaviour,

    pub staff_attributes: StaffAttributes,

    pub contract: Option<StaffClubContract>,

    pub relations: Relations,

    pub license: StaffLicenseType,

    pub focus: Option<CoachFocus>,
}

impl Staff {
    pub fn new(
        id: u32,
        full_name: FullName,
        country_id: u32,
        birth_date: NaiveDate,
        staff_attributes: StaffAttributes,
        contract: Option<StaffClubContract>,
        attributes: PersonAttributes,
        license: StaffLicenseType,
        focus: Option<CoachFocus>,
    ) -> Self {
        Staff {
            id,
            full_name,
            country_id,
            birth_date,
            staff_attributes,
            contract,
            behaviour: PersonBehaviour::default(),
            relations: Relations::new(),
            attributes,
            license,
            focus,
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> StaffResult {
        let now = ctx.simulation.date;

        let mut result = StaffResult::new();

        if DateUtils::is_birthday(self.birth_date, now.date()) {
            self.behaviour.try_increase();
        }

        self.process_contract(&mut result, now);

        result
    }

    fn process_contract(&mut self, _: &mut StaffResult, _: NaiveDateTime) {}
}

//DISPLAY
impl Display for Staff {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}, {}", self.full_name, self.birth_date)
    }
}

#[derive(Debug)]
pub struct StaffCollection {
    pub staffs: Vec<Staff>,

    pub responsibility: StaffResponsibility,

    stub: Staff,
}

impl StaffCollection {
    pub fn new(staffs: Vec<Staff>) -> Self {
        StaffCollection {
            staffs,
            responsibility: StaffResponsibility::default(),
            stub: StaffStub::default(),
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> StaffCollectionResult {
        let staff_results = self
            .staffs
            .iter_mut()
            .map(|staff| {
                let message = &format!("simulate staff: id: {}", &staff.id);
                Logging::estimate_result(|| staff.simulate(ctx.with_staff(Some(staff.id))), message)
            })
            .collect();

        StaffCollectionResult::new(staff_results)
    }

    pub fn training_coach(&self, team_type: &TeamType) -> &Staff {
        let responsibility_coach = match team_type {
            TeamType::Main => self.responsibility.training.training_first_team,
            _ => self.responsibility.training.training_youth_team,
        };

        match responsibility_coach {
            Some(_) => self.get_by_id(responsibility_coach.unwrap()),
            None => self.get_by_position(StaffPosition::Coach),
        }
    }

    fn manager(&self) -> Option<&Staff> {
        let manager = self
            .staffs
            .iter()
            .filter(|staff| staff.contract.is_some())
            .find(|staff| {
                staff
                    .contract
                    .as_ref()
                    .expect("no staff contract found")
                    .position
                    == StaffPosition::Manager
            });

        match manager {
            Some(_) => manager,
            None => None,
        }
    }

    pub fn head_coach(&self) -> &Staff {
        match self.manager() {
            Some(ref head_coach) => head_coach,
            None => self.get_by_position(StaffPosition::AssistantManager),
        }
    }

    pub fn contract_resolver(&self, team_type: TeamType) -> &Staff {
        let staff_id = match team_type {
            TeamType::Main => {
                self.responsibility
                    .contract_renewal
                    .handle_first_team_contracts
            }
            TeamType::B => {
                self.responsibility
                    .contract_renewal
                    .handle_other_staff_contracts
            }
            _ => {
                self.responsibility
                    .contract_renewal
                    .handle_youth_team_contracts
            }
        };

        self.get_by_id(staff_id.unwrap())
    }

    fn get_by_position(&self, position: StaffPosition) -> &Staff {
        let staffs: Vec<&Staff> = self
            .staffs
            .iter()
            .filter(|staff| {
                staff.contract.is_some() && staff.contract.as_ref().unwrap().position == position
            })
            .collect();

        if staffs.is_empty() {
            return &self.stub;
        }

        //TODO most relevant

        staffs.first().unwrap()
    }

    fn get_by_id(&self, id: u32) -> &Staff {
        self.staffs.iter().find(|staff| staff.id == id).unwrap()
    }
}

impl Person for Staff {
    fn id(&self) -> u32 {
        self.id
    }

    fn fullname(&self) -> &FullName {
        &self.full_name
    }

    fn birthday(&self) -> NaiveDate {
        self.birth_date
    }

    fn behaviour(&self) -> &PersonBehaviour {
        &self.behaviour
    }

    fn attributes(&self) -> &PersonAttributes {
        &self.attributes
    }

    fn relations(&self) -> &Relations {
        &self.relations
    }
}

#[derive(Debug)]
pub enum StaffLicenseType {
    ContinentalPro,
    ContinentalA,
    ContinentalB,
    ContinentalC,
    NationalA,
    NationalB,
    NationalC,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::{
        BoardResponsibility, ContractRenewalResponsibility, IncomingTransfersResponsibility,
        OutgoingTransfersResponsibility, RecruitmentResponsibility, ScoutingResponsibility,
        StaffResponsibility, TrainingResponsibility,
    };

    fn create_staff_collection(id: u32) -> StaffCollection {
        let mut staff = StaffStub::default();

        staff.id = id;

        StaffCollection {
            responsibility: StaffResponsibility {
                board: BoardResponsibility::default(),
                recruitment: RecruitmentResponsibility::default(),
                incoming_transfers: IncomingTransfersResponsibility::default(),
                outgoing_transfers: OutgoingTransfersResponsibility::default(),
                contract_renewal: ContractRenewalResponsibility {
                    handle_first_team_contracts: Some(1),
                    handle_youth_team_contracts: Some(2),
                    handle_director_of_football_contract: Some(3),
                    handle_other_staff_contracts: Some(4),
                },
                scouting: ScoutingResponsibility::default(),
                training: TrainingResponsibility::default(),
            },
            staffs: vec![staff],
            stub: StaffStub::default(),
        }
    }

    #[test]
    fn staff_get_contract_resolver_first_team_is_correct() {
        let staff_collection = create_staff_collection(1);

        let contract_resolver = staff_collection.contract_resolver(TeamType::Main);

        assert_eq!(1, contract_resolver.id);
    }

    #[test]
    fn staff_get_contract_resolver_b_team_is_correct() {
        let staff_collection = create_staff_collection(4);

        let contract_resolver = staff_collection.contract_resolver(TeamType::B);

        assert_eq!(4, contract_resolver.id);
    }

    #[test]
    fn staff_get_contract_resolver_youth_team_is_correct() {
        let staff_collection = create_staff_collection(2);

        let contract_resolver = staff_collection.contract_resolver(TeamType::U18);

        assert_eq!(2, contract_resolver.id);
    }
}
