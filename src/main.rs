#![no_std]
#![no_main]

// Declare modules at the crate root
mod cfg;
mod hal;
mod modem;
mod net;
mod task;
mod util;
// Import the necessary modules
//use crate::hal::flash;
use crate::cfg::net_cfg::*;
use crate::net::atcmd::Urc;
// Import the necessary modules
use atat::{ResponseSlot, UrcChannel};
use embassy_executor::Spawner;
use embassy_net::{Stack, StackResources};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
#[cfg(feature = "ota")]
use embassy_time::Duration;
use embassy_time::Timer;
use esp_backtrace as _;
#[cfg(feature = "wdg")]
use esp_hal::rtc_cntl::{Rtc, RwdtStage};
use esp_hal::{
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig},
    rng::Trng,
    timer::timg::TimerGroup,
    twai::{self, TwaiMode},
    uart::{Config, RxConfig, Uart},
};
use esp_wifi::{init, EspWifiController, InitializationError};
use log::{error, info};
use modem::*;
use static_cell::StaticCell;
use task::can::*;
use task::lte::*;
use task::mqtt::*;
use task::netmgr::net_manager_task;
#[cfg(feature = "ota")]
use task::ota::ota_handler;
use task::wifi::*;
pub type GpsOutbox = Channel<NoopRawMutex, TripData, 8>;

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) -> ! {
    // Init logging system
    esp_println::logger::init_logger_from_env();

    // Init peripherals
    let peripherals = esp_hal::init({
        let config = esp_hal::Config::default();
        let _ = config.with_cpu_clock(CpuClock::max());
        config
    });

    info!("Telematic started");
    esp_alloc::heap_allocator!(size: 200 * 1024);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let timg1 = TimerGroup::new(peripherals.TIMG1);
    esp_hal_embassy::init(timg1.timer0);
    // Init True Random Number Generator
    let trng = &mut *mk_static!(Trng<'static>, Trng::new(peripherals.RNG, peripherals.ADC1));

    // Init EspWifiController
    let init = &*mk_static!(
        EspWifiController<'static>,
        match init(timg0.timer0, trng.rng, peripherals.RADIO_CLK) {
            Ok(controller) => controller,
            Err(e) => {
                // Log specific error details based on the error type
                match e {
                    InitializationError::WrongClockConfig => {
                        error!("Failed to initialize WiFi - WrongClockConfig");
                    }
                    InitializationError::WifiError(wifi_err) => {
                        error!("Failed to initialize WiFi - WifiError: {wifi_err:?}");
                    }
                    InitializationError::General(code) => {
                        error!("Failed to initialize WiFi - General: {code:?}");
                    }
                    InitializationError::InterruptsDisabled => {
                        error!("Failed to initialize WiFi - Interrupts are disabled");
                    }
                    _ => todo!(),
                }

                // Wifi is a critical component (could replace the panic! with the code to reset later)
                panic!("WiFi initialization failure");
            }
        }
    );

    // Create Wifi interface
    let wifi = peripherals.WIFI;
    let (controller, wifi_interface) = match esp_wifi::wifi::new(init, wifi) {
        Ok(val) => val,
        Err(e) => {
            match e {
                esp_wifi::wifi::WifiError::NotInitialized => {
                    error!(" Wi-Fi module is not initialized or not initialized for `Wi-Fi` - NotInitialized");
                }
                esp_wifi::wifi::WifiError::InternalError(internal_err) => {
                    error!("Internal Wi-Fi error: {internal_err:?}");
                }
                esp_wifi::wifi::WifiError::Disconnected => {
                    error!("The device disconnected from the network or failed to connect to it - Disconnected");
                }
                esp_wifi::wifi::WifiError::UnknownWifiMode => {
                    error!("Unknown Wi-Fi mode (not Sta/Ap/ApSta) - UnknowWifiMode");
                }
                esp_wifi::wifi::WifiError::Unsupported => {
                    error!("Unsupported operation or mode - Unsupported");
                }
                esp_wifi::wifi::WifiError::InvalidArguments => {
                    error!("Invalid arguments provided to Wi-Fi function");
                }
                _ => todo!(),
            }
            // Could replace panic! with the code to reset process
            panic!("Fail to create Wifi interface");
        }
    };

    // Configure network stack for DHCP
    // modified: new_with_mode -> new, change position of wifi_interface and controller
    // let (controller, wifi_interface) = esp_wifi::wifi::new(init, wifi).unwrap();

    let config = embassy_net::Config::dhcpv4(Default::default());
    #[cfg(feature = "wdg")]
    let mut rtc = {
        let mut rtc = Rtc::new(peripherals.LPWR);
        rtc.rwdt.enable();
        rtc.rwdt.set_timeout(RwdtStage::Stage0, 5.secs());
        rtc
    };

    let seed = 1234;
    // modified: wifi_interface is a interface, not a WifiDevice, add ".sta" to get Device
    let (stack, runner) = embassy_net::new(
        wifi_interface.sta,
        config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );
    let stack = &*mk_static!(Stack, stack);

    // ==============================
    // === LTE UART + Quectel ===
    // ==============================
    let uart_tx_pin = peripherals.GPIO23;
    let uart_rx_pin = peripherals.GPIO15;
    let pen_pin = Output::new(peripherals.GPIO21, Level::High, OutputConfig::default());
    let dtr_pin = Output::new(peripherals.GPIO22, Level::High, OutputConfig::default());

    let config = Config::default().with_rx(RxConfig::default().with_fifo_full_threshold(64));
    let uart0 = Uart::new(peripherals.UART0, config)
        .expect("Fail to initialize UART0")
        .with_rx(uart_rx_pin)
        .with_tx(uart_tx_pin)
        .into_async();
    let (uart_rx, uart_tx) = uart0.split();

    static RES_SLOT: ResponseSlot<1024> = ResponseSlot::new();
    static URC_CHANNEL: UrcChannel<Urc, 128, 3> = UrcChannel::new();
    static INGRESS_BUF: StaticCell<[u8; 1024]> = StaticCell::new();
    let ingress = atat::Ingress::new(
        atat::AtDigester::<Urc>::default(),
        INGRESS_BUF.init([0; 1024]),
        &RES_SLOT,
        &URC_CHANNEL,
    );
    static BUF: StaticCell<[u8; 1024]> = StaticCell::new();
    let client = atat::asynch::Client::new(
        uart_tx,
        &RES_SLOT,
        BUF.init([0; 1024]),
        atat::Config::default(),
    );

    // ==============================
    // === CAN / MQTT ===
    // ==============================
    let can_tx_pin = peripherals.GPIO1;
    let can_rx_pin = peripherals.GPIO10;
    const CAN_BAUDRATE: twai::BaudRate = twai::BaudRate::B250K;
    let mut twai_config = twai::TwaiConfiguration::new(
        peripherals.TWAI0,
        can_rx_pin,
        can_tx_pin,
        CAN_BAUDRATE,
        TwaiMode::Normal,
    )
    .into_async();
    twai_config.set_filter(
        const { twai::filter::SingleExtendedFilter::new(b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxx", b"x") },
    );
    let can = twai_config.start();
    static CAN_CHANNEL: StaticCell<TwaiOutbox> = StaticCell::new();
    static GPS_CHANNEL: StaticCell<GpsOutbox> = StaticCell::new();
    let can_channel = &*CAN_CHANNEL.init(Channel::new());
    let (can_rx, _can_tx) = can.split();

    let gps_channel = &*GPS_CHANNEL.init(Channel::new());
    spawner.spawn(can_receiver(can_rx, can_channel)).ok();
    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(runner)).ok();
    spawner
        .spawn(wifi_mqtt_handler(
            stack,
            can_channel,
            gps_channel,
            peripherals.SHA,
            peripherals.RSA,
        ))
        .ok();

    // ====================================
    // === Spawn RX handler Quectel ===
    // ====================================
    spawner.spawn(modem_rx_handler(ingress, uart_rx)).ok();

    // ====================================
    // === Quectel flow API driver ===
    // ====================================

    let quectel = Modem::new(
        client,
        pen_pin,
        dtr_pin,
        &URC_CHANNEL,
        ModemModel::QuectelEG800k,
    );
    let ca_chain = include_str!("../certs/ca.crt").as_bytes();
    let certificate = include_str!("../certs/dvt.crt").as_bytes();
    let private_key = include_str!("../certs/dvt.key").as_bytes();

    // Handle spawner.spawn Result
    spawner
        .spawn(lte_mqtt_handler_fsm(
            MQTT_CLIENT_ID,
            quectel,
            can_channel,
            gps_channel,
            ca_chain,
            certificate,
            private_key,
        ))
        .ok();

    spawner.spawn(net_manager_task(spawner)).ok();
    #[cfg(feature = "ota")]
    //wait until wifi connected
    {
        loop {
            if stack.is_link_up() {
                break;
            }
            Timer::after(Duration::from_millis(500)).await;
        }
        spawner
            .spawn(ota_handler(spawner, trng, stack))
            .expect("Failed to spawn OTA handler task");
    }

    // WDG feed task
    loop {
        Timer::after_secs(2).await;
        #[cfg(feature = "wdg")]
        rtc.rwdt.feed();
    }
}
