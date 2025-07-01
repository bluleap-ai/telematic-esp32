use crate::cfg::net_cfg::*;
use crate::net::atcmd::general::*;
use crate::net::atcmd::response::*;
use crate::net::atcmd::Urc;
use crate::task::can::TwaiOutbox;
use crate::task::netmgr::{
    ActiveConnection, ConnectionEvent, ACTIVE_CONNECTION_CHAN_LTE, CONN_EVENT_CHAN,
};
use crate::util::time::utc_date_to_unix_timestamp;
use atat::{
    asynch::{AtatClient, Client},
    AtatIngress, DefaultDigester, Ingress, UrcChannel,
};
use core::sync::atomic::{AtomicBool, Ordering};
use core::{fmt::Debug, fmt::Write, str::FromStr};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::gpio::Output;
use esp_hal::uart::{UartRx, UartTx};
use esp_hal::Async;
use heapless::String;
use log::{debug, error, info, trace, warn};
use serde::{Deserialize, Serialize};

// Network registration status constants
const REGISTERED_HOME: u8 = 1;
const REGISTERED_ROAMING: u8 = 5;
const UNREGISTERED_SEARCHING: u8 = 2;
const REGISTRATION_DENIED: u8 = 3;
const REGISTRATION_FAILED: u8 = 4;

// Atomic boolean for tracking LTE connection status
static IS_LTE: AtomicBool = AtomicBool::new(false);

/// Error types for Quectel modem operations.
#[derive(Debug)]
pub enum QuectelError {
    /// Indicates a failure in executing an AT command.
    CommandFailed,
    /// Indicates a failure to connect to the MQTT broker.
    MqttConnectionFailed,
    /// Indicates a failure to publish MQTT data.
    MqttPublishFailed,
    /// Indicates a failure in network registration.
    NetworkRegistrationFailed,
}

/// Error types for MQTT connection operations.
#[derive(Debug, PartialEq)]
pub enum MqttConnectError {
    CommandFailed,
    StringConversion,
    Timeout,
    ModemError(u8),
}

/// Data structure for GPS-related trip data.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TripData {
    device_id: String<36>,
    trip_id: String<36>,
    latitude: f64,
    longitude: f64,
    timestamp: u64,
}

/// Placeholder struct for CAN frame data (replace with actual definition from crate::task::can).
#[derive(Debug, Serialize, Deserialize)]
pub struct CanFrame {
    pub id: u32,
    pub len: u8,
    pub data: [u8; 8],
}

/// States for the Quectel modem state machine.
#[derive(Debug)]
pub enum State {
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

/// Quectel modem driver for managing communication and GPS functionality.
pub struct Quectel {
    client: Client<'static, UartTx<'static, Async>, 1024>,
    pen: Output<'static>,
    dtr: Output<'static>,
    urc_channel: &'static UrcChannel<Urc, 128, 3>,
    is_connected: bool,
}

impl Quectel {
    /// Creates a new instance of the Quectel modem driver.
    ///
    /// # Arguments
    ///
    /// * `client` - The AT command client for sending commands to the modem.
    /// * `pen` - GPIO output pin for controlling modem power.
    /// * `dtr` - GPIO output pin for controlling modem data terminal ready (DTR) signal.
    /// * `urc_channel` - Channel for handling unsolicited result codes (URCs) from the modem.
    ///
    /// # Returns
    ///
    /// A new `Quectel` instance configured with the provided parameters.
    pub fn new(
        client: Client<'static, UartTx<'static, Async>, 1024>,
        pen: Output<'static>,
        dtr: Output<'static>,
        urc_channel: &'static UrcChannel<Urc, 128, 3>,
    ) -> Self {
        Self {
            client,
            pen,
            dtr,
            urc_channel,
            is_connected: false,
        }
    }

