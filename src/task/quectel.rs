use atat::{
    asynch::{Client, Ingress},
    AtDigester, DefaultDigester, Error as AtatError, ResponseSlot, StaticCell, UrcChannel,
};
use core::fmt::Write;
use core::marker::PhantomData;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use esp_hal::{
    gpio::{GpioPin, Input, InputPin, Level, Output, OutputPin},
    peripherals::{GPIO, UART0},
    uart::{Async, Config, Uart, UartRx, UartTx},
};
use heapless::String;
use log::{error, info, warn};

// Type alias for the Quectel configuration, generic over pin types
pub type QuectelDefault<TXP, RXP, PENP, DPR> =
    Quectel<UartTx<'static, Async>, TXP, PENP, DPR, 1024, 128, 3>;

// Placeholder types
pub enum Urc {
    // Define URC variants as per your AT commands
}

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
    GetGPSData,
    SetModemFunctionality,
    UploadMqttCert,
    CheckNetworkRegistration,
    MqttOpenConnection,
    MqttConnectBroker,
    MqttPublishData,
    ErrorConnection,
}

pub struct TripData {
    pub device_id: String<32>,
    pub trip_id: String<32>,
    pub latitude: f64,
    pub longitude: f64,
    pub timestamp: u64,
}

pub enum ConnectionEvent {
    LteRegistered,
    LteUnregistered,
    LteConnected,
    LteDisconnected,
}

pub enum FunctionalityLevelOfUE {
    Full,
}

pub struct SetUeFunctionality {
    pub fun: FunctionalityLevelOfUE,
}

pub struct DisableEchoMode;
pub struct GetModelId;
pub struct GetSoftwareVersion;
pub struct GetSimCardStatus;
pub struct GetNetworkSignalQuality;
pub struct GetNetworkInfo;
pub struct EnableGpsFunc;
pub struct EnableAssistGpsFunc;
pub struct RetrieveGpsRmc {
    pub utc: String<32>,
    pub date: String<32>,
    pub latitude: f64,
    pub longitude: f64,
}

pub struct TwaiOutbox;

