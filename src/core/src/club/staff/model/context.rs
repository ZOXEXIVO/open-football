#[derive(Clone)]
pub struct StaffContext {
    pub id: Option<u32>,
}

impl StaffContext {
    pub fn new(id: Option<u32>) -> Self {
        StaffContext { id }
    }
}
