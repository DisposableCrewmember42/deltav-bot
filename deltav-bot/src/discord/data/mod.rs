//! Database helpers
//!
//! ## Return types
//! This module, and its submodules, use return type conventions that may be unusual:
//! - Error logging is generally done for you.
//! - Functions in this module will return Options if they are expected to fail due to missing data.
//! - They return Result<(), ()> if they could fail unexpectedly, but the user does not need to / should not be informed about the specific cause.
//! - They return Result<(), String> if the error String should be presented to the user.

pub mod config;
pub mod discussions;
pub mod forums;
pub mod macros;
