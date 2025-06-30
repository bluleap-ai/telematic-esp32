#![no_std]
#![no_main]

// Declare modules at the crate root
mod cfg;
mod hal;
mod net;
mod task;
mod util;

// Import the necessary modules
//use crate::hal::flash;
use crate::net::atcmd::Urc;
use task::can::*;
use task::lte::*;
use task::mqtt::*;
#[cfg(feature = "ota")]
use task::ota::ota_handler;
use task::wifi::*;

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
    gpio::Output,
    rng::Trng,
    timer::timg::TimerGroup,
    twai::{self, TwaiMode},
    uart::{Config, Uart},
};
use esp_wifi::{init, wifi::WifiStaDevice, EspWifiController};
use log::info;
use static_cell::StaticCell;
use task::lte::TripData;
use task::netmgr::net_manager_task;
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
/*
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
    //let trng = &*mk_static!(
    //    Mutex<Trng<'static>, EspWifiController<'static>>,
    //    Mutex::new(Trng::new(peripherals.RNG, peripherals.ADC1))
    //);
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
    spawner.spawn(can_receiver(can_rx, channel)).ok();
    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(runner)).ok();
    spawner
        .spawn(mqtt_handler(
            stack,
            channel,
            gps_channel,
            peripherals.SHA,
            peripherals.RSA,
        ))
        .ok();
    spawner.spawn(quectel_rx_handler(ingress, uart_rx)).ok();
    spawner
        .spawn(quectel_tx_handler(
            client,
            quectel_pen_pin,
            quectel_dtr_pin,
            &URC_CHANNEL,
            gps_channel,
            channel,
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

*/

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::peripherals::Peripherals::take().unwrap();
    let mut gpio = peripherals.GPIO;
    let uart = peripherals.UART0;

    let tx_pin = gpio.GPIO23;
    let rx_pin = gpio.GPIO15;
    let pen_pin = gpio.GPIO21;
    let dtr_pin = gpio.GPIO22;

    static GPS_CHANNEL: Channel<NoopRawMutex, TripData, 8> = Channel::new();
    static CAN_CHANNEL: Channel<NoopRawMutex, TwaiOutbox, 8> = Channel::new();
    static CONN_EVENT_CHAN: Channel<NoopRawMutex, quectel::ConnectionEvent, 8> = Channel::new();
    static ACTIVE_CONNECTION_CHAN: Channel<NoopRawMutex, (), 1> = Channel::new();
    static URC_CHANNEL: quectel::UrcChannel<Urc, 128, 3> = quectel::UrcChannel::new();

    static CA_CHAIN: &[u8] = include_bytes!("../certs/crt.pem");
    static CERTIFICATE: &[u8] = include_bytes!("../certs/dvt.crt");
    static PRIVATE_KEY: &[u8] = include_bytes!("../certs/dvt.key");
    static MQTT_CLIENT_ID: &str = "my_mqtt_client_id";

    let (mut quectel, ingress, uart_rx) = QuectelDefault::init(
        &mut gpio,
        uart,
        tx_pin,
        rx_pin,
        pen_pin,
        dtr_pin,
        CA_CHAIN,
        CERTIFICATE,
        PRIVATE_KEY,
        MQTT_CLIENT_ID,
        &URC_CHANNEL,
        &GPS_CHANNEL,
        &CAN_CHANNEL,
        &CONN_EVENT_CHAN,
        &ACTIVE_CONNECTION_CHAN,
    );

    // Optional: Spawn RX handler for URCs
    spawner
        .spawn(quectel_rx_handler(ingress, uart_rx))
        .expect("Failed to spawn RX handler");

    // Call specific methods
    quectel.reset_hardware().await;
    quectel.enable_gps().await;
    quectel.enable_assist_gps().await;

    loop {
        match quectel.get_gps().await {
            Ok(gps_data) => {
                info!("[Main] GPS data: {:?}", gps_data);
                info!(
                    "[Main] GPS - Latitude: {}, Longitude: {}, Timestamp: {}",
                    gps_data.latitude, gps_data.longitude, gps_data.timestamp
                );
            }
            Err(e) => {
                warn!("[Main] Failed to get GPS data: {:?}", e);
            }
        }
        embassy_time::Timer::after(embassy_time::Duration::from_secs(5)).await;
    }
}
