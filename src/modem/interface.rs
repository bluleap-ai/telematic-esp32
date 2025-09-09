use crate::cfg::net_cfg::*;
use crate::net::atcmd::general::*;
use crate::net::atcmd::response::*;
use crate::net::atcmd::Urc;
// use crate::task::netmgr::{ConnectionEvent, CONN_EVENT_CHAN};
use crate::task::can::*;
use crate::util::time::utc_date_to_unix_timestamp;
use atat::{
    asynch::{AtatClient, Client},
    AtatIngress, DefaultDigester, Ingress, UrcChannel,
};
use core::{fmt::Debug, fmt::Write, str::FromStr};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::gpio::Output;
use esp_hal::uart::{UartRx, UartTx};
use esp_hal::Async;
use heapless::String;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
macro_rules! run_cmd {
    ($self:ident, $cmd:expr) => {
        match $self.client.send($cmd).await {
            Ok(response) => {
                info!("[modem] Command successful: {:?}", response);
                Ok(response)
            }
            Err(e) => {
                error!("[modem] AT command failed: {:?}", e);
                Err(e)
            }
        }
    };
}

// Network registration status constants
const REGISTERED_HOME: u8 = 1; // Registered on home network
const UNREGISTERED_SEARCHING: u8 = 2; // Not registered, searching for network
const REGISTRATION_DENIED: u8 = 3; // Registration denied
const REGISTRATION_FAILED: u8 = 4; // Registration failed
const REGISTERED_ROAMING: u8 = 5; // Registered while roaming

/// Represents errors that can occur during the upload process.
/// This enum is used to indicate specific issues that might arise when
/// attempting to upload data. Each variant corresponds to a distinct
/// type of error, and appropriate handling should be implemented for
/// each case.
pub enum UploadError {
    /// Indicates that a `heapless::String` exceeded its maximum capacity.
    ///
    /// This error occurs when attempting to write more data into a
    /// `heapless::String` than its allocated size allows. To handle this
    /// error, ensure that the data being written fits within the string's
    /// capacity or increase the string's size if possible.
    HeaplessStringOverflow,
    /// Represents an error that occurred while sending data via the client.
    ///
    /// This error is returned when the underlying client fails to send
    /// data, possibly due to connectivity issues or internal client errors.
    /// Handling this error might involve retrying the operation or
    /// checking the client's state.
    ClientSendError,
}
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ModemState {
    LteInitializationCompleted,
    GpsInitializationCompleted,
    FullyInitialized,
    FetchingGpsData,
    ServerConnectionEstablished,
    DataPublishing,
    Busy,
    Off,
    ModemInitialized,
    Error(ModemError),
}

#[derive(Debug)]
pub enum ModemModel {
    QuectelEG800k,
    // QuectelEC25,
    // QuectelEC21,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModemError {
    LteInitialization,
    GpsInitialization,
    FetchingGpsData,
    ServerConnection,
    DataPublish,
    Other,
}

#[derive(Debug, PartialEq)]
pub enum MqttConnectError {
    Command,          // AT command error
    StringConversion, // String conversion failed
    Timeout,          // Operation timed out
    ModemError(u8),   // Generic modem error with code
}

/// Trip-related GPS and device data.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TripData {
    pub device_id: String<36>, // Device UUID
    pub trip_id: String<36>,   // Trip UUID
    pub latitude: f64,         // Latitude
    pub longitude: f64,        // Longitude
    pub timestamp: u64,        // Epoch timestamp
}

