#![no_std]
#![no_main]

mod cfg;
mod hal;
mod net;
mod task;
mod util;

//use crate::hal::flash;
use crate::cfg::net_cfg::MQTT_CLIENT_ID;
use crate::net::atcmd::Urc;
use atat::{ResponseSlot, UrcChannel};
use core::{fmt::Debug, fmt::Write, str::FromStr};
use embassy_executor::Spawner;
use embassy_net::{Stack, StackResources};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
#[cfg(feature = "ota")]
use esp_backtrace as _;
#[cfg(feature = "wdg")]
use esp_hal::rtc_cntl::{Rtc, RwdtStage};

use esp_hal::{
    clock::CpuClock,
    gpio::Output,
    rng::Trng,
    timer::timg::TimerGroup,
    twai::{self, TwaiMode},
    uart::{Config, Uart},
};
use esp_wifi::{init, wifi::WifiStaDevice, EspWifiController};
use log::{error, info};
use static_cell::StaticCell;
use task::can::*;
// use task::lte::TripData;
use task::mqtt::*;
use task::netmgr::net_manager_task;
#[cfg(feature = "ota")]
use task::ota::ota_handler;
use task::quectel::*;
// use task::lte::*;

use task::wifi::*;
pub type GpsOutbox = Channel<NoopRawMutex, TripData, 8>;
static GPS_CHANNEL: StaticCell<GpsOutbox> = StaticCell::new();
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
    esp_println::logger::init_logger_from_env();
    let peripherals = esp_hal::init({
        let mut config = esp_hal::Config::default();
        config.cpu_clock = CpuClock::max();
        config
    });
    info!("Telematic started");
    esp_alloc::heap_allocator!(200 * 1024);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let timg1 = TimerGroup::new(peripherals.TIMG1);
    esp_hal_embassy::init(timg1.timer0);
    let trng = &mut *mk_static!(Trng<'static>, Trng::new(peripherals.RNG, peripherals.ADC1));
    let init = &*mk_static!(
        EspWifiController<'static>,
        init(timg0.timer0, trng.rng, peripherals.RADIO_CLK).unwrap()
    );
    let wifi = peripherals.WIFI;
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(init, wifi, WifiStaDevice).unwrap();
    let config = embassy_net::Config::dhcpv4(Default::default());
    #[cfg(feature = "wdg")]
    let mut rtc = {
        let mut rtc = Rtc::new(peripherals.LPWR);
        rtc.rwdt.enable();
        rtc.rwdt.set_timeout(RwdtStage::Stage0, 5.secs());
        rtc
    };

    let seed = 1234;

    let (stack, runner) = embassy_net::new(
        wifi_interface,
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
    let quectel_pen_pin = Output::new(peripherals.GPIO21, esp_hal::gpio::Level::High);
    let quectel_dtr_pin = Output::new(peripherals.GPIO22, esp_hal::gpio::Level::High);

    let config = Config::default().with_rx_fifo_full_threshold(64);
    let uart0 = Uart::new(peripherals.UART0, config)
        .unwrap()
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
    static CHANNEL: StaticCell<TwaiOutbox> = StaticCell::new();
    let channel = &*CHANNEL.init(Channel::new());
    let (can_rx, _can_tx) = can.split();

    let gps_channel = &*GPS_CHANNEL.init(Channel::new());

    spawner.spawn(can_receiver(can_rx, channel)).unwrap();
    // spawner.spawn(connection(controller)).unwrap();
    // spawner.spawn(net_task(runner)).unwrap();
    // spawner
    //     .spawn(mqtt_handler(
    //         stack,
    //         channel,
    //         gps_channel,
    //         peripherals.SHA,
    //         peripherals.RSA,
    //     ))
    //     .unwrap();

    // spawner.spawn(net_manager_task(spawner)).unwrap();
    // spawner.spawn(quectel_rx_handler(ingress, uart_rx)).ok();
    // spawner
    //     .spawn(quectel_tx_handler(
    //         client,
    //         quectel_pen_pin,
    //         quectel_dtr_pin,
    //         &URC_CHANNEL,
    //         gps_channel,
    //         channel,
    //     ))
    //     .ok();
    // ====================================
    // === Spawn RX handler Quectel ===
    // ====================================
    spawner.spawn(quectel_rx_handler(ingress, uart_rx)).ok();

    // ====================================
    // === Quectel flow API driver ===
    // ====================================
    let mut quectel = Quectel::new(client, quectel_pen_pin, quectel_dtr_pin, &URC_CHANNEL);

    match quectel.quectel_initialize().await {
        Ok(()) => {
            info!("[main] Modem initialized successfully");
        }
        Err(e) => {
            error!("[main] Modem init failed: {:?}", e);
            Timer::after(Duration::from_secs(5)).await;
        }
    }

    let ca_chain = include_str!("../certx/crt.pem").as_bytes();
    let certificate = include_str!("../certx/dvt.crt").as_bytes();
    let private_key = include_str!("../certx/dvt.key").as_bytes();
    // Initialize LTE

    match quectel
        .lte_initialize(MQTT_CLIENT_ID, ca_chain, certificate, private_key)
        .await
    {
        Ok(()) => {
            info!("[main] Modem initialized successfully");
        }
        Err(e) => {
            error!("[main] Modem init failed: {:?}", e);
            Timer::after(Duration::from_secs(5)).await;
        }
    }
    match quectel.gps_initialize().await {
        Ok(()) => {
            info!("[main] LTE initialized successfully");
        }
        Err(e) => {
            error!("[main] LTE init failed: {:?}", e);
            Timer::after(Duration::from_secs(5)).await;
        }
    }

    match quectel.lte_handle_mqtt().await {
        Ok(()) => {
            info!("[main] LTE initialized successfully");
        }
        Err(e) => {
            error!("[main] LTE init failed: {:?}", e);
            Timer::after(Duration::from_secs(5)).await;
        }
    }
    // Spawn GPS state machine task
    spawner
        .spawn(gps_task(quectel, MQTT_CLIENT_ID, gps_channel))
        .unwrap();

    #[cfg(feature = "ota")]
    //wait until wifi connected
    {
        loop {
            if stack.is_link_up() {
                break;
            }
            Timer::after(Duration::from_millis(500)).await;
        }
        spawner.spawn(ota_handler(spawner, trng, stack)).unwrap();
    }

    // WDG feed task
    loop {
        Timer::after_secs(2).await;
        #[cfg(feature = "wdg")]
        rtc.rwdt.feed();
    }
}
