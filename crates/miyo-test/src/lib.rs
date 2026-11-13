//! miyo-test：自动化测试运行器（SPEC §17 / M9）。

pub mod assert;
pub mod config;
pub mod extract;
pub mod load;
pub mod materialize;
pub mod report;
pub mod runner;

pub use assert::{evaluate, Outcome};
pub use config::{AssertionSpec, ExtractSpec, SetVariable, TestSpec};
pub use load::{run_load, LoadConfig, LoadResult};
pub use report::{compute_stats, html_report, junit_xml, ReportRun, ReportStats};
pub use runner::{order_endpoints, run_endpoint, EndpointResult};
