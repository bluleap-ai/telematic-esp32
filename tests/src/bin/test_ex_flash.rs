#![no_std]
#![no_main]

#[allow(unused_imports)]
#[path = "../../../src/mem/ex_flash.rs"]
mod ex_flash;

use embassy_executor::Spawner;
use embassy_time::Timer;
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
use ex_flash::{ExFlashError, W25Q128FVSG};
use log::{error, info, warn};

#[esp_hal_embassy::main]
async fn main(_spawner: Spawner) -> ! {
    // Initialize ESP HAL for ESP32C6
    info!("Initializing HAL...");
    esp_println::logger::init_logger_from_env();
    let peripherals = esp_hal::init({
        let mut config = esp_hal::Config::default();
        config.cpu_clock = CpuClock::max();
        config
    });
    // Initialize the timer group for embassy
    esp_alloc::heap_allocator!(200 * 1024);
    let timg1 = TimerGroup::new(peripherals.TIMG1);
    esp_hal_embassy::init(timg1.timer0);
    // Initialize peripherals
    let sclk = peripherals.GPIO18;
    let miso = peripherals.GPIO20;
    let mosi = peripherals.GPIO19;
    let cs_pin = Output::new(peripherals.GPIO3, esp_hal::gpio::Level::High);
    let spi = Spi::new(
        peripherals.SPI2,
        Config::default()
            .with_frequency(100_u32.kHz())
            .with_mode(Mode::_0), // CPOL = 0, CPHA = 0 (Mode 0 per datasheet)
    )
    .unwrap()
    .with_sck(sclk)
    .with_mosi(mosi)
    .with_miso(miso);

    // Run the test directly
    info!("Starting flash communication test...");

    // Software CS pin control, enable chip select
    let mut flash = W25Q128FVSG::new(spi, cs_pin);

    // Initialize the flash chip
    info!("Initializing flash...");
    match flash.init().await {
        Ok(()) => info!("Flash initialized successfully."),
        Err(e) => {
            error!("Flash initialization failed: {e:?}");
            panic!("Cannot continue without flash initialization");
        }
    }

    // Read JEDEC ID
    info!("Reading JEDEC ID...");
    match flash.read_id() {
        Ok(id) => {
            info!("JEDEC ID: {id:02x?}");
            // Expected ID for W25Q128FV: [0xEF, 0x40, 0x18]
            if id[0] == 0xEF && id[1] == 0x40 && id[2] == 0x18 {
                info!("✓ Correct W25Q128FV detected");
            } else {
                warn!("⚠ Unexpected JEDEC ID, expected [0xEF, 0x40, 0x18]");
            }
        }
        Err(e) => {
            error!("Failed to read JEDEC ID: {e:?}");
        }
    }

    // Display flash info
    info!("Flash capacity: {} MB", flash.capacity() / (1024 * 1024));
    info!("Page size: {} bytes", flash.page_size());
    info!("Sector size: {} bytes", flash.sector_size());

    // Erase the entire chip, comment out as it takes a long time
    // Uncomment the following lines to erase the chip
    /*
    info!("Erasing entire chip...");
    match flash.erase_chip().await {
        Ok(()) => info!("Chip erased successfully."),
        Err(e) => error!("Chip erase failed: {:?}", e),
    }
    */

    // Test writing and reading data
    let address = 0x1000;
    let write_data = [0xDE, 0xAD, 0xBE, 0xEF];
    let mut read_data = [0u8; 4];

    info!("Writing data to address {address:#08x}: {write_data:02x?}");
    match flash.write_data(address, &write_data).await {
        Ok(()) => info!("✓ Data written successfully"),
        Err(e) => {
            error!("✗ Write failed: {e:?}");
            match e {
                ExFlashError::AddressInvalid => error!("Invalid address {address:#08x}"),
                ExFlashError::PageBoundaryViolation => error!("Write crosses page boundary"),
                ExFlashError::LenInvalid => error!("Invalid data length"),
                ExFlashError::SpiError => error!("SPI communication error"),
                ExFlashError::WriteEnableFailed => error!("Could not enable write"),
                ExFlashError::Timeout => error!("Took too long"),
                ExFlashError::WriteFailed => error!("Failed to write data"),
                //_ => error!("Other write error"),
            }
        }
    }

    info!("Reading data from address {address:#08x}...");
    match flash.read_data(address, &mut read_data) {
        Ok(()) => {
            info!("✓ Data read successfully: {read_data:02x?}");
            if read_data == write_data {
                info!("✓ Write/Read test PASSED");
            } else {
                error!(
                    "✗ Write/Read data mismatch: wrote {write_data:02x?}, read {read_data:02x?}"
                );
            }
        }
        Err(e) => {
            error!("✗ Read failed: {e:?}");
            match e {
                ExFlashError::AddressInvalid => error!("Invalid address {address:#08x}"),
                ExFlashError::LenInvalid => error!("Invalid buffer length"),
                ExFlashError::SpiError => error!("SPI communication error"),
                _ => error!("Other read error"),
            }
        }
    }

    // Test sector erase
    let sector_addr = address; // Use the actual address, not sector number
    info!("Erasing sector at address {sector_addr:#08x}...");
    match flash.erase_sector(sector_addr).await {
        Ok(()) => info!("✓ Sector erased successfully"),
        Err(e) => {
            error!("✗ Sector erase failed: {e:?}");
            match e {
                ExFlashError::AddressInvalid => {
                    error!("Invalid sector address {sector_addr:#08x}")
                }
                ExFlashError::SpiError => error!("SPI communication error during erase"),
                ExFlashError::WriteEnableFailed => error!("Could not enable write for erase"),
                _ => error!("Other erase error"),
            }
        }
    }

    // Verify erase by reading back
    info!("Verifying erase at address {address:#08x}...");
    match flash.read_data(address, &mut read_data) {
        Ok(()) => {
            info!("Read after erase: {read_data:02x?}");
            if read_data == [0xFF, 0xFF, 0xFF, 0xFF] {
                info!("✓ Sector erase verification PASSED");
            } else {
                error!(
                    "✗ Sector erase verification FAILED: expected [0xFF; 4], got {read_data:02x?}"
                );
            }
        }
        Err(e) => {
            error!("✗ Read verification failed: {e:?}");
        }
    }

    // Test single byte operations
    info!("Testing single byte operations...");
    let byte_addr = 0x2000;
    let test_byte = 0x42;

    match flash.write_byte(byte_addr, test_byte).await {
        Ok(()) => info!("✓ Single byte written successfully"),
        Err(e) => error!("✗ Single byte write failed: {e:?}"),
    }

    match flash.read_byte(byte_addr) {
        Ok(byte) => {
            info!("Single byte read: {byte:#04x}");
            if byte == test_byte {
                info!("✓ Single byte test PASSED");
            } else {
                error!("✗ Single byte test FAILED: wrote {test_byte:#04x}, read {byte:#04x}");
            }
        }
        Err(e) => error!("✗ Single byte read failed: {e:?}"),
    }

    // Test boundary conditions
    info!("Testing boundary conditions...");

    // Test writing at page boundary
    let page_boundary = 0x2100; // Should be at page boundary (256-byte aligned)
    let boundary_data = [0x12, 0x34];
    match flash.write_data(page_boundary, &boundary_data).await {
        Ok(()) => info!("✓ Page boundary write successful"),
        Err(e) => info!("Page boundary write result: {e:?}"),
    }

    // Test invalid address (beyond flash capacity)
    let invalid_addr = flash.capacity() + 1000;
    let mut dummy_buffer = [0u8; 4];
    match flash.read_data(invalid_addr, &mut dummy_buffer) {
        Ok(()) => error!("✗ Should have failed with invalid address"),
        Err(ExFlashError::AddressInvalid) => info!("✓ Invalid address correctly rejected"),
        Err(e) => info!("Invalid address test result: {e:?}"),
    }

    info!("Flash test completed successfully!");
    info!("All tests finished.");

    loop {
        Timer::after_secs(5).await;
        info!(
            "System running... Flash capacity: {} MB",
            flash.capacity() / (1024 * 1024)
        );
    }
}
