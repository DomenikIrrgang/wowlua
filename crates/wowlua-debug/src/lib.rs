//! wowlua's debugger and profiler on the emulated client (spec chapter 18):
//! the controller the VM's hooks report to, which stops at breakpoints,
//! steps, shows stacks, variables and the UI, and evaluates code; sessions
//! of a test file or the addon; and the Debug Adapter Protocol.
//!
//! Everything but [`session`] and [`dap`] (the `native` feature) builds
//! for the browser: requests come through a [`Link`], time from the host.

pub mod controller;
mod eval;
pub mod profile;
pub mod protocol;
pub mod ui;
mod values;

#[cfg(feature = "native")]
pub mod dap;
#[cfg(feature = "native")]
pub mod session;

pub use controller::{Clock, Controller, Link, Shared, SharedRef};
pub use protocol::{Event, Request, Response};
