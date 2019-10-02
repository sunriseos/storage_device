//! Randomly accessible device
//!
//! This modules exposes the [`StorageDevice`] to represent any device that exposes byte-granular
//! read and write operations, as opposed to block-size operations, and the [`StorageBlockDevice`]
//! struct that can turn any `BlockDevice` into a `StorageDevice` by performing multiple block-align
//! operations.

use crate::block_device::{BlockDevice, BlockIndex};
use core::mem::size_of;
use core::fmt::Debug;
use plain::Plain;

/// A trait to represent any device that exposes byte-granular read and write operations,
/// as opposed to block-size operations.
///
/// A `StorageDevice` can read/write to/from arbitrary length buffers, and at arbitrary offsets.
// we don't need is_empty, this would be stupid.
#[allow(clippy::len_without_is_empty)]
pub trait StorageDevice: Debug {
    /// Error type returned by this block device when an operation fails
    type Error: Debug;

    /// Read the data at the given ``offset`` in the storage device into a given buffer.
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Self::Error>;

    /// Write the data from the given buffer at the given ``offset`` in the storage device.
    ///
    /// Writes aren't guaranteed to persist to disk until `flush()` is called.
    fn write(&mut self, offset: u64, buf: &[u8]) -> Result<(), Self::Error>;

    /// Persists writes to disk.
    ///
    /// Default implementation is a noop.
    fn flush(&mut self) -> Result<(), Self::Error>;

    /// Return the total size of the storage device in bytes.
    fn len(&mut self) -> Result<u64, Self::Error>;
}

/// Turns any [`BlockDevice`] to a [`StorageDevice`] by implementing the logic to read and write
/// from/to block-size unaligned offsets and length.
///
/// This implementation doesn't use the heap, which means it will try to perform the requests
/// in-place in the user-provided buffer, and deal with the first and last incomplete block in a
/// single temporary block that resides in the StorageBlockDevice.
///
/// Because we're reading in the (often to small) user-provided buffer, we cannot read everything
/// in one go, but will try to reduce the number of requests to the underlying `BlockDevice` to
/// a minimum. An operation will be split in at most 3 requests, for the first truncated block,
/// the last truncated block, and every other block in the middle in one go.
///
/// Note however that if the buffer we're reading from/to isn't Block aligned, we will do a lot more
/// requests, and performances are going to be highly degraded.
pub struct StorageBlockDevice<BD: BlockDevice> {
    /// The inner block device.
    block_device: BD,
    /// A single block used for partial read/writes.
    tmp_block: BD::Block,
}

impl<BD: BlockDevice> core::fmt::Debug for StorageBlockDevice<BD> {
    /// Debugging a StorageBlockDevice doesn't display `.tmp_block`.
    fn fmt(&self, f: &mut core::fmt::Formatter) -> Result<(), core::fmt::Error> {
        f.debug_struct("StorageBlockDevice")
            .field("block_device", &self.block_device)
            .finish()
    }
}

impl<BD: BlockDevice> StorageBlockDevice<BD> {
    /// Create a new storage block device.
    pub fn new(block_device: BD) -> Self {
        StorageBlockDevice { block_device, tmp_block: BD::Block::default() }
    }

