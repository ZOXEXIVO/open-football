#[derive(Debug, Clone)]
pub enum Season {
    OneYear(u16),
    TwoYear(u16, u16),
}
