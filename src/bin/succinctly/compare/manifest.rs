//! `comparison/jq/manifest.toml`: the pinned references and inputs (#5409).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Reference {
    pub id: String,
    pub version: String,
    /// `primary` or `secondary`.
    pub role: String,
    pub source_url: String,
    pub source_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SuccinctlyBuild {
    pub profile: String,
    pub features: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Queries {
    pub smoke: String,
}

/// Probe data: CLI flags, succinctly-only surface, and the directory they run in.
#[derive(Debug, Clone, Deserialize)]
pub struct Probes {
    pub cli: String,
    pub extensions: String,
    pub fixtures: String,
    pub divergences: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    #[serde(rename = "reference")]
    pub references: Vec<Reference>,
    pub succinctly: SuccinctlyBuild,
    pub queries: Queries,
    pub probes: Probes,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Manifest {
    /// Parse manifest text; returns the manifest and the sha256 of its bytes.
    pub fn parse(text: &str) -> Result<(Self, String)> {
        let m: Self = toml::from_str(text).context("invalid manifest.toml")?;
        if m.schema_version != 1 {
            bail!("unsupported manifest schema_version {}", m.schema_version);
        }
        let primaries = m.references.iter().filter(|r| r.role == "primary").count();
        if primaries != 1 {
            bail!("manifest needs exactly one primary reference, found {primaries}");
        }
        Ok((m, sha256_hex(text.as_bytes())))
    }

    pub fn load(root: &Path) -> Result<(Self, String)> {
        let path = root.join("comparison/jq/manifest.toml");
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text)
    }

    pub fn primary(&self) -> &Reference {
        self.references
            .iter()
            .find(|r| r.role == "primary")
            .expect("checked in parse")
    }

    /// Where the hermetic build of `r` lives.
    pub fn jq_path(root: &Path, r: &Reference) -> PathBuf {
        root.join("target/compare")
            .join(format!("jq-{}", r.version))
            .join("bin/jq")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_manifest_parses_with_one_primary() {
        let text = include_str!("../../../../comparison/jq/manifest.toml");
        let (m, hash) = Manifest::parse(text).unwrap();
        assert_eq!(m.primary().version, "1.7.1");
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn two_primaries_are_rejected() {
        let text = "schema_version = 1\n\
            [succinctly]\nprofile='release'\nfeatures=''\n[queries]\nsmoke='x'\n\
            [probes]\ncli='x'\nextensions='x'\nfixtures='x'\ndivergences='x'\n\
            [[reference]]\nid='a'\nversion='1'\nrole='primary'\nsource_url=''\nsource_sha256=''\n\
            [[reference]]\nid='b'\nversion='2'\nrole='primary'\nsource_url=''\nsource_sha256=''\n";
        assert!(Manifest::parse(text).is_err());
    }
}
