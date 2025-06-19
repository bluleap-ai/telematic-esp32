use core::marker::PhantomData;
use embassy_time::{Duration, Timer};
use esp_hal::gpio::Output;
#[allow(unused_imports)]
use esp_hal::spi::master::Config;
use esp_hal::spi::master::Spi;
use esp_hal::Blocking;
use heapless::Vec; // Use Blocking as the mode type

// SPI setup and constants
const WRITE_ENABLE: u8 = 0x06;
const READ_JEDEC_ID: u8 = 0x9F;
const READ_DATA: u8 = 0x03;
const PAGE_PROGRAM: u8 = 0x02;
const SECTOR_ERASE_4KB: u8 = 0x20;
const BLOCK_ERASE_64KB: u8 = 0xD8;
const CHIP_ERASE: u8 = 0xC7;
const DUMMY_BYTE: u8 = 0x00;

pub struct W25Q128FVSG<'d> {
    spi: Spi<'d, Blocking>, // Use Blocking for the mode
    cs: Output<'d>,
    _mode: PhantomData<Blocking>,
}

impl<'d> W25Q128FVSG<'d> {
    pub fn new(spi: Spi<'d, Blocking>, cs: Output<'d>) -> Self {
        Self {
            spi,
            cs,
            _mode: PhantomData,
        }
    }

    pub async fn init(&mut self) {
        self.cs.set_low();
        Timer::after(Duration::from_millis(1)).await;
        self.cs.set_high();
    }

    pub async fn read_id(&mut self) -> [u8; 3] {
        let mut id = [0; 3];
        self.cs.set_low();
        self.spi
            .transfer(&mut id) // Correct usage of transfer method
            .unwrap();
        self.cs.set_high();
        id
    }

    pub async fn read_data(&mut self, address: u32, buffer: &mut [u8]) {
        let mut command = [
            READ_DATA,
            (address >> 16) as u8,
            (address >> 8) as u8,
            address as u8,
        ];
        self.cs.set_low();
        self.spi.transfer(&mut command).unwrap(); // Transfer command to SPI
        self.spi.transfer(buffer).unwrap(); // Transfer data buffer
        self.cs.set_high();
    }

    pub async fn write_enable(&mut self) {
        self.cs.set_low();
        self.spi
            .transfer(&mut [WRITE_ENABLE]) // Transfer the write enable command
            .unwrap();
        self.cs.set_high();
    }

    pub async fn write_data(&mut self, address: u32, data: &mut [u8]) {
        self.write_enable().await;
        let mut command = [
            PAGE_PROGRAM,
            (address >> 16) as u8,
            (address >> 8) as u8,
            address as u8,
        ];
        self.cs.set_low();
        self.spi.transfer(&mut command).unwrap(); // Transfer write command
        self.spi.transfer(data).unwrap(); // Transfer the data to write
        self.cs.set_high();
    }

    pub async fn erase_sector(&mut self, sector_num: u32) {
        let sector_addr = sector_num * 4096; // 4KB sector size
        self.write_enable().await;
        self.cs.set_low();
        self.spi
            .transfer(&mut [
                SECTOR_ERASE_4KB,
                (sector_addr >> 16) as u8,
                (sector_addr >> 8) as u8,
                sector_addr as u8,
            ])
            .unwrap();
        self.cs.set_high();
    }

    pub async fn erase_block(&mut self, block_num: u32) {
        let block_addr = block_num * 65536; // 64KB block size
        self.write_enable().await;
        self.cs.set_low();
        self.spi
            .transfer(&mut [
                BLOCK_ERASE_64KB,
                (block_addr >> 16) as u8,
                (block_addr >> 8) as u8,
                block_addr as u8,
            ])
            .unwrap();
        self.cs.set_high();
    }

    pub async fn erase_chip(&mut self) {
        self.write_enable().await;
        self.cs.set_low();
        self.spi.transfer(&mut [CHIP_ERASE]).unwrap(); // Send chip erase command
        self.cs.set_high();
    }

    pub async fn read_byte(&mut self, address: u32) {
        self.cs.set_low();
        self.spi
            .transfer(&mut [
                READ_DATA,
                (address >> 16) as u8,
                (address >> 8) as u8,
                address as u8,
            ])
            .unwrap();
        self.spi.transfer(&mut [DUMMY_BYTE]).unwrap(); // Receive the byte
        self.cs.set_high();
    }

    pub async fn write_byte(&mut self, address: u32, data: u8) {
        self.write_enable().await;
        self.cs.set_low();
        self.spi
            .transfer(&mut [
                PAGE_PROGRAM,
                (address >> 16) as u8,
                (address >> 8) as u8,
                address as u8,
                data,
            ])
            .unwrap();
        self.cs.set_high();
    }

    pub async fn write_bytes(&mut self, address: u32, data: &[u8]) {
        let mut command: Vec<u8, 4> = Vec::new();
        command.push(PAGE_PROGRAM).unwrap();
        command.push((address >> 16) as u8).unwrap();
        command.push((address >> 8) as u8).unwrap();
        command.push(address as u8).unwrap();
        let _ = command.extend_from_slice(data);
        self.cs.set_low();
        self.spi.transfer(&mut command).unwrap(); // Transfer full command with data
        self.cs.set_high();
    }
}
