// Flash File System Controller for W25Q128FVSG
// ---------------------------------------------------
// * Purpose: Provides high-level API to erase, write, read, list, and verify
//   files within defined regions on external SPI Flash chip W25Q128FVSG
//   using the `embassy` async runtime on ESP32.
// * All functions use Rust async idioms (`async/await`) in combination
//   with a custom `ex_flash` driver.
//
// Key Terminology:
// * **Region**: Logical storage area (Firmware / Certstore / UserData) mapped
//   to specific physical addresses on flash.
// * **Page**: Smallest write unit (256 bytes).
// * **Sector**: Smallest erase unit (4096 bytes).
// * **Block**: Larger erase unit (64 KiB) – faster than sector.
//
// File format inside `Certstore`:
// ```text
// | name_len | name (UTF-8) | data_len (LE u32) | data ... | crc32 (LE u32) | (pad with 0xFF) |
// ```
// * Each file entry is page-aligned for simpler handling.
// * A 0xFF byte at the start indicates an unused entry.
// * CRC32 is computed over `data_len` (4 bytes, LE) and `data`.

// pub mod ex_flash;
#[allow(unused_imports)]
use crate::mem::ex_flash::{ExFlashError, W25Q128FVSG};
// use embassy_executor::Spawner;
use embassy_time::{with_timeout, Duration};
use esp_backtrace as _;
// use esp_hal::time::Rate;
use heapless::{/*String,*/ Vec};
use log::{error, info, warn};

// FsError: High-level errors returned by the FlashController.
#[derive(Debug)]
#[allow(dead_code)]
pub enum FsError {
    FlashError(ExFlashError),
    InvalidAddress,
    FileTooLarge,
    VerificationFailed,
    FilenameTooLong,
    FileNotFound,
    Timeout,
}

impl From<ExFlashError> for FsError {
    fn from(err: ExFlashError) -> Self {
        FsError::FlashError(err)
    }
}

// FlashRegion: Logical to physical address mapping for different data sections.
#[derive(Copy, Clone, Debug, Eq, PartialEq)] // add Copy + Clone
#[allow(dead_code)]
pub enum FlashRegion {
    Firmware = 0x000000,
    Certstore = 0x300000,
    UserData = 0x340000,
}

const MAX_FILES: usize = 16;
const MAX_NAME: usize = 32;

// DirEntry: Structure describing a file entry inside the flash system.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct DirEntry {
    pub name_len: u8,
    pub name: heapless::String<MAX_NAME>,
    pub offset: u32,
    pub len: u32,
}

pub struct FileEntry {
    pub region: FlashRegion,
    pub name: &'static str,
    pub data: &'static [u8],
    pub is_fw: bool,
}
// FlashController: Main interface for managing file system operations on flash.

pub struct FlashController<'a> {
    flash: &'a mut W25Q128FVSG<'a>,
    last_offset: u32, // Track last used offset for Certstore
}

pub struct RegionInfo {
    pub start: u32,
    pub size: u32,
}

// FlashRegion: Logical to physical address mapping for different data sections.
impl FlashRegion {
    pub const fn info(&self) -> RegionInfo {
        match self {
            FlashRegion::Firmware => RegionInfo {
                start: 0x000000,
                size: 0x300000, // 3 MiB
            },
            FlashRegion::Certstore => RegionInfo {
                start: 0x300000,
                size: 0x040000, // 256 KiB
            },
            FlashRegion::UserData => RegionInfo {
                start: 0x340000,
                size: 0x0C0000, // 768 KiB
            },
        }
    }
}

#[allow(dead_code)]
impl<'a> FlashController<'a> {
    // - `new`: Create a new flash controller with initial offset set to 0.
    pub fn new(flash: &'a mut W25Q128FVSG<'a>) -> Self {
        Self {
            flash,
            last_offset: 0, // Initialize to start of flash region
        }
    }

    // - `erase_region`: Erase an entire region (Firmware, Certstore, or UserData).
    pub async fn erase_region(&mut self, region: FlashRegion) -> Result<(), FsError> {
        let info = region.info();
        self.erase_range(info.start, info.start + info.size)
            .await
            .map_err(FsError::FlashError)?;
        self.last_offset = 0; // Reset offset after full erase
        info!("Erased region {region:?}");
        Ok(())
    }

