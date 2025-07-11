use core::{fmt::Debug, fmt::Write, str::FromStr};

// use alloc::string::ToString;
use atat::{
    asynch::{AtatClient, Client},
    AtatIngress, DefaultDigester, Ingress, UrcChannel,
};

use crate::cfg::net_cfg::*;
use crate::net::atcmd::general::*;
use crate::net::atcmd::response::*;
use crate::net::atcmd::Urc;
use crate::task::can::*;
use crate::task::netmgr::ConnectionEvent;
use crate::task::netmgr::{ActiveConnection, ACTIVE_CONNECTION_CHAN_LTE, CONN_EVENT_CHAN};
use crate::util::time::utc_date_to_unix_timestamp;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
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
const REGISTERED_HOME: u8 = 1;
const UNREGISTERED_SEARCHING: u8 = 2;
const REGISTRATION_DENIED: u8 = 3;
const REGISTRATION_FAILED: u8 = 4;
const REGISTERED_ROAMING: u8 = 5;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TripData {
    device_id: heapless::String<36>,
    trip_id: heapless::String<36>,
    latitude: f64,
    longitude: f64,
    timestamp: u64,
}
static IS_LTE: AtomicBool = AtomicBool::new(false);
#[derive(Debug)]
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
async fn handle_publish_mqtt_data(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
    mqtt_client_id: &str,
    gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    can_channel: &'static TwaiOutbox,
) -> bool {
    if let Ok(active_connection) = ACTIVE_CONNECTION_CHAN_LTE.receiver().try_receive() {
        IS_LTE.store(active_connection == ActiveConnection::Lte, Ordering::SeqCst);
        info!("[LTE] Updated IS_LTE: {}", IS_LTE.load(Ordering::SeqCst));
    }

    // If LTE is not active, return false
    if !IS_LTE.load(Ordering::SeqCst) {
        info!("[LTE] LTE not active, skipping MQTT publish");
        return true;
    }

    let mut trip_topic: heapless::String<128> = heapless::String::new();
    let mut trip_payload: heapless::String<1024> = heapless::String::new();
    let mut buf: [u8; 1024] = [0u8; 1024];
    let mut is_gps_success = false;
    let mut is_can_success = false;

    writeln!(
        &mut trip_topic,
        "channels/{mqtt_client_id}/messages/client/trip"
    )
    .unwrap();

    // --- GPS Data ---
    let trip_result = client.send(&RetrieveGpsRmc).await;

    match trip_result {
        Ok(res) => {
            info!("[LTE] GPS RMC data received: {res:?}");

            let timestamp = utc_date_to_unix_timestamp(&res.utc, &res.date);
            let mut device_id = heapless::String::new();
            let mut trip_id = heapless::String::new();
            write!(&mut trip_id, "{mqtt_client_id}").unwrap();
            write!(&mut device_id, "{mqtt_client_id}").unwrap();

            let trip_data = TripData {
                device_id,
                trip_id,
                latitude: ((res.latitude as u64 / 100) as f64)
                    + ((res.latitude % 100.0f64) / 60.0f64),
                longitude: ((res.longitude as u64 / 100) as f64)
                    + ((res.longitude % 100.0f64) / 60.0f64),
                timestamp,
            };

            if gps_channel.try_send(trip_data.clone()).is_err() {
                error!("[LTE] Failed to send TripData to channel");
            } else {
                info!("[LTE] GPS data sent to channel: {trip_data:?}");
            }

            // Serialize to JSON
            if let Ok(len) = serde_json_core::to_slice(&trip_data, &mut buf) {
                let json = core::str::from_utf8(&buf[..len])
                    .unwrap_or_default()
                    .replace('\"', "'");

                if trip_payload.push_str(&json).is_err() {
                    error!("[LTE] Payload buffer overflow");
                    return false;
                }

                info!("[LTE] MQTT payload (GPS/trip): {trip_payload}");
                if check_result(
                    client
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
                    is_gps_success = false;
                }
            } else {
                error!("[LTE] Failed to serialize trip/GPS data");
            }
        }
        Err(e) => {
            warn!("[LTE] Failed to retrieve GPS data: {e:?}");
        }
    }

    // --- CAN Data ---

    let mut can_topic: heapless::String<128> = heapless::String::new();
    let mut can_payload: heapless::String<1024> = heapless::String::new();
    let mut buf: [u8; 1024] = [0u8; 1024];

    if let Ok(frame) = can_channel.try_receive() {
        info!("CAN data from LTE");

        // Prepare CAN topic
        let can_data = CanFrame {
            id: frame.id,
            len: frame.len,
            data: frame.data,
        };

        writeln!(
            &mut can_topic,
            "channels/{mqtt_client_id}/messages/client/can"
        )
        .unwrap();

        // Serialize to JSON
        if let Ok(len) = serde_json_core::to_slice(&can_data, &mut buf) {
            let json = core::str::from_utf8(&buf[..len])
                .unwrap_or_default()
                .replace('\"', "'");

            if can_payload.push_str(&json).is_err() {
                error!("[LTE] Payload buffer overflow");
                return false;
            }

            info!("[LTE] MQTT payload (CAN): {can_payload}");
            if check_result(
                client
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
                is_can_success = false;
            }
        } else {
            error!("[LTE] Failed to serialize CAN data");
            // false
        }
    }

    is_can_success && is_gps_success
}

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