    /// Reads from the block device from an arbitrary offset to an arbitrary len buffer.
    ///
    /// The logic is the following:
    ///
    /// 1. Read the first truncated block to our `.tmp_block`, and copy only the desired bytes to
    /// the destination buffer.
    /// 2. Read all the middle blocks in one go.
    /// 3. Read the last truncated block to our `.tmp_block`, and copy only the desired bytes to
    /// the destination buffer.
    ///
    /// Depending on how `offset` and/or `buf.len` relate to block size, we might not need the
    /// first and/or last step, and we will save one disk request. This operation will perform
    /// at most 3 device requests when `buf` is properly aligned.
    ///
    /// # Unaligned buffers
    ///
    /// When at step 2, if the buffer's middle part isn't block aligned, we cannot read directly to
    /// it. In this case, we're reading one block at a time, and the number of requests we will make
    /// can be alarming. So try to avoid this condition the better you can.
    fn read_internal(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), BD::Error> {
        // here's how we're splitting our operation
        let first_part_block = offset / size_of::<BD::Block>() as u64;
        let first_part_len = (size_of::<BD::Block>() as u64 - (offset % size_of::<BD::Block>() as u64)) as usize;
        let middle_part_block = if first_part_len == 0 { first_part_block } else { first_part_block + 1 };
        let end_part_block = (offset + buf.len() as u64) / size_of::<BD::Block>() as u64;
        let end_part_len = ((offset + buf.len() as u64) % size_of::<BD::Block>() as u64) as usize;
        let middle_part_len = buf.len() - first_part_len - end_part_len;

        {
            // the the first part, if any

            // truncate the buffer to only the interesting part so we're sure we don't spill.
            let buf = &mut buf[..first_part_len];

            if first_part_len > 0 {
                // first read a whole block into our tmp block.
                self.block_device.read(
                    core::slice::from_mut(&mut self.tmp_block),
                    BlockIndex(first_part_block)
                )?;
                // and copy only the end bytes to our destination buffer
                buf.copy_from_slice(&self.tmp_block[(size_of::<BD::Block>() - first_part_len)..]);
            }
        }

        {
            // the middle part, if any

            // truncate the buffer to only the interesting part so we're sure we don't spill.
            let buf = &mut buf[first_part_len..(first_part_len + middle_part_len)];

            if middle_part_len > 0 {
                // try to cast the buffer as an array of Blocks, but align may be wrong
                if let Ok(blocks) = BD::Block::slice_from_mut_bytes(buf) {
                    // read everything in one go
                    self.block_device.read(
                        blocks,
                        BlockIndex(middle_part_block)
                    )?;
                } else {
                    // buffer isn't block aligned, we can't read directly to it easily.
                    // we're going to read one block at a time and perfs are going to be shit.
                    for (i, block) in (middle_part_block..end_part_block).enumerate() {
                        // read to tmp block
                        self.block_device.read(
                            core::slice::from_mut(&mut self.tmp_block),
                            BlockIndex(block)
                        )?;
                        // copy to buffer
                        buf[(i * size_of::<BD::Block>())..((i + 1) * size_of::<BD::Block>())]
                            .copy_from_slice(&self.tmp_block);
                    }
                }
            }
        }

        {
            // and finally the last part, if any

            // truncate the buffer to only the interesting part so we're sure we don't spill.
            let buf = &mut buf[(first_part_len + middle_part_len)..];

            if end_part_len > 0 {
                // read a whole block into our tmp block.
                self.block_device.read(
                    core::slice::from_mut(&mut self.tmp_block),
                    BlockIndex(end_part_block)
                )?;
                // and copy only the end bytes to our destination buffer
                buf.copy_from_slice(&self.tmp_block[..end_part_len]);
            }
        }

        Ok(())
    }

