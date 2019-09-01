pub use plain::Plain;
use core::ops::{Deref, DerefMut};

/// Represent a block error.
#[derive(Debug)]
pub enum BlockError {
    /// Read error.
    ReadError,

    /// Write error.
    WriteError,

    /// Unknown error.
    Unknown,
}

/// Represent a block result.
pub type BlockResult<T> = core::result::Result<T, BlockError>;

/// Represent a certain amount of data from a block device.
#[derive(Clone, Copy)]
#[repr(C, align(2))]
pub struct Block {
    /// The actual storage of the block.
    pub contents: [u8; Block::LEN],
}

// Safety: A Block is just a wrapper around a byte array. There's no padding.
unsafe impl Plain for Block {}

/// Represent the position of a block on a block device.
#[derive(Debug, Copy, Clone, Hash, PartialOrd, PartialEq, Ord, Eq)]
pub struct BlockIndex(pub u64);

/// Represent the count of blocks that a block device hold.
#[derive(Debug, Copy, Clone)]
pub struct BlockCount(pub u64);

impl BlockCount {
    /// Get the block count as a raw bytes count.
    pub fn into_bytes_count(self) -> u64 {
        self.0 * Block::LEN_U64
    }
}

impl Block {
    /// The size of a block in bytes.
    pub const LEN: usize = 512;

    /// The size of a block in bytes as a 64 bits unsigned value.
    pub const LEN_U64: u64 = Self::LEN as u64;

    /// Create a new block instance.
    pub fn new() -> Block {
        Block::default()
    }

    /// Return the content of the block.
    pub fn as_contents(&self) -> [u8; Block::LEN] {
        self.contents
    }
}

impl Default for Block {
    fn default() -> Self {
        Block {
            contents: [0u8; Self::LEN],
        }
    }
}

impl Deref for Block {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.contents
    }
}

impl DerefMut for Block {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.contents
    }
}

impl BlockIndex {
    /// Convert the block index into an offset in bytes.
    pub fn into_offset(self) -> u64 {
        self.0 * Block::LEN_U64
    }
}

impl BlockCount {
    /// Convert the block count into a size in bytes.
    pub fn into_size(self) -> u64 {
        self.0 * Block::LEN_U64
    }
}

/// Represent a device holding blocks.
///
/// This trait is agnostic over the size of block that is being held. The user
/// is free (and encouraged) to define its own block type to use with a
/// BlockDevice.
pub trait BlockDevice: core::fmt::Debug {
    /// Represents a Block that this BlockDevice can read or write to. A Block
    /// is generally a byte array of a certain fixed size. It might also have
    /// alignment constraints.
    ///
    /// For instance, an AHCI block would be defined as:
    ///
    /// ```rust
    /// use plain::Plain;
    /// use std::ops::{Deref, DerefMut};
    ///
    /// #[repr(C, align(2))]
    /// #[derive(Clone, Copy)]
    /// struct AhciBlock([u8; 512]);
    ///
    /// // Safety: Safe because AhciBlock is just a Repr(c) wrapper around a
    /// // byte array, which respects all of plain's invariants already.
    /// unsafe impl Plain for AhciBlock {}
    ///
    /// impl Default for AhciBlock {
    ///     fn default() -> AhciBlock {
    ///         AhciBlock([0; 512])
    ///     }
    /// }
    ///
    /// impl Deref for AhciBlock {
    ///     type Target = [u8];
    ///     fn deref(&self) -> &[u8] {
    ///         &self.0[..]
    ///     }
    /// }
    ///
    /// impl DerefMut for AhciBlock {
    ///     fn deref_mut(&mut self) -> &mut [u8] {
    ///         &mut self.0[..]
    ///     }
    /// }
    /// ```
    ///
    /// # Invariants
    ///
    /// There are several invariants Block must respect in order to make
    /// BlockDevice safe to use:
    ///
    /// 1. Block MUST have no padding bytes. In other words, the size
    ///    of all its component MUST be equal to its size_of::<Self>.
    ///
    ///    This comes as an additional invariant to all the other Plain
    ///    requires, and is necessary to be able to cast a Block to an u8 array,
    ///    which is internally done by the StorageDevice implementation for
    ///    a BlockDevice.
    /// 2. Its Deref implementation MUST deref to its internal byte array.
    // TODO: Add a trait to encode Block's invariants
    // BODY: Currently, Block's invariants aren't properly encoded in the type
    // BODY: system. They are merely documented in BlockDevice's documentation.
    // BODY: This is, obviously, absolutely terrible. We need to come up with
    // BODY: the correct set of functions and rules necessary for a proper
    // BODY: Block implementation that makes this whole crate safe to use.
    type Block: Plain + Copy + Default + Deref<Target=[u8]> + DerefMut;

