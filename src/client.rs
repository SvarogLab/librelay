//! The Linux side: spawning the Windows server, watching it, logging, and systemd integration.

pub mod lifecycle;
pub mod log;
pub mod mux;
pub mod relay;
pub mod spawn;
pub mod systemd;
