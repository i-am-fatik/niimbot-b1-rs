# Niimbot B1 in Rust

A Rust port of the Python client in [vkolarik/niimbot-b1](https://github.com/vkolarik/niimbot-b1): one protocol library, a command-line tool, and an installable web app that prints custom labels from Chrome or Edge over USB or Bluetooth. It writes the same bytes as the Python client and builds the same patched firmware.

| Crate | What it is |
|---|---|
| `niimbot-b1` | Library: status, RFID, calibration, printing a 1-bit bitmap over any `Transport` |
| `niimbot-b1-cli` | The `niimbot-b1` binary: `info`, `print`, `calibrate`, `build-firmware` |
| `niimbot-b1-pwa` | Web app: compose a label, preview the exact dots, print over Web Serial or Web Bluetooth |

## CLI

```sh
cargo build --release -p niimbot-b1-cli
./target/release/niimbot-b1 info
./target/release/niimbot-b1 print label.png -d 5
./target/release/niimbot-b1 --ble info
```

| Option | Default | Meaning |
|---|---|---|
| `-p, --port` | autodetect | USB serial port, `/dev/cu.usbmodemB1_<serial>` on macOS, `/dev/ttyACM*` on Linux |
| `--ble[=ADDRESS]` | off | Bluetooth LE, optionally a device address or platform id |
| `-v, --verbose` | off | Log every packet |
| `print -d, --density` | `5` | `1` light to `5` dark |
| `print -n, --copies` | `1` | Copies |
| `print -t, --label-type`, `calibrate -t` | `1` | `1` gap, `2` black mark, `3` continuous, `5` transparent |
| `print --align` | off | Feed to the next gap first, for stock firmware after closing the cover without a tag |
| `print --threshold` | off | Threshold instead of the Pillow-identical dithering |
| `build-firmware --dir` | `firmware` | Directory with the two inputs, receives the output |
| `build-firmware --keep-align3` | off | Also write `B1_6.19_density_align3.bin` |

The image prints as is, its width across the 384 dot head. macOS asks for Bluetooth permission on the first `--ble` run, see [bluetooth.md](https://github.com/vkolarik/niimbot-b1/blob/main/docs/bluetooth.md).

## Firmware

```sh
mkdir -p firmware
curl -fsSL -o firmware/B1_6.19_stock.bin https://fw.niim.blue/stable/B1/B1_6.19.bin
curl -fsSL -o firmware/B1_6.19_density_coeff.bin https://raw.githubusercontent.com/lotabout/niimbot/niimbot-hw6.01/firmware/B1_6.19_density_coeff.bin
./target/release/niimbot-b1 build-firmware --dir firmware
```

Writes `firmware/B1_6.19_density_align7.bin`, MD5 `540ef521d7416cfe1eb561b6effcb0b4`, only when both inputs are the expected firmware. Next to it goes `firmware/B1_6.19_density_align7_nolock.bin`, MD5 `6fd1612be6fb0cfb0ffb27808df3c44c`, the same image that never refuses a tagged roll with error `0x14`, so a used up, cloned or unwritable tag still prints at full darkness. What the patch does, how to flash it and how to roll back is in [firmware/README.md](https://github.com/vkolarik/niimbot-b1/blob/main/firmware/README.md). Flashing can brick the printer and voids the warranty.

## Web app

```sh
wasm-pack build niimbot-b1-pwa --release --target web --out-dir www/pkg --no-typescript --no-pack
python3 -m http.server 8787 -d niimbot-b1-pwa/www
```

Open `http://localhost:8787` in Chrome or Edge, press Connect USB or Connect Bluetooth, pick the printer, press Print. Web Serial and Web Bluetooth only run on `https` or `localhost`. After the first visit the app installs from the address bar and opens offline. The label, its settings and its images save themselves in the browser as you edit and come back after a reload, on that browser only. To use it on other machines, serve `niimbot-b1-pwa/www` from any static `https` host.

| Field | Meaning |
|---|---|
| Width, Height mm | Label size at 8 dots per mm, width capped at the 48 mm head |
| Orientation | Vertical lays the label out a quarter turn round, upright in the preview and turned back on print. Width and Height stay the label as loaded |
| Rendering | Sharp thresholds text, Dithered suits photos |
| Text | Its own layer, on top at first. Press it on the preview or pick Text in the list to select it, drag to move it, Align lines up its rows |
| Images | Add one or more. The first fills the label, each next one lands at half size in the next quarter, above the other images. Press one on the preview or pick it in the list to select it, drag to move it, Remove deletes it |
| Layers, Move up, Move down, Center | The list shows the top layer first, the text included. Move up and Move down move the selected image or the text one layer, Center puts it in the middle of the label |
| Edges | An image's or the text's centre always stays on the label, so part of it stays on the preview to grab. A drag stops at the edge, a smaller or turned label pulls layers back, and a layer lost in an older save comes back on opening |
| Centre lines | While you drag, an image or the text sticks to the label's vertical and horizontal centre lines within 6 dots, and a guide shows each line it sticks to. The guides never print |
| Touch gestures | Two fingers on the preview edit the selected layer: pinch to resize, twist to rotate with right angles sticking within 5°, move both to move it. A pinch on the text changes its font size. Lifting one finger lets the other keep dragging. On a phone the preview stays at the top while the settings scroll |
| Image size | Scales the selected image from a tenth to four times its fitted size |
| Image rotation, Rotate 90° | Turns the selected image to any angle, or a quarter turn clockwise per press. It refits to stay whole inside the label |
| Image darkness | Left prints the selected image lighter, right darker, the middle leaves it as drawn |
| Density, Copies, Media | Same as the CLI options |

## Library

```toml
[dependencies]
niimbot-b1 = { git = "https://github.com/i-am-fatik/niimbot-b1-rs", features = ["serial"] }
```

| Feature | Adds |
|---|---|
| `serial` | `SerialTransport`, `find_port` |
| `ble` | `BleTransport`, needs a tokio runtime |
| `web` | `WebTransport` for `wasm32-unknown-unknown`, needs `--cfg=web_sys_unstable_apis` as set in `.cargo/config.toml` |

API docs: `cargo doc -p niimbot-b1 --features serial,ble --open`. The wire protocol is in [protocol.md](https://github.com/vkolarik/niimbot-b1/blob/main/docs/protocol.md).

## Checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo clippy -p niimbot-b1-pwa --target wasm32-unknown-unknown
cargo test --workspace
```

## Credits

Protocol, print sequence and firmware patch from [vkolarik/niimbot-b1](https://github.com/vkolarik/niimbot-b1), which credits [niimbluelib](https://github.com/MultiMote/niimbluelib) and [lotabout/niimbot](https://github.com/lotabout/niimbot). MIT, see [LICENSE](LICENSE).