    /// Runs the Quectel modem state machine.
    ///
    /// This function manages the modem's state transitions, executing each state in sequence
    /// and handling errors by transitioning to the error state. It runs indefinitely, with
    /// a 1-second delay between state transitions.
    ///
    /// # Arguments
    ///
    /// * `mqtt_client_id` - The MQTT client identifier for publishing data.
    /// * `ca_chain` - The CA certificate chain for MQTT authentication.
    /// * `certificate` - The client certificate for MQTT authentication.
    /// * `private_key` - The client private key for MQTT authentication.
    /// * `gps_channel` - Channel for receiving GPS-related TripData.
    /// * `can_channel` - Channel for receiving CAN-related TripData (TWAI outbox).
    ///
    /// # Returns
    ///
    /// This function runs indefinitely and does not return.
    pub async fn run_quectel(
        &mut self,
        mqtt_client_id: &str,
        ca_chain: &'static [u8],
        certificate: &'static [u8],
        private_key: &'static [u8],
        gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
        can_channel: &'static TwaiOutbox,
    ) -> ! {
        let mut state = State::ResetHardware;
        loop {
            match state {
                State::ResetHardware => {
                    if self.reset_hardware().await.is_ok() {
                        state = State::DisableEchoMode;
                    }
                }
                State::DisableEchoMode => {
                    if self.disable_echo_mode().await.is_ok() {
                        state = State::GetModelId;
                    }
                }
                State::GetModelId => {
                    if self.get_model_id().await.is_ok() {
                        state = State::GetSoftwareVersion;
                    }
                }
                State::GetSoftwareVersion => {
                    if self.get_software_version().await.is_ok() {
                        state = State::GetSimCardStatus;
                    }
                }
                State::GetSimCardStatus => {
                    if self.get_sim_card_status().await.is_ok() {
                        state = State::GetNetworkSignalQuality;
                    }
                }
                State::GetNetworkSignalQuality => {
                    if self.get_network_signal_quality().await.is_ok() {
                        state = State::GetNetworkInfo;
                    }
                }
                State::GetNetworkInfo => {
                    if self.get_network_info().await.is_ok() {
                        state = State::EnableGps;
                    }
                }
                State::EnableGps => {
                    if self.enable_gps().await.is_ok() {
                        state = State::EnableAssistGps;
                    }
                }
                State::EnableAssistGps => {
                    if self.enable_assist_gps().await.is_ok() {
                        state = State::SetModemFunctionality;
                    }
                }
                State::SetModemFunctionality => {
                    if self.set_modem_functionality().await.is_ok() {
                        state = State::UploadMqttCert;
                    }
                }
                State::UploadMqttCert => {
                    if self
                        .upload_mqtt_cert(ca_chain, certificate, private_key)
                        .await
                        .is_ok()
                    {
                        state = State::CheckNetworkRegistration;
                    } else {
                        state = State::ErrorConnection;
                    }
                }
                State::CheckNetworkRegistration => {
                    if self.check_network_registration().await.is_ok() {
                        state = State::MqttOpenConnection;
                    } else {
                        state = State::ErrorConnection;
                    }
                }
                State::MqttOpenConnection => {
                    if self.mqtt_open_connection().await.is_ok() {
                        state = State::MqttConnectBroker;
                    } else {
                        state = State::ErrorConnection;
                    }
                }
                State::MqttConnectBroker => {
                    if self.mqtt_connect_broker().await.is_ok() {
                        state = State::MqttPublishData;
                    } else {
                        state = State::ErrorConnection;
                    }
                }
                State::MqttPublishData => {
                    if self
                        .mqtt_publish_data(mqtt_client_id, gps_channel, can_channel)
                        .await
                        .is_ok()
                    {
                        state = State::MqttPublishData; // Loop in publishing state
                    } else {
                        state = State::ErrorConnection;
                    }
                }
                State::ErrorConnection => {
                    let _ = self.error_connection().await;
                    state = State::ResetHardware;
                }
            }
            Timer::after(Duration::from_secs(1)).await;
        }
    }

    /// Resets the modem hardware by toggling the power pin.
    ///
    /// Sets the power pin low for 1 second and then high, followed by a 5-second delay
    /// to allow the modem to complete its reset process.
    ///
    /// # Returns
    ///
    /// * `Ok(())` on successful reset.
    pub async fn reset_hardware(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Reset Hardware");
        self.pen.set_low();
        Timer::after(Duration::from_secs(1)).await;
        self.pen.set_high();
        Timer::after(Duration::from_secs(5)).await;
        Ok(())
    }
    /// Disables echo mode on the modem.
    ///
    /// Sends an AT command to disable command echoing, preventing the modem from repeating
    /// commands in its responses.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the command succeeds.
    /// * `Err(QuectelError::CommandFailed)` if the command fails.
    pub async fn disable_echo_mode(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Disable Echo Mode");
        if check_result(self.client.send(&DisableEchoMode).await) {
            Ok(())
        } else {
            Err(QuectelError::CommandFailed)
        }
    }

    /// Retrieves the modem's model ID.
    ///
    /// Sends an AT command to query the modem's model identification.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the command succeeds.
    /// * `Err(QuectelError::CommandFailed)` if the command fails.
    pub async fn get_model_id(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Get Model Id");
        if check_result(self.client.send(&GetModelId).await) {
            Ok(())
        } else {
            Err(QuectelError::CommandFailed)
        }
    }

