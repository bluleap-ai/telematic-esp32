use core::{fmt::Debug, fmt::Write, str::FromStr};

// use alloc::string::ToString;
use atat::{
    asynch::{AtatClient, Client},
    AtatIngress, DefaultDigester, Ingress, UrcChannel,
};

/// Publishes GPS and CAN data to the MQTT broker.
///
/// Checks LTE connection status and, if active, retrieves GPS data via `RetrieveGpsRmc` and
/// CAN data from `can_channel`. Serializes the data to JSON and publishes to MQTT topics.
///
/// # Arguments
///
/// * `client` - The AT command client for sending commands to the modem.
/// * `mqtt_client_id` - The MQTT client identifier for topic construction.
/// * `gps_channel` - Channel for sending GPS-related `TripData`.
/// * `can_channel` - Channel for receiving CAN-related `CanFrame` data.
///
/// # Returns
///
/// * `true` if both GPS and CAN data are published successfully or if LTE is inactive.
/// * `false` if serialization or publishing fails due to buffer overflow or AT command errors.
// pub async fn handle_publish_mqtt_data(
//     client: &mut Client<'static, UartTx<'static, Async>, 1024>,
//     mqtt_client_id: &str,
//     gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
//     can_channel: &'static TwaiOutbox,
// ) -> ! {
//     loop {
//         if let Ok(active_connection) = ACTIVE_CONNECTION_CHAN_LTE.receiver().try_receive() {
//             IS_LTE.store(active_connection == ActiveConnection::Lte, Ordering::SeqCst);
//             info!("[LTE] Updated IS_LTE: {}", IS_LTE.load(Ordering::SeqCst));
//         }

//         // If LTE is not active, return false
//         if !IS_LTE.load(Ordering::SeqCst) {
//             info!("[LTE] LTE not active, skipping MQTT publish");
//             return true;
//         }
//         loop {
//             if let Ok(frame) = can_channel.try_receive() {
//                 let mut can_topic: heapless::String<128> = heapless::String::new();
//                 let mut can_payload: heapless::String<1024> = heapless::String::new();
//                 let mut buf: [u8; 1024] = [0u8; 1024];
//                 info!("CAN data from LTE");

//                 // Prepare CAN topic
//                 let can_data = CanFrame {
//                     id: frame.id,
//                     len: frame.len,
//                     data: frame.data,
//                 };

//                 writeln!(
//                     &mut can_topic,
//                     "channels/{mqtt_client_id}/messages/client/can"
//                 )
//                 .unwrap();

//                 // Serialize to JSON
//                 if let Ok(len) = serde_json_core::to_slice(&can_data, &mut buf) {
//                     let json = core::str::from_utf8(&buf[..len])
//                         .unwrap_or_default()
//                         .replace('\"', "'");

//                     if can_payload.push_str(&json).is_err() {
//                         error!("[LTE] Payload buffer overflow");
//                         return false;
//                     }

//                     info!("[LTE] MQTT payload (CAN): {can_payload}");
//                     if check_result(
//                         client
//                             .send(&MqttPublishExtended {
//                                 tcp_connect_id: 0,
//                                 msg_id: 0,
//                                 qos: 0,
//                                 retain: 0,
//                                 topic: can_topic,
//                                 payload: can_payload,
//                             })
//                             .await,
//                     ) {
//                         info!("[LTE] CAN data published successfully");
//                     } else {
//                         error!("[LTE] Failed to publish CAN data");
//                         is_can_success = false;
//                     }
//                 } else {
//                     error!("[LTE] Failed to serialize CAN data");
//                     // false
//                 }
//             }

//             if let Ok(trip_data) = gps_channel.try_receive() {
//                 info!("[WIFI] GPS data received from channel: {trip_data:?}");
//                 let mut trip_payload: heapless::String<1024> = heapless::String::new();
//                 let mut buf: [u8; 1024] = [0u8; 1024];
//                 let mut trip_topic: heapless::String<80> = heapless::String::new();
//                 let mut trip_str: heapless::String<1024> = heapless::String::new();

