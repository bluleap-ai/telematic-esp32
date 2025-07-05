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

#[derive(Debug, PartialEq)] // Thêm PartialEq để so sánh trạng thái
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
    GetGPSData,
    ErrorConnection,
}

const REGISTERED_HOME: u8 = 1;
const REGISTERED_ROAMING: u8 = 5;
const UNREGISTERED_SEARCHING: u8 = 2;
const REGISTRATION_DENIED: u8 = 3;
const REGISTRATION_FAILED: u8 = 4;

#[derive(Debug)]
pub enum ModemModel {
    QuectelEG800k,
    QuectelEC25,
    QuectelEC21,
}

#[derive(Debug)]
pub enum ModemError {
    CommandFailed,
    MqttConnectionFailed,
    MqttPublishFailed,
    NetworkRegistrationFailed,
}

#[derive(Debug, PartialEq)]
pub enum MqttConnectError {
    CommandFailed,
    StringConversion,
    Timeout,
    ModemError(u8),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TripData {
    pub device_id: String<36>,
    pub trip_id: String<36>,
    pub latitude: f64,
    pub longitude: f64,
    pub timestamp: u64,
}

pub struct Modem {
    pub client: Client<'static, UartTx<'static, Async>, 1024>,
    pen: Output<'static>,
    dtr: Output<'static>,
    urc_channel: &'static UrcChannel<Urc, 128, 3>,
    is_connected: bool,
    modem_model: ModemModel,
}

impl Modem {
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
            is_connected: false,
            modem_model,
        }
    }

    pub async fn run_state_machine(
        &mut self,
        initial_state: State,
        transitions: &[(
            State,
            Box<dyn Fn(&mut Self) -> Result<(), ModemError> + 'static>,
        )],
    ) -> Result<(), ModemError> {
        let mut state = initial_state;
        let mut retry_count = 0;
        const MAX_RETRIES: usize = 3;

        loop {
            match state {
                _ => {
                    for &(next_state, ref handler) in transitions {
                        match handler(self).await {
                            Ok(()) => {
                                state = next_state;
                                retry_count = 0; // Reset retry count on success
                                break;
                            }
                            Err(e) => {
                                retry_count += 1;
                                if retry_count > MAX_RETRIES {
                                    error!("[modem] Max retries exceeded for state: {:?}", state);
                                    return Err(e);
                                }
                                warn!("[modem] Retry {} for state: {:?}", retry_count, state);
                                Timer::after(Duration::from_secs(1 << retry_count)).await;
                                // Exponential backoff
                            }
                        }
                    }
                    if state == State::ErrorConnection {
                        return Err(ModemError::CommandFailed);
                    }
                }
            }
            Timer::after(Duration::from_secs(1)).await;
        }
    }

    pub async fn modem_initialize(&mut self) -> Result<(), ModemError> {
        info!("[modem] Starting modem initialization");
        let transitions = vec![
            (
                State::DisableEchoMode,
                Box::new(|m: &mut Self| m.reset_hardware())
                    as Box<dyn Fn(&mut Self) -> Result<(), ModemError> + 'static>,
            ),
            (State::GetModelId, Box::new(|m| m.disable_echo_mode())),
            (State::GetSoftwareVersion, Box::new(|m| m.get_model_id())),
            (
                State::ErrorConnection,
                Box::new(|m| m.get_software_version()),
            ),
        ];
        self.run_state_machine(State::ResetHardware, &transitions)
            .await?;
        Ok(())
    }

    pub async fn lte_initialize(
        &mut self,
        _mqtt_client_id: &str,
        _ca_chain: &'static [u8],
        _certificate: &'static [u8],
        _private_key: &'static [u8],
    ) -> Result<(), ModemError> {
        info!("[modem] Starting LTE initialization");
        let transitions = vec![
            (
                State::GetNetworkSignalQuality,
                Box::new(|m| m.get_sim_card_status()),
            ),
            (
                State::GetNetworkInfo,
                Box::new(|m| m.get_network_signal_quality()),
            ),
            (
                State::SetModemFunctionality,
                Box::new(|m| m.get_network_info()),
            ),
            (
                State::UploadMqttCert,
                Box::new(|m| m.set_modem_functionality()),
            ),
            (
                State::CheckNetworkRegistration,
                Box::new(|m| {
                    m.upload_mqtt_cert(
                        include_str!("../../certx/crt.pem").as_bytes(),
                        include_str!("../../certx/dvt.crt").as_bytes(),
                        include_str!("../../certx/dvt.key").as_bytes(),
                    )
                }),
            ),
        ];
        self.run_state_machine(State::GetSimCardStatus, &transitions)
            .await?;
        Ok(())
    }

    pub async fn lte_handle_mqtt(&mut self) -> Result<(), ModemError> {
        info!("[modem] Starting LTE MQTT initialization");
        let transitions = vec![(
            State::MqttConnectBroker,
            Box::new(|m| m.mqtt_open_connection()),
        )];
        self.run_state_machine(State::MqttOpenConnection, &transitions)
            .await?;
        Ok(())
    }

    pub async fn gps_initialize(&mut self) -> Result<(), ModemError> {
        info!("[modem] Starting GPS initialization");
        let transitions = vec![
            (State::EnableAssistGps, Box::new(|m| m.enable_gps())),
            (State::GetGPSData, Box::new(|m| m.enable_assist_gps())),
        ];
        self.run_state_machine(State::EnableGps, &transitions)
            .await?;
        Ok(())
    }

    pub async fn gps_state_machine(
        &mut self,
        mqtt_client_id: &str,
        gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    ) -> ! {
        info!("[modem] Starting GPS state machine");
        loop {
            self.get_gps(mqtt_client_id, gps_channel).await;
            Timer::after(Duration::from_secs(1)).await;
        }
    }

    pub async fn reset_hardware(&mut self) -> Result<(), ModemError> {
        info!("[modem] Reset Hardware");
        self.pen.set_low();
        Timer::after(Duration::from_secs(1)).await;
        self.pen.set_high();
        Timer::after(Duration::from_secs(2)).await;
        Ok(())
    }

    pub async fn disable_echo_mode(&mut self) -> Result<(), ModemError> {
        info!("[modem] Disable Echo Mode");
        if check_result(self.client.send(&DisableEchoMode).await) {
            Ok(())
        } else {
            Err(ModemError::CommandFailed)
        }
    }

    pub async fn get_model_id(&mut self) -> Result<(), ModemError> {
        info!("[modem] Get Model Id");
        if check_result(self.client.send(&GetModelId).await) {
            Ok(())
        } else {
            Err(ModemError::CommandFailed)
        }
    }

    pub async fn get_software_version(&mut self) -> Result<(), ModemError> {
        info!("[modem] Get Software Version");
        if check_result(self.client.send(&GetSoftwareVersion).await) {
            Ok(())
        } else {
            Err(ModemError::CommandFailed)
        }
    }

    pub async fn get_sim_card_status(&mut self) -> Result<(), ModemError> {
        info!("[modem] Get Sim Card Status");
        if check_result(self.client.send(&GetSimCardStatus).await) {
            Ok(())
        } else {
            Err(ModemError::CommandFailed)
        }
    }

    pub async fn get_network_signal_quality(&mut self) -> Result<(), ModemError> {
        info!("[modem] Get Network Signal Quality");
        if check_result(self.client.send(&GetNetworkSignalQuality).await) {
            Ok(())
        } else {
            Err(ModemError::CommandFailed)
        }
    }

    pub async fn get_network_info(&mut self) -> Result<(), ModemError> {
        info!("[modem] Get Network Info");
        if check_result(self.client.send(&GetNetworkInfo).await) {
            Ok(())
        } else {
            Err(ModemError::CommandFailed)
        }
    }

    pub async fn enable_gps(&mut self) -> Result<(), ModemError> {
        info!("[modem] Enable GPS");
        if check_result(self.client.send(&EnableGpsFunc).await) {
            Ok(())
        } else {
            Err(ModemError::CommandFailed)
        }
    }

    pub async fn enable_assist_gps(&mut self) -> Result<(), ModemError> {
        info!("[modem] Enable Assist GPS");
        if check_result(self.client.send(&EnableAssistGpsFunc).await) {
            Ok(())
        } else {
            Err(ModemError::CommandFailed)
        }
    }

    pub async fn set_modem_functionality(&mut self) -> Result<(), ModemError> {
        info!("[modem] Set Modem Functionality");
        if check_result(
            self.client
                .send(&SetUeFunctionality {
                    fun: FunctionalityLevelOfUE::Full,
                })
                .await,
        ) {
            Ok(())
        } else {
            Err(ModemError::CommandFailed)
        }
    }

    pub async fn upload_mqtt_cert(
        &mut self,
        ca_chain: &'static [u8],
        certificate: &'static [u8],
        private_key: &'static [u8],
    ) -> Result<(), ModemError> {
        info!("[modem] Upload Files");
        if self
            .upload_mqtt_cert_files(ca_chain, certificate, private_key)
            .await
        {
            Ok(())
        } else {
            Err(ModemError::CommandFailed)
        }
    }

    async fn upload_mqtt_cert_files(
        &mut self,
        ca_chain: &[u8],
        certificate: &[u8],
        private_key: &[u8],
    ) -> bool {
        let mut raw_data = heapless::Vec::<u8, 4096>::new();
        raw_data.clear();
        let mut subscriber = self.urc_channel.subscribe().unwrap();
        let _ = self.client.send(&FileList).await.unwrap();
        let now = Instant::now();
        while now.elapsed().as_secs() < 10 {
            Timer::after(Duration::from_secs(1)).await;
            match subscriber.try_next_message_pure() {
                Some(Urc::ListFile(file)) => info!("File: {file:?}"),
                Some(e) => error!("Unknown URC {e:?}"),
                None => info!("Waiting for response..."),
            }
        }

        for name in ["crt.pem", "dvt.crt", "dvt.key"] {
            let _ = self
                .client
                .send(&FileDel {
                    name: String::from_str(name).unwrap(),
                })
                .await;
            info!("Deleted old {name}");
        }

        async fn upload_file(
            client: &mut Client<'static, UartTx<'static, Async>, 1024>,
            name: &str,
            content: &[u8],
            raw_data: &mut heapless::Vec<u8, 4096>,
        ) -> bool {
            let name_str = match String::from_str(name) {
                Ok(s) => s,
                Err(_) => {
                    error!("Heapless string overflow for file name: {name}");
                    return false;
                }
            };
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
            for chunk in content.chunks(1024) {
                raw_data.clear();
                if raw_data.extend_from_slice(chunk).is_err() {
                    error!("Raw data buffer overflow");
                    return false;
                }
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

        if !upload_file(&mut self.client, "crt.pem", ca_chain, &mut raw_data).await {
            return false;
        }
        if !upload_file(&mut self.client, "dvt.crt", certificate, &mut raw_data).await {
            return false;
        }
        if !upload_file(&mut self.client, "dvt.key", private_key, &mut raw_data).await {
            return false;
        }

        info!("Configuring MQTT over TLS...");
        let _ = self
            .client
            .send(&MqttConfig {
                name: String::from_str("recv/mode").unwrap(),
                param_1: Some(0),
                param_2: Some(0),
                param_3: Some(1),
            })
            .await;
        let _ = self
            .client
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
            let _ = self
                .client
                .send(&SslConfigCert {
                    name: String::from_str(cfg_name).unwrap(),
                    context_id: 2,
                    cert_path: Some(String::from_str(path).unwrap()),
                })
                .await;
        }

        let _ = self
            .client
            .send(&SslConfigOther {
                name: String::from_str("seclevel").unwrap(),
                context_id: 2,
                level: 2,
            })
            .await;
        let _ = self
            .client
            .send(&SslConfigOther {
                name: String::from_str("sslversion").unwrap(),
                context_id: 2,
                level: 4,
            })
            .await;
        let _ = self.client.send(&SslSetCipherSuite).await;
        let _ = self
            .client
            .send(&SslConfigOther {
                name: String::from_str("ignorelocaltime").unwrap(),
                context_id: 2,
                level: 1,
            })
            .await;
        let _ = self
            .client
            .send(&MqttConfig {
                name: String::from_str("version").unwrap(),
                param_1: Some(0),
                param_2: Some(4),
                param_3: None,
            })
            .await;

        true
    }

    pub async fn check_network_registration(&mut self) -> Result<(), ModemError> {
        info!("[modem] Check Network Registration");
        if self.check_network_registration_internal().await {
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
            Err(ModemError::NetworkRegistrationFailed)
        }
    }

    async fn check_network_registration_internal(&mut self) -> bool {
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
                            return true;
                        }
                        UNREGISTERED_SEARCHING => {
                            Timer::after(Duration::from_secs(1)).await;
                        }
                        REGISTRATION_DENIED => {
                            error!("[modem] Registration denied");
                            return false;
                        }
                        REGISTRATION_FAILED => {
                            error!("[modem] Registration failed");
                            return false;
                        }
                        REGISTERED_ROAMING => {
                            let elapsed = start_time.elapsed().as_secs();
                            info!("[modem] Registered (Roaming) after {elapsed} seconds");
                            return true;
                        }
                        _ => {
                            error!("[modem] Unknown registration status: {}", status.stat);
                            return false;
                        }
                    }
                }
                Err(e) => {
                    error!("[modem] Failed to get EPS network registration status: {e:?}");
                    return false;
                }
            }
        }
        error!("[modem] Network registration timed out");
        false
    }

    pub async fn mqtt_open_connection(&mut self) -> Result<(), ModemError> {
        info!("[modem] Opening MQTT connection");
        self.open_mqtt_connection_internal().await.map_err(|e| {
            error!("[modem] Failed to open MQTT connection: {e:?}");
            ModemError::MqttConnectionFailed
        })?;
        info!("[modem] MQTT connection opened successfully");
        Ok(())
    }

    async fn open_mqtt_connection_internal(&mut self) -> Result<(), MqttConnectError> {
        let server = heapless::String::from_str(MQTT_SERVER_NAME)
            .map_err(|_| MqttConnectError::StringConversion)?;

        self.client
            .send(&MqttOpen {
                link_id: 0,
                server,
                port: MQTT_SERVER_PORT,
            })
            .await
            .map_err(|_| MqttConnectError::CommandFailed)?;

        info!("[modem] MQTT open command sent, waiting for response...");

        let mut subscriber = self
            .urc_channel
            .subscribe()
            .map_err(|_| MqttConnectError::CommandFailed)?;

        let start = Instant::now();
        const TIMEOUT: Duration = Duration::from_secs(30);
        loop {
            if start.elapsed() >= TIMEOUT {
                error!("[modem] MQTT open timed out");
                return Err(MqttConnectError::Timeout);
            }

            Timer::after(Duration::from_secs(1)).await;
            match subscriber.try_next_message_pure() {
                Some(Urc::MqttOpen(response)) => {
                    info!("[modem] Received MQTT open response: {response:?}");
                    return match response.result {
                        0 => Ok(()),
                        code => {
                            error!("[modem] Modem reported error code: {}", code as u8);
                            Err(MqttConnectError::ModemError(code as u8))
                        }
                    };
                }
                Some(other_urc) => {
                    info!("[modem] Received unrelated URC: {other_urc:?}");
                }
                None => {
                    warn!("[modem] No URC received yet...");
                }
            }
        }
    }

    pub async fn mqtt_connect_broker(&mut self) -> Result<(), ModemError> {
        info!("[modem] Connecting to MQTT broker");
        self.connect_mqtt_broker_internal().await.map_err(|e| {
            error!("[modem] MQTT connection failed: {e:?}");
            let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteDisconnected);
            ModemError::MqttConnectionFailed
        })?;
        info!("[modem] MQTT connection established");
        let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteConnected);
        Ok(())
    }

    async fn connect_mqtt_broker_internal(&mut self) -> Result<(), MqttConnectError> {
        const MAX_RETRIES: usize = 3;
        const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
        const CLIENT_ID: &str = "telematics-control-unit";

        let username = heapless::String::<64>::from_str(MQTT_USR_NAME)
            .map_err(|_| MqttConnectError::StringConversion)?;
        let password = heapless::String::<64>::from_str("f57f9bf3-07b3-4ba5-ae1f-bf6f579e346d")
            .map_err(|_| MqttConnectError::StringConversion)?;
        let client_id = heapless::String::<23>::from_str(CLIENT_ID)
            .map_err(|_| MqttConnectError::StringConversion)?;

        for attempt in 1..=MAX_RETRIES {
            info!("[modem] MQTT connect attempt {attempt}/{MAX_RETRIES}");
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
                    error!("[modem] Final connect attempt failed: {e:?}");
                    return Err(MqttConnectError::CommandFailed);
                }
                Err(e) => {
                    warn!("[modem] Connect attempt failed: {e:?} - retrying");
                    Timer::after(Duration::from_secs(1 << (attempt - 1))).await;
                }
            }
        }

        let mut subscriber = self
            .urc_channel
            .subscribe()
            .map_err(|_| MqttConnectError::CommandFailed)?;
        let start = Instant::now();

        info!("[modem] Waiting for MQTT connect response...");
        loop {
            if start.elapsed() > RESPONSE_TIMEOUT {
                error!("[modem] MQTT connect timeout");
                return Err(MqttConnectError::Timeout);
            }

            Timer::after(Duration::from_millis(100)).await;
            match subscriber.try_next_message_pure() {
                Some(Urc::MqttConnect(response)) => {
                    info!("[modem] Received MQTT connect response: {response:?}");
                    return match response.result {
                        0 => Ok(()),
                        code => {
                            error!("[modem] Modem connection error: {code}");
                            Err(MqttConnectError::ModemError(code))
                        }
                    };
                }
                Some(other_urc) => {
                    info!("Ignoring unrelated URC: {other_urc:?}");
                }
                None => {
                    warn!("Waiting for MQTT connect response...");
                }
            }
        }
    }

    pub async fn get_gps(
        &mut self,
        mqtt_client_id: &str,
        gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    ) {
        info!("[modem] Retrieving GPS data");

        match self.client.send(&RetrieveGpsRmc).await {
            Ok(res) => {
                info!("[modem] GPS RMC data received: {res:?}");

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
                    error!("[modem] Failed to send TripData to channel");
                } else {
                    info!("[modem] GPS data sent to channel: {trip_data:?}");
                }
            }
            Err(e) => {
                error!("[modem] Failed to retrieve GPS data: {e:?}");
            }
        }
    }

    pub async fn error_connection(&mut self) -> Result<(), ModemError> {
        if self.is_connected {
            let _ = CONN_EVENT_CHAN.try_send(ConnectionEvent::LteDisconnected);
            self.is_connected = false;
        }
        error!("[modem] System in error state - attempting recovery");
        Timer::after(Duration::from_secs(5)).await;
        self.reset_hardware().await
    }
}

pub fn check_result<T>(res: Result<T, atat::Error>) -> bool
where
    T: Debug,
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

#[embassy_executor::task]
pub async fn modem_rx_handler(
    mut ingress: Ingress<'static, DefaultDigester<Urc>, Urc, 1024, 128, 3>,
    mut reader: UartRx<'static, Async>,
) -> ! {
    ingress.read_from(&mut reader).await
}
