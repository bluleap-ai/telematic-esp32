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
use esp_println::print;
use log::{debug, error, info, trace, warn};
use serde::{Deserialize, Serialize};

use crate::svc::atcmd::general::*;
use crate::svc::atcmd::response::*;
use crate::svc::atcmd::Urc;

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
        "channels/{}/messages/client/trip",
        mqtt_client_id
    )
    .unwrap();

    match client.send(&RetrieveGpsRmc).await {
        Ok(res) => {
            info!("GPS RMC data received: {:?}", res);
            let timestamp = utc_date_to_unix_timestamp(&res.utc, &res.date);
            let mut device_id = heapless::String::new();
            let mut trip_id = heapless::String::new();
            write!(&mut trip_id, "{}", mqtt_client_id).unwrap();
            write!(&mut device_id, "{}", mqtt_client_id).unwrap();

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
                    error!("Payload buffer overflow");
                    return false;
                }

                info!("MQTT payload: {}", payload);
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
                error!("Failed to serialize trip data");
                false
            }
        }
        Err(e) => {
            warn!("Failed to retrieve GPS data: {:?}", e);
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
            info!("\t Command succeeded: {:?}", value);
            true
        }
        Err(e) => {
            error!("Failed to send AT command: {:?}", e);
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
    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
        match subscriber.try_next_message_pure() {
            Some(Urc::ListFile(file)) => {
                log::info!("File: {:?}", file);
            }
            Some(e) => {
                error!("Unknown URC {:?}", e);
            }
            None => {
                info!("Waiting for response...");
            }
        }
        if now.elapsed().as_secs() > 10 {
            break;
        }
    }
    info!("Quectel: remove CA_CRT path");
    let _ = client
        .send(&FileDel {
            name: heapless::String::from_str("crt.pem").unwrap(),
        })
        .await;
    info!("Quectel: remove CLIENT_CRT path");
    let _ = client
        .send(&FileDel {
            name: heapless::String::from_str("dvt.crt").unwrap(),
        })
        .await;
    info!("Quectel: remove CLIENT_KEY path");
    let _ = client
        .send(&FileDel {
            name: heapless::String::from_str("dvt.key").unwrap(),
        })
        .await;
    // Upload CA cert
    info!("Quectel: Upload MQTT certs to quectel");
    let _ = raw_data.extend_from_slice(&ca_chain[0..1024]);
    let _ = client
        .send(&FileUpl {
            name: heapless::String::from_str("crt.pem").unwrap(),
            size: 2574,
        })
        .await;
    let _ = client
        .send(&SendRawData {
            raw_data: raw_data.clone(),
            len: 1024,
        })
        .await;
    raw_data.clear();
    let _ = raw_data.extend_from_slice(&ca_chain[1024..2048]);
    let _ = client
        .send(&SendRawData {
            raw_data: raw_data.clone(),
            len: 1024,
        })
        .await;
    raw_data.clear();
    let _ = raw_data.extend_from_slice(&ca_chain[2048..]);
    let _ = client
        .send(&SendRawData {
            raw_data: raw_data.clone(),
            len: 526,
        })
        .await;
    embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
    // Upload client cert
    let _ = client
        .send(&FileUpl {
            name: heapless::String::from_str("dvt.crt").unwrap(),
            size: 1268,
        })
        .await;
    raw_data.clear();
    let _ = raw_data.extend_from_slice(&certificate[0..1024]);
    let _ = client
        .send(&SendRawData {
            raw_data: raw_data.clone(),
            len: 1024,
        })
        .await;
    raw_data.clear();
    let _ = raw_data.extend_from_slice(&certificate[1024..]);
    let _ = client
        .send(&SendRawData {
            raw_data: raw_data.clone(),
            len: 244,
        })
        .await;
    embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
    // Upload client key
    let _ = client
        .send(&FileUpl {
            name: heapless::String::from_str("dvt.key").unwrap(),
            size: 1678,
        })
        .await;
    raw_data.clear();
    let _ = raw_data.extend_from_slice(&private_key[0..1024]);
    let _ = client
        .send(&SendRawData {
            raw_data: raw_data.clone(),
            len: 1024,
        })
        .await;
    raw_data.clear();
    let _ = raw_data.extend_from_slice(&private_key[1024..]);
    let _ = client
        .send(&SendRawData {
            raw_data: raw_data.clone(),
            len: 654,
        })
        .await;

    info!("Quectel: set MQTTS configuration");
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
    let _ = client
        .send(&SslConfigCert {
            name: heapless::String::from_str("cacert").unwrap(),
            context_id: 2,
            cert_path: Some(heapless::String::from_str("UFS:crt.pem").unwrap()),
        })
        .await;
    let _ = client
        .send(&SslConfigCert {
            name: heapless::String::from_str("clientcert").unwrap(),
            context_id: 2,
            cert_path: Some(heapless::String::from_str("UFS:dvt.crt").unwrap()),
        })
        .await;
    let _ = client
        .send(&SslConfigCert {
            name: heapless::String::from_str("clientkey").unwrap(),
            context_id: 2,
            cert_path: Some(heapless::String::from_str("UFS:dvt.key").unwrap()),
        })
        .await;
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
                log::info!("EPS network registration status: {:?}", status);

                match status.stat {
                    REGISTERED_HOME => {
                        let elapsed = start_time.elapsed().as_secs();
                        info!("Registered (Home) after {} seconds", elapsed);
                        return true; // Successfully registered
                    }
                    UNREGISTERED_SEARCHING => {
                        print!("."); // Indicating ongoing search
                        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
                    }
                    REGISTRATION_DENIED => {
                        error!("Registration denied");
                        return false; // Registration denied
                    }
                    REGISTRATION_FAILED => {
                        error!("Registration failed");
                        return false; // Registration failed
                    }
                    REGISTERED_ROAMING => {
                        let elapsed = start_time.elapsed().as_secs();
                        info!("Registered (Roaming) after {} seconds", elapsed);
                        return true; // Successfully registered
                    }
                    _ => {
                        error!("Unknown registration status: {}", status.stat);
                        return false; // Unknown status
                    }
                }
            }
            Err(e) => {
                error!("Failed to get EPS network registration status: {:?}", e);
                return false; // Error occurred
            }
        }
    }

    // Timeout reached without successful registration
    error!("Network registration timed out");
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

    info!("MQTT open command sent, waiting for response...");

    let mut subscriber = urc_channel
        .subscribe()
        .map_err(|_| MqttConnectError::CommandFailed)?; // Optionally log the error here for more info

    let start = embassy_time::Instant::now();
    const TIMEOUT: embassy_time::Duration = embassy_time::Duration::from_secs(30);

    loop {
        // Check timeout first
        if start.elapsed() >= TIMEOUT {
            error!("MQTT open timed out");
            return Err(MqttConnectError::Timeout);
        }

        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;

        match subscriber.try_next_message_pure() {
            Some(Urc::MqttOpen(response)) => {
                info!("Received MQTT open response: {:?}", response);
                return match response.result {
                    0 => Ok(()),
                    code => {
                        error!("Modem reported error code: {}", code as u8);
                        Err(MqttConnectError::ModemError(code as u8))
                    }
                };
            }
            Some(other_urc) => {
                info!("Received unrelated URC: {:?}", other_urc);
                // Continue waiting for MQTT open response
            }
            None => {
                warn!("No URC received yet...");
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
    let password = heapless::String::<64>::from_str(MQTT_USR_NAME) // Note: Same as username - is this intentional?
        .map_err(|_| MqttConnectError::StringConversion)?;
    let client_id = heapless::String::<23>::from_str(CLIENT_ID)
        .map_err(|_| MqttConnectError::StringConversion)?;

    // Send connect command with retries
    for attempt in 1..=MAX_RETRIES {
        info!("MQTT connect attempt {}/{}", attempt, MAX_RETRIES);

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
                error!("Final connect attempt failed: {:?}", e);
                return Err(MqttConnectError::CommandFailed);
            }
            Err(e) => {
                warn!("Connect attempt failed: {:?} - retrying", e);
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
            error!("MQTT connect timeout");
            return Err(MqttConnectError::Timeout);
        }

        embassy_time::Timer::after(embassy_time::Duration::from_millis(100)).await;

        match subscriber.try_next_message_pure() {
            Some(Urc::MqttConnect(response)) => {
                info!("Received MQTT connect response: {:?}", response);
                return match response.result {
                    0 => Ok(()),
                    code => {
                        error!("Modem connection error: {}", code);
                        Err(MqttConnectError::ModemError(code))
                    }
                };
            }
            Some(other_urc) => {
                debug!("Ignoring unrelated URC: {:?}", other_urc);
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
    let ca_chain = include_str!("../../cert/crt.pem").as_bytes();
    let certificate = include_str!("../../cert/dvt.crt").as_bytes();
    let private_key = include_str!("../../cert/dvt.key").as_bytes();

    loop {
        match state {
            State::ResetHardware => {
                // 0: Reset Hardware
                info!("Quectel: Reset Hardware");
                reset_modem(&mut pen).await;
                state = State::DisableEchoMode;
            }
            State::DisableEchoMode => {
                info!("Quectel: Disable Echo Mode");
                if check_result(client.send(&DisableEchoMode).await) {
                    state = State::GetModelId;
                }
            }
            State::GetModelId => {
                info!("Quectel: Get Model Id");
                if check_result(client.send(&GetModelId).await) {
                    state = State::GetSoftwareVersion;
                }
            }
            State::GetSoftwareVersion => {
                info!("Quectel: Get Software Version");
                if check_result(client.send(&GetSoftwareVersion).await) {
                    state = State::GetSimCardStatus;
                }
            }
            State::GetSimCardStatus => {
                info!("Quectel: Get Sim Card Status");
                if check_result(client.send(&GetSimCardStatus).await) {
                    state = State::GetNetworkSignalQuality;
                }
            }
            State::GetNetworkSignalQuality => {
                info!("Quectel: Get Network Signal Quality");
                if check_result(client.send(&GetNetworkSignalQuality).await) {
                    state = State::GetNetworkInfo;
                }
            }
            State::GetNetworkInfo => {
                info!("Quectel: Get Network Info");
                if check_result(client.send(&GetNetworkInfo).await) {
                    state = State::EnableGps;
                }
            }
            State::EnableGps => {
                info!("Quectel: Enable GPS");
                if check_result(client.send(&EnableGpsFunc).await) {
                    state = State::EnableAssistGps;
                }
            }
            State::EnableAssistGps => {
                info!("Quectel: Enable Assist GPS");
                if check_result(client.send(&EnableAssistGpsFunc).await) {
                    state = State::SetModemFunctionality;
                }
            }
            State::SetModemFunctionality => {
                info!("Quectel: Set Modem Functionality");
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
                info!("Quectel: Upload Files");
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
                    error!("File upload failed, resetting hardware");
                    State::ErrorConnection
                };
            }
            State::CheckNetworkRegistration => {
                info!("Quectel: Check Network Registration");
                let res = check_network_registration(&mut client).await;
                state = if res {
                    State::MqttOpenConnection
                } else {
                    error!("Network registration failed, resetting hardware");
                    State::ErrorConnection
                };
            }
            State::MqttOpenConnection => {
                info!("Opening MQTT connection");
                match open_mqtt_connection(&mut client, urc_channel).await {
                    Ok(_) => {
                        info!("MQTT connection opened successfully");
                        state = State::MqttConnectBroker;
                    }
                    Err(e) => {
                        error!("Failed to open MQTT connection: {:?}", e);
                    }
                }
            }
            State::MqttConnectBroker => {
                info!("Connecting to MQTT broker");
                match connect_mqtt_broker(&mut client, urc_channel).await {
                    Ok(_) => {
                        info!("MQTT connection established");
                        state = State::MqttPublishData;
                    }
                    Err(e) => {
                        error!("MQTT connection failed: {:?}", e);
                    }
                }
            }
            State::MqttPublishData => {
                info!("Quectel: Publishing MQTT Data");
                if handle_publish_mqtt_data(&mut client, MQTT_CLIENT_ID).await {
                    info!("MQTT data published successfully");
                    // Transition to next state or maintain publishing state
                    state = State::MqttPublishData;
                } else {
                    error!("MQTT publish failed");
                }
            }
            State::ErrorConnection => {
                error!("System in error state - attempting recovery");
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
