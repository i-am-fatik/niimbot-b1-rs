//! Browser app that composes a label on a canvas and prints it on a Niimbot B1 over Web Serial
//! or Web Bluetooth, and builds and flashes a known firmware image over Web Serial.

mod gesture;
mod label;
mod saved;
#[cfg(all(test, target_arch = "wasm32"))]
mod tests;

use std::cell::{Cell, RefCell};
use std::fmt::Display;
use std::rc::Rc;

use js_sys::{Array, Uint8Array};
use niimbot_b1::{
    Bitmap, Error, Firmware, LabelType, Patched, PrintOptions, Printer, Raster, WebTransport,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Blob, Document, HtmlAnchorElement, HtmlButtonElement, HtmlCanvasElement, HtmlElement,
    HtmlImageElement, HtmlInputElement, HtmlOptionElement, HtmlSelectElement, HtmlTextAreaElement,
    PointerEvent, Response, Storage, Url,
};

use crate::gesture::Pinch;
use crate::label::{Align, Label, Layer, MAX_DARKNESS, Picture};
use crate::saved::Saved;

const NO_SERIAL: &str = "This browser has no Web Serial. Use Chrome or Edge on a desktop.";
const NO_BLUETOOTH: &str = "This browser has no Web Bluetooth. Use Chrome or Edge.";
const DENSITY_COEFF_URL: &str = "https://raw.githubusercontent.com/lotabout/niimbot/niimbot-hw6.01/firmware/B1_6.19_density_coeff.bin";
const NOLOCK_FILE: &str = "B1_6.19_density_align7_nolock.bin";
const STOCK_FIRMWARE: &str = "firmware/B1_6.19.bin";

