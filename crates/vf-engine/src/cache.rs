use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use vf_ir::Module;

/// Deterministic 128-bit FNV-1a structural hash of a COI-pruned `Module` + verification tag.
pub fn content_hash_hex(data: &[u8]) -> String {
    // Dual independent 64-bit FNV-1a lanes for collision-resistant 128-bit fingerprint.
    let mut h1: u64 = 0xcbf29ce484222325;
    let mut h2: u64 = 0x84222325cbf29ce4;
    for &b in data {
        h1 ^= u64::from(b);
        h1 = h1.wrapping_mul(0x100000001b3);
        h2 = h2.wrapping_mul(0x100000001b3);
        h2 ^= u64::from(b);
    }
    format!("{h1:016x}{h2:016x}")
}

pub fn module_fingerprint(m: &Module, tag: &str) -> String {
    let mut pruned = m.clone();
    vf_ir::passes::coi_prune(&mut pruned);
    let dump = vf_ir::fmt::dump_module(&pruned);
    let mut lines: Vec<&str> = dump.lines().collect();
    lines.sort_unstable();
    let canonical = lines.join("\n");
    let payload = format!("vf-v{}|{tag}|{canonical}", env!("CARGO_PKG_VERSION"));
    content_hash_hex(payload.as_bytes())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedVerdict {
    pub engine: String,
    pub k: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    entries: HashMap<String, CachedVerdict>,
}

/// Content-addressed incremental proof cache stored in `.vf-cache/proofs.json`.
pub struct ProofCache {
    path: PathBuf,
    data: CacheFile,
}

impl ProofCache {
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("proofs.json");
        let data = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, data }
    }

    pub fn get(&self, key: &str) -> Option<&CachedVerdict> {
        self.data.entries.get(key)
    }

    pub fn insert(&mut self, key: String, verdict: CachedVerdict) -> Result<()> {
        self.data.entries.insert(key, verdict);
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, serde_json::to_string_pretty(&self.data)?)?;
        Ok(())
    }
}