// Placeholder functions
async fn reset_modem<P: OutputPin>(pen: &mut Output<P>) {
    pen.set_low(); // Power down the modem
    embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
    pen.set_high(); // Power up the modem
    embassy_time::Timer::after(embassy_time::Duration::from_secs(5)).await;
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

pub async fn upload_mqtt_cert(&mut self) -> State {
    info!("[Quectel] Upload MQTT certs to Quectel");

    // File names for certificates (not hardcoded, but using fixed names for UFS)
    const CA_NAME: &str = "crt.pem";
    const CERT_NAME: &str = "dvt.crt";
    const KEY_NAME: &str = "dvt.key";
    const CHUNK_SIZE: usize = 1024;

    let mut raw_data = Vec::<u8, 4096>::new();
    raw_data.clear();
    let mut subscriber = self.urc_channel.subscribe().unwrap();

    // List files and log them
    let _ = self.client.send(&FileList).await;
    let now = embassy_time::Instant::now();
    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
        match subscriber.try_next_message_pure() {
            Some(Urc::ListFile(file)) => {
                info!("[Quectel] File: {file:?}");
            }
            Some(e) => {
                error!("[Quectel] Unknown URC {e:?}");
            }
            None => {
                info!("[Quectel] Waiting for response...");
            }
        }
        if now.elapsed().as_secs() > 10 {
            break;
        }
    }

    // Delete existing certificate files
    info!("[Quectel] Remove CA_CRT path");
    let _ = self
        .client
        .send(&FileDel {
            name: String::from_str(CA_NAME).unwrap(),
        })
        .await;

    info!("[Quectel] Remove CLIENT_CRT path");
    let _ = self
        .client
        .send(&FileDel {
            name: String::from_str(CERT_NAME).unwrap(),
        })
        .await;

    info!("[Quectel] Remove CLIENT_KEY path");
    let _ = self
        .client
        .send(&FileDel {
            name: String::from_str(KEY_NAME).unwrap(),
        })
        .await;

    // Upload CA certificate
    info!("[Quectel] Upload CA certificate");
    let ca_size = self.ca_chain.len();
    let _ = self
        .client
        .send(&FileUpl {
            name: String::from_str(CA_NAME).unwrap(),
            size: ca_size,
        })
        .await;

    for chunk in self.ca_chain.chunks(CHUNK_SIZE) {
        raw_data.clear();
        let _ = raw_data.extend_from_slice(chunk);
        let _ = self
            .client
            .send(&SendRawData {
                raw_data: raw_data.clone(),
                len: chunk.len(),
            })
            .await;
    }
    embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;

    // Upload client certificate
    info!("[Quectel] Upload client certificate");
    let cert_size = self.certificate.len();
    let _ = self
        .client
        .send(&FileUpl {
            name: String::from_str(CERT_NAME).unwrap(),
            size: cert_size,
        })
        .await;

    for chunk in self.certificate.chunks(CHUNK_SIZE) {
        raw_data.clear();
        let _ = raw_data.extend_from_slice(chunk);
        let _ = self
            .client
            .send(&SendRawData {
                raw_data: raw_data.clone(),
                len: chunk.len(),
            })
            .await;
    }
    embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;

    // Upload client private key
    info!("[Quectel] Upload client private key");
    let key_size = self.private_key.len();
    let _ = self
        .client
        .send(&FileUpl {
            name: String::from_str(KEY_NAME).unwrap(),
            size: key_size,
        })
        .await;

    for chunk in self.private_key.chunks(CHUNK_SIZE) {
        raw_data.clear();
        let _ = raw_data.extend_from_slice(chunk);
        let _ = self
            .client
            .send(&SendRawData {
                raw_data: raw_data.clone(),
                len: chunk.len(),
            })
            .await;
    }
    embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;

    // Set MQTT and SSL configurations
    info!("[Quectel] Set MQTTS configuration");
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
    let _ = self
        .client
        .send(&SslConfigCert {
            name: String::from_str("cacert").unwrap(),
            context_id: 2,
            cert_path: Some(String::from_str("UFS:crt.pem").unwrap()),
        })
        .await;
    let _ = self
        .client
        .send(&SslConfigCert {
            name: String::from_str("clientcert").unwrap(),
            context_id: 2,
            cert_path: Some(String::from_str("UFS:dvt.crt").unwrap()),
        })
        .await;
    let _ = self
        .client
        .send(&SslConfigCert {
            name: String::from_str("clientkey").unwrap(),
            context_id: 2,
            cert_path: Some(String::from_str("UFS:dvt.key").unwrap()),
        })
        .await;
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

    State::CheckNetworkRegistration
}

async fn check_network_registration(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
) -> bool {
    true
}

async fn open_mqtt_connection(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
    urc_channel: &UrcChannel<Urc, 128, 3>,
) -> Result<(), AtatError> {
    Ok(())
}

async fn connect_mqtt_broker(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
    urc_channel: &UrcChannel<Urc, 128, 3>,
) -> Result<(), AtatError> {
    Ok(())
}

async fn handle_publish_mqtt_data(
    client: &mut Client<'static, UartTx<'static, Async>, 1024>,
    mqtt_client_id: &str,
    gps_channel: &Channel<NoopRawMutex, TripData, 8>,
    can_channel: &TwaiOutbox,
) -> bool {
    true
}

fn utc_date_to_unix_timestamp(utc: &str, date: &str) -> u64 {
    0
}

pub struct Quectel<
    UART,
    TXP,
    PENP,
    DPR,
    const RES_SIZE: usize,
    const URC_SIZE: usize,
    const URC_COUNT: usize,
> {
    client: Client<UART, RES_SIZE>,
    pen_pin: Output<PENP>,
    dtr_pin: Output<DPR>,
    ca_chain: &'static [u8],
    certificate: &'static [u8],
    private_key: &'static [u8],
    mqtt_client_id: &'static str,
    urc_channel: &'static UrcChannel<Urc, URC_SIZE, URC_COUNT>,
    gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    can_channel: &'static TwaiOutbox,
    conn_event_chan: &'static Channel<NoopRawMutex, ConnectionEvent, 8>,
    active_connection_chan: &'static Channel<NoopRawMutex, (), 1>,
    is_connected: bool,
    _phantom: PhantomData<(UART, TXP, PENP, DPR)>,
}

impl<
        UART,
        TXP,
        RXP,
        PENP,
        DPR,
        const RES_SIZE: usize,
        const URC_SIZE: usize,
        const URC_COUNT: usize,
    > Quectel<UART, TXP, PENP, DPR, RES_SIZE, URC_SIZE, URC_COUNT>
