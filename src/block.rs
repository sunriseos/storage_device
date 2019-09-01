pub use plain::Plain;
use core::ops::{Deref, DerefMut};

/// Represent a certain amount of data from a block device.
#[derive(Clone, Copy)]
#[repr(C, align(2))]
pub struct Block {
    /// The actual storage of the block.
    pub contents: [u8; Block::LEN],
}

// Safety: A Block is just a wrapper around a byte array. There's no padding.
unsafe impl Plain for Block {}

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
