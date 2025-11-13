use crate::r#match::{MatchPlayerLite, StateProcessingContext};

/// Extension trait for Iterator<Item = MatchPlayerLite> to enable chaining filters
pub trait MatchPlayerIteratorExt<'a>: Iterator<Item = MatchPlayerLite> + Sized {
    /// Filter players who have the ball
    fn with_ball(self, ctx: &'a StateProcessingContext<'a>) -> WithBallIterator<'a, Self> {
        WithBallIterator {
            inner: self,
            ctx,
            has_ball: true,
        }
    }

    /// Filter players who don't have the ball
    fn without_ball(self, ctx: &'a StateProcessingContext<'a>) -> WithBallIterator<'a, Self> {
        WithBallIterator {
            inner: self,
            ctx,
            has_ball: false,
        }
    }
}

// Implement the trait for all iterators that yield MatchPlayerLite
impl<'a, I> MatchPlayerIteratorExt<'a> for I where I: Iterator<Item = MatchPlayerLite> {}

/// Iterator adapter that filters players based on ball possession
pub struct WithBallIterator<'a, I> {
    inner: I,
    ctx: &'a StateProcessingContext<'a>,
    has_ball: bool,
}

impl<'a, I> Iterator for WithBallIterator<'a, I>
where
    I: Iterator<Item = MatchPlayerLite>,
{
    type Item = MatchPlayerLite;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.find(|player| {
            let player_has_ball = self.ctx.ball().owner_id() == Some(player.id);
            player_has_ball == self.has_ball
        })
    }
}
