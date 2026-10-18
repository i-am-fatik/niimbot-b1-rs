use std::fmt;
use std::ops::RangeInclusive;

/// Width of the print head in dots: 48 mm at 203 dpi.
pub const PRINTHEAD_PX: usize = 384;

const HEADER: [u8; 2] = [0x55, 0x55];
const TRAILER: [u8; 2] = [0xAA, 0xAA];
const FRAMING_LEN: usize = HEADER.len() + 2 + 1 + TRAILER.len();
const CHUNK_FRAMING_LEN: usize = HEADER.len() + 4 + 4 + TRAILER.len();
const FIRMWARE_EXCHANGE: RangeInclusive<u8> = 0x90..=0x9E;
const ROW_SLICE_BYTES: usize = PRINTHEAD_PX / 8 / 3;
const ROW_REPEAT: u8 = 1;

/// The kind of media in the printer, as the `0x23` command and the RFID tag encode it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LabelType(pub u8);

impl LabelType {
    /// Die-cut labels separated by gaps, the common roll.
    pub const WITH_GAPS: Self = Self(1);
    /// Labels with a black mark.
    pub const BLACK: Self = Self(2);
    /// Continuous thermal paper.
    pub const CONTINUOUS: Self = Self(3);
    /// Perforated labels.
    pub const PERFORATED: Self = Self(4);
    /// Transparent labels.
    pub const TRANSPARENT: Self = Self(5);
    /// PVC tags.
    pub const PVC_TAG: Self = Self(6);
    /// Black-mark labels with gaps.
    pub const BLACK_MARK_GAP: Self = Self(10);
    /// Heat-shrink tube.
    pub const HEAT_SHRINK_TUBE: Self = Self(11);

    /// The protocol's name for this type, `None` for a value the protocol does not define.
    #[must_use]
    pub fn name(self) -> Option<&'static str> {
        match self {
            Self::WITH_GAPS => Some("with_gaps"),
            Self::BLACK => Some("black"),
            Self::CONTINUOUS => Some("continuous"),
            Self::PERFORATED => Some("perforated"),
            Self::TRANSPARENT => Some("transparent"),
            Self::PVC_TAG => Some("pvc_tag"),
            Self::BLACK_MARK_GAP => Some("black_mark_gap"),
            Self::HEAT_SHRINK_TUBE => Some("heat_shrink_tube"),
            _ => None,
        }
    }
}

impl Default for LabelType {
    fn default() -> Self {
        Self::WITH_GAPS
    }
}

impl fmt::Display for LabelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => write!(f, "{}", self.0),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Command {
    pub(crate) code: u8,
    pub(crate) response: u8,
}

pub(crate) const PRINT_START: Command = Command {
    code: 0x01,
    response: 0x02,
};
pub(crate) const PAGE_START: Command = Command {
    code: 0x03,
    response: 0x04,
};
pub(crate) const PAGE_SIZE: Command = Command {
    code: 0x13,
    response: 0x14,
};
pub(crate) const RFID_INFO: Command = Command {
    code: 0x1A,
    response: 0x1B,
};
pub(crate) const SET_DENSITY: Command = Command {
    code: 0x21,
    response: 0x31,
};
pub(crate) const SET_LABEL_TYPE: Command = Command {
    code: 0x23,
    response: 0x33,
};
pub(crate) const CALIBRATE: Command = Command {
    code: 0x8E,
    response: 0x8F,
};
pub(crate) const PRINT_STATUS: Command = Command {
    code: 0xA3,
    response: 0xB3,
};
pub(crate) const HEARTBEAT: Command = Command {
    code: 0xDC,
    response: 0xDD,
};
pub(crate) const PAGE_END: Command = Command {
    code: 0xE3,
    response: 0xE4,
};
pub(crate) const PRINT_END: Command = Command {
    code: 0xF3,
    response: 0xF4,
};
pub(crate) const CONNECT: Command = Command {
    code: 0xC1,
    response: 0xC2,
};
pub(crate) const START_UPGRADE: Command = Command {
    code: 0xF5,
    response: 0xF6,
};
pub(crate) const FIRMWARE_CRC_WANTED: u8 = 0x90;
pub(crate) const FIRMWARE_CRC: Command = Command {
    code: 0x91,
    response: 0x9A,
};
pub(crate) const FIRMWARE_CHUNK: Command = Command {
    code: 0x9B,
    response: 0x9A,
};
pub(crate) const FIRMWARE_END: Command = Command {
    code: 0x9C,
    response: 0x9D,
};
pub(crate) const FIRMWARE_COMMIT: Command = Command {
    code: 0x92,
    response: 0x9E,
};

const EMPTY_ROW: u8 = 0x84;
const BITMAP_ROW: u8 = 0x85;
pub(crate) const PRINTER_ERROR: u8 = 0xDB;

pub(crate) fn packet(code: u8, payload: &[u8]) -> Vec<u8> {
    let len = u8::try_from(payload.len()).expect("payload fits one packet");
    let checksum = payload.iter().fold(code ^ len, |sum, byte| sum ^ byte);
    let mut packet = Vec::with_capacity(payload.len() + FRAMING_LEN);
    packet.extend_from_slice(&HEADER);
    packet.extend_from_slice(&[code, len]);
    packet.extend_from_slice(payload);
    packet.push(checksum);
    packet.extend_from_slice(&TRAILER);
    packet
}