    // - `erase_range`: Erase a specified address range using sector or block erase.
    async fn erase_range(&mut self, start: u32, end: u32) -> Result<(), ExFlashError> {
        if start >= end || end > self.flash.capacity() {
            return Err(ExFlashError::AddressInvalid);
        }

        let block_size = 64 * 1024; // 64 KiB
        let sector_size = self.flash.sector_size() as u32; // 4 KiB

        if end - start >= block_size {
            let start_block = start & !(block_size - 1);
            let end_block = (end + block_size - 1) & !(block_size - 1);
            info!("Erasing 64KiB blocks from 0x{start_block:08X} to 0x{end_block:08X}");
            for addr in (start_block..end_block).step_by(block_size as usize) {
                let mut buf = [0u8; 64];
                self.flash.read_data(addr, &mut buf)?;
                if buf.iter().all(|&b| b == 0xFF) {
                    info!("Block at 0x{addr:08X} already erased, skipping");
                    continue;
                }
                info!("Erasing 64KiB block at 0x{addr:08X}");
                match with_timeout(
                    Duration::from_millis(2500),
                    self.flash.erase_block_64kb(addr),
                )
                .await
                {
                    Ok(Ok(())) => info!("✓ Block erased at 0x{addr:08X}"),
                    Ok(Err(e)) => {
                        error!("✗ Failed to erase block at 0x{addr:08X}: {e:?}");
                        return Err(e);
                    }
                    Err(_) => {
                        error!("✗ Timeout erasing block at 0x{addr:08X}");
                        return Err(ExFlashError::Timeout);
                    }
                }
            }
        } else {
            let start_sector = start & !(sector_size - 1);
            let end_sector = (end + sector_size - 1) & !(sector_size - 1);
            info!("Erasing sectors from 0x{start_sector:08X} to 0x{end_sector:08X}");
            for addr in (start_sector..end_sector).step_by(sector_size as usize) {
                let mut buf = [0u8; 64];
                self.flash.read_data(addr, &mut buf)?;
                if buf.iter().all(|&b| b == 0xFF) {
                    info!("Sector at 0x{addr:08X} already erased, skipping");
                    continue;
                }
                info!("Erasing sector at 0x{addr:08X}");
                match with_timeout(Duration::from_millis(500), self.flash.erase_sector(addr)).await
                {
                    Ok(Ok(())) => info!("✓ Sector erased at 0x{addr:08X}"),
                    Ok(Err(e)) => {
                        error!("✗ Failed to erase sector at 0x{addr:08X}: {e:?}");
                        return Err(e);
                    }
                    Err(_) => {
                        error!("✗ Timeout erasing sector at 0x{addr:08X}");
                        return Err(ExFlashError::Timeout);
                    }
                }
            }
        }
        Ok(())
    }

    // - `find_free_offset`: Scan for the next available (unused) entry offset within a region.
    // 1. Start at self.last_offset (where the previous write ended).
    // 2. Loop while cur < region.size:
    //    a. addr = region.start + cur
    //    b. Read one byte → name_len
    //       • 0xFF → this page is untouched ⇒ free space found
    //                 ↳ return cur aligned up to page boundary.
    //    c. Sanity-check: if name_len > MAX_NAME ⇒ corrupted entry ⇒ abort.
    //    d. Read 4-byte data_len located at size_addr = addr + 1 + name_len
    //    e. Compute total entry length: entry_size = 1 + name_len + 4 + data_len + 4 (CRC32)
    //    f. Advance cur by entry_size, then round it up to the next page
    // 3. If the loop ends with no 0xFF byte encountered, the region is full ⇒ return None.
    async fn find_free_offset(&mut self, region: &FlashRegion) -> Option<u32> {
        let info = region.info();
        let page_size = self.flash.page_size() as u32;
        let mut cur = self.last_offset;

        while cur < info.size {
            let addr = info.start + cur;

            let mut len_buf = [0u8; 1];
            if let Err(e) = self.flash.read_data(addr, &mut len_buf) {
                error!("Failed to read filename length at address 0x{addr:08X}: {e:?}");
                return None;
            }

            let name_len = len_buf[0] as u32;
            info!("find_free_offset: addr 0x{addr:08X}, name_len {name_len}");
            if name_len == 0xFF {
                let aligned_offset = (cur + page_size - 1) & !(page_size - 1);
                if aligned_offset + 1 > info.size {
                    error!("No free space left in region {region:?}");
                    return None;
                }
                info!("Found free offset 0x{aligned_offset:X}");
                return Some(aligned_offset);
            }

            if name_len > MAX_NAME as u32 {
                error!("Invalid name length {name_len} at address 0x{addr:08X}");
                return None;
            }

            let mut size_buf = [0u8; 4];
            let size_addr = addr + 1 + name_len;
            if let Err(e) = self.flash.read_data(size_addr, &mut size_buf) {
                error!("Failed to read data length at address 0x{size_addr:08X}: {e:?}");
                return None;
            }
            let data_len = u32::from_le_bytes(size_buf);
            info!("Found entry at 0x{addr:08X}, data_len {data_len}");

            cur += 1 + name_len + 4 + data_len + 4; // Updated to include 4-byte CRC32
            cur = (cur + page_size - 1) & !(page_size - 1);
        }
        error!("No free space found in region {region:?}");
        None
    }

