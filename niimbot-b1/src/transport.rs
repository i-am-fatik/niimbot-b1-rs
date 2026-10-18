use std::time::Duration;

use crate::error::Error;

#[cfg(feature = "ble")]
mod ble;
#[cfg(feature = "serial")]
mod serial;
#[cfg(feature = "web")]
mod web;

#[cfg(feature = "ble")]
pub use ble::BleTransport;
#[cfg(feature = "serial")]
pub use serial::{SerialTransport, find_port};
#[cfg(feature = "web")]
pub use web::WebTransport;

#[cfg(any(feature = "ble", feature = "web"))]
const BLE_SERVICE: &str = "e7810a71-73ae-499d-8c15-faa9aef0c3f2";
#[cfg(any(feature = "ble", feature = "web"))]
const BLE_NAME_PREFIX: &str = "B1";
#[cfg(any(feature = "ble", feature = "web"))]
const BLE_PACKET_INTERVAL: Duration = Duration::from_millis(10);

/// A byte link to the printer. Packets are the same on every link.
#[expect(
    async_fn_in_trait,
    reason = "browser futures are never Send, so no Send bound can be promised"
)]
pub trait Transport {
    /// Sends one whole packet.
    ///
    /// # Errors
    ///
    /// When the link fails.
    async fn write(&mut self, packet: &[u8]) -> Result<(), Error>;

    /// Waits up to `timeout` for bytes from the printer and returns what arrived, empty when
    /// nothing did.
    ///
    /// # Errors
    ///
    /// When the link fails.
    async fn read(&mut self, timeout: Duration) -> Result<Vec<u8>, Error>;
}