pub struct Modem {
    ///
    /// # Parameters
    /// - `client`: AT command client for modem communication.
    /// - `pen`: GPIO pin for modem power control.
    /// - `dtr`: GPIO pin for data terminal ready signal.
    /// - `urc_channel`: Channel for handling unsolicited result codes (URCs).
    /// - `modem_model`: The modem model being used.
    ///
    /// # Returns
    /// A configured `Modem` instance.
    pub client: Client<'static, UartTx<'static, Async>, 1024>, // AT command client
    pub pen: Output<'static>, // Modem power enable pin
    #[allow(dead_code)] // Suppress warnings for unused variants
    dtr: Output<'static>, // Data terminal ready pin (optional)
    urc_channel: &'static UrcChannel<Urc, 128, 3>, // Channel for unsolicited result codes
    #[allow(dead_code)] // Suppress warnings for unused variants
    modem_model: ModemModel, // Modem model type
    state: ModemState,        // Current state of the modem
}

impl Modem {
    /// Creates a new `Modem` instance.
    pub fn new(
        client: Client<'static, UartTx<'static, Async>, 1024>,
        pen: Output<'static>,
        dtr: Output<'static>,
        urc_channel: &'static UrcChannel<Urc, 128, 3>,
        modem_model: ModemModel,
    ) -> Self {
        Self {
            client,
            pen,
            dtr,
            urc_channel,
            modem_model,
            state: ModemState::Off,
        }
    }

    /// State transition function.
    async fn transition(&mut self, to: ModemState) -> Result<(), ModemError> {
        info!("[modem] Transitioning from {:?} to {:?}", self.state, to);
        if self.is_valid_transition(to) {
            self.state = to;
            Ok(())
        } else {
            error!(
                "[modem] Invalid state transition: from {:?} to {:?}",
                self.state, to
            );
            self.state = ModemState::Error(ModemError::Other);
            Err(ModemError::Other)
        }
    }

    /// Validates state transitions (OCP: Easy to extend allowed transitions).
    fn is_valid_transition(&self, to: ModemState) -> bool {
        matches!(
            (&self.state, &to),
            (ModemState::Off, ModemState::ModemInitialized)
                | (ModemState::ModemInitialized, ModemState::Busy)
                | (ModemState::Busy, ModemState::LteInitializationCompleted)
                | (ModemState::LteInitializationCompleted, ModemState::Busy)
                | (ModemState::Busy, ModemState::GpsInitializationCompleted)
                | (
                    ModemState::GpsInitializationCompleted,
                    ModemState::FullyInitialized
                )
                | (ModemState::FullyInitialized, ModemState::FetchingGpsData)
                | (
                    ModemState::FetchingGpsData,
                    ModemState::ServerConnectionEstablished
                )
                | (
                    ModemState::ServerConnectionEstablished,
                    ModemState::DataPublishing
                )
                | (ModemState::DataPublishing, ModemState::FetchingGpsData)
                | (ModemState::Off, ModemState::Error(_))
                | (ModemState::ModemInitialized, ModemState::Error(_))
                | (
                    ModemState::ServerConnectionEstablished,
                    ModemState::Error(_)
                )
                | (ModemState::Busy, ModemState::Error(_))
                | (ModemState::FetchingGpsData, ModemState::Error(_))
                | (ModemState::DataPublishing, ModemState::Error(_))
        )
    }

    /// Initializes the modem.
    pub async fn modem_init(&mut self) -> Result<(), ModemError> {
        info!(
            "[modem] Starting Modem initialization {:?}",
            self.modem_model
        );

        self.reset_hardware().await?;
        run_cmd!(self, &DisableEchoMode).map_err(|_| ModemError::LteInitialization)?;
        Timer::after(Duration::from_secs(1)).await;
        run_cmd!(self, &GetModelId).map_err(|_| ModemError::LteInitialization)?;
        Timer::after(Duration::from_secs(1)).await;
        run_cmd!(self, &GetSoftwareVersion).map_err(|_| ModemError::LteInitialization)?;

        info!("[modem] Modem initialized successfully");
        self.transition(ModemState::ModemInitialized).await?;

        Ok(())
    }

