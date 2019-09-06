
use crate::block_device::{BlockDevice, BlockIndex, BlockCount};

/// A BlockDevice that reduces device accesses by keeping the most recently used blocks in a cache.
///
/// It will keep track of which blocks are dirty, and will only write those ones to device when
/// flushing, or when they are evicted from the cache.
///
/// When a CachedBlockDevice is dropped, it flushes its cache.
pub struct CachedBlockDevice<B: BlockDevice> {
    /// The inner block device.
    block_device: B,

    /// The LRU cache.
    lru_cache: lru::LruCache<BlockIndex, CachedBlock<B::Block>>,
}

/// Represent a cached block in the LRU cache.
struct CachedBlock<B> {
    /// Bool indicating whether this block should be written to device when flushing.
    dirty: bool,
    /// The data of this block.
    data: B,
}

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
    pub fn flush(&mut self) -> Result<(), B::Error> {
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

impl<B: BlockDevice> Drop for CachedBlockDevice<B> {
    /// Dropping a CachedBlockDevice flushes it.
    ///
    /// If a device write fails, it is silently ignored.
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

impl<B: BlockDevice> BlockDevice for CachedBlockDevice<B> {
    type Block = B::Block;
    type Error = B::Error;

    /// Attempts to fill `blocks` with blocks found in the cache, and will fetch them from device if it can't.
    ///
    /// Will update the access time of every block involved.
    fn read(&mut self, blocks: &mut [B::Block], index: BlockIndex) -> Result<(), Self::Error> {
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
    fn write(&mut self, blocks: &[B::Block], index: BlockIndex) -> Result<(), Self::Error> {
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

    fn count(&mut self) -> Result<BlockCount, Self::Error> {
        self.block_device.count()
    }
}
