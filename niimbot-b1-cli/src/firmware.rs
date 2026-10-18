use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, bail, ensure};
use md5::{Digest as _, Md5};

const STOCK: &str = "B1_6.19_stock.bin";
const DENSITY_COEFF: &str = "B1_6.19_density_coeff.bin";
const ALIGN3: &str = "B1_6.19_density_align3.bin";
const ALIGN7: &str = "B1_6.19_density_align7.bin";
const NOLOCK: &str = "B1_6.19_density_align7_nolock.bin";
const IMAGE_LEN: usize = 124_436;
const ALIGN3_MD5: &str = "78dcde4bd7a4a21dfb965b40d7607cc0";
const ALIGN7_MD5: &str = "540ef521d7416cfe1eb561b6effcb0b4";
const NOLOCK_MD5: &str = "6fd1612be6fb0cfb0ffb27808df3c44c";

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

pub(crate) fn build(dir: &Path, keep_align3: bool) -> anyhow::Result<()> {
    let mut image = read_input(dir, DENSITY_COEFF)?;
    let stock = read_input(dir, STOCK)?;

    seek_gap_without_tag(&mut image)?;
    let align3 = image.clone();
    check_stage("align3", &align3, ALIGN3_MD5)?;

    heat_by_tag(&mut image, &stock)?;
    check_stage("align7", &image, ALIGN7_MD5)?;
    let mut nolock = image.clone();
    never_reject_roll(&mut nolock)?;
    check_stage("nolock", &nolock, NOLOCK_MD5)?;

    if keep_align3 {
        write_output(dir, ALIGN3, &align3)?;
    }
    write_output(dir, ALIGN7, &image)?;
    write_output(dir, NOLOCK, &nolock)
}

fn never_reject_roll(image: &mut [u8]) -> anyhow::Result<()> {
    for (address, flag_set, no_flag) in ROLL_REJECTS {
        replace(image, address, &flag_set, &no_flag)?;
    }
    for (address, branch) in OVERUSE_DOWNGRADES {
        replace(image, address, &branch, &NOP)?;
    }
    Ok(())
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

    fn read_stock() -> Option<Vec<u8>> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(FIRMWARE_DIR)
            .join(STOCK);
        let image = fs::read(path).ok()?;
        (image.len() == IMAGE_LEN).then_some(image)
    }

    #[test]
    fn nolock_firmware_prints_a_roll_the_stock_firmware_refuses() {
        let Some(stock) = read_stock() else {
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
    fn missing_input_writes_nothing() {
        let dir = std::env::temp_dir().join("niimbot-b1-missing-input");
        fs::create_dir_all(&dir).unwrap();
        let error = build(&dir, true).unwrap_err();
        assert!(error.to_string().starts_with("missing input"));
        assert!(!dir.join(ALIGN7).exists() && !dir.join(ALIGN3).exists());
    }
}
