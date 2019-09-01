//! A storage device representation to help filesystem development.
//! Designed to be used in filesystem drivers.

#![no_std]

#[cfg(feature = "std")]
extern crate std;
#[cfg(all(feature = "alloc", not(feature = "std")))]
extern crate alloc;
#[cfg(all(feature = "alloc", feature = "std"))]
extern crate std as alloc;

pub mod block;
pub mod block_device;
pub mod storage_device;
pub mod error;

#[cfg(any(
feature = "cached-block-device",
feature = "cached-block-device-nightly"
))]
pub mod cached_block_device;
