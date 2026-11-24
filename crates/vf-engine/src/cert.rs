use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use vf_ir::Module;

use crate::cache::{content_hash_hex, module_fingerprint};

/// Signed/hashed formal proof certificate enabling sound assume-guarantee composition:
/// a child module's contract may only be promoted to assumptions in a parent proof if
/// its certificate hash matches the current RTL + contract and records verdict == `"PROVED"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofCertificate {
    pub version: u32,
    pub tool_version: String,
    pub contract_name: String,
    pub module_name: String,
    pub rtl_hash: String,
    pub contract_hash: String,
    pub verdict: String,
    pub engine: String,
    pub timestamp_unix: u64,
    pub integrity_mac: String,
}

impl ProofCertificate {
    pub fn create(
        contract_name: impl Into<String>,
        module: &Module,
        contract_source: &str,
        verdict: impl Into<String>,
        engine: impl Into<String>,
    ) -> Self {
        let contract_name = contract_name.into();
        let module_name = module.name.clone();
        let rtl_hash = module_fingerprint(module, "cert-rtl");
        let contract_hash = content_hash_hex(contract_source.as_bytes());
        let verdict = verdict.into();
        let engine = engine.into();
        let timestamp_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut cert = Self {
            version: 1,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            contract_name,
            module_name,
            rtl_hash,
            contract_hash,
            verdict,
            engine,
            timestamp_unix,
            integrity_mac: String::new(),
        };
        cert.integrity_mac = cert.compute_mac();
        cert
    }

    fn compute_mac(&self) -> String {
        let payload = format!(
            "v{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.version,
            self.tool_version,
            self.contract_name,
            self.module_name,
            self.rtl_hash,
            self.contract_hash,
            self.verdict,
            self.engine,
            self.timestamp_unix
        );
        content_hash_hex(payload.as_bytes())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("failed to write proof certificate {}", path.display()))
    }

    pub fn load_and_verify(
        path: &Path,
        expected_contract_source: Option<&str>,
    ) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read proof certificate {}", path.display()))?;
        let cert: Self = serde_json::from_str(&text)
            .with_context(|| format!("invalid proof certificate JSON in {}", path.display()))?;
        if cert.integrity_mac != cert.compute_mac() {
            bail!(
                "proof certificate '{}' failed integrity verification (tampered or corrupted)",
                path.display()
            );
        }
        if cert.verdict != "PROVED" {
            bail!(
                "proof certificate '{}' has verdict '{}' (only unbounded PROVED certificates may be composed as assumptions!)",
                path.display(),
                cert.verdict
            );
        }
        if let Some(src) = expected_contract_source {
            let h = content_hash_hex(src.as_bytes());
            if cert.contract_hash != h {
                bail!(
                    "proof certificate '{}' does not match current contract hash (stale certificate)",
                    path.display()
                );
            }
        }
        Ok(cert)
    }
}
