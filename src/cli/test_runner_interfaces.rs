//! Test runner boundary interfaces (scaffolding)
//!
//! Non-invasive interfaces to guide decomposition of the CLI test runner.

pub trait TestDiscovery {
    fn discover(&self, workspace_root: &str) -> Vec<String>;
}

pub trait HarnessGenerator {
    fn generate(&self, tests: &[String]) -> Result<String, String>;
}

pub trait TestExecutor {
    fn execute(&self, harness_src: &str) -> Result<(), String>;
}
