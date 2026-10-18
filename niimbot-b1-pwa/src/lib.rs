//! Browser app that composes a label on a canvas and prints it on a Niimbot B1 over Web Serial
//! or Web Bluetooth.

mod label;

use std::cell::RefCell;
use std::fmt::Display;
use std::rc::Rc;

use niimbot_b1::{Bitmap, Error, LabelType, PrintOptions, Printer, Raster, WebTransport};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    Document, HtmlButtonElement, HtmlCanvasElement, HtmlElement, HtmlImageElement,
    HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement, Url,
};

use crate::label::{Align, Label};

const NO_SERIAL: &str = "This browser has no Web Serial. Use Chrome or Edge on a desktop.";
const NO_BLUETOOTH: &str = "This browser has no Web Bluetooth. Use Chrome or Edge.";

/// Starts the app when the module loads.
///
/// # Errors
///
/// When the page lacks an element the app expects.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    let app = Rc::new(App::new()?);
    App::wire(&app)?;
    app.render();
    app.sync_controls();
    register_service_worker();
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Link {
    Usb,
    Bluetooth,
}

#[derive(Debug)]
enum Connection {
    Disconnected,
    Busy,
    Ready(Printer<WebTransport>),
}

#[derive(Debug)]
struct Page {
    connect_usb: HtmlButtonElement,
    connect_bluetooth: HtmlButtonElement,
    disconnect: HtmlButtonElement,
    print: HtmlButtonElement,
    status: HtmlElement,
    preview: HtmlCanvasElement,
    size: HtmlElement,
    width: HtmlInputElement,
    height: HtmlInputElement,
    text: HtmlTextAreaElement,
    font_size: HtmlInputElement,
    font: HtmlSelectElement,
    bold: HtmlInputElement,
    align: HtmlSelectElement,
    border: HtmlInputElement,
    picture: HtmlInputElement,
    clear_picture: HtmlButtonElement,
    raster: HtmlSelectElement,
    density: HtmlSelectElement,
    copies: HtmlInputElement,
    label_type: HtmlSelectElement,
}

#[derive(Debug)]
struct App {
    page: Page,
    canvas: HtmlCanvasElement,
    connection: RefCell<Connection>,
    picture: RefCell<Option<HtmlImageElement>>,
    bitmap: RefCell<Bitmap>,
}

