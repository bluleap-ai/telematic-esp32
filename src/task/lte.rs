use crate::cfg::net_cfg::MQTT_CLIENT_ID;
use crate::modem::*;
use crate::net::atcmd::general::*;
use crate::task::can::*;
use crate::task::netmgr::ActiveConnection;
use crate::task::netmgr::ConnectionEvent;
use crate::task::netmgr::{ACTIVE_CONNECTION_CHAN_LTE, CHECK_LTE_HEALTH_CHAN, CONN_EVENT_CHAN};
use atat::asynch::AtatClient;
use core::sync::atomic::{AtomicBool, Ordering};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer};
use log::{error, info, warn};
#[allow(dead_code)] // Suppress unused struct warning
pub struct Lte {
    modem: Modem,
}
pub enum LteState {
    Off,
    Lte,
    Gps,
    Wifi,
    HealthCheck,
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
    ca_chain: &'static [u8],
    certificate: &'static [u8],
    private_key: &'static [u8],
) -> ! {
    modem.modem_init(); // state here
    modem.gps_init(); // state here
    modem.lte_init(mqtt_client_id, ca_chain, certificate, private_key); //state here

    //state here
    if let Ok(true) = CHECK_LTE_HEALTH_CHAN.try_receive() {
        info!("[LTE] Received LTE health check request");
        if modem.health_check_lte().await.is_ok() {
            info!("[LTE] LTE health check successful");
        } else {
            error!("[LTE] LTE health check failed");
        }
    }
    loop {
        // state here
        modem.get_gps(MQTT_CLIENT_ID, gps_channel).await;
        if let Ok(active_connection) = ACTIVE_CONNECTION_CHAN_LTE.receiver().try_receive() {
            IS_LTE.store(active_connection == ActiveConnection::Lte, Ordering::SeqCst);
            info!("[LTE] Updated IS_LTE: {}", IS_LTE.load(Ordering::SeqCst));
        }

        // condition
        if !IS_LTE.load(Ordering::SeqCst) {
            info!("[LTE] LTE not active, skipping MQTT publish");
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }
        info!("[LTE] Using LTE connection");

        // state here
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

            // for testing
            // let can_data = CanFrame {
            //     id: 0,
            //     len: 0,
            //     data: [0, 1, 2, 3, 4, 5, 6, 7],
            // };

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
                        warn!("[LTE] CAN data published successfully");
                        // Timer::after(Duration::from_secs(2)).await;
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

/// Task to handle LTE MQTT operations and health checks using a state machine.
///
/// Follows a sequence of modem initialization, GPS setup, LTE connectivity, GPS data
/// retrieval, health checks, and MQTT publishing. Transitions to Error state on failures
/// and attempts recovery. Listens for health check requests on `CHECK_LTE_HEALTH_CHAN`
/// and active connection updates on `ACTIVE_CONNECTION_CHAN_LTE`. Sends connection
/// events to `CONN_EVENT_CHAN`.
#[embassy_executor::task]
pub async fn lte_mqtt_handler(
    mqtt_client_id: &'static str,
    mut modem: Modem,
    can_channel: &'static TwaiOutbox,
    gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    ca_chain: &'static [u8],
    certificate: &'static [u8],
    private_key: &'static [u8],
) -> ! {
    let mut state = LteState::Off;
    let mut last_error: Option<ModemError> = None;

    loop {
        match state {
            LteState::Off => {
                info!("[LTE] State: Off - Initializing modem");
                match modem.modem_init().await {
                    Ok(()) => {
                        info!("[LTE] Modem initialized successfully");
                        state = LteState::Gps;
                    }
                    Err(e) => {
                        error!("[LTE] Modem initialization failed: {e:?}");
                        last_error = Some(e);
                        state = LteState::Error(e);
                    }
                }
            }
            LteState::Gps => {
                info!("[LTE] State: Gps - Initializing GPS");
                match modem.gps_init().await {
                    Ok(()) => {
                        info!("[LTE] GPS initialized successfully");
                        state = LteState::Lte;
                    }
                    Err(e) => {
                        error!("[LTE] GPS initialization failed: {e:?}");
                        last_error = Some(e);
                        state = LteState::Error(e);
                    }
                }
            }
            LteState::Lte => {
                info!("[LTE] State: Lte - Initializing LTE");
                match modem
                    .lte_init(mqtt_client_id, ca_chain, certificate, private_key)
                    .await
                {
                    Ok(()) => {
                        info!("[LTE] LTE initialized successfully");
                        state = LteState::GetGps;
                    }
                    Err(e) => {
                        error!("[LTE] LTE initialization failed: {e:?}");
                        last_error = Some(e);
                        state = LteState::Error(e);
                    }
                }
            }
            LteState::Wifi => {
                info!("[LTE] State: Wifi - Not implemented");
                Timer::after(Duration::from_secs(1)).await;
            }
            LteState::GetGps => {
                info!("[LTE] State: GetGps - Retrieving GPS data");
                modem.get_gps(mqtt_client_id, gps_channel).await;

                // Check for health check request
                if let Ok(true) = CHECK_LTE_HEALTH_CHAN.try_receive() {
                    state = LteState::HealthCheck;
                } else {
                    // Check IS_LTE condition
                    if let Ok(active_connection) =
                        ACTIVE_CONNECTION_CHAN_LTE.receiver().try_receive()
                    {
                        IS_LTE.store(active_connection == ActiveConnection::Lte, Ordering::SeqCst);
                        info!("[LTE] Updated IS_LTE: {}", IS_LTE.load(Ordering::SeqCst));
                    }
                    if IS_LTE.load(Ordering::SeqCst) {
                        state = LteState::Operational;
                    } else {
                        state = LteState::GetGps;
                        Timer::after(Duration::from_secs(1)).await;
                    }
                }
            }
            LteState::HealthCheck => {
                info!("[LTE] State: HealthCheck - Performing LTE health check");
                match modem.health_check_lte().await {
                    Ok(()) => {
                        info!("[LTE] LTE health check successful");
                        let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteConnected);
                        // Check IS_LTE condition
                        if let Ok(active_connection) =
                            ACTIVE_CONNECTION_CHAN_LTE.receiver().try_receive()
                        {
                            IS_LTE.store(
                                active_connection == ActiveConnection::Lte,
                                Ordering::SeqCst,
                            );
                            info!("[LTE] Updated IS_LTE: {}", IS_LTE.load(Ordering::SeqCst));
                        }
                        if IS_LTE.load(Ordering::SeqCst) {
                            state = LteState::Operational;
                        } else {
                            state = LteState::GetGps;
                        }
                    }
                    Err(e) => {
                        error!("[LTE] LTE health check failed: {e:?}");
                        last_error = Some(e);
                        let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteDisconnected);
                        state = LteState::Error(e);
                    }
                }
            }
            LteState::Operational => {
                info!("[LTE] State: Operational - Performing LTE tasks (MQTT)");

                // Handle CAN data
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

                    // for testing
                    // let can_data = CanFrame {
                    //     id: 0,
                    //     len: 0,
                    //     data: [0, 1, 2, 3, 4, 5, 6, 7],
                    // };

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
                            last_error = Some(ModemError::Command);
                            state = LteState::Error(ModemError::Command);
                            continue;
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
                                warn!("[LTE] CAN data published successfully");
                                // Timer::after(Duration::from_secs(2)).await;
                            } else {
                                error!("[LTE] Failed to publish CAN data");
                                last_error = Some(ModemError::MqttPublish);
                                state = LteState::Error(ModemError::MqttPublish);
                                continue;
                            }
                        }
                    } else {
                        error!("[LTE] Failed to serialize CAN data");
                        last_error = Some(ModemError::Command);
                        state = LteState::Error(ModemError::Command);
                        continue;
                    }
                }

                // Handle GPS data
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
                            last_error = Some(ModemError::Command);
                            state = LteState::Error(ModemError::Command);
                            continue;
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
                                last_error = Some(ModemError::MqttPublish);
                                state = LteState::Error(ModemError::MqttPublish);
                                continue;
                            }
                        }
                    } else {
                        error!("[LTE] Failed to serialize trip/GPS data");
                        last_error = Some(ModemError::Command);
                        state = LteState::Error(ModemError::Command);
                        continue;
                    }
                }

                // Return to GetGps after processing
                state = LteState::GetGps;
                Timer::after(Duration::from_secs(1)).await;
            }
            LteState::Error(error) => {
                error!("[LTE] State: Error - Last error: {error:?}");
                info!("[LTE] Attempting recovery from error: {error:?}");

                // Attempt recovery based on error type
                match error {
                    ModemError::Command | ModemError::NetworkRegistration => {
                        state = LteState::Off; // Retry modem initialization
                    }
                    ModemError::MqttConnection | ModemError::MqttPublish => {
                        state = LteState::Lte; // Retry LTE initialization
                    }
                }

                // Clear the last error after logging
                last_error = None;
                Timer::after(Duration::from_secs(5)).await; // Wait before retrying
            }
        }
    }
}
