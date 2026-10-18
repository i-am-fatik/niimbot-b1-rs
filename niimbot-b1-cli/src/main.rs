//! `niimbot-b1`: print on a Niimbot B1 over USB or Bluetooth LE, and build and flash the patched
//! firmware.

mod firmware;

use std::path::PathBuf;

use anyhow::Context as _;
use clap::{Args, Parser, Subcommand};
use log::LevelFilter;
use niimbot_b1::{
    Bitmap, BleTransport, Firmware, LabelType, PrintOptions, Printer, Raster, SerialTransport,
    Transport,
};

#[derive(Debug, Parser)]
#[command(version, about = "Niimbot B1 label printer client")]
struct Cli {
    #[arg(
        short,
        long,
        global = true,
        conflicts_with = "ble",
        help = "USB serial port [default: autodetect]"
    )]
    port: Option<String>,
    #[arg(
        long,
        global = true,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "",
        value_name = "ADDRESS",
        help = "Use Bluetooth LE, optionally the device address or platform id"
    )]
    ble: Option<String>,
    #[arg(short, long, global = true, help = "Log every packet")]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(flatten)]
    Printer(PrinterCommand),
    #[command(
        about = "Build B1_6.19_density_align7.bin and its _nolock variant from the stock and community firmware"
    )]
    BuildFirmware(FirmwareArgs),
    #[command(
        about = "Flash the stock or a patched 6.19 image over USB, the printer turns off when done"
    )]
    Flash(FlashArgs),
}

#[derive(Debug, Subcommand)]
enum PrinterCommand {
    #[command(about = "Show printer status and RFID roll info")]
    Info,
    #[command(about = "Print an image, its width across the 384 px head")]
    Print(PrintArgs),
    #[command(about = "Measure the label pitch, feeds about 15 cm of media")]
    Calibrate(LabelTypeArg),
}

#[derive(Debug, Args)]
struct PrintArgs {
    #[arg(help = "Image file, PNG, JPEG, GIF, BMP or WebP")]
    image: PathBuf,
    #[arg(short, long, default_value_t = 5, value_parser = clap::value_parser!(u8).range(1..=5), help = "1 light .. 5 dark")]
    density: u8,
    #[arg(short = 'n', long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..))]
    copies: u16,
    #[command(flatten)]
    label_type: LabelTypeArg,
    #[arg(
        long,
        help = "Feed to the next gap first, for stock firmware after closing the cover without a tag"
    )]
    align: bool,
    #[arg(long, help = "Threshold instead of dithering, sharper text")]
    threshold: bool,
}

#[derive(Debug, Args)]
struct LabelTypeArg {
    #[arg(
        short = 't',
        long = "label-type",
        value_name = "TYPE",
        default_value_t = 1,
        help = "1 gap, 2 black mark, 3 continuous, 5 transparent"
    )]
    value: u8,
}

#[derive(Debug, Args)]
struct FirmwareArgs {
    #[arg(
        long,
        default_value = "firmware",
        help = "Directory holding the inputs, receives the output"
    )]
    dir: PathBuf,
    #[arg(long, help = "Also write the intermediate B1_6.19_density_align3.bin")]
    keep_align3: bool,
}

#[derive(Debug, Args)]
struct FlashArgs {
    #[arg(help = "Firmware image, refused unless it is a known B1 6.19 image")]
    image: PathBuf,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    env_logger::Builder::new()
        .filter_module(
            "niimbot_b1",
            if cli.verbose {
                LevelFilter::Debug
            } else {
                LevelFilter::Warn
            },
        )
        .init();
    match cli.command {
        Command::BuildFirmware(args) => firmware::build(&args.dir, args.keep_align3),
        Command::Flash(args) => {
            anyhow::ensure!(cli.ble.is_none(), "flashing runs over USB only");
            let image = std::fs::read(&args.image)
                .with_context(|| format!("reading {}", args.image.display()))?;
            let firmware = Firmware::identify(image).with_context(|| {
                format!("{} is not a known B1 firmware image", args.image.display())
            })?;
            let transport = SerialTransport::open(cli.port.as_deref()).context("use --port")?;
            Printer::new(transport)
                .flash(&firmware, |sent, total| {
                    eprint!("\rflashing {} chunk {sent} of {total}", firmware.name());
                })
                .await?;
            println!(
                "\nflashed {}, the printer turns off, press its button to turn it back on",
                firmware.name()
            );
            Ok(())
        }
        Command::Printer(command) => {
            if let Some(address) = cli.ble {
                let transport =
                    BleTransport::connect(Some(address.as_str()).filter(|a| !a.is_empty())).await?;
                let mut printer = Printer::new(transport);
                let outcome = run(&mut printer, command).await;
                printer.into_transport().close().await?;
                outcome
            } else {
                let transport =
                    SerialTransport::open(cli.port.as_deref()).context("use --port or --ble")?;
                run(&mut Printer::new(transport), command).await
            }
        }
    }
}

async fn run<T: Transport>(
    printer: &mut Printer<T>,
    command: PrinterCommand,
) -> anyhow::Result<()> {
    match command {
        PrinterCommand::Info => {
            println!("{}", printer.heartbeat().await?);
            match printer.rfid_info().await? {
                Some(roll) => println!("{roll}"),
                None => println!("roll without RFID tag"),
            }
        }
        PrinterCommand::Print(args) => {
            let image = image::open(&args.image)
                .with_context(|| format!("reading {}", args.image.display()))?
                .to_rgba8();
            let raster = if args.threshold {
                Raster::Threshold
            } else {
                Raster::Dither
            };
            let (width, height) = (
                usize::try_from(image.width())?,
                usize::try_from(image.height())?,
            );
            let bitmap = Bitmap::from_rgba(width, height, image.as_raw(), raster);
            let label_type = LabelType(args.label_type.value);
            if args.align {
                printer.align(label_type).await?;
            }
            printer
                .print(
                    &bitmap,
                    PrintOptions {
                        density: args.density,
                        copies: args.copies,
                        label_type,
                    },
                )
                .await?;
            println!("printed {}", args.image.display());
        }
        PrinterCommand::Calibrate(label_type) => {
            let accepted = printer.calibrate(LabelType(label_type.value)).await?;
            println!("calibration {}", if accepted { "done" } else { "refused" });
        }
    }
    Ok(())
}
