use serde::{Deserialize, Serialize};

pub const PKG_HEADER_MAGIC_SIZE: usize = 16;
pub const MANIFEST_FILENAME: &str = "pkg_manifest.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PkgHeader {
    pub v1: u32,
    pub v2: u32,
    pub data_size: u32,
    pub header_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkgManifest {
    pub v1: u32,
    pub v2: u32,
    pub files: Vec<ManifestFileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestFileEntry {
    pub name: String,
    pub flag: u32,
    pub h1: String,
    pub h2: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PkgEntry {
    pub idx: usize,
    pub offset: u32,
    pub size: u32,
    pub flag: u32,
    pub h1: [u8; 16],
    pub h2: Option<[u8; 16]>,
    pub name: String,
}
