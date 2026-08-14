//! Consolidated real-SBF parity suite; one binary keeps CI linking bounded.

#[path = "svm/common.rs"]
mod common;
#[path = "svm/configuration.rs"]
mod configuration;
#[path = "svm/deposit.rs"]
mod deposit;
#[path = "svm/dispute_lifecycle.rs"]
mod dispute_lifecycle;
#[path = "svm/initialization.rs"]
mod initialization;
#[path = "svm/orchestrator.rs"]
mod orchestrator;
#[path = "svm/stake.rs"]
mod stake;
