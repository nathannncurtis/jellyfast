# Jellyfast

A native, fast macOS client for [Jellyfin](https://jellyfin.org), written in Rust.

Jellyfast talks to AppKit and AVKit directly, so the UI is fully native (vibrancy, SF Symbols, light/dark) and playback uses Apple's hardware-decoding stack. Anything macOS can decode direct-plays; everything else falls back to a server-side HLS transcode.

> Jellyfast is an independent project. It is not affiliated with or endorsed by the Jellyfin project.

## Features

- Browse movies, shows, music, and Live TV from your Jellyfin server
- Direct play with hardware decode, with automatic HLS fallback
- Resume position, watched state, and playback reporting
- Subtitle and audio track selection, playback speed, Picture in Picture
- [Seerr](https://github.com/seerr-team/seerr) integration: search for something new and request it without leaving the app
- Credentials stored in the macOS Keychain, never in plain-text config

## Requirements

- macOS 14 or later (Apple silicon or Intel)
- Rust (stable) — install with [rustup](https://rustup.rs)
- A Jellyfin server (and optionally a Seerr instance)

## Build and run

```sh
cargo build --release
./target/release/jellyfin-mac
```

If your Jellyfin server is plain HTTP, AVPlayer needs an App Transport Security exception in the app bundle's `Info.plist` (`NSAppTransportSecurity` → `NSAllowsArbitraryLoads`).

## Testing

```sh
cargo test
```

Unit and integration tests run offline against mock Jellyfin and Seerr servers; they never touch your Keychain or config.

The UI has a headless self-test that drives every screen and action with fake data and exits on its own:

```sh
HOME=$(mktemp -d) JELLYFIN_MAC_SELFTEST=1 ./target/debug/jellyfin-mac
```

Optional live tests against your own servers are `#[ignore]`d and configured through environment variables; see the test modules for the variable names.

## Credits

Jellyfast is inspired by [Spotifast](https://github.com/crmne/spotifast) by Carmine Paolino — a native, fast, lightweight Spotify client in Rust. Jellyfast shares its philosophy, not its code.

## License

[MIT](LICENSE)
