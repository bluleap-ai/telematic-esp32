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
const REGISTERED_HOME: u8 = 1; // Modem registered on home network
const REGISTERED_ROAMING: u8 = 5; // Modem registered on roaming network
const UNREGISTERED_SEARCHING: u8 = 2; // Modem searching for network
const REGISTRATION_DENIED: u8 = 3; // Network registration denied
const REGISTRATION_FAILED: u8 = 4; // Network registration failed

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
    /// Indicates a failure in executing an MQTT command.
    CommandFailed,
    /// Indicates a failure to convert data to a string.
    StringConversion,
    /// Indicates a timeout during MQTT connection.
    Timeout,
    /// Indicates a modem-specific error with an error code.
    ModemError(u8),
}

/// Data structure for GPS-related trip data.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TripData {
    /// Device identifier (up to 36 characters).
    pub device_id: String<36>,
    /// Trip identifier (up to 36 characters).
    pub trip_id: String<36>,
    /// Latitude in decimal degrees.
    pub latitude: f64,
    /// Longitude in decimal degrees.
    pub longitude: f64,
    /// Unix timestamp of the GPS data.
    pub timestamp: u64,
}

/// States for the Quectel modem state machine.
#[derive(Debug)]
pub enum State {
    /// Resets the modem hardware.
    ResetHardware,
    /// Disables echo mode on the modem.
    DisableEchoMode,
    /// Retrieves the modem's model ID.
    GetModelId,
    /// Retrieves the modem's software version.
    GetSoftwareVersion,
    /// Checks the SIM card status.
    GetSimCardStatus,
    /// Retrieves the network signal quality.
    GetNetworkSignalQuality,
    /// Retrieves network information.
    GetNetworkInfo,
    /// Enables GPS functionality.
    EnableGps,
    /// Enables assisted GPS functionality.
    EnableAssistGps,
    /// Sets the modem's functionality level.
    SetModemFunctionality,
    /// Uploads MQTT certificates.
    UploadMqttCert,
    /// Checks network registration status.
    CheckNetworkRegistration,
    /// Opens an MQTT connection.
    MqttOpenConnection,
    /// Connects to the MQTT broker.
    MqttConnectBroker,
    /// Publishes GPS and CAN data to the MQTT broker.
    GetGPSData,
    /// Handles error recovery.
    ErrorConnection,
}

/// Quectel modem driver for managing communication and GPS functionality.
pub struct Quectel {
    /// AT command client for sending commands to the modem.
    client: Client<'static, UartTx<'static, Async>, 1024>,
    /// GPIO output pin for controlling modem power.
    pen: Output<'static>,
    /// GPIO output pin for controlling modem data terminal ready (DTR) signal.
    dtr: Output<'static>,
    /// Channel for handling unsolicited result codes (URCs) from the modem.
    urc_channel: &'static UrcChannel<Urc, 128, 3>,
    /// Tracks whether the modem is connected to the network.
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

    /// Initializes the Quectel modem hardware and basic configuration.
    ///
    /// Executes the initialization sequence from ResetHardware to GetNetworkInfo.
    /// Returns on success or if an error occurs during initialization.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if initialization completes successfully.
    /// * `Err(QuectelError)` if any step fails.
    ///
    pub async fn quectel_initialize(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Starting LTE initialization");
        let mut state = State::ResetHardware;

        loop {
            match state {
                State::ResetHardware => {
                    if self.reset_hardware().await.is_ok() {
                        info!("[Quectel] Modem hardware reset successfully");
                        state = State::DisableEchoMode;
                    } else {
                        error!("[Quectel] Modem init failed at ResetHardware");
                        return Err(QuectelError::CommandFailed);
                    }
                }
                State::DisableEchoMode => {
                    if self.disable_echo_mode().await.is_ok() {
                        info!("[Quectel] Echo mode disabled successfully");
                        state = State::GetModelId;
                    } else {
                        error!("[Quectel] Modem init failed at DisableEchoMode");
                        return Err(QuectelError::CommandFailed);
                    }
                }
                State::GetModelId => {
                    if self.get_model_id().await.is_ok() {
                        info!("[Quectel] Model ID retrieved successfully");
                        state = State::GetSoftwareVersion;
                    } else {
                        error!("[Quectel] Modem init failed at GetModelId");
                        return Err(QuectelError::CommandFailed);
                    }
                }
                State::GetSoftwareVersion => {
                    if self.get_software_version().await.is_ok() {
                        info!("[Quectel] Software version retrieved successfully");
                        return Ok(());
                    } else {
                        error!("[Quectel] Modem init failed at GetSoftwareVersion");
                        return Err(QuectelError::CommandFailed);
                    }
                }
                _ => {
                    error!("[Quectel] Invalid state in quectel_initialize: {:?}", state);
                    return Err(QuectelError::CommandFailed);
                }
            }
            Timer::after(Duration::from_secs(1)).await;
        }
    }

