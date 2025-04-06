// src/task/mod.rs
pub mod can;
pub mod lte;
pub mod mqtt;
#[cfg(feature = "ota")]
pub mod ota;
pub mod wifi;
