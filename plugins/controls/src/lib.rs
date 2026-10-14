//! Controls component exports; business logic lives in logic.rs.
mod logic;
pub use logic::Controls;
touchbar_logic_sdk::export!(Controls);
