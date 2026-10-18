use std::fmt;

use md5::{Digest as _, Md5};

use crate::firmware::Firmware;
use crate::protocol::Hex;

const IMAGE_LEN: usize = 124_436;
const ALIGN3_MD5: &str = "78dcde4bd7a4a21dfb965b40d7607cc0";

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
const ROLL_REJECTS: [(u32, [u8; 2], [u8; 2]); 4] = [
    (0x0102_4E8A, MOVS_R3_1, MOVS_R3_0),
    (0x0102_4EC8, MOVS_R2_1, MOVS_R2_0),
    (0x0102_5012, MOVS_R2_1, MOVS_R2_0),
    (0x0102_59BA, MOVS_R2_1, MOVS_R2_0),
];
const OVERUSE_DOWNGRADES: [(u32, [u8; 2]); 2] =
    [(0x0101_6126, [0x01, 0xD8]), (0x0101_DB42, [0x0C, 0xD8])];

const BNE_ALIGN_SKIP: [u8; 2] = [0x1C, 0xD1];
const NOP: [u8; 2] = [0x00, 0xBF];
const PUSH_LR: [u8; 2] = [0x00, 0xB5];
const PUSH_R2_LR: [u8; 2] = [0x04, 0xB5];
const POP_PC: [u8; 2] = [0x00, 0xBD];
const POP_R2_PC: [u8; 2] = [0x04, 0xBD];
const MOVS_R0_1: [u8; 2] = [0x01, 0x20];
const MOVS_R2_0: [u8; 2] = [0x00, 0x22];
const MOVS_R2_1: [u8; 2] = [0x01, 0x22];
const MOVS_R3_0: [u8; 2] = [0x00, 0x23];
const MOVS_R3_1: [u8; 2] = [0x01, 0x23];
const LDR_R1_PC_4: [u8; 2] = [0x01, 0x49];
const LDR_R1_PC_12: [u8; 2] = [0x03, 0x49];
const LDR_R2_PC_16: [u8; 2] = [0x04, 0x4A];
const LDRB_R2_R2_8: [u8; 2] = [0x12, 0x7A];
const STRB_R0_R1: [u8; 2] = [0x08, 0x70];
const CMP_R2_1: [u8; 2] = [0x01, 0x2A];
const BEQ_SKIP_NEXT: [u8; 2] = [0x00, 0xD0];
const PAD: [u8; 2] = [0x00, 0x00];

/// The images [`Firmware::build`] makes from the stock B1 6.19 firmware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patched {
    /// The intermediate image that seeks the label gap without a tag.
    pub align3: Vec<u8>,
    /// Seeks the gap and prints at full darkness without a tag.
    pub align7: Firmware,
    /// As `align7`, and never refuses a tagged roll with error `0x14`.
    pub nolock: Firmware,
}

/// Why [`Firmware::build`] could not build the patched images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchError(String);

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PatchError {}

impl Firmware {
    /// Builds the patched images from the stock B1 6.19 firmware and the community density
    /// image `B1_6.19_density_coeff.bin`, checking every stage by MD5.
    ///
    /// # Errors
    ///
    /// When either input is not the expected firmware.
    pub fn build(stock: &[u8], density_coeff: &[u8]) -> Result<Patched, PatchError> {
        for (name, input) in [("stock", stock), ("density_coeff", density_coeff)] {
            if input.len() != IMAGE_LEN {
                return Err(PatchError(format!(
                    "{name} is {} bytes, expected {IMAGE_LEN}",
                    input.len()
                )));
            }
        }
        let mut image = density_coeff.to_vec();
        seek_gap_without_tag(&mut image)?;
        let align3 = image.clone();
        let align3_md5 = Hex(&Md5::digest(&align3)).to_string();
        if align3_md5 != ALIGN3_MD5 {
            return Err(stage_failed("align3"));
        }
        heat_by_tag(&mut image, stock)?;
        let mut nolock = image.clone();
        never_reject_roll(&mut nolock)?;
        Ok(Patched {
            align3,
            align7: Firmware::identify(image).ok_or_else(|| stage_failed("align7"))?,
            nolock: Firmware::identify(nolock).ok_or_else(|| stage_failed("nolock"))?,
        })
    }
}

