pub mod ex_flash;
#[allow(unused_imports)]
use crate::filesystem::ex_flash::{ExFlashError, W25Q128FVSG};
use embassy_executor::Spawner;
use embassy_time::{with_timeout, Duration, Timer};
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
use heapless::Vec;
use log::{error, info, warn};

#[derive(Debug)]
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

#[derive(Debug)]
pub enum FlashRegion {
    Firmware = 0x000000,
    Certstore = 0x300000,
    UserData = 0x340000,
}

const CERT_DIR_ADDR: u32 = FlashRegion::Certstore as u32;
const CERT_DIR_SIZE: usize = 0x1000;
const MAX_FILES: usize = 16;
const MAX_NAME: usize = 32;

#[derive(Clone)]
struct DirEntry {
    name_len: u8,
    name: heapless::String<MAX_NAME>,
    offset: u32,
    len: u32,
}

pub struct FlashController<'a> {
    flash: &'a mut W25Q128FVSG<'a>,
    last_offset: u32, // Track last used offset for Certstore
}

pub struct RegionInfo {
    pub start: u32,
    pub size: u32,
}

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

impl<'a> FlashController<'a> {
    pub fn new(flash: &'a mut W25Q128FVSG<'a>) -> Self {
        Self {
            flash,
            last_offset: 0, // Initialize to start of Certstore
        }
    }

    pub async fn erase_region(&mut self, region: FlashRegion) -> Result<(), FsError> {
        let info = region.info();
        self.erase_range(info.start, info.start + info.size)
            .await
            .map_err(FsError::FlashError)?;
        self.last_offset = 0; // Reset offset after full erase
        info!("Erased region {:?}", region);
        Ok(())
    }

    async fn erase_range(&mut self, start: u32, end: u32) -> Result<(), ExFlashError> {
        if start >= end || end > self.flash.capacity() as u32 {
            return Err(ExFlashError::AddressInvalid);
        }

        let block_size = 64 * 1024; // 64 KiB
        let sector_size = self.flash.sector_size() as u32; // 4 KiB

        if end - start >= block_size {
            let start_block = start & !(block_size - 1);
            let end_block = (end + block_size - 1) & !(block_size - 1);
            info!(
                "Erasing 64KiB blocks from 0x{:08X} to 0x{:08X}",
                start_block, end_block
            );
            for addr in (start_block..end_block).step_by(block_size as usize) {
                let mut buf = [0u8; 64];
                self.flash.read_data(addr, &mut buf).await?;
                if buf.iter().all(|&b| b == 0xFF) {
                    info!("Block at 0x{:08X} already erased, skipping", addr);
                    continue;
                }
                info!("Erasing 64KiB block at 0x{:08X}", addr);
                match with_timeout(
                    Duration::from_millis(2500),
                    self.flash.erase_block_64kb(addr),
                )
                .await
                {
                    Ok(Ok(())) => info!("✓ Block erased at 0x{:08X}", addr),
                    Ok(Err(e)) => {
                        error!("✗ Failed to erase block at 0x{:08X}: {:?}", addr, e);
                        return Err(e);
                    }
                    Err(_) => {
                        error!("✗ Timeout erasing block at 0x{:08X}", addr);
                        return Err(ExFlashError::Timeout);
                    }
                }
            }
        } else {
            let start_sector = start & !(sector_size - 1);
            let end_sector = (end + sector_size - 1) & !(sector_size - 1);
            info!(
                "Erasing sectors from 0x{:08X} to 0x{:08X}",
                start_sector, end_sector
            );
            for addr in (start_sector..end_sector).step_by(sector_size as usize) {
                let mut buf = [0u8; 64];
                self.flash.read_data(addr, &mut buf).await?;
                if buf.iter().all(|&b| b == 0xFF) {
                    info!("Sector at 0x{:08X} already erased, skipping", addr);
                    continue;
                }
                info!("Erasing sector at 0x{:08X}", addr);
                match with_timeout(Duration::from_millis(500), self.flash.erase_sector(addr)).await
                {
                    Ok(Ok(())) => info!("✓ Sector erased at 0x{:08X}", addr),
                    Ok(Err(e)) => {
                        error!("✗ Failed to erase sector at 0x{:08X}: {:?}", addr, e);
                        return Err(e);
                    }
                    Err(_) => {
                        error!("✗ Timeout erasing sector at 0x{:08X}", addr);
                        return Err(ExFlashError::Timeout);
                    }
                }
            }
        }
        Ok(())
    }

