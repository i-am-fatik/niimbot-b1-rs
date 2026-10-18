use std::fmt;
use std::time::Duration;

use web_time::Instant;

use crate::bitmap::Bitmap;
use crate::error::Error;
use crate::firmware::{self, Firmware};
use crate::protocol::{
    CALIBRATE, CONNECT, Command, Decoder, FIRMWARE_CHUNK, FIRMWARE_COMMIT, FIRMWARE_CRC,
    FIRMWARE_CRC_WANTED, FIRMWARE_END, HEARTBEAT, Hex, LabelType, PAGE_END, PAGE_SIZE, PAGE_START,
    PRINT_END, PRINT_START, PRINT_STATUS, PRINTER_ERROR, PRINTHEAD_PX, Packet, RFID_INFO,
    SET_DENSITY, SET_LABEL_TYPE, START_UPGRADE, chunk_packet, packet, row_packet,
};
use crate::transport::Transport;

const DEFAULT_ARG: &[u8] = &[1];
const ACCEPTED: &[u8] = &[1];
const SINGLE_COLOR: u8 = 0;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const PAGE_END_TIMEOUT: Duration = Duration::from_secs(10);
const CALIBRATE_TIMEOUT: Duration = Duration::from_secs(20);
const PRINT_TIMEOUT: Duration = Duration::from_secs(30);
const FIRMWARE_TIMEOUT: Duration = Duration::from_secs(5);
const FIRMWARE_CHUNK_LEN: usize = 200;
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(300);
const ALIGN_ROWS: usize = 8;

/// Printer state from the heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heartbeat {
    /// The lid is shut.
    pub cover_closed: bool,
    /// Battery level as the printer reports it, a small step count rather than a percentage.
    pub battery: u8,
    /// Media is loaded.
    pub paper_ok: bool,
    /// The roll carries a readable RFID tag.
    pub rfid_ok: bool,
}

impl fmt::Display for Heartbeat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cover = if self.cover_closed { "closed" } else { "open" };
        let paper = if self.paper_ok { "ok" } else { "out" };
        let rfid = if self.rfid_ok { "tag" } else { "no tag" };
        write!(
            f,
            "cover {cover}, paper {paper}, rfid {rfid}, battery {}",
            self.battery
        )
    }
}

/// The RFID record of the inserted roll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RfidInfo {
    /// Tag uuid.
    pub uuid: [u8; 8],
    /// Roll barcode.
    pub barcode: String,
    /// Roll serial number.
    pub serial: String,
    /// Labels on a full roll.
    pub total: u16,
    /// Labels printed, stored on the tag.
    pub used: u16,
    /// Media type of the roll.
    pub label_type: LabelType,
    /// Roll capacity, sent by the B1 only.
    pub capacity: Option<u16>,
    /// Second uuid, sent by the B1 only.
    pub uuid2: Option<[u8; 8]>,
}

impl fmt::Display for RfidInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "roll {} serial {}, used {}/{}",
            self.barcode, self.serial, self.used, self.total
        )?;
        if let Some(capacity) = self.capacity {
            write!(f, ", capacity {capacity}")?;
        }
        write!(f, ", type {}, uuid {}", self.label_type, Hex(&self.uuid))
    }
}

/// Progress of the running print job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrintStatus {
    /// Pages finished.
    pub page: u16,
    /// Print progress of the current page.
    pub print_progress: u8,
    /// Feed progress of the current page.
    pub feed_progress: u8,
}

/// How a bitmap is printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrintOptions {
    /// Darkness from 1, lightest, to 5, darkest.
    pub density: u8,
    /// Number of copies.
    pub copies: u16,
    /// Media in the printer.
    pub label_type: LabelType,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            density: 5,
            copies: 1,
            label_type: LabelType::WITH_GAPS,
        }
    }
}

/// A connection to a Niimbot B1.
///
/// Every method sends requests and waits for the matching response, so one call runs at a
/// time.
#[derive(Debug)]
pub struct Printer<T> {
    transport: T,
    decoder: Decoder,
}