    /// Initializes LTE connection.
    pub async fn lte_init(
        &mut self,
        _mqtt_client_id: &str,
        _ca_chain: &'static [u8],
        _certificate: &'static [u8],
        _private_key: &'static [u8],
    ) -> Result<(), ModemError> {
        info!("[modem] Starting LTE initialization");

        if self.state != ModemState::ModemInitialized {
            return Err(ModemError::Other);
        }
        self.transition(ModemState::Busy).await?;

        Timer::after(Duration::from_secs(1)).await;
        run_cmd!(self, &GetSimCardStatus).map_err(|_| ModemError::LteInitialization)?;
        Timer::after(Duration::from_secs(1)).await;
        run_cmd!(self, &GetNetworkSignalQuality).map_err(|_| ModemError::LteInitialization)?;
        Timer::after(Duration::from_secs(1)).await;
        run_cmd!(self, &GetNetworkInfo).map_err(|_| ModemError::LteInitialization)?;
        Timer::after(Duration::from_secs(1)).await;
        run_cmd!(
            self,
            &SetUeFunctionality {
                fun: FunctionalityLevelOfUE::Full
            }
        )
        .map_err(|_| ModemError::LteInitialization)?;
        Timer::after(Duration::from_secs(1)).await;
        self.upload_mqtt_cert(_ca_chain, _certificate, _private_key)
            .await
            .map_err(|_e| ModemError::LteInitialization)?;

        info!("[modem] LTE initialized successfully");
        self.transition(ModemState::LteInitializationCompleted)
            .await?;

        Ok(())
    }

    /// Initializes GPS functionality.
    pub async fn gps_init(&mut self) -> Result<(), ModemError> {
        info!("[modem] Starting GPS initialization");
        if self.state != ModemState::LteInitializationCompleted {
            return Err(ModemError::Other);
        }
        self.transition(ModemState::Busy).await?;

        run_cmd!(self, &EnableGpsFunc).map_err(|_| ModemError::GpsInitialization)?;
        Timer::after(Duration::from_secs(1)).await;
        run_cmd!(self, &EnableAssistGpsFunc).map_err(|_| ModemError::GpsInitialization)?;

        info!("[modem] GPS initialized successfully");
        self.transition(ModemState::GpsInitializationCompleted)
            .await?;
        self.transition(ModemState::FullyInitialized).await?;

        Ok(())
    }

    /// Initializes MQTT over LTE.
    pub async fn init_mqtt_over_lte(&mut self) -> Result<(), ModemError> {
        info!("[modem] Starting LTE MQTT initialization");

        if self.state != ModemState::FetchingGpsData {
            return Err(ModemError::Other);
        }

        self.check_network_registration().await?;
        self.mqtt_open_connection()
            .await
            .map_err(|_e| ModemError::Other)?;

        self.mqtt_connect_broker()
            .await
            .map_err(|_e| ModemError::Other)?;

        self.transition(ModemState::ServerConnectionEstablished)
            .await?;
        info!("[modem] MQTT over LTE initialized successfully");
        Ok(())
    }

    /// Sends GPS data to a channel.
    pub async fn send_gps_to_channel(
        &self,
        trip_data: TripData,
        gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    ) -> Result<(), ModemError> {
        if gps_channel.try_send(trip_data.clone()).is_err() {
            error!("[modem] Failed to send TripData to channel");
            Err(ModemError::Other)
        } else {
            info!("[modem] GPS data sent to channel: {trip_data:?}");
            Ok(())
        }
    }

