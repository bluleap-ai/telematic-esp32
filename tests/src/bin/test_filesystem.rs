#![no_std]
#![no_main]
#[allow(clippy::needless_range_loop)]
#[allow(unused_imports)]
#[path = "../../../src/mem/ex_flash.rs"]
mod ex_flash;
#[allow(unused_imports)]
#[path = "../../../src/mem/fs.rs"]
mod fs;
// extern crate alloc;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::gpio::OutputConfig;
use esp_hal::time::Rate;
use esp_hal::{
    clock::CpuClock,
    gpio::Output,
    spi::{
        master::{Config, Spi},
        Mode,
    },
    timer::timg::TimerGroup,
};
use ex_flash::W25Q128FVSG;
use fs::{create_dir, list_files, read_file, write_file, FlashFs};
use littlefs2::fs::{Allocation, FileAllocation, ReadDirAllocation};
use log::{error, info};
const FIRMWARE: &[u8] = include_bytes!("../firmware.bin");
const CA_CRT: &[u8] = include_bytes!("../../../certs/ca.crt");
const DVT_CRT: &[u8] = include_bytes!("../../../certs/dvt.crt");
const DVT_KEY: &[u8] = include_bytes!("../../../certs/dvt.key");
esp_bootloader_esp_idf::esp_app_desc!();
#[esp_hal_embassy::main]
async fn main(_spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    info!("=== Flash File System Test Starting ===");
    esp_alloc::heap_allocator!(size: 200 * 1024);
    let peripherals = esp_hal::init({
        let config = esp_hal::Config::default();
        let _ = config.with_cpu_clock(CpuClock::max());
        config
    });

    let timg1 = TimerGroup::new(peripherals.TIMG1);
    esp_hal_embassy::init(timg1.timer0);

    let sclk = peripherals.GPIO18;
    let miso = peripherals.GPIO20;
    let mosi = peripherals.GPIO19;
    let cs = Output::new(
        peripherals.GPIO3,
        esp_hal::gpio::Level::High,
        OutputConfig::default(),
    );
    let spi = Spi::new(
        peripherals.SPI2,
        Config::default()
            .with_frequency(Rate::from_mhz(10u32))
            .with_mode(Mode::_0),
    )
    .unwrap()
    .with_sck(sclk)
    .with_mosi(mosi)
    .with_miso(miso);

    let flash = W25Q128FVSG::new(spi, cs);
    info!("Flash initialized");

    let mut flash_fs = FlashFs::new(flash);
    log::info!("PASS ✅");
    let mut alloc = Allocation::new();
    let mut file_alloc = FileAllocation::new();
    let mut dir_alloc = ReadDirAllocation::new();
    let mut fs = flash_fs.init(&mut alloc).await.expect("Failed to init FS");
    log::info!("Creating cert directory");
    create_dir(&mut fs, "/cert\0").ok();
    log::info!("PASS ✅");
    log::info!("CA_CRT size: {}", CA_CRT.len());
    log::info!("DVT_CRT size: {}", DVT_CRT.len());
    log::info!("DVT_KEY size: {}", DVT_KEY.len());
    log::info!("Firmware size: {}", FIRMWARE.len());

    match write_file(&mut fs, "/cert/ca.crt\0", CA_CRT, &mut file_alloc) {
        Ok(_) => info!("Successfully wrote ca.crt"),
        Err(e) => error!("Failed to write ca.crt: {e:?}"),
    }
    log::info!("PASS ✅");
    Timer::after(Duration::from_millis(5)).await;
    match write_file(&mut fs, "/cert/dvt.crt\0", DVT_CRT, &mut file_alloc) {
        Ok(_) => info!("Successfully wrote dvt.crt"),
        Err(e) => error!("Failed to write dvt.crt: {e:?}"),
    }
    log::info!("PASS ✅");
    Timer::after(Duration::from_millis(5)).await;
    match write_file(&mut fs, "/cert/dvt.key\0", DVT_KEY, &mut file_alloc) {
        Ok(_) => info!("Successfully wrote dvt.key"),
        Err(e) => error!("Failed to write dvt.key: {e:?}"),
    }
    Timer::after(Duration::from_millis(5)).await;
    log::info!("PASS ✅");
    match list_files(&mut fs, "/cert\0", &mut dir_alloc) {
        Ok(_) => info!("Directory listing completed"),
        Err(e) => error!("Failed to list files: {e:?}"),
    }
    log::info!("PASS ✅");
    log::info!("Creating firmware directory");
    create_dir(&mut fs, "/firmware\0").ok();
    log::info!("PASS ✅");
    match write_file(
        &mut fs,
        "/firmware/firmware.bin\0",
        FIRMWARE,
        &mut file_alloc,
    ) {
        Ok(_) => info!("Successfully wrote firmware.bin"),
        Err(e) => error!("Failed to write FIRMWARE.BIN: {e:?}"),
    }
    log::info!("PASS ✅");
    match list_files(&mut fs, "/firmware\0", &mut dir_alloc) {
        Ok(_) => info!("Directory listing completed"),
        Err(e) => error!("Failed to list files: {e:?}"),
    }
    log::info!("PASS ✅");
    let mut file_alloc = FileAllocation::new();
    let mut buffer = [0u8; 4096];

    match read_file(
        &mut fs,
        "/firmware/firmware.bin\0",
        &mut buffer,
        &mut file_alloc,
    ) {
        Ok(n) => {
            #[allow(clippy::needless_range_loop)]
            for i in 0..core::cmp::min(n, 32) {
                log::info!("Byte {} = 0x{:02X}", i, buffer[i]);
            }

            log::info!("Read {n} bytes from firmware.bin");
            let preview = core::str::from_utf8(&buffer[..n]).unwrap_or("<non-utf8>");
            log::info!("Content preview: {preview}");
            log::info!("PASS ✅");
        }
        Err(e) => {
            log::error!("Failed to read firmware.bin: {e:?}");
        }
    }

    loop {
        Timer::after(Duration::from_secs(5)).await;
        info!("System running... Flash usage: Firmware + Certificates stored");
    }
}