    // - `write_file`: Write a file into the flash region.
    // 1. Validate filename length and input data length
    // 2. Compute CRC32 over data_len and data
    // 3. Compute total file size: filename + 4-byte length + data + 4-byte CRC
    // 4. Get flash region information and calculate aligned offset
    // 5. Prepare in-memory file buffer with:
    //    - [filename length (1 byte)]
    //    - [filename bytes]
    //    - [data length (4 bytes, little-endian)]
    //    - [payload data]
    //    - [CRC32 of data_len + data (4 bytes)]
    // 6. Scan current flash region to preserve existing valid entries
    // 7. Check if any page about to be written is not fully erased
    // 8. Pad the file buffer to align with full page size
    // 9. Write the padded buffer to flash, page-by-page
    // 10. Update `last_offset` tracker
    pub async fn write_file(
        &mut self,
        region: FlashRegion,
        filename: &str,
        data: &[u8],
    ) -> Result<(), FsError> {
        if filename.len() > MAX_NAME {
            error!("Filename '{filename}' exceeds maximum length of {MAX_NAME} characters");
            return Err(FsError::FilenameTooLong);
        }

        if data.is_empty() {
            warn!("Writing empty file '{filename}'");
            return Ok(());
        }

        let info = region.info();
        // Compute CRC32 over data_len and data
        let data_len = data.len() as u32;
        let mut crc_input = Vec::<u8, 4096>::new();

        if crc_input
            .extend_from_slice(&data_len.to_le_bytes())
            .is_err()
        {
            error!("CRC input buffer overflow for data length");
            return Err(FsError::FileTooLarge);
        }
        if crc_input.extend_from_slice(data).is_err() {
            error!("CRC input buffer overflow for file data");
            return Err(FsError::FileTooLarge);
        }

        let crc = crc32fast::hash(&crc_input); // Updated to include data_len

        let file_size = 1 + filename.len() + 4 + data.len() + 4;
        if file_size > 4096 {
            error!("File '{filename}' too large for buffer: {file_size} bytes");
            return Err(FsError::FileTooLarge);
        }

        let page_size = self.flash.page_size() as u32; // 256 bytes
        let sector_size = self.flash.sector_size() as u32; // 4096 bytes

        // Find free offset, aligned to page boundary
        let offset = self
            .find_free_offset(&region)
            .await
            .ok_or(FsError::InvalidAddress)?;
        let abs_address = (info.start + offset + page_size - 1) & !(page_size - 1);

        if abs_address + file_size as u32 > info.start + info.size {
            error!(
                "Not enough space in region {:?} to write file '{}': {} bytes required, {} bytes available",
                region, filename, file_size, info.size - (abs_address - info.start)
            );
            return Err(FsError::FileTooLarge);
        }

        // Prepare the file buffer
        let mut buf = Vec::<u8, 4096>::new();
        if buf.push(filename.len() as u8).is_err() {
            error!("Buffer overflow when adding filename length");
            return Err(FsError::FileTooLarge);
        }
        if buf.extend_from_slice(filename.as_bytes()).is_err() {
            error!("Buffer overflow when adding filename");
            return Err(FsError::FileTooLarge);
        }
        if buf.extend_from_slice(&data_len.to_le_bytes()).is_err() {
            error!("Buffer overflow when adding data length");
            return Err(FsError::FileTooLarge);
        }
        if buf.extend_from_slice(data).is_err() {
            error!("Buffer overflow when adding file data");
            return Err(FsError::FileTooLarge);
        }
        if buf.extend_from_slice(&crc.to_le_bytes()).is_err() {
            error!("Buffer overflow when adding CRC");
            return Err(FsError::FileTooLarge);
        }

        info!(
            "Prepared file buffer, first 16 bytes: {:?}",
            &buf[..core::cmp::min(16, buf.len())]
        );

        // Calculate affected pages and sector
        let start_page = abs_address;
        let end_page = ((abs_address + file_size as u32 - 1) & !(page_size - 1)) + page_size;
        let start_sector = abs_address & !(sector_size - 1);

        // Read existing pages that contain valid data before the new file
        let mut preserved_data = Vec::<u8, 8192>::new();
        let mut cur = 0;
        info!("Scanning for existing files up to offset 0x{offset:08X}");
        while cur < offset {
            let addr = info.start + cur;
            let mut len_buf = [0u8; 1];
            if let Err(e) = self.flash.read_data(addr, &mut len_buf) {
                error!("Failed to read name length at 0x{addr:08X}: {e:?}");
                return Err(FsError::FlashError(e));
            }
            let name_len = len_buf[0] as u32;
            info!("At 0x{addr:08X}, name_len: {name_len}");
            if name_len == 0xFF {
                info!("No more valid entries at 0x{addr:08X}");
                break;
            }
            if name_len > MAX_NAME as u32 {
                error!("Invalid name length {name_len} at 0x{addr:08X}");
                return Err(FsError::InvalidAddress);
            }

            let mut name_buf = [0u8; MAX_NAME];
            if let Err(e) = self
                .flash
                .read_data(addr + 1, &mut name_buf[..name_len as usize])
            {
                error!("Failed to read name at 0x{:08X}: {:?}", addr + 1, e);
                return Err(FsError::FlashError(e));
            }
            let name = core::str::from_utf8(&name_buf[..name_len as usize]).unwrap_or("<invalid>");
            info!("Found file '{name}' at 0x{addr:08X}");

            let mut size_buf = [0u8; 4];
            let size_addr = addr + 1 + name_len;
            if let Err(e) = self.flash.read_data(size_addr, &mut size_buf) {
                error!("Failed to read data length at 0x{size_addr:08X}: {e:?}");
                return Err(FsError::FlashError(e));
            }
            let data_len = u32::from_le_bytes(size_buf);
            info!("Data length: {data_len} bytes");

            let entry_size = 1 + name_len + 4 + data_len + 4; // Include CRC32
            let entry_end = addr + entry_size;
            let aligned_end = (entry_end + page_size - 1) & !(page_size - 1);

            let mut entry_data = Vec::<u8, 4096>::new();
            entry_data
                .extend_from_slice(&[0xFFu8; 4096][..(aligned_end - addr) as usize])
                .map_err(|_| {
                    error!("Entry data buffer overflow at 0x{addr:08X}");
                    FsError::FileTooLarge
                })?;
            if let Err(e) = self
                .flash
                .read_data(addr, &mut entry_data[..(aligned_end - addr) as usize])
            {
                error!("Failed to read entry at 0x{addr:08X}: {e:?}");
                return Err(FsError::FlashError(e));
            }
            info!(
                "Read {} bytes for entry at 0x{:08X}: {:?}",
                (aligned_end - addr),
                addr,
                &entry_data[..16]
            );
            preserved_data.extend_from_slice(&entry_data).map_err(|_| {
                error!("Preserved data buffer overflow at 0x{addr:08X}");
                FsError::FileTooLarge
            })?;

            cur += entry_size;
            cur = (cur + page_size - 1) & !(page_size - 1);
        }

        // Check if the pages to be written contain non-erased data
        let mut needs_erase = false;
        for page_addr in (start_page..end_page).step_by(page_size as usize) {
            let mut page_buf = [0xFFu8; 256];
            if let Err(e) = self.flash.read_data(page_addr, &mut page_buf) {
                error!("Failed to read page at 0x{page_addr:08X}: {e:?}");
                return Err(FsError::FlashError(e));
            }
            if !page_buf.iter().all(|&b| b == 0xFF) {
                needs_erase = true;
                break;
            }
        }

        if needs_erase {
            info!("Erasing sector at 0x{start_sector:08X}");
            self.flash
                .erase_sector(start_sector)
                .await
                .map_err(FsError::FlashError)?;

            // Rewrite preserved data
            if !preserved_data.is_empty() {
                info!(
                    "Rewriting preserved {} bytes to 0x{:08X}",
                    preserved_data.len(),
                    start_sector
                );
                for page_addr in (start_sector..start_page).step_by(page_size as usize) {
                    let page_offset = (page_addr - start_sector) as usize;
                    let chunk_size =
                        core::cmp::min(page_size as usize, preserved_data.len() - page_offset);
                    if chunk_size > 0 {
                        info!("Rewriting {chunk_size} bytes to 0x{page_addr:08X}");
                        self.flash
                            .write_data(
                                page_addr,
                                &preserved_data[page_offset..page_offset + chunk_size],
                            )
                            .await
                            .map_err(|e| {
                                error!(
                                    "Failed to rewrite preserved data to 0x{page_addr:08X}: {e:?}"
                                );
                                FsError::FlashError(e)
                            })?;

                        let mut verify_buf = [0xFFu8; 256];
                        if let Err(e) = self
                            .flash
                            .read_data(page_addr, &mut verify_buf[..chunk_size])
                        {
                            error!("Failed to verify preserved data at 0x{page_addr:08X}: {e:?}");
                            return Err(FsError::FlashError(e));
                        }
                        if verify_buf[..chunk_size]
                            != preserved_data[page_offset..page_offset + chunk_size]
                        {
                            error!(
                                "Preserved data verification failed at 0x{:08X}, expected: {:?}",
                                page_addr,
                                &preserved_data[page_offset..page_offset + chunk_size]
                            );
                            return Err(FsError::FlashError(ExFlashError::WriteFailed));
                        }
                        info!("Verified preserved data at 0x{page_addr:08X}");
                    }
                }
            }
        } else {
            info!("No erase needed for pages 0x{start_page:08X} to 0x{end_page:08X}");
        }

        // Pad the file buffer to a full page
        let mut padded_buf = Vec::<u8, 4096>::new();
        padded_buf
            .extend_from_slice(&buf)
            .map_err(|_| FsError::FileTooLarge)?;
        while (padded_buf.len() as u32) < (end_page - start_page) {
            padded_buf.push(0xFF).map_err(|_| FsError::FileTooLarge)?;
        }
        info!(
            "Padded file buffer to {} bytes for writing from 0x{:08X} to 0x{:08X}",
            padded_buf.len(),
            start_page,
            end_page
        );

        // Write the new file (full pages)
        for page_addr in (start_page..end_page).step_by(page_size as usize) {
            let page_offset = (page_addr - start_page) as usize;
            let chunk_size = core::cmp::min(page_size as usize, padded_buf.len() - page_offset);
            info!("Writing {chunk_size} bytes to 0x{page_addr:08X}");
            self.flash
                .write_data(
                    page_addr,
                    &padded_buf[page_offset..page_offset + chunk_size],
                )
                .await
                .map_err(|e| {
                    error!("Failed to write to 0x{page_addr:08X}: {e:?}");
                    FsError::FlashError(e)
                })?;

            let mut verify_buf = [0xFFu8; 256];
            if let Err(e) = self
                .flash
                .read_data(page_addr, &mut verify_buf[..chunk_size])
            {
                error!("Failed to verify read at 0x{page_addr:08X}: {e:?}");
                return Err(FsError::FlashError(e));
            }
            if verify_buf[..chunk_size] != padded_buf[page_offset..page_offset + chunk_size] {
                error!(
                    "Write verification failed at 0x{:08X}, expected: {:?}",
                    page_addr,
                    &padded_buf[page_offset..page_offset + chunk_size]
                );
                return Err(FsError::FlashError(ExFlashError::WriteFailed));
            }
            info!("Verified write at 0x{page_addr:08X}");
        }

        // Update last_offset, aligning to next page boundary
        self.last_offset = (abs_address - info.start) + file_size as u32;
        self.last_offset = (self.last_offset + page_size - 1) & !(page_size - 1);
        info!("Updated last_offset to 0x{:08X}", self.last_offset);

        info!(
            "✓ File '{}' written with {} bytes to {:?} at 0x{:08X}",
            filename,
            data.len(),
            region,
            abs_address
        );
        Ok(())
    }

