use crate::ex_flash::{FLASH_CAPACITY, PAGE_SIZE, SECTOR_SIZE, W25Q128FVSG};
// extern crate alloc;
use littlefs2::fs::FileAllocation;
use littlefs2::{
    driver::Storage,
    fs::{Allocation, File, Filesystem, OpenOptions, ReadDirAllocation},
    io::{Error as LfsError, Result as LfsResult},
    path::Path,
};

// Constants for LittleFS configuration
pub const READ_SIZE: usize = 16; // 16 bytes
pub const WRITE_SIZE: usize = PAGE_SIZE; // 256 bytes
pub const BLOCK_SIZE: usize = SECTOR_SIZE; // 4KB
pub const BLOCK_COUNT: usize = FLASH_CAPACITY as usize / BLOCK_SIZE; // 4096 blocks
pub const _CACHE: usize = 256;
pub const _LOOKAHEADWORDS_SIZE: usize = 32;

pub struct W25QStorage<'d> {
    flash: W25Q128FVSG<'d>,
}

impl<'d> Storage for W25QStorage<'d> {
    const READ_SIZE: usize = READ_SIZE;
    const WRITE_SIZE: usize = WRITE_SIZE;
    const BLOCK_SIZE: usize = BLOCK_SIZE;
    const BLOCK_COUNT: usize = BLOCK_COUNT;

    type CACHE_SIZE = littlefs2::consts::U256; // 256 bytes cache
    type LOOKAHEAD_SIZE = littlefs2::consts::U64;

    fn read(&mut self, off: usize, buf: &mut [u8]) -> LfsResult<usize> {
        if off >= FLASH_CAPACITY as usize {
            log::error!("Invalid read offset {off}: exceeds flash capacity {FLASH_CAPACITY}");
            return Err(LfsError::IO);
        }
        // log::info!("Reading at offset {}, len={}", off, buf.len());
        self.flash.read_data(off as u32, buf).map_err(|e| {
            log::error!("Read error at offset {off}: {e:?}");
            LfsError::IO
        })?;
        Ok(buf.len())
    }

    fn write(&mut self, off: usize, data: &[u8]) -> LfsResult<usize> {
        if off >= FLASH_CAPACITY as usize || off + data.len() > FLASH_CAPACITY as usize {
            log::error!(
                "Invalid write offset {} or length {}: exceeds flash capacity {}",
                off,
                data.len(),
                FLASH_CAPACITY
            );
            return Err(LfsError::IO);
        }
        // log::info!("W25Q write: offset={}, len={}", off, data.len());
        self.flash.write_data(off as u32, data).map_err(|e| {
            log::error!("Write error at offset {off}: {e:?}");
            LfsError::IO
        })?;
        Ok(data.len())
    }

    fn erase(&mut self, off: usize, len: usize) -> LfsResult<usize> {
        if off >= FLASH_CAPACITY as usize || off + len > FLASH_CAPACITY as usize {
            log::error!(
                "Invalid erase offset {off} or length {len}: exceeds flash capacity {FLASH_CAPACITY}"
            );
            return Err(LfsError::IO);
        }
        let start_block = off / BLOCK_SIZE;
        let end_block = (off + len - 1) / BLOCK_SIZE;
        for block in start_block..=end_block {
            let offset = block * BLOCK_SIZE;
            // log::info!("Erasing 4KB sector at offset: {}", offset);
            // Use 4KB sector erase to match BLOCK_SIZE
            self.flash.erase_sector(offset as u32).map_err(|e| {
                log::error!("Erase error at offset {offset}: {e:?}");
                LfsError::IO
            })?;
        }
        Ok(len)
    }
}

pub struct FlashFs<'d> {
    pub storage: W25QStorage<'d>,
    // alloc: Allocation<W25QStorage<'d>>,
    // fs: Option<Filesystem<'d, W25QStorage<'d>>>,
}

impl<'d> FlashFs<'d> {
    pub fn new(flash: W25Q128FVSG<'d>) -> Self {
        let storage = W25QStorage { flash };
        FlashFs { storage }
    }

