use std::fmt;
use std::pin::Pin;
use std::time::Duration;

use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter,
    ValueNotification, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::{FutureExt, Stream, StreamExt};
use web_time::Instant;

use crate::error::Error;
use crate::transport::{BLE_NAME_PREFIX, BLE_PACKET_INTERVAL, BLE_SERVICE, Transport};

const SCAN_TIMEOUT: Duration = Duration::from_secs(10);
const SCAN_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Bluetooth LE link. Needs a running tokio runtime.
pub struct BleTransport {
    peripheral: Peripheral,
    characteristic: Characteristic,
    notifications: Pin<Box<dyn Stream<Item = ValueNotification> + Send>>,
}

impl BleTransport {
    /// Connects to the B1 whose address or platform id is `address`, or to the first device
    /// named `B1...` or advertising the B1 service when `address` is `None`.
    ///
    /// # Errors
    ///
    /// When there is no Bluetooth adapter, no printer shows up within the scan timeout, or the
    /// connection fails.
    pub async fn connect(address: Option<&str>) -> Result<Self, Error> {
        let manager = Manager::new().await.map_err(Error::transport)?;
        let adapter = manager
            .adapters()
            .await
            .map_err(Error::transport)?
            .into_iter()
            .next()
            .ok_or_else(|| Error::transport("no Bluetooth adapter"))?;
        adapter
            .start_scan(ScanFilter::default())
            .await
            .map_err(Error::transport)?;
        let found = find_printer(&adapter, address).await;
        adapter.stop_scan().await.map_err(Error::transport)?;
        let peripheral = found?;
        peripheral.connect().await.map_err(Error::transport)?;
        peripheral
            .discover_services()
            .await
            .map_err(Error::transport)?;
        let characteristic = peripheral
            .characteristics()
            .into_iter()
            .find(|c| {
                c.properties
                    .contains(CharPropFlags::NOTIFY | CharPropFlags::WRITE_WITHOUT_RESPONSE)
            })
            .ok_or_else(|| {
                Error::transport("printer has no notify and write-without-response characteristic")
            })?;
        peripheral
            .subscribe(&characteristic)
            .await
            .map_err(Error::transport)?;
        let notifications = peripheral.notifications().await.map_err(Error::transport)?;
        Ok(Self {
            peripheral,
            characteristic,
            notifications,
        })
    }

    /// Disconnects from the printer.
    ///
    /// # Errors
    ///
    /// When the platform fails to disconnect.
    pub async fn close(self) -> Result<(), Error> {
        self.peripheral.disconnect().await.map_err(Error::transport)
    }
}

async fn find_printer(adapter: &Adapter, address: Option<&str>) -> Result<Peripheral, Error> {
    let deadline = Instant::now() + SCAN_TIMEOUT;
    while Instant::now() < deadline {
        for peripheral in adapter.peripherals().await.map_err(Error::transport)? {
            let Some(properties) = peripheral.properties().await.map_err(Error::transport)? else {
                continue;
            };
            let is_printer = match address {
                Some(address) => [peripheral.id().to_string(), properties.address.to_string()]
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(address)),
                None => {
                    properties
                        .local_name
                        .as_deref()
                        .is_some_and(|name| name.starts_with(BLE_NAME_PREFIX))
                        || properties
                            .services
                            .iter()
                            .any(|service| service.to_string() == BLE_SERVICE)
                }
            };
            if is_printer {
                return Ok(peripheral);
            }
        }
        tokio::time::sleep(SCAN_POLL_INTERVAL).await;
    }
    Err(Error::transport(
        "B1 printer not found over Bluetooth, is it on and not connected elsewhere?",
    ))
}

impl Transport for BleTransport {
    async fn write(&mut self, packet: &[u8]) -> Result<(), Error> {
        tokio::time::sleep(BLE_PACKET_INTERVAL).await;
        self.peripheral
            .write(&self.characteristic, packet, WriteType::WithoutResponse)
            .await
            .map_err(Error::transport)
    }

    async fn read(&mut self, timeout: Duration) -> Result<Vec<u8>, Error> {
        let Ok(first) = tokio::time::timeout(timeout, self.notifications.next()).await else {
            return Ok(Vec::new());
        };
        let mut bytes = first
            .ok_or_else(|| Error::transport("Bluetooth notifications ended"))?
            .value;
        while let Some(Some(more)) = self.notifications.next().now_or_never() {
            bytes.extend(more.value);
        }
        Ok(bytes)
    }
}

impl fmt::Debug for BleTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BleTransport")
            .field("peripheral", &self.peripheral.id())
            .field("characteristic", &self.characteristic.uuid)
            .finish_non_exhaustive()
    }
}