async fn reset_modem(pen: &mut Output<'static>) {
    pen.set_low(); // Power down the modem
    embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
    pen.set_high(); // Power up the modem
    embassy_time::Timer::after(embassy_time::Duration::from_secs(5)).await;
}

pub async fn upload_mqtt_cert_files(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
    urc_channel: &'static UrcChannel<Urc, 128, 3>,
    ca_chain: &[u8],
    certificate: &[u8],
    private_key: &[u8],
) -> bool {
    let mut raw_data = heapless::Vec::<u8, 4096>::new();
    raw_data.clear();
    let mut subscriber = urc_channel.subscribe().unwrap();
    let _ = client.send(&FileList).await.unwrap();
    let now = embassy_time::Instant::now();
    while now.elapsed().as_secs() < 10 {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
        match subscriber.try_next_message_pure() {
            Some(Urc::ListFile(file)) => log::info!("File: {file:?}"),
            Some(e) => error!("Unknown URC {e:?}"),
            None => info!("Waiting for response..."),
        }
    }

    // Remove old certs
    for name in ["crt.pem", "dvt.crt", "dvt.key"] {
        let _ = client
            .send(&FileDel {
                name: heapless::String::from_str(name).unwrap(),
            })
            .await;
        info!("Deleted old {name}");
    }

    // Upload helper
    async fn upload_file(
        client: &mut Client<'static, UartTx<'static, Async>, 1024>,
        name: &str,
        content: &[u8],
        raw_data: &mut heapless::Vec<u8, 4096>,
    ) -> bool {
        //Sending file upload command to notify the modem about the file to be uploaded
        let name_str = match heapless::String::from_str(name) {
            Ok(s) => s,
            Err(_) => {
                error!("Heapless string overflow for file name: {name}");
                return false;
            }
        };
        //Notify the modem about the file to be uploaded
        if let Err(e) = client
            .send(&FileUpl {
                name: name_str,
                size: content.len() as u32,
            })
            .await
        {
            error!("FileUpl command failed: {e:?}");
            return false;
        }
        //Uploading data payload in 1 Kib of chunks
        for chunk in content.chunks(1024) {
            raw_data.clear();
            if raw_data.extend_from_slice(chunk).is_err() {
                error!("Raw data buffer overflow");
                return false;
            };

            if let Err(_e) = client
                .send(&SendRawData {
                    raw_data: raw_data.clone(),
                    len: chunk.len(),
                })
                .await
            {
                error!("SendRawData command failed");
                return false;
            }
        }

        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
        true
    }

    // Upload certs
    info!("Uploading CA cert...");
    upload_file(client, "crt.pem", ca_chain, &mut raw_data).await;

    info!("Uploading client cert...");
    upload_file(client, "dvt.crt", certificate, &mut raw_data).await;

    info!("Uploading client key...");
    upload_file(client, "dvt.key", private_key, &mut raw_data).await;

    // Configure MQTTS
    info!("Configuring MQTT over TLS...");
    let _ = client
        .send(&MqttConfig {
            name: heapless::String::from_str("recv/mode").unwrap(),
            param_1: Some(0),
            param_2: Some(0),
            param_3: Some(1),
        })
        .await;
    let _ = client
        .send(&MqttConfig {
            name: heapless::String::from_str("SSL").unwrap(),
            param_1: Some(0),
            param_2: Some(1),
            param_3: Some(2),
        })
        .await;

    for (cfg_name, path) in [
        ("cacert", "UFS:ca.crt"),
        ("clientcert", "UFS:dvt.crt"),
        ("clientkey", "UFS:dvt.key"),
    ] {
        let _ = client
            .send(&SslConfigCert {
                name: heapless::String::from_str(cfg_name).unwrap(),
                context_id: 2,
                cert_path: Some(heapless::String::from_str(path).unwrap()),
            })
            .await;
    }

    let _ = client
        .send(&SslConfigOther {
            name: heapless::String::from_str("seclevel").unwrap(),
            context_id: 2,
            level: 2,
        })
        .await;
    let _ = client
        .send(&SslConfigOther {
            name: heapless::String::from_str("sslversion").unwrap(),
            context_id: 2,
            level: 4,
        })
        .await;
    let _ = client.send(&SslSetCipherSuite).await;
    let _ = client
        .send(&SslConfigOther {
            name: heapless::String::from_str("ignorelocaltime").unwrap(),
            context_id: 2,
            level: 1,
        })
        .await;
    let _ = client
        .send(&MqttConfig {
            name: heapless::String::from_str("version").unwrap(),
            param_1: Some(0),
            param_2: Some(4),
            param_3: None,
        })
        .await;

    true
}

