#![no_std]
#![no_main]

#[allow(unused_imports)]
#[path = "../../../src/mem/ex_flash.rs"]
mod ex_flash;

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
use ex_flash::{ExFlashError, W25Q128FVSG};
use log::{error, info, warn};

#[derive(Debug)]
pub enum FsError {
    FlashError(ExFlashError),
    InvalidAddress,
    FileTooLarge,
    VerificationFailed,
}

impl From<ExFlashError> for FsError {
    fn from(err: ExFlashError) -> Self {
        FsError::FlashError(err)
    }
}

pub struct FlashController<'a> {
    flash: &'a mut W25Q128FVSG<'a>,
}

impl<'a> FlashController<'a> {
    pub fn new(flash: &'a mut W25Q128FVSG<'a>) -> Self {
        Self { flash }
    }

    pub async fn write_file(
        &mut self,
        address: u32,
        filename: &str,
        data: &[u8],
    ) -> Result<(), FsError> {
        info!(
            "Writing file '{}' to address 0x{:08X} ({} bytes)",
            filename,
            address,
            data.len()
        );

        // Validate address is within flash bounds
        if address >= self.flash.capacity() {
            error!("Address 0x{address:08X} exceeds flash capacity");
            return Err(FsError::InvalidAddress);
        }

        // Check if file would exceed flash capacity
        if address as u64 + data.len() as u64 > self.flash.capacity() as u64 {
            error!(
                "File size {} bytes would exceed flash capacity at address 0x{:08X}",
                data.len(),
                address
            );
            return Err(FsError::FileTooLarge);
        }

        if data.is_empty() {
            warn!("Writing empty file '{filename}'");
            return Ok(());
        }

        // Calculate sectors needed and erase them
        let sector_size = self.flash.sector_size() as u32;
        let start_sector = address / sector_size;
        let end_sector = (address + data.len() as u32 - 1) / sector_size;

        info!("Erasing sectors {start_sector} to {end_sector} for file '{filename}'");
        for sector in start_sector..=end_sector {
            let sector_addr = sector * sector_size;
            match self.flash.erase_sector(sector_addr).await {
                Ok(()) => info!("✓ Erased sector {sector} at 0x{sector_addr:08X}"),
                Err(e) => {
                    error!("✗ Failed to erase sector {sector}: {e:?}");
                    return Err(FsError::FlashError(e));
                }
            }
        }

        // Write data in page-sized chunks
        let page_size = self.flash.page_size();
        let mut offset = 0;
        let mut pages_written = 0;

        while offset < data.len() {
            let current_addr = address + offset as u32;
            let page_offset = current_addr as usize % page_size;
            let max_write = page_size - page_offset;
            let chunk_size = core::cmp::min(max_write, data.len() - offset);

            match self
                .flash
                .write_data(current_addr, &data[offset..offset + chunk_size])
                .await
            {
                Ok(()) => {
                    pages_written += 1;
                    if pages_written % 10 == 0 {
                        info!(
                            "Written {} pages ({} bytes) of file '{}'",
                            pages_written,
                            offset + chunk_size,
                            filename
                        );
                    }
                }
                Err(e) => {
                    error!(
                        "✗ Failed to write data at offset {offset} (address 0x{current_addr:08X}): {e:?}"
                    );
                    return Err(FsError::FlashError(e));
                }
            }
            offset += chunk_size;
        }

        info!(
            "✓ Successfully wrote file '{}' ({} bytes, {} pages)",
            filename,
            data.len(),
            pages_written
        );
        Ok(())
    }

