//! Netz-Tests des Proxys gegen lokale Test-Upstreams (ADR-0033: nie gegen öffentliche Hosts).

mod core_tests;
mod credential_tests;
mod fuzzing_tests;
mod h2_tests;
mod passthrough_tests;
mod private_ip_tests;
mod sandbox_env_tests;
mod support;
mod tls_tests;
