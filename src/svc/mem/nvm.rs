use embedded_storage::{ReadStorage, Storage};
use esp_storage::FlashStorage;

#[derive(Copy, Clone)]
#[allow(dead_code)]
pub enum BlockId {
    CrtPemId = 0x0,
    DvtCrtId = 0x1,
    DvtKeyId = 0x2,
    BlockNum = 0x3,
}

pub enum NvsError {
    IdInvalid,
    LenInvalid,
    WriteErr,
    ReadErr,
}

#[allow(dead_code)]
struct NvmConf {
    pub addr: u32,
    pub size: usize,
}

#[allow(dead_code)]
struct Nvm {
    blocks: [NvmConf; 0x03],
    storage: FlashStorage,
}

#[allow(dead_code)]
impl Nvm {
    pub fn init() -> Self {
        Self {
            blocks: [
                NvmConf {
                    addr: 0x9000,
                    size: 2574,
                },
                NvmConf {
                    addr: 0xA000,
                    size: 1268,
                },
                NvmConf {
                    addr: 0xB000,
                    size: 1678,
                },
            ],
            storage: FlashStorage::new(),
        }
    }

    pub fn nvs_write(&mut self, id: BlockId, buf: &[u8]) -> Result<(), NvsError> {
        if id as usize >= self.blocks.len() {
            return Err(NvsError::IdInvalid);
        }

        let block = &self.blocks[id as usize];

        if buf.len() != block.size {
            return Err(NvsError::LenInvalid);
        }

        match self.storage.write(block.addr, buf) {
            Ok(_) => Ok(()),
            Err(_) => Err(NvsError::WriteErr),
        }
    }

    pub fn nvs_read(&mut self, id: BlockId, buf: &mut [u8]) -> Result<(), NvsError> {
        if id as usize >= self.blocks.len() {
            return Err(NvsError::IdInvalid);
        }

        let block = &self.blocks[id as usize];

        if buf.len() != block.size {
            return Err(NvsError::LenInvalid);
        }

        match self.storage.read(block.addr, buf) {
            Ok(_) => Ok(()),
            Err(_) => Err(NvsError::ReadErr),
        }
    }
}