pub async fn check_network_registration(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
) -> bool {
    let timeout: embassy_time::Duration = embassy_time::Duration::from_secs(30); // 30 seconds timeout
    let start_time = embassy_time::Instant::now();

    while start_time.elapsed() < timeout {
        match client.send(&GetEPSNetworkRegistrationStatus {}).await {
            Ok(status) => {
                log::info!("[Quectel] EPS network registration status: {status:?}");

                match status.stat {
                    REGISTERED_HOME => {
                        let elapsed = start_time.elapsed().as_secs();
                        info!("[Quectel] Registered (Home) after {elapsed} seconds");
                        return true; // Successfully registered
                    }
                    UNREGISTERED_SEARCHING => {
                        esp_println::print!("."); // Indicating ongoing search
                        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
                    }
                    REGISTRATION_DENIED => {
                        error!("[Quectel] Registration denied");
                        return false; // Registration denied
                    }
                    REGISTRATION_FAILED => {
                        error!("[Quectel] Registration failed");
                        return false; // Registration failed
                    }
                    REGISTERED_ROAMING => {
                        let elapsed = start_time.elapsed().as_secs();
                        info!("[Quectel] Registered (Roaming) after {elapsed} seconds");
                        return true; // Successfully registered
                    }
                    _ => {
                        error!("[Quectel] Unknown registration status: {}", status.stat);
                        return false; // Unknown status
                    }
                }
            }
            Err(e) => {
                error!("[Quectel] Failed to get EPS network registration status: {e:?}");
                return false; // Error occurred
            }
        }
    }

    // Timeout reached without successful registration
    error!("[Quectel] Network registration timed out");
    false
}

#[derive(Debug, PartialEq)]
pub enum MqttConnectError {
    CommandFailed,
    StringConversion,
    Timeout,
    ModemError(u8),
}

pub async fn open_mqtt_connection(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
    urc_channel: &'static UrcChannel<Urc, 128, 3>,
) -> Result<(), MqttConnectError> {
    // Create server string safely
    let server = heapless::String::from_str(MQTT_SERVER_NAME)
        .map_err(|_| MqttConnectError::StringConversion)?; // Optionally log the error here for more info

    // Send MQTT open command
    client
        .send(&MqttOpen {
            link_id: 0,
            server,
            port: MQTT_SERVER_PORT,
        })
        .await
        .map_err(|_| MqttConnectError::CommandFailed)?; // Optionally log the error here for more info

    info!("[Quectel] MQTT open command sent, waiting for response...");

    let mut subscriber = urc_channel
        .subscribe()
        .map_err(|_| MqttConnectError::CommandFailed)?; // Optionally log the error here for more info

    let start = embassy_time::Instant::now();
    const TIMEOUT: embassy_time::Duration = embassy_time::Duration::from_secs(30);

    loop {
        // Check timeout first
        if start.elapsed() >= TIMEOUT {
            error!("[Quectel] MQTT open timed out");
            return Err(MqttConnectError::Timeout);
        }

        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;

        match subscriber.try_next_message_pure() {
            Some(Urc::MqttOpen(response)) => {
                info!("[Quectel] Received MQTT open response: {response:?}");
                return match response.result {
                    0 => Ok(()),
                    code => {
                        error!("[Quectel] Modem reported error code: {}", code as u8);
                        Err(MqttConnectError::ModemError(code as u8))
                    }
                };
            }
            Some(other_urc) => {
                info!("[Quectel] Received unrelated URC: {other_urc:?}");
                // Continue waiting for MQTT open response
            }
            None => {
                warn!("[Quectel] No URC received yet...");
            }
        }
    }
}

