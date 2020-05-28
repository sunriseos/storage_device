//! A storage device representation to help filesystem development.
//! Designed to be used in filesystem drivers.

#![no_std]

#[cfg(feature = "std")]
extern crate std;
#[cfg(feature = "alloc")]
extern crate alloc;

pub mod block;
pub mod block_device;
pub mod storage_device;

#[cfg(feature = "cached-block-device")]
pub mod cached_block_device;

pub use crate::storage_device::StorageDevice;
pub use block_device::BlockDevice;