    /// Read blocks from the block device starting at the given ``index``.
    fn read(&mut self, blocks: &mut [Self::Block], index: BlockIndex) -> BlockResult<()>;

    /// Write blocks to the block device starting at the given ``index``.
    fn write(&mut self, blocks: &[Self::Block], index: BlockIndex) -> BlockResult<()>;

    /// Return the amount of blocks hold by the block device.
    fn count(&mut self) -> BlockResult<BlockCount>;
}

/// A BlockDevice that reduces device accesses by keeping the most recently used blocks in a cache.
///
/// It will keep track of which blocks are dirty, and will only write those ones to device when
/// flushing, or when they are evicted from the cache.
///
/// When a CachedBlockDevice is dropped, it flushes its cache.
#[cfg(any(
    feature = "cached-block-device",
    feature = "cached-block-device-nightly"
))]
pub struct CachedBlockDevice<B: BlockDevice> {
    /// The inner block device.
    block_device: B,

    /// The LRU cache.
    lru_cache: lru::LruCache<BlockIndex, CachedBlock<B::Block>>,
}

/// Represent a cached block in the LRU cache.
#[cfg(any(
    feature = "cached-block-device",
    feature = "cached-block-device-nightly"
))]
struct CachedBlock<B> {
    /// Bool indicating whether this block should be written to device when flushing.
    dirty: bool,
    /// The data of this block.
    data: B,
}

#[cfg(any(
    feature = "cached-block-device",
    feature = "cached-block-device-nightly"
))]
impl<B> core::fmt::Debug for CachedBlockDevice<B>
where
    B: BlockDevice,
{
    fn fmt(&self, fmt: &mut core::fmt::Formatter) -> Result<(), core::fmt::Error> {
        fmt.debug_struct("CachedBlockDevice")
            .field("block_device", &self.block_device)
            .finish()
    }
}

#[cfg(any(
    feature = "cached-block-device",
    feature = "cached-block-device-nightly"
))]
impl<B: BlockDevice> CachedBlockDevice<B> {
    /// Creates a new CachedBlockDevice that wraps `device`, and can hold at most `cap` blocks in cache.
    pub fn new(device: B, cap: usize) -> CachedBlockDevice<B> {
        CachedBlockDevice {
            block_device: device,
            lru_cache: lru::LruCache::new(cap),
        }
    }

    /// Writes every dirty cached block to device.
    ///
    /// Note that this will not empty the cache, just perform device writes
    /// and update dirty blocks as now non-dirty.
    ///
    /// This function has no effect on lru order.
    pub fn flush(&mut self) -> BlockResult<()> {
        for (index, block) in self.lru_cache.iter_mut() {
            if block.dirty {
                self.block_device
                    .write(core::slice::from_ref(&block.data), *index)?;
                block.dirty = false;
            }
        }
        Ok(())
    }
}

#[cfg(any(
    feature = "cached-block-device",
    feature = "cached-block-device-nightly"
))]
impl<B: BlockDevice> Drop for CachedBlockDevice<B> {
    /// Dropping a CachedBlockDevice flushes it.
    ///
    /// If a device write fails, it is silently ignored.
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(any(
    feature = "cached-block-device",
    feature = "cached-block-device-nightly"
))]
impl<B: BlockDevice> BlockDevice for CachedBlockDevice<B> {
    type Block = B::Block;