    /// Retrieves the modem's software version.
    ///
    /// Sends an AT command to query the modem's software version.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the command succeeds.
    /// * `Err(QuectelError::CommandFailed)` if the command fails.
    pub async fn get_software_version(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Get Software Version");
        if check_result(self.client.send(&GetSoftwareVersion).await) {
            Ok(())
        } else {
            Err(QuectelError::CommandFailed)
        }
    }

    /// Checks the SIM card status.
    ///
    /// Sends an AT command to verify the presence and status of the SIM card.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the command succeeds.
    /// * `Err(QuectelError::CommandFailed)` if the command fails.
    pub async fn get_sim_card_status(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Get Sim Card Status");
        if check_result(self.client.send(&GetSimCardStatus).await) {
            Ok(())
        } else {
            Err(QuectelError::CommandFailed)
        }
    }

    /// Retrieves the network signal quality.
    ///
    /// Sends an AT command to query the signal quality of the network connection.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the command succeeds or fails (logs failure but does not propagate error).
    pub async fn get_network_signal_quality(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Get Network Signal Quality");
        if check_result(self.client.send(&GetNetworkSignalQuality).await) {
            Ok(())
        } else {
            info!("[Quectel] Failed to get network signal quality");
            Ok(())
        }
    }
    /// Retrieves network information.
    ///
    /// Sends an AT command to query network-related information.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the command succeeds.
    /// * `Err(QuectelError::CommandFailed)` if the command fails.
    pub async fn get_network_info(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Get Network Info");
        if check_result(self.client.send(&GetNetworkInfo).await) {
            Ok(())
        } else {
            Err(QuectelError::CommandFailed)
        }
    }

    /// Enables GPS functionality on the modem.
    ///
    /// Sends an AT command to enable the GPS module.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the command succeeds.
    /// * `Err(QuectelError::CommandFailed)` if the command fails.
    pub async fn enable_gps(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Enable GPS");
        if check_result(self.client.send(&EnableGpsFunc).await) {
            Ok(())
        } else {
            Err(QuectelError::CommandFailed)
        }
    }

    /// Enables assisted GPS functionality.
    ///
    /// Sends an AT command to enable assisted GPS for faster location fixes.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the command succeeds.
    /// * `Err(QuectelError::CommandFailed)` if the command fails.
    pub async fn enable_assist_gps(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Enable Assist GPS");
        if check_result(self.client.send(&EnableAssistGpsFunc).await) {
            Ok(())
        } else {
            Err(QuectelError::CommandFailed)
        }
    }

    /// Sets the modem's functionality level.
    ///
    /// Sends an AT command to configure the modem's functionality to full mode.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the command succeeds.
    /// * `Err(QuectelError::CommandFailed)` if the command fails.
    pub async fn set_modem_functionality(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Set Modem Functionality");
        if check_result(
            self.client
                .send(&SetUeFunctionality {
                    fun: FunctionalityLevelOfUE::Full,
                })
                .await,
        ) {
            Ok(())
        } else {
            Err(QuectelError::CommandFailed)
        }
    }

    /// Uploads MQTT certificates to the modem.
    ///
    /// Uploads the CA chain, certificate, and private key for secure MQTT communication.
    ///
    /// # Arguments
    ///
    /// * `ca_chain` - The CA certificate chain as a byte slice.
    /// * `certificate` - The client certificate as a byte slice.
    /// * `private_key` - The client private key as a byte slice.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the upload succeeds.
    /// * `Err(QuectelError::CommandFailed)` if the upload fails.
    pub async fn upload_mqtt_cert(
        &mut self,
        ca_chain: &'static [u8],
        certificate: &'static [u8],
        private_key: &'static [u8],
    ) -> Result<(), QuectelError> {
        info!("[Quectel] Upload Files");
        if upload_mqtt_cert_files(
            &mut self.client,
            self.urc_channel,
            ca_chain,
            certificate,
            private_key,
        )
        .await
        {
            Ok(())
        } else {
            error!("[Quectel] File upload failed, resetting hardware");
            Err(QuectelError::CommandFailed)
        }
    }

