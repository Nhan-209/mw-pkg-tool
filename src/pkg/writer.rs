use super::types::{PkgManifest, MANIFEST_FILENAME, PKG_HEADER_MAGIC_SIZE};
use anyhow::{bail, Context, Result};
use md5::{Digest, Md5};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use walkdir::WalkDir;

pub struct PackOptions {
    pub force_compression: bool,
    pub no_compression: bool,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self {
            force_compression: false,
            no_compression: false,
        }
    }
}

struct PreparedEntry {
    name: String,
    flag: u32,
    h1: [u8; 16],
    h2: Option<[u8; 16]>,
    payload: Vec<u8>,
}

fn hex_decode(hex_str: &str) -> Option<[u8; 16]> {
    if hex_str.len() != 32 {
        return None;
    }
    let mut arr = [0u8; 16];
    for i in 0..16 {
        let byte_str = &hex_str[i * 2..i * 2 + 2];
        arr[i] = u8::from_str_radix(byte_str, 16).ok()?;
    }
    Some(arr)
}

pub fn repack_pkg<P: AsRef<Path>, Q: AsRef<Path>>(
    input_dir: P,
    output_pkg: Option<Q>,
    options: PackOptions,
) -> Result<PathBuf> {
    let input_dir = input_dir.as_ref();
    if !input_dir.is_dir() {
        bail!("Input path is not a directory: {:?}", input_dir);
    }

    let out_pkg = match output_pkg {
        Some(p) => p.as_ref().to_path_buf(),
        None => {
            let parent = input_dir.parent().unwrap_or_else(|| Path::new("."));
            let dir_name = input_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("archive");
            let clean_name = dir_name.strip_suffix("_extracted").unwrap_or(dir_name);
            parent.join(format!("{}.pkg", clean_name))
        }
    };

    let timer = Instant::now();
    println!("[*] Repacking folder: {:?} -> {:?}", input_dir, out_pkg);

    // 1. Check for manifest
    let manifest_path = input_dir.join(MANIFEST_FILENAME);
    let maybe_manifest: Option<PkgManifest> = if manifest_path.is_file() {
        let content = fs::read_to_string(&manifest_path)?;
        match serde_json::from_str::<PkgManifest>(&content) {
            Ok(m) => {
                println!("    - Loaded manifest with {} entries", m.files.len());
                Some(m)
            }
            Err(e) => {
                eprintln!("[!] Warning: invalid manifest.json ({}), will scan directory directly", e);
                None
            }
        }
    } else {
        None
    };

    let (v1, v2) = if let Some(ref m) = maybe_manifest {
        (m.v1, m.v2)
    } else {
        (139, 9)
    };

    // 2. Discover and order files to pack
    let has_dotdot_prefix = if let Some(ref m) = maybe_manifest {
        m.files.iter().any(|f| f.name.starts_with("../"))
    } else {
        true
    };

    let mut manifest_by_norm = HashMap::new();
    let mut manifest_order = Vec::new();
    let mut seen_norm_paths = HashSet::new();

    if let Some(ref m) = maybe_manifest {
        for f in &m.files {
            let norm = f.name.trim_start_matches("../").replace('\\', "/");
            manifest_by_norm.insert(norm.clone(), f.clone());
            manifest_order.push((f.name.clone(), norm));
        }
    }

    // Scan disk files
    let mut disk_files_map = HashMap::new();
    for entry in WalkDir::new(input_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() {
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or_default();
            if file_name == MANIFEST_FILENAME || file_name.ends_with(".pkg.tmp") {
                continue;
            }
            let rel = path.strip_prefix(input_dir)?;
            let rel_norm = rel.to_string_lossy().replace('\\', "/");
            disk_files_map.insert(rel_norm, path.to_path_buf());
        }
    }

    // Order files: first manifest entries in their original sequence, then any newly added files
    let mut files_to_pack = Vec::new();

    for (archive_name, norm_rel) in manifest_order {
        if let Some(disk_path) = disk_files_map.get(&norm_rel) {
            let me = manifest_by_norm.get(&norm_rel).cloned();
            files_to_pack.push((archive_name, disk_path.clone(), me));
            seen_norm_paths.insert(norm_rel);
        }
    }

    let mut new_disk_files: Vec<_> = disk_files_map
        .into_iter()
        .filter(|(norm_rel, _)| !seen_norm_paths.contains(norm_rel))
        .collect();
    new_disk_files.sort_by(|a, b| a.0.cmp(&b.0));

    for (norm_rel, disk_path) in new_disk_files {
        let archive_name = if has_dotdot_prefix {
            format!("../{}", norm_rel)
        } else {
            norm_rel.clone()
        };
        files_to_pack.push((archive_name, disk_path, None));
    }

    println!("    - Found {} files to pack", files_to_pack.len());

    let mut prepared_entries = Vec::with_capacity(files_to_pack.len());

    for (name, path, manifest_entry) in files_to_pack {
        let raw_data = fs::read(&path)
            .with_context(|| format!("Failed to read file {:?}", path))?;

        let flag = if options.no_compression {
            0
        } else if options.force_compression {
            1
        } else if let Some(ref me) = manifest_entry {
            me.flag
        } else {
            1 // Default: LZ4 compress
        };

        // Compress payload if flag & 1 != 0
        let payload = if (flag & 1) != 0 {
            let uncomp_len = raw_data.len() as u32;
            let compressed_body = lz4_flex::block::compress(&raw_data);
            let mut buf = Vec::with_capacity(4 + compressed_body.len());
            buf.extend_from_slice(&uncomp_len.to_le_bytes());
            buf.extend_from_slice(&compressed_body);
            buf
        } else {
            raw_data
        };

        // MD5 hash of the payload written to PKG
        let mut hasher = Md5::new();
        hasher.update(&payload);
        let hash_result = hasher.finalize();
        let mut h1 = [0u8; 16];
        h1.copy_from_slice(&hash_result);

        let h2 = manifest_entry.as_ref().and_then(|me| me.h2.as_ref().and_then(|s| hex_decode(s)));

        prepared_entries.push(PreparedEntry {
            name,
            flag,
            h1,
            h2,
            payload,
        });
    }

    // 3. Write PKG
    let tmp_pkg = out_pkg.with_extension("pkg.tmp");
    let mut file = File::create(&tmp_pkg)?;

    // Reserve 16 bytes for header
    file.write_all(&[0u8; PKG_HEADER_MAGIC_SIZE])?;

    let mut current_offset: u32 = PKG_HEADER_MAGIC_SIZE as u32;
    struct WrittenMeta {
        offset: u32,
        size: u32,
        flag: u32,
        h1: [u8; 16],
        h2: Option<[u8; 16]>,
        name: String,
    }

    let mut written_meta = Vec::with_capacity(prepared_entries.len());

    for entry in prepared_entries {
        let size = entry.payload.len() as u32;
        file.write_all(&entry.payload)?;

        written_meta.push(WrittenMeta {
            offset: current_offset,
            size,
            flag: entry.flag,
            h1: entry.h1,
            h2: entry.h2,
            name: entry.name,
        });

        current_offset += size;
    }

    let data_size = current_offset;

    // 4. Construct uncompressed index table
    let mut tbl = Vec::new();
    tbl.extend_from_slice(&(written_meta.len() as u32).to_le_bytes());

    for meta in &written_meta {
        tbl.extend_from_slice(&meta.h1);
        tbl.extend_from_slice(&meta.offset.to_le_bytes());
        tbl.extend_from_slice(&meta.size.to_le_bytes());
        tbl.extend_from_slice(&meta.flag.to_le_bytes());
        if (meta.flag & 0x20) != 0 {
            if let Some(h2) = meta.h2 {
                tbl.extend_from_slice(&h2);
            } else {
                tbl.extend_from_slice(&[0u8; 16]);
            }
        }
    }

    // Align 4 bytes
    let pad = (4 - (tbl.len() % 4)) % 4;
    for _ in 0..pad {
        tbl.push(0);
    }

    // String table
    tbl.extend_from_slice(&(written_meta.len() as u32).to_le_bytes());
    for (idx, meta) in written_meta.iter().enumerate() {
        let name_bytes = meta.name.as_bytes();
        tbl.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
        tbl.extend_from_slice(name_bytes);
        tbl.extend_from_slice(&(idx as u32).to_le_bytes());
    }

    // 5. Compress table
    let comp_tbl = lz4_flex::block::compress(&tbl);
    let mut table_with_len = Vec::with_capacity(4 + comp_tbl.len());
    table_with_len.extend_from_slice(&(tbl.len() as u32).to_le_bytes());
    table_with_len.extend_from_slice(&comp_tbl);

    let header_size = table_with_len.len() as u32;
    file.write_all(&table_with_len)?;

    // 6. Write final header
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&v1.to_le_bytes())?;
    file.write_all(&v2.to_le_bytes())?;
    file.write_all(&data_size.to_le_bytes())?;
    file.write_all(&header_size.to_le_bytes())?;

    drop(file);

    // Atomically replace target file
    if out_pkg.exists() {
        fs::remove_file(&out_pkg)?;
    }
    fs::rename(&tmp_pkg, &out_pkg)?;

    let elapsed = timer.elapsed();
    let final_sz = fs::metadata(&out_pkg)?.len();
    println!(
        "[+] Successfully packed {} files into {:?} ({:.2} MB) in {:.2?}",
        written_meta.len(),
        out_pkg,
        final_sz as f64 / 1_048_576.0,
        elapsed
    );

    Ok(out_pkg)
}