    /// Publishes CAN data to the MQTT server.
    pub async fn push_can_data_to_server(
        &mut self,
        can_channel: &'static TwaiOutbox,
        mqtt_client_id: &str,
    ) -> Result<(), ModemError> {
        if self.state != ModemState::ServerConnectionEstablished {
            return Err(ModemError::Other);
        }
        self.transition(ModemState::DataPublishing).await?;

        if let Ok(frame) = can_channel.try_receive() {
            info!("[LTE] CAN data received from channel: {frame:?}");

            let mut can_topic: heapless::String<128> = heapless::String::new();
            let mut can_payload: heapless::String<1024> = heapless::String::new();
            let mut buf: [u8; 1024] = [0u8; 1024];

            let can_data = CanFrame {
                id: frame.id,
                len: frame.len,
                data: frame.data,
            };

            if core::fmt::write(
                &mut can_topic,
                format_args!("channels/{mqtt_client_id}/messages/client/can"),
            )
            .is_err()
            {
                error!("[LTE] Failed to format CAN topic");
                return Err(ModemError::DataPublish);
            }

            if let Ok(len) = serde_json_core::to_slice(&can_data, &mut buf) {
                let json = core::str::from_utf8(&buf[..len])
                    .unwrap_or_default()
                    .replace('\"', "'");

                if write!(&mut can_payload, "{json}").is_err() {
                    error!("[LTE] Failed to copy JSON to payload string buffer");
                    return Err(ModemError::DataPublish);
                }

                info!("[LTE] MQTT payload (CAN): {can_payload}");

                // Publish
                run_cmd!(
                    self,
                    &MqttPublishExtended {
                        tcp_connect_id: 0,
                        msg_id: 0,
                        qos: 0,
                        retain: 0,
                        topic: can_topic.clone(),
                        payload: can_payload.clone(),
                    }
                )
                .map_err(|_| ModemError::DataPublish)?;

                info!("[LTE] CAN data published successfully");
                return Ok(());
            } else {
                error!("[LTE] Failed to serialize CAN data");
                return Err(ModemError::DataPublish);
            }
        }
        Err(ModemError::DataPublish)
    }

    /// Publishes GPS data to the MQTT server.
    pub async fn push_gps_data_to_server(
        &mut self,
        gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
        mqtt_client_id: &str,
    ) -> Result<(), ModemError> {
        if self.state != ModemState::ServerConnectionEstablished {
            return Err(ModemError::Other);
        }
        self.transition(ModemState::DataPublishing).await?;

        if let Ok(trip_data) = gps_channel.try_receive() {
            info!("[LTE] GPS data received from channel: {trip_data:?}");
            let mut trip_payload: heapless::String<1024> = heapless::String::new();
            let mut buf: [u8; 1024] = [0u8; 1024];
            let mut trip_topic: heapless::String<128> = heapless::String::new();

            if core::fmt::write(
                &mut trip_topic,
                format_args!("channels/{mqtt_client_id}/messages/client/trip"),
            )
            .is_err()
            {
                error!("[LTE] Failed to format trip topic");
            }

            if let Ok(len) = serde_json_core::to_slice(&trip_data, &mut buf) {
                let json = core::str::from_utf8(&buf[..len])
                    .unwrap_or_default()
                    .replace('\"', "'");

                if write!(&mut trip_payload, "{json}").is_err() {
                    error!("[LTE] Failed to copy JSON to payload string buffer");
                    return Err(ModemError::DataPublish);
                }

                info!("[LTE] MQTT payload (GPS/trip): {trip_payload}");
                run_cmd!(
                    self,
                    &MqttPublishExtended {
                        tcp_connect_id: 0,
                        msg_id: 0,
                        qos: 0,
                        retain: 0,
                        topic: trip_topic.clone(),
                        payload: trip_payload.clone(),
                    }
                )
                .map_err(|_| ModemError::DataPublish)?;
                info!("[LTE] Trip data published successfully");
                // retry_count = 0; // Reset retry count on success
                return Ok(());
            }
        }
        Err(ModemError::DataPublish)
    }

    pub async fn get_gps(&mut self, mqtt_client_id: &str) -> Result<TripData, ModemError> {
        info!("[modem] Starting GPS data retrieval");
        let trip_result = self.client.send(&RetrieveGpsRmc).await;
        match trip_result {
            Ok(res) => {
                info!("[modem] GPS RMC data received: {res:?}");

                if self.state == ModemState::FullyInitialized
                    || self.state == ModemState::DataPublishing
                {
                    self.transition(ModemState::FetchingGpsData).await?;
                } else {
                    return Err(ModemError::Other);
                }
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

                Ok(trip_data)
            }
            Err(e) => {
                error!("[modem] Failed to retrieve GPS data: {e:?}");
                Err(ModemError::FetchingGpsData)
            }
        }
    }

