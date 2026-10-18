//! Browser app that composes a label on a canvas and prints it on a Niimbot B1 over Web Serial
//! or Web Bluetooth.

mod label;
mod saved;

use std::cell::{Cell, RefCell};
use std::fmt::Display;
use std::rc::Rc;

use niimbot_b1::{Bitmap, Error, LabelType, PrintOptions, Printer, Raster, WebTransport};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Blob, Document, HtmlButtonElement, HtmlCanvasElement, HtmlElement, HtmlImageElement,
    HtmlInputElement, HtmlOptionElement, HtmlSelectElement, HtmlTextAreaElement, PointerEvent,
    Storage, Url,
};

use crate::label::{Align, Label, MAX_DARKNESS, Picture};
use crate::saved::Saved;

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
    let saved = app.storage.as_ref().map(Saved::read).unwrap_or_default();
    saved.restore_fields(&app.document);
    app.render();
    app.sync_controls();
    spawn_local(Rc::clone(&app).restore_pictures(saved));
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

#[derive(Debug, Clone, Copy)]
struct Drag {
    pointer_start: (f64, f64),
    offset_start: (f64, f64),
}

#[derive(Debug)]
struct Page {
    connect_usb: HtmlButtonElement,
    connect_bluetooth: HtmlButtonElement,
    disconnect: HtmlButtonElement,
    print: HtmlButtonElement,
    form: HtmlElement,
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
    add_picture: HtmlInputElement,
    remove_picture: HtmlButtonElement,
    picture_list: HtmlSelectElement,
    picture_size: HtmlInputElement,
    raster: HtmlSelectElement,
    darkness: HtmlInputElement,
    density: HtmlSelectElement,
    copies: HtmlInputElement,
    label_type: HtmlSelectElement,
}