    // - `write_firmware`: Write firmware at fixed offset with CRC32.
    // 1. Validate firmware size
    // 2. Prepare metadata: [filename length (1 byte)] + [filename] + [firmware length (4 bytes)] + [CRC32 (4 bytes)]
    // 3. Erase the flash region
    // 4. Write metadata and firmware content
    pub async fn write_firmware(
        &mut self,
        region: FlashRegion,
        filename: &str,
        data: &[u8],
    ) -> Result<(), FsError> {
        let address = region as u32;
        if address + data.len() as u32 > self.flash.capacity() {
            error!("Firmware data exceeds flash capacity at address 0x{address:08X}");
            return Err(FsError::FileTooLarge);
        }

        // Compute CRC32 over data_len and data
        let data_len = data.len() as u32;
        let mut crc_input = Vec::<u8, 4096>::new();
        crc_input
            .extend_from_slice(&data_len.to_le_bytes())
            .unwrap();
        let _ = crc_input.extend_from_slice(data);
        let crc = crc32fast::hash(&crc_input);

        let mut metadata = [0u8; 272];
        let n = 1 + filename.len();
        metadata[0] = filename.len() as u8;
        metadata[1..1 + filename.len()].copy_from_slice(filename.as_bytes());
        metadata[n..n + 4].copy_from_slice(&data_len.to_le_bytes());
        metadata[n + 4..n + 8].copy_from_slice(&crc.to_le_bytes()); // Added CRC32

        self.erase_range(address, address + metadata.len() as u32)
            .await?;
        self.flash.write_data(address, &metadata[..n + 8]).await?; // Updated to include CRC32

        let header_len = (n + 8) as u32; // Updated to account for CRC32
        let mut addr = address + header_len;
        let mut offset = 0;
        let page = self.flash.page_size() as u32; // 256

        while offset < data.len() {
            let page_off = (addr & (page - 1)) as usize;
            let space_in_page = (page as usize) - page_off;
            let chunk = core::cmp::min(space_in_page, data.len() - offset);

            self.flash
                .write_data(addr, &data[offset..offset + chunk])
                .await
                .map_err(FsError::FlashError)?;

            addr += chunk as u32;
            offset += chunk;
        }

        info!(
            "✓ Firmware '{}' written with {} bytes to 0x{:08X}",
            filename,
            data.len(),
            address
        );
        Ok(())
    }