/// Starts the app when the module loads.
///
/// # Errors
///
/// When the page lacks an element the app expects.
#[cfg_attr(not(test), wasm_bindgen(start))]
pub fn start() -> Result<(), JsValue> {
    let app = Rc::new(App::new()?);
    App::wire(&app)?;
    let saved = app.storage.as_ref().map(Saved::read).unwrap_or_default();
    saved.restore_fields(&app.document);
    app.stack.replace(vec![saved.text()]);
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
struct DragStart {
    pointer: (f64, f64),
    offset: (f64, f64),
}

#[derive(Debug, Clone, Copy)]
struct PinchStart {
    fingers: [(f64, f64); 2],
    size: u16,
    rotation: i16,
    offset: (f64, f64),
}

#[derive(Debug)]
struct Page {
    connect_usb: HtmlButtonElement,
    connect_bluetooth: HtmlButtonElement,
    disconnect: HtmlButtonElement,
    print: HtmlButtonElement,
    form: HtmlElement,
    status: HtmlElement,
    sheet: HtmlElement,
    preview: HtmlCanvasElement,
    guide_vertical: HtmlElement,
    guide_horizontal: HtmlElement,
    size: HtmlElement,
    width: HtmlInputElement,
    height: HtmlInputElement,
    orientation: HtmlSelectElement,
    text: HtmlTextAreaElement,
    font_size: HtmlInputElement,
    font: HtmlSelectElement,
    bold: HtmlInputElement,
    align: HtmlSelectElement,
    border: HtmlInputElement,
    add_picture: HtmlInputElement,
    remove_picture: HtmlButtonElement,
    layers: HtmlElement,
    layer_list: HtmlSelectElement,
    raise_layer: HtmlButtonElement,
    lower_layer: HtmlButtonElement,
    centre_layer: HtmlButtonElement,
    picture_size: HtmlInputElement,
    picture_rotation: HtmlInputElement,
    turn_picture: HtmlButtonElement,
    picture_darkness: HtmlInputElement,
    raster: HtmlSelectElement,
    density: HtmlSelectElement,
    copies: HtmlInputElement,
    label_type: HtmlSelectElement,
    firmware: HtmlElement,
    firmware_file: HtmlInputElement,
    build_firmware: HtmlButtonElement,
    save_firmware: HtmlAnchorElement,
    flash: HtmlButtonElement,
}

#[derive(Debug)]
struct App {
    page: Page,
    document: Document,
    storage: Option<Storage>,
    canvas: HtmlCanvasElement,
    connection: RefCell<Connection>,
    stack: RefCell<Vec<Layer>>,
    selected: Cell<Option<usize>>,
    pointers: RefCell<Vec<(i32, (f64, f64))>>,
    drag: Cell<Option<DragStart>>,
    pinch: Cell<Option<PinchStart>>,
    bitmap: RefCell<Bitmap>,
    firmware: RefCell<Option<Firmware>>,
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
            sheet: element(&document, "sheet")?,
            preview: element(&document, "preview")?,
            guide_vertical: element(&document, "guide-vertical")?,
            guide_horizontal: element(&document, "guide-horizontal")?,
            size: element(&document, "size")?,
            width: element(&document, "width")?,
            height: element(&document, "height")?,
            orientation: element(&document, "orientation")?,
            text: element(&document, "text")?,
            font_size: element(&document, "font-size")?,
            font: element(&document, "font")?,
            bold: element(&document, "bold")?,
            align: element(&document, "align")?,
            border: element(&document, "border")?,
            add_picture: element(&document, "add-picture")?,
            remove_picture: element(&document, "remove-picture")?,
            layers: element(&document, "layers")?,
            layer_list: element(&document, "layer-list")?,
            raise_layer: element(&document, "raise-layer")?,
            lower_layer: element(&document, "lower-layer")?,
            centre_layer: element(&document, "centre-layer")?,
            picture_size: element(&document, "picture-size")?,
            picture_rotation: element(&document, "picture-rotation")?,
            turn_picture: element(&document, "turn-picture")?,
            picture_darkness: element(&document, "picture-darkness")?,
            raster: element(&document, "raster")?,
            density: element(&document, "density")?,
            copies: element(&document, "copies")?,
            label_type: element(&document, "label-type")?,
            firmware: element(&document, "firmware")?,
            firmware_file: element(&document, "firmware-file")?,
            build_firmware: element(&document, "build-firmware")?,
            save_firmware: element(&document, "save-firmware")?,
            flash: element(&document, "flash")?,
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
            stack: RefCell::new(Vec::new()),
            selected: Cell::new(None),
            pointers: RefCell::new(Vec::new()),
            drag: Cell::new(None),
            pinch: Cell::new(None),
            bitmap: RefCell::new(Bitmap::blank(0, 0)),
            firmware: RefCell::new(None),
        })
    }

    fn wire(app: &Rc<Self>) -> Result<(), JsValue> {
        let page = &app.page;
        let label_fields: [&HtmlElement; 10] = [
            &page.width,
            &page.height,
            &page.orientation,
            &page.text,
            &page.font_size,
            &page.font,
            &page.bold,
            &page.align,
            &page.border,
            &page.raster,
        ];
        for field in label_fields {
            on(field, "input", app, |app| {
                app.confine_layers();
                app.render();
            })?;
        }
        on(&page.add_picture, "change", app, |app| {
            spawn_local(Rc::clone(app).add_pictures());
        })?;
        on(&page.remove_picture, "click", app, Self::remove_picture)?;
        on(&page.layer_list, "change", app, |app| {
            let row = usize::try_from(app.page.layer_list.selected_index()).ok();
            let count = app.stack.borrow().len();
            app.select_layer(row.and_then(|row| top_first(count, row)));
            app.save();
        })?;
        on(&page.raise_layer, "click", app, |app| app.restack(1))?;
        on(&page.lower_layer, "click", app, |app| app.restack(-1))?;
        on(&page.centre_layer, "click", app, Self::centre_layer)?;
        on(&page.form, "input", app, Self::save)?;
        on(&page.picture_size, "input", app, Self::resize_picture)?;
        on(&page.picture_rotation, "input", app, Self::rotate_picture)?;
        on(&page.turn_picture, "click", app, Self::turn_picture)?;
        on(&page.picture_darkness, "input", app, Self::darken_picture)?;
        on_pointer(&page.preview, "pointerdown", app, Self::press)?;
        on_pointer(&page.preview, "pointermove", app, Self::move_pointer)?;
        on_pointer(&page.preview, "pointerup", app, Self::release)?;
        on_pointer(&page.preview, "pointercancel", app, Self::release)?;
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
        on(&page.firmware_file, "change", app, |app| {
            spawn_local(Rc::clone(app).choose_firmware());
        })?;
        on(&page.build_firmware, "click", app, |app| {
            spawn_local(Rc::clone(app).build_firmware());
        })?;
        on(&page.flash, "click", app, |app| {
            spawn_local(Rc::clone(app).flash());
        })?;
        if !WebTransport::serial_supported() {
            page.connect_usb.set_title(NO_SERIAL);
            page.firmware.set_hidden(true);
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
            vertical: page.orientation.value() == "vertical",
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
            .rasterize(&self.canvas, &self.stack.borrow())
            .and_then(|bitmap| label.show(&bitmap, &self.page.preview).map(|()| bitmap));
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
                let ratio =
                    f64::from(self.page.preview.width()) / f64::from(self.page.preview.height());
                let _styled = self
                    .page
                    .sheet
                    .style()
                    .set_property("--ratio", &ratio.to_string());
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
            let earlier = self
                .stack
                .borrow()
                .iter()
                .filter_map(Layer::picture)
                .count();
            let landing = self.label().landing(earlier);
            let picture = Picture::new(random_id(), file.name(), image, landing);
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
        let mut restored = Vec::new();
        for picture in saved.pictures {
            let Ok(file) = saved::load_file(&picture.id).await else {
                continue;
            };
            let Ok(image) = decode(&file).await else {
                continue;
            };
            restored.push(Layer::Picture(Picture {
                id: picture.id,
                name: picture.name,
                image,
                size: picture.size,
                offset: picture.offset,
                rotation: picture.rotation,
                darkness: picture.darkness,
            }));
        }
        let text_at = saved
            .text_layer
            .unwrap_or(restored.len())
            .min(restored.len());
        {
            let mut stack = self.stack.borrow_mut();
            restored.splice(text_at..text_at, stack.drain(..));
            *stack = restored;
        }
        let selected = saved.selected.and_then(|id| {
            self.stack
                .borrow()
                .iter()
                .position(|layer| layer.id() == id)
        });
        self.confine_layers();
        self.list_layers();
        self.select_layer(selected);
        self.render();
    }

    fn save(self: &Rc<Self>) {
        let Some(storage) = &self.storage else {
            return;
        };
        let saved = Saved::capture(&self.document, &self.stack.borrow(), self.selected.get());
        if let Err(error) = saved.write(storage) {
            web_sys::console::warn_1(&error);
        }
    }

    fn add_picture(self: &Rc<Self>, picture: Picture) {
        let added = {
            let mut stack = self.stack.borrow_mut();
            let above_pictures = stack
                .iter()
                .rposition(|layer| layer.picture().is_some())
                .map_or(0, |top| top + 1);
            stack.insert(above_pictures, Layer::Picture(picture));
            above_pictures
        };
        self.list_layers();
        self.select_layer(Some(added));
        self.render();
    }

    fn list_layers(&self) {
        let list = &self.page.layer_list;
        list.set_length(0);
        for layer in self.stack.borrow().iter().rev() {
            let listed = HtmlOptionElement::new_with_text(layer.name())
                .and_then(|option| list.add_with_html_option_element(&option));
            if let Err(error) = listed {
                self.fail(format!("{error:?}"));
                return;
            }
        }
    }

    fn restack(self: &Rc<Self>, step: isize) {
        let Some(index) = self.selected.get() else {
            return;
        };
        let count = self.stack.borrow().len();
        let Some(target) = index
            .checked_add_signed(step)
            .filter(|&target| target < count)
        else {
            return;
        };
        self.stack.borrow_mut().swap(index, target);
        self.list_layers();
        self.select_layer(Some(target));
        self.render();
        self.save();
    }

    fn centre_layer(self: &Rc<Self>) {
        let Some(index) = self.selected.get() else {
            return;
        };
        let label = self.label();
        if let Some(layer) = self.stack.borrow_mut().get_mut(index) {
            let centred = label.centred_offset(&self.canvas, layer);
            *layer.offset_mut() = centred;
        }
        self.render();
        self.save();
    }

    fn remove_picture(self: &Rc<Self>) {
        let Some(index) = self.selected.get() else {
            return;
        };
        let Some(id) = self
            .stack
            .borrow()
            .get(index)
            .and_then(Layer::picture)
            .map(|picture| picture.id.clone())
        else {
            return;
        };
        self.stack.borrow_mut().remove(index);
        self.list_layers();
        let top_picture = self
            .stack
            .borrow()
            .iter()
            .rposition(|layer| layer.picture().is_some());
        self.select_layer(top_picture);
        self.render();
        self.save();
        if self.storage.is_some() {
            spawn_local(async move {
                if let Err(error) = saved::delete_file(&id).await {
                    web_sys::console::warn_1(&error);
                }
            });
        }
    }

    fn select_layer(&self, index: Option<usize>) {
        self.selected.set(index);
        let stack = self.stack.borrow();
        let picture = index
            .and_then(|index| stack.get(index))
            .and_then(Layer::picture);
        let page = &self.page;
        page.layers
            .set_hidden(!stack.iter().any(|layer| layer.picture().is_some()));
        page.layer_list.set_selected_index(
            index
                .and_then(|index| top_first(stack.len(), index))
                .and_then(|row| i32::try_from(row).ok())
                .unwrap_or(-1),
        );
        page.raise_layer
            .set_disabled(index.is_none_or(|index| index + 1 >= stack.len()));
        page.lower_layer
            .set_disabled(index.is_none_or(|index| index == 0));
        page.centre_layer.set_disabled(index.is_none());
        page.picture_size.set_disabled(picture.is_none());
        page.picture_size
            .set_value(&picture.map_or(100, |picture| picture.size).to_string());
        page.picture_rotation.set_disabled(picture.is_none());
        page.picture_rotation
            .set_value(&picture.map_or(0, |picture| picture.rotation).to_string());
        page.turn_picture.set_disabled(picture.is_none());
        page.picture_darkness.set_disabled(picture.is_none());
        page.picture_darkness
            .set_value(&picture.map_or(0, |picture| picture.darkness).to_string());
        page.remove_picture.set_disabled(picture.is_none());
    }

    fn update_picture(&self, update: impl FnOnce(&mut Picture)) {
        if let Some(index) = self.selected.get()
            && let Some(Layer::Picture(picture)) = self.stack.borrow_mut().get_mut(index)
        {
            update(picture);
        }
    }

    fn resize_picture(self: &Rc<Self>) {
        let size = parse(&self.page.picture_size.value(), 100).clamp(10, 400);
        self.update_picture(|picture| picture.size = size);
        self.render();
    }

    fn rotate_picture(self: &Rc<Self>) {
        let rotation = parse(&self.page.picture_rotation.value(), 0).clamp(-180, 180);
        self.update_picture(|picture| picture.rotation = rotation);
        self.render();
    }

    fn turn_picture(self: &Rc<Self>) {
        self.update_picture(|picture| {
            picture.rotation = (picture.rotation + 270).rem_euclid(360) - 180;
        });
        self.select_layer(self.selected.get());
        self.render();
        self.save();
    }

    fn darken_picture(self: &Rc<Self>) {
        let darkness =
            parse(&self.page.picture_darkness.value(), 0).clamp(-MAX_DARKNESS, MAX_DARKNESS);
        self.update_picture(|picture| picture.darkness = darkness);
        self.render();
    }

    fn press(self: &Rc<Self>, event: &PointerEvent) {
        let point = self.dots_at(event);
        let _captured = self.page.preview.set_pointer_capture(event.pointer_id());
        self.pointers.borrow_mut().push((event.pointer_id(), point));
        let fingers = self.pointers.borrow().len();
        match fingers {
            1 => {
                if let Some(hit) = self
                    .label()
                    .layer_at(&self.canvas, &self.stack.borrow(), point)
                {
                    self.select_layer(Some(hit));
                }
                self.start_drag(point);
            }
            2 => self.start_pinch(),
            _ => {}
        }
    }

    fn move_pointer(self: &Rc<Self>, event: &PointerEvent) {
        let point = self.dots_at(event);
        {
            let mut pointers = self.pointers.borrow_mut();
            let Some((_, at)) = pointers
                .iter_mut()
                .find(|(id, _)| *id == event.pointer_id())
            else {
                return;
            };
            *at = point;
        }
        if self.pinch.get().is_some() {
            self.apply_pinch();
        } else {
            self.drag_layer(point);
        }
    }

    fn release(self: &Rc<Self>, event: &PointerEvent) {
        self.pointers
            .borrow_mut()
            .retain(|(id, _)| *id != event.pointer_id());
        let remaining: Vec<(f64, f64)> = self.pointers.borrow().iter().map(|(_, at)| *at).collect();
        match remaining.as_slice() {
            [] => self.end_press(),
            [point] => {
                self.pinch.set(None);
                self.start_drag(*point);
            }
            _ => self.start_pinch(),
        }
    }

    fn start_drag(&self, point: (f64, f64)) {
        let Some(index) = self.selected.get() else {
            return;
        };
        self.drag.set(Some(DragStart {
            pointer: point,
            offset: self.stack.borrow()[index].offset(),
        }));
    }

    fn start_pinch(&self) {
        let (Some(fingers), Some(index)) = (self.fingers(), self.selected.get()) else {
            return;
        };
        let (size, rotation, offset) = match &self.stack.borrow()[index] {
            Layer::Picture(picture) => (picture.size, picture.rotation, picture.offset),
            Layer::Text { offset } => (self.label().font_size, 0, *offset),
        };
        self.drag.set(None);
        self.pinch.set(Some(PinchStart {
            fingers,
            size,
            rotation,
            offset,
        }));
    }

    fn fingers(&self) -> Option<[(f64, f64); 2]> {
        match self.pointers.borrow().as_slice() {
            [(_, first), (_, second), ..] => Some([*first, *second]),
            _ => None,
        }
    }

    fn apply_pinch(self: &Rc<Self>) {
        let (Some(start), Some(fingers), Some(index)) =
            (self.pinch.get(), self.fingers(), self.selected.get())
        else {
            return;
        };
        let pinch = Pinch::between(start.fingers, fingers);
        let text_selected = self.stack.borrow().get(index).is_some_and(Layer::is_text);
        if text_selected {
            self.page
                .font_size
                .set_value(&pinch.resize(start.size, 6..=400).to_string());
        } else {
            self.update_picture(|picture| {
                picture.size = pinch.resize(start.size, 10..=400);
                picture.rotation = pinch.rotate(start.rotation);
            });
        }
        self.place(
            index,
            (
                start.offset.0 + pinch.shift.0,
                start.offset.1 + pinch.shift.1,
            ),
        );
        self.select_layer(Some(index));
        self.render();
    }

    fn drag_layer(self: &Rc<Self>, (x, y): (f64, f64)) {
        let (Some(start), Some(index)) = (self.drag.get(), self.selected.get()) else {
            return;
        };
        self.place(
            index,
            (
                start.offset.0 + x - start.pointer.0,
                start.offset.1 + y - start.pointer.1,
            ),
        );
        self.render();
    }

    fn place(&self, index: usize, moved: (f64, f64)) {
        let label = self.label();
        let Some(placed) = self
            .stack
            .borrow()
            .get(index)
            .map(|layer| label.place(&self.canvas, layer, moved))
        else {
            return;
        };
        self.page
            .guide_vertical
            .set_hidden(!placed.on_vertical_line);
        self.page
            .guide_horizontal
            .set_hidden(!placed.on_horizontal_line);
        if let Some(layer) = self.stack.borrow_mut().get_mut(index) {
            *layer.offset_mut() = placed.offset;
        }
    }

    fn confine_layers(&self) {
        let label = self.label();
        for layer in self.stack.borrow_mut().iter_mut() {
            let offset = label.confine(&self.canvas, layer, layer.offset());
            *layer.offset_mut() = offset;
        }
    }

    fn end_press(self: &Rc<Self>) {
        self.drag.set(None);
        self.pinch.set(None);
        self.page.guide_vertical.set_hidden(true);
        self.page.guide_horizontal.set_hidden(true);
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

    async fn choose_firmware(self: Rc<Self>) {
        let Some(file) = self
            .page
            .firmware_file
            .files()
            .and_then(|files| files.get(0))
        else {
            return;
        };
        let firmware = JsFuture::from(file.array_buffer())
            .await
            .ok()
            .and_then(|buffer| Firmware::identify(Uint8Array::new(&buffer).to_vec()));
        match &firmware {
            Some(firmware) => self.say(&format!(
                "{} is {}, Flash over USB writes it to the printer",
                file.name(),
                firmware.name()
            )),
            None => self.fail(format!(
                "{} is not a known B1 firmware image, so it cannot be flashed",
                file.name()
            )),
        }
        self.page.save_firmware.set_hidden(true);
        *self.firmware.borrow_mut() = firmware;
        self.sync_controls();
    }

    async fn build_firmware(self: Rc<Self>) {
        self.page.build_firmware.set_disabled(true);
        let built = self.patch_stock().await.and_then(|patched| {
            self.offer_save(&patched.nolock)?;
            Ok(patched.nolock)
        });
        match built {
            Ok(firmware) => {
                self.say(&format!(
                    "Built {}, Flash over USB writes it and Save image keeps a copy",
                    firmware.name()
                ));
                *self.firmware.borrow_mut() = Some(firmware);
            }
            Err(error) => self.fail(error),
        }
        self.page.build_firmware.set_disabled(false);
        self.sync_controls();
    }

    async fn patch_stock(&self) -> Result<Patched, String> {
        let picked = self.firmware.borrow().clone().filter(Firmware::is_stock);
        let stock = if let Some(stock) = picked {
            stock
        } else {
            self.say("Downloading the stock firmware...");
            download(STOCK_FIRMWARE)
                .await
                .ok()
                .and_then(Firmware::identify)
                .filter(Firmware::is_stock)
                .ok_or_else(|| {
                    "The stock firmware did not download, download it with the link above and pick it"
                        .to_owned()
                })?
        };
        self.say("Downloading the density image from GitHub...");
        let density_coeff = download(DENSITY_COEFF_URL)
            .await
            .map_err(|error| format!("The density image did not download: {error:?}"))?;
        Firmware::build(stock.image(), &density_coeff).map_err(|error| error.to_string())
    }

    fn offer_save(&self, firmware: &Firmware) -> Result<(), String> {
        let link = &self.page.save_firmware;
        let blob =
            Blob::new_with_u8_array_sequence(&Array::of1(&Uint8Array::from(firmware.image())))
                .and_then(|blob| Url::create_object_url_with_blob(&blob))
                .map_err(|error| format!("{error:?}"))?;
        let earlier = link.href();
        if earlier.starts_with("blob:") {
            let _revoked = Url::revoke_object_url(&earlier);
        }
        link.set_href(&blob);
        link.set_download(NOLOCK_FILE);
        link.set_hidden(false);
        Ok(())
    }

    async fn flash(self: Rc<Self>) {
        let Some(firmware) = self.firmware.borrow().clone() else {
            return;
        };
        let question = format!(
            "Flash {}? Keep the cable in and the printer on until it turns itself off. A broken flash can brick the printer.",
            firmware.name()
        );
        let confirmed = web_sys::window()
            .is_some_and(|window| window.confirm_with_message(&question).unwrap_or(false));
        if !confirmed {
            return;
        }
        let Some(mut printer) = self.take_printer() else {
            return;
        };
        let flashed = printer
            .flash(&firmware, |sent, total| {
                self.say(&format!(
                    "Flashing {}, chunk {sent} of {total}",
                    firmware.name()
                ));
            })
            .await;
        match flashed {
            Ok(()) => {
                let _turned_off = printer.into_transport().close().await;
                self.set_connection(Connection::Disconnected);
                self.say(&format!(
                    "Flashed {}. The printer turns off, press its button to turn it back on.",
                    firmware.name()
                ));
            }
            Err(error) => self.settle(printer, Err(error)),
        }
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
        let usb =
            matches!(&*connection, Connection::Ready(printer) if printer.transport().is_serial());
        self.page
            .connect_usb
            .set_disabled(!disconnected || !WebTransport::serial_supported());
        self.page
            .connect_bluetooth
            .set_disabled(!disconnected || !WebTransport::bluetooth_supported());
        self.page.disconnect.set_hidden(disconnected);
        self.page.disconnect.set_disabled(!ready);
        self.page.print.set_disabled(!ready);
        self.page
            .flash
            .set_disabled(!usb || self.firmware.borrow().is_none());
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

async fn download(url: &str) -> Result<Vec<u8>, JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let response: Response = JsFuture::from(window.fetch_with_str(url))
        .await?
        .dyn_into()?;
    if !response.ok() {
        return Err(format!("{url} answered {}", response.status()).into());
    }
    let buffer = JsFuture::from(response.array_buffer()?).await?;
    Ok(Uint8Array::new(&buffer).to_vec())
}

fn random_id() -> String {
    js_sys::Math::random().to_string()
}

fn top_first(count: usize, position: usize) -> Option<usize> {
    count.checked_sub(position + 1)
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
