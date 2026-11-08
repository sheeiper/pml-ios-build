# pml-ios-build

Public build mirror for the Personal Media Library native iOS app.

This repository intentionally contains **only** the iOS app sources and the
Tauri-free Rust core required to build the unsigned IPA. It does not contain
server deployment code, credentials, SSH keys, or Docker configuration.

Build: Actions -> "Build iOS Native" -> Run workflow -> download
`personal-media-library-native-ios` artifact.