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
use esp_wifi::wifi::WifiState;
use log::{error, info, warn};
#[allow(dead_code)] // Suppress unused struct warning
pub struct Lte {
    modem: Modem,
}
pub enum LteState {
    Off,
    Lte,
    Gps,
    HealthCheck,
    GetGps,
    Operational,
    InitMqttLte,
    Error(ModemError),
}
static IS_LTE: AtomicBool = AtomicBool::new(false);

/// Task to handle LTE MQTT operations and health checks using a state machine.
///
/// Follows a sequence of modem initialization, GPS setup, LTE connectivity, GPS data
/// retrieval, health checks, and MQTT publishing. Transitions to Error state on failures
/// and attempts recovery. Listens for health check requests on `CHECK_LTE_HEALTH_CHAN`
/// and active connection updates on `ACTIVE_CONNECTION_CHAN_LTE`. Sends connection
/// events to `CONN_EVENT_CHAN`.
#[embassy_executor::task]
pub async fn lte_mqtt_handler_fsm(
    mqtt_client_id: &'static str,
    mut modem: Modem,
    can_channel: &'static TwaiOutbox,
    gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    ca_chain: &'static [u8],
    certificate: &'static [u8],
    private_key: &'static [u8],
) -> ! {
    let mut state = LteState::Off;
    let mut retry_count = 0;
    const MAX_RETRIES: u8 = 3;

    loop {
        match state {
            LteState::Off => {
                if retry_count > 0 {
                    info!(
                        "[LTE] State: Off - Initializing modem (Attempt {}/{})",
                        retry_count + 1,
                        MAX_RETRIES
                    );
                }
                match modem.modem_init().await {
                    Ok(()) => {
                        info!("[LTE] Modem initialized successfully");
                        state = LteState::Lte;
                        retry_count = 0; // Reset retry count on success
                    }
                    Err(e) => {
                        retry_count += 1;
                        error!(
                            "[LTE] Modem initialization failed: {e:?} (Attempt {}/{})",
                            retry_count, MAX_RETRIES
                        );
                        if retry_count >= MAX_RETRIES {
                            state = LteState::Error(e);
                            retry_count = 0; // Reset retry count for error state
                        } else {
                            Timer::after(Duration::from_secs(1)).await; // Wait before retrying
                        }
                    }
                }
            }
            LteState::Lte => {
                embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
                if retry_count > 0 {
                    info!(
                        "[LTE] State: Lte - Initializing LTE (Attempt {}/{})",
                        retry_count + 1,
                        MAX_RETRIES
                    );
                }
                match modem
                    .lte_init(mqtt_client_id, ca_chain, certificate, private_key)
                    .await
                {
                    Ok(()) => {
                        info!("[LTE] LTE initialized successfully");
                        state = LteState::Gps;
                        retry_count = 0; // Reset retry count on success
                    }
                    Err(e) => {
                        retry_count += 1;
                        error!(
                            "[LTE] LTE initialization failed: {e:?} (Attempt {}/{})",
                            retry_count, MAX_RETRIES
                        );
                        if retry_count >= MAX_RETRIES {
                            if esp_wifi::wifi::wifi_state() == WifiState::StaConnected {
                                state = LteState::Gps;
                                retry_count = 0; // Reset retry count for error state
                            } else {
                                state = LteState::Error(e);
                                retry_count = 0; // Reset retry count for error state
                            }
                        } else {
                            Timer::after(Duration::from_secs(1)).await; // Wait before retrying
                        }
                    }
                }
            }
            LteState::Gps => {
                embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
                if retry_count > 0 {
                    info!(
                        "[LTE] State: Gps - Initializing GPS (Attempt {}/{})",
                        retry_count + 1,
                        MAX_RETRIES
                    );
                }
                match modem.gps_init().await {
                    Ok(()) => {
                        info!("[LTE] GPS initialized successfully");
                        state = LteState::GetGps;
                        retry_count = 0; // Reset retry count on success
                    }
                    Err(e) => {
                        retry_count += 1;
                        error!(
                            "[LTE] GPS initialization failed: {e:?} (Attempt {}/{})",
                            retry_count, MAX_RETRIES
                        );
                        if retry_count >= MAX_RETRIES {
                            state = LteState::Error(e);
                            retry_count = 0; // Reset retry count for error state
                        } else {
                            Timer::after(Duration::from_secs(1)).await; // Wait before retrying
                        }
                    }
                }
            }
            LteState::GetGps => {
                if retry_count > 0 {
                    info!(
                        "[LTE] State: GetGps - Retrieving GPS data (Attempt {}/{})",
                        retry_count + 1,
                        MAX_RETRIES
                    );
                }
                match modem.get_gps(mqtt_client_id, gps_channel).await {
                    Ok(()) => {
                        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
                        retry_count = 0; // Reset retry count on success
                        if let Ok(true) = CHECK_LTE_HEALTH_CHAN.try_receive() {
                            info!("[LTE] get health check from GetGps");
                            state = LteState::HealthCheck;
                        } else {
                            // Check IS_LTE condition
                            if let Ok(active_connection) =
                                ACTIVE_CONNECTION_CHAN_LTE.receiver().try_receive()
                            {
                                IS_LTE.store(
                                    active_connection == ActiveConnection::Lte,
                                    Ordering::SeqCst,
                                );
                            }
                            info!("[LTE] Updated IS_LTE: {}", IS_LTE.load(Ordering::SeqCst));
                            if IS_LTE.load(Ordering::SeqCst) {
                                state = LteState::InitMqttLte;
                            } else {
                                state = LteState::GetGps;
                                Timer::after(Duration::from_secs(1)).await;
                            }
                        }
                    }
                    Err(e) => {
                        retry_count += 1;
                        error!(
                            "[LTE] GPS data retrieval failed: {e:?} (Attempt {}/{})",
                            retry_count, MAX_RETRIES
                        );
                        if retry_count >= MAX_RETRIES {
                            state = LteState::Error(e);
                            retry_count = 0; // Reset retry count for error state
                        } else {
                            Timer::after(Duration::from_secs(1)).await; // Wait before retrying
                        }
                    }
                }
            }
            LteState::HealthCheck => {
                embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
                if retry_count > 0 {
                    info!(
                        "[LTE] State: HealthCheck (Attempt {}/{})",
                        retry_count + 1,
                        MAX_RETRIES
                    );
                }
                match modem.health_check_lte().await {
                    Ok(()) => {
                        info!("[LTE] HealthCheck successful");
                        state = LteState::GetGps;
                        retry_count = 0; // Reset retry count on success
                    }
                    Err(e) => {
                        retry_count += 1;
                        error!(
                            "[LTE] HealthCheck failed: {e:?} (Attempt {}/{})",
                            retry_count, MAX_RETRIES
                        );
                        if retry_count >= MAX_RETRIES {
                            state = LteState::Error(e);
                            retry_count = 0; // Reset retry count for error state
                        } else {
                            Timer::after(Duration::from_secs(1)).await; // Wait before retrying
                        }
                    }
                }
            }
            LteState::InitMqttLte => {
                embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
                if retry_count > 0 {
                    info!(
                        "[LTE] State: InitMqttLte (Attempt {}/{})",
                        retry_count + 1,
                        MAX_RETRIES
                    );
                }
                match modem.init_mqtt_over_lte().await {
                    Ok(()) => {
                        info!("[LTE] InitMqttLte successful");
                        match CONN_EVENT_CHAN.try_send(ConnectionEvent::LteConnected) {
                            Ok(_) => info!("[LTE] LTE connected event sent successfully"),
                            Err(e) => error!("[LTE] Failed to send LTE connected event: {e:?}"),
                        };
                        state = LteState::Operational;
                        retry_count = 0; // Reset retry count on success
                    }
                    Err(e) => {
                        retry_count += 1;
                        error!(
                            "[LTE] InitMqttLte failed: {e:?} (Attempt {}/{})",
                            retry_count, MAX_RETRIES
                        );
                        let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteDisconnected);
                        if retry_count >= MAX_RETRIES {
                            state = LteState::Error(e);
                            retry_count = 0; // Reset retry count for error state
                        } else {
                            Timer::after(Duration::from_secs(1)).await; // Wait before retrying
                        }
                    }
                }
            }
            LteState::Operational => {
                embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
                if retry_count > 0 {
                    info!(
                        "[LTE] State: Operational - Performing LTE tasks (MQTT) (Attempt {}/{})",
                        retry_count + 1,
                        MAX_RETRIES
                    );
                }

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

                    if let Err(e) = core::fmt::write(
                        &mut can_topic,
                        format_args!("channels/{mqtt_client_id}/messages/client/can"),
                    ) {
                        error!("[LTE] Failed to format CAN topic: {e:?}");
                        state = LteState::Error(ModemError::Command);
                        continue;
                    }

                    if let Ok(len) = serde_json_core::to_slice(&can_data, &mut buf) {
                        let json = core::str::from_utf8(&buf[..len])
                            .unwrap_or_default()
                            .replace('\"', "'");

                        if can_payload.push_str(&json).is_err() {
                            retry_count += 1;
                            error!(
                                "[LTE] Payload buffer overflow (Attempt {}/{})",
                                retry_count, MAX_RETRIES
                            );
                            if retry_count >= MAX_RETRIES {
                                state = LteState::Error(ModemError::Command);
                                retry_count = 0; // Reset retry count for error state
                            }
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
                                info!("[LTE] CAN data published successfully");
                                info!("[LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE]");
                                info!("                                                SUCCESS");
                                info!("[LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE]");
                                retry_count = 0; // Reset retry count on success
                            } else {
                                retry_count += 1;
                                error!(
                                    "[LTE] Failed to publish CAN data (Attempt {}/{})",
                                    retry_count, MAX_RETRIES
                                );
                                if retry_count >= MAX_RETRIES {
                                    state = LteState::Error(ModemError::MqttPublish);
                                    retry_count = 0; // Reset retry count for error state
                                }
                                continue;
                            }
                        }
                    } else {
                        retry_count += 1;
                        error!(
                            "[LTE] Failed to serialize CAN data (Attempt {}/{})",
                            retry_count, MAX_RETRIES
                        );
                        if retry_count >= MAX_RETRIES {
                            state = LteState::Error(ModemError::Command);
                            retry_count = 0; // Reset retry count for error state
                        }
                        continue;
                    }
                }

                // Handle GPS data
                if let Ok(trip_data) = gps_channel.try_receive() {
                    info!("[LTE] GPS data received from channel: {trip_data:?}");
                    let mut trip_payload: heapless::String<1024> = heapless::String::new();
                    let mut buf: [u8; 1024] = [0u8; 1024];
                    let mut trip_topic: heapless::String<128> = heapless::String::new();

                    if let Err(e) = core::fmt::write(
                        &mut trip_topic,
                        format_args!("channels/{mqtt_client_id}/messages/client/trip"),
                    ) {
                        error!("[LTE] Failed to format trip topic: {e:?}");
                        state = LteState::Error(ModemError::Command);
                        continue;
                    }

                    if let Ok(len) = serde_json_core::to_slice(&trip_data, &mut buf) {
                        let json = core::str::from_utf8(&buf[..len])
                            .unwrap_or_default()
                            .replace('\"', "'");

                        if trip_payload.push_str(&json).is_err() {
                            retry_count += 1;
                            error!(
                                "[LTE] Payload buffer overflow (Attempt {}/{})",
                                retry_count, MAX_RETRIES
                            );
                            if retry_count >= MAX_RETRIES {
                                state = LteState::Error(ModemError::Command);
                                retry_count = 0; // Reset retry count for error state
                            }
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
                                        topic: trip_topic.clone(),
                                        payload: trip_payload.clone(),
                                    })
                                    .await,
                            ) {
                                info!("[LTE] Trip data published successfully");
                                info!("[LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE]");
                                info!("                                                SUCCESS");
                                info!("[LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE][LTE]");
                                retry_count = 0; // Reset retry count on success
                            } else {
                                retry_count += 1;
                                error!(
                                    "[LTE] Failed to publish trip data (Attempt {}/{})",
                                    retry_count, MAX_RETRIES
                                );
                                if retry_count >= MAX_RETRIES {
                                    state = LteState::Error(ModemError::MqttPublish);
                                    retry_count = 0; // Reset retry count for error state
                                }
                                continue;
                            }
                        }
                    } else {
                        retry_count += 1;
                        error!(
                            "[LTE] Failed to serialize trip/GPS data (Attempt {}/{})",
                            retry_count, MAX_RETRIES
                        );
                        if retry_count >= MAX_RETRIES {
                            state = LteState::Error(ModemError::Command);
                            retry_count = 0; // Reset retry count for error state
                        }
                        continue;
                    }
                }

                // Return to GetGps after processing
                state = LteState::GetGps;
                retry_count = 0; // Reset retry count when transitioning
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
                        state = LteState::GetGps; // Retry LTE initialization
                    }
                }
                retry_count = 0; // Reset retry count for new state
                Timer::after(Duration::from_secs(5)).await; // Wait before retrying
            }
        }
    }
}
