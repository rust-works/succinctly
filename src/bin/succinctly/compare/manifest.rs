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
pub struct Perf {
    pub queries: String,
}

/// A vendored upstream test file, pinned by content.
#[derive(Debug, Clone, Deserialize)]
pub struct Suite {
    pub id: String,
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    #[serde(rename = "reference")]
    pub references: Vec<Reference>,
    pub succinctly: SuccinctlyBuild,
    pub queries: Queries,
    pub probes: Probes,
    #[serde(rename = "suite", default)]
    pub suites: Vec<Suite>,
    pub json_suite: Suite,
    pub perf: Perf,
    #[serde(rename = "input", default)]
    pub inputs: Vec<super::perf::InputSpec>,
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

    /// Read a pinned file, refusing one whose sha256 differs from the manifest's.
    pub fn read_pinned(root: &Path, s: &Suite) -> Result<String> {
        let bytes =
            std::fs::read(root.join(&s.path)).with_context(|| format!("reading {}", s.path))?;
        let got = sha256_hex(&bytes);
        if got != s.sha256 {
            bail!(
                "{} has sha256 {got}, the manifest pins {}",
                s.path,
                s.sha256
            );
        }
        Ok(String::from_utf8(bytes)?)
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
            [probes]\ncli='x'\nextensions='x'\nfixtures='x'\ndivergences='x'\n[json_suite]\nid='j'\npath='p'\nsha256='s'\n[perf]\nqueries='q'\n\
            [[reference]]\nid='a'\nversion='1'\nrole='primary'\nsource_url=''\nsource_sha256=''\n\
            [[reference]]\nid='b'\nversion='2'\nrole='primary'\nsource_url=''\nsource_sha256=''\n";
        assert!(Manifest::parse(text).is_err());
    }
}

#[cfg(test)]
mod pin_tests {
    use super::*;

    /// Every pinned file in the checked-in manifest must match its hash:
    /// editing a vendored file without re-pinning is a failure, not a drift.
    #[test]
    fn checked_in_pins_match_the_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (m, _) = Manifest::load(root).unwrap();
        assert!(m.suites.len() >= 4);
        assert!(m.inputs.len() >= 18 && m.inputs.iter().all(|i| i.sha256.len() == 64));
        for s in m.suites.iter().chain([&m.json_suite]) {
            Manifest::read_pinned(root, s).unwrap();
        }
    }

    #[test]
    fn a_changed_file_is_refused() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let s = Suite {
            id: "x".into(),
            path: "comparison/jq/manifest.toml".into(),
            sha256: "0".repeat(64),
        };
        assert!(Manifest::read_pinned(root, &s).is_err());
    }
}