    /// Attempts to fill `blocks` with blocks found in the cache, and will fetch them from device if it can't.
    ///
    /// Will update the access time of every block involved.
    fn read(&mut self, blocks: &mut [B::Block], index: BlockIndex) -> BlockResult<()> {
        // check if we can satisfy the request only from what we have in cache
        let mut fully_cached = true;
        if blocks.len() > self.lru_cache.len() {
            // requested more blocks that cache is holding
            fully_cached = false
        } else {
            // check each block is found in the cache
            for i in 0..blocks.len() {
                if !self.lru_cache.contains(&BlockIndex(index.0 + i as u64)) {
                    fully_cached = false;
                    break;
                }
            }
        }

        if !fully_cached {
            // we must read from device
            self.block_device.read(blocks, index)?
        }

        // update from/to cache
        for (i, block) in blocks.iter_mut().enumerate() {
            if let Some(cached_block) = self.lru_cache.get(&BlockIndex(index.0 + i as u64)) {
                // block was found in cache, its access time was updated.
                if fully_cached || cached_block.dirty {
                    // fully_cached: block[i] is uninitialized, copy it from cache.
                    // dirty:        block[i] is initialized from device if !fully_cached,
                    //               but we hold a newer dirty version in cache, overlay it.
                    *block = cached_block.data;
                }
            } else {
                // add the block we just read to the cache.
                // if cache is full, flush its lru entry
                if self.lru_cache.len() == self.lru_cache.cap() {
                    let (evicted_index, evicted_block) = self.lru_cache.pop_lru().unwrap();
                    if evicted_block.dirty {
                        self.block_device
                            .write(core::slice::from_ref(&evicted_block.data), evicted_index)?;
                    }
                }
                let new_cached_block = CachedBlock {
                    dirty: false,
                    data: *block,
                };
                self.lru_cache
                    .put(BlockIndex(index.0 + i as u64), new_cached_block);
            }
        }
        Ok(())
    }

    /// Adds dirty blocks to the cache.
    ///
    /// If the block was already present in the cache, it will simply be updated.
    ///
    /// When the cache is full, least recently used blocks will be evicted and written to device.
    /// This operation may fail, and this function will return an error when it happens.
    fn write(&mut self, blocks: &[B::Block], index: BlockIndex) -> BlockResult<()> {
        if blocks.len() < self.lru_cache.cap() {
            for (i, block) in blocks.iter().enumerate() {
                let new_block = CachedBlock {
                    dirty: true,
                    data: *block,
                };
                // add it to the cache
                // if cache is full, flush its lru entry
                if self.lru_cache.len() == self.lru_cache.cap() {
                    let (evicted_index, evicted_block) = self.lru_cache.pop_lru().unwrap();
                    if evicted_block.dirty {
                        self.block_device
                            .write(core::slice::from_ref(&evicted_block.data), evicted_index)?;
                    }
                }
                self.lru_cache
                    .put(BlockIndex(index.0 + i as u64), new_block);
            }
        } else {
            // we're performing a big write, that will evict all cache blocks.
            // evict it in one go, and repopulate with the first `cap` blocks from `blocks`.
            for (evicted_index, evicted_block) in self.lru_cache.iter() {
                if evicted_block.dirty
                    // if evicted block is `blocks`, don't bother writing it as we're about to re-write it anyway.
                    && !(index >= *evicted_index && index < BlockIndex(evicted_index.0 + blocks.len() as u64))
                {
                    self.block_device
                        .write(core::slice::from_ref(&evicted_block.data), *evicted_index)?;
                }
            }
            // write in one go
            self.block_device.write(blocks, index)?;
            // add first `cap` blocks to cache
            for (i, block) in blocks.iter().take(self.lru_cache.cap()).enumerate() {
                self.lru_cache.put(
                    BlockIndex(index.0 + i as u64),
                    CachedBlock {
                        dirty: false,
                        data: *block,
                    },
                )
            }
        }
        Ok(())
    }

