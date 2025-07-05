use core::{fmt::Debug, fmt::Write, str::FromStr};

// use alloc::string::ToString;
use crate::cfg::net_cfg::*;
use crate::modem::*;
use crate::net::atcmd::general::*;
use crate::net::atcmd::response::*;
use crate::net::atcmd::Urc;
use crate::task::can::*;
// use crate::task::modem::*;
use crate::task::netmgr::ConnectionEvent;
use crate::task::netmgr::{ActiveConnection, ACTIVE_CONNECTION_CHAN_LTE, CONN_EVENT_CHAN};
use crate::util::time::utc_date_to_unix_timestamp;
use atat::{
    asynch::{AtatClient, Client},
    AtatIngress, DefaultDigester, Ingress, UrcChannel,
};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer};
// use embedded_can::Frame;
use esp_hal::{
    gpio::Output,
    uart::{UartRx, UartTx},
    Async,
};
// use esp_println::print;
use core::sync::atomic::{AtomicBool, Ordering};
use log::{debug, error, info, trace, warn};
use serde::{Deserialize, Serialize};

// Atomic boolean for tracking GPS status (optional, can be removed if not needed)
static IS_GPS_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Manages GPS functionality using the modem.
pub struct GpsManager {
    modem: Modem,
}

impl GpsManager {
    /// Creates a new instance of the GPS manager with the provided modem.
    ///
    /// # Arguments
    ///
    /// * `modem` - The modem instance to manage GPS functionality.
    ///
    /// # Returns
    ///
    /// A new `GpsManager` instance.
    pub fn new(modem: Modem) -> Self {
        Self { modem }
    }

    /// Initializes GPS functionality on the modem.
    ///
    /// Enables GPS and assisted GPS features. Assumes the modem has been minimally initialized.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if initialization completes successfully.
    /// * `Err(ModemError)` if any step fails.
    pub async fn init(&mut self) -> Result<(), ModemError> {
        info!("[Gps] Starting GPS initialization");

        // Step 1: Enable GPS
        self.modem.enable_gps().await?;
        info!("[Gps] GPS enabled successfully");

        // Step 2: Enable assisted GPS (optional, depending on modem support)
        self.modem.enable_assist_gps().await?;
        info!("[Gps] Assisted GPS enabled successfully");

        IS_GPS_ACTIVE.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Handles GPS data retrieval and sending to the GPS channel in a continuous loop.
    ///
    /// Retrieves GPS data using the modem, processes it into `TripData`, and sends it to the provided channel.
    /// Runs indefinitely and can be integrated with MQTT publishing if needed.
    ///
    /// # Arguments
    ///
    /// * `mqtt_client_id` - The MQTT client identifier for constructing `device_id` and `trip_id`.
    /// * `gps_channel` - Channel for sending GPS-related `TripData`.
    ///
    /// # Returns
    ///
    /// This function runs indefinitely and does not return.
    pub async fn gps_handler(
        mut self,
        mqtt_client_id: &'static str,
        gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    ) -> ! {
        info!("[Gps] Starting GPS handler");

        loop {
            // Check GPS status (optional)
            if !IS_GPS_ACTIVE.load(Ordering::SeqCst) {
                info!("[Gps] GPS not active, skipping data retrieval");
                Timer::after(Duration::from_secs(5)).await; // Wait before retrying
                continue;
            }

            // Retrieve and process GPS data
            self.modem.get_gps(mqtt_client_id, gps_channel).await;

            // Add a delay to control polling rate (adjust based on needs)
            Timer::after(Duration::from_secs(1)).await;
        }
    }
}