//                 // Serialize to JSON
//                 if let Ok(len) = serde_json_core::to_slice(&trip_data, &mut buf) {
//                     let json = core::str::from_utf8(&buf[..len])
//                         .unwrap_or_default()
//                         .replace('\"', "'");

//                     if trip_payload.push_str(&json).is_err() {
//                         error!("[LTE] Payload buffer overflow");
//                         return false;
//                     }

//                     info!("[LTE] MQTT payload (GPS/trip): {trip_payload}");
//                     if check_result(
//                         client
//                             .send(&MqttPublishExtended {
//                                 tcp_connect_id: 0,
//                                 msg_id: 0,
//                                 qos: 0,
//                                 retain: 0,
//                                 topic: trip_topic,
//                                 payload: trip_payload,
//                             })
//                             .await,
//                     ) {
//                         info!("[LTE] Trip data published successfully");
//                     } else {
//                         error!("[LTE] Failed to publish trip data");
//                         is_gps_success = false;
//                     }
//                 } else {
//                     error!("[LTE] Failed to serialize trip/GPS data");
//                 }
//             }
//             mqtt_client.poll().await;
//             Timer::after_secs(1).await;
//         }
//     }
// }
use crate::cfg::net_cfg::*;
use crate::net::atcmd::general::*;
use crate::net::atcmd::response::*;
use crate::net::atcmd::Urc;
use crate::task::can::*;
use crate::task::modem::*;
use crate::task::netmgr::ConnectionEvent;
use crate::task::netmgr::{ActiveConnection, ACTIVE_CONNECTION_CHAN_LTE, CONN_EVENT_CHAN};
use crate::util::time::utc_date_to_unix_timestamp;
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
pub struct Lte {
    modem: Modem,
    mqtt_client_id: &'static str,
    gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
}
// Atomic boolean for tracking LTE connection status
static IS_LTE: AtomicBool = AtomicBool::new(false);

impl Lte {
    pub fn new(
        modem: Modem,
        mqtt_client_id: &'static str,
        gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    ) -> Self {
        Self {
            modem,
            mqtt_client_id,
            gps_channel,
        }
    }

    /// Initializes LTE connectivity, including certificate upload and MQTT connection.
    pub async fn init(
        &mut self,
        ca_chain: &'static [u8],
        certificate: &'static [u8],
        private_key: &'static [u8],
    ) -> Result<(), ModemError> {
        info!("[Lte] Starting LTE initialization");
        let mut state = State::UploadMqttCert;

        loop {
            match state {
                State::UploadMqttCert => {
                    self.modem
                        .upload_mqtt_cert(ca_chain, certificate, private_key)
                        .await?;
                    state = State::CheckNetworkRegistration;
                }
                State::CheckNetworkRegistration => {
                    self.modem.check_network_registration().await?;
                    state = State::MqttOpenConnection;
                }
                State::MqttOpenConnection => {
                    self.modem.mqtt_open_connection().await?;
                    state = State::MqttConnectBroker;
                }
                State::MqttConnectBroker => {
                    self.modem.mqtt_connect_broker().await?;
                    info!("[Lte] LTE initialized successfully");
                    // self.modem.is_connected = true;
                    let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteConnected);
                    break;
                }
                _ => {
                    error!("[Lte] Invalid state in init: {:?}", state);
                    return Err(ModemError::NetworkRegistrationFailed);
                }
            }
            Timer::after(Duration::from_secs(1)).await;
        }
        Ok(())
    }

    /// Runs the GPS data retrieval state machine using the underlying modem.
    pub async fn run_gps_state_machine(&mut self) -> ! {
        info!("[Lte] Starting GPS state machine");
        self.modem
            .gps_state_machine(self.mqtt_client_id, self.gps_channel)
            .await
    }
}
