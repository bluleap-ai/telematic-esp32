// src/task/mod.rs
pub mod can;
pub mod gps;
pub mod lte;
pub mod mqtt;
#[allow(dead_code)]
pub mod netmgr;
#[cfg(feature = "ota")]
pub mod ota;
pub mod wifi;
