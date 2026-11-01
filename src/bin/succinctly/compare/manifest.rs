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
    /// A: syntax forms and the manual's sections, in the CLI probe format.
    pub syntax: String,
    pub fixtures: String,
    pub divergences: String,
    pub robust: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Perf {
    pub queries: String,
}

/// Dimension B's grammar fuzzer (#5552): the script, and the seeds and sizes of a run.
#[derive(Debug, Clone, Deserialize)]
pub struct Fuzz {
    pub script: String,
    /// One run per seed, so the programs a run covers are the same every time.
    pub seeds: Vec<u64>,
    /// Programs per seed. The same for `--quick` and a full run: see manifest.toml.
    pub programs: u32,
    pub depth: u32,
}

/// A vendored upstream test file, pinned by content.
#[derive(Debug, Clone, Deserialize)]
pub struct Suite {
    pub id: String,
    pub path: String,
    pub sha256: String,
    /// Id of the reference this suite runs against; absent means the primary. A suite run
    /// against the secondary stays out of the primary columns and the ledger (#5622).
    #[serde(default)]
    pub reference: Option<String>,
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
    pub fuzz: Fuzz,
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
        // A generated input needs its generator arguments, and a corpus input has none: the
        // defaults that let the corpus ones omit them must not hide a missing `size`.
        for i in &m.inputs {
            let generated = !i.pattern.is_empty() && !i.size.is_empty();
            if i.corpus.is_some() == generated || (i.corpus.is_some() && !i.pattern.is_empty()) {
                bail!(
                    "input {} needs either `corpus`, or `pattern` and `size` (and not both)",
                    i.id
                );
            }
        }
        if m.fuzz.seeds.is_empty() || m.fuzz.programs == 0 {
            bail!("[fuzz] needs at least one seed and a non-zero program count");
        }
        for suite in &m.suites {
            if let Some(id) = &suite.reference {
                if !m
                    .references
                    .iter()
                    .any(|r| &r.id == id && r.role == "secondary")
                {
                    bail!(
                        "suite {} names reference {id}, which is not a secondary reference",
                        suite.id
                    );
                }
            }
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
    fn a_suite_naming_an_unknown_reference_is_rejected() {
        let text = "schema_version = 1\n\
            [succinctly]\nprofile='release'\nfeatures=''\n[queries]\nsmoke='x'\n\
            [probes]\ncli='x'\nextensions='x'\nsyntax='x'\nfixtures='x'\ndivergences='x'\nrobust='x'\n[json_suite]\nid='j'\npath='p'\nsha256='s'\n[perf]\nqueries='q'\n[fuzz]\nscript='s'\nseeds=[1]\nprograms=1\ndepth=1\n\
            [[reference]]\nid='a'\nversion='1'\nrole='primary'\nsource_url=''\nsource_sha256=''\n\
            [[suite]]\nid='s'\npath='p'\nsha256='s'\nreference='a'\n";
        // Naming the primary, or a typo, would run the suite as a primary one.
        assert!(Manifest::parse(text).is_err());
        assert!(Manifest::parse(&text.replace("reference='a'\n", "reference='b'\n")).is_err());
    }

    #[test]
    fn an_input_is_generated_or_from_the_corpus_and_never_neither_or_both() {
        let base = "schema_version = 1\n\
            [succinctly]\nprofile='release'\nfeatures=''\n[queries]\nsmoke='x'\n\
            [probes]\ncli='x'\nextensions='x'\nsyntax='x'\nfixtures='x'\ndivergences='x'\nrobust='x'\n[json_suite]\nid='j'\npath='p'\nsha256='s'\n[perf]\nqueries='q'\n[fuzz]\nscript='s'\nseeds=[1]\nprograms=1\ndepth=1\n\
            [[reference]]\nid='a'\nversion='1'\nrole='primary'\nsource_url=''\nsource_sha256=''\n";
        let input = |fields: &str| {
            format!("{base}[[input]]\nid='i'\nshape='s'\ntier='1mb'\nsha256='h'\n{fields}")
        };
        assert!(Manifest::parse(&input("pattern='users'\nsize='1mb'\n")).is_ok());
        assert!(Manifest::parse(&input("corpus='json/a.json'\n")).is_ok());
        assert!(Manifest::parse(&input("")).is_err());
        assert!(Manifest::parse(&input("pattern='users'\n")).is_err());
        assert!(Manifest::parse(&input("size='1mb'\n")).is_err());
        assert!(Manifest::parse(&input(
            "corpus='json/a.json'\npattern='users'\nsize='1mb'\n"
        ))
        .is_err());
    }

    #[test]
    fn two_primaries_are_rejected() {
        let text = "schema_version = 1\n\
            [succinctly]\nprofile='release'\nfeatures=''\n[queries]\nsmoke='x'\n\
            [probes]\ncli='x'\nextensions='x'\nsyntax='x'\nfixtures='x'\ndivergences='x'\nrobust='x'\n[json_suite]\nid='j'\npath='p'\nsha256='s'\n[perf]\nqueries='q'\n[fuzz]\nscript='s'\nseeds=[1]\nprograms=1\ndepth=1\n\
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

    /// A corpus input names a file the corpus manifest records, with the same hash: the
    /// comparison pins what it measures, and a corpus file re-pinned on one side only would
    /// otherwise surface as a failed run on a machine that has fetched it (#5552).
    #[test]
    fn a_corpus_input_is_pinned_like_the_corpus_manifest() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (m, _) = Manifest::load(root).unwrap();
        let corpus: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("tests/data/bench-corpus/manifest.json")).unwrap(),
        )
        .unwrap();
        let files = corpus["files"].as_array().unwrap();
        let named: Vec<_> = m.inputs.iter().filter(|i| i.corpus.is_some()).collect();
        assert!(named.len() >= 7, "{}", named.len());
        for i in named {
            let file = i.corpus.as_deref().unwrap();
            let entry = files
                .iter()
                .find(|f| f["file"] == file)
                .unwrap_or_else(|| panic!("{} names {file}, not in the corpus manifest", i.id));
            assert_eq!(entry["sha256"], i.sha256.as_str(), "{}", i.id);
            assert_eq!(entry["format"], "json", "{} is not a jq input", i.id);
            assert_eq!(
                i.shape,
                format!("corpus-{}", entry["workload"].as_str().unwrap())
            );
        }
    }

    #[test]
    fn a_changed_file_is_refused() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let s = Suite {
            id: "x".into(),
            path: "comparison/jq/manifest.toml".into(),
            sha256: "0".repeat(64),
            reference: None,
        };
        assert!(Manifest::read_pinned(root, &s).is_err());
    }

    /// A suite names the secondary reference it runs against or none (#5622): a typo or the
    /// primary's id would otherwise drop the suite from both columns without a word.
    #[test]
    fn a_suite_reference_names_the_secondary_reference() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (m, _) = Manifest::load(root).unwrap();
        let secondary: Vec<_> = m.suites.iter().filter(|s| s.reference.is_some()).collect();
        assert!(secondary.len() >= 7, "{}", secondary.len());
        for s in &secondary {
            let id = s.reference.as_deref().unwrap();
            assert!(
                m.references
                    .iter()
                    .any(|r| r.id == id && r.role == "secondary"),
                "{} names {id}",
                s.id
            );
        }
        assert!(m.suites.iter().any(|s| s.reference.is_none()));
    }
}
