mod policy;
mod search;
mod tools;
mod workspace;

pub use policy::{PolicyError, Refusal};
pub use tools::ToolDescription;
pub use workspace::{ToolError, Workspace, WorkspaceError};

pub fn check_patterns(patterns: &[String]) -> Result<(), PolicyError> {
    policy::PathPolicy::reading(patterns).map(|_| ())
}
