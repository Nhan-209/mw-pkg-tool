# mw-pkg-tool

> **Ultra-fast, native standalone tool for unpacking and repacking Mini World `.pkg` asset archives.**
> Built in Rust with LTO and LZ4 compression. Zero external dependencies or Python runtimes required.

[![Rust CI & Release Build](https://github.com/Nhan-209/mw-pkg-tool/actions/workflows/ci.yml/badge.svg)](https://github.com/Nhan-209/mw-pkg-tool/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

---

## ⚡ Features

- **Blazing Fast & Lightweight**: Written in Rust, compiled to a single standalone `.exe` (~1-2 MB).
- **Zero Dependencies**: Does NOT require Python, Visual C++ Redistributable, or runtime interpreters.
- **Smart Drag & Drop**:
  - Drag a `.pkg` file onto `mw-pkg-tool.exe` $\rightarrow$ automatically unpacks into `<name>_extracted/`.
  - Drag a folder onto `mw-pkg-tool.exe` $\rightarrow$ automatically repacks into `<folder>.pkg`.
- **Bit-Perfect Repacking**: Generates and preserves `pkg_manifest.json` containing original metadata, entry order, and hash verification (`h1`/`h2`).
- **Interactive Menu**: Double-click `mw-pkg-tool.exe` in Windows Explorer to open a guided interactive console with automatic `%APPDATA%` scanner.
- **CLI Support**: Full command-line interface for automation and modding scripts.

---

## 🚀 Quick Start (Drag & Drop)

1. Download `mw-pkg-tool.exe` from GitHub Actions / Releases.
2. **To Unpack (Extract)**: Drag any `.pkg` file (e.g. `dx_res.pkg`, `script_res.pkg`) onto `mw-pkg-tool.exe`.
3. **To Repack**: After editing textures, shaders, or Lua scripts in the extracted folder, drag the folder back onto `mw-pkg-tool.exe`.

---

## 💻 CLI Usage

### 1. Unpack a `.pkg` archive:
```bash
# Unpack into default <name>_extracted folder
mw-pkg-tool unpack dx_res.pkg

# Unpack into custom directory
mw-pkg-tool unpack dx_res.pkg -o my_extracted_res
```

### 2. Repack a directory back to `.pkg`:
```bash
# Repack folder into <folder_name>.pkg
mw-pkg-tool pack dx_res_extracted

# Repack into specific output file
mw-pkg-tool pack dx_res_extracted -o dx_res.pkg

# Force LZ4 compression or disable compression
mw-pkg-tool pack my_folder -o custom.pkg --force-compress
mw-pkg-tool pack my_folder -o custom.pkg --no-compress
```

### 3. Clean & decode Lua scripts:
```bash
# Clean all Lua scripts in folder (decode \ddd escapes to UTF-8 & remove decompiler junk)
mw-pkg-tool clean extracted_script_res

# Unpack and clean in one step:
mw-pkg-tool unpack script_res.pkg --clean
```

### 4. Scan system for Mini World packages:
```bash
mw-pkg-tool scan
```

---

## 🛠️ Architecture & Binary Format

Mini World `.pkg` files use a little-endian chunk format:
1. **Header (16 bytes)**:
   - `v1` (u32), `v2` (u32)
   - `data_size` (u32): Offset where index table begins
   - `header_size` (u32): Size of compressed index table
2. **Payload Area**: File payloads written sequentially from offset 16 to `data_size`. If compressed (`flag & 1`), prefixed with 4-byte uncompressed size followed by raw LZ4 block.
3. **Index Table**: Compressed with LZ4.
   - `num_entries` (u32)
   - Table entries: `h1` (16 bytes MD5 hash), `offset` (u32), `size` (u32), `flag` (u32), optional `h2` (16 bytes).
   - 4-byte alignment padding.
   - String table: `num_strings` (u32), followed by `(str_len: u32, utf8_name, entry_index: u32)`.

---

## 📜 License

MIT License. Developed for Mini World modding research.