    /// Writes to the block device from an arbitrary offset and an arbitrary len buffer.
    ///
    /// The logic is the following:
    ///
    /// 1. Read the first truncated block to our `.tmp_block`, copy only the desired bytes from
    /// the destination buffer, and write back the updated block to the device.
    /// 2. Write all the middle blocks in one go.
    /// 3. Read the last truncated block to our `.tmp_block`, copy only the desired bytes from
    /// the destination buffer, and write back the updated block to the device.
    ///
    /// Depending on how `offset` and/or `buf.len` relate to block size, we might not need the
    /// first and/or last step, and we will save one disk request. This operation will perform at
    /// most 5 device requests when `buf` is properly aligned.
    ///
    /// # Unaligned buffers
    ///
    /// When at step 2, if the buffer's middle part isn't block aligned, we cannot write directly to
    /// it. In this case, we're writing one block at a time, and the number of requests we will make
    /// can be alarming. So try to avoid this condition the better you can.
    fn write_internal(&mut self, offset: u64, buf: &[u8]) -> Result<(), BD::Error> {
        // here's how we're splitting our operation
        let first_part_block = offset / size_of::<BD::Block>() as u64;
        let first_part_len = (size_of::<BD::Block>() as u64 - (offset % size_of::<BD::Block>() as u64)) as usize;
        let middle_part_block = if first_part_len == 0 { first_part_block } else { first_part_block + 1 };
        let end_part_block = (offset + buf.len() as u64) / size_of::<BD::Block>() as u64;
        let end_part_len = ((offset + buf.len() as u64) % size_of::<BD::Block>() as u64) as usize;
        let middle_part_len = buf.len() - first_part_len - end_part_len;

        {
            // the the first part, if any

            // truncate the buffer to only the interesting part so we're sure we don't spill.
            let buf = &buf[..first_part_len];

            if first_part_len > 0 {
                // first read a whole block into our tmp block.
                self.block_device.read(
                    core::slice::from_mut(&mut self.tmp_block),
                    BlockIndex(first_part_block)
                )?;
                // copy bytes from our buffer to last bytes of our tmp block
                let block_bytes = unsafe {
                    // safe: the contract on Blocks guarantees us we can do that
                    plain::as_mut_bytes(&mut self.tmp_block)
                };
                block_bytes[(size_of::<BD::Block>() - first_part_len)..].copy_from_slice(buf);

                // and write back the block to the device
                self.block_device.write(
                    core::slice::from_ref(&self.tmp_block),
                    BlockIndex(first_part_block)
                )?;
            }
        }

        {
            // the middle part, if any

            // truncate the buffer to only the interesting part so we're sure we don't spill.
            let buf = &buf[first_part_len..(first_part_len + middle_part_len)];

            if middle_part_len > 0 {
                // try to cast the buffer as an array of Blocks, but align may be wrong
                if let Ok(blocks) = BD::Block::slice_from_bytes(buf) {
                    // write everything in one go
                    self.block_device.write(
                        blocks,
                        BlockIndex(middle_part_block)
                    )?;
                } else {
                    // buffer isn't block aligned, we can't write directly from it easily.
                    // we're going to write one block at a time and perfs are going to be shit.
                    for (i, block) in (middle_part_block..end_part_block).enumerate() {
                        // copy from buffer to aligned tmp block
                        self.tmp_block.copy_from_slice(
                            &buf[(i * size_of::<BD::Block>())..((i + 1) * size_of::<BD::Block>())]);
                        // write the tmp block
                        self.block_device.write(
                            core::slice::from_mut(&mut self.tmp_block),
                            BlockIndex(block)
                        )?;
                    }
                }
            }
        }

        {
            // and finally the last part, if any

            // truncate the buffer to only the interesting part so we're sure we don't spill.
            let buf = &buf[(first_part_len + middle_part_len)..];

            if end_part_len > 0 {
                // read a whole block into our tmp block.
                self.block_device.read(
                    core::slice::from_mut(&mut self.tmp_block),
                    BlockIndex(end_part_block)
                )?;
                // copy only the end bytes from our buffer to the first bytes of our tmp block
                let block_bytes = unsafe {
                    // safe: the contract on Blocks guarantees us we can do that
                    plain::as_mut_bytes(&mut self.tmp_block)
                };
                block_bytes[..end_part_len].copy_from_slice(buf);
                // and write back the tmp block
                self.block_device.write(
                    core::slice::from_mut(&mut self.tmp_block),
                    BlockIndex(end_part_block)
                )?;
            }
        }

        Ok(())
    }
}

impl<B: BlockDevice> StorageDevice for StorageBlockDevice<B> {
    type Error = B::Error;

    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Self::Error> {
        self.read_internal(offset, buf)
    }

    fn write(&mut self, offset: u64, buf: &[u8]) -> Result<(), Self::Error> {
        // call write_internal and add some nice error context
        self.write_internal(offset, buf)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.block_device.flush()
    }

    fn len(&mut self) -> Result<u64, Self::Error> {
        self.block_device.count()
            .map(|bc| bc.0 * size_of::<B::Block>() as u64)
    }
}

#[cfg(feature = "alloc")]
impl<S: StorageDevice + ?Sized> StorageDevice for alloc::boxed::Box<S> {
    type Error = S::Error;

    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Self::Error> {
        (**self).read(offset, buf)
    }

    fn write(&mut self, offset: u64, buf: &[u8]) -> Result<(), Self::Error> {
        (**self).write(offset, buf)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        (**self).flush()
    }

    fn len(&mut self) -> Result<u64, Self::Error> {
        (**self).len()
    }
}

#[cfg(feature = "std")]
impl StorageDevice for std::fs::File {
    type Error = std::io::Error;

    /// Read the data at the given ``offset`` in the storage device into a given buffer.
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Self::Error> {
        use std::io::{Read, Seek};

        self.seek(std::io::SeekFrom::Start(offset))
            .and_then(|_| self.read_exact(buf))
    }

