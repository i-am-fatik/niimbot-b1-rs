use std::cell::RefCell;
use std::mem;
use std::rc::Rc;
use std::time::Duration;

use js_sys::{JsString, Promise, Reflect, Uint8Array};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    BluetoothDevice, BluetoothLeScanFilterInit, BluetoothRemoteGattCharacteristic,
    ReadableStreamDefaultReader, RequestDeviceOptions, SerialOptions, SerialPort,
    WritableStreamDefaultWriter,
};
use web_time::Instant;

use crate::error::Error;
use crate::transport::{BLE_NAME_PREFIX, BLE_PACKET_INTERVAL, BLE_SERVICE, Transport};

const BAUD_RATE: u32 = 115_200;
const INBOX_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Browser link over Web Serial or Web Bluetooth. Both need a user gesture to open the
/// device chooser and a secure context, `https` or `localhost`.
#[derive(Debug)]
pub struct WebTransport {
    link: Link,
    inbox: Rc<Inbox>,
}

#[derive(Debug)]
enum Link {
    Serial {
        port: SerialPort,
        reader: ReadableStreamDefaultReader,
        writer: WritableStreamDefaultWriter,
    },
    Bluetooth {
        device: BluetoothDevice,
        characteristic: BluetoothRemoteGattCharacteristic,
        _listeners: [Closure<dyn FnMut()>; 2],
    },
}

#[derive(Debug, Default)]
struct Inbox {
    received: RefCell<Vec<u8>>,
    failure: RefCell<Option<String>>,
}

impl Inbox {
    fn receive(&self, bytes: &[u8]) {
        self.received.borrow_mut().extend_from_slice(bytes);
    }

    fn fail(&self, reason: String) {
        self.failure.borrow_mut().get_or_insert(reason);
    }
}

impl WebTransport {
    /// Whether the browser offers Web Serial.
    #[must_use]
    pub fn serial_supported() -> bool {
        web_sys::window().is_some_and(|window| {
            Reflect::has(&window.navigator(), &"serial".into()).unwrap_or(false)
        })
    }

    /// Whether the browser offers Web Bluetooth.
    #[must_use]
    pub fn bluetooth_supported() -> bool {
        web_sys::window().is_some_and(|window| window.navigator().bluetooth().is_some())
    }

    /// Asks the user to pick the printer's USB serial port and opens it.
    ///
    /// # Errors
    ///
    /// When the user cancels, the browser has no Web Serial, or the port does not open.
    pub async fn request_serial() -> Result<Self, Error> {
        let port = window()?
            .navigator()
            .serial()
            .request_port()
            .await
            .map_err(js_error)?;
        port.open(&SerialOptions::new(BAUD_RATE))
            .await
            .map_err(js_error)?;
        let reader = ReadableStreamDefaultReader::new(&port.readable()).map_err(js_error)?;
        let writer = WritableStreamDefaultWriter::new(&port.writable()).map_err(js_error)?;
        let inbox = Rc::new(Inbox::default());
        spawn_local(pump_serial(reader.clone(), Rc::clone(&inbox)));
        Ok(Self {
            link: Link::Serial {
                port,
                reader,
                writer,
            },
            inbox,
        })
    }

    /// Asks the user to pick a printer named `B1...` over Bluetooth and connects to it.
    ///
    /// # Errors
    ///
    /// When the user cancels, the browser has no Web Bluetooth, or the connection fails.
    pub async fn request_bluetooth() -> Result<Self, Error> {
        let bluetooth = window()?
            .navigator()
            .bluetooth()
            .ok_or_else(|| Error::transport("this browser has no Web Bluetooth"))?;
        let filter = BluetoothLeScanFilterInit::new();
        filter.set_name_prefix(BLE_NAME_PREFIX);
        let options = RequestDeviceOptions::new();
        options.set_filters(&[filter]);
        options.set_optional_services(&[JsString::from(BLE_SERVICE)]);
        let device = bluetooth.request_device(&options).await.map_err(js_error)?;
        let gatt = device
            .gatt()
            .ok_or_else(|| Error::transport("printer has no GATT server"))?;
        let server = gatt.connect().await.map_err(js_error)?;
        let service = server
            .get_primary_service_with_str(BLE_SERVICE)
            .await
            .map_err(js_error)?;
        let characteristic = service
            .get_characteristics()
            .await
            .map_err(js_error)?
            .iter()
            .find(|candidate| {
                candidate.properties().notify() && candidate.properties().write_without_response()
            })
            .ok_or_else(|| {
                Error::transport("printer has no notify and write-without-response characteristic")
            })?;

        let inbox = Rc::new(Inbox::default());
        let on_value = Closure::<dyn FnMut()>::new({
            let (inbox, characteristic) = (Rc::clone(&inbox), characteristic.clone());
            move || {
                if let Some(view) = characteristic.value() {
                    inbox.receive(
                        &Uint8Array::new(&view.buffer())
                            .subarray(
                                offset(view.byte_offset()),
                                offset(view.byte_offset() + view.byte_length()),
                            )
                            .to_vec(),
                    );
                }
            }
        });
        let on_disconnect = Closure::<dyn FnMut()>::new({
            let inbox = Rc::clone(&inbox);
            move || inbox.fail("printer disconnected".to_owned())
        });
        characteristic
            .add_event_listener_with_callback(
                "characteristicvaluechanged",
                on_value.as_ref().unchecked_ref(),
            )
            .map_err(js_error)?;
        device
            .add_event_listener_with_callback(
                "gattserverdisconnected",
                on_disconnect.as_ref().unchecked_ref(),
            )
            .map_err(js_error)?;
        characteristic
            .start_notifications()
            .await
            .map_err(js_error)?;
        Ok(Self {
            link: Link::Bluetooth {
                device,
                characteristic,
                _listeners: [on_value, on_disconnect],
            },
            inbox,
        })
    }