    pub async fn init(
        &'d mut self,
        alloc: &'d mut Allocation<W25QStorage<'d>>,
    ) -> LfsResult<Filesystem<'d, W25QStorage<'d>>> {
        self.storage.flash.init().await.map_err(|e| {
            log::error!("Flash initialization error: {e:?}");
            LfsError::IO
        })?;
        log::info!("Flash initialized successfully");
        Filesystem::format(&mut self.storage).map_err(|e| {
            log::error!("Filesystem format error: {e:?}");
            LfsError::IO
        })?;
        log::info!("Filesystem formatted successfully");

        let fs = Filesystem::mount(alloc, &mut self.storage).map_err(|e| {
            log::error!("Filesystem mount error: {e:?}");
            LfsError::IO
        })?;
        // self.fs = Some(fs);
        log::info!("Filesystem mounted successfully");
        Ok(fs)
    }
}
pub fn write_file<'d>(
    fs: &mut Filesystem<'d, W25QStorage<'d>>,
    path: &str,
    data: &[u8],
    file_alloc: &mut FileAllocation<W25QStorage<'d>>,
) -> LfsResult<()> {
    let path = Path::from_str_with_nul(path).unwrap();
    let file = unsafe {
        OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(fs, file_alloc, path)?
    };
    let res = file.write(data).map(|_| ());
    if res.is_err() {
        // Nếu write lỗi, vẫn cố gắng close file để tránh giữ resource
        unsafe {
            file.close().ok();
        }
        return res;
    }

    unsafe {
        file.close()?; // Always close
    }

    log::info!("File written successfully: {path}");
    Ok(())
}

pub fn read_file<'d>(
    fs: &mut Filesystem<'d, W25QStorage<'d>>,
    path: &str,
    buffer: &mut [u8],
    file_alloc: &mut FileAllocation<W25QStorage<'d>>,
) -> LfsResult<usize> {
    let path = Path::from_str_with_nul(path).map_err(|_| LfsError::INVALID)?;
    let file = unsafe { File::open(fs, file_alloc, path)? };
    let n = file.read(buffer)?;
    Ok(n)
}
pub fn list_files<'d>(
    fs: &mut Filesystem<'d, W25QStorage<'d>>,
    path: &str,
    read_dir_alloc: &mut ReadDirAllocation,
) -> LfsResult<()> {
    // Ensure the path is null-terminated
    let path = Path::from_str_with_nul(path).map_err(|_| {
        log::error!("Path must be null-terminated");
        LfsError::INVALID
    })?;

    // Open the directory
    let read_dir = unsafe {
        Filesystem::read_dir(fs, read_dir_alloc, path).map_err(|e| {
            log::error!("Failed to open directory '{path}': {e:?}");
            LfsError::IO
        })?
    };

    // Iterate over entries
    for entry_result in read_dir {
        match entry_result {
            Ok(entry) => {
                let name = entry.file_name().as_str().as_bytes();
                let name_str = core::str::from_utf8(name).unwrap_or("<invalid utf8>");
                let kind = if entry.metadata().is_dir() {
                    "Dir "
                } else {
                    "File"
                };

                log::info!("[{kind}] {name_str}");
            }
            Err(e) => {
                log::error!("Error reading directory entry: {e:?}");
                return Err(LfsError::IO);
            }
        }
    }

    Ok(())
}

pub fn create_dir<'d>(fs: &mut Filesystem<'d, W25QStorage<'d>>, path: &str) -> LfsResult<()> {
    let path = Path::from_str_with_nul(path).map_err(|_| {
        log::error!("Invalid dir path (must be null-terminated)");
        LfsError::INVALID
    })?;
    fs.create_dir(path).map_err(|e| {
        log::error!("Failed to create directory '{path}': {e:?}");
        LfsError::IO
    })?;
    log::info!("Directory created: {path}");
    Ok(())
}
#[allow(unused)]
pub fn delete_dir<'d>(fs: &mut Filesystem<'d, W25QStorage<'d>>, path: &str) -> LfsResult<()> {
    let path = Path::from_str_with_nul(path).map_err(|_| LfsError::INVALID)?;
    fs.remove(path).map_err(|e| {
        log::error!("Failed to delete dir '{path}': {e:?}");
        LfsError::IO
    })?;
    log::info!("Directory deleted: {path}");
    Ok(())
}