    /// Write the data from the given buffer at the given ``offset`` in the storage device.
    fn write(&mut self, offset: u64, buf: &[u8]) -> Result<(), Self::Error> {
        use std::io::{Seek, Write};

        self.seek(std::io::SeekFrom::Start(offset))
            .and_then(|_| self.write_all(buf))
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.sync_data()
    }

    /// Return the total size of the storage device.
    fn len(&mut self) -> Result<u64, Self::Error> {
        self.metadata()
            .map(|meta| meta.len())
    }
}

#[cfg(feature = "std")]
impl StorageDevice for &std::fs::File {
    type Error = std::io::Error;

    /// Read the data at the given ``offset`` in the storage device into a given buffer.
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Self::Error> {
        use std::io::{Read, Seek};

        self.seek(std::io::SeekFrom::Start(offset))
            .and_then(|_| self.read_exact(buf))
    }

    /// Write the data from the given buffer at the given ``offset`` in the storage device.
    fn write(&mut self, offset: u64, buf: &[u8]) -> Result<(), Self::Error> {
        use std::io::{Seek, Write};

        self.seek(std::io::SeekFrom::Start(offset))
            .and_then(|_| self.write_all(buf))
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.sync_data()
    }

    /// Return the total size of the storage device.
    fn len(&mut self) -> Result<u64, Self::Error> {
        self.metadata()
            .map(|meta| meta.len())
    }
}

#[cfg(test)]
mod test {
    use crate::block_device::{BlockIndex, BlockCount, BlockDevice};
    use crate::storage_device::{StorageDevice, StorageBlockDevice};
    use crate::block::Block;

    /// Block device that when read from returns blocks filled with for every byte
    /// their index in the block,
    /// and when wrote to checks that for every byte it's its index in the block.
    ///
    /// Used to debug that our reading logic for unaligned buffers is correct.
    #[derive(Debug)]
    struct DbgBlockDevice;

    impl BlockDevice for DbgBlockDevice {
        type Block = crate::block::Block;
        type Error = ();

        fn read(&mut self, blocks: &mut [Block], _index: BlockIndex) -> Result<(), Self::Error> {
            assert_eq!(((&blocks[0]) as *const Block as usize) % core::mem::align_of::<Block>(), 0, "DbgBlockDevice got a misaligned block");
            for block in blocks.iter_mut() {
                for (index, byte) in block.contents.iter_mut().enumerate()  {
                    *byte = index as u8 // overflows once per block
                }
            }
            Ok(())
        }

        fn write(&mut self, blocks: &[Block], _index: BlockIndex) -> Result<(), Self::Error> {
            assert_eq!(((&blocks[0]) as *const Block as usize) % core::mem::align_of::<Block>(), 0, "DbgBlockDevice got a misaligned block");
            for block in blocks.iter() {
                for (idx, byte) in block.contents.iter().enumerate() {
                    if *byte != (idx as u8) {
                        return Err(())
                    }
                }
            }
            Ok(())
        }

        fn count(&mut self) -> Result<BlockCount, Self::Error> {
            Ok(BlockCount(8))
        }
    }

    /// Block device that when read from returns blocks filled with their block index in every byte,
    /// and when wrote to checks that for every byte it's its index in the block.
    ///
    /// Used to debug that our reading logic for unaligned buffers is correct.
    #[derive(Debug)]
    struct DbgIdxBlockDevice;

    impl BlockDevice for DbgIdxBlockDevice {
        type Block = crate::block::Block;
        type Error = ();

        fn read(&mut self, blocks: &mut [Block], index: BlockIndex) -> Result<(), Self::Error> {
            assert_eq!(((&blocks[0]) as *const Block as usize) % core::mem::align_of::<Block>(), 0, "DbgIdxBlockDevice got a misaligned block");
            for (i, block) in blocks.iter_mut().enumerate() {
                for byte in block.contents.iter_mut() {
                    *byte = (i as u64 + index.0) as u8
                }
            }
            Ok(())
        }

