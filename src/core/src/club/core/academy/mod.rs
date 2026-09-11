//! The club end of the academy pipeline.
//!
//! [`crate::club::academy`] owns the boys and decides who is ready; this is
//! the club acting on that — graduation day, and the weekly rescue when a
//! youth side cannot put eleven players on the pitch.

mod callups;
mod graduation;

pub(in crate::club::core) use graduation::GraduationTerms;
