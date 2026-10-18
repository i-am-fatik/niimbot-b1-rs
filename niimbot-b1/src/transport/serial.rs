use std::future::{self, Future};
use std::io::{self, Read, Write};
use std::time::Duration;

use serialport::SerialPort;

use crate::error::Error;
use crate::transport::Transport;

const BAUD_RATE: u32 = 115_200;
const MACOS_PORT_PREFIX: &str = "cu.usbmodemB1_";
const LINUX_PORT_PREFIX: &str = "ttyACM";
const READ_CHUNK: usize = 512;

/// USB serial link.
#[derive(Debug)]
pub struct SerialTransport {
    port: Box<dyn SerialPort>,
}

impl SerialTransport {
    /// Opens the B1 at `path`, or the first one [`find_port`] finds when `path` is `None`.
    ///
    /// # Errors
    ///
    /// When no printer is found or the port does not open.
    pub fn open(path: Option<&str>) -> Result<Self, Error> {
        let path = match path {
            Some(path) => path.to_owned(),
            None => find_port()?,
        };
        let port = serialport::new(&path, BAUD_RATE)
            .open()
            .map_err(Error::transport)?;
        Ok(Self { port })
    }
}

/// Returns the first B1 serial port: `/dev/cu.usbmodemB1_<serial>` on macOS, `/dev/ttyACM*`
/// on Linux.
///
/// # Errors
///
/// When `/dev` cannot be listed or holds no such port.
pub fn find_port() -> Result<String, Error> {
    let mut names: Vec<String> = std::fs::read_dir("/dev")
        .map_err(Error::transport)?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect();
    names.sort();
    [MACOS_PORT_PREFIX, LINUX_PORT_PREFIX]
        .iter()
        .find_map(|prefix| names.iter().find(|name| name.starts_with(prefix)))
        .map(|name| format!("/dev/{name}"))
        .ok_or_else(|| Error::transport("B1 printer not found on USB"))
}

impl Transport for SerialTransport {
    fn write(&mut self, packet: &[u8]) -> impl Future<Output = Result<(), Error>> {
        future::ready(self.port.write_all(packet).map_err(Error::transport))
    }

    fn read(&mut self, timeout: Duration) -> impl Future<Output = Result<Vec<u8>, Error>> {
        future::ready(self.read_blocking(timeout))
    }
}

impl SerialTransport {
    fn read_blocking(&mut self, timeout: Duration) -> Result<Vec<u8>, Error> {
        self.port.set_timeout(timeout).map_err(Error::transport)?;
        let mut chunk = [0; READ_CHUNK];
        match self.port.read(&mut chunk) {
            Ok(len) => Ok(chunk[..len].to_vec()),
            Err(error) if error.kind() == io::ErrorKind::TimedOut => Ok(Vec::new()),
            Err(error) => Err(Error::transport(error)),
        }
    }
}
