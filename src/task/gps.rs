// use crate::modem::{Modem, ModemError, TripData, MODEM};
// use embassy_sync::blocking_mutex::raw::NoopRawMutex;
// use embassy_sync::channel::Channel;
// use embassy_time::{Duration, Timer};
// use log::{error, info};

// pub async fn gps_init() -> Result<(), ModemError> {
//     info!("[Gps] Starting GPS initialization");
//     let mut modem = MODEM.get().expect("MODEM not initialized").lock().await;

//     // Step 1: Enable GPS
//     if modem.enable_gps().await.is_ok() {
//         info!("[Gps] GPS enabled successfully");
//     } else {
//         error!("[Gps] Failed to enable GPS");
//         return Err(ModemError::CommandFailed);
//     }

//     // Step 2: Enable assisted GPS
//     if modem.enable_assist_gps().await.is_ok() {
//         info!("[Gps] Assisted GPS enabled successfully");
//     } else {
//         error!("[Gps] Failed to enable Assisted GPS");
//         return Err(ModemError::CommandFailed);
//     }

//     Ok(())
// }

// #[embassy_executor::task]
// pub async fn gps_handler(
//     mqtt_client_id: &'static str,
//     gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
// ) {
//     info!("[Gps] Starting GPS handler");

//     loop {
//         {
//             let mut modem = MODEM.get().expect("MODEM not initialized").lock().await;
//             modem.get_gps(mqtt_client_id, gps_channel).await;
//         }
//         Timer::after(Duration::from_secs(1)).await;
//     }
// }
