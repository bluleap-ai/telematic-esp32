// Import core macros needed by Serde in no_std environment
use core::marker::PhantomData;
use embassy_time::{Duration, Timer};
use esp_hal::gpio::Output;
use esp_hal::spi::master::Spi;
use esp_hal::Blocking;

#[allow(dead_code)]
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum SpiCommand {
    WriteEnable = 0x06,
    WriteDisable = 0x04,
    ReadStatusReg1 = 0x05,
    ReadStatusReg2 = 0x35,
    ReadStatusReg3 = 0x15,
    WriteStatusReg1 = 0x01,
    WriteStatusReg2 = 0x31,
    WriteStatusReg3 = 0x11,
    ReadJedecId = 0x9F,
    ReadData = 0x03,
    FastRead = 0x0B,
    PageProgram = 0x02,
    SectorErase4Kb = 0x20,
    BlockErase32Kb = 0x52,
    BlockErase64Kb = 0xD8,
    ChipErase = 0xC7,
    PowerDown = 0xB9,
    ReleasePowerDown = 0xAB,
    EnableReset = 0x66,
    ResetDevice = 0x99,
}

#[derive(Debug, Clone, Copy)]
pub enum ExFlashError {
    AddressInvalid,
    LenInvalid,
    SpiError,
    WriteEnableFailed,
    //DeviceBusy,
    //WriteProtected,
    //BufferTooLarge,
    PageBoundaryViolation,
}

// Status Register 1 bits
const BUSY_BIT: u8 = 0x01;
const WEL_BIT: u8 = 0x02;

// Timing constants (from datasheet)
const PAGE_SIZE: usize = 256;
const SECTOR_SIZE: usize = 4096;
const FLASH_CAPACITY: u32 = 16 * 1024 * 1024; // 16MB for W25Q128FV

#[derive(Debug)]
pub struct W25Q128FVSG<'d> {
    spi: Spi<'d, Blocking>,
    cs: Output<'d>,
    _mode: PhantomData<Blocking>,
}

#[allow(dead_code)]
impl<'d> W25Q128FVSG<'d> {
    pub fn new(spi: Spi<'d, Blocking>, cs: Output<'d>) -> Self {
        Self {
            spi,
            cs,
            _mode: PhantomData,
        }
    }

    pub async fn init(&mut self) -> Result<(), ExFlashError> {
        // Initialize CS pin high
        self.cs.set_high();
        Timer::after(Duration::from_millis(10)).await;

        // Release from power-down if needed
        self.release_power_down().await?;
        Timer::after(Duration::from_millis(1)).await;

        Ok(())
    }

    /// Validate address is within flash capacity
    fn validate_address(&self, address: u32) -> Result<(), ExFlashError> {
        if address >= FLASH_CAPACITY {
            return Err(ExFlashError::AddressInvalid);
        }
        Ok(())
    }

    /// Validate address range is within flash capacity
    fn validate_address_range(&self, address: u32, length: usize) -> Result<(), ExFlashError> {
        if address >= FLASH_CAPACITY {
            return Err(ExFlashError::AddressInvalid);
        }
        if address as u64 + length as u64 > FLASH_CAPACITY as u64 {
            return Err(ExFlashError::LenInvalid);
        }
        Ok(())
    }

    /// Read JEDEC ID (Manufacturer ID + Device ID)
    pub async fn read_id(&mut self) -> Result<[u8; 3], ExFlashError> {
        let mut id = [0u8; 3];

        self.cs.set_low();
        // Send command
        self.spi
            .write_bytes(&[SpiCommand::ReadJedecId as u8])
            .map_err(|_| ExFlashError::SpiError)?;
        // Read 3 bytes of ID
        self.spi
            .transfer(&mut id)
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();

        Ok(id)
    }

    /// Read status register 1
    pub async fn read_status_reg1(&mut self) -> Result<u8, ExFlashError> {
        let mut status = [0u8; 1];

        self.cs.set_low();
        self.spi
            .write_bytes(&[SpiCommand::ReadStatusReg1 as u8])
            .map_err(|_| ExFlashError::SpiError)?;
        self.spi
            .transfer(&mut status)
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();

        Ok(status[0])
    }