    // - `read_file`: Read a file’s content by filename with CRC32 verification.
    // 1. Search for the file
    // 2. Read data and CRC32
    // 3. Verify CRC32 before returning data
    pub async fn read_file(
        &mut self,
        region: FlashRegion,
        filename: &str,
        buffer: &mut [u8],
    ) -> Result<usize, FsError> {
        let info = region.info();
        let page_size = self.flash.page_size() as u32;
        let mut cur = 0;

        info!("Searching for file '{filename}' in region {region:?}");
        while cur < info.size {
            let addr = info.start + cur;
            info!("Checking entry at address 0x{addr:08X}");

            let mut len = [0u8; 1];
            if let Err(e) = self.flash.read_data(addr, &mut len) {
                error!("Failed to read name length at 0x{addr:08X}: {e:?}");
                return Err(FsError::FlashError(e));
            }
            let name_len = len[0] as usize;
            info!("Name length: {name_len}");

            if name_len == 0xFF {
                info!("End of directory reached at 0x{addr:08X}");
                break;
            }

            if name_len > MAX_NAME {
                error!("Invalid name length {name_len} at 0x{addr:08X}");
                return Err(FsError::InvalidAddress);
            }

            let mut name_buf = [0u8; MAX_NAME];
            if let Err(e) = self.flash.read_data(addr + 1, &mut name_buf[..name_len]) {
                error!("Failed to read filename at 0x{:08X}: {:?}", addr + 1, e);
                return Err(FsError::FlashError(e));
            }
            let name = core::str::from_utf8(&name_buf[..name_len]).unwrap_or("");
            info!("Found file '{name}'");

            let mut size_buf = [0u8; 4];
            let size_addr = addr + 1 + name_len as u32;
            if let Err(e) = self.flash.read_data(size_addr, &mut size_buf) {
                error!("Failed to read data length at 0x{size_addr:08X}: {e:?}");
                return Err(FsError::FlashError(e));
            }
            let data_len = u32::from_le_bytes(size_buf);
            info!("Data length: {data_len} bytes");

            if name == filename {
                if data_len as usize > buffer.len() {
                    error!(
                        "Buffer too small: {} bytes needed, {} available",
                        data_len,
                        buffer.len()
                    );
                    return Err(FsError::FileTooLarge);
                }
                let data_addr = addr + 1 + name_len as u32 + 4;
                if let Err(e) = self
                    .flash
                    .read_data(data_addr, &mut buffer[..data_len as usize])
                {
                    error!("Failed to read data at 0x{data_addr:08X}: {e:?}");
                    return Err(FsError::FlashError(e));
                }
                // Read and verify CRC32
                let crc_addr = data_addr + data_len;
                let mut crc_buf = [0u8; 4];
                if let Err(e) = self.flash.read_data(crc_addr, &mut crc_buf) {
                    error!("Failed to read CRC at 0x{crc_addr:08X}: {e:?}");
                    return Err(FsError::FlashError(e));
                }
                let stored_crc = u32::from_le_bytes(crc_buf);
                let mut crc_input = Vec::<u8, 4096>::new();
                if crc_input
                    .extend_from_slice(&data_len.to_le_bytes())
                    .is_err()
                {
                    error!("CRC input buffer overflow when adding data length");
                    return Err(FsError::FileTooLarge);
                }
                if crc_input
                    .extend_from_slice(&buffer[..data_len as usize])
                    .is_err()
                {
                    error!("CRC input buffer overflow when adding file data");
                    return Err(FsError::FileTooLarge);
                }
                let computed_crc = crc32fast::hash(&crc_input);
                if stored_crc != computed_crc {
                    error!(
                        "CRC mismatch for file '{}': expected 0x{:08X}, got 0x{:08X}",
                        filename, stored_crc, computed_crc
                    );
                    return Err(FsError::VerificationFailed);
                }
                info!("✓ Read file '{filename}' ({data_len} B) from 0x{addr:08X}, CRC verified");
                return Ok(data_len as usize);
            }

            cur += 1 + name_len as u32 + 4 + data_len + 4; // Updated to include CRC32
            cur = (cur + page_size - 1) & !(page_size - 1);
        }

        error!("File '{filename}' not found in region {region:?}");
        Err(FsError::FileNotFound)
    }

