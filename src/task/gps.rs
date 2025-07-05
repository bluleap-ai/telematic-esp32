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

pub struct Gps {
    modem: Modem,
}

impl Gps {
    pub fn new(modem: Modem) -> Self {
        Self { modem }
    }

    pub async fn init(&mut self) -> Result<(), ModemError> {
        info!("[Gps] Starting GPS initialization");
        let transitions = vec![
            (
                State::EnableAssistGps,
                Box::new(|m: &mut Modem| m.enable_gps())
                    as Box<dyn Fn(&mut Modem) -> Result<(), ModemError> + 'static>,
            ),
            (State::GetGPSData, Box::new(|m| m.enable_assist_gps())),
        ];
        self.modem
            .run_state_machine(State::EnableGps, &transitions)
            .await?;
        Ok(())
    }

    pub async fn gps_handler(
        mut self,
        mqtt_client_id: &'static str,
        gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    ) -> ! {
        info!("[Gps] Starting GPS handler");
        loop {
            self.modem.get_gps(mqtt_client_id, gps_channel).await;
            Timer::after(Duration::from_secs(1)).await;
        }
    }
}
