use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::Context as _;
use md5::{Digest as _, Md5};
use niimbot_b1::Firmware;

const STOCK: &str = "B1_6.19_stock.bin";
const DENSITY_COEFF: &str = "B1_6.19_density_coeff.bin";
const ALIGN3: &str = "B1_6.19_density_align3.bin";
const ALIGN7: &str = "B1_6.19_density_align7.bin";
const NOLOCK: &str = "B1_6.19_density_align7_nolock.bin";

pub(crate) fn build(dir: &Path, keep_align3: bool) -> anyhow::Result<()> {
    let stock = read_input(dir, STOCK)?;
    let density_coeff = read_input(dir, DENSITY_COEFF)?;
    let patched = Firmware::build(&stock, &density_coeff)?;
    for (stage, image) in [
        ("align3", &patched.align3[..]),
        ("align7", patched.align7.image()),
        ("nolock", patched.nolock.image()),
    ] {
        println!("{stage}  md5 {}  OK", hex(&Md5::digest(image)));
    }

    if keep_align3 {
        write_output(dir, ALIGN3, &patched.align3)?;
    }
    write_output(dir, ALIGN7, patched.align7.image())?;
    write_output(dir, NOLOCK, patched.nolock.image())
}

fn read_input(dir: &Path, name: &str) -> anyhow::Result<Vec<u8>> {
    let path = dir.join(name);
    fs::read(&path).with_context(|| {
        format!(
            "missing input {}, see the firmware section of the README",
            path.display()
        )
    })
}

fn write_output(dir: &Path, name: &str, image: &[u8]) -> anyhow::Result<()> {
    let path = dir.join(name);
    fs::write(&path, image).with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
            hex
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_input_writes_nothing() {
        let dir = std::env::temp_dir().join("niimbot-b1-missing-input");
        fs::create_dir_all(&dir).unwrap();
        let error = build(&dir, true).unwrap_err();
        assert!(error.to_string().starts_with("missing input"));
        assert!(!dir.join(ALIGN7).exists() && !dir.join(ALIGN3).exists());
    }
}
