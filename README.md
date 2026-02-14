<div align="center">

<img src="resources/icons/hicolor/scalable/apps/io.github.m0Rf30.Lyra.svg" width="120" height="120" alt="Lyra">

# Lyra

**A modern music player for the COSMIC desktop**

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![Rust](https://img.shields.io/badge/Rust-2024-orange.svg)](https://www.rust-lang.org/)
[![COSMIC](https://img.shields.io/badge/COSMIC-Desktop-purple.svg)](https://github.com/pop-os/cosmic)

</div>

---

Lyra is a sleek, native music player designed specifically for the [COSMIC desktop environment](https://github.com/pop-os/cosmic). Named after the constellation and the ancient lyre, it brings together elegant design with powerful music management capabilities.

## Features

- **Multiple Sources** — Connect to MPD servers, Subsonic/OpenSubsonic servers, or play local files
- **Library Management** — SQLite-backed library with automatic metadata extraction via lofty
- **Browse Your Way** — View your collection by albums, artists, or individual tracks
- **Lyrics Support** — Automatic lyrics fetching so you can sing along
- **Cover Art** — Beautiful album artwork display
- **Real-time Updates** — File system watching keeps your library in sync
- **Visualizer Ready** — Optional ProjectM visualizer support for immersive audio experiences

## Installation

### Requirements

- Rust toolchain (2024 edition)
- COSMIC desktop environment (or compatible Wayland compositor)
- For visualizer support: OpenGL libraries

### Building from Source

```bash
git clone https://github.com/M0Rf30/lyra
cd lyra
cargo build --release
```

### Optional Features

```bash
# Enable visualizer support
cargo build --release --features visualizer

# Enable tokio-console for debugging
cargo build --release --features tokio-console
```

## Usage

Launch Lyra from your application menu or run:

```bash
cargo run --release
```

### Adding Music Sources

1. **Local Library** — Your `~/Music` folder is automatically indexed
2. **MPD Server** — Connect to any MPD instance (local or remote)
3. **Subsonic Server** — Stream from your self-hosted music server

## Architecture

Lyra is built with a modern Rust stack:

- **UI Framework**: [libcosmic](https://github.com/pop-os/libcosmic) — Native COSMIC toolkit
- **Audio Playback**: [rodio](https://github.com/RustAudio/rodio) with symphonia for broad format support
- **Async Runtime**: [tokio](https://tokio.rs/) for responsive, non-blocking I/O
- **Database**: SQLite via [rusqlite](https://github.com/rusqlite/rusqlite) for efficient library queries
- **Protocols**: Native MPD and OpenSubsonic clients for server connectivity

## License

Lyra is free software released under the GNU General Public License v3.0. See [LICENSE](LICENSE) for details.

---

<div align="center">

Made with ♪ for the COSMIC ecosystem

</div>
