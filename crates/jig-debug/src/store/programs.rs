//! Planning a debug session against the files on disk.

use std::path::Path;

use jig_debuggers::Registry;
use jig_run::run_configs::RunConfig;

use crate::domain::debug_target::{DebugPlan, plan_in};

/// How to debug `config`: the plan, or why there's none.
pub fn plan_for(config: &RunConfig, registry: &Registry) -> Result<DebugPlan, String> {
    plan_in(config, registry, Path::exists)
}
