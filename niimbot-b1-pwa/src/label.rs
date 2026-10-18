use js_sys::{Object, Reflect};
use niimbot_b1::{Bitmap, PRINTHEAD_PX, Raster};
use wasm_bindgen::{Clamped, JsCast as _, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement, ImageData};

const DOTS_PER_MM: u32 = 8;
const MARGIN: f64 = 8.0;
const BORDER_WIDTH: f64 = 3.0;
const LINE_SPACING: f64 = 1.15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug)]
pub(crate) struct Label {
    pub(crate) width_mm: u16,
    pub(crate) height_mm: u16,
    pub(crate) text: String,
    pub(crate) font_size: u16,
    pub(crate) font: String,
    pub(crate) bold: bool,
    pub(crate) align: Align,
    pub(crate) border: bool,
    pub(crate) raster: Raster,
}

impl Label {
    pub(crate) fn width(&self) -> u32 {
        let head = u32::try_from(PRINTHEAD_PX).expect("print head width fits u32");
        (u32::from(self.width_mm) * DOTS_PER_MM).min(head)
    }

    pub(crate) fn height(&self) -> u32 {
        u32::from(self.height_mm) * DOTS_PER_MM
    }

    pub(crate) fn rasterize(
        &self,
        canvas: &HtmlCanvasElement,
        picture: Option<&HtmlImageElement>,
    ) -> Result<Bitmap, JsValue> {
        let (width, height) = (self.width(), self.height());
        canvas.set_width(width);
        canvas.set_height(height);
        let context = context(canvas)?;
        let (w, h) = (f64::from(width), f64::from(height));

        context.set_fill_style_str("#fff");
        context.fill_rect(0.0, 0.0, w, h);
        if let Some(picture) = picture.filter(|picture| picture.natural_width() > 0) {
            let (pw, ph) = (
                f64::from(picture.natural_width()),
                f64::from(picture.natural_height()),
            );
            let scale = (w / pw).min(h / ph);
            let (dw, dh) = (pw * scale, ph * scale);
            context.draw_image_with_html_image_element_and_dw_and_dh(
                picture,
                (w - dw) / 2.0,
                (h - dh) / 2.0,
                dw,
                dh,
            )?;
        }

        context.set_fill_style_str("#000");
        context.set_font(&format!(
            "{} {}px {}",
            if self.bold { "bold" } else { "normal" },
            self.font_size,
            self.font
        ));
        context.set_text_baseline("middle");
        let x = match self.align {
            Align::Left => {
                context.set_text_align("left");
                MARGIN
            }
            Align::Center => {
                context.set_text_align("center");
                w / 2.0
            }
            Align::Right => {
                context.set_text_align("right");
                w - MARGIN
            }
        };
        let lines: Vec<&str> = self.text.lines().collect();
        let line_height = f64::from(self.font_size) * LINE_SPACING;
        let block_height = line_height * f64::from(u16::try_from(lines.len()).unwrap_or(u16::MAX));
        let mut y = (h - block_height) / 2.0 + line_height / 2.0;
        for line in lines {
            context.fill_text(line, x, y)?;
            y += line_height;
        }

        if self.border {
            context.set_stroke_style_str("#000");
            context.set_line_width(BORDER_WIDTH);
            let inset = BORDER_WIDTH / 2.0;
            context.stroke_rect(inset, inset, w - BORDER_WIDTH, h - BORDER_WIDTH);
        }

        let pixels = context
            .get_image_data(0, 0, signed(width), signed(height))?
            .data();
        Ok(Bitmap::from_rgba(
            dots(width),
            dots(height),
            &pixels,
            self.raster,
        ))
    }
}

pub(crate) fn show(bitmap: &Bitmap, preview: &HtmlCanvasElement) -> Result<(), JsValue> {
    let (width, height) = (bitmap.width(), bitmap.height());
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let level = if bitmap.is_black(x, y) { 0 } else { 255 };
            rgba.extend_from_slice(&[level, level, level, 255]);
        }
    }
    let (width, height) = (
        u32::try_from(width).map_err(error)?,
        u32::try_from(height).map_err(error)?,
    );
    preview.set_width(width);
    preview.set_height(height);
    let image = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&rgba), width, height)?;
    context(preview)?.put_image_data(&image, 0, 0)
}

fn context(canvas: &HtmlCanvasElement) -> Result<CanvasRenderingContext2d, JsValue> {
    let options = Object::new();
    Reflect::set(&options, &"willReadFrequently".into(), &true.into())?;
    canvas
        .get_context_with_context_options("2d", &options)?
        .ok_or_else(|| JsValue::from_str("canvas has no 2d context"))?
        .dyn_into()
        .map_err(|_| JsValue::from_str("2d context has an unexpected type"))
}

fn signed(dots: u32) -> i32 {
    i32::try_from(dots).expect("label dots fit i32")
}

fn dots(dots: u32) -> usize {
    usize::try_from(dots).expect("label dots fit usize")
}

fn error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
