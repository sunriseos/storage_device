//! A storage device representation to help filesystem development.
//! Designed to be used in filesystem drivers.

#![no_std]

/// Block device representation.
pub mod block;

pub use block::*;

/// Represent a storage device error.
#[derive(Debug)]
pub enum StorageDeviceError {
    /// Read error.
    ReadError,

    /// Write error.
    WriteError,

    /// Unknown error.
    Unknown,
}

/// Represent a storage device result.
pub type StorageDeviceResult<T> = core::result::Result<T, StorageDeviceError>;

/// Represent a device managing storage.
// we don't need is_empty, this would be stupid.
#[allow(clippy::len_without_is_empty)]
pub trait StorageDevice {
    /// Read the data at the given ``offset`` in the storage device into a given buffer.
    fn read(&self, offset: u64, buf: &mut [u8]) -> StorageDeviceResult<()>;

    /// Write the data from the given buffer at the given ``offset`` in the storage device.
    fn write(&self, offset: u64, buf: &[u8]) -> StorageDeviceResult<()>;

    /// Return the total size of the storage device.
    fn len(&self) -> StorageDeviceResult<u64>;
}

impl From<BlockError> for StorageDeviceError {
    fn from(error: BlockError) -> Self {
        match error {
            BlockError::ReadError => StorageDeviceError::ReadError,
            BlockError::WriteError => StorageDeviceError::WriteError,
            BlockError::Unknown => StorageDeviceError::Unknown,
        }
    }
}

/// Implementation of storage device for block device.
/// NOTE: This implementation doesn't use the heap.
/// NOTE: As it doesn't use a heap, read/write operations are done block by block. If you wish better performances, please consider implementing your own wrapper.
pub struct StorageBlockDevice<B: BlockDevice> {
    /// The inner block device.
    block_device: B,
}

impl<B: BlockDevice> StorageBlockDevice<B> {
    /// Create a new storage block device.
    pub fn new(block_device: B) -> Self {
        StorageBlockDevice { block_device }
    }
}

impl<B: BlockDevice> StorageDevice for StorageBlockDevice<B> {
    fn read(&self, offset: u64, buf: &mut [u8]) -> StorageDeviceResult<()> {
        let mut read_size = 0u64;
        let mut blocks = [Block::new()];

        while read_size < buf.len() as u64 {
            // Compute the next offset of the data to read.
            let current_offset = offset + read_size;

            // Extract the block index containing the data.
            let current_block_index = BlockIndex(current_offset / Block::LEN_U64);

            // Extract the offset inside the block containing the data.
            let current_block_offset = current_offset % Block::LEN_U64;

            // Read the block.
            self.block_device.read(&mut blocks, BlockIndex(current_block_index.0))?;

            // Slice on the part of the buffer we need.
            let buf_slice = &mut buf[read_size as usize..];

            // Limit copy to the size of a block or lower.
            let buf_limit = if buf_slice.len() + current_block_offset as usize >= Block::LEN {
                Block::LEN - current_block_offset as usize
            } else {
                buf_slice.len()
            };

            // Copy the data into the buffer.
            for (index, buf_entry) in buf_slice.iter_mut().take(buf_limit).enumerate() {
                *buf_entry = blocks[0][current_block_offset as usize + index];
            }

            // Increment with what we read.
            read_size += buf_limit as u64;
        }

        Ok(())
    }

    fn write(&self, offset: u64, buf: &[u8]) -> StorageDeviceResult<()> {
        let mut write_size = 0u64;
        let mut blocks = [Block::new()];

        while write_size < buf.len() as u64 {
            // Compute the next offset of the data to write.
            let current_offset = offset + write_size;

            // Extract the block index containing the data.
            let current_block_index = BlockIndex(current_offset / Block::LEN_U64);

            // Extract the offset inside the block containing the data.
            let current_block_offset = current_offset % Block::LEN_U64;

            // Read the block.
            self.block_device.read(&mut blocks, BlockIndex(current_block_index.0))?;

            // Slice on the part of the buffer we need.
            let buf_slice = &buf[write_size as usize..];

            // Limit copy to the size of a block or lower.
            let buf_limit = if buf_slice.len() + current_block_offset as usize >= Block::LEN {
                Block::LEN - current_block_offset as usize
            } else {
                buf_slice.len()
            };

            let block_slice = &mut blocks[0][current_block_offset as usize..];

            // Copy the data from the buffer.
            for (index, buf_entry) in block_slice.iter_mut().take(buf_limit).enumerate() {
                *buf_entry = buf_slice[index];
            }

            self.block_device.write(&blocks, BlockIndex(current_block_index.0))?;

            // Increment with what we wrote.
            write_size += buf_limit as u64;
        }

        Ok(())
    }

    fn len(&self) -> StorageDeviceResult<u64> {
        Ok(self.block_device.count()?.into_bytes_count())
    }
}
