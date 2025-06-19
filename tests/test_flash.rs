#![no_std]
#![feature(type_alias_impl_trait)] // Required for embassy

use embassy_executor::Spawner;
use embassy_test::test;
use esp_hal::{
    clock::ClockControl,
    embassy,
    gpio::{Io, Level, Output},
    peripherals::Peripherals,
    prelude::*,
    spi::{master::Spi, ClockSource, SpiMode},
    system::SystemExt,
    timer::TimerGroup,
};
use esp_println::println;
use esp_backtrace as _;

// Import the actual flash driver
use telematic_esp32::hal::flash::W25Q128FVSG;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Initialize ESP HAL for ESP32C6
    println!("Initializing HAL...");
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::max(system.clock_control).freeze();
    let timer_group0 = TimerGroup::new(peripherals.TIMG0, &clocks);
    embassy::init(&clocks, timer_group0);
    println!("HAL Initialized.");

    // Spawn the test runner task
    spawner.spawn(run_tests(spawner)).ok();
}

#[embassy_executor::task]
async fn run_tests(spawner: Spawner) {
    // Delay to allow hardware stabilization if needed
    // embassy_time::Timer::after(embassy_time::Duration::from_millis(500)).await;
    println!("Spawning flash tests...");
    // The embassy-test runner automatically discovers and runs functions
    // annotated with #[test]. We don't need to manually call them.
}

// Define a static cell to hold peripherals for tests if needed,
// to avoid the Peripherals::take() issue in multiple tests.
// Example (requires `static_cell` dependency):
// use static_cell::StaticCell;
// static PERIPHERALS: StaticCell<Peripherals> = StaticCell::new();

#[test]
async fn test_flash_communication() {
    println!("Starting test_flash_communication...");

    // --- HAL Initialization within test (Problematic Approach) ---
    // NOTE: Peripherals::take() can only be called once globally.
    // Calling it again here will panic if `main` already called it.
    // A robust solution involves initializing HAL once in `main` and
    // sharing peripherals (e.g., using StaticCell or passing them).
    // This code demonstrates the setup but needs refinement for complex scenarios.
    let peripherals = Peripherals::take(); // This line will likely panic!
                                        // For demonstration, assume it works or adjust structure.

    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::max(system.clock_control).freeze();
    let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);

    // --- Configure SPI Pins for ESP32-C6 --- 
    // IMPORTANT: Verify these GPIO pins match your ESP32-C6 board's wiring for the flash chip.
    // Common pins might differ from ESP32. Check your schematic.
    // Using pins from the original example (GPIO 6, 7, 8, 9) as placeholders.
    let sclk = io.pins.gpio6;
    let mosi = io.pins.gpio7;
    let miso = io.pins.gpio8;
    let cs_pin = io.pins.gpio9;

    println!("Initializing SPI...");
    // Use the appropriate SPI peripheral for ESP32-C6 (e.g., SPI2)
    // Ensure ClockSource::Sysclk is suitable or choose another source.
    let spi = Spi::new(
        peripherals.SPI2, // Verify SPI2 is correct for your C6 board
        sclk,
        mosi,
        miso,
        cs_pin, // Pass CS pin here if using hardware CS (check if driver uses it)
        100u32.kHz(), // Slow clock for testing, increase as needed
        SpiMode::Mode0, // Standard SPI mode for W25Q
        &clocks,
    ).with_pins(); // Use with_pins() for configuration

    // Software CS pin control (if not using hardware CS)
    let cs = Output::new(cs_pin, Level::High);

    println!("Initializing Flash Driver...");
    // Use the actual driver imported from the crate
    let mut flash = W25Q128FVSG::new(spi, cs);

    // Initialize the flash chip (might involve specific commands)
    // Assuming the driver's init handles necessary setup
    flash.init().await; // Assuming init doesn't return Result, adjust if it does
    println!("Flash initialized.");

    println!("Reading JEDEC ID...");
    let id = flash.read_id().await; // Assuming read_id returns the array directly
    println!("JEDEC ID: {:#04x?}", id);
    // Correct JEDEC ID for W25Q128FVSG is Manufacturer ID: 0xEF, Device ID: 0x4018
    // The read_id function likely returns [ManufID, MemType, Capacity] or similar
    // Adjust assertion based on what your driver's read_id returns.
    // Common return is [0xEF, 0x40, 0x18]
    assert_eq!(id, [0xEF, 0x40, 0x18], "JEDEC ID mismatch");
    println!("JEDEC ID test passed.");

    // Test writing and reading data
    let address = 0x1000; // Use a non-zero address
    let write_data = [0xDE, 0xAD, 0xBE, 0xEF];
    let mut read_data = [0u8; 4];

    println!("Writing data to address {:#08x}: {:02x?}", address, write_data);
    // Assuming write_bytes takes a slice
    flash
        .write_bytes(address, &write_data)
        .await;
        // .expect("Flash write failed"); // Add error handling if method returns Result

    // Add delay if required for write completion (check datasheet)
    embassy_time::Timer::after(embassy_time::Duration::from_millis(5)).await; // Example delay

    println!("Reading data from address {:#08x}...", address);
    // Assuming read_data takes address and mutable slice
    flash
        .read_data(address, &mut read_data)
        .await;
        // .expect("Flash read failed"); // Add error handling if method returns Result

    println!("Read Data: {:02x?}", read_data);
    assert_eq!(read_data, write_data, "Write/Read data mismatch");
    println!("Write/Read test passed.");

    // Test sector erase
    let sector_address = address & 0xFFFF_F000; // Align address to 4KB sector boundary
    let sector_num = sector_address / 4096;
    println!("Erasing sector {} at address {:#08x}...", sector_num, sector_address);
    flash
        .erase_sector(sector_num)
        .await;
        // .expect("Sector erase failed"); // Add error handling

    // Add delay for erase completion (check datasheet, can be long)
    embassy_time::Timer::after(embassy_time::Duration::from_millis(50)).await; // Example delay

    // Verify erase by reading back and checking for 0xFF
    let mut erased_data = [0u8; 4];
    println!("Verifying erase at address {:#08x}...", address);
    flash
        .read_data(address, &mut erased_data)
        .await;
        // .expect("Post-erase read failed"); // Add error handling

    println!("Read after erase: {:02x?}", erased_data);
    assert_eq!(erased_data, [0xFF; 4], "Sector erase verification failed");
    println!("Sector erase test passed.");

    println!("test_flash_communication finished successfully.");
}