    async fn find_free_offset(&mut self, region: &FlashRegion) -> Option<u32> {
        let info = region.info();
        let page_size = self.flash.page_size() as u32;
        let mut cur = self.last_offset;

        while cur < info.size {
            let addr = info.start + cur;

            let mut len_buf = [0u8; 1];
            if let Err(e) = self.flash.read_data(addr, &mut len_buf).await {
                error!(
                    "Failed to read filename length at address 0x{:08X}: {:?}",
                    addr, e
                );
                return None;
            }

            let name_len = len_buf[0] as u32;
            info!(
                "find_free_offset: addr 0x{:08X}, name_len {}",
                addr, name_len
            );
            if name_len == 0xFF {
                let aligned_offset = (cur + page_size - 1) & !(page_size - 1);
                if aligned_offset + 1 > info.size {
                    error!("No free space left in region {:?}", region);
                    return None;
                }
                info!("Found free offset 0x{:X}", aligned_offset);
                return Some(aligned_offset);
            }

            if name_len > MAX_NAME as u32 {
                error!("Invalid name length {} at address 0x{:08X}", name_len, addr);
                return None;
            }

            let mut size_buf = [0u8; 4];
            let size_addr = addr + 1 + name_len;
            if let Err(e) = self.flash.read_data(size_addr, &mut size_buf).await {
                error!(
                    "Failed to read data length at address 0x{:08X}: {:?}",
                    size_addr, e
                );
                return None;
            }
            let data_len = u32::from_le_bytes(size_buf);
            info!("Found entry at 0x{:08X}, data_len {}", addr, data_len);

            cur += 1 + name_len + 4 + data_len;
            cur = (cur + page_size - 1) & !(page_size - 1);
        }
        error!("No free space found in region {:?}", region);
        None
    }

