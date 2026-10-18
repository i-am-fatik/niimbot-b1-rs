use md5::{Digest as _, Md5};

use crate::protocol::Hex;

pub(crate) const VERSION: [u8; 2] = [6, 19];

const STOCK: &str = "B1 6.19 stock";
const KNOWN: [(&str, &str); 3] = [
    ("fd9efd1441b5f05ca46c310b8d162dc1", STOCK),
    ("540ef521d7416cfe1eb561b6effcb0b4", "B1 6.19 patched"),
    (
        "6fd1612be6fb0cfb0ffb27808df3c44c",
        "B1 6.19 patched, never locks",
    ),
];

/// A firmware image known to run on the B1, the only kind [`Printer::flash`](crate::Printer::flash)
/// writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Firmware {
    pub(crate) name: &'static str,
    pub(crate) image: Vec<u8>,
}

impl Firmware {
    /// Recognizes `image` by its MD5 as the stock B1 6.19 firmware or one of the two images
    /// `niimbot-b1 build-firmware` writes, `None` for anything else.
    #[must_use]
    pub fn identify(image: Vec<u8>) -> Option<Self> {
        let digest = Hex(&Md5::digest(&image)).to_string();
        let (_, name) = KNOWN.iter().find(|(md5, _)| *md5 == digest)?;
        Some(Self { name, image })
    }

    /// What the image is, such as `B1 6.19 patched, never locks`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Whether this is the stock image, the input [`Firmware::build`] patches.
    #[must_use]
    pub fn is_stock(&self) -> bool {
        self.name == STOCK
    }

    /// The image's bytes.
    #[must_use]
    pub fn image(&self) -> &[u8] {
        &self.image
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_image_is_not_firmware() {
        assert_eq!(Firmware::identify(vec![0; 124_436]), None);
        assert_eq!(Firmware::identify(Vec::new()), None);
    }
}