impl App {
    fn new() -> Result<Self, JsValue> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or("no document")?;
        let page = Page {
            connect_usb: element(&document, "connect-usb")?,
            connect_bluetooth: element(&document, "connect-bluetooth")?,
            disconnect: element(&document, "disconnect")?,
            print: element(&document, "print")?,
            status: element(&document, "status")?,
            preview: element(&document, "preview")?,
            size: element(&document, "size")?,
            width: element(&document, "width")?,
            height: element(&document, "height")?,
            text: element(&document, "text")?,
            font_size: element(&document, "font-size")?,
            font: element(&document, "font")?,
            bold: element(&document, "bold")?,
            align: element(&document, "align")?,
            border: element(&document, "border")?,
            picture: element(&document, "picture")?,
            clear_picture: element(&document, "clear-picture")?,
            raster: element(&document, "raster")?,
            density: element(&document, "density")?,
            copies: element(&document, "copies")?,
            label_type: element(&document, "label-type")?,
        };
        let canvas = document.create_element("canvas")?.dyn_into()?;
        Ok(Self {
            page,
            canvas,
            connection: RefCell::new(Connection::Disconnected),
            picture: RefCell::new(None),
            bitmap: RefCell::new(Bitmap::blank(0, 0)),
        })
    }

    fn wire(app: &Rc<Self>) -> Result<(), JsValue> {
        let page = &app.page;
        let label_fields: [&HtmlElement; 9] = [
            &page.width,
            &page.height,
            &page.text,
            &page.font_size,
            &page.font,
            &page.bold,
            &page.align,
            &page.border,
            &page.raster,
        ];
        for field in label_fields {
            on(field, "input", app, Self::render)?;
        }
        on(&page.picture, "change", app, Self::load_picture)?;
        on(&page.clear_picture, "click", app, Self::clear_picture)?;
        on(&page.connect_usb, "click", app, |app| {
            spawn_local(Rc::clone(app).connect(Link::Usb));
        })?;
        on(&page.connect_bluetooth, "click", app, |app| {
            spawn_local(Rc::clone(app).connect(Link::Bluetooth));
        })?;
        on(&page.disconnect, "click", app, |app| {
            spawn_local(Rc::clone(app).disconnect());
        })?;
        on(&page.print, "click", app, |app| {
            spawn_local(Rc::clone(app).print());
        })?;
        if !WebTransport::serial_supported() {
            page.connect_usb.set_title(NO_SERIAL);
        }
        if !WebTransport::bluetooth_supported() {
            page.connect_bluetooth.set_title(NO_BLUETOOTH);
        }
        Ok(())
    }

    fn label(&self) -> Label {
        let page = &self.page;
        Label {
            width_mm: parse(&page.width.value(), 50).clamp(10, 50),
            height_mm: parse(&page.height.value(), 30).clamp(5, 300),
            text: page.text.value(),
            font_size: parse(&page.font_size.value(), 48).clamp(6, 400),
            font: page.font.value(),
            bold: page.bold.checked(),
            align: match page.align.value().as_str() {
                "left" => Align::Left,
                "right" => Align::Right,
                _ => Align::Center,
            },
            border: page.border.checked(),
            raster: if page.raster.value() == "dither" {
                Raster::Dither
            } else {
                Raster::Threshold
            },
        }
    }

    fn print_options(&self) -> PrintOptions {
        let page = &self.page;
        PrintOptions {
            density: parse(&page.density.value(), 5).clamp(1, 5),
            copies: parse(&page.copies.value(), 1).max(1),
            label_type: LabelType(parse(&page.label_type.value(), 1)),
        }
    }

    fn render(self: &Rc<Self>) {
        let label = self.label();
        let rendered = label
            .rasterize(&self.canvas, self.picture.borrow().as_ref())
            .and_then(|bitmap| label::show(&bitmap, &self.page.preview).map(|()| bitmap));
        match rendered {
            Ok(bitmap) => {
                self.page.size.set_text_content(Some(&format!(
                    "{} x {} mm, {} x {} dots",
                    label.width_mm,
                    label.height_mm,
                    bitmap.width(),
                    bitmap.height()
                )));
                *self.bitmap.borrow_mut() = bitmap;
            }
            Err(error) => self.fail(format!("{error:?}")),
        }
    }

    fn load_picture(self: &Rc<Self>) {
        let Some(file) = self.page.picture.files().and_then(|files| files.get(0)) else {
            return;
        };
        let loaded = Url::create_object_url_with_blob(&file).and_then(|url| {
            let picture = HtmlImageElement::new()?;
            let onload = Closure::once({
                let (app, picture, url) = (Rc::clone(self), picture.clone(), url.clone());
                move || {
                    let _revoked = Url::revoke_object_url(&url);
                    *app.picture.borrow_mut() = Some(picture);
                    app.render();
                }
            });
            picture.set_onload(Some(onload.as_ref().unchecked_ref()));
            onload.forget();
            picture.set_src(&url);
            Ok(())
        });
        if let Err(error) = loaded {
            self.fail(format!("{error:?}"));
        }
    }

    fn clear_picture(self: &Rc<Self>) {
        self.page.picture.set_value("");
        *self.picture.borrow_mut() = None;
        self.render();
    }

    async fn connect(self: Rc<Self>, link: Link) {
        if !matches!(*self.connection.borrow(), Connection::Disconnected) {
            return;
        }
        self.set_connection(Connection::Busy);
        self.say("Connecting...");
        let transport = match link {
            Link::Usb => WebTransport::request_serial().await,
            Link::Bluetooth => WebTransport::request_bluetooth().await,
        };
        match transport {
            Ok(transport) => {
                let mut printer = Printer::new(transport);
                let summary = summarize(&mut printer).await;
                self.settle(printer, summary);
            }
            Err(error) => {
                self.fail(error);
                self.set_connection(Connection::Disconnected);
            }
        }
    }

    async fn print(self: Rc<Self>) {
        let Some(mut printer) = self.take_printer() else {
            return;
        };
        self.say("Printing...");
        let bitmap = self.bitmap.borrow().clone();
        let options = self.print_options();
        let outcome = match printer.print(&bitmap, options).await {
            Ok(()) => {
                let plural = if options.copies == 1 { "" } else { "s" };
                summarize(&mut printer)
                    .await
                    .map(|summary| format!("Printed {} label{plural}. {summary}", options.copies))
            }
            Err(error) => Err(error),
        };
        self.settle(printer, outcome);
    }

    async fn disconnect(self: Rc<Self>) {
        let Some(printer) = self.take_printer() else {
            return;
        };
        let closed = printer.into_transport().close().await;
        self.set_connection(Connection::Disconnected);
        match closed {
            Ok(()) => self.say("Disconnected"),
            Err(error) => self.fail(error),
        }
    }

    fn take_printer(&self) -> Option<Printer<WebTransport>> {
        let mut connection = self.connection.borrow_mut();
        match std::mem::replace(&mut *connection, Connection::Busy) {
            Connection::Ready(printer) => {
                drop(connection);
                self.sync_controls();
                Some(printer)
            }
            other => {
                *connection = other;
                None
            }
        }
    }

    fn settle(&self, printer: Printer<WebTransport>, outcome: Result<String, Error>) {
        match outcome {
            Ok(message) => {
                self.say(&message);
                self.set_connection(Connection::Ready(printer));
            }
            Err(error) if error.is_transport() => {
                self.fail(&error);
                self.set_connection(Connection::Disconnected);
                spawn_local(async move {
                    let _already_failed = printer.into_transport().close().await;
                });
            }
            Err(error) => {
                self.fail(&error);
                self.set_connection(Connection::Ready(printer));
            }
        }
    }

    fn set_connection(&self, connection: Connection) {
        *self.connection.borrow_mut() = connection;
        self.sync_controls();
    }

    fn sync_controls(&self) {
        let connection = self.connection.borrow();
        let disconnected = matches!(*connection, Connection::Disconnected);
        let ready = matches!(*connection, Connection::Ready(_));
        self.page
            .connect_usb
            .set_disabled(!disconnected || !WebTransport::serial_supported());
        self.page
            .connect_bluetooth
            .set_disabled(!disconnected || !WebTransport::bluetooth_supported());
        self.page.disconnect.set_hidden(disconnected);
        self.page.disconnect.set_disabled(!ready);
        self.page.print.set_disabled(!ready);
        if disconnected
            && self
                .page
                .status
                .text_content()
                .unwrap_or_default()
                .is_empty()
        {
            self.say(
                match (
                    WebTransport::serial_supported(),
                    WebTransport::bluetooth_supported(),
                ) {
                    (false, false) => {
                        "This browser has neither Web Serial nor Web Bluetooth. Use Chrome or Edge."
                    }
                    _ => "Not connected",
                },
            );
        }
    }

    fn say(&self, message: &str) {
        self.page.status.set_class_name("");
        self.page.status.set_text_content(Some(message));
    }

    fn fail(&self, error: impl Display) {
        self.page.status.set_class_name("error");
        self.page.status.set_text_content(Some(&error.to_string()));
    }
}

