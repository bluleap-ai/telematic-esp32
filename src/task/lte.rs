use crate::modem::*;
use crate::net::atcmd::general::*;
use crate::task::can::*;
use crate::task::netmgr::ActiveConnection;
use crate::task::netmgr::ConnectionEvent;
use crate::task::netmgr::{ACTIVE_CONNECTION_CHAN_LTE, CONN_EVENT_CHAN};
use atat::asynch::AtatClient;
use core::sync::atomic::{AtomicBool, Ordering};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer};
use log::{error, info};
#[allow(dead_code)] // Suppress unused struct warning
pub struct Lte {
    modem: Modem,
}

static IS_LTE: AtomicBool = AtomicBool::new(false);

#[allow(dead_code)] // Suppress unused method warnings
impl Lte {
    pub fn new(modem: Modem) -> Self {
        Self { modem }
    }

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
                    let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteConnected);
                    break;
                }
                _ => {
                    error!("[Lte] Invalid state in init: {state:?}");
                    return Err(ModemError::NetworkRegistration);
                }
            }
            Timer::after(Duration::from_secs(1)).await;
        }
        Ok(())
    }
}

#[embassy_executor::task]
pub async fn lte_mqtt_handler(
    mqtt_client_id: &'static str,
    mut modem: Modem,
    can_channel: &'static TwaiOutbox,
    gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
) -> ! {
    loop {
        info!("LTELTELTELTELTELTELTELTELTELTELTELTELTELTELTE");
        if let Err(e) = modem.check_network_registration().await {
            error!("[LTE] Network registration check failed: {e:?}");
        }
        if let Ok(active_connection) = ACTIVE_CONNECTION_CHAN_LTE.receiver().try_receive() {
            IS_LTE.store(active_connection == ActiveConnection::Lte, Ordering::SeqCst);
            info!("[LTE] Updated IS_LTE: {}", IS_LTE.load(Ordering::SeqCst));
        }

        if !IS_LTE.load(Ordering::SeqCst) {
            info!("[LTE] LTE not active, skipping MQTT publish");
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }

        if let Ok(frame) = can_channel.try_receive() {
            let mut can_topic: heapless::String<128> = heapless::String::new();
            let mut can_payload: heapless::String<1024> = heapless::String::new();
            let mut buf: [u8; 1024] = [0u8; 1024];
            info!("[LTE] CAN data received");

            let can_data = CanFrame {
                id: frame.id,
                len: frame.len,
                data: frame.data,
            };

            let _ = core::fmt::write(
                &mut can_topic,
                format_args!("channels/{mqtt_client_id}/messages/client/can"),
            );

            if let Ok(len) = serde_json_core::to_slice(&can_data, &mut buf) {
                let json = core::str::from_utf8(&buf[..len])
                    .unwrap_or_default()
                    .replace('\"', "'");

                if can_payload.push_str(&json).is_err() {
                    error!("[LTE] Payload buffer overflow");
                } else {
                    info!("[LTE] MQTT payload (CAN): {can_payload}");
                    if check_result(
                        modem
                            .client
                            .send(&MqttPublishExtended {
                                tcp_connect_id: 0,
                                msg_id: 0,
                                qos: 0,
                                retain: 0,
                                topic: can_topic,
                                payload: can_payload,
                            })
                            .await,
                    ) {
                        info!("[LTE] CAN data published successfully");
                    } else {
                        error!("[LTE] Failed to publish CAN data");
                    }
                }
            } else {
                error!("[LTE] Failed to serialize CAN data");
            }
        }

        if let Ok(trip_data) = gps_channel.try_receive() {
            info!("[LTE] GPS data received from channel: {trip_data:?}");
            let mut trip_payload: heapless::String<1024> = heapless::String::new();
            let mut buf: [u8; 1024] = [0u8; 1024];
            let mut trip_topic: heapless::String<128> = heapless::String::new();

            let _ = core::fmt::write(
                &mut trip_topic,
                format_args!("channels/{mqtt_client_id}/messages/client/trip"),
            );

            if let Ok(len) = serde_json_core::to_slice(&trip_data, &mut buf) {
                let json = core::str::from_utf8(&buf[..len])
                    .unwrap_or_default()
                    .replace('\"', "'");

                if trip_payload.push_str(&json).is_err() {
                    error!("[LTE] Payload buffer overflow");
                } else {
                    info!("[LTE] MQTT payload (GPS/trip): {trip_payload}");
                    if check_result(
                        modem
                            .client
                            .send(&MqttPublishExtended {
                                tcp_connect_id: 0,
                                msg_id: 0,
                                qos: 0,
                                retain: 0,
                                topic: trip_topic,
                                payload: trip_payload,
                            })
                            .await,
                    ) {
                        info!("[LTE] Trip data published successfully");
                    } else {
                        error!("[LTE] Failed to publish trip data");
                    }
                }
            } else {
                error!("[LTE] Failed to serialize trip/GPS data");
            }
        }

        Timer::after(Duration::from_secs(1)).await;
    }
}

pub fn check_result<T>(res: Result<T, atat::Error>) -> bool
where
    T: core::fmt::Debug,
{
    match res {
        Ok(value) => {
            info!("[modem] \t Command succeeded: {value:?}");
            true
        }
        Err(e) => {
            error!("[modem] Failed to send AT command: {e:?}");
            false
        }
    }
}