fn stage_failed(stage: &str) -> PatchError {
    PatchError(format!(
        "{stage} did not build as expected, the input files may be wrong"
    ))
}

fn never_reject_roll(image: &mut [u8]) -> Result<(), PatchError> {
    for (address, flag_set, no_flag) in ROLL_REJECTS {
        replace(image, address, &flag_set, &no_flag)?;
    }
    for (address, branch) in OVERUSE_DOWNGRADES {
        replace(image, address, &branch, &NOP)?;
    }
    Ok(())
}

fn seek_gap_without_tag(image: &mut [u8]) -> Result<(), PatchError> {
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

fn heat_by_tag(image: &mut [u8], stock: &[u8]) -> Result<(), PatchError> {
    for (address, len) in MULTIPLIER_POOLS.into_iter().chain([MOTOR_RAMP]) {
        let range = offset(address)..offset(address) + len;
        image[range.clone()].copy_from_slice(&stock[range]);
    }
    let (literal_at, literal) = UNTOUCHED_LITERAL;
    if at(image, literal_at, 4) != literal.to_le_bytes() {
        return Err(PatchError(format!("literal at {literal_at:#x} changed")));
    }

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

fn replace(image: &mut [u8], address: u32, expected: &[u8], new: &[u8]) -> Result<(), PatchError> {
    let found = at(image, address, expected.len());
    if found != expected {
        return Err(PatchError(format!(
            "unexpected bytes at {address:#x}: {}, input is not the expected firmware",
            Hex(found)
        )));
    }
    image[offset(address)..offset(address) + new.len()].copy_from_slice(new);
    Ok(())
}

fn write_helper(image: &mut [u8], address: u32, helper: &[u8]) -> Result<(), PatchError> {
    replace(image, address, &vec![0; helper.len()], helper)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    const STOCK: &str = "B1_6.19_stock.bin";
    const DENSITY_COEFF: &str = "B1_6.19_density_coeff.bin";
    const FIRMWARE_DIR: &str = "../firmware";
    const RAM_LEN: u32 = 0x0004_0000;
    const TAG_USED: u32 = 0x0002_E458 + 0x0E;
    const TAG_TOTAL: u32 = 0x0002_E458 + 0x4E;
    const OVERUSE_FLAG_WORD: u32 = 0x0002_0614 + 0x14;
    const OVERUSE_FLAG: u32 = 1 << 19;
    const COUNT_LABEL_FN: u32 = 0x0102_4E52;
    const PAST_THE_LIMIT_CHECK: u32 = 0x0102_4E94;

    struct Core<'a> {
        reg: [u32; 16],
        carry: bool,
        zero: bool,
        image: &'a [u8],
        ram: Vec<u8>,
    }

    impl<'a> Core<'a> {
        fn new(image: &'a [u8]) -> Self {
            Self {
                reg: [0; 16],
                carry: false,
                zero: false,
                image,
                ram: vec![0; RAM_LEN as usize],
            }
        }

        fn load_u8(&self, address: u32) -> u8 {
            if address >= BASE {
                self.image[(address - BASE) as usize]
            } else {
                self.ram[address as usize]
            }
        }

        fn load_u16(&self, address: u32) -> u32 {
            u32::from(u16::from_le_bytes([
                self.load_u8(address),
                self.load_u8(address + 1),
            ]))
        }

        fn load_u32(&self, address: u32) -> u32 {
            u32::from_le_bytes([
                self.load_u8(address),
                self.load_u8(address + 1),
                self.load_u8(address + 2),
                self.load_u8(address + 3),
            ])
        }

        fn store_u8(&mut self, address: u32, value: u8) {
            self.ram[address as usize] = value;
        }

        fn store_u16(&mut self, address: u32, value: u16) {
            let [low, high] = value.to_le_bytes();
            self.store_u8(address, low);
            self.store_u8(address + 1, high);
        }

        fn store_u32(&mut self, address: u32, value: u32) {
            let [a, b, c, d] = value.to_le_bytes();
            self.store_u8(address, a);
            self.store_u8(address + 1, b);
            self.store_u8(address + 2, c);
            self.store_u8(address + 3, d);
        }

        fn run(&mut self, entry: u32, stop: u32) {
            let mut pc = entry;
            for _ in 0..1000 {
                if pc == stop {
                    return;
                }
                pc = self.step(pc);
            }
            panic!("firmware function did not reach {stop:#x}");
        }

        fn step(&mut self, pc: u32) -> u32 {
            let op = u16::try_from(self.load_u16(pc)).expect("16-bit opcode");
            let field = |shift: u32| usize::from((op >> shift) & 7);
            let mut next = pc + 2;
            match op >> 12 {
                0 | 1 if op & 0xFE00 == 0x1800 => {
                    self.reg[field(0)] = self.reg[field(3)].wrapping_add(self.reg[field(6)]);
                }
                0 | 1 if op & 0xFE00 == 0x1C00 => {
                    self.reg[field(0)] = self.reg[field(3)].wrapping_add(u32::from((op >> 6) & 7));
                }
                0 | 1 if op & 0xF800 == 0 => {
                    self.reg[field(0)] = self.reg[field(3)] << ((op >> 6) & 0x1F);
                }
                2 | 3 if op & 0x0800 == 0 => {
                    self.reg[usize::from((op >> 8) & 7)] = u32::from(op & 0xFF);
                }
                2 | 3 => {
                    let rd = usize::from((op >> 8) & 7);
                    self.reg[rd] = self.reg[rd].wrapping_add(u32::from(op & 0xFF));
                }
                4 if op >> 11 == 0b0_1001 => {
                    let base = (pc + 4) & !3;
                    self.reg[usize::from((op >> 8) & 7)] =
                        self.load_u32(base + u32::from((op & 0xFF) << 2));
                }
                4 if op >> 10 == 0b01_0000 => {
                    let rm = self.reg[field(3)];
                    let rdn = field(0);
                    match (op >> 6) & 0xF {
                        0b1010 => {
                            self.zero = self.reg[rdn] == rm;
                            self.carry = self.reg[rdn] >= rm;
                        }
                        0b1100 => self.reg[rdn] |= rm,
                        0b1101 => self.reg[rdn] = self.reg[rdn].wrapping_mul(rm),
                        other => panic!("unsupported alu op {other:#x} at {pc:#x}"),
                    }
                }
                4 if op >> 8 == 0x46 => {
                    let rd = usize::from(((op >> 4) & 8) | (op & 7));
                    self.reg[rd] = self.reg[usize::from((op >> 3) & 0xF)];
                }
                5 if op & 0xFE00 == 0x5A00 => {
                    let address = self.reg[field(3)].wrapping_add(self.reg[field(6)]);
                    self.reg[field(0)] = self.load_u16(address);
                }
                6 => {
                    let address = self.reg[field(3)].wrapping_add(u32::from((op >> 6) & 0x1F) << 2);
                    if op & 0x0800 == 0 {
                        self.store_u32(address, self.reg[field(0)]);
                    } else {
                        self.reg[field(0)] = self.load_u32(address);
                    }
                }
                7 => {
                    let address = self.reg[field(3)].wrapping_add(u32::from((op >> 6) & 0x1F));
                    if op & 0x0800 == 0 {
                        let byte = u8::try_from(self.reg[field(0)] & 0xFF).expect("byte");
                        self.store_u8(address, byte);
                    } else {
                        self.reg[field(0)] = u32::from(self.load_u8(address));
                    }
                }
                8 => {
                    let address = self.reg[field(3)].wrapping_add(u32::from((op >> 6) & 0x1F) << 1);
                    if op & 0x0800 == 0 {
                        let half = u16::try_from(self.reg[field(0)] & 0xFFFF).expect("halfword");
                        self.store_u16(address, half);
                    } else {
                        self.reg[field(0)] = self.load_u16(address);
                    }
                }
                0xB if op & 0xFE00 == 0xB400 => {
                    let with_lr = u32::from((op >> 8) & 1);
                    let count = (op & 0xFF).count_ones() + with_lr;
                    self.reg[13] = self.reg[13].wrapping_sub(4 * count);
                    let mut address = self.reg[13];
                    for r in 0u16..8 {
                        if op & (1 << r) != 0 {
                            self.store_u32(address, self.reg[usize::from(r)]);
                            address += 4;
                        }
                    }
                    if with_lr == 1 {
                        self.store_u32(address, self.reg[14]);
                    }
                }
                0xD if (op >> 8) & 0xF == 0b1001 => {
                    let raw = i32::from(op & 0xFF);
                    let offset = if raw >= 0x80 { raw - 0x100 } else { raw };
                    if !self.carry || self.zero {
                        next = (pc + 4).wrapping_add_signed(offset * 2);
                    }
                }
                _ => panic!("unsupported opcode {op:#06x} at {pc:#x}"),
            }
            next
        }
    }

    fn refuses_to_print(image: &[u8], used: u16, total: u16) -> bool {
        let mut core = Core::new(image);
        core.store_u16(TAG_USED, used);
        core.store_u16(TAG_TOTAL, total);
        core.reg[13] = RAM_LEN;
        core.run(COUNT_LABEL_FN, PAST_THE_LIMIT_CHECK);
        core.load_u32(OVERUSE_FLAG_WORD) & OVERUSE_FLAG != 0
    }

    fn read_input(name: &str) -> Option<Vec<u8>> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(FIRMWARE_DIR)
            .join(name);
        let image = fs::read(path).ok()?;
        (image.len() == IMAGE_LEN).then_some(image)
    }

    #[test]
    fn nolock_firmware_prints_a_roll_the_stock_firmware_refuses() {
        let Some(stock) = read_input(STOCK) else {
            eprintln!("skipping: firmware/{STOCK} not present");
            return;
        };
        let mut nolock = stock.clone();
        never_reject_roll(&mut nolock).expect("patch the stock image");

        assert!(refuses_to_print(&stock, 277, 276));
        assert!(!refuses_to_print(&nolock, 277, 276));
        assert!(!refuses_to_print(&stock, 100, 276));
        assert!(!refuses_to_print(&nolock, 100, 276));
    }

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
    fn build_reproduces_the_published_images() {
        let (Some(stock), Some(density_coeff)) = (read_input(STOCK), read_input(DENSITY_COEFF))
        else {
            eprintln!("skipping: firmware/{STOCK} or firmware/{DENSITY_COEFF} not present");
            return;
        };
        let patched = Firmware::build(&stock, &density_coeff).unwrap();
        assert_eq!(Hex(&Md5::digest(&patched.align3)).to_string(), ALIGN3_MD5);
        assert_eq!(patched.align7.name(), "B1 6.19 patched");
        assert_eq!(patched.nolock.name(), "B1 6.19 patched, never locks");
    }

    #[test]
    fn wrong_inputs_are_refused() {
        let blank = vec![0; IMAGE_LEN];
        let short = Firmware::build(&blank[..10], &blank).unwrap_err();
        assert_eq!(short.to_string(), "stock is 10 bytes, expected 124436");
        let unpatchable = Firmware::build(&blank, &blank).unwrap_err();
        assert!(
            unpatchable
                .to_string()
                .ends_with("input is not the expected firmware")
        );
    }
}