    // - `verify_file`: Verify a file using CRC32.
    // 1. Read file data and stored CRC32
    // 2. Compute CRC32 over data_len and data
    // 3. Compare with expected CRC32
    pub async fn verify_file(
        &mut self,
        base: FlashRegion,
        filename: &str,
        expected_data: &[u8],
    ) -> bool {
        let mut buf = [0u8; 4096];
        match self.read_file(base, filename, &mut buf).await {
            Ok(n) => {
                if n != expected_data.len() {
                    error!(
                        "File '{}' size mismatch: expected {}, got {}",
                        filename,
                        expected_data.len(),
                        n
                    );
                    return false;
                }
                // Compute expected CRC32
                let data_len = expected_data.len() as u32;
                let mut crc_input = Vec::<u8, 4096>::new();
                if crc_input
                    .extend_from_slice(&data_len.to_le_bytes())
                    .is_err()
                {
                    error!("CRC input buffer overflow for expected data length");
                    return false;
                }
                if crc_input.extend_from_slice(expected_data).is_err() {
                    error!("CRC input buffer overflow for expected data");
                    return false;
                }
                let expected_crc = crc32fast::hash(&crc_input);
                // Recompute CRC32 for read data
                let mut crc_input = Vec::<u8, 4096>::new();
                if crc_input
                    .extend_from_slice(&data_len.to_le_bytes())
                    .is_err()
                {
                    error!("CRC input buffer overflow for read data length");
                    return false;
                }
                if crc_input.extend_from_slice(&buf[..n]).is_err() {
                    error!("CRC input buffer overflow for read data");
                    return false;
                }
                let computed_crc = crc32fast::hash(&crc_input);
                if computed_crc == expected_crc {
                    info!(
                        "✓ File '{filename}' verification passed (CRC32: 0x{:08X})",
                        computed_crc
                    );
                    true
                } else {
                    error!(
                        "✗ File '{filename}' verification failed - CRC mismatch: expected 0x{:08X}, got 0x{:08X}",
                        expected_crc, computed_crc
                    );
                    false
                }
            }
            Err(e) => {
                error!("✗ Failed to read file '{filename}': {e:?}");
                false
            }
        }
    }

