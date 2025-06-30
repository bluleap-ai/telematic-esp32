use crate::net::atcmd::general::*;
use crate::net::atcmd::response::*;
use crate::net::atcmd::Urc;
use crate::net::atcmd::*;
use crate::task::lte::TripData;
use atat::{
    asynch::{AtatClient, Client},
    AtatIngress, DefaultDigester, Ingress, UrcChannel,
};
use core::{fmt::Debug, fmt::Write, str::FromStr};
use embassy_time::{Duration, Timer};
use esp_hal::gpio::Output;
use esp_hal::uart::UartRx;
use esp_hal::uart::UartTx;
use esp_hal::Async;
use log::{debug, error, info, trace, warn};

#[derive(Debug)]
pub enum QuectelError {
    NoFix,
    CommandFailed,
    // Add more as needed
}
enum State {
    ResetHardware,
    DisableEchoMode,
    GetModelId,
    GetSoftwareVersion,
    GetSimCardStatus,
    GetNetworkSignalQuality,
    GetNetworkInfo,
    EnableGps,
    EnableAssistGps,
    SetModemFunctionality,
    UploadMqttCert,
    CheckNetworkRegistration,
    MqttOpenConnection,
    MqttConnectBroker,
    MqttPublishData,
    ErrorConnection,
    // GetGPSData,
    //Connected,
    //Disconnected,
}
#[derive(Debug)]
pub struct GpsData {
    pub latitude: f64,
    pub longitude: f64,
    pub timestamp: u64,
}

// === Quectel Driver Struct ===
pub struct Quectel<'a> {
    client: Client<'a, UartTx<'a, Async>, 1024>,
    pen: Output<'a>,
    dtr: Output<'a>,
    urc_channel: &'a UrcChannel<Urc, 128, 3>,
}

impl<'a> Quectel<'a> {
    pub fn new(
        client: Client<'a, UartTx<'a, Async>, 1024>,
        pen: Output<'a>,
        dtr: Output<'a>,
        urc_channel: &'a UrcChannel<Urc, 128, 3>,
    ) -> Self {
        Self {
            client,
            pen,
            dtr,
            urc_channel,
        }
    }

    /// Reset modem bằng chân PWRKEY
    pub async fn reset_hardware(&mut self) {
        info!("[Quectel - MAIN] Reset Hardware");

        self.pen.set_low();
        embassy_time::Timer::after(embassy_time::Duration::from_millis(100)).await;
        self.pen.set_high();
        embassy_time::Timer::after(embassy_time::Duration::from_secs(2)).await;

        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
    }

    /// Disable Echo Mode
    pub async fn disable_echo_mode(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel - MAIN] Disable Echo Mode");
        if check_result(self.client.send(&DisableEchoMode).await) {
            Ok(())
        } else {
            Err(QuectelError::CommandFailed)
        }
    }

    /// Get Model ID
    pub async fn get_model_id(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel - MAIN] Get Model ID");
        if check_result(self.client.send(&GetModelId).await) {
            Ok(())
        } else {
            Err(QuectelError::CommandFailed)
        }
    }

    pub async fn get_netword_signal_quality(&mut self) -> Result<(), atat::Error> {
        info!("[MAIN] Get Network Signal Quality");
        if check_result(self.client.send(&GetNetworkSignalQuality).await) {
            Ok(())
        } else {
            // Err(atat::Error::CommandFailed)
            info!("[MAIN] Failed to get network signal quality");
            Ok(())
        }
    }

    /// Enable GPS
    pub async fn enable_gps(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Enable GPS");
        // if check_result(self.client.send(&EnableGpsFunc).await) {
        //     Ok(())
        // } else {
        //     Err(QuectelError::CommandFailed)
        // }
        Ok(())
    }

    /// Enable Assist GPS
    pub async fn enable_assist_gps(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Enable Assist GPS");
        // if check_result(self.client.send(&EnableAssistGpsFunc).await) {
        Ok(())
        // } else {
        //     Err(QuectelError::CommandFailed)
        // }
    }

    pub async fn get_gps(&mut self) -> Result<GpsData, QuectelError> {
        info!("[Quectel] Getting GPS data...");
        Ok(GpsData {
            latitude: 0.0,  // Placeholder value
            longitude: 0.0, // Placeholder value
            timestamp: embassy_time::Instant::now().as_ticks(),
        })
        // if let Ok(urc) = embassy_futures::select::select(
        //     self.urc_channel
        //         .wait_for(|urc| matches!(urc, Urc::GpsData(_))),
        //     Timer::after(Duration::from_secs(2)),
        // )
        // .await
        // {
        //     match urc {
        //         embassy_futures::select::Either::First(Urc::GpsData(data)) => {
        //             // Giả sử data có latitude/longitude
        //             Ok(GpsData {
        //                 latitude: data.latitude,
        //                 longitude: data.longitude,
        //                 timestamp: embassy_time::Instant::now().as_ticks(),
        //             })
        //         }
        //         _ => Err(QuectelError::NoFix),
        //     }
        // } else {
        //     warn!("[Quectel] No GPS fix");
        //     Err(QuectelError::NoFix)
        // }
    }
}

// === RX Handler task ===
#[embassy_executor::task]
pub async fn rx_handler(
    mut ingress: atat::Ingress<'static, atat::DefaultDigester<Urc>, Urc, 1024, 128, 3>,
    mut reader: UartRx<'static, Async>,
) -> ! {
    ingress.read_from(&mut reader).await
}

// === Helper: Reset modem ===

async fn reset_modem(pen: &mut Output<'static>) {
    pen.set_low(); // Power down the modem
    embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
    pen.set_high(); // Power up the modem
    embassy_time::Timer::after(embassy_time::Duration::from_secs(5)).await;
}
// === Helper: Check AT result ===
fn check_result<T>(res: Result<T, atat::Error>) -> bool
where
    T: Debug,
{
    match res {
        Ok(value) => {
            info!("[Quectel] \t Command succeeded: {value:?}");
            true
        }
        Err(e) => {
            error!("[Quectel] Failed to send AT command: {e:?}");
            false
        }
    }
}
