// Module declarations for the app-server protocol namespace.
// Exposes protocol pieces used by `lib.rs` via `pub use protocol::common::*;`.

pub mod common;
pub mod event_mapping;
pub mod item_builders;
mod mappers;
mod serde_helpers;
pub mod thread_history;
pub mod thread_history_projection;
mod thread_history_text_echo;
#[cfg(test)]
#[path = "thread_history_text_echo_tests.rs"]
mod thread_history_text_echo_tests;
mod turn_items_view;
pub mod v1;
pub mod v2;

pub mod external_input_projection;
