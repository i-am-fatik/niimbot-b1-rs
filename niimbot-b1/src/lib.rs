//! Client for the Niimbot B1 thermal label printer.
//!
//! The printer speaks one protocol over USB serial and Bluetooth LE. [`Printer`] wraps it for
//! status, RFID roll info, printing a [`Bitmap`] and flashing a known [`Firmware`], over any
//! [`Transport`]. Transports for
//! native USB serial, native Bluetooth LE, and the browser's Web Serial and Web Bluetooth ship
//! behind the `serial`, `ble` and `web` features.
//!
//! # Examples
//!
//! ```no_run
//! # #[cfg(feature = "serial")]
//! # async fn demo() -> Result<(), niimbot_b1::Error> {
//! use niimbot_b1::{Bitmap, PrintOptions, Printer, Raster, SerialTransport};
//!
//! let mut printer = Printer::new(SerialTransport::open(None)?);
//! println!("{}", printer.heartbeat().await?);
//! let label = Bitmap::from_rgba(384, 240, &vec![255; 384 * 240 * 4], Raster::Threshold);
//! printer.print(&label, PrintOptions::default()).await?;
//! # Ok(())
//! # }
//! ```

mod bitmap;
mod error;
mod firmware;
mod patch;
mod printer;
mod protocol;
mod transport;

pub use bitmap::{Bitmap, Raster};
pub use error::Error;
pub use firmware::Firmware;
pub use patch::{PatchError, Patched};
pub use printer::{Heartbeat, PrintOptions, PrintStatus, Printer, RfidInfo};
pub use protocol::{LabelType, PRINTHEAD_PX};
#[cfg(feature = "ble")]
pub use transport::BleTransport;
pub use transport::Transport;
#[cfg(feature = "web")]
pub use transport::WebTransport;
#[cfg(feature = "serial")]
pub use transport::{SerialTransport, find_port};