    /// Check if device is busy (programming/erasing)
    pub async fn is_busy(&mut self) -> Result<bool, ExFlashError> {
        let status = self.read_status_reg1().await?;
        Ok((status & BUSY_BIT) != 0)
    }

    /// Wait for device to become ready
    pub async fn wait_ready(&mut self) -> Result<(), ExFlashError> {
        while self.is_busy().await? {
            Timer::after(Duration::from_millis(1)).await;
        }
        Ok(())
    }

    /// Check if write enable latch is set
    pub async fn is_write_enabled(&mut self) -> Result<bool, ExFlashError> {
        let status = self.read_status_reg1().await?;
        Ok((status & WEL_BIT) != 0)
    }

    /// Send write enable command
    pub async fn write_enable(&mut self) -> Result<(), ExFlashError> {
        self.cs.set_low();
        self.spi
            .write_bytes(&[SpiCommand::WriteEnable as u8])
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();

        // Verify write enable was set
        Timer::after(Duration::from_micros(10)).await;

        if !self.is_write_enabled().await? {
            return Err(ExFlashError::WriteEnableFailed);
        }

        Ok(())
    }

    /// Send write disable command
    pub async fn write_disable(&mut self) -> Result<(), ExFlashError> {
        self.cs.set_low();
        self.spi
            .write_bytes(&[SpiCommand::WriteDisable as u8])
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();
        Ok(())
    }

    /// Read data from flash memory
    pub async fn read_data(&mut self, address: u32, buffer: &mut [u8]) -> Result<(), ExFlashError> {
        // Validate address range
        self.validate_address_range(address, buffer.len())?;

        if buffer.is_empty() {
            return Err(ExFlashError::LenInvalid);
        }

        let command = [
            SpiCommand::ReadData as u8,
            (address >> 16) as u8,
            (address >> 8) as u8,
            address as u8,
        ];

        self.cs.set_low();
        // Send command and address
        self.spi
            .write_bytes(&command)
            .map_err(|_| ExFlashError::SpiError)?;
        // Read data
        self.spi
            .transfer(buffer)
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();

        Ok(())
    }

    /// Fast read with dummy byte
    pub async fn fast_read(&mut self, address: u32, buffer: &mut [u8]) -> Result<(), ExFlashError> {
        // Validate address range
        self.validate_address_range(address, buffer.len())?;

        if buffer.is_empty() {
            return Err(ExFlashError::LenInvalid);
        }

        let command = [
            SpiCommand::FastRead as u8,
            (address >> 16) as u8,
            (address >> 8) as u8,
            address as u8,
            0x00, // Dummy byte
        ];

        self.cs.set_low();
        // Send command, address, and dummy byte
        self.spi
            .write_bytes(&command)
            .map_err(|_| ExFlashError::SpiError)?;
        // Read data
        self.spi
            .transfer(buffer)
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();

        Ok(())
    }

    /// Write data to flash memory (page program)
    pub async fn write_data(&mut self, address: u32, data: &[u8]) -> Result<(), ExFlashError> {
        // Validate inputs
        self.validate_address(address)?;

        if data.is_empty() {
            return Err(ExFlashError::LenInvalid);
        }

        // Ensure we don't cross page boundaries
        let page_offset = address as usize % PAGE_SIZE;
        let max_write_size = PAGE_SIZE - page_offset;

        if data.len() > max_write_size {
            return Err(ExFlashError::PageBoundaryViolation);
        }

        // Check if we would exceed flash capacity
        self.validate_address_range(address, data.len())?;

        self.wait_ready().await?;
        self.write_enable().await?;

        let command = [
            SpiCommand::PageProgram as u8,
            (address >> 16) as u8,
            (address >> 8) as u8,
            address as u8,
        ];

        self.cs.set_low();
        // Send command and address
        self.spi
            .write_bytes(&command)
            .map_err(|_| ExFlashError::SpiError)?;
        // Send data
        self.spi
            .write_bytes(data)
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();

        // Wait for programming to complete
        self.wait_ready().await?;
        Ok(())
    }

