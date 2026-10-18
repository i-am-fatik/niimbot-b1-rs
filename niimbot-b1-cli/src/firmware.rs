use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, bail, ensure};
use md5::{Digest as _, Md5};

const STOCK: &str = "B1_6.19_stock.bin";
const DENSITY_COEFF: &str = "B1_6.19_density_coeff.bin";
const ALIGN3: &str = "B1_6.19_density_align3.bin";
const ALIGN7: &str = "B1_6.19_density_align7.bin";
const IMAGE_LEN: usize = 124_436;
const ALIGN3_MD5: &str = "78dcde4bd7a4a21dfb965b40d7607cc0";
const ALIGN7_MD5: &str = "540ef521d7416cfe1eb561b6effcb0b4";

const BASE: u32 = 0x0101_0000;

const GAP_SEEK_HELPER: u32 = 0x0101_0140;
const NO_TAG_FALLBACK: u32 = 0x0102_543E;
const DISPATCH_FALLBACK_CALL: u32 = 0x0102_5512;
const ALIGN_TAG_CHECK: u32 = 0x0102_5832;
const ALIGN_FLAG: u32 = 0x0002_0637;

const HEAT_HELPER: u32 = 0x0101_0160;
const FMUL: u32 = 0x0102_6156;
const MULTIPLIER_POOLS: [(u32, usize); 4] = [
    (0x0101_1E60, 16),
    (0x0101_6600, 4),
    (0x0101_6614, 20),
    (0x0101_6D4C, 20),
];
const MOTOR_RAMP: (u32, usize) = (0x0102_D21C, 200);
const UNTOUCHED_LITERAL: (u32, u32) = (0x0101_6610, 0x0002_E48C);
const TAG_STATE: u32 = 0x0002_E458;
const NO_TAG_MULTIPLIER: f32 = 400.0;
const HEAT_MULTIPLY_CALLS: [u32; 18] = [
    0x0101_1D26,
    0x0101_1D10,
    0x0101_1DDC,
    0x0101_1DF2,
    0x0101_6208,
    0x0101_622C,
    0x0101_623A,
    0x0101_6248,
    0x0101_6276,
    0x0101_6284,
    0x0101_6296,
    0x0101_6954,
    0x0101_6A32,
    0x0101_6990,
    0x0101_69D6,
    0x0101_69E4,
    0x0101_69F6,
    0x0101_6A20,
];

const BNE_ALIGN_SKIP: [u8; 2] = [0x1C, 0xD1];
const NOP: [u8; 2] = [0x00, 0xBF];
const PUSH_LR: [u8; 2] = [0x00, 0xB5];
const PUSH_R2_LR: [u8; 2] = [0x04, 0xB5];
const POP_PC: [u8; 2] = [0x00, 0xBD];
const POP_R2_PC: [u8; 2] = [0x04, 0xBD];
const MOVS_R0_1: [u8; 2] = [0x01, 0x20];
const LDR_R1_PC_4: [u8; 2] = [0x01, 0x49];
const LDR_R1_PC_12: [u8; 2] = [0x03, 0x49];
const LDR_R2_PC_16: [u8; 2] = [0x04, 0x4A];
const LDRB_R2_R2_8: [u8; 2] = [0x12, 0x7A];
const STRB_R0_R1: [u8; 2] = [0x08, 0x70];
const CMP_R2_1: [u8; 2] = [0x01, 0x2A];
const BEQ_SKIP_NEXT: [u8; 2] = [0x00, 0xD0];
const PAD: [u8; 2] = [0x00, 0x00];

pub(crate) fn build(dir: &Path, keep_align3: bool) -> anyhow::Result<()> {
    let mut image = read_input(dir, DENSITY_COEFF)?;
    let stock = read_input(dir, STOCK)?;

    seek_gap_without_tag(&mut image)?;
    let align3 = image.clone();
    check_stage("align3", &align3, ALIGN3_MD5)?;

    heat_by_tag(&mut image, &stock)?;
    check_stage("align7", &image, ALIGN7_MD5)?;

    if keep_align3 {
        write_output(dir, ALIGN3, &align3)?;
    }
    write_output(dir, ALIGN7, &image)
}

fn seek_gap_without_tag(image: &mut [u8]) -> anyhow::Result<()> {
    replace(image, ALIGN_TAG_CHECK, &BNE_ALIGN_SKIP, &NOP)?;
    let helper = [
        &PUSH_LR[..],
        &thumb_bl(GAP_SEEK_HELPER + 2, NO_TAG_FALLBACK),
        &MOVS_R0_1,
        &LDR_R1_PC_4,
        &STRB_R0_R1,
        &POP_PC,
        &PAD,
        &ALIGN_FLAG.to_le_bytes(),
    ]
    .concat();
    write_helper(image, GAP_SEEK_HELPER, &helper)?;
    let fallback_call = thumb_bl(DISPATCH_FALLBACK_CALL, NO_TAG_FALLBACK);
    replace(
        image,
        DISPATCH_FALLBACK_CALL,
        &fallback_call,
        &thumb_bl(DISPATCH_FALLBACK_CALL, GAP_SEEK_HELPER),
    )
}

