//! Block device IO error type

use crate::block_device::{BlockIndex, BlockCount};

/// The operation that was being carried on when the fail happened. Read or Write.
#[derive(Debug, Copy, Clone)]
pub enum IoOperation {
    /// We were trying to read from the block device when the fail happened.
    Read,
    /// We were trying to write to the block device when the fail happened.
    Write
}

/// Error type to give some context when an IO operation failed on a block device.
/// Contains a representation of the request that failed.
#[derive(Debug, Copy, Clone)]
pub struct BlockDeviceError {
    /// Whether we were trying to read from or write to the block device.
    pub operation: IoOperation,
    /// The destination of our request on the disk.
    pub start_index: BlockIndex,
    /// The number of blocks of our request.
    pub block_count: BlockCount,
}

/// Error type to give some context when an IO operation failed on a storage device.
/// Contains a representation of the request that failed.
/// If the storage device is backup up by a block device, the parent [BlockDeviceError] is stored.
#[derive(Debug, Copy, Clone)]
pub struct IoError {
    /// Whether we were trying to read from or write to the block device.
    pub operation: IoOperation,
    /// The destination of our request on the disk.
    pub offset: u64,
    /// The total length of our request.
    pub len: usize,
    /// If this device is backed by [BlockDevice], the error that caused us to fail.
    pub block_device_error: Option<BlockDeviceError>
}

/// Result where the error type is [IoError].
pub type IoResult<T> = core::result::Result<T, IoError>;