    /// Erase 4KB sector
    pub async fn erase_sector(&mut self, address: u32) -> Result<(), ExFlashError> {
        // Validate address and align to sector boundary
        self.validate_address(address)?;
        let aligned_address = address & !(SECTOR_SIZE as u32 - 1);

        self.wait_ready().await?;
        self.write_enable().await?;

        let command = [
            SpiCommand::SectorErase4Kb as u8,
            (aligned_address >> 16) as u8,
            (aligned_address >> 8) as u8,
            aligned_address as u8,
        ];

        self.cs.set_low();
        self.spi
            .write_bytes(&command)
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();

        // Wait for erase to complete
        self.wait_ready().await?;
        Ok(())
    }

    /// Erase 64KB block
    pub async fn erase_block_64kb(&mut self, address: u32) -> Result<(), ExFlashError> {
        // Validate address and align to 64KB boundary
        self.validate_address(address)?;
        let aligned_address = address & !((64 * 1024) - 1);

        self.wait_ready().await?;
        self.write_enable().await?;

        let command = [
            SpiCommand::BlockErase64Kb as u8,
            (aligned_address >> 16) as u8,
            (aligned_address >> 8) as u8,
            aligned_address as u8,
        ];

        self.cs.set_low();
        self.spi
            .write_bytes(&command)
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();

        // Wait for erase to complete (64KB takes longer - up to 2000ms)
        self.wait_ready().await?;
        Ok(())
    }

    /// Erase 32KB block
    pub async fn erase_block_32kb(&mut self, address: u32) -> Result<(), ExFlashError> {
        // Validate address and align to 32KB boundary
        self.validate_address(address)?;
        let aligned_address = address & !((32 * 1024) - 1);

        self.wait_ready().await?;
        self.write_enable().await?;

        let command = [
            SpiCommand::BlockErase32Kb as u8,
            (aligned_address >> 16) as u8,
            (aligned_address >> 8) as u8,
            aligned_address as u8,
        ];

        self.cs.set_low();
        self.spi
            .write_bytes(&command)
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();

        // Wait for erase to complete
        self.wait_ready().await?;
        Ok(())
    }

    /// Erase entire chip
    pub async fn erase_chip(&mut self) -> Result<(), ExFlashError> {
        self.wait_ready().await?;
        self.write_enable().await?;

        self.cs.set_low();
        self.spi
            .write_bytes(&[SpiCommand::ChipErase as u8])
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();

        // Wait for erase to complete (this can take a very long time - up to 200 seconds)
        self.wait_ready().await?;
        Ok(())
    }

    /// Enter power-down mode
    pub async fn power_down(&mut self) -> Result<(), ExFlashError> {
        self.cs.set_low();
        self.spi
            .write_bytes(&[SpiCommand::PowerDown as u8])
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();
        Ok(())
    }

    /// Release from power-down mode
    pub async fn release_power_down(&mut self) -> Result<(), ExFlashError> {
        self.cs.set_low();
        self.spi
            .write_bytes(&[SpiCommand::ReleasePowerDown as u8])
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();
        Ok(())
    }

    /// Software reset sequence
    pub async fn software_reset(&mut self) -> Result<(), ExFlashError> {
        // Enable reset
        self.cs.set_low();
        self.spi
            .write_bytes(&[SpiCommand::EnableReset as u8])
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();

        // Reset device
        self.cs.set_low();
        self.spi
            .write_bytes(&[SpiCommand::ResetDevice as u8])
            .map_err(|_| ExFlashError::SpiError)?;
        self.cs.set_high();

        // Wait for reset to complete
        Timer::after(Duration::from_micros(30)).await;
        Ok(())
    }

    /// Read a single byte
    pub async fn read_byte(&mut self, address: u32) -> Result<u8, ExFlashError> {
        let mut buffer = [0u8; 1];
        self.read_data(address, &mut buffer).await?;
        Ok(buffer[0])
    }

    /// Write a single byte
    pub async fn write_byte(&mut self, address: u32, data: u8) -> Result<(), ExFlashError> {
        self.write_data(address, &[data]).await
    }

    /// Get device capacity in bytes
    pub fn capacity(&self) -> u32 {
        FLASH_CAPACITY
    }

    /// Get page size
    pub fn page_size(&self) -> usize {
        PAGE_SIZE
    }

    /// Get sector size
    pub fn sector_size(&self) -> usize {
        SECTOR_SIZE
    }
}
