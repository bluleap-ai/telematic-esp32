#![no_std]
#![no_main]

// Declare modules at the crate root
mod cfg;
mod svc;
mod task;
mod util;

// Import the necessary modules
use crate::svc::atcmd::Urc;

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

use static_cell::StaticCell;

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

    // Init network stack
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
        const {
            twai::filter::SingleStandardFilter::new(
                b"xxxxxxxxxxx",
                b"x",
                [b"xxxxxxxx", b"xxxxxxxx"],
            )
        },
    );
    let can = twai_config.start();
    static CHANNEL: StaticCell<TwaiOutbox> = StaticCell::new();
    let channel = &*CHANNEL.init(Channel::new());
    let (can_rx, _can_tx) = can.split();

    spawner.spawn(can_receiver(can_rx, channel)).ok();
    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(runner)).ok();
    spawner
        .spawn(mqtt_handler(
            stack,
            channel,
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
        ))
        .ok();
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
