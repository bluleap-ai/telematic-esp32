use core::{fmt::Debug, fmt::Write, str::FromStr};

use atat::{
    asynch::{AtatClient, Client},
    AtatIngress, DefaultDigester, Ingress, UrcChannel,
};

use esp_hal::{
    gpio::Output,
    uart::{UartRx, UartTx},
    Async,
};
use log::{debug, error, info, trace, warn};
use serde::{Deserialize, Serialize};

use crate::net::atcmd::general::*;
use crate::net::atcmd::response::*;
use crate::net::atcmd::Urc;
use crate::task::netmgr::{ConnectionEvent, CONN_EVENT_CHAN};

use crate::cfg::net_cfg::*;

use crate::util::time::utc_date_to_unix_timestamp;

const REGISTERED_HOME: u8 = 1;
const UNREGISTERED_SEARCHING: u8 = 2;
const REGISTRATION_DENIED: u8 = 3;
const REGISTRATION_FAILED: u8 = 4;
const REGISTERED_ROAMING: u8 = 5;

#[derive(Debug, Serialize, Deserialize)]
pub struct TripData {
    device_id: heapless::String<36>,
    trip_id: heapless::String<36>,
    latitude: f64,
    longitude: f64,
    timestamp: u64,
}

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
    //Connected,
    //Disconnected,
}

async fn handle_publish_mqtt_data(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
    mqtt_client_id: &str,
) -> bool {
    let mut mqtt_topic: heapless::String<128> = heapless::String::new();
    let mut payload: heapless::String<1024> = heapless::String::new();
    let mut deserialized: [u8; 1024] = [0u8; 1024];

    writeln!(
        &mut mqtt_topic,
        "m/4fd2230f-e5b1-4fe9-ad30-4c19832e8cef/c/{}",
        mqtt_client_id
    )
    .unwrap();

    match client.send(&RetrieveGpsRmc).await {
        Ok(res) => {
            info!("[Quectel] GPS RMC data received: {res:?}");
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

            if let Ok(len) = serde_json_core::to_slice(&trip_data, &mut deserialized) {
                let single_quote = core::str::from_utf8(&deserialized[..len])
                    .unwrap_or_default()
                    .replace('\"', "'");

                if payload.push_str(&single_quote).is_err() {
                    error!("[Quectel] Payload buffer overflow");
                    return false;
                }

                info!("[Quectel] MQTT payload: {payload}");
                check_result(
                    client
                        .send(&MqttPublishExtended {
                            tcp_connect_id: 0,
                            msg_id: 0,
                            qos: 0,
                            retain: 0,
                            topic: mqtt_topic,
                            payload,
                        })
                        .await,
                )
            } else {
                error!("[Quectel] Failed to serialize trip data");
                false
            }
        }
        Err(e) => {
            warn!("[Quectel] Failed to retrieve GPS data: {e:?}");
            false
        }
    }
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
            Some(Urc::ListFile(file)) => log::info!("File: {:?}", file),
            Some(e) => error!("Unknown URC {:?}", e),
            None => info!("Waiting for response..."),
        }
    }

    // Remove old certs
    for name in ["ca.crt", "dvt.crt", "dvt.key"] {
        let _ = client
            .send(&FileDel {
                name: heapless::String::from_str(name).unwrap(),
            })
            .await;
        info!("Deleted old {}", name);
    }

    // Upload helper
    async fn upload_file(
        client: &mut Client<'static, UartTx<'static, Async>, 1024>,
        name: &str,
        content: &[u8],
        raw_data: &mut heapless::Vec<u8, 4096>,
    ) {
        let _ = client
            .send(&FileUpl {
                name: heapless::String::from_str(name).unwrap(),
                size: content.len() as u32,
            })
            .await;

        let mut offset = 0;
        while offset < content.len() {
            // vẫn giới hạn chunk nhỏ
            let chunk_len = (content.len() - offset).min(1024);
            raw_data.clear();
            raw_data
                .extend_from_slice(&content[offset..offset + chunk_len])
                .unwrap();

            let _ = client
                .send(&SendRawData {
                    raw_data: raw_data.clone(),
                    len: chunk_len,
                })
                .await;
            offset += chunk_len;
        }

        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
    }

    // Upload certs
    info!("Uploading CA cert...");
    upload_file(client, "ca.crt", ca_chain, &mut raw_data).await;

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
) -> ! {
    let mut state: State = State::ResetHardware;
    let mut is_connected = false;
    let ca_chain = include_str!("../../certs/ca.crt").as_bytes();
    let certificate = include_str!("../../certs/dvt.crt").as_bytes();
    let private_key = include_str!("../../certs/dvt.key").as_bytes();

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
                if handle_publish_mqtt_data(&mut client, MQTT_CLIENT_ID).await {
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