    /// Returns the current state of the modem.
    #[allow(dead_code)]
    pub async fn get_state(&self) -> ModemState {
        self.state
    }

    /// Resets the modem hardware.
    async fn reset_hardware(&mut self) -> Result<(), ModemError> {
        info!("[modem] Reset Hardware");
        self.pen.set_low();
        Timer::after(Duration::from_secs(1)).await;
        self.pen.set_high();
        Timer::after(Duration::from_secs(5)).await;
        Ok(())
    }

    /// Uploads MQTT certificates to the modem.
    pub async fn upload_mqtt_cert(
        &mut self,
        ca_chain: &'static [u8],
        certificate: &'static [u8],
        private_key: &'static [u8],
    ) -> Result<(), UploadError> {
        let mut raw_data = heapless::Vec::<u8, 4096>::new();
        raw_data.clear();
        let mut subscriber = self
            .urc_channel
            .subscribe()
            .map_err(|_| UploadError::ClientSendError)?;
        self.client
            .send(&FileList)
            .await
            .map_err(|_| UploadError::ClientSendError)?;
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
            let name_str = match heapless::String::from_str(name) {
                Ok(s) => s,
                Err(_) => {
                    error!("[modem] Failed to create string for file name: {name}");
                    return Err(UploadError::HeaplessStringOverflow);
                }
            };
            if let Err(e) = self.client.send(&FileDel { name: name_str }).await {
                warn!("[modem] Failed to delete old file {name}: {e:?}");
                // Continue anyway - file might not exist
            } else {
                info!("Deleted old {name}");
            }
        }

        // Upload helper
        async fn upload_file(
            client: &mut Client<'static, UartTx<'static, Async>, 1024>,
            name: &str,
            content: &[u8],
            raw_data: &mut heapless::Vec<u8, 4096>,
        ) -> Result<(), UploadError> {
            //Sending file upload command to notify the modem about the file to be uploaded
            let name_str = heapless::String::from_str(name)
                .map_err(|_| UploadError::HeaplessStringOverflow)?;
            //Notify the modem about the file to be uploaded
            client
                .send(&FileUpl {
                    name: name_str,
                    size: content.len() as u32,
                })
                .await
                .map_err(|_| UploadError::ClientSendError)?;

            //Uploading data payload in 1 Kib of chunks
            for chunk in content.chunks(1024) {
                raw_data.clear();
                raw_data
                    .extend_from_slice(chunk)
                    .map_err(|_| UploadError::HeaplessStringOverflow)?;

                client
                    .send(&SendRawData {
                        raw_data: raw_data.clone(),
                        len: chunk.len(),
                    })
                    .await
                    .map_err(|_| UploadError::ClientSendError)?;
            }

            embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
            Ok(())
        }

        // Upload certs
        info!("Uploading CA cert...");
        upload_file(&mut self.client, "crt.pem", ca_chain, &mut raw_data).await?;

        info!("Uploading client cert...");
        upload_file(&mut self.client, "dvt.crt", certificate, &mut raw_data).await?;

        info!("Uploading client key...");
        upload_file(&mut self.client, "dvt.key", private_key, &mut raw_data).await?;

        // Configure MQTTS
        info!("Configuring MQTT over TLS...");
        let recv_mode_name = match heapless::String::from_str("recv/mode") {
            Ok(s) => s,
            Err(_) => {
                error!("[LTE] Failed to create string for recv/mode config");
                return Err(UploadError::HeaplessStringOverflow);
            }
        };
        if let Err(e) = self
            .client
            .send(&MqttConfig {
                name: recv_mode_name,
                param_1: Some(0),
                param_2: Some(0),
                param_3: Some(1),
            })
            .await
        {
            error!("[LTE] Failed to configure MQTT recv/mode: {e:?}");
            return Err(UploadError::ClientSendError);
        }

