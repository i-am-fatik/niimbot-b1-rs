use js_sys::{Object, Reflect};
use niimbot_b1::{Bitmap, PRINTHEAD_PX, Raster};
use wasm_bindgen::{Clamped, JsCast as _, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement, ImageData};

const DOTS_PER_MM: u32 = 8;
const MARGIN: f64 = 8.0;
const BORDER_WIDTH: f64 = 3.0;
const LINE_SPACING: f64 = 1.15;
pub(crate) const MAX_DARKNESS: i8 = 10;
const MAX_GAMMA: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone)]
pub(crate) struct Picture {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) image: HtmlImageElement,
    pub(crate) size: u16,
    pub(crate) offset: (f64, f64),
}

#[derive(Debug, Clone, Copy)]
struct Frame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Picture {
    pub(crate) fn new(id: String, name: String, image: HtmlImageElement) -> Self {
        Self {
            id,
            name,
            image,
            size: 100,
            offset: (0.0, 0.0),
        }
    }

    fn frame(&self, label_width: f64, label_height: f64) -> Frame {
        let natural_width = f64::from(self.image.natural_width());
        let natural_height = f64::from(self.image.natural_height());
        let scale = (label_width / natural_width).min(label_height / natural_height)
            * f64::from(self.size)
            / 100.0;
        let (width, height) = (natural_width * scale, natural_height * scale);
        Frame {
            x: (label_width - width) / 2.0 + self.offset.0,
            y: (label_height - height) / 2.0 + self.offset.1,
            width,
            height,
        }
    }
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
    pub(crate) darkness: i8,
}

impl Label {
    pub(crate) fn width(&self) -> u32 {
        let head = u32::try_from(PRINTHEAD_PX).expect("print head width fits u32");
        (u32::from(self.width_mm) * DOTS_PER_MM).min(head)
    }

    pub(crate) fn height(&self) -> u32 {
        u32::from(self.height_mm) * DOTS_PER_MM
    }

    pub(crate) fn picture_at(&self, pictures: &[Picture], (x, y): (f64, f64)) -> Option<usize> {
        let (width, height) = (f64::from(self.width()), f64::from(self.height()));
        pictures.iter().rposition(|picture| {
            let frame = picture.frame(width, height);
            (frame.x..frame.x + frame.width).contains(&x)
                && (frame.y..frame.y + frame.height).contains(&y)
        })
    }

    pub(crate) fn rasterize(
        &self,
        canvas: &HtmlCanvasElement,
        pictures: &[Picture],
    ) -> Result<Bitmap, JsValue> {
        let (width, height) = (self.width(), self.height());
        canvas.set_width(width);
        canvas.set_height(height);
        let context = context(canvas)?;
        let (w, h) = (f64::from(width), f64::from(height));

        context.set_fill_style_str("#fff");
        context.fill_rect(0.0, 0.0, w, h);
        for picture in pictures
            .iter()
            .filter(|picture| picture.image.natural_width() > 0)
        {
            let frame = picture.frame(w, h);
            context.draw_image_with_html_image_element_and_dw_and_dh(
                &picture.image,
                frame.x,
                frame.y,
                frame.width,
                frame.height,
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

        let mut pixels = context
            .get_image_data(0, 0, signed(width), signed(height))?
            .data()
            .0;
        darken(&mut pixels, self.darkness);
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

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the curve maps lightness 0..=1 back onto 0..=255"
)]
fn darken(rgba: &mut [u8], darkness: i8) {
    let gamma = MAX_GAMMA.powf(f64::from(darkness) / f64::from(MAX_DARKNESS));
    let curve: Vec<u8> = (0..=u8::MAX)
        .map(|level| (255.0 * (f64::from(level) / 255.0).powf(gamma)).round() as u8)
        .collect();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        for channel in &mut pixel[..3] {
            *channel = curve[usize::from(*channel)];
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(level: u8, count: usize) -> Vec<u8> {
        [level, level, level, 255].repeat(count)
    }

    fn dot_count(rgba: &[u8]) -> usize {
        let bitmap = Bitmap::from_rgba(64, 64, rgba, Raster::Dither);
        (0..64)
            .flat_map(|y| (0..64).map(move |x| (x, y)))
            .filter(|&(x, y)| bitmap.is_black(x, y))
            .count()
    }

    #[test]
    fn neutral_darkness_changes_nothing() {
        let original: Vec<u8> = (0..=u8::MAX).flat_map(|level| gray(level, 1)).collect();
        let mut pixels = original.clone();
        darken(&mut pixels, 0);
        assert_eq!(pixels, original);
    }

    #[test]
    fn darkness_keeps_white_and_black() {
        for darkness in -MAX_DARKNESS..=MAX_DARKNESS {
            let mut pixels = [gray(255, 1), gray(0, 1)].concat();
            darken(&mut pixels, darkness);
            assert_eq!(pixels, [gray(255, 1), gray(0, 1)].concat(), "{darkness}");
        }
    }

    #[test]
    fn darker_gray_prints_more_dots() {
        let counts: Vec<usize> = [-5, 0, 5]
            .into_iter()
            .map(|darkness| {
                let mut pixels = gray(160, 64 * 64);
                darken(&mut pixels, darkness);
                dot_count(&pixels)
            })
            .collect();
        assert!(counts.is_sorted() && counts[0] < counts[2], "{counts:?}");
    }
}
