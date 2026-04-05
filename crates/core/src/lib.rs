// crates/core/src/lib.rs
pub mod price_matrix;
pub mod rich_engine;

// Temporary stub for GNN until real implementation is added
pub mod gnn_stub;

pub use gnn_stub::GnnOracle;
pub use price_matrix::PriceMatrix;
pub use rich_engine::RichEngine;