    /// Checks network registration status.
    ///
    /// Verifies if the modem is registered to the network and updates the connection state.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the modem is registered.
    /// * `Err(QuectelError::NetworkRegistrationFailed)` if registration fails or times out.
    pub async fn check_network_registration(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Check Network Registration");
        if check_network_registration(&mut self.client).await {
            if !self.is_connected {
                let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteRegistered);
                self.is_connected = true;
            }
            Ok(())
        } else {
            if self.is_connected {
                let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteUnregistered);
                self.is_connected = false;
            }
            error!("[Quectel] Network registration failed, resetting hardware");
            Err(QuectelError::NetworkRegistrationFailed)
        }
    }

    /// Opens an MQTT connection.
    ///
    /// Initiates an MQTT connection to the broker.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the connection is opened successfully.
    /// * `Err(QuectelError::MqttConnectionFailed)` if the connection fails.
    pub async fn mqtt_open_connection(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Opening MQTT connection");
        open_mqtt_connection(&mut self.client, self.urc_channel)
            .await
            .map_err(|e| {
                error!("[Quectel] Failed to open MQTT connection: {e:?}");
                QuectelError::MqttConnectionFailed
            })?;
        info!("[Quectel] MQTT connection opened successfully");
        Ok(())
    }

    /// Connects to the MQTT broker.
    ///
    /// Establishes a connection to the MQTT broker and updates the connection state.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the connection is established.
    /// * `Err(QuectelError::MqttConnectionFailed)` if the connection fails.
    pub async fn mqtt_connect_broker(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Connecting to MQTT broker");
        connect_mqtt_broker(&mut self.client, self.urc_channel)
            .await
            .map_err(|e| {
                error!("[Quectel] MQTT connection failed: {e:?}");
                let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteDisconnected);
                QuectelError::MqttConnectionFailed
            })?;
        info!("[Quectel] MQTT connection established");
        let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteConnected);
        Ok(())
    }

    /// Publishes data to the MQTT broker.
    ///
    /// Publishes GPS and CAN data to the MQTT broker.
    ///
    /// # Arguments
    ///
    /// * `mqtt_client_id` - The MQTT client identifier.
    /// * `gps_channel` - Channel for receiving GPS-related TripData.
    /// * `can_channel` - Channel for receiving CAN-related TripData (TWAI outbox).
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the data is published successfully.
    /// * `Err(QuectelError::MqttPublishFailed)` if publishing fails.
    pub async fn mqtt_publish_data(
        &mut self,
        mqtt_client_id: &str,
        gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
        can_channel: &'static TwaiOutbox,
    ) -> Result<(), QuectelError> {
        info!("[Quectel] Publishing MQTT Data");
        if handle_publish_mqtt_data(&mut self.client, mqtt_client_id, gps_channel, can_channel)
            .await
        {
            info!("[Quectel] MQTT data published successfully");
            Ok(())
        } else {
            error!("[Quectel] MQTT publish failed");
            Err(QuectelError::MqttPublishFailed)
        }
    }

    /// Handles error recovery by resetting the modem.
    ///
    /// Updates the connection state and initiates a hardware reset after a 5-second delay.
    ///
    /// # Returns
    ///
    /// * `Ok(())` after initiating the reset process.
    pub async fn error_connection(&mut self) -> Result<(), QuectelError> {
        if self.is_connected {
            let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteDisconnected);
            self.is_connected = false;
        }
        error!("[Quectel] System in error state - attempting recovery");
        Timer::after(Duration::from_secs(5)).await;
        self.reset_hardware().await
    }
}

/// Uploads MQTT certificate files to the modem.
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
    let now = Instant::now();
    while now.elapsed().as_secs() < 10 {
        Timer::after(Duration::from_secs(1)).await;
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
                name: String::from_str(name).unwrap(),
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
        // Sending file upload command to notify the modem about the file to be uploaded
        let name_str = match String::from_str(name) {
            Ok(s) => s,
            Err(_) => {
                error!("Heapless string overflow for file name: {name}");
                return false;
            }
        };
        // Notify the modem about the file to be uploaded
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
        // Uploading data payload in 1 Kib of chunks
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

        Timer::after(Duration::from_secs(1)).await;
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
            name: String::from_str("recv/mode").unwrap(),
            param_1: Some(0),
            param_2: Some(0),
            param_3: Some(1),
        })
        .await;
    let _ = client
        .send(&MqttConfig {
            name: String::from_str("SSL").unwrap(),
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
                name: String::from_str(cfg_name).unwrap(),
                context_id: 2,
                cert_path: Some(String::from_str(path).unwrap()),
            })
            .await;
    }

    let _ = client
        .send(&SslConfigOther {
            name: String::from_str("seclevel").unwrap(),
            context_id: 2,
            level: 2,
        })
        .await;
    let _ = client
        .send(&SslConfigOther {
            name: String::from_str("sslversion").unwrap(),
            context_id: 2,
            level: 4,
        })
        .await;
    let _ = client.send(&SslSetCipherSuite).await;
    let _ = client
        .send(&SslConfigOther {
            name: String::from_str("ignorelocaltime").unwrap(),
            context_id: 2,
            level: 1,
        })
        .await;
    let _ = client
        .send(&MqttConfig {
            name: String::from_str("version").unwrap(),
            param_1: Some(0),
            param_2: Some(4),
            param_3: None,
        })
        .await;

    true
}

