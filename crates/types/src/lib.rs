//! types — canonical data structures shared across all apex-mev crates.
//!
//! Design decisions:
//! - All integer math uses `u64` for lamports (matches Solana SDK) and `u128`
//!   intermediates to prevent overflow in multiplication chains.
//! - `ArbPath` is copy-free: hop lists use `SmallVec` to keep small paths stack-allocated.
//! - No heap-allocated `String` in hot-path structs; prefer `[u8; N]` or `Pubkey`.
//! - `#[repr(C)]` where needed for zero-copy parsing compatibility.

pub mod dex;
pub mod path;
pub mod pool;
pub mod token;

pub use dex::*;
pub use path::*;
pub use pool::*;
pub use token::*;
