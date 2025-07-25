<!-- markdownlint-disable MD024 -->
# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- reexport of `mockall` crate
- add async capabilities with `async-actor`, `async-tokio`, and `async-smol` features

### Changed

- update minimum version of rust to 1.75 (for using async fn in a trait)
- update default version of rust to 1.86
- rename `ActiveActor` to `ActiveMailbox` (kept a deprecated alias for `ActiveActor`)

## [0.5.1] - 2025-04-18

### Changed

- add build/CI support for rust 1.86.0

### Fixed

- fixed some warnings

## [0.5.0] - 2025-02-03

### Added

- add minimum supported rust version (1.64)
- adapt README.md and Cargo.toml to rtactor being at the root of the repository
- add categories and keywords to Cargo.toml
- add CI workflows for build and release
- published to [crates.io](https://crates.io)

### Fixed

- fix a test that would sometime fail due to timing issues between threads

## [0.4.0] - 2025-03-25

### Added

- Placeholders published to [crates.io](https://crates.io)

[unreleased]: https://github.com/rtreactive/rtactor-rs/compare/0.5.1...HEAD
[0.5.1]: https://github.com/rtreactive/rtactor-rs/releases/tag/0.5.1
[0.5.0]: https://github.com/rtreactive/rtactor-rs/releases/tag/0.5.0
[0.4.0]: https://github.com/rtreactive/rtactor-rs/releases/tag/0.4.0