impl<T: Transport> Printer<T> {
    /// Wraps an open transport.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            decoder: Decoder::default(),
        }
    }

    /// Returns the transport, for closing it.
    pub fn into_transport(self) -> T {
        self.transport
    }

    /// The transport, for asking which link it is.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Reads cover, battery, paper and RFID state.
    ///
    /// # Errors
    ///
    /// When the link fails, the printer does not answer, or the answer is too short.
    pub async fn heartbeat(&mut self) -> Result<Heartbeat, Error> {
        let response = self.request(HEARTBEAT, DEFAULT_ARG).await?;
        let Some(&[cover, battery, paper, rfid]) = response.get(9..13) else {
            return Err(Error::malformed(HEARTBEAT.code));
        };
        Ok(Heartbeat {
            cover_closed: cover == 0,
            battery,
            paper_ok: paper == 0,
            rfid_ok: rfid == 1,
        })
    }

    /// Reads the RFID tag of the roll, `None` when the roll has no tag.
    ///
    /// # Errors
    ///
    /// When the link fails, the printer does not answer, or the record is malformed.
    pub async fn rfid_info(&mut self) -> Result<Option<RfidInfo>, Error> {
        let response = self.request(RFID_INFO, DEFAULT_ARG).await?;
        if response.len() == 1 {
            return Ok(None);
        }
        parse_rfid(&response)
            .map(Some)
            .ok_or_else(|| Error::malformed(RFID_INFO.code))
    }

    /// Stores the label type and measures the label pitch, feeding about 15 cm of media. Works
    /// without an RFID tag. Returns whether the printer accepted the calibration.
    ///
    /// # Errors
    ///
    /// When the link fails or the printer does not answer.
    pub async fn calibrate(&mut self, label_type: LabelType) -> Result<bool, Error> {
        let response = self
            .request_within(CALIBRATE, &[label_type.0], CALIBRATE_TIMEOUT)
            .await?;
        Ok(response.first() == Some(&1))
    }

    /// Feeds to the next label gap by printing a blank page, wasting one label. Only needed on
    /// stock firmware after closing the cover on a roll without a tag.
    ///
    /// # Errors
    ///
    /// As [`print`](Self::print).
    pub async fn align(&mut self, label_type: LabelType) -> Result<(), Error> {
        let options = PrintOptions {
            density: 1,
            copies: 1,
            label_type,
        };
        self.print(&Bitmap::blank(PRINTHEAD_PX, ALIGN_ROWS), options)
            .await
    }

    /// Reads the progress of the running job.
    ///
    /// # Errors
    ///
    /// When the link fails, the printer does not answer, or the answer is too short.
    pub async fn print_status(&mut self) -> Result<PrintStatus, Error> {
        let response = self.request(PRINT_STATUS, DEFAULT_ARG).await?;
        let Some(&[page_high, page_low, print_progress, feed_progress]) = response.get(..4) else {
            return Err(Error::malformed(PRINT_STATUS.code));
        };
        Ok(PrintStatus {
            page: u16::from_be_bytes([page_high, page_low]),
            print_progress,
            feed_progress,
        })
    }

    /// Prints `bitmap` as is, its width across the head, and waits until every copy is done.
    ///
    /// # Errors
    ///
    /// When the bitmap is wider than [`PRINTHEAD_PX`], before anything is sent. Otherwise when
    /// the link fails, the printer refuses or does not answer, or the job does not finish.
    pub async fn print(&mut self, bitmap: &Bitmap, options: PrintOptions) -> Result<(), Error> {
        let too_large = || Error::too_large(bitmap.width(), bitmap.height());
        if bitmap.width() > PRINTHEAD_PX {
            return Err(too_large());
        }
        let rows = u16::try_from(bitmap.height()).map_err(|_| too_large())?;
        let columns = u16::try_from(bitmap.width()).map_err(|_| too_large())?;
        let [copies_high, copies_low] = options.copies.to_be_bytes();

        self.request(SET_DENSITY, &[options.density]).await?;
        self.request(SET_LABEL_TYPE, &[options.label_type.0])
            .await?;
        self.request(
            PRINT_START,
            &[copies_high, copies_low, 0, 0, 0, 0, SINGLE_COLOR],
        )
        .await?;
        self.request(PAGE_START, DEFAULT_ARG).await?;
        let page_size = [
            rows.to_be_bytes(),
            columns.to_be_bytes(),
            options.copies.to_be_bytes(),
        ]
        .concat();
        self.request(PAGE_SIZE, &page_size).await?;
        for y in 0..rows {
            self.write(&row_packet(y, bitmap.row(y.into()))).await?;
        }
        self.request_within(PAGE_END, DEFAULT_ARG, PAGE_END_TIMEOUT)
            .await?;
        self.wait_until_printed(options.copies).await?;
        self.request(PRINT_END, DEFAULT_ARG).await?;
        Ok(())
    }

    /// Writes `firmware` to the printer over the B1's upgrade exchange, calling `progress` with
    /// the number of the chunk being sent and the chunk count. The printer turns itself off
    /// when it is done.
    ///
    /// # Errors
    ///
    /// When the link fails, the printer does not answer, or it rejects the image's check or
    /// the commit.
    pub async fn flash(
        &mut self,
        firmware: &Firmware,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<(), Error> {
        let chunks: Vec<&[u8]> = firmware.image.chunks(FIRMWARE_CHUNK_LEN).collect();
        self.request(CONNECT, DEFAULT_ARG).await?;
        self.request(START_UPGRADE, &firmware::VERSION).await?;
        self.receive(START_UPGRADE.code, FIRMWARE_CRC_WANTED, FIRMWARE_TIMEOUT)
            .await?;
        let crc = crc32fast::hash(&firmware.image).to_be_bytes();
        let mut wanted = self.exchange(FIRMWARE_CRC, 0, &crc).await?;
        while let Some(chunk) = chunks.get(usize::from(wanted.chunk)) {
            progress(usize::from(wanted.chunk) + 1, chunks.len());
            wanted = self.exchange(FIRMWARE_CHUNK, wanted.chunk, chunk).await?;
        }
        for step in [FIRMWARE_END, FIRMWARE_COMMIT] {
            let answer = self.exchange(step, 0, DEFAULT_ARG).await?;
            if answer.payload != ACCEPTED {
                return Err(Error::printer(step.code, answer.payload));
            }
        }
        Ok(())
    }

    async fn wait_until_printed(&mut self, copies: u16) -> Result<(), Error> {
        let deadline = Instant::now() + PRINT_TIMEOUT;
        while Instant::now() < deadline {
            let status = self.print_status().await?;
            log::debug!("status {status:?}");
            if status.page >= copies {
                return Ok(());
            }
            self.listen(STATUS_POLL_INTERVAL).await?;
        }
        Err(Error::unfinished())
    }

    async fn request(&mut self, command: Command, payload: &[u8]) -> Result<Vec<u8>, Error> {
        self.request_within(command, payload, REQUEST_TIMEOUT).await
    }

    async fn request_within(
        &mut self,
        command: Command,
        payload: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, Error> {
        self.write(&packet(command.code, payload)).await?;
        let response = self
            .receive(command.code, command.response, timeout)
            .await?;
        Ok(response.payload)
    }

    async fn exchange(
        &mut self,
        command: Command,
        chunk: u16,
        payload: &[u8],
    ) -> Result<Packet, Error> {
        self.write(&chunk_packet(command.code, chunk, payload))
            .await?;
        self.receive(command.code, command.response, FIRMWARE_TIMEOUT)
            .await
    }

    async fn receive(
        &mut self,
        sent: u8,
        expected: u8,
        timeout: Duration,
    ) -> Result<Packet, Error> {
        let deadline = Instant::now() + timeout;
        loop {
            while let Some(response) = self.decoder.next_packet() {
                if response.code == expected {
                    return Ok(response);
                }
                if response.code == PRINTER_ERROR {
                    return Err(Error::printer(sent, response.payload));
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::timeout(sent));
            }
            let bytes = self.transport.read(remaining).await?;
            log::debug!("recv {}", Hex(&bytes));
            self.decoder.push(&bytes);
        }
    }

    async fn listen(&mut self, duration: Duration) -> Result<(), Error> {
        let until = Instant::now() + duration;
        loop {
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            let bytes = self.transport.read(remaining).await?;
            self.decoder.push(&bytes);
        }
    }

    async fn write(&mut self, packet: &[u8]) -> Result<(), Error> {
        log::debug!("send {}", Hex(packet));
        self.transport.write(packet).await
    }
}

fn parse_rfid(record: &[u8]) -> Option<RfidInfo> {
    let (uuid, rest) = record.split_first_chunk::<8>()?;
    let (barcode, rest) = length_prefixed(rest)?;
    let (serial, rest) = length_prefixed(rest)?;
    let (&[total_high, total_low, used_high, used_low, label_type], rest) =
        rest.split_first_chunk::<5>()?;
    let (capacity, rest) = match rest.split_first_chunk::<2>() {
        Some((capacity, rest)) => (Some(u16::from_be_bytes(*capacity)), rest),
        None => (None, rest),
    };
    Some(RfidInfo {
        uuid: *uuid,
        barcode,
        serial,
        total: u16::from_be_bytes([total_high, total_low]),
        used: u16::from_be_bytes([used_high, used_low]),
        label_type: LabelType(label_type),
        capacity,
        uuid2: rest.first_chunk::<8>().copied(),
    })
}

fn length_prefixed(bytes: &[u8]) -> Option<(String, &[u8])> {
    let (&len, rest) = bytes.split_first()?;
    let (text, rest) = rest.split_at_checked(usize::from(len))?;
    Some((String::from_utf8(text.to_vec()).ok()?, rest))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::future::{self, Future};

    use futures::executor::block_on;

    use super::*;
    use crate::bitmap::Raster;
    use crate::protocol::Packet;

    #[derive(Debug, Default)]
    struct Emulator {
        written: Vec<Packet>,
        replies: VecDeque<u8>,
        rfid: Vec<u8>,
        refuse: Option<u8>,
        silent: bool,
        failed_check: bool,
    }

    impl Transport for Emulator {
        fn write(&mut self, bytes: &[u8]) -> impl Future<Output = Result<(), Error>> {
            let mut decoder = Decoder::default();
            decoder.push(bytes);
            let sent = decoder.next_packet().expect("one whole packet per write");
            let reply = match sent.code {
                0x84 | 0x85 => None,
                code if self.refuse == Some(code) => Some(packet(PRINTER_ERROR, &[0x14])),
                0xDC => Some(packet(0xDD, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 1])),
                0x1A => Some(packet(0x1B, &self.rfid)),
                0xA3 => Some(packet(0xB3, &[0, 1, 100, 100])),
                0x21 | 0x23 => Some(packet(sent.code + 0x10, &[1])),
                0xF5 => Some([packet(0xF6, &[1]), chunk_packet(0x90, 0, &[1])].concat()),
                0x91 => Some(chunk_packet(0x9A, 0, &[1])),
                0x9B => Some(chunk_packet(0x9A, sent.chunk + 1, &[1])),
                0x9C => Some(chunk_packet(0x9D, 0, &[u8::from(!self.failed_check)])),
                0x92 => Some(chunk_packet(0x9E, 0, &[1])),
                code => Some(packet(code + 1, &[1])),
            };
            if !self.silent {
                self.replies.extend(reply.into_iter().flatten());
            }
            self.written.push(sent);
            future::ready(Ok(()))
        }

        fn read(&mut self, timeout: Duration) -> impl Future<Output = Result<Vec<u8>, Error>> {
            if self.replies.is_empty() {
                std::thread::sleep(timeout);
            }
            future::ready(Ok(self.replies.drain(..).collect()))
        }
    }

    fn sent_codes(printer: Printer<Emulator>) -> Vec<u8> {
        printer
            .into_transport()
            .written
            .iter()
            .map(|sent| sent.code)
            .collect()
    }

    #[test]
    fn heartbeat_decodes_the_b1_variant() {
        let mut printer = Printer::new(Emulator::default());
        let heartbeat = block_on(printer.heartbeat()).unwrap();
        assert_eq!(
            heartbeat,
            Heartbeat {
                cover_closed: true,
                battery: 3,
                paper_ok: true,
                rfid_ok: true
            }
        );
    }

    #[test]
    fn rfid_without_tag_is_none() {
        let mut printer = Printer::new(Emulator {
            rfid: vec![0],
            ..Emulator::default()
        });
        assert_eq!(block_on(printer.rfid_info()).unwrap(), None);
    }

    #[test]
    fn rfid_decodes_b1_extra_fields() {
        let mut record = vec![1, 2, 3, 4, 5, 6, 7, 8, 4];
        record.extend_from_slice(b"TEST");
        record.push(9);
        record.extend_from_slice(b"ROLL-0001");
        record.extend_from_slice(&[0, 200, 0, 12, 1, 0, 180, 8, 7, 6, 5, 4, 3, 2, 1]);
        let mut printer = Printer::new(Emulator {
            rfid: record,
            ..Emulator::default()
        });
        let info = block_on(printer.rfid_info()).unwrap().unwrap();
        assert_eq!(info.barcode, "TEST");
        assert_eq!(info.serial, "ROLL-0001");
        assert_eq!((info.total, info.used, info.capacity), (200, 12, Some(180)));
        assert_eq!(info.label_type, LabelType::WITH_GAPS);
        assert_eq!(info.uuid2, Some([8, 7, 6, 5, 4, 3, 2, 1]));
    }

    #[test]
    fn rfid_without_b1_fields_has_none() {
        let mut record = vec![1, 2, 3, 4, 5, 6, 7, 8, 1, b'A', 1, b'B'];
        record.extend_from_slice(&[0, 50, 0, 0, 3]);
        let mut printer = Printer::new(Emulator {
            rfid: record,
            ..Emulator::default()
        });
        let info = block_on(printer.rfid_info()).unwrap().unwrap();
        assert_eq!((info.capacity, info.uuid2), (None, None));
    }

    #[test]
    fn print_follows_the_b1_sequence() {
        let mut rgba = [255; 16 * 3 * 4];
        rgba[16 * 4..16 * 4 + 3].fill(0);
        let bitmap = Bitmap::from_rgba(16, 3, &rgba, Raster::Threshold);
        let mut printer = Printer::new(Emulator::default());
        block_on(printer.print(&bitmap, PrintOptions::default())).unwrap();
        assert_eq!(
            sent_codes(printer),
            [
                0x21, 0x23, 0x01, 0x03, 0x13, 0x84, 0x85, 0x84, 0xE3, 0xA3, 0xF3
            ]
        );
    }

    #[test]
    fn print_rejects_wide_bitmap_before_sending() {
        let mut printer = Printer::new(Emulator::default());
        let error =
            block_on(printer.print(&Bitmap::blank(PRINTHEAD_PX + 1, 1), PrintOptions::default()))
                .unwrap_err();
        assert!(error.to_string().contains("does not fit"));
        assert!(sent_codes(printer).is_empty());
    }

    #[test]
    fn printer_error_names_the_command() {
        let mut printer = Printer::new(Emulator {
            refuse: Some(0x01),
            ..Emulator::default()
        });
        let error =
            block_on(printer.print(&Bitmap::blank(8, 1), PrintOptions::default())).unwrap_err();
        assert!(error.is_printer());
        assert!(
            error
                .to_string()
                .starts_with("printer error for command 0x01: 14")
        );
    }

    fn test_firmware(len: usize) -> Firmware {
        Firmware {
            name: "test",
            image: (0..len).map(|at| u8::try_from(at % 251).unwrap()).collect(),
        }
    }

    #[test]
    fn flash_sends_every_chunk_and_commits() {
        let firmware = test_firmware(450);
        let mut printer = Printer::new(Emulator::default());
        let mut counted = Vec::new();
        block_on(printer.flash(&firmware, |sent, total| counted.push((sent, total)))).unwrap();
        assert_eq!(counted, [(1, 3), (2, 3), (3, 3)]);
        let written = printer.into_transport().written;
        let codes: Vec<u8> = written.iter().map(|sent| sent.code).collect();
        assert_eq!(codes, [0xC1, 0xF5, 0x91, 0x9B, 0x9B, 0x9B, 0x9C, 0x92]);
        assert_eq!(written[1].payload, [6, 19]);
        assert_eq!(
            written[2].payload,
            crc32fast::hash(&firmware.image).to_be_bytes()
        );
        let chunks: Vec<&Packet> = written.iter().filter(|sent| sent.code == 0x9B).collect();
        assert_eq!(
            chunks.iter().map(|sent| sent.chunk).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(
            chunks
                .iter()
                .flat_map(|sent| sent.payload.clone())
                .collect::<Vec<_>>(),
            firmware.image
        );
    }

    #[test]
    fn flash_stops_when_the_check_fails() {
        let mut printer = Printer::new(Emulator {
            failed_check: true,
            ..Emulator::default()
        });
        let error = block_on(printer.flash(&test_firmware(10), |_, _| {})).unwrap_err();
        assert!(error.is_printer());
        assert!(
            error
                .to_string()
                .starts_with("printer error for command 0x9c: 00")
        );
        assert!(!sent_codes(printer).contains(&0x92));
    }

    #[test]
    fn silence_times_out() {
        let mut printer = Printer::new(Emulator {
            silent: true,
            ..Emulator::default()
        });
        let error = block_on(printer.heartbeat()).unwrap_err();
        assert!(error.is_timeout());
        assert!(error.to_string().starts_with("no response to command 0xdc"));
    }
}
