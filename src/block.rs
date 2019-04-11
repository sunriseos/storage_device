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
#[derive(Clone)]
pub struct Block {
    /// The actual storage of the block.
    pub contents: [u8; Block::LEN],
}

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

impl core::ops::Deref for Block {
    type Target = [u8; Block::LEN];
    fn deref(&self) -> &Self::Target {
        &self.contents
    }
}

impl core::ops::DerefMut for Block {
    fn deref_mut(&mut self) -> &mut [u8; Block::LEN] {
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
pub trait BlockDevice: Sized {
    /// Read blocks from the block device starting at the given ``index``.
    fn read(&self, blocks: &mut [Block], index: BlockIndex) -> BlockResult<()>;

    /// Write blocks to the block device starting at the given ``index``.
    fn write(&self, blocks: &[Block], index: BlockIndex) -> BlockResult<()>;

    /// Return the amount of blocks hold by the block device.
    fn count(&self) -> BlockResult<BlockCount>;
}

/// A BlockDevice that reduces device accesses by keeping the most recently used blocks in a cache.
///
/// It will keep track of which blocks are dirty, and will only write those ones to device when
/// flushing, or when they are evicted from the cache.
///
/// When a CachedBlockDevice is dropped, it flushes its cache.
#[cfg(feature = "cached-block-device")]
pub struct CachedBlockDevice<B: BlockDevice> {
    /// The inner block device.
    block_device: B,

    /// The LRU cache.
    lru_cache: spin::Mutex<lru::LruCache<BlockIndex, CachedBlock>>,
}

/// Represent a cached block in the LRU cache.
#[cfg(feature = "cached-block-device")]
struct CachedBlock {
    /// Bool indicating whether this block should be written to device when flushing.
    dirty: bool,
    /// The data of this block.
    data: Block,
}

#[cfg(feature = "cached-block-device")]
impl<B: BlockDevice> CachedBlockDevice<B> {
    /// Creates a new CachedBlockDevice that wraps `device`, and can hold at most `cap` blocks in cache.
    pub fn new(device: B, cap: usize) -> CachedBlockDevice<B> {
        CachedBlockDevice {
            block_device: device,
            lru_cache: spin::Mutex::new(lru::LruCache::new(cap)),
        }
    }

    /// Writes every dirty cached block to device.
    ///
    /// Note that this will not empty the cache, just perform device writes
    /// and update dirty blocks as now non-dirty.
    ///
    /// This function has no effect on lru order.
    pub fn flush(&self) -> BlockResult<()> {
        for (index, block) in self.lru_cache.lock().iter_mut() {
            if block.dirty {
                self.block_device
                    .raw_write(core::slice::from_ref(&block.data), *index)?;
                block.dirty = false;
            }
        }
        Ok(())
    }
}

#[cfg(feature = "cached-block-device")]
impl<B: BlockDevice> Drop for CachedBlockDevice<B> {
    /// Dropping a CachedBlockDevice flushes it.
    ///
    /// If a device write fails, it is silently ignored.
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(feature = "cached-block-device")]
impl<B: BlockDevice> BlockDevice for CachedBlockDevice<B> {
    /// Attempts to fill `blocks` with blocks found in the cache, and will fetch them from device if it can't.
    ///
    /// Will update the access time of every block involved.
    fn read(&self, blocks: &mut [Block], index: BlockIndex) -> BlockResult<()> {
        let mut lru = self.lru_cache.lock();
        // check if we can satisfy the request only from what we have in cache
        let mut fully_cached = true;
        if blocks.len() > lru.len() {
            // requested more blocks that cache is holding
            fully_cached = false
        } else {
            // check each block is found in the cache
            for i in 0..blocks.len() {
                if !lru.contains(&BlockIndex(index.0 + i as u64)) {
                    fully_cached = false;
                    break;
                }
            }
        }

        if !fully_cached {
            // we must read from device
            self.block_device.raw_read(blocks, index)?
        }

        // update from/to cache
        for (i, block) in blocks.iter_mut().enumerate() {
            if let Some(cached_block) = lru.get(&BlockIndex(index.0 + i as u64)) {
                // block was found in cache, its access time was updated.
                if fully_cached || cached_block.dirty {
                    // fully_cached: block[i] is uninitialized, copy it from cache.
                    // dirty:        block[i] is initialized from device if !fully_cached,
                    //               but we hold a newer dirty version in cache, overlay it.
                    *block = cached_block.data.clone();
                }
            } else {
                // add the block we just read to the cache.
                // if cache is full, flush its lru entry
                if lru.len() == lru.cap() {
                    let (evicted_index, evicted_block) = lru.pop_lru().unwrap();
                    if evicted_block.dirty {
                        self.block_device
                            .raw_write(core::slice::from_ref(&evicted_block.data), evicted_index)?;
                    }
                }
                let new_cached_block = CachedBlock {
                    dirty: false,
                    data: block.clone(),
                };
                lru.put(BlockIndex(index.0 + i as u64), new_cached_block);
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
    fn write(&self, blocks: &[Block], index: BlockIndex) -> BlockResult<()> {
        let mut lru = self.lru_cache.lock();

        if blocks.len() < lru.cap() {
            for (i, block) in blocks.iter().enumerate() {
                let new_block = CachedBlock {
                    dirty: true,
                    data: block.clone(),
                };
                // add it to the cache
                // if cache is full, flush its lru entry
                if lru.len() == lru.cap() {
                    let (evicted_index, evicted_block) = lru.pop_lru().unwrap();
                    if evicted_block.dirty {
                        self.block_device
                            .raw_write(core::slice::from_ref(&evicted_block.data), evicted_index)?;
                    }
                }
                lru.put(BlockIndex(index.0 + i as u64), new_block);
            }
        } else {
            // we're performing a big write, that will evict all cache blocks.
            // evict it in one go, and repopulate with the first `cap` blocks from `blocks`.
            for (evicted_index, evicted_block) in lru.iter() {
                if evicted_block.dirty
                    // if evicted block is `blocks`, don't bother writing it as we're about to re-write it anyway.
                    && !(index >= *evicted_index && index < BlockIndex(evicted_index.0 + blocks.len() as u64))
                {
                    self.block_device
                        .raw_write(core::slice::from_ref(&evicted_block.data), *evicted_index)?;
                }
            }
            // write in one go
            self.block_device.raw_write(blocks, index)?;
            // add first `cap` blocks to cache
            for (i, block) in blocks.iter().take(lru.cap()).enumerate() {
                lru.put(
                    BlockIndex(index.0 + i as u64),
                    CachedBlock {
                        dirty: false,
                        data: block.clone(),
                    },
                )
            }
        }
        Ok(())
    }

    fn count(&self) -> BlockResult<BlockCount> {
        self.block_device.count()
    }
}