    /// Closes the port or disconnects from the printer.
    ///
    /// # Errors
    ///
    /// When the browser fails to close the serial port.
    pub async fn close(self) -> Result<(), Error> {
        match self.link {
            Link::Serial {
                port,
                reader,
                writer,
            } => {
                reader.cancel().await.map_err(js_error)?;
                reader.release_lock();
                writer.release_lock();
                port.close().await.map_err(js_error)?;
            }
            Link::Bluetooth { device, .. } => {
                if let Some(gatt) = device.gatt() {
                    gatt.disconnect();
                }
            }
        }
        Ok(())
    }
}

impl Transport for WebTransport {
    async fn write(&mut self, packet: &[u8]) -> Result<(), Error> {
        let chunk = Uint8Array::new_from_slice(packet);
        match &self.link {
            Link::Serial { writer, .. } => {
                writer.write_with_chunk(&chunk).await.map_err(js_error)?;
            }
            Link::Bluetooth { characteristic, .. } => {
                sleep(BLE_PACKET_INTERVAL).await?;
                characteristic
                    .write_value_without_response_with_buffer_source(&chunk)
                    .map_err(js_error)?
                    .await
                    .map_err(js_error)?;
            }
        }
        Ok(())
    }

    async fn read(&mut self, timeout: Duration) -> Result<Vec<u8>, Error> {
        let deadline = Instant::now() + timeout;
        loop {
            let bytes = mem::take(&mut *self.inbox.received.borrow_mut());
            if !bytes.is_empty() {
                return Ok(bytes);
            }
            if let Some(reason) = self.inbox.failure.borrow().clone() {
                return Err(Error::transport(reason));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(bytes);
            }
            sleep(remaining.min(INBOX_POLL_INTERVAL)).await?;
        }
    }
}

async fn pump_serial(reader: ReadableStreamDefaultReader, inbox: Rc<Inbox>) {
    loop {
        let chunk = match reader.read().await {
            Ok(chunk) => chunk,
            Err(error) => return inbox.fail(describe(error)),
        };
        if Reflect::get(&chunk, &"done".into()).is_ok_and(|done| done.is_truthy()) {
            return inbox.fail("serial port closed".to_owned());
        }
        if let Ok(bytes) =
            Reflect::get(&chunk, &"value".into()).and_then(JsCast::dyn_into::<Uint8Array>)
        {
            inbox.receive(&bytes.to_vec());
        }
    }
}

async fn sleep(duration: Duration) -> Result<(), Error> {
    let window = window()?;
    let millis = i32::try_from(duration.as_millis()).unwrap_or(i32::MAX);
    let mut scheduled = Ok(0);
    let timer = Promise::new(&mut |resolve, _reject| {
        scheduled = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis);
    });
    scheduled.map_err(js_error)?;
    timer.await.map_err(js_error)?;
    Ok(())
}

fn window() -> Result<web_sys::Window, Error> {
    web_sys::window().ok_or_else(|| Error::transport("no browser window"))
}

fn offset(bytes: usize) -> u32 {
    u32::try_from(bytes).expect("a notification is far below 4 GiB")
}

fn js_error(error: JsValue) -> Error {
    Error::transport(describe(error))
}

fn describe(error: JsValue) -> String {
    match error.dyn_into::<js_sys::Error>() {
        Ok(error) => error.message().into(),
        Err(other) => other.as_string().unwrap_or_else(|| format!("{other:?}")),
    }
}
