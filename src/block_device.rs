
use core::ops::{Deref, DerefMut};
use plain::Plain;
use core::fmt::Debug;
#[cfg(feature = "std")]
use core::mem::size_of;
#[cfg(feature = "std")]
use crate::block::Block;

/// Represent the position of a block on a block device.
#[derive(Debug, Copy, Clone, Hash, PartialOrd, PartialEq, Ord, Eq)]
pub struct BlockIndex(pub u64);

/// Represent the count of blocks that a block device hold.
#[derive(Debug, Copy, Clone)]
pub struct BlockCount(pub u64);

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

    /// Error type returned by this block device when an operation fails
    type Error: Debug;

    /// Read blocks from the block device starting at the given ``index``.
    fn read(&mut self, blocks: &mut [Self::Block], index: BlockIndex) -> Result<(), Self::Error>;

    /// Write blocks to the block device starting at the given ``index``.
    fn write(&mut self, blocks: &[Self::Block], index: BlockIndex) -> Result<(), Self::Error>;

    /// Return the amount of blocks hold by the block device.
    fn count(&mut self) -> Result<BlockCount, Self::Error>;
}

#[cfg(feature = "std")]
impl BlockDevice for std::fs::File {
    type Block = Block;

    type Error = std::io::Error;

    /// Seeks to the appropriate position, and reads block by block.
    fn read(&mut self, blocks: &mut [Block], index: BlockIndex) -> Result<(), Self::Error> {
        use std::io::{Read, Seek};

        self.seek(std::io::SeekFrom::Start(index.0 * size_of::<Self::Block>() as u64))?;

        let blocks_as_slice = unsafe {
            // safe: the bounds on Block explicitly guarantees us this is safe.
            core::slice::from_raw_parts_mut(blocks as *mut _ as *mut u8, blocks.len() * size_of::<Self::Block>())
        };

        self.read_exact(blocks_as_slice)
    }

    /// Seeks to the appropriate position, and writes block by block.
    fn write(&mut self, blocks: &[Block], index: BlockIndex) -> Result<(), Self::Error> {
        use std::io::{Seek, Write};

        self.seek(std::io::SeekFrom::Start(index.0 * size_of::<Self::Block>() as u64))?;

        let blocks_as_slice = unsafe {
            // safe: the bounds on Block explicitly guarantees us this is safe.
            core::slice::from_raw_parts(blocks as *const _ as *const u8, blocks.len() * size_of::<Self::Block>())
        };

        self.write_all(blocks_as_slice)
    }

    fn count(&mut self) -> Result<BlockCount, Self::Error> {
        self.metadata()
            .map(|meta| BlockCount(meta.len() / (size_of::<Block>() as u64)))
    }
}