async fn summarize(printer: &mut Printer<WebTransport>) -> Result<String, Error> {
    let heartbeat = printer.heartbeat().await?;
    let roll = match printer.rfid_info().await? {
        Some(roll) => roll.to_string(),
        None => "roll without RFID tag".to_owned(),
    };
    Ok(format!("{heartbeat}. {roll}"))
}

fn on(
    target: &HtmlElement,
    event: &str,
    app: &Rc<App>,
    handler: impl Fn(&Rc<App>) + 'static,
) -> Result<(), JsValue> {
    let app = Rc::clone(app);
    let listener = Closure::<dyn Fn()>::new(move || handler(&app));
    target.add_event_listener_with_callback(event, listener.as_ref().unchecked_ref())?;
    listener.forget();
    Ok(())
}

fn element<T: JsCast>(document: &Document, id: &str) -> Result<T, JsValue> {
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("missing #{id}")))?
        .dyn_into()
        .map_err(|_| JsValue::from_str(&format!("#{id} has an unexpected element type")))
}

fn parse<T: std::str::FromStr>(value: &str, fallback: T) -> T {
    value.trim().parse().unwrap_or(fallback)
}

fn register_service_worker() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let registration = window.navigator().service_worker().register("sw.js");
    spawn_local(async move {
        if let Err(error) = registration.await {
            web_sys::console::warn_1(&error);
        }
    });
}