        let ssl_name = match heapless::String::from_str("SSL") {
            Ok(s) => s,
            Err(_) => {
                error!("[LTE] Failed to create string for SSL config");
                return Err(UploadError::HeaplessStringOverflow);
            }
        };
        if let Err(e) = self
            .client
            .send(&MqttConfig {
                name: ssl_name,
                param_1: Some(0),
                param_2: Some(1),
                param_3: Some(2),
            })
            .await
        {
            error!("[LTE] Failed to configure MQTT SSL: {e:?}");
            return Err(UploadError::ClientSendError);
        }

        for (cfg_name, path) in [
            ("cacert", "UFS:ca.crt"),
            ("clientcert", "UFS:dvt.crt"),
            ("clientkey", "UFS:dvt.key"),
        ] {
            let config_name = match heapless::String::from_str(cfg_name) {
                Ok(s) => s,
                Err(_) => {
                    error!("[LTE] Failed to create string for config name: {cfg_name}");
                    return Err(UploadError::HeaplessStringOverflow);
                }
            };

            let cert_path = match heapless::String::from_str(path) {
                Ok(s) => s,
                Err(_) => {
                    error!("[LTE] Failed to create string for cert path: {path}");
                    return Err(UploadError::HeaplessStringOverflow);
                }
            };

            if let Err(e) = self
                .client
                .send(&SslConfigCert {
                    name: config_name,
                    context_id: 2,
                    cert_path: Some(cert_path),
                })
                .await
            {
                error!("[LTE] Failed to configure SSL cert {cfg_name}: {e:?}");
                return Err(UploadError::ClientSendError);
            }
        }

        let name_seclevel = match heapless::String::from_str("seclevel") {
            Ok(s) => s,
            Err(_) => {
                error!("[modem] Failed to create string for seclevel config");
                return Err(UploadError::HeaplessStringOverflow);
            }
        };

        if let Err(e) = self
            .client
            .send(&SslConfigOther {
                name: name_seclevel,
                context_id: 2,
                level: 2,
            })
            .await
        {
            error!("[modem] Failed to configure SSL security level: {e:?}");
            return Err(UploadError::ClientSendError);
        }

        let sslversion_name = match heapless::String::from_str("sslversion") {
            Ok(s) => s,
            Err(_) => {
                error!("[modem] Failed to create string for sslversion config");
                return Err(UploadError::HeaplessStringOverflow);
            }
        };
        if let Err(e) = self
            .client
            .send(&SslConfigOther {
                name: sslversion_name,
                context_id: 2,
                level: 4,
            })
            .await
        {
            error!("[modem] Failed to configure SSL version: {e:?}");
            return Err(UploadError::ClientSendError);
        }

        if let Err(e) = self.client.send(&SslSetCipherSuite).await {
            error!("[modem] Failed to set SSL cipher suite: {e:?}");
            return Err(UploadError::ClientSendError);
        }

        let ignorelocaltime_name = match heapless::String::from_str("ignorelocaltime") {
            Ok(s) => s,
            Err(_) => {
                error!("[modem] Failed to create string for ignorelocaltime config");
                return Err(UploadError::HeaplessStringOverflow);
            }
        };
        if let Err(e) = self
            .client
            .send(&SslConfigOther {
                name: ignorelocaltime_name,
                context_id: 2,
                level: 1,
            })
            .await
        {
            error!("[modem] Failed to configure SSL ignore local time: {e:?}");
            return Err(UploadError::ClientSendError);
        }

