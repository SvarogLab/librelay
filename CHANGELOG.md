# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)
and follows [Semantic Versioning](https://semver.org/).

## [0.1.2] - 2026-11-11

### Build system 🛠️

- Track the lockfile and check it with cargo-deny (#14)

### Documentation 📚

- Give the cost of a process per request instead of a closed wsl issue

### Miscellaneous 🧹

- Update svaroglab/.github action to v2 (#15)

### Tests ✅

- Give the exit-status test a server that exits on the transport's end

## [0.1.1] - 2026-11-05

### Fixed 🐛

- End streams closed after an empty write and parse log filters alike on both sides

## [0.1.0] - 2026-10-31

### Added ✨

- Share the client and server sides of a wsl relay

### Changed 🔧

- Name the steps of closing the link and of forwarding a server line
- Share the yamux and logging setup, and split the mux drivers

### Fixed 🐛

- Report the server's exit status when its pipes close first

### Miscellaneous 🧹

- Initialize the repository