where
    UART: embedded_hal_async::serial::Write<u8>,
    TXP: OutputPin,
    RXP: InputPin,
    PENP: OutputPin,
    DPR: OutputPin,
{
    pub fn new(
        uart_tx: UART,
        pen_pin: Output<PENP>,
        dtr_pin: Output<DPR>,
        res_slot: &'static ResponseSlot<RES_SIZE>,
        buf: &'static mut [u8; RES_SIZE],
        ca_chain: &'static [u8],
        certificate: &'static [u8],
        private_key: &'static [u8],
        mqtt_client_id: &'static str,
        urc_channel: &'static UrcChannel<Urc, URC_SIZE, URC_COUNT>,
        gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
        can_channel: &'static TwaiOutbox,
        conn_event_chan: &'static Channel<NoopRawMutex, ConnectionEvent, 8>,
        active_connection_chan: &'static Channel<NoopRawMutex, (), 1>,
    ) -> Self {
        let config = atat::Config::default();
        let client = Client::new(uart_tx, res_slot, buf, config);
        Quectel {
            client,
            pen_pin,
            dtr_pin,
            ca_chain,
            certificate,
            private_key,
            mqtt_client_id,
            urc_channel,
            gps_channel,
            can_channel,
            conn_event_chan,
            active_connection_chan,
            is_connected: false,
            _phantom: PhantomData,
        }
    }

    pub fn init(
        peripherals: &mut GPIO,
        uart: UART0,
        tx_pin: impl Into<GpioPin<Output, { esp_hal::gpio::Pull::None }>>,
        rx_pin: impl Into<GpioPin<Input, { esp_hal::gpio::Pull::None }>>,
        pen_pin: impl Into<GpioPin<Output, { esp_hal::gpio::Pull::None }>>,
        dtr_pin: impl Into<GpioPin<Output, { esp_hal::gpio::Pull::None }>>,
        ca_chain: &'static [u8],
        certificate: &'static [u8],
        private_key: &'static [u8],
        mqtt_client_id: &'static str,
        urc_channel: &'static UrcChannel<Urc, 128, 3>,
        gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
        can_channel: &'static TwaiOutbox,
        conn_event_chan: &'static Channel<NoopRawMutex, ConnectionEvent, 8>,
        active_connection_chan: &'static Channel<NoopRawMutex, (), 1>,
    ) -> (
        QuectelDefault<TXP, RXP, PENP, DPR>,
        Ingress<'static, DefaultDigester<Urc>, Urc, 1024, 128, 3>,
        UartRx<'static, Async>,
    ) {
        let uart_tx_pin = tx_pin.into();
        let uart_rx_pin = rx_pin.into();
        let quectel_pen_pin = Output::new(pen_pin.into(), Level::High);
        let quectel_dtr_pin = Output::new(dtr_pin.into(), Level::High);

        let config = Config::default().with_rx_fifo_full_threshold(64);
        let uart0 = Uart::new(uart, config)
            .unwrap()
            .with_rx(uart_rx_pin)
            .with_tx(uart_tx_pin)
            .into_async();
        let (uart_rx, uart_tx) = uart0.split();

        static RES_SLOT: ResponseSlot<1024> = ResponseSlot::new();
        static INGRESS_BUF: StaticCell<[u8; 1024]> = StaticCell::new();
        let ingress = Ingress::new(
            AtDigester::<Urc>::default(),
            INGRESS_BUF.init([0; 1024]),
            &RES_SLOT,
            urc_channel,
        );

        static BUF: StaticCell<[u8; 1024]> = StaticCell::new();
        let client = Client::new(
            uart_tx,
            &RES_SLOT,
            BUF.init([0; 1024]),
            atat::Config::default(),
        );

        (
            Quectel {
                client,
                pen_pin: quectel_pen_pin,
                dtr_pin: quectel_dtr_pin,
                ca_chain,
                certificate,
                private_key,
                mqtt_client_id,
                urc_channel,
                gps_channel,
                can_channel,
                conn_event_chan,
                active_connection_chan,
                is_connected: false,
                _phantom: PhantomData,
            },
            ingress,
            uart_rx,
        )
    }

    pub fn client(&mut self) -> &mut Client<UART, RES_SIZE> {
        &mut self.client
    }

    pub fn set_pen_pin(&mut self, level: Level) {
        self.pen_pin.set_level(level);
    }

    pub fn set_dtr_pin(&mut self, level: Level) {
        self.dtr_pin.set_level(level);
    }

    pub async fn get_gps(&mut self) -> Result<TripData, AtatError> {
        info!("[Quectel] Retrieving GPS Data");
        let trip_result = self.client.send(&RetrieveGpsRmc).await;

        match trip_result {
            Ok(res) => {
                info!("[LTE] GPS RMC data received: {res:?}");
                let timestamp = utc_date_to_unix_timestamp(&res.utc, &res.date);
                let mut device_id = String::new();
                let mut trip_id = String::new();
                write!(&mut trip_id, "{}", self.mqtt_client_id).unwrap();
                write!(&mut device_id, "{}", self.mqtt_client_id).unwrap();

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
                warn!("[Quectel] Failed to retrieve GPS data: {e:?}");
                Err(e)
            }
        }
    }

    pub async fn get_lte_registration(&mut self) -> bool {
        info!("[Quectel] Checking LTE Registration");
        check_network_registration(&mut self.client).await
    }

    pub async fn reset_hardware(&mut self) -> State {
        info!("[Quectel] Reset Hardware");
        reset_modem(&mut self.pen_pin).await;
        State::DisableEchoMode
    }

    pub async fn disable_echo_mode(&mut self) -> State {
        info!("[Quectel] Disable Echo Mode");
        if check_result(self.client.send(&DisableEchoMode).await) {
            State::GetModelId
        } else {
            State::ErrorConnection
        }
    }

    pub async fn get_model_id(&mut self) -> State {
        info!("[Quectel] Get Model Id");
        if check_result(self.client.send(&GetModelId).await) {
            State::GetSoftwareVersion
        } else {
            State::ErrorConnection
        }
    }

    pub async fn get_software_version(&mut self) -> State {
        info!("[Quectel] Get Software Version");
        if check_result(self.client.send(&GetSoftwareVersion).await) {
            State::GetSimCardStatus
        } else {
            State::ErrorConnection
        }
    }

    pub async fn get_sim_card_status(&mut self) -> State {
        info!("[Quectel] Get Sim Card Status");
        if check_result(self.client.send(&GetSimCardStatus).await) {
            State::GetNetworkSignalQuality
        } else {
            State::ErrorConnection
        }
    }

    pub async fn get_network_signal_quality(&mut self) -> State {
        info!("[Quectel] Get Network Signal Quality");
        if check_result(self.client.send(&GetNetworkSignalQuality).await) {
            State::GetNetworkInfo
        } else {
            State::ErrorConnection
        }
    }

    pub async fn get_network_info(&mut self) -> State {
        info!("[Quectel] Get Network Info");
        if check_result(self.client.send(&GetNetworkInfo).await) {
            State::EnableGps
        } else {
            State::ErrorConnection
        }
    }

    pub async fn enable_gps(&mut self) -> State {
        info!("[Quectel] Enable GPS");
        if check_result(self.client.send(&EnableGpsFunc).await) {
            State::EnableAssistGps
        } else {
            State::ErrorConnection
        }
    }

    pub async fn enable_assist_gps(&mut self) -> State {
        info!("[Quectel] Enable Assist GPS");
        if check_result(self.client.send(&EnableAssistGpsFunc).await) {
            State::GetGPSData
        } else {
            State::ErrorConnection
        }
    }

    pub async fn get_gps_data(&mut self) -> State {
        info!("[Quectel] Retrieving GPS Data");
        let trip_result = self.client.send(&RetrieveGpsRmc).await;

        let active_net = self.active_connection_chan.receiver();
        let connetion_event = active_net.try_receive();
        info!("[Quectel] Waiting for active network connection {connetion_event:?}");

        match trip_result {
            Ok(res) => {
                info!("[LTE] GPS RMC data received: {res:?}");
                let timestamp = utc_date_to_unix_timestamp(&res.utc, &res.date);
                let mut device_id = String::new();
                let mut trip_id = String::new();
                write!(&mut trip_id, "{}", self.mqtt_client_id).unwrap();
                write!(&mut device_id, "{}", self.mqtt_client_id).unwrap();

                let trip_data = TripData {
                    device_id,
                    trip_id,
                    latitude: ((res.latitude as u64 / 100) as f64)
                        + ((res.latitude % 100.0f64) / 60.0f64),
                    longitude: ((res.longitude as u64 / 100) as f64)
                        + ((res.longitude % 100.0f64) / 60.0f64),
                    timestamp,
                };

                if self.gps_channel.try_send(trip_data.clone()).is_err() {
                    error!("[Quectel] Failed to send TripData to channel");
                    State::ResetHardware
                } else {
                    info!("[Quectel] GPS data sent to channel: {trip_data:?}");
                    // TODO: Add logic to handle WiFi availability
                    State::SetModemFunctionality
                }
            }
            Err(e) => {
                warn!("[Quectel] Failed to retrieve GPS data: {e:?}");
                State::ResetHardware
            }
        }
    }

    pub async fn set_modem_functionality(&mut self) -> State {
        info!("[Quectel] Set Modem Functionality");
        if check_result(
            self.client
                .send(&SetUeFunctionality {
                    fun: FunctionalityLevelOfUE::Full,
                })
                .await,
        ) {
            State::UploadMqttCert
        } else {
            State::ErrorConnection
        }
    }

    pub async fn upload_mqtt_cert(&mut self) -> State {
        info!("[Quectel] Upload Files");
        let res = upload_mqtt_cert_files(
            &mut self.client,
            self.urc_channel,
            self.ca_chain,
            self.certificate,
            self.private_key,
        )
        .await;
        if res {
            State::CheckNetworkRegistration
        } else {
            error!("[Quectel] File upload failed, resetting hardware");
            State::ErrorConnection
        }
    }

    pub async fn check_network_registration(&mut self) -> State {
        info!("[Quectel] Check Network Registration");
        let res = check_network_registration(&mut self.client).await;
        if res {
            if !self.is_connected {
                let _ = self
                    .conn_event_chan
                    .try_send(ConnectionEvent::LteRegistered);
                self.is_connected = true;
            }
            State::MqttOpenConnection
        } else {
            if self.is_connected {
                let _ = self
                    .conn_event_chan
                    .try_send(ConnectionEvent::LteUnregistered);
                self.is_connected = false;
            }
            error!("[Quectel] Network registration failed, resetting hardware");
            State::ErrorConnection
        }
    }

    pub async fn mqtt_open_connection(&mut self) -> State {
        info!("[Quectel] Opening MQTT connection");
        match open_mqtt_connection(&mut self.client, self.urc_channel).await {
            Ok(_) => {
                info!("[Quectel] MQTT connection opened successfully");
                State::MqttConnectBroker
            }
            Err(e) => {
                error!("[Quectel] Failed to open MQTT connection: {e:?}");
                State::ErrorConnection
            }
        }
    }

    pub async fn mqtt_connect_broker(&mut self) -> State {
        info!("[Quectel] Connecting to MQTT broker");
        match connect_mqtt_broker(&mut self.client, self.urc_channel).await {
            Ok(_) => {
                info!("[Quectel] MQTT connection established");
                let _ = self.conn_event_chan.try_send(ConnectionEvent::LteConnected);
                State::MqttPublishData
            }
            Err(e) => {
                error!("[Quectel] MQTT connection failed: {e:?}");
                let _ = self
                    .conn_event_chan
                    .try_send(ConnectionEvent::LteDisconnected);
                State::ErrorConnection
            }
        }
    }

    pub async fn mqtt_publish_data(&mut self) -> State {
        info!("[Quectel] Publishing MQTT Data");
        if handle_publish_mqtt_data(
            &mut self.client,
            self.mqtt_client_id,
            self.gps_channel,
            self.can_channel,
        )
        .await
        {
            info!("[Quectel] MQTT data published successfully");
            State::MqttPublishData
        } else {
            error!("[Quectel] MQTT publish failed");
            State::ErrorConnection
        }
    }

    pub async fn error_connection(&mut self) -> State {
        if self.is_connected {
            let _ = self
                .conn_event_chan
                .try_send(ConnectionEvent::LteDisconnected);
            self.is_connected = false;
        }
        error!("[Quectel] System in error state - attempting recovery");
        embassy_time::Timer::after(embassy_time::Duration::from_secs(5)).await;
        State::ResetHardware
    }
}

// Optional RX handler for URCs
#[embassy_executor::task]
pub async fn quectel_rx_handler(
    mut ingress: Ingress<'static, DefaultDigester<Urc>, Urc, 1024, 128, 3>,
    mut reader: UartRx<'static, Async>,
) -> ! {
    ingress.read_from(&mut reader).await
}