        let version_name = match heapless::String::from_str("version") {
            Ok(s) => s,
            Err(_) => {
                error!("[modem] Failed to create string for version config");
                return Err(UploadError::HeaplessStringOverflow);
            }
        };
        if let Err(e) = self
            .client
            .send(&MqttConfig {
                name: version_name,
                param_1: Some(0),
                param_2: Some(4),
                param_3: None,
            })
            .await
        {
            error!("[modem] Failed to configure MQTT version: {e:?}");
            return Err(UploadError::ClientSendError);
        }

        Ok(())
    }

    /// Checks network registration status.
    async fn check_network_registration(&mut self) -> Result<(), ModemError> {
        info!("[modem] Check Network Registration");
        let timeout = Duration::from_secs(30);
        let start_time = Instant::now();

        while start_time.elapsed() < timeout {
            match self.client.send(&GetEPSNetworkRegistrationStatus {}).await {
                Ok(status) => {
                    info!("[modem] EPS network registration status: {status:?}");
                    match status.stat {
                        REGISTERED_HOME => {
                            let elapsed = start_time.elapsed().as_secs();
                            info!("[modem] Registered (Home) after {elapsed} seconds");
                            return Ok(());
                        }
                        UNREGISTERED_SEARCHING => {
                            Timer::after(Duration::from_secs(1)).await;
                        }
                        REGISTRATION_DENIED => {
                            error!("[modem] Registration denied");
                            return Err(ModemError::ServerConnection);
                        }
                        REGISTRATION_FAILED => {
                            error!("[modem] Registration failed");
                            return Err(ModemError::ServerConnection);
                        }
                        REGISTERED_ROAMING => {
                            let elapsed = start_time.elapsed().as_secs();
                            info!("[modem] Registered (Roaming) after {elapsed} seconds");
                            return Ok(());
                        }
                        _ => {
                            error!("[modem] Unknown registration status: {}", status.stat);
                            return Err(ModemError::ServerConnection);
                        }
                    }
                }
                Err(e) => {
                    error!("[modem] Failed to get EPS network registration status: {e:?}");
                    return Err(ModemError::ServerConnection);
                }
            }
        }
        error!("[modem] Network registration timed out");
        Err(ModemError::ServerConnection)
    }

    /// Opens an MQTT connection.
    pub async fn mqtt_open_connection(&mut self) -> Result<(), MqttConnectError> {
        // Create server string safely
        let server = heapless::String::from_str(MQTT_SERVER_NAME)
            .map_err(|_| MqttConnectError::StringConversion)?; // Optionally log the error here for more info

        // Send MQTT open command
        self.client
            .send(&MqttOpen {
                link_id: 0,
                server,
                port: MQTT_SERVER_PORT,
            })
            .await
            .map_err(|_| MqttConnectError::Command)?; // Optionally log the error here for more info

        info!("[Quectel] MQTT open command sent, waiting for response...");

        let mut subscriber = self
            .urc_channel
            .subscribe()
            .map_err(|_| MqttConnectError::Command)?; // Optionally log the error here for more info

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

    /// Connects to the MQTT broker.
    pub async fn mqtt_connect_broker(&mut self) -> Result<(), MqttConnectError> {
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

            match self
                .client
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
                    return Err(MqttConnectError::Command);
                }
                Err(e) => {
                    warn!("[Quectel] Connect attempt failed: {e:?} - retrying");
                    embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
                }
            }
        }

        // Wait for connection acknowledgement
        let mut subscriber = self
            .urc_channel
            .subscribe()
            .map_err(|_| MqttConnectError::Command)?;
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
                    warn!("Ignoring unrelated URC: {other_urc:?}");
                }
                None => {
                    warn!("Waiting for MQTT connect response...");
                }
            }
        }
    }
}

#[embassy_executor::task]
pub async fn modem_rx_handle(
    mut ingress: Ingress<'static, DefaultDigester<Urc>, Urc, 1024, 128, 3>,
    mut reader: UartRx<'static, Async>,
) -> ! {
    ingress.read_from(&mut reader).await
}
