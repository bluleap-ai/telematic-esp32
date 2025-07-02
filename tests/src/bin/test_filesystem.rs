#![no_std]
#![no_main]

// #[path = "../../../src/mem/ex_flash.rs"]
// mod ex_flash
#[allow(unused_imports)]
#[path = "../../../src/mem/filesystem.rs"]
mod filesystem;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::time::RateExtU32;
use esp_hal::{
    clock::CpuClock,
    gpio::Output,
    spi::{
        master::{Config, Spi},
        Mode,
    },
    timer::timg::TimerGroup,
};
use filesystem::{FlashController, FlashRegion};
use log::{error, info};

#[esp_hal_embassy::main]
async fn main(_spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    info!("=== Flash File System Test Starting ===");

    let peripherals = esp_hal::init({
        let mut config = esp_hal::Config::default();
        config.cpu_clock = CpuClock::max();
        config
    });

    esp_alloc::heap_allocator!(200 * 1024);
    let timg1 = TimerGroup::new(peripherals.TIMG1);
    esp_hal_embassy::init(timg1.timer0);

    let sclk = peripherals.GPIO18;
    let miso = peripherals.GPIO20;
    let mosi = peripherals.GPIO19;
    let cs = Output::new(peripherals.GPIO3, esp_hal::gpio::Level::High);
    let spi = Spi::new(
        peripherals.SPI2,
        Config::default()
            .with_frequency(10u32.MHz())
            .with_mode(Mode::_0),
    )
    .unwrap()
    .with_sck(sclk)
    .with_mosi(mosi)
    .with_miso(miso);
    let mut flash = filesystem::ex_flash::W25Q128FVSG::new(spi, cs);
    match flash.init().await {
        Ok(()) => info!("✓ Flash initialized successfully"),
        Err(e) => {
            error!("✗ Flash initialization failed: {e:?}");
            panic!("Cannot continue without flash");
        }
    }
    let mut fs = FlashController::new(&mut flash);

    // Erase Certstore region to ensure a clean slate
    match fs.erase_region(FlashRegion::Certstore).await {
        Ok(()) => info!("✓ Certstore region erased successfully"),
        Err(e) => {
            error!("✗ Failed to erase Certstore region: {e:?}");
            panic!("Cannot continue without erasing Certstore");
        }
    }

    let firmware = include_bytes!("../../../firmware.bin");
    let ca_chain = include_bytes!("../../../certs/ca.crt");
    let cert_data = include_bytes!("../../../certs/dvt.crt");
    let private_key = include_bytes!("../../../certs/dvt.key");
    info!("=== Starting File Write Operations ===");
    info!("Files to write:");
    info!("  - firmware: {} bytes", firmware.len());
    info!("  - ca.crt: {} bytes", ca_chain.len());
    info!("  - dvt.crt: {} bytes", cert_data.len());
    info!("  - dvt.key: {} bytes", private_key.len());

    match fs
        .write_firmware(FlashRegion::Firmware, "firmware.bin", firmware)
        .await
    {
        Ok(()) => info!("✓ Firmware written successfully"),
        Err(e) => error!("✗ Failed to write Firmware: {e:?}"),
    }
    match fs
        .write_file(FlashRegion::Certstore, "ca.crt", ca_chain)
        .await
    {
        Ok(()) => info!("✓ CA chain written successfully"),
        Err(e) => error!("✗ Failed to write CA chain: {e:?}"),
    }
    match fs
        .write_file(FlashRegion::Certstore, "dvt.crt", cert_data)
        .await
    {
        Ok(()) => info!("✓ Certificate written successfully"),
        Err(e) => error!("✗ Failed to write certificate: {e:?}"),
    }
    match fs
        .write_file(FlashRegion::Certstore, "dvt.key", private_key)
        .await
    {
        Ok(()) => info!("✓ Private key written successfully"),
        Err(e) => error!("✗ Failed to write private key: {e:?}"),
    }
    info!("=== Starting File Verification ===");
    let mut ok = true;
    ok &= fs
        .verify_file(FlashRegion::Certstore, "ca.crt", ca_chain)
        .await;
    ok &= fs
        .verify_file(FlashRegion::Certstore, "dvt.crt", cert_data)
        .await;
    ok &= fs
        .verify_file(FlashRegion::Certstore, "dvt.key", private_key)
        .await;

    if ok {
        info!("✓ All TLS files verified!");
    } else {
        error!("✗ Verification failed");
    }

    let (capacity, page_size, sector_size) = fs.get_flash_info().await;
    info!("Flash capacity: {capacity} bytes");
    info!("Page size: {page_size} bytes");
    info!("Sector size: {sector_size} bytes");

    // Dump flash for debugging

    info!("=== Flash File System Test Completed ===");
    //List file in CertStore
    info!("== List files in CertStore");
    match fs.list_files(FlashRegion::Certstore).await {
        Ok(files) => {
            if files.is_empty() {
                info!("(Certstore trống)");
            } else {
                for e in files.iter() {
                    info!("• {:<32}  {:>6} B  @0x{:06X}", e.name, e.len, e.offset);
                }
            }
        }
        Err(e) => error!("Không duyệt được Certstore: {e:?}"),
    }
    //Listfile in firmware
    match fs.list_files(FlashRegion::Firmware).await {
        Ok(files) => {
            if files.is_empty() {
                info!("(Certstore trống)");
            } else {
                for e in files.iter() {
                    info!("• {:<32}  {:>6} B  @0x{:06X}", e.name, e.len, e.offset);
                }
            }
        }
        Err(e) => error!("Không duyệt được Certstore: {e:?}"),
    }
    loop {
        Timer::after(Duration::from_secs(5)).await;
        info!("System running... Flash usage: Firmware + Certificates stored");
    }
}