fn heat_by_tag(image: &mut [u8], stock: &[u8]) -> anyhow::Result<()> {
    for (address, len) in MULTIPLIER_POOLS.into_iter().chain([MOTOR_RAMP]) {
        let range = offset(address)..offset(address) + len;
        image[range.clone()].copy_from_slice(&stock[range]);
    }
    let (literal_at, literal) = UNTOUCHED_LITERAL;
    ensure!(
        at(image, literal_at, 4) == literal.to_le_bytes(),
        "literal at {literal_at:#x} changed"
    );

    let helper = [
        &PUSH_R2_LR[..],
        &LDR_R2_PC_16,
        &LDRB_R2_R2_8,
        &CMP_R2_1,
        &BEQ_SKIP_NEXT,
        &LDR_R1_PC_12,
        &thumb_bl(HEAT_HELPER + 0xC, FMUL),
        &POP_R2_PC,
        &PAD,
        &TAG_STATE.to_le_bytes(),
        &NO_TAG_MULTIPLIER.to_bits().to_le_bytes(),
    ]
    .concat();
    write_helper(image, HEAT_HELPER, &helper)?;
    for call in HEAT_MULTIPLY_CALLS {
        replace(
            image,
            call,
            &thumb_bl(call, FMUL),
            &thumb_bl(call, HEAT_HELPER),
        )?;
    }
    Ok(())
}

fn thumb_bl(address: u32, target: u32) -> [u8; 4] {
    let displacement = i64::from(target) - i64::from(address) - 4;
    assert!(
        displacement % 2 == 0 && (-(1 << 24)..1 << 24).contains(&displacement),
        "bl out of range {address:#x} -> {target:#x}"
    );
    let imm25 = u32::try_from(displacement & 0x01FF_FFFF).expect("masked to 25 bits");
    let s = (imm25 >> 24) & 1;
    let j1 = ((imm25 >> 23) & 1) ^ s ^ 1;
    let j2 = ((imm25 >> 22) & 1) ^ s ^ 1;
    let high = 0xF000 | (s << 10) | ((imm25 >> 12) & 0x3FF);
    let low = 0xD000 | (j1 << 13) | (j2 << 11) | ((imm25 >> 1) & 0x7FF);
    let [h0, h1] = u16::try_from(high).expect("16-bit halfword").to_le_bytes();
    let [l0, l1] = u16::try_from(low).expect("16-bit halfword").to_le_bytes();
    [h0, h1, l0, l1]
}

fn offset(address: u32) -> usize {
    usize::try_from(address - BASE).expect("image offsets fit usize")
}

fn at(image: &[u8], address: u32, len: usize) -> &[u8] {
    &image[offset(address)..offset(address) + len]
}

fn replace(image: &mut [u8], address: u32, expected: &[u8], new: &[u8]) -> anyhow::Result<()> {
    let found = at(image, address, expected.len());
    ensure!(
        found == expected,
        "unexpected bytes at {address:#x}: {}, input is not the expected firmware",
        hex(found)
    );
    image[offset(address)..offset(address) + new.len()].copy_from_slice(new);
    Ok(())
}

fn write_helper(image: &mut [u8], address: u32, helper: &[u8]) -> anyhow::Result<()> {
    replace(image, address, &vec![0; helper.len()], helper)
}

fn read_input(dir: &Path, name: &str) -> anyhow::Result<Vec<u8>> {
    let path = dir.join(name);
    let image = fs::read(&path).with_context(|| {
        format!(
            "missing input {}, see the firmware section of the README",
            path.display()
        )
    })?;
    ensure!(
        image.len() == IMAGE_LEN,
        "{name} is {} bytes, expected {IMAGE_LEN}",
        image.len()
    );
    Ok(image)
}

fn check_stage(stage: &str, image: &[u8], expected: &str) -> anyhow::Result<()> {
    let actual = hex(&Md5::digest(image));
    if actual != expected {
        println!("{stage}  md5 {actual}  MISMATCH (expected {expected})");
        bail!("{stage} did not build as expected, the input files may be wrong");
    }
    println!("{stage}  md5 {actual}  OK");
    Ok(())
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
    fn thumb_bl_matches_the_python_encoder() {
        assert_eq!(
            thumb_bl(DISPATCH_FALLBACK_CALL, GAP_SEEK_HELPER),
            [0xEA, 0xF7, 0x15, 0xFE]
        );
        assert_eq!(thumb_bl(HEAT_HELPER + 0xC, FMUL), [0x15, 0xF0, 0xF3, 0xFF]);
        assert_eq!(
            thumb_bl(HEAT_MULTIPLY_CALLS[0], HEAT_HELPER),
            [0xFE, 0xF7, 0x1B, 0xFA]
        );
    }

    #[test]
    fn missing_input_writes_nothing() {
        let dir = std::env::temp_dir().join("niimbot-b1-missing-input");
        fs::create_dir_all(&dir).unwrap();
        let error = build(&dir, true).unwrap_err();
        assert!(error.to_string().starts_with("missing input"));
        assert!(!dir.join(ALIGN7).exists() && !dir.join(ALIGN3).exists());
    }
}
