# AirPlay sources

Pinned Music Assistant airplay-cli v0.5.8 (1701bc2), with libraop
81c2182649da8645ac2a58b78e9f370c79a4165b and the source dependencies pinned
by that commit. Upstream: https://github.com/music-assistant/airplay-cli/tree/v0.5.8

Textamp compiles the RAOP/AirPlay 2 sender and Apple ALAC codec into its own
executable. The CLI entry point is renamed to textamp_airplay_main at build
time and runs in a child of the same Textamp executable. No helper executable,
prebuilt archive, download, PATH entry or environment override is required.
OpenSSL is built from source through the vendored openssl-sys dependency.

Upstream sources are unmodified; build.rs selects sources and renames symbols.
The combined executable is distributed under GPL-3.0; Textamp's original Rust
sources retain the Unlicense. See LICENSE and THIRD_PARTY_NOTICES.md here.
The upstream OpenSSL 1.1.1 distribution described in its notices is replaced
by the OpenSSL 3 dependency with its Apache-2.0 license.

Included dependency revisions:
- crosstools: 41544c653760a205cbf6cebfdeda4a9394cc6455
- dmap-parser: 2f8ee61e2427a177beea919d72dde8931084fe51
- libcodecs (ALAC wrapper header): 3234c96680fbe9f45047936a14df10edca822e5e
- Apple ALAC: 30e526563714325ff384968e116060c4ed606f06
- mdnssd: 16402f3293b8bf09b8274ba7188202953d8e4f40