    // - `list_files`: Enumerate all valid file entries within a region.
    pub async fn list_files(
        &mut self,
        region: &FlashRegion,
    ) -> Result<Vec<DirEntry, MAX_FILES>, FsError> {
        let mut files = Vec::<DirEntry, MAX_FILES>::new();
        let info = region.info();
        let mut cur = 0;
        let page_size = self.flash.page_size() as u32;

        info!("Listing files in region {region:?}");

        while cur < info.size {
            let addr = info.start + cur;

            let mut len_buf = [0u8; 1];
            self.flash
                .read_data(addr, &mut len_buf)
                .map_err(FsError::FlashError)?;
            let name_len = len_buf[0] as usize;

            if name_len == 0xFF {
                info!("End of directory at 0x{addr:08X}");
                break;
            }

            if name_len == 0 || name_len > MAX_NAME {
                error!("Invalid name_len {name_len} at 0x{addr:08X}");
                return Err(FsError::InvalidAddress);
            }

            let mut name_buf = [0u8; MAX_NAME];
            self.flash
                .read_data(addr + 1, &mut name_buf[..name_len])
                .map_err(FsError::FlashError)?;
            let name_str = core::str::from_utf8(&name_buf[..name_len]).unwrap_or("<invalid>");
            let mut name = heapless::String::<MAX_NAME>::new();
            if name.push_str(name_str).is_err() {
                error!("Failed to create filename string for file at 0x{addr:08X}");
                return Err(FsError::InvalidAddress);
            }

            let mut size_buf = [0u8; 4];
            let size_addr = addr + 1 + name_len as u32;
            self.flash
                .read_data(size_addr, &mut size_buf)
                .map_err(FsError::FlashError)?;
            let data_len = u32::from_le_bytes(size_buf);

            let entry = DirEntry {
                name_len: name_len as u8,
                name,
                offset: addr,
                len: data_len,
            };
            files.push(entry).map_err(|_| FsError::FileTooLarge)?;

            cur += 1 + name_len as u32 + 4 + data_len + 4; // Updated to include CRC32
            cur = (cur + page_size - 1) & !(page_size - 1);
        }

        Ok(files)
    }

