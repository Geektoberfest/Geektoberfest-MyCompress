# Changelog

All notable changes to MyCompress are documented here.

## [Unreleased]

- Fixed a critical archive-writer bug where a file added after one of its
  descendants was accepted, letting the writer emit archives the reader
  rejects (file/directory path conflict).
- Made the archive property tests skip hierarchically conflicting paths
  instead of panicking on the writer's (correct) error.
- Continue improving documentation and release packaging.

## [0.1.0] - 2026-10-04

- Initial command-line compressor and extractor.
- Added `.mcz` version 1 archives.
- Added RLE compression with archive integrity checks.
- Added secure path validation and overwrite protection.
- Added Linux x86_64 and Windows x86_64 release packaging.
