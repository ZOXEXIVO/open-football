use crate::r#match::{
    PlayerOpponentsOperationsImpl, PlayerTeammatesOperationsImpl,
    StateProcessingContext,
};

pub struct PlayersOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

impl<'p> PlayersOperationsImpl<'p> {
    pub fn new(ctx: &'p StateProcessingContext<'p>) -> Self {
        PlayersOperationsImpl { ctx }
    }
}

impl<'p> PlayersOperationsImpl<'p> {
    // Teammates
    pub fn teammates(&self) -> PlayerTeammatesOperationsImpl<'p> {
        PlayerTeammatesOperationsImpl::new(self.ctx)
    }

    // Opponents
    pub fn opponents(&self) -> PlayerOpponentsOperationsImpl<'p> {
        PlayerOpponentsOperationsImpl::new(self.ctx)
    }
}