        fn write(&mut self, blocks: &[Block], index: BlockIndex) -> Result<(), Self::Error> {
            assert_eq!(((&blocks[0]) as *const Block as usize) % core::mem::align_of::<Block>(), 0, "DbgIdxBlockDevice got a misaligned block");
            for (i, block) in blocks.iter().enumerate() {
                for byte in block.contents.iter() {
                    if *byte != (i as u64 + index.0) as u8 {
                        return Err(())
                    }
                }
            }
            Ok(())
        }

        fn count(&mut self) -> Result<BlockCount, Self::Error> {
            Ok(BlockCount(8))
        }
    }

    /// An aligned buffer.
    ///
    /// To get a misaligned buffer from this, just do `align_buf.buf[1..]`.
    #[repr(C, align(8))]
    struct AlignedBuf {
        buf: [u8; 4096]
    }

    #[test]
    fn check_dbg_block_device_aligned() {
        let mut storage_dev = StorageBlockDevice::new(DbgBlockDevice);
        let mut aligned = AlignedBuf { buf: [0x55; 4096] };
        let aligned_buf = &mut aligned.buf[0..];
        assert_eq!((&aligned_buf[0] as *const u8 as usize) % 2, 0, "buf is not actually aligned");

        {
            StorageDevice::read(&mut storage_dev, 0, aligned_buf)
                .expect("reading failed");

            for (index, byte) in aligned_buf.iter().enumerate() {
                assert_eq!(*byte, index as u8, "failed checking block content. Index: {:02x}, Your buffer:\n{:02x?}", index, &aligned_buf);
            }

            // writing back should also work
            StorageDevice::write(&mut storage_dev, 0, aligned_buf)
                .expect("writing failed");
        }

        let mut storage_dev = StorageBlockDevice::new(DbgIdxBlockDevice);
        {
            StorageDevice::read(&mut storage_dev, 0, aligned_buf)
                .expect("reading failed");

            // writing back to check
            StorageDevice::write(&mut storage_dev, 0, aligned_buf)
                .expect("writing failed");
        }
    }


    #[test]
    fn check_dbg_block_device_misaligned() {
        let mut storage_dev = StorageBlockDevice::new(DbgBlockDevice);
        let mut aligned_buf = AlignedBuf { buf: [0x55; 4096] };
        let misaligned_buf = &mut aligned_buf.buf[1..];
        assert_eq!((&misaligned_buf[0] as *const u8 as usize) % 2, 1, "buf is not actually misaligned");

        {
            StorageDevice::read(&mut storage_dev, 0, misaligned_buf)
                .expect("reading failed");

            for (index, byte) in misaligned_buf.iter().enumerate() {
                assert_eq!(*byte, index as u8, "failed checking block content. Index: {:02x}, Your buffer:\n{:02x?}", index, &misaligned_buf);
            }

            // writing back should also work
            StorageDevice::write(&mut storage_dev, 0, misaligned_buf)
                .expect("writing failed");
        }
        let mut storage_dev = StorageBlockDevice::new(DbgIdxBlockDevice);
        {
            StorageDevice::read(&mut storage_dev, 0, misaligned_buf)
                .expect("reading failed");

            // writing back to check
            StorageDevice::write(&mut storage_dev, 0, misaligned_buf)
                .expect("writing failed");
        }
    }

    #[test]
    fn check_dbg_block_device_aligned_offset_8() {
        let mut storage_dev = StorageBlockDevice::new(DbgBlockDevice);
        let mut aligned = AlignedBuf { buf: [0x55; 4096] };
        let aligned_buf = &mut aligned.buf[0..];
        assert_eq!((&aligned_buf[0] as *const u8 as usize) % 2, 0, "buf is not actually aligned");

        {
            StorageDevice::read(&mut storage_dev, 8, aligned_buf)
                .expect("reading failed");

            for (index, byte) in aligned_buf.iter().enumerate() {
                assert_eq!(*byte, (index + 8) as u8, "failed checking block content. Index: {:02x}, Your buffer:\n{:02x?}", index, &aligned_buf);
            }

            // writing back should also work
            StorageDevice::write(&mut storage_dev, 8, aligned_buf)
                .expect("writing failed");
        }
        let mut storage_dev = StorageBlockDevice::new(DbgIdxBlockDevice);
        {
            StorageDevice::read(&mut storage_dev, 8, aligned_buf)
                .expect("reading failed");

            // writing back to check
            StorageDevice::write(&mut storage_dev, 8, aligned_buf)
                .expect("writing failed");
        }
    }