    // - `print_directory`: Print a formatted directory listing for a region.
    // 1. Fetch all file entries using list_files
    // 2. Format and log each entry with name, size, and offset
    // 3. Include a summary of total files and space used
    pub async fn print_directory(&mut self, region: FlashRegion) -> Result<(), FsError> {
        let files = self.list_files(&region).await?;
        let info = region.info();
        info!("Directory of {:?}", region);
        info!("--------------------------------");
        info!("{:<32} {:>10} {:>12}", "Name", "Size (B)", "Offset");
        let mut total_size = 0;
        for entry in files.iter() {
            info!(
                "{:<32} {:>10} 0x{:08X}",
                entry.name.as_str(),
                entry.len,
                entry.offset
            );
            total_size += entry.len;
        }
        let file_count = files.len();
        if file_count == 0 {
            info!("No files found in {:?}", region);
        } else {
            info!("--------------------------------");
            info!("Total: {} file(s), {} bytes used", file_count, total_size);
        }
        Ok(())
    }
    // - `get_flash_info`: Returns flash capacity, page size, and sector size.
    pub async fn get_flash_info(&self) -> (u32, usize, usize) {
        (
            self.flash.capacity(),
            self.flash.page_size(),
            self.flash.sector_size(),
        )
    }

    // - `dump_flash`: Reads and prints the start of a region as a debug hex dump.
    pub async fn dump_flash(&mut self, region: FlashRegion, len: usize) {
        let info = region.info();
        let mut buf = [0u8; 4096];
        let read_len = core::cmp::min(len, buf.len());
        if let Ok(()) = self.flash.read_data(info.start, &mut buf[..read_len]) {
            info!("Flash dump at 0x{:08X}: {:?}", info.start, &buf[..read_len]);
        } else {
            error!("Failed to dump flash at 0x{:08X}", info.start);
        }
    }
}