    pub async fn read_file(&mut self, address: u32, buffer: &mut [u8]) -> Result<usize, FsError> {
        if buffer.is_empty() {
            return Ok(0);
        }

        // Validate address range
        if address >= self.flash.capacity() {
            return Err(FsError::InvalidAddress);
        }

        if address as u64 + buffer.len() as u64 > self.flash.capacity() as u64 {
            return Err(FsError::InvalidAddress);
        }

        match self.flash.read_data(address, buffer) {
            Ok(()) => {
                info!(
                    "✓ Read {} bytes from address 0x{:08X}",
                    buffer.len(),
                    address
                );
                Ok(buffer.len())
            }
            Err(e) => {
                error!(
                    "✗ Failed to read {} bytes from address 0x{:08X}: {:?}",
                    buffer.len(),
                    address,
                    e
                );
                Err(FsError::FlashError(e))
            }
        }
    }

    pub async fn verify_file(
        &mut self,
        address: u32,
        expected_data: &[u8],
    ) -> Result<bool, FsError> {
        if expected_data.is_empty() {
            return Ok(true);
        }

        info!(
            "Verifying {} bytes at address 0x{:08X}",
            expected_data.len(),
            address
        );

        let mut buffer = [0u8; 256]; // Read in 256-byte chunks
        let mut offset = 0;
        let mut chunks_verified = 0;

        while offset < expected_data.len() {
            let chunk_size = core::cmp::min(buffer.len(), expected_data.len() - offset);
            let current_addr = address + offset as u32;

            match self
                .flash
                .read_data(current_addr, &mut buffer[..chunk_size])
            {
                Ok(()) => {
                    if buffer[..chunk_size] != expected_data[offset..offset + chunk_size] {
                        error!(
                            "✗ Verification failed at offset {offset} (address 0x{current_addr:08X})"
                        );
                        error!(
                            "Expected: {:02X?}",
                            &expected_data[offset..offset + core::cmp::min(16, chunk_size)]
                        );
                        error!(
                            "Got:      {:02X?}",
                            &buffer[..core::cmp::min(16, chunk_size)]
                        );
                        return Ok(false);
                    }
                    chunks_verified += 1;
                    if chunks_verified % 10 == 0 {
                        info!(
                            "Verified {} chunks ({} bytes)",
                            chunks_verified,
                            offset + chunk_size
                        );
                    }
                }
                Err(e) => {
                    error!("✗ Failed to read data for verification at offset {offset}: {e:?}");
                    return Err(FsError::FlashError(e));
                }
            }
            offset += chunk_size;
        }

        info!("✓ Verification successful ({chunks_verified} chunks)");
        Ok(true)
    }

