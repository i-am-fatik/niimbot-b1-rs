/// How gray pixels become black or white dots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Raster {
    /// Floyd-Steinberg error diffusion, identical to Pillow's conversion to mode `1`. Best for
    /// photos and the same output as the Python client.
    #[default]
    Dither,
    /// Pixels darker than mid-gray print black, nothing else. Sharpest for text and line art.
    Threshold,
}

/// A 1-bit image, one bit per dot, rows packed most significant bit first. A set bit prints
/// black.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl Bitmap {
    /// An all-white bitmap.
    #[must_use]
    pub fn blank(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![0; width.div_ceil(8) * height],
        }
    }

    /// Rasterizes RGBA pixels the way Pillow converts an image to grayscale and then to 1-bit.
    /// Alpha is ignored, so draw transparent images onto white first.
    ///
    /// # Panics
    ///
    /// When `rgba` is not `width * height * 4` bytes long.
    #[must_use]
    pub fn from_rgba(width: usize, height: usize, rgba: &[u8], raster: Raster) -> Self {
        assert_eq!(
            rgba.len(),
            width * height * 4,
            "rgba buffer does not match {width}x{height}"
        );
        let ink: Vec<u8> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[r, g, b, _]| 255 - luma(r, g, b))
            .collect();
        let mut bitmap = Self::blank(width, height);
        match raster {
            Raster::Dither => bitmap.diffuse(&ink),
            Raster::Threshold => {
                for (i, &level) in ink.iter().enumerate() {
                    if is_dot(i32::from(level)) {
                        bitmap.set_black(i % width, i / width);
                    }
                }
            }
        }
        bitmap
    }

    /// Width in dots.
    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in dots, which is the number of rows sent to the printer.
    #[must_use]
    pub fn height(&self) -> usize {
        self.height
    }

    /// The packed bytes of row `y`.
    ///
    /// # Panics
    ///
    /// When `y` is not below [`height`](Self::height).
    #[must_use]
    pub fn row(&self, y: usize) -> &[u8] {
        let stride = self.stride();
        &self.data[y * stride..(y + 1) * stride]
    }

    /// Whether the dot at `x`, `y` prints black.
    ///
    /// # Panics
    ///
    /// When the dot lies outside the bitmap.
    #[must_use]
    pub fn is_black(&self, x: usize, y: usize) -> bool {
        self.row(y)[x / 8] & (0x80 >> (x % 8)) != 0
    }

    fn set_black(&mut self, x: usize, y: usize) {
        let stride = self.stride();
        self.data[y * stride + x / 8] |= 0x80 >> (x % 8);
    }

    fn stride(&self) -> usize {
        self.width.div_ceil(8)
    }

    fn diffuse(&mut self, ink: &[u8]) {
        let width = self.width;
        let mut from_above = vec![0_i32; width];
        for y in 0..self.height {
            let mut to_below = vec![0_i32; width];
            let mut from_left = 0;
            for x in 0..width {
                let level = (i32::from(ink[y * width + x]) + (from_left + from_above[x]) / 16)
                    .clamp(0, 255);
                let dot = is_dot(level);
                if dot {
                    self.set_black(x, y);
                }
                let error = level - if dot { 255 } else { 0 };
                from_left = 7 * error;
                if x > 0 {
                    to_below[x - 1] += 3 * error;
                }
                to_below[x] += 5 * error;
                if x + 1 < width {
                    to_below[x + 1] += error;
                }
            }
            from_above = to_below;
        }
    }
}

fn is_dot(ink: i32) -> bool {
    ink > 128
}

fn luma(r: u8, g: u8, b: u8) -> u8 {
    let weighted = u32::from(r) * 19595 + u32::from(g) * 38470 + u32::from(b) * 7471 + 0x8000;
    u8::try_from(weighted >> 16).expect("luma weights sum to 1.0")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(width: usize, height: usize, level: u8) -> Vec<u8> {
        [level, level, level, 255].repeat(width * height)
    }

    #[test]
    fn white_prints_nothing_and_black_prints_everything() {
        let white = Bitmap::from_rgba(9, 2, &gray(9, 2, 255), Raster::Dither);
        let black = Bitmap::from_rgba(9, 2, &gray(9, 2, 0), Raster::Dither);
        assert_eq!(white, Bitmap::blank(9, 2));
        assert_eq!(black.row(1), &[0xFF, 0x80]);
    }

    #[test]
    fn threshold_splits_at_mid_gray() {
        assert!(Bitmap::from_rgba(1, 1, &gray(1, 1, 126), Raster::Threshold).is_black(0, 0));
        assert!(!Bitmap::from_rgba(1, 1, &gray(1, 1, 127), Raster::Threshold).is_black(0, 0));
    }

    #[test]
    fn dither_spreads_mid_gray_to_about_half_the_dots() {
        let bitmap = Bitmap::from_rgba(64, 64, &gray(64, 64, 128), Raster::Dither);
        let dots = (0..64)
            .flat_map(|y| (0..64).map(move |x| (x, y)))
            .filter(|&(x, y)| bitmap.is_black(x, y))
            .count();
        assert!((1900..=2200).contains(&dots), "{dots} dots");
    }

    #[test]
    fn luma_uses_pillow_weights() {
        assert_eq!(luma(255, 0, 0), 76);
        assert_eq!(luma(0, 255, 0), 150);
        assert_eq!(luma(0, 0, 255), 29);
    }
}