    fn count(&mut self) -> BlockResult<BlockCount> {
        self.block_device.count()
    }
}

#[cfg(feature = "std")]
impl BlockDevice for std::fs::File {
    type Block = Block;

    /// Seeks to the appropriate position, and reads block by block.
    fn read(&mut self, blocks: &mut [Block], index: BlockIndex) -> BlockResult<()> {
        use std::io::{Read, Seek};

        self.seek(std::io::SeekFrom::Start(index.into_offset()))
            .map_err(|_| BlockError::ReadError)?;
        for block in blocks.iter_mut() {
            self.read_exact(&mut block.contents)
                .map_err(|_| BlockError::ReadError)?;
        }
        Ok(())
    }

    /// Seeks to the appropriate position, and writes block by block.
    fn write(&mut self, blocks: &[Block], index: BlockIndex) -> BlockResult<()> {
        use std::io::{Seek, Write};

        self.seek(std::io::SeekFrom::Start(index.into_offset()))
            .map_err(|_| BlockError::ReadError)?;
        for block in blocks.iter() {
            self.write_all(&block.contents)
                .map_err(|_| BlockError::WriteError)?;
        }
        Ok(())
    }

    fn count(&mut self) -> BlockResult<BlockCount> {
        let num_blocks = self.metadata().map_err(|_| BlockError::Unknown)?.len() / (Block::LEN_U64);
        Ok(BlockCount(num_blocks))
    }
}

#[cfg(feature = "std")]
use crate::{StorageDevice, StorageDeviceError, StorageDeviceResult};

#[cfg(feature = "std")]
impl StorageDevice for std::fs::File {
    /// Read the data at the given ``offset`` in the storage device into a given buffer.
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> StorageDeviceResult<()> {
        use std::io::{Read, Seek};

        self.seek(std::io::SeekFrom::Start(offset))
            .map_err(|_| BlockError::ReadError)?;
        self.read_exact(buf).map_err(|_| BlockError::ReadError)?;
        Ok(())
    }

    /// Write the data from the given buffer at the given ``offset`` in the storage device.
    fn write(&mut self, offset: u64, buf: &[u8]) -> StorageDeviceResult<()> {
        use std::io::{Seek, Write};

        self.seek(std::io::SeekFrom::Start(offset))
            .map_err(|_| BlockError::WriteError)?;
        self.write_all(buf).map_err(|_| BlockError::WriteError)?;
        Ok(())
    }

    /// Return the total size of the storage device.
    fn len(&mut self) -> StorageDeviceResult<u64> {
        Ok(self
            .metadata()
            .map_err(|_| StorageDeviceError::Unknown)?
            .len())
    }
}

#[cfg(feature = "std")]
impl StorageDevice for &std::fs::File {
    /// Read the data at the given ``offset`` in the storage device into a given buffer.
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> StorageDeviceResult<()> {
        use std::io::{Read, Seek};

        self.seek(std::io::SeekFrom::Start(offset))
            .map_err(|_| BlockError::ReadError)?;
        self.read_exact(buf).map_err(|_| BlockError::ReadError)?;
        Ok(())
    }

    /// Write the data from the given buffer at the given ``offset`` in the storage device.
    fn write(&mut self, offset: u64, buf: &[u8]) -> StorageDeviceResult<()> {
        use std::io::{Seek, Write};

        self.seek(std::io::SeekFrom::Start(offset))
            .map_err(|_| BlockError::WriteError)?;
        self.write_all(buf).map_err(|_| BlockError::WriteError)?;
        Ok(())
    }

    /// Return the total size of the storage device.
    fn len(&mut self) -> StorageDeviceResult<u64> {
        Ok(self
            .metadata()
            .map_err(|_| StorageDeviceError::Unknown)?
            .len())
    }
}
