/// Constraint for stage ordering
#[derive(Clone, Debug)]
pub enum StageConstraint {
    /// This stage must run before the named stage
    Before(String),
    /// This stage must run after the named stage
    After(String),
}