/// Checks the network registration status of the modem.
pub async fn check_network_registration(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
) -> bool {
    let timeout: Duration = Duration::from_secs(30); // 30 seconds timeout
    let start_time = Instant::now();

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
                        Timer::after(Duration::from_secs(1)).await;
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

/// Opens an MQTT connection to the broker.
pub async fn open_mqtt_connection(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
    urc_channel: &'static UrcChannel<Urc, 128, 3>,
) -> Result<(), MqttConnectError> {
    // Create server string safely
    let server =
        String::from_str(MQTT_SERVER_NAME).map_err(|_| MqttConnectError::StringConversion)?;

    // Send MQTT open command
    client
        .send(&MqttOpen {
            link_id: 0,
            server,
            port: MQTT_SERVER_PORT,
        })
        .await
        .map_err(|_| MqttConnectError::CommandFailed)?;

    info!("[Quectel] MQTT open command sent, waiting for response...");

    let mut subscriber = urc_channel
        .subscribe()
        .map_err(|_| MqttConnectError::CommandFailed)?;

    let start = Instant::now();
    const TIMEOUT: Duration = Duration::from_secs(30);

    loop {
        // Check timeout first
        if start.elapsed() >= TIMEOUT {
            error!("[Quectel] MQTT open timed out");
            return Err(MqttConnectError::Timeout);
        }

        Timer::after(Duration::from_secs(1)).await;

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

/// Connects to the MQTT broker.
pub async fn connect_mqtt_broker(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
    urc_channel: &'static UrcChannel<Urc, 128, 3>,
) -> Result<(), MqttConnectError> {
    const MAX_RETRIES: usize = 3;
    const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
    const CLIENT_ID: &str = "telematics-control-unit";

    // Create credentials with proper error handling
    let username =
        String::<64>::from_str(MQTT_USR_NAME).map_err(|_| MqttConnectError::StringConversion)?;
    let password = String::<64>::from_str(MQTT_USR_NAME) // Note: Same as username - is this intentional?
        .map_err(|_| MqttConnectError::StringConversion)?;
    let client_id =
        String::<23>::from_str(CLIENT_ID).map_err(|_| MqttConnectError::StringConversion)?;

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
                Timer::after(Duration::from_secs(1)).await;
            }
        }
    }

    // Wait for connection acknowledgement
    let mut subscriber = urc_channel
        .subscribe()
        .map_err(|_| MqttConnectError::CommandFailed)?;
    let start = Instant::now();

    loop {
        if start.elapsed() > RESPONSE_TIMEOUT {
            error!("[Quectel] MQTT connect timeout");
            return Err(MqttConnectError::Timeout);
        }

        Timer::after(Duration::from_millis(100)).await;

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

/// Publishes GPS and CAN data to the MQTT broker.
pub async fn handle_publish_mqtt_data(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
    mqtt_client_id: &str,
    gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    can_channel: &'static TwaiOutbox,
) -> bool {
    if let Ok(active_connection) = ACTIVE_CONNECTION_CHAN_LTE.receiver().try_receive() {
        IS_LTE.store(active_connection == ActiveConnection::Lte, Ordering::SeqCst);
        info!("[LTE] Updated IS_LTE: {}", IS_LTE.load(Ordering::SeqCst));
    }

    // If LTE is not active, return true to avoid error state
    if !IS_LTE.load(Ordering::SeqCst) {
        info!("[LTE] LTE not active, skipping MQTT publish");
        return true;
    }

    let mut trip_topic: String<128> = String::new();
    let mut trip_payload: String<1024> = String::new();
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
            let mut device_id = String::new();
            let mut trip_id = String::new();
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
                    is_gps_success = true;
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
    let mut can_topic: String<128> = String::new();
    let mut can_payload: String<1024> = String::new();
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
                is_can_success = true;
            } else {
                error!("[LTE] Failed to publish CAN data");
                is_can_success = false;
            }
        } else {
            error!("[LTE] Failed to serialize CAN data");
        }
    }

    is_can_success && is_gps_success
}

/// Task for handling incoming data from the modem's UART interface.
#[embassy_executor::task]
pub async fn rx_handler(
    mut ingress: Ingress<'static, DefaultDigester<Urc>, Urc, 1024, 128, 3>,
    mut reader: UartRx<'static, Async>,
) -> ! {
    ingress.read_from(&mut reader).await
}

/// Checks the result of an AT command execution.
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
