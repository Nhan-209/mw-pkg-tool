use super::types::{ManifestFileEntry, PkgEntry, PkgHeader, PkgManifest, MANIFEST_FILENAME, PKG_HEADER_MAGIC_SIZE};
use anyhow::{bail, Context, Result};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub fn sanitize_path(path_str: &str) -> PathBuf {
    let clean = path_str.replace('\\', "/");
    let mut parts = Vec::new();

    for part in clean.split('/') {
        let trimmed = part.trim();
        if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
            continue;
        }
        let safe_part: String = trimmed
            .chars()
            .map(|c| match c {
                '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
                _ => c,
            })
            .collect();
        if !safe_part.is_empty() {
            parts.push(safe_part);
        }
    }

    if parts.is_empty() {
        PathBuf::from("unnamed_file.bin")
    } else {
        parts.iter().collect()
    }
}

pub fn unpack_pkg<P: AsRef<Path>, Q: AsRef<Path>>(pkg_path: P, output_dir: Option<Q>) -> Result<PathBuf> {
    let pkg_path = pkg_path.as_ref();
    let file_size = fs::metadata(pkg_path)
        .with_context(|| format!("Cannot stat file: {:?}", pkg_path))?
        .len();

    if file_size < PKG_HEADER_MAGIC_SIZE as u64 {
        bail!("File too small to be a valid PKG: {:?}", pkg_path);
    }

    let pkg_stem = pkg_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("pkg_extracted");

    let out_dir = match output_dir {
        Some(d) => d.as_ref().to_path_buf(),
        None => {
            let parent = pkg_path.parent().unwrap_or_else(|| Path::new("."));
            parent.join(format!("{}_extracted", pkg_stem))
        }
    };

    fs::create_dir_all(&out_dir)?;

    let timer = Instant::now();
    println!("[*] Unpacking: {:?} ({:.2} MB)", pkg_path.file_name().unwrap_or_default(), file_size as f64 / 1_048_576.0);

    let mut file = File::open(pkg_path)?;

    // 1. Read Header (16 bytes)
    let mut header_buf = [0u8; PKG_HEADER_MAGIC_SIZE];
    file.read_exact(&mut header_buf)?;

    let v1 = u32::from_le_bytes(header_buf[0..4].try_into().unwrap());
    let v2 = u32::from_le_bytes(header_buf[4..8].try_into().unwrap());
    let data_size = u32::from_le_bytes(header_buf[8..12].try_into().unwrap());
    let header_size = u32::from_le_bytes(header_buf[12..16].try_into().unwrap());

    let header = PkgHeader {
        v1,
        v2,
        data_size,
        header_size,
    };

    if (data_size as u64) + (header_size as u64) != file_size {
        eprintln!(
            "[!] Warning: data_size ({}) + header_size ({}) != file_size ({})",
            data_size, header_size, file_size
        );
    }

    // 2. Read and decompress index table
    file.seek(SeekFrom::Start(data_size as u64))?;
    let mut table_compressed = vec![0u8; header_size as usize];
    file.read_exact(&mut table_compressed)?;

    if table_compressed.len() < 4 {
        bail!("PKG header table is corrupted (less than 4 bytes)");
    }

    let uncomp_table_len = u32::from_le_bytes(table_compressed[0..4].try_into().unwrap()) as usize;
    let table_decomp = lz4_flex::block::decompress(&table_compressed[4..], uncomp_table_len)
        .context("Failed to decompress PKG index table via LZ4")?;

    if table_decomp.len() < 4 {
        bail!("Decompressed PKG table is empty");
    }

    // 3. Parse entries
    let num_entries = u32::from_le_bytes(table_decomp[0..4].try_into().unwrap()) as usize;
    let mut pos = 4;
    let mut entries = Vec::with_capacity(num_entries);

    for i in 0..num_entries {
        if pos + 28 > table_decomp.len() {
            bail!("Unexpected end of table while reading entry #{}", i);
        }

        let mut h1 = [0u8; 16];
        h1.copy_from_slice(&table_decomp[pos..pos + 16]);

        let offset = u32::from_le_bytes(table_decomp[pos + 16..pos + 20].try_into().unwrap());
        let size = u32::from_le_bytes(table_decomp[pos + 20..pos + 24].try_into().unwrap());
        let flag = u32::from_le_bytes(table_decomp[pos + 24..pos + 28].try_into().unwrap());
        pos += 28;

        let mut h2 = None;
        if (flag & 0x20) != 0 {
            if pos + 16 > table_decomp.len() {
                bail!("Unexpected end of table reading h2 for entry #{}", i);
            }
            let mut h2_bytes = [0u8; 16];
            h2_bytes.copy_from_slice(&table_decomp[pos..pos + 16]);
            h2 = Some(h2_bytes);
            pos += 16;
        }

        entries.push(PkgEntry {
            idx: i,
            offset,
            size,
            flag,
            h1,
            h2,
            name: format!("unnamed_{:06}.bin", i),
        });
    }

    // 4. Align 4 bytes and parse string table
    pos = (pos + 3) & !3;
    if pos + 4 <= table_decomp.len() {
        let num_strings = u32::from_le_bytes(table_decomp[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;

        for _ in 0..num_strings {
            if pos + 4 > table_decomp.len() {
                break;
            }
            let str_len = u32::from_le_bytes(table_decomp[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;

            if pos + str_len > table_decomp.len() {
                break;
            }
            let name_bytes = &table_decomp[pos..pos + str_len];
            let name = String::from_utf8_lossy(name_bytes).to_string();
            pos += str_len;

            if pos + 4 > table_decomp.len() {
                break;
            }
            let entry_idx = u32::from_le_bytes(table_decomp[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;

            if entry_idx < entries.len() {
                entries[entry_idx].name = name;
            }
        }
    }

    println!("    - Indexed {} files inside PKG", entries.len());

    // 5. Extract payloads
    let mut manifest_files = Vec::with_capacity(entries.len());
    let mut total_extracted_bytes: u64 = 0;
    let mut raw_buf = Vec::new();

    for (idx, entry) in entries.iter().enumerate() {
        file.seek(SeekFrom::Start(entry.offset as u64))?;
        raw_buf.resize(entry.size as usize, 0);
        file.read_exact(&mut raw_buf)?;

        let file_content = if (entry.flag & 1) != 0 && raw_buf.len() >= 4 {
            let uncomp_sz = u32::from_le_bytes(raw_buf[0..4].try_into().unwrap()) as usize;
            match lz4_flex::block::decompress(&raw_buf[4..], uncomp_sz) {
                Ok(decomp) => decomp,
                Err(_) => raw_buf.clone(),
            }
        } else {
            raw_buf.clone()
        };

        total_extracted_bytes += file_content.len() as u64;

        let safe_rel = sanitize_path(&entry.name);
        let out_file_path = out_dir.join(&safe_rel);

        if let Some(parent) = out_file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out_f = File::create(&out_file_path)?;
        out_f.write_all(&file_content)?;

        manifest_files.push(ManifestFileEntry {
            name: entry.name.clone(),
            flag: entry.flag,
            h1: hex::encode(entry.h1),
            h2: entry.h2.map(|h| hex::encode(h)),
        });

        if (idx + 1) % 5000 == 0 || idx + 1 == entries.len() {
            let pct = ((idx + 1) as f64 / entries.len() as f64) * 100.0;
            println!("    - Progress: {}/{} ({:.1}%)", idx + 1, entries.len(), pct);
        }
    }

    // 6. Write manifest for perfect repacking
    let manifest = PkgManifest {
        v1: header.v1,
        v2: header.v2,
        files: manifest_files,
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    fs::write(out_dir.join(MANIFEST_FILENAME), manifest_json)?;

    let elapsed = timer.elapsed();
    println!(
        "[+] Successfully unpacked {} files ({:.2} MB) into {:?} in {:.2?}",
        entries.len(),
        total_extracted_bytes as f64 / 1_048_576.0,
        out_dir,
        elapsed
    );

    Ok(out_dir)
}

mod hex {
    pub fn encode<T: AsRef<[u8]>>(data: T) -> String {
        let bytes = data.as_ref();
        let mut hex = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            use std::fmt::Write;
            write!(&mut hex, "{:02x}", b).unwrap();
        }
        hex
    }
}