#[derive(Debug)]
struct App {
    page: Page,
    document: Document,
    storage: Option<Storage>,
    canvas: HtmlCanvasElement,
    connection: RefCell<Connection>,
    pictures: RefCell<Vec<Picture>>,
    selected: Cell<Option<usize>>,
    drag: Cell<Option<Drag>>,
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
            form: element(&document, "label")?,
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
            add_picture: element(&document, "add-picture")?,
            remove_picture: element(&document, "remove-picture")?,
            picture_list: element(&document, "picture-list")?,
            picture_size: element(&document, "picture-size")?,
            raster: element(&document, "raster")?,
            darkness: element(&document, "darkness")?,
            density: element(&document, "density")?,
            copies: element(&document, "copies")?,
            label_type: element(&document, "label-type")?,
        };
        let canvas = document.create_element("canvas")?.dyn_into()?;
        let storage = web_sys::window().and_then(|window| window.local_storage().ok().flatten());
        if storage.is_none() {
            web_sys::console::warn_1(
                &"This browser blocks storage, so the label is not saved".into(),
            );
        }
        Ok(Self {
            page,
            document,
            storage,
            canvas,
            connection: RefCell::new(Connection::Disconnected),
            pictures: RefCell::new(Vec::new()),
            selected: Cell::new(None),
            drag: Cell::new(None),
            bitmap: RefCell::new(Bitmap::blank(0, 0)),
        })
    }

    fn wire(app: &Rc<Self>) -> Result<(), JsValue> {
        let page = &app.page;
        let label_fields: [&HtmlElement; 10] = [
            &page.width,
            &page.height,
            &page.text,
            &page.font_size,
            &page.font,
            &page.bold,
            &page.align,
            &page.border,
            &page.raster,
            &page.darkness,
        ];
        for field in label_fields {
            on(field, "input", app, Self::render)?;
        }
        on(&page.add_picture, "change", app, |app| {
            spawn_local(Rc::clone(app).add_pictures());
        })?;
        on(&page.remove_picture, "click", app, Self::remove_picture)?;
        on(&page.picture_list, "change", app, |app| {
            app.select_picture(usize::try_from(app.page.picture_list.selected_index()).ok());
            app.save();
        })?;
        on(&page.form, "input", app, Self::save)?;
        on(&page.picture_size, "input", app, Self::resize_picture)?;
        on_pointer(&page.preview, "pointerdown", app, Self::start_drag)?;
        on_pointer(&page.preview, "pointermove", app, Self::drag_picture)?;
        on(&page.preview, "pointerup", app, Self::end_drag)?;
        on(&page.preview, "pointercancel", app, Self::end_drag)?;
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
            darkness: parse(&page.darkness.value(), 0).clamp(-MAX_DARKNESS, MAX_DARKNESS),
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
            .rasterize(&self.canvas, &self.pictures.borrow())
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

    async fn add_pictures(self: Rc<Self>) {
        let Some(files) = self.page.add_picture.files() else {
            return;
        };
        let files: Vec<_> = (0..files.length())
            .filter_map(|index| files.get(index))
            .collect();
        self.page.add_picture.set_value("");
        for file in files {
            let Ok(image) = decode(&file).await else {
                self.fail(format!(
                    "{} is not an image this browser can open",
                    file.name()
                ));
                continue;
            };
            let picture = Picture::new(random_id(), file.name(), image);
            if self.storage.is_some()
                && let Err(error) = saved::store_file(&picture.id, &file).await
            {
                web_sys::console::warn_1(&error);
            }
            self.add_picture(picture);
        }
        self.save();
    }

    async fn restore_pictures(self: Rc<Self>, saved: Saved) {
        for picture in saved.pictures {
            let Ok(file) = saved::load_file(&picture.id).await else {
                continue;
            };
            let Ok(image) = decode(&file).await else {
                continue;
            };
            self.add_picture(Picture {
                id: picture.id,
                name: picture.name,
                image,
                size: picture.size,
                offset: picture.offset,
            });
        }
        let selected = saved.selected.and_then(|id| {
            self.pictures
                .borrow()
                .iter()
                .position(|picture| picture.id == id)
        });
        self.select_picture(selected);
    }

    fn save(self: &Rc<Self>) {
        let Some(storage) = &self.storage else {
            return;
        };
        let saved = Saved::capture(&self.document, &self.pictures.borrow(), self.selected.get());
        if let Err(error) = saved.write(storage) {
            web_sys::console::warn_1(&error);
        }
    }

    fn add_picture(self: &Rc<Self>, picture: Picture) {
        let listed = HtmlOptionElement::new_with_text(&picture.name)
            .and_then(|option| self.page.picture_list.add_with_html_option_element(&option));
        if let Err(error) = listed {
            self.fail(format!("{error:?}"));
            return;
        }
        self.pictures.borrow_mut().push(picture);
        self.select_picture(self.pictures.borrow().len().checked_sub(1));
        self.render();
    }

    fn remove_picture(self: &Rc<Self>) {
        let Some(index) = self.selected.get() else {
            return;
        };
        let removed = self.pictures.borrow_mut().remove(index);
        self.page
            .picture_list
            .remove_with_index(list_index(Some(index)));
        self.select_picture(self.pictures.borrow().len().checked_sub(1));
        self.render();
        self.save();
        if self.storage.is_some() {
            spawn_local(async move {
                if let Err(error) = saved::delete_file(&removed.id).await {
                    web_sys::console::warn_1(&error);
                }
            });
        }
    }

    fn select_picture(&self, index: Option<usize>) {
        self.selected.set(index);
        let pictures = self.pictures.borrow();
        let selected = index.and_then(|index| pictures.get(index));
        let page = &self.page;
        page.picture_list.set_hidden(pictures.is_empty());
        page.picture_list.set_selected_index(list_index(index));
        page.picture_size.set_disabled(selected.is_none());
        page.picture_size
            .set_value(&selected.map_or(100, |picture| picture.size).to_string());
        page.remove_picture.set_disabled(selected.is_none());
        page.preview
            .set_class_name(if pictures.is_empty() { "" } else { "movable" });
    }

    fn update_selected(&self, update: impl FnOnce(&mut Picture)) {
        if let Some(index) = self.selected.get()
            && let Some(picture) = self.pictures.borrow_mut().get_mut(index)
        {
            update(picture);
        }
    }

    fn resize_picture(self: &Rc<Self>) {
        let size = parse(&self.page.picture_size.value(), 100).clamp(10, 400);
        self.update_selected(|picture| picture.size = size);
        self.render();
    }

    fn start_drag(self: &Rc<Self>, event: &PointerEvent) {
        let point = self.dots_at(event);
        if let Some(hit) = self.label().picture_at(&self.pictures.borrow(), point) {
            self.select_picture(Some(hit));
        }
        let Some(index) = self.selected.get() else {
            return;
        };
        let _captured = self.page.preview.set_pointer_capture(event.pointer_id());
        self.drag.set(Some(Drag {
            pointer_start: point,
            offset_start: self.pictures.borrow()[index].offset,
        }));
    }

    fn drag_picture(self: &Rc<Self>, event: &PointerEvent) {
        let Some(drag) = self.drag.get() else {
            return;
        };
        let (x, y) = self.dots_at(event);
        let offset = (
            drag.offset_start.0 + x - drag.pointer_start.0,
            drag.offset_start.1 + y - drag.pointer_start.1,
        );
        self.update_selected(|picture| picture.offset = offset);
        self.render();
    }

    fn end_drag(self: &Rc<Self>) {
        self.drag.set(None);
        self.save();
    }

    fn dots_at(&self, event: &PointerEvent) -> (f64, f64) {
        let preview = &self.page.preview;
        let bounds = preview.get_bounding_client_rect();
        let dots_per_pixel = f64::from(preview.width()) / bounds.width();
        (
            (event.client_x() - bounds.left()) * dots_per_pixel,
            (event.client_y() - bounds.top()) * dots_per_pixel,
        )
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

fn on_pointer(
    target: &HtmlElement,
    event: &str,
    app: &Rc<App>,
    handler: impl Fn(&Rc<App>, &PointerEvent) + 'static,
) -> Result<(), JsValue> {
    let app = Rc::clone(app);
    let listener =
        Closure::<dyn Fn(PointerEvent)>::new(move |event: PointerEvent| handler(&app, &event));
    target.add_event_listener_with_callback(event, listener.as_ref().unchecked_ref())?;
    listener.forget();
    Ok(())
}

async fn decode(blob: &Blob) -> Result<HtmlImageElement, JsValue> {
    let url = Url::create_object_url_with_blob(blob)?;
    let image = HtmlImageElement::new()?;
    image.set_src(&url);
    let decoded = JsFuture::from(image.decode()).await;
    Url::revoke_object_url(&url)?;
    decoded.map(|_| image)
}

fn random_id() -> String {
    js_sys::Math::random().to_string()
}

fn list_index(index: Option<usize>) -> i32 {
    index
        .and_then(|index| i32::try_from(index).ok())
        .unwrap_or(-1)
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