pub async fn connect_mqtt_broker(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
    urc_channel: &'static UrcChannel<Urc, 128, 3>,
) -> Result<(), MqttConnectError> {
    const MAX_RETRIES: usize = 3;
    const RESPONSE_TIMEOUT: embassy_time::Duration = embassy_time::Duration::from_secs(30);
    const CLIENT_ID: &str = "telematics-control-unit";

    // Create credentials with proper error handling
    let username = heapless::String::<64>::from_str(MQTT_USR_NAME)
        .map_err(|_| MqttConnectError::StringConversion)?;
    let password = heapless::String::<64>::from_str("f57f9bf3-07b3-4ba5-ae1f-bf6f579e346d") // Note: Same as username - is this intentional?
        .map_err(|_| MqttConnectError::StringConversion)?;
    let client_id = heapless::String::<23>::from_str(CLIENT_ID)
        .map_err(|_| MqttConnectError::StringConversion)?;

    // Send connect command with retries
    for attempt in 1..=MAX_RETRIES {
        info!("[Quectel] MQTT connect attempt {attempt}/{MAX_RETRIES}");

        match client
            .send(&MqttConnect {
                tcp_connect_id: 0,
                client_id: client_id.clone(),
                username: Some(username.clone()),
                password: Some(password.clone()),
            })
            .await
        {
            Ok(_) => break,
            Err(e) if attempt == MAX_RETRIES => {
                error!("[Quectel] Final connect attempt failed: {e:?}");
                return Err(MqttConnectError::CommandFailed);
            }
            Err(e) => {
                warn!("[Quectel] Connect attempt failed: {e:?} - retrying");
                embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
            }
        }
    }

    // Wait for connection acknowledgement
    let mut subscriber = urc_channel
        .subscribe()
        .map_err(|_| MqttConnectError::CommandFailed)?;
    let start = embassy_time::Instant::now();

    loop {
        if start.elapsed() > RESPONSE_TIMEOUT {
            error!("[Quectel] MQTT connect timeout");
            return Err(MqttConnectError::Timeout);
        }

        embassy_time::Timer::after(embassy_time::Duration::from_millis(100)).await;

        match subscriber.try_next_message_pure() {
            Some(Urc::MqttConnect(response)) => {
                info!("[Quectel] Received MQTT connect response: {response:?}");
                return match response.result {
                    0 => Ok(()),
                    code => {
                        error!("[Quectel] Modem connection error: {code}");
                        Err(MqttConnectError::ModemError(code))
                    }
                };
            }
            Some(other_urc) => {
                debug!("Ignoring unrelated URC: {other_urc:?}");
            }
            None => {
                trace!("Waiting for MQTT connect response...");
            }
        }
    }
}

