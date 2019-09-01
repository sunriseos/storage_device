
use crate::block_device::{BlockDevice, BlockIndex};
use crate::block::Block;
use crate::error::{IoError, IoResult, IoOperation};
use core::mem::size_of;

/// Represent a device managing storage.
// we don't need is_empty, this would be stupid.
#[allow(clippy::len_without_is_empty)]
pub trait StorageDevice: core::fmt::Debug {
    /// Read the data at the given ``offset`` in the storage device into a given buffer.
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> IoResult<()>;

    /// Write the data from the given buffer at the given ``offset`` in the storage device.
    fn write(&mut self, offset: u64, buf: &[u8]) -> IoResult<()>;

    /// Return the total size of the storage device in bytes.
    fn len(&mut self) -> Result<u64, ()>;
}

/// Implementation of storage device for block device.
/// NOTE: This implementation doesn't use the heap.
/// NOTE: As it doesn't use a heap, read/write operations are done block by block. If you wish better performances, please consider implementing your own wrapper.
#[derive(Debug)]
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
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> IoResult<()> {
        let mut read_size = 0u64;
        let mut blocks = [B::Block::default()];

        while read_size < buf.len() as u64 {
            // Compute the next offset of the data to read.
            let current_offset = offset + read_size;

            // Extract the block index containing the data.
            let current_block_index = BlockIndex(current_offset / Block::LEN_U64);

            // Extract the offset inside the block containing the data.
            let current_block_offset = current_offset % Block::LEN_U64;

            // Read the block.
            self.block_device
                .read(&mut blocks[..], BlockIndex(current_block_index.0))
                .map_err(|block_device_err| IoError {
                    operation: IoOperation::Read,
                    offset,
                    len: buf.len(),
                    block_device_error: Some(block_device_err)
                })?;

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

    fn write(&mut self, offset: u64, buf: &[u8]) -> IoResult<()> {
        let mut write_size = 0u64;
        let mut blocks = [B::Block::default()];

        while write_size < buf.len() as u64 {
            // Compute the next offset of the data to write.
            let current_offset = offset + write_size;

            // Extract the block index containing the data.
            let current_block_index = BlockIndex(current_offset / Block::LEN_U64);

            // Extract the offset inside the block containing the data.
            let current_block_offset = current_offset % Block::LEN_U64;

            // Read the block.
            self.block_device
                .read(&mut blocks, BlockIndex(current_block_index.0))
                .map_err(|block_device_err| IoError {
                    operation: IoOperation::Write,
                    offset,
                    len: buf.len(),
                    block_device_error: Some(block_device_err)
                })?;

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

            self.block_device
                .write(&blocks, BlockIndex(current_block_index.0))
                .map_err(|block_device_err| IoError {
                    operation: IoOperation::Write,
                    offset,
                    len: buf.len(),
                    block_device_error: Some(block_device_err)
                })?;

            // Increment with what we wrote.
            write_size += buf_limit as u64;
        }

        Ok(())
    }

    fn len(&mut self) -> Result<u64, ()> {
        self.block_device.count()
            .map(|bc| bc.0 * size_of::<B::Block>() as u64)
    }
}

#[cfg(feature = "alloc")]
impl<S: StorageDevice + ?Sized> StorageDevice for alloc::boxed::Box<S> {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> IoResult<()> {
        (**self).read(offset, buf)
    }
    fn write(&mut self, offset: u64, buf: &[u8]) -> IoResult<()> {
        (**self).write(offset, buf)
    }

    fn len(&mut self) -> Result<u64, ()> {
        (**self).len()
    }
}

#[cfg(feature = "std")]
impl StorageDevice for std::fs::File {
    /// Read the data at the given ``offset`` in the storage device into a given buffer.
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> IoResult<()> {
        use std::io::{Read, Seek};

        self.seek(std::io::SeekFrom::Start(offset))
            .and_then(|_| self.read_exact(buf))
            .map_err(|_| IoError {
                operation: IoOperation::Read,
                offset,
                len: buf.len(),
                block_device_error: None // we're reading directly
            })
    }

    /// Write the data from the given buffer at the given ``offset`` in the storage device.
    fn write(&mut self, offset: u64, buf: &[u8]) -> IoResult<()> {
        use std::io::{Seek, Write};

        self.seek(std::io::SeekFrom::Start(offset))
            .and_then(|_| self.write_all(buf))
            .map_err(|_| IoError {
                operation: IoOperation::Write,
                offset,
                len: buf.len(),
                block_device_error: None // we're reading directly
            })
    }

    /// Return the total size of the storage device.
    fn len(&mut self) -> Result<u64, ()> {
        self.metadata()
            .map(|meta| meta.len())
            .map_err(|_| ())
    }
}

#[cfg(feature = "std")]
impl StorageDevice for &std::fs::File {
    /// Read the data at the given ``offset`` in the storage device into a given buffer.
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> IoResult<()> {
        use std::io::{Read, Seek};

        self.seek(std::io::SeekFrom::Start(offset))
            .and_then(|_| self.read_exact(buf))
            .map_err(|_| IoError {
                operation: IoOperation::Read,
                offset,
                len: buf.len(),
                block_device_error: None // we're reading directly
            })
    }

    /// Write the data from the given buffer at the given ``offset`` in the storage device.
    fn write(&mut self, offset: u64, buf: &[u8]) -> IoResult<()> {
        use std::io::{Seek, Write};

        self.seek(std::io::SeekFrom::Start(offset))
            .and_then(|_| self.write_all(buf))
            .map_err(|_| IoError {
                operation: IoOperation::Read,
                offset,
                len: buf.len(),
                block_device_error: None // we're reading directly
            })
    }

    /// Return the total size of the storage device.
    fn len(&mut self) -> Result<u64, ()> {
        self.metadata()
            .map(|meta| meta.len())
            .map_err(|_| ())
    }
}