    pub async fn write_file(
        &mut self,
        region: FlashRegion,
        filename: &str,
        data: &[u8],
    ) -> Result<(), FsError> {
        if filename.len() > MAX_NAME {
            error!(
                "Filename '{}' exceeds maximum length of {} characters",
                filename, MAX_NAME
            );
            return Err(FsError::FilenameTooLong);
        }

        if data.is_empty() {
            warn!("Writing empty file '{}'", filename);
            return Ok(());
        }

        let info = region.info();
        let file_size = 1 + filename.len() + 4 + data.len();
        if file_size > 4096 {
            error!(
                "File '{}' too large for buffer: {} bytes",
                filename, file_size
            );
            return Err(FsError::FileTooLarge);
        }

        let page_size = self.flash.page_size() as u32; // 256 bytes
        let sector_size = self.flash.sector_size() as u32; // 4096 bytes

        // Find free offset, aligned to page boundary
        let offset = self
            .find_free_offset(&region)
            .await
            .ok_or(FsError::InvalidAddress)?;
        let abs_address = (info.start + offset + page_size - 1) & !(page_size - 1); // Align to next page boundary

        if abs_address + file_size as u32 > info.start + info.size {
            error!(
                "Not enough space in region {:?} to write file '{}': {} bytes required, {} bytes available",
                region, filename, file_size, info.size - (abs_address - info.start)
            );
            return Err(FsError::FileTooLarge);
        }

        // Prepare the file buffer
        let mut buf = Vec::<u8, 4096>::new();
        buf.push(filename.len() as u8).unwrap();
        buf.extend_from_slice(filename.as_bytes()).unwrap();
        buf.extend_from_slice(&(data.len() as u32).to_le_bytes())
            .unwrap();
        buf.extend_from_slice(data).unwrap();
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
        info!("Scanning for existing files up to offset 0x{:08X}", offset);
        while cur < offset {
            let addr = info.start + cur;
            let mut len_buf = [0u8; 1];
            if let Err(e) = self.flash.read_data(addr, &mut len_buf).await {
                error!("Failed to read name length at 0x{:08X}: {:?}", addr, e);
                return Err(FsError::FlashError(e));
            }
            let name_len = len_buf[0] as u32;
            info!("At 0x{:08X}, name_len: {}", addr, name_len);
            if name_len == 0xFF {
                info!("No more valid entries at 0x{:08X}", addr);
                break;
            }
            if name_len > MAX_NAME as u32 {
                error!("Invalid name length {} at 0x{:08X}", name_len, addr);
                return Err(FsError::InvalidAddress);
            }

            let mut name_buf = [0u8; MAX_NAME];
            if let Err(e) = self
                .flash
                .read_data(addr + 1, &mut name_buf[..name_len as usize])
                .await
            {
                error!("Failed to read name at 0x{:08X}: {:?}", addr + 1, e);
                return Err(FsError::FlashError(e));
            }
            let name = core::str::from_utf8(&name_buf[..name_len as usize]).unwrap_or("<invalid>");
            info!("Found file '{}' at 0x{:08X}", name, addr);

            let mut size_buf = [0u8; 4];
            let size_addr = addr + 1 + name_len;
            if let Err(e) = self.flash.read_data(size_addr, &mut size_buf).await {
                error!("Failed to read data length at 0x{:08X}: {:?}", size_addr, e);
                return Err(FsError::FlashError(e));
            }
            let data_len = u32::from_le_bytes(size_buf);
            info!("Data length: {} bytes", data_len);

            let entry_size = 1 + name_len + 4 + data_len;
            let entry_end = addr + entry_size;
            let aligned_end = (entry_end + page_size - 1) & !(page_size - 1);

            // Read the entire entry (aligned to page boundaries)
            let mut entry_data = Vec::<u8, 4096>::new();
            entry_data
                .extend_from_slice(&[0xFFu8; 4096][..(aligned_end - addr) as usize])
                .map_err(|_| {
                    error!("Entry data buffer overflow at 0x{:08X}", addr);
                    FsError::FileTooLarge
                })?;
            if let Err(e) = self
                .flash
                .read_data(addr, &mut entry_data[..(aligned_end - addr) as usize])
                .await
            {
                error!("Failed to read entry at 0x{:08X}: {:?}", addr, e);
                return Err(FsError::FlashError(e));
            }
            info!(
                "Read {} bytes for entry at 0x{:08X}: {:?}",
                (aligned_end - addr),
                addr,
                &entry_data[..16]
            );
            preserved_data.extend_from_slice(&entry_data).map_err(|_| {
                error!("Preserved data buffer overflow at 0x{:08X}", addr);
                FsError::FileTooLarge
            })?;

            cur += entry_size;
            cur = (cur + page_size - 1) & !(page_size - 1);
        }

        // Check if the pages to be written contain non-erased data
        let mut needs_erase = false;
        for page_addr in (start_page..end_page).step_by(page_size as usize) {
            let mut page_buf = [0xFFu8; 256];
            if let Err(e) = self.flash.read_data(page_addr, &mut page_buf).await {
                error!("Failed to read page at 0x{:08X}: {:?}", page_addr, e);
                return Err(FsError::FlashError(e));
            }
            if !page_buf.iter().all(|&b| b == 0xFF) {
                needs_erase = true;
                break;
            }
        }

        if needs_erase {
            info!("Erasing sector at 0x{:08X}", start_sector);
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
                        info!("Rewriting {} bytes to 0x{:08X}", chunk_size, page_addr);
                        self.flash
                            .write_data(
                                page_addr,
                                &preserved_data[page_offset..page_offset + chunk_size],
                            )
                            .await
                            .map_err(|e| {
                                error!(
                                    "Failed to rewrite preserved data to 0x{:08X}: {:?}",
                                    page_addr, e
                                );
                                FsError::FlashError(e)
                            })?;

                        // Verify preserved data
                        let mut verify_buf = [0xFFu8; 256];
                        if let Err(e) = self
                            .flash
                            .read_data(page_addr, &mut verify_buf[..chunk_size])
                            .await
                        {
                            error!(
                                "Failed to verify preserved data at 0x{:08X}: {:?}",
                                page_addr, e
                            );
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
                        info!("Verified preserved data at 0x{:08X}", page_addr);
                    }
                }
            }
        } else {
            info!(
                "No erase needed for pages 0x{:08X} to 0x{:08X}",
                start_page, end_page
            );
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
            info!("Writing {} bytes to 0x{:08X}", chunk_size, page_addr);
            self.flash
                .write_data(
                    page_addr,
                    &padded_buf[page_offset..page_offset + chunk_size],
                )
                .await
                .map_err(|e| {
                    error!("Failed to write to 0x{:08X}: {:?}", page_addr, e);
                    FsError::FlashError(e)
                })?;

            // Verify the write
            let mut verify_buf = [0xFFu8; 256];
            if let Err(e) = self
                .flash
                .read_data(page_addr, &mut verify_buf[..chunk_size])
                .await
            {
                error!("Failed to verify read at 0x{:08X}: {:?}", page_addr, e);
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
            info!("Verified write at 0x{:08X}", page_addr);
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

    pub async fn write_firmware(
        &mut self,
        region: FlashRegion,
        filename: &str,
        data: &[u8],
    ) -> Result<(), FsError> {
        let address = region as u32;
        if address + data.len() as u32 > self.flash.capacity() as u32 {
            error!(
                "Firmware data exceeds flash capacity at address 0x{:08X}",
                address
            );
            return Err(FsError::FileTooLarge);
        }
        let mut metadata = [0u8; 272];
        let n = 1 + filename.len();
        metadata[0] = filename.len() as u8;
        metadata[1..1 + filename.len()].copy_from_slice(filename.as_bytes());
        metadata[n..n + 4].copy_from_slice(&(data.len() as u32).to_le_bytes());

        self.erase_range(address, address + metadata.len() as u32)
            .await?;
        self.flash.write_data(address, &metadata[..n + 4]).await?;

        let mut offset = 0;
        let page = self.flash.page_size();
        while offset < data.len() {
            let chunk = core::cmp::min(page, data.len() - offset);
            self.flash
                .write_data(
                    address + (n + 4) as u32 + offset as u32,
                    &data[offset..offset + chunk],
                )
                .await
                .map_err(FsError::FlashError)?;
            offset += page;
        }
        Ok(())
    }

    pub async fn read_file(
        &mut self,
        region: FlashRegion,
        filename: &str,
        buffer: &mut [u8],
    ) -> Result<usize, FsError> {
        let info = region.info();
        let page_size = self.flash.page_size() as u32;
        let mut cur = 0;

        info!("Searching for file '{}' in region {:?}", filename, region);
        while cur < info.size {
            let addr = info.start + cur;
            info!("Checking entry at address 0x{:08X}", addr);

            let mut len = [0u8; 1];
            if let Err(e) = self.flash.read_data(addr, &mut len).await {
                error!("Failed to read name length at 0x{:08X}: {:?}", addr, e);
                return Err(FsError::FlashError(e));
            }
            let name_len = len[0] as usize;
            info!("Name length: {}", name_len);

            if name_len == 0xFF {
                info!("End of directory reached at 0x{:08X}", addr);
                break;
            }

            if name_len > MAX_NAME {
                error!("Invalid name length {} at 0x{:08X}", name_len, addr);
                return Err(FsError::InvalidAddress);
            }

            let mut name_buf = [0u8; MAX_NAME];
            if let Err(e) = self
                .flash
                .read_data(addr + 1, &mut name_buf[..name_len])
                .await
            {
                error!("Failed to read filename at 0x{:08X}: {:?}", addr + 1, e);
                return Err(FsError::FlashError(e));
            }
            let name = core::str::from_utf8(&name_buf[..name_len]).unwrap_or("");
            info!("Found file '{}'", name);

            let mut size_buf = [0u8; 4];
            let size_addr = addr + 1 + name_len as u32;
            if let Err(e) = self.flash.read_data(size_addr, &mut size_buf).await {
                error!("Failed to read data length at 0x{:08X}: {:?}", size_addr, e);
                return Err(FsError::FlashError(e));
            }
            let data_len = u32::from_le_bytes(size_buf);
            info!("Data length: {} bytes", data_len);

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
                    .await
                {
                    error!("Failed to read data at 0x{:08X}: {:?}", data_addr, e);
                    return Err(FsError::FlashError(e));
                }
                info!(
                    "✓ Read file '{}' ({} B) from 0x{:08X}",
                    filename, data_len, addr
                );
                return Ok(data_len as usize);
            }

            cur += 1 + name_len as u32 + 4 + data_len;
            cur = (cur + page_size - 1) & !(page_size - 1);
        }

        error!("File '{}' not found in region {:?}", filename, region);
        Err(FsError::FileNotFound)
    }

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
                if &buf[..n] == expected_data {
                    info!("✓ File '{}' verification passed", filename);
                    true
                } else {
                    error!("✗ File '{}' verification failed - data mismatch", filename);
                    false
                }
            }
            Err(e) => {
                error!("✗ Failed to read file '{}': {:?}", filename, e);
                false
            }
        }
    }

    pub async fn list_files(
        &mut self,
        region: FlashRegion,
    ) -> Result<Vec<DirEntry, MAX_FILES>, FsError> {
        let mut files = Vec::<DirEntry, MAX_FILES>::new();
        let info = region.info();
        let mut cur = 0;
        let page_size = self.flash.page_size() as u32;

        info!("Listing files in region {:?}", region);

        while cur < info.size {
            let addr = info.start + cur;

            let mut len_buf = [0u8; 1];
            self.flash
                .read_data(addr, &mut len_buf)
                .await
                .map_err(FsError::FlashError)?;
            let name_len = len_buf[0] as usize;

            if name_len == 0xFF {
                info!("End of directory at 0x{:08X}", addr);
                break;
            }

            if name_len == 0 || name_len > MAX_NAME {
                error!("Invalid name_len {} at 0x{:08X}", name_len, addr);
                return Err(FsError::InvalidAddress);
            }

            let mut name_buf = [0u8; MAX_NAME];
            self.flash
                .read_data(addr + 1, &mut name_buf[..name_len])
                .await
                .map_err(FsError::FlashError)?;
            let name = core::str::from_utf8(&name_buf[..name_len])
                .unwrap_or("<invalid>")
                .to_string();

            let mut size_buf = [0u8; 4];
            let size_addr = addr + 1 + name_len as u32;
            self.flash
                .read_data(size_addr, &mut size_buf)
                .await
                .map_err(FsError::FlashError)?;
            let data_len = u32::from_le_bytes(size_buf);

            let entry = DirEntry {
                name_len: name_len as u8,
                name: heapless::String::from(name),
                offset: addr,
                len: data_len,
            };
            files.push(entry).map_err(|_| FsError::FileTooLarge)?;

            cur += 1 + name_len as u32 + 4 + data_len;
            cur = (cur + page_size - 1) & !(page_size - 1);
        }

        Ok(files)
    }

    pub async fn get_flash_info(&self) -> (u32, usize, usize) {
        (
            self.flash.capacity(),
            self.flash.page_size(),
            self.flash.sector_size(),
        )
    }

    pub async fn dump_flash(&mut self, region: FlashRegion, len: usize) {
        let info = region.info();
        let mut buf = [0u8; 4096];
        let read_len = core::cmp::min(len, buf.len());
        if let Ok(()) = self.flash.read_data(info.start, &mut buf[..read_len]).await {
            info!("Flash dump at 0x{:08X}: {:?}", info.start, &buf[..read_len]);
        } else {
            error!("Failed to dump flash at 0x{:08X}", info.start);
        }
    }
}