#[embassy_executor::task]
pub async fn quectel_tx_handler(
    mut client: Client<'static, UartTx<'static, Async>, 1024>,
    mut pen: Output<'static>,
    mut _dtr: Output<'static>,
    urc_channel: &'static UrcChannel<Urc, 128, 3>,
    gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    can_channel: &'static TwaiOutbox,
) -> ! {
    let mut state: State = State::ResetHardware;
    let mut is_connected = false;
    let ca_chain = include_str!("../../cert/crt.pem").as_bytes();
    let certificate = include_str!("../../cert/dvt.crt").as_bytes();
    let private_key = include_str!("../../cert/dvt.key").as_bytes();

    loop {
        match state {
            State::ResetHardware => {
                // 0: Reset Hardware
                info!("[Quectel] Reset Hardware");
                reset_modem(&mut pen).await;
                state = State::DisableEchoMode;
            }
            State::DisableEchoMode => {
                info!("[Quectel] Disable Echo Mode");
                if check_result(client.send(&DisableEchoMode).await) {
                    state = State::GetModelId;
                }
            }
            State::GetModelId => {
                info!("[Quectel] Get Model Id");
                if check_result(client.send(&GetModelId).await) {
                    state = State::GetSoftwareVersion;
                }
            }
            State::GetSoftwareVersion => {
                info!("[Quectel] Get Software Version");
                if check_result(client.send(&GetSoftwareVersion).await) {
                    state = State::GetSimCardStatus;
                }
            }
            State::GetSimCardStatus => {
                info!("[Quectel] Get Sim Card Status");
                if check_result(client.send(&GetSimCardStatus).await) {
                    state = State::GetNetworkSignalQuality;
                }
            }
            State::GetNetworkSignalQuality => {
                info!("[Quectel] Get Network Signal Quality");
                if check_result(client.send(&GetNetworkSignalQuality).await) {
                    state = State::GetNetworkInfo;
                }
            }
            State::GetNetworkInfo => {
                info!("[Quectel] Get Network Info");
                if check_result(client.send(&GetNetworkInfo).await) {
                    state = State::EnableGps;
                }
            }
            State::EnableGps => {
                info!("[Quectel] Enable GPS");
                if check_result(client.send(&EnableGpsFunc).await) {
                    state = State::EnableAssistGps;
                }
            }
            State::EnableAssistGps => {
                info!("[Quectel] Enable Assist GPS");
                if check_result(client.send(&EnableAssistGpsFunc).await) {
                    state = State::SetModemFunctionality;
                }
            }
            State::SetModemFunctionality => {
                info!("[Quectel] Set Modem Functionality");
                if check_result(
                    client
                        .send(&SetUeFunctionality {
                            fun: FunctionalityLevelOfUE::Full,
                        })
                        .await,
                ) {
                    state = State::UploadMqttCert;
                }
            }
            State::UploadMqttCert => {
                info!("[Quectel] Upload Files");
                let res: bool = upload_mqtt_cert_files(
                    &mut client,
                    urc_channel,
                    ca_chain,
                    certificate,
                    private_key,
                )
                .await;
                state = if res {
                    State::CheckNetworkRegistration
                } else {
                    error!("[Quectel] File upload failed, resetting hardware");
                    State::ErrorConnection
                };
            }
            State::CheckNetworkRegistration => {
                info!("[Quectel] Check Network Registration");
                let res = check_network_registration(&mut client).await;
                if res {
                    if !is_connected {
                        let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteRegistered);
                        is_connected = true;
                    }
                    state = State::MqttOpenConnection;
                } else {
                    if is_connected {
                        let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteUnregistered);
                        is_connected = false;
                    }
                    error!("[Quectel] Network registration failed, resetting hardware");
                    state = State::ErrorConnection;
                }
            }
            State::MqttOpenConnection => {
                info!("[Quectel] Opening MQTT connection");
                match open_mqtt_connection(&mut client, urc_channel).await {
                    Ok(_) => {
                        info!("[Quectel] MQTT connection opened successfully");
                        state = State::MqttConnectBroker;
                    }
                    Err(e) => {
                        error!("[Quectel] Failed to open MQTT connection: {e:?}");
                    }
                }
            }
            State::MqttConnectBroker => {
                info!("[Quectel] Connecting to MQTT broker");
                match connect_mqtt_broker(&mut client, urc_channel).await {
                    Ok(_) => {
                        info!("[Quectel] MQTT connection established");
                        let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteConnected);
                        state = State::MqttPublishData;
                    }
                    Err(e) => {
                        error!("[Quectel] MQTT connection failed: {e:?}");
                        let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteDisconnected);
                        state = State::ErrorConnection;
                    }
                }
            }

            State::MqttPublishData => {
                info!("[Quectel] Publishing MQTT Data");
                if handle_publish_mqtt_data(&mut client, MQTT_CLIENT_ID, gps_channel, can_channel)
                    .await
                {
                    info!("[Quectel] MQTT data published successfully");
                    // Transition to next state or maintain publishing state
                    state = State::MqttPublishData;
                } else {
                    error!("[Quectel] MQTT publish failed");
                }
            }
            State::ErrorConnection => {
                if is_connected {
                    let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteDisconnected);
                    is_connected = false;
                }
                error!("[Quectel] System in error state - attempting recovery");
                embassy_time::Timer::after(embassy_time::Duration::from_secs(5)).await;
                state = State::ResetHardware;
            }
        }
        // Wait for 1 second before transitioning to the next state
        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
    }
}

#[embassy_executor::task]
pub async fn quectel_rx_handler(
    mut ingress: Ingress<'static, DefaultDigester<Urc>, Urc, 1024, 128, 3>,
    mut reader: UartRx<'static, Async>,
) -> ! {
    ingress.read_from(&mut reader).await
}