pub(crate) fn chunk_packet(code: u8, chunk: u16, payload: &[u8]) -> Vec<u8> {
    let len = u8::try_from(payload.len()).expect("payload fits one packet");
    let mut packet = Vec::with_capacity(payload.len() + CHUNK_FRAMING_LEN);
    packet.extend_from_slice(&HEADER);
    packet.push(code);
    packet.extend_from_slice(&chunk.to_be_bytes());
    packet.push(len);
    packet.extend_from_slice(payload);
    let checksum = crc32fast::hash(&packet[HEADER.len()..]);
    packet.extend_from_slice(&checksum.to_be_bytes());
    packet.extend_from_slice(&TRAILER);
    packet
}

pub(crate) fn row_packet(y: u16, row: &[u8]) -> Vec<u8> {
    let [y_high, y_low] = y.to_be_bytes();
    if row.iter().all(|&byte| byte == 0) {
        return packet(EMPTY_ROW, &[y_high, y_low, ROW_REPEAT]);
    }
    let mut dots = [0; 3];
    for (count, slice) in dots.iter_mut().zip(row.chunks(ROW_SLICE_BYTES)) {
        *count = u8::try_from(slice.iter().map(|byte| byte.count_ones()).sum::<u32>())
            .expect("a row slice holds at most 128 dots");
    }
    let mut payload = vec![y_high, y_low, dots[0], dots[1], dots[2], ROW_REPEAT];
    payload.extend_from_slice(row);
    packet(BITMAP_ROW, &payload)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Packet {
    pub(crate) code: u8,
    pub(crate) chunk: u16,
    pub(crate) payload: Vec<u8>,
}

#[derive(Debug, Default)]
pub(crate) struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    pub(crate) fn next_packet(&mut self) -> Option<Packet> {
        let Some(start) = self.buffer.windows(2).position(|pair| pair == HEADER) else {
            let partial_header = self.buffer.len().saturating_sub(1);
            self.buffer.drain(..partial_header);
            return None;
        };
        self.buffer.drain(..start);
        let code = *self.buffer.get(2)?;
        let chunked = FIRMWARE_EXCHANGE.contains(&code);
        let (len_at, framing) = if chunked {
            (5, CHUNK_FRAMING_LEN)
        } else {
            (3, FRAMING_LEN)
        };
        let len = usize::from(*self.buffer.get(len_at)?);
        if self.buffer.len() < len + framing {
            return None;
        }
        let frame: Vec<u8> = self.buffer.drain(..len + framing).collect();
        Some(Packet {
            code,
            chunk: if chunked {
                u16::from_be_bytes([frame[3], frame[4]])
            } else {
                0
            },
            payload: frame[len_at + 1..len_at + 1 + len].to_vec(),
        })
    }
}

pub(crate) struct Hex<'a>(pub(crate) &'a [u8]);

impl fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_frames_payload_with_xor_checksum() {
        assert_eq!(
            packet(0x21, &[5]),
            [0x55, 0x55, 0x21, 0x01, 0x05, 0x25, 0xAA, 0xAA]
        );
    }

    #[test]
    fn empty_row_is_short_packet() {
        assert_eq!(
            row_packet(0x0102, &[0; 48]),
            packet(EMPTY_ROW, &[0x01, 0x02, ROW_REPEAT])
        );
    }

    #[test]
    fn bitmap_row_counts_dots_per_third_of_the_head() {
        let mut row = [0; 48];
        row[0] = 0xFF;
        row[16] = 0x01;
        row[47] = 0x03;
        let sent = row_packet(7, &row);
        assert_eq!(sent[2], BITMAP_ROW);
        assert_eq!(&sent[4..10], &[0, 7, 8, 1, 2, ROW_REPEAT]);
        assert_eq!(&sent[10..58], &row);
    }

    #[test]
    fn decoder_skips_noise_and_waits_for_whole_frame() {
        let frame = packet(0xDD, &[1, 2, 3]);
        let mut decoder = Decoder::default();
        decoder.push(&[0x00, 0x13]);
        decoder.push(&frame[..5]);
        assert_eq!(decoder.next_packet(), None);
        decoder.push(&frame[5..]);
        assert_eq!(
            decoder.next_packet(),
            Some(Packet {
                code: 0xDD,
                chunk: 0,
                payload: vec![1, 2, 3]
            })
        );
        assert_eq!(decoder.next_packet(), None);
    }

    #[test]
    fn decoder_keeps_a_split_header() {
        let frame = packet(0x31, &[1]);
        let mut decoder = Decoder::default();
        decoder.push(&[0x07, 0x55]);
        assert_eq!(decoder.next_packet(), None);
        decoder.push(&frame[1..]);
        assert_eq!(
            decoder.next_packet(),
            Some(Packet {
                code: 0x31,
                chunk: 0,
                payload: vec![1]
            })
        );
    }

    #[test]
    fn firmware_packets_carry_a_chunk_number_and_a_crc32() {
        let frame = chunk_packet(0x9B, 2, &[0xAB, 0xCD]);
        assert_eq!(
            frame,
            [
                0x55, 0x55, 0x9B, 0x00, 0x02, 0x02, 0xAB, 0xCD, 0xC7, 0xC8, 0x80, 0xE9, 0xAA, 0xAA
            ]
        );
        let mut decoder = Decoder::default();
        decoder.push(&frame[..7]);
        assert_eq!(decoder.next_packet(), None);
        decoder.push(&frame[7..]);
        decoder.push(&packet(0x31, &[1]));
        assert_eq!(
            decoder.next_packet(),
            Some(Packet {
                code: 0x9B,
                chunk: 2,
                payload: vec![0xAB, 0xCD]
            })
        );
        assert_eq!(decoder.next_packet().map(|next| next.code), Some(0x31));
    }

    #[test]
    fn label_type_names_unknown_values_by_number() {
        assert_eq!(LabelType::WITH_GAPS.to_string(), "with_gaps");
        assert_eq!(LabelType(42).to_string(), "42");
    }
}