    pub async fn get_flash_info(&self) -> (u32, usize, usize) {
        (
            self.flash.capacity(),
            self.flash.page_size(),
            self.flash.sector_size(),
        )
    }
}

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

    // Initialize SPI and flash
    let sclk = peripherals.GPIO18;
    let miso = peripherals.GPIO20;
    let mosi = peripherals.GPIO19;
    let cs = Output::new(peripherals.GPIO3, esp_hal::gpio::Level::High);
    let spi = Spi::new(
        peripherals.SPI2,
        Config::default()
            .with_frequency(100u32.kHz())
            .with_mode(Mode::_0),
    )
    .unwrap()
    .with_sck(sclk)
    .with_mosi(mosi)
    .with_miso(miso);

    let mut flash = W25Q128FVSG::new(spi, cs);

    // Initialize flash with error handling
    match flash.init().await {
        Ok(()) => info!("✓ Flash initialized successfully"),
        Err(e) => {
            error!("✗ Flash initialization failed: {e:?}");
            panic!("Cannot continue without flash");
        }
    }

    // Read and display flash ID
    match flash.read_id() {
        Ok(id) => {
            info!("Flash JEDEC ID: {id:02X?}");
            if id == [0xEF, 0x40, 0x18] {
                info!("✓ W25Q128FV flash detected");
            } else {
                warn!("⚠ Unexpected flash ID, expected [0xEF, 0x40, 0x18]");
            }
        }
        Err(e) => error!("✗ Failed to read flash ID: {e:?}"),
    }

    let mut controller = FlashController::new(&mut flash);
    let (capacity, page_size, sector_size) = controller.get_flash_info().await;
    info!(
        "Flash Info: {} MB capacity, {} byte pages, {} byte sectors",
        capacity / (1024 * 1024),
        page_size,
        sector_size
    );

    // Test data
    let firmware_data = b"Firmware v1.0\nThis is test firmware data for embedded system";
    let ca_chain = include_str!("../../../certs/crt.pem").as_bytes();
    let cert_data = include_str!("../../../certs/dvt.crt").as_bytes();
    let private_key = include_str!("../../../certs/dvt.key").as_bytes();

    info!("=== Starting File Write Operations ===");
    info!("Files to write:");
    info!("  - firmware.bin: {} bytes", firmware_data.len());
    info!("  - crt.pem: {} bytes", ca_chain.len());
    info!("  - dvt.crt: {} bytes", cert_data.len());
    info!("  - dvt.key: {} bytes", private_key.len());

    // Write firmware at address 0x10000
    match controller
        .write_file(0x10000, "firmware.bin", firmware_data)
        .await
    {
        Ok(()) => info!("✓ Firmware written successfully"),
        Err(e) => error!("✗ Failed to write firmware: {e:?}"),
    }

    // Write CA chain at address 0x20000
    match controller.write_file(0x20000, "crt.pem", ca_chain).await {
        Ok(()) => info!("✓ CA chain written successfully"),
        Err(e) => error!("✗ Failed to write CA chain: {e:?}"),
    }

    // Write certificate at address 0x40000
    match controller.write_file(0x40000, "dvt.crt", cert_data).await {
        Ok(()) => info!("✓ Certificate written successfully"),
        Err(e) => error!("✗ Failed to write certificate: {e:?}"),
    }

    // Write private key at address 0x60000
    match controller.write_file(0x60000, "dvt.key", private_key).await {
        Ok(()) => info!("✓ Private key written successfully"),
        Err(e) => error!("✗ Failed to write private key: {e:?}"),
    }

    info!("=== Starting File Verification ===");

    // Verify firmware
    match controller.verify_file(0x10000, firmware_data).await {
        Ok(true) => info!("✓ Firmware verification: PASS"),
        Ok(false) => error!("✗ Firmware verification: FAIL - Data mismatch"),
        Err(e) => error!("✗ Firmware verification: ERROR - {e:?}"),
    }

    // Verify CA chain
    match controller.verify_file(0x20000, ca_chain).await {
        Ok(true) => info!("✓ CA chain verification: PASS"),
        Ok(false) => error!("✗ CA chain verification: FAIL - Data mismatch"),
        Err(e) => error!("✗ CA chain verification: ERROR - {e:?}"),
    }

    // Verify certificate
    match controller.verify_file(0x40000, cert_data).await {
        Ok(true) => info!("✓ Certificate verification: PASS"),
        Ok(false) => error!("✗ Certificate verification: FAIL - Data mismatch"),
        Err(e) => error!("✗ Certificate verification: ERROR - {e:?}"),
    }

    // Verify private key
    match controller.verify_file(0x60000, private_key).await {
        Ok(true) => info!("✓ Private key verification: PASS"),
        Ok(false) => error!("✗ Private key verification: FAIL - Data mismatch"),
        Err(e) => error!("✗ Private key verification: ERROR - {e:?}"),
    }

    // Test reading individual files
    info!("=== Testing File Read Operations ===");
    let mut read_buffer = [0u8; 64];

    match controller.read_file(0x10000, &mut read_buffer).await {
        Ok(bytes_read) => {
            info!("✓ Read {bytes_read} bytes from firmware");
            info!("First 32 bytes: {:02X?}", &read_buffer[..32]);
        }
        Err(e) => error!("✗ Failed to read firmware: {e:?}"),
    }

    info!("=== Flash File System Test Completed ===");

    loop {
        Timer::after(Duration::from_secs(5)).await;
        info!("System running... Flash usage: Firmware + Certificates stored");
    }
}