    /// Initializes the LTE-specific configuration and MQTT connection.
    ///
    /// Executes the LTE setup sequence from EnableGps to MqttConnectBroker.
    /// Assumes the modem is initialized via `quectel_initialize`.
    ///
    /// # Arguments
    ///
    /// * `mqtt_client_id` - The MQTT client identifier for connection.
    /// * `ca_chain` - The CA certificate chain for MQTT authentication.
    /// * `certificate` - The client certificate for MQTT authentication.
    /// * `private_key` - The client private key for MQTT authentication.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if LTE initialization completes successfully.
    /// * `Err(QuectelError)` if any step fails.
    pub async fn lte_initialize(
        &mut self,
        mqtt_client_id: &str,
        ca_chain: &'static [u8],
        certificate: &'static [u8],
        private_key: &'static [u8],
    ) -> Result<(), QuectelError> {
        info!("[Quectel] Starting modem initialization");
        let mut state = State::GetSimCardStatus;

        loop {
            match state {
                State::GetSimCardStatus => {
                    info!("[Quectel] Checking SIM card status");
                    if self.get_sim_card_status().await.is_ok() {
                        state = State::GetNetworkSignalQuality;
                    } else {
                        error!("[Quectel] Modem init failed at GetSimCardStatus");
                        return Err(QuectelError::CommandFailed);
                    }
                }
                State::GetNetworkSignalQuality => {
                    info!("[Quectel] Retrieving network signal quality");
                    if self.get_network_signal_quality().await.is_ok() {
                        state = State::GetNetworkInfo;
                    } else {
                        error!("[Quectel] Modem init failed at GetNetworkSignalQuality");
                        return Err(QuectelError::CommandFailed);
                    }
                }
                State::GetNetworkInfo => {
                    info!("[Quectel] Retrieving network information");
                    if self.get_network_info().await.is_ok() {
                        state = State::SetModemFunctionality;
                    } else {
                        error!("[Quectel] Modem init failed at GetNetworkInfo");
                        return Err(QuectelError::CommandFailed);
                    }
                }
                State::SetModemFunctionality => {
                    info!("[Quectel] Setting modem functionality");
                    if self.set_modem_functionality().await.is_ok() {
                        info!("[Quectel] Modem initialization completed");
                        state = State::UploadMqttCert;
                    } else {
                        error!("[Quectel] LTE init failed at SetModemFunctionality");
                        return Err(QuectelError::CommandFailed);
                    }
                }
                State::UploadMqttCert => {
                    let ca_chain = include_str!("../../certx/crt.pem").as_bytes();
                    let certificate = include_str!("../../certx/dvt.crt").as_bytes();
                    let private_key = include_str!("../../certx/dvt.key").as_bytes();
                    if self
                        .upload_mqtt_cert(ca_chain, certificate, private_key)
                        .await
                        .is_ok()
                    {
                        info!("[Quectel] MQTT certificates uploaded successfully");
                        state = State::CheckNetworkRegistration;
                    } else {
                        error!("[Quectel] LTE init failed at UploadMqttCert");
                        return Err(QuectelError::CommandFailed);
                    }
                }
                State::CheckNetworkRegistration => {
                    info!("[Quectel] Checking network registration");
                    if self.check_network_registration().await.is_ok() {
                        // state = State::MqttOpenConnection;
                        return Ok(());
                    } else {
                        error!("[Quectel] LTE init failed at CheckNetworkRegistration");
                        return Err(QuectelError::NetworkRegistrationFailed);
                    }
                }
                _ => {
                    error!("[Quectel] Invalid state in quectel_initialize: {:?}", state);
                    return Err(QuectelError::CommandFailed);
                }
            }
            Timer::after(Duration::from_secs(1)).await;
        }
    }
    pub async fn lte_handle_mqtt(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Starting LTE initialization");
        let mut state = State::MqttOpenConnection;

        loop {
            match state {
                State::MqttOpenConnection => {
                    info!("[Quectel] Opening MQTT connection");
                    if self.mqtt_open_connection().await.is_ok() {
                        state = State::MqttConnectBroker;
                    } else {
                        error!("[Quectel] LTE init failed at MqttOpenConnection");
                        return Err(QuectelError::MqttConnectionFailed);
                    }
                }
                State::MqttConnectBroker => {
                    info!("[Quectel] Connecting to MQTT broker");
                    if self.mqtt_connect_broker().await.is_ok() {
                        info!("[Quectel] LTE initialization completed");
                        return Ok(());
                    } else {
                        error!("[Quectel] LTE init failed at MqttConnectBroker");
                        return Err(QuectelError::MqttConnectionFailed);
                    }
                }
                _ => {
                    error!("[Quectel] Invalid state in lte_init: {:?}", state);
                    return Err(QuectelError::CommandFailed);
                }
            }
            Timer::after(Duration::from_secs(1)).await;
        }
    }
    pub async fn gps_initialize(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Starting GPS state machine");
        let mut state = State::GetGPSData;

        loop {
            match state {
                State::EnableGps => {
                    if self.enable_gps().await.is_ok() {
                        info!("[Quectel] GPS enabled successfully");
                        state = State::EnableAssistGps;
                    } else {
                        error!("[Quectel] GPS init failed at EnableGps");
                        state = State::ErrorConnection;
                    }
                }
                State::EnableAssistGps => {
                    if self.enable_assist_gps().await.is_ok() {
                        info!("[Quectel] Assist GPS enabled successfully");
                        return Ok(());
                    } else {
                        error!("[Quectel] GPS init failed at EnableAssistGps");
                        state = State::ErrorConnection;
                    }
                }
                State::ErrorConnection => {
                    let _ = self.error_connection().await;
                    state = State::GetGPSData;
                }
                _ => {
                    error!("[Quectel] Invalid state in gps_state_machine: {:?}", state);
                    state = State::ErrorConnection;
                }
            }
            Timer::after(Duration::from_secs(1)).await;
        }
    }
    /// Runs the GPS data retrieval state machine.
    ///
    /// Continuously retrieves GPS data using `get_gps` and handles errors by transitioning
    /// to the ErrorConnection state. Runs indefinitely with a 1-second delay between attempts.
    ///
    /// # Arguments
    ///
    /// * `mqtt_client_id` - The MQTT client identifier for GPS data.
    /// * `gps_channel` - Channel for sending GPS-related `TripData`.
    ///
    /// # Returns
    ///
    /// This function runs indefinitely and does not return.
    ///
    pub async fn gps_state_machine(
        &mut self,
        mqtt_client_id: &str,
        gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    ) -> ! {
        info!("[Quectel] Starting GPS state machine");
        let mut state = State::GetGPSData;

        loop {
            match state {
                State::GetGPSData => match self.get_gps(mqtt_client_id, gps_channel).await {
                    Ok(trip_data) => {
                        info!("[Quectel] GPS data retrieved: {:?}", trip_data);
                        state = State::GetGPSData;
                    }
                    Err(e) => {
                        error!("[Quectel] Failed to get GPS data: {:?}", e);
                        state = State::ErrorConnection;
                    }
                },
                State::ErrorConnection => {
                    let _ = self.error_connection().await;
                    state = State::GetGPSData;
                }
                _ => {
                    error!("[Quectel] Invalid state in gps_state_machine: {:?}", state);
                    state = State::ErrorConnection;
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
    /// * `Err(QuectelError::CommandFailed)` if the reset fails.
    pub async fn reset_hardware(&mut self) -> Result<(), QuectelError> {
        info!("[Quectel] Reset Hardware");
        self.pen.set_low();
        Timer::after(Duration::from_secs(1)).await;
        self.pen.set_high();
        Timer::after(Duration::from_secs(5)).await; // need to wait for the modem to reset but i think we can reduce this
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
    /// Uploads the CA chain, client certificate, and private key for secure MQTT communication.
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
    /// Sends connection events (`LteRegistered` or `LteUnregistered`) via `CONN_EVENT_CHAN`.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the modem is registered (home or roaming).
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

    /// Opens an MQTT connection to the broker.
    ///
    /// Initiates an MQTT connection using the configured server and port.
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
    /// Establishes a connection to the MQTT broker using the provided credentials and updates
    /// the connection state by sending `LteConnected` or `LteDisconnected` events via `CONN_EVENT_CHAN`.
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

    /// Retrieves GPS data from the modem and sends it to the GPS channel.
    ///
    /// Checks if LTE is active, sends the `RetrieveGpsRmc` command to the modem, processes
    /// the response to create a `TripData` struct, and sends it to the provided `gps_channel`.
    ///
    /// # Arguments
    ///
    /// * `mqtt_client_id` - The MQTT client identifier used for `device_id` and `trip_id`.
    /// * `gps_channel` - Channel for sending GPS-related `TripData`.
    ///
    /// # Returns
    ///
    /// * `Ok(TripData)` if GPS data is successfully retrieved and sent.
    /// * `Err(QuectelError::CommandFailed)` if the LTE is not active or the command fails.
    pub async fn get_gps(
        &mut self,
        mqtt_client_id: &str,
        gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    ) -> Result<TripData, QuectelError> {
        info!("[Quectel] Retrieving GPS data");

        // Send RetrieveGpsRmc command
        let trip_result = self.client.send(&RetrieveGpsRmc).await;
        match trip_result {
            Ok(res) => {
                info!("[Quectel] GPS RMC data received: {res:?}");

                // Convert UTC date and time to Unix timestamp
                let timestamp = utc_date_to_unix_timestamp(&res.utc, &res.date);
                let mut device_id = String::new();
                let mut trip_id = String::new();
                write!(&mut trip_id, "{mqtt_client_id}").unwrap();
                write!(&mut device_id, "{mqtt_client_id}").unwrap();

                // Create TripData with converted coordinates
                let trip_data = TripData {
                    device_id,
                    trip_id,
                    latitude: ((res.latitude as u64 / 100) as f64)
                        + ((res.latitude % 100.0f64) / 60.0f64),
                    longitude: ((res.longitude as u64 / 100) as f64)
                        + ((res.longitude % 100.0f64) / 60.0f64),
                    timestamp,
                };

                // Send TripData to channel
                if gps_channel.try_send(trip_data.clone()).is_err() {
                    error!("[Quectel] Failed to send TripData to channel");
                } else {
                    info!("[Quectel] GPS data sent to channel: {trip_data:?}");
                }

                Ok(trip_data)
            }
            Err(e) => {
                error!("[Quectel] Failed to retrieve GPS data: {e:?}");
                Err(QuectelError::CommandFailed)
            }
        }
    }

    /// Handles error recovery by resetting the modem.
    ///
    /// Updates the connection state by sending an `LteDisconnected` event if connected,
    /// then initiates a hardware reset after a 5-second delay.
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
///
/// Deletes existing certificates, uploads new CA chain, client certificate, and private key,
/// and configures MQTT over TLS settings.
///
/// # Arguments
///
/// * `client` - The AT command client for sending commands to the modem.
/// * `urc_channel` - Channel for handling unsolicited result codes (URCs).
/// * `ca_chain` - The CA certificate chain as a byte slice.
/// * `certificate` - The client certificate as a byte slice.
/// * `private_key` - The client private key as a byte slice.
///
/// # Returns
///
/// * `true` if the upload and configuration succeed.
/// * `false` if any step fails (e.g., file upload or configuration errors).
pub async fn upload_mqtt_cert_files(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
    urc_channel: &'static UrcChannel<Urc, 128, 3>,
    ca_chain: &[u8],
    certificate: &[u8],
    private_key: &[u8],
) -> bool {
    // Buffer for raw data during file upload
    let mut raw_data = heapless::Vec::<u8, 4096>::new();
    raw_data.clear();
    let mut subscriber = urc_channel.subscribe().unwrap();
    let _ = client.send(&FileList).await.unwrap();
    let now = Instant::now();
    // Check for file list response for up to 10 seconds
    while now.elapsed().as_secs() < 10 {
        Timer::after(Duration::from_secs(1)).await;
        match subscriber.try_next_message_pure() {
            Some(Urc::ListFile(file)) => log::info!("File: {file:?}"),
            Some(e) => error!("Unknown URC {e:?}"),
            None => info!("Waiting for response..."),
        }
    }

    // Remove old certificates
    for name in ["crt.pem", "dvt.crt", "dvt.key"] {
        let _ = client
            .send(&FileDel {
                name: String::from_str(name).unwrap(),
            })
            .await;
        info!("Deleted old {name}");
    }

    // Helper function to upload a single file
    async fn upload_file(
        client: &mut Client<'static, UartTx<'static, Async>, 1024>,
        name: &str,
        content: &[u8],
        raw_data: &mut heapless::Vec<u8, 4096>,
    ) -> bool {
        // Convert file name to heapless string
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
        // Upload data in 1 KiB chunks
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

    // Upload CA certificate
    info!("Uploading CA cert...");
    if !upload_file(client, "crt.pem", ca_chain, &mut raw_data).await {
        return false;
    }

    // Upload client certificate
    info!("Uploading client cert...");
    if !upload_file(client, "dvt.crt", certificate, &mut raw_data).await {
        return false;
    }

    // Upload client private key
    info!("Uploading client key...");
    if !upload_file(client, "dvt.key", private_key, &mut raw_data).await {
        return false;
    }

    // Configure MQTT over TLS
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

    // Configure certificate paths
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

    // Configure SSL settings
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
///
/// Polls the modem for up to 30 seconds to verify network registration status.
/// Logs the status and returns whether registration was successful.
///
/// # Arguments
///
/// * `client` - The AT command client for sending commands to the modem.
///
/// # Returns
///
/// * `true` if the modem is registered (home or roaming).
/// * `false` if registration fails, is denied, or times out.
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
                        esp_println::print!("."); // Indicate ongoing search
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
///
/// Sends an MQTT open command and waits for a response via the URC channel for up to 30 seconds.
///
/// # Arguments
///
/// * `client` - The AT command client for sending commands to the modem.
/// * `urc_channel` - Channel for handling unsolicited result codes (URCs).
///
/// # Returns
///
/// * `Ok(())` if the connection is opened successfully.
/// * `Err(MqttConnectError)` if the command fails, times out, or the modem reports an error.
pub async fn open_mqtt_connection(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
    urc_channel: &'static UrcChannel<Urc, 128, 3>,
) -> Result<(), MqttConnectError> {
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
    info!("fuckkkk");
    loop {
        // Check timeout first
        if start.elapsed() >= TIMEOUT {
            error!("[Quectel] MQTT open timed out");
            return Err(MqttConnectError::Timeout);
        }

        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
        info!("fuckkkk");
        info!("fuckkkk");
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
///
/// Attempts to connect to the MQTT broker with up to 3 retries, using the provided credentials.
/// Waits for a connection acknowledgment for up to 30 seconds.
///
/// # Arguments
///
/// * `client` - The AT command client for sending commands to the modem.
/// * `urc_channel` - Channel for handling unsolicited result codes (URCs).
///
/// # Returns
///
/// * `Ok(())` if the connection is established.
/// * `Err(MqttConnectError)` if the command fails, times out, or the modem reports an error.
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

    info!("fuckkkk");

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

    info!("[Quectel] Waiting for MQTT FUCK response...");

    loop {
        if start.elapsed() > RESPONSE_TIMEOUT {
            error!("[Quectel] MQTT connect timeout");
            return Err(MqttConnectError::Timeout);
        }
        error!("[Quectel] Waiting for MQTT FUCK response...");

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

/// Checks the result of an AT command execution.
///
/// Logs the result and returns whether the command was successful.
///
/// # Arguments
///
/// * `res` - The result of the AT command execution.
///
/// # Returns
///
/// * `true` if the command succeeded.
/// * `false` if the command failed.
pub fn check_result<T>(res: Result<T, atat::Error>) -> bool
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

/// Task for handling incoming data from the modem's UART interface.
///
/// Reads data from the UART receiver and processes it using the ATAT ingress handler.
///
/// # Arguments
///
/// * `ingress` - The ATAT ingress handler for processing modem responses.
/// * `reader` - The UART receiver for reading modem data.
///
/// # Returns
///
/// This function runs indefinitely and does not return.
#[embassy_executor::task]
pub async fn quectel_rx_handler(
    mut ingress: Ingress<'static, DefaultDigester<Urc>, Urc, 1024, 128, 3>,
    mut reader: UartRx<'static, Async>,
) -> ! {
    ingress.read_from(&mut reader).await
}

#[embassy_executor::task]
pub async fn gps_task(
    mut quectel: Quectel,
    mqtt_client_id: &'static str,
    gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
) -> ! {
    quectel.gps_state_machine(mqtt_client_id, gps_channel).await
}