    #[test]
    fn check_dbg_block_device_misaligned_offset_8() {
        let mut storage_dev = StorageBlockDevice::new(DbgBlockDevice);
        let mut aligned_buf = AlignedBuf { buf: [0x55; 4096] };
        let misaligned_buf = &mut aligned_buf.buf[1..];
        assert_eq!((&misaligned_buf[0] as *const u8 as usize) % 2, 1, "buf is not actually misaligned");

        {
            StorageDevice::read(&mut storage_dev, 8, misaligned_buf)
                .expect("reading failed");

            for (index, byte) in misaligned_buf.iter().enumerate() {
                assert_eq!(*byte, (index + 8) as u8, "failed checking block content. Index: {:02x}, Your buffer:\n{:02x?}", index, &misaligned_buf);
            }

            // writing back should also work
            StorageDevice::write(&mut storage_dev, 8, misaligned_buf)
                .expect("writing failed");
        }
        let mut storage_dev = StorageBlockDevice::new(DbgIdxBlockDevice);
        {
            StorageDevice::read(&mut storage_dev, 8, misaligned_buf)
                .expect("reading failed");

            // writing back to check
            StorageDevice::write(&mut storage_dev, 8, misaligned_buf)
                .expect("writing failed");
        }
    }

    #[test]
    fn check_dbg_block_device_aligned_offset_7() {
        let mut storage_dev = StorageBlockDevice::new(DbgBlockDevice);
        let mut aligned = AlignedBuf { buf: [0x55; 4096] };
        let aligned_buf = &mut aligned.buf[0..];
        assert_eq!((&aligned_buf[0] as *const u8 as usize) % 2, 0, "buf is not actually aligned");

        {
            StorageDevice::read(&mut storage_dev, 7, aligned_buf)
                .expect("reading failed");

            for (index, byte) in aligned_buf.iter().enumerate() {
                assert_eq!(*byte, (index + 7) as u8, "failed checking block content. Index: {:02x}, Your buffer:\n{:02x?}", index, &aligned_buf);
            }

            // writing back should also work
            StorageDevice::write(&mut storage_dev, 7, aligned_buf)
                .expect("writing failed");
        }
        let mut storage_dev = StorageBlockDevice::new(DbgIdxBlockDevice);
        {
            StorageDevice::read(&mut storage_dev, 7, aligned_buf)
                .expect("reading failed");

            // writing back to check
            StorageDevice::write(&mut storage_dev, 7, aligned_buf)
                .expect("writing failed");
        }
    }

    #[test]
    fn check_dbg_block_device_misaligned_offset_7() {
        let mut storage_dev = StorageBlockDevice::new(DbgBlockDevice);
        let mut aligned_buf = AlignedBuf { buf: [0x55; 4096] };
        let misaligned_buf = &mut aligned_buf.buf[1..];
        assert_eq!((&misaligned_buf[0] as *const u8 as usize) % 2, 1, "buf is not actually misaligned");

        {
            StorageDevice::read(&mut storage_dev, 7, misaligned_buf)
                .expect("reading failed");

            for (index, byte) in misaligned_buf.iter().enumerate() {
                assert_eq!(*byte, (index + 7) as u8, "failed checking block content. Index: {:02x}, Your buffer:\n{:02x?}", index, &misaligned_buf);
            }

            // writing back should also work
            StorageDevice::write(&mut storage_dev, 7, misaligned_buf)
                .expect("writing failed");
        }
        let mut storage_dev = StorageBlockDevice::new(DbgIdxBlockDevice);
        {
            StorageDevice::read(&mut storage_dev, 7, misaligned_buf)
                .expect("reading failed");

            // writing back to check
            StorageDevice::write(&mut storage_dev, 7, misaligned_buf)
                .expect("writing failed");
        }
    }

    #[test]
    fn check_small_read_write() {
        let mut storage_dev = StorageBlockDevice::new(DbgBlockDevice);
        let mut buf = [0x55; 0x80];

        {
            StorageDevice::read(&mut storage_dev, 0x400, &mut buf)
                .expect("reading failed");

            // writing back should also work
            StorageDevice::write(&mut storage_dev, 0x400, &mut buf)
                .expect("writing failed");
        }
    }
}
