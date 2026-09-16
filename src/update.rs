use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use attestation_verify::{
    Bundle, CheckpointOriginPolicy, GithubPolicy, RefPolicy, RepositoryIdentity, SignerPolicy,
    SourcePolicy, TrustStore, Verifier, WorkflowPath, WorkflowRevisionPolicy,
};
use serde_json::{json, Value};
use uuid::Uuid;

const LATEST_RELEASE_URL: &str = "https://github.com/combinatrix-ai/knapper/releases/latest";
const RELEASE_DOWNLOAD_URL: &str = "https://github.com/combinatrix-ai/knapper/releases/download";
// Update code remains inside the already-trusted running binary. Fetching a
// mutable installer after attestation would let that code ignore the verified
// archive digest and bypass the trust decision made above.
const INSTALLER_SCRIPT: &str = include_str!("../install.sh");

const KNAPPER_REPOSITORY: &str = "combinatrix-ai/knapper";
const KNAPPER_SOURCE_OWNER_ID: u64 = 139_831_903;
const KNAPPER_SOURCE_REPOSITORY_ID: u64 = 1_314_722_218;
const KNAPPER_RELEASE_WORKFLOW_PATH: &str = ".github/workflows/release.yml";
const PUBLIC_GOOD_CHECKPOINT_ORIGIN: &str = "rekor.sigstore.dev - 1193050959916656506";
pub fn run(check_only: bool, yes: bool, force: bool, no_skill: bool) -> Result<()> {
    let latest = resolve_latest_version()?;
    let current = env!("CARGO_PKG_VERSION");
    if !force && !version_is_newer(&latest, current) {
        println!("knapper {current} is up to date.");
        return Ok(());
    }
    if check_only {
        println!("knapper {current} -> {latest} is available. Run `knapper self-update`.");
        return Ok(());
    }
    if !yes {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            bail!("use --yes to confirm a non-interactive update");
        }
        eprint!("Install knapper {latest}? [y/N] ");
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            return Ok(());
        }
    }
    let result = install_version(&latest, no_skill)?;
    println!(
        "Updated to knapper {latest} (release provenance verified).\n{}",
        result["binary"].as_str().unwrap_or_default()
    );
    Ok(())
}

fn install_version(latest: &str, no_skill: bool) -> Result<Value> {
    let current = env!("CARGO_PKG_VERSION");
    let tag = format!("v{latest}");
    let target = release_target();

    let verified = verify_release_attestation(&tag, &target)?;
    let attestation = json!({
        "verified": true,
        "source_ref": verified.source_ref,
        "log_index": verified.log_index,
        "trust_root": verified.trust_root,
    });
    let expected_sha256 = verified.sha256;

    let executable = std::env::current_exe().context("failed to locate knapper executable")?;
    let bin_dir = executable
        .parent()
        .context("knapper executable has no parent directory")?;
    let installer =
        std::env::temp_dir().join(format!("knapper-installer-{}.sh", Uuid::new_v4().simple()));
    write_embedded_installer(&installer)?;
    let mut command = Command::new("sh");
    command
        .arg(&installer)
        .args(installer_args(&tag, &target, &expected_sha256, bin_dir));
    if no_skill {
        command.arg("--no-skill");
    }
    command.env_remove("KNAPPER_INSTALLER_NO_MAIN");
    let result = command.output();
    let _ = std::fs::remove_file(&installer);
    let result = result.context("failed to run knapper installer")?;
    if !result.status.success() {
        bail!(
            "knapper installer failed: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    Ok(json!({
        "updated": true,
        "previous_version": current,
        "version": latest,
        "binary": executable,
        "skill_registration": !no_skill,
        "attestation": attestation,
    }))
}

pub(crate) fn resolve_latest_version() -> Result<String> {
    let output = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--tlsv1.2",
            "--max-time",
            "3",
            "--output",
            "/dev/null",
            "--write-out",
            "%{url_effective}",
            LATEST_RELEASE_URL,
        ])
        .output()
        .context("failed to check the latest knapper release")?;
    if !output.status.success() {
        bail!(
            "latest release check failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let url = String::from_utf8(output.stdout).context("latest release URL was not UTF-8")?;
    let tag = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let version = tag.strip_prefix('v').unwrap_or(tag);
    parse_version(version).context("latest release had an invalid version")?;
    Ok(version.to_owned())
}

fn installer_args(tag: &str, target: &str, sha256: &str, bin_dir: &Path) -> Vec<OsString> {
    [
        OsString::from("--bin-dir"),
        bin_dir.as_os_str().to_owned(),
        OsString::from("--skill"),
        OsString::from("auto"),
        OsString::from("--version"),
        OsString::from(tag),
        OsString::from("--target"),
        OsString::from(target),
        OsString::from("--expect-sha256"),
        OsString::from(sha256),
    ]
    .into()
}

fn write_embedded_installer(path: &Path) -> Result<()> {
    let mut created = false;
    let outcome = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .context("failed to create temporary knapper installer")?;
        created = true;
        file.write_all(INSTALLER_SCRIPT.as_bytes())
            .context("failed to write embedded knapper installer")?;
        file.sync_all()
            .context("failed to sync embedded knapper installer")?;
        Ok(())
    })();
    if created && outcome.is_err() {
        let _ = std::fs::remove_file(path);
    }
    outcome
}

/// The result of a successful release-attestation verification: the facts
/// worth surfacing to the caller, plus the expected sha256 for this
/// platform's release archive (extracted from the now-verified manifest).
struct VerifiedRelease {
    source_ref: String,
    log_index: u64,
    trust_root: String,
    sha256: String,
}

/// Downloads `tag`'s checksum manifest and its Sigstore bundle, verifies the
/// bundle against the embedded public-good trust root and knapper's GitHub
/// identity policy, and extracts this platform's expected release-archive
/// sha256 from the now-authenticated manifest.
fn verify_release_attestation(tag: &str, target: &str) -> Result<VerifiedRelease> {
    let temp_dir =
        std::env::temp_dir().join(format!("knapper-attestation-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&temp_dir).context("failed to create attestation temp directory")?;

    let outcome = (|| -> Result<VerifiedRelease> {
        let manifest_name = format!("knapper-{tag}-checksums.txt");
        let bundle_name = format!("{manifest_name}.sigstore.json");
        let manifest_path = temp_dir.join(&manifest_name);
        let bundle_path = temp_dir.join(&bundle_name);

        download_release_asset(tag, &manifest_name, &manifest_path)?;
        download_release_asset(tag, &bundle_name, &bundle_path)?;

        let manifest_bytes = std::fs::read(&manifest_path)
            .context("failed to read the downloaded checksum manifest")?;
        let bundle_bytes =
            std::fs::read(&bundle_path).context("failed to read the downloaded sigstore bundle")?;

        let trust_store = TrustStore::embedded_public_good()
            .context("failed to load the embedded attestation trust root")?;
        let checkpoint_origin_policy = public_good_checkpoint_origin_policy(&trust_store)?;
        let verifier = Verifier::builder()
            .trust_store(trust_store)
            .github_policy(release_attestation_policy(tag)?)
            .checkpoint_origin_policy(checkpoint_origin_policy)
            .build()
            .context("failed to build the attestation verifier")?;
        let bundle =
            Bundle::from_json(&bundle_bytes).context("failed to parse the sigstore bundle")?;
        let report = verifier
            .verify_bytes(&manifest_bytes, &bundle)
            .context("checksum manifest attestation did not verify")?;

        let asset_name = format!("knapper-{tag}-{target}.tar.gz");
        let manifest_text =
            String::from_utf8(manifest_bytes).context("checksum manifest was not valid UTF-8")?;
        let sha256 = manifest_digest_for(&manifest_text, &asset_name)
            .with_context(|| format!("checksum manifest has no entry for {asset_name}"))?;

        Ok(VerifiedRelease {
            source_ref: report.signer.source_ref,
            log_index: report.transparency.log_index,
            trust_root: report.trust.fingerprint,
            sha256,
        })
    })();

    let _ = std::fs::remove_dir_all(&temp_dir);
    outcome
}

/// Binds the exact signed Rekor checkpoint origin used by GitHub Artifact
/// Attestations to the public-good Rekor v1 ECDSA log key. The origin is not
/// inferred from the log URL; both the URL and key algorithm are authoritative
/// selectors for the trusted-root entry.
fn public_good_checkpoint_origin_policy(
    trust_store: &TrustStore,
) -> Result<CheckpointOriginPolicy> {
    let log = trust_store
        .tlogs
        .iter()
        .find(|log| {
            log.base_url == "https://rekor.sigstore.dev"
                && log.public_key.key_details == "PKIX_ECDSA_P256_SHA_256"
        })
        .context("embedded trust root has no public-good Rekor v1 ECDSA log")?;
    CheckpointOriginPolicy::builder()
        .allow_origin(log, PUBLIC_GOOD_CHECKPOINT_ORIGIN)
        .context("failed to bind the public-good Rekor checkpoint origin")?
        .build()
        .context("failed to build the public-good checkpoint-origin policy")
}

/// The GitHub identity policy release attestations must satisfy: the
/// artifact must come from tag `tag` on `combinatrix-ai/knapper` (pinned by
/// numeric owner/repository id), signed by that same repository's release
/// workflow at that tag.
fn release_attestation_policy(tag: &str) -> Result<GithubPolicy> {
    let repository = RepositoryIdentity::parse(KNAPPER_REPOSITORY)
        .context("invalid knapper repository identity")?;
    let source_repository = repository
        .clone()
        .with_owner_id(KNAPPER_SOURCE_OWNER_ID)
        .with_repository_id(KNAPPER_SOURCE_REPOSITORY_ID);
    let tag_ref = format!("refs/tags/{tag}");
    GithubPolicy::builder()
        .source(SourcePolicy {
            repository: source_repository,
            git_ref: RefPolicy::Exact(tag_ref.clone()),
            commit: None,
        })
        .signer(SignerPolicy {
            repository,
            path: WorkflowPath::new(KNAPPER_RELEASE_WORKFLOW_PATH)
                .context("invalid release workflow path")?,
            revision: WorkflowRevisionPolicy::Ref(tag_ref),
        })
        .build()
        .context("failed to build the attestation policy")
}

fn download_release_asset(tag: &str, name: &str, dest: &Path) -> Result<()> {
    let url = format!("{RELEASE_DOWNLOAD_URL}/{tag}/{name}");
    let output = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--tlsv1.2",
            "--connect-timeout",
            "10",
            "--max-time",
            "300",
            "--output",
        ])
        .arg(dest)
        .arg(&url)
        .output()
        .with_context(|| format!("failed to download release asset {name}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_file(dest);
        bail!(
            "release asset download failed for {name}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// This build's release-target string, in the same form as install.sh's
/// `detect_target` (e.g. `aarch64-apple-darwin`, `x86_64-unknown-linux-musl`).
fn release_target() -> String {
    format_release_target(
        std::env::consts::ARCH,
        std::env::consts::OS,
        cfg!(target_env = "musl"),
    )
}

/// Pure formatting half of [`release_target`], parameterized so every
/// supported `(arch, os, musl)` combination is directly testable without
/// depending on the compiling platform.
fn format_release_target(arch: &str, os: &str, musl: bool) -> String {
    match os {
        "macos" => format!("{arch}-apple-darwin"),
        "linux" => {
            let _ = musl;
            let libc = "musl";
            format!("{arch}-unknown-linux-{libc}")
        }
        other => format!("{arch}-unknown-{other}"),
    }
}

/// Finds `asset_name`'s line in a `sha256sum`-format checksum manifest and
/// returns its digest. Lines that do not split into exactly a digest and a
/// filename, or whose digest is not 64 hex characters, are skipped rather
/// than matched.
fn manifest_digest_for(manifest: &str, asset_name: &str) -> Option<String> {
    manifest.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let digest = fields.next()?;
        let filename = fields.next()?.trim_start_matches('*');
        if fields.next().is_some() || filename != asset_name || !is_sha256_hex(digest) {
            return None;
        }
        Some(digest.to_ascii_lowercase())
    })
}

fn is_sha256_hex(candidate: &str) -> bool {
    candidate.len() == 64 && candidate.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn version_is_newer(candidate: &str, current: &str) -> bool {
    parse_version(candidate)
        .zip(parse_version(current))
        .is_some_and(|(candidate, current)| candidate > current)
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(str::parse::<u64>);
    let parsed = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    parts.next().is_none().then_some(parsed)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;
    use std::process::Command;

    use attestation_verify::{RefPolicy, TrustStore, Verifier, WorkflowRevisionPolicy};
    use tempfile::tempdir;

    use super::{
        installer_args, manifest_digest_for, parse_version, public_good_checkpoint_origin_policy,
        release_attestation_policy, version_is_newer, write_embedded_installer, INSTALLER_SCRIPT,
    };

    #[test]
    fn embedded_installer_is_written_once_without_network_replacement(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("install.sh");

        write_embedded_installer(&path)?;

        assert_eq!(std::fs::read_to_string(&path)?, INSTALLER_SCRIPT);
        assert!(write_embedded_installer(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path)?, INSTALLER_SCRIPT);
        assert!(INSTALLER_SCRIPT.contains("--expect-sha256"));
        assert!(INSTALLER_SCRIPT.contains("verify_expected_sha256"));
        Ok(())
    }

    #[test]
    fn embedded_installer_rejects_an_unattested_archive() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempdir()?;
        let installer = directory.path().join("install.sh");
        let archive = directory.path().join("knapper.tar.gz");
        write_embedded_installer(&installer)?;
        std::fs::write(&archive, b"not the attested archive")?;

        let output = Command::new("sh")
            .args(["-c", ". \"$1\"; verify_expected_sha256 \"$2\" \"$3\""])
            .arg("embedded-installer-test")
            .arg(&installer)
            .arg(&archive)
            .arg("0".repeat(64))
            .env("KNAPPER_INSTALLER_NO_MAIN", "1")
            .output()?;

        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("attested checksum mismatch"));
        Ok(())
    }

    #[test]
    fn installer_arguments_bind_tag_target_digest_and_exact_bin_path() {
        let digest = "a".repeat(64);
        let args = installer_args(
            "v0.3.4",
            "x86_64-unknown-linux-musl",
            &digest,
            Path::new("/opt/knapper bin"),
        );

        assert_eq!(
            args,
            vec![
                OsString::from("--bin-dir"),
                OsString::from("/opt/knapper bin"),
                OsString::from("--skill"),
                OsString::from("auto"),
                OsString::from("--version"),
                OsString::from("v0.3.4"),
                OsString::from("--target"),
                OsString::from("x86_64-unknown-linux-musl"),
                OsString::from("--expect-sha256"),
                OsString::from(digest),
            ]
        );
    }

    #[test]
    fn compares_release_versions_numerically() {
        assert!(version_is_newer("0.10.0", "0.9.9"));
        assert!(!version_is_newer("0.1.4", "0.1.4"));
        assert!(!version_is_newer("0.1.3", "0.1.4"));
        assert_eq!(parse_version("1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("main"), None);
    }

    const SAMPLE_MANIFEST_TARGETS: [(&str, char); 4] = [
        ("aarch64-apple-darwin", '0'),
        ("x86_64-apple-darwin", '1'),
        ("aarch64-unknown-linux-musl", '4'),
        ("x86_64-unknown-linux-musl", '5'),
    ];

    /// Builds a `sha256sum`-format manifest, as the release workflow's
    /// `knapper-<tag>-checksums.txt` would read, with one line per published
    /// target and a distinct (but validly-shaped) digest per line.
    fn sample_manifest(tag: &str) -> String {
        use std::fmt::Write as _;

        SAMPLE_MANIFEST_TARGETS
            .iter()
            .fold(String::new(), |mut manifest, (target, digit)| {
                let _ = writeln!(
                    manifest,
                    "{}  knapper-{tag}-{target}.tar.gz",
                    digit.to_string().repeat(64)
                );
                manifest
            })
    }

    #[test]
    fn manifest_digest_for_matches_each_of_the_four_release_assets() {
        let tag = "v0.4.0";
        let manifest = sample_manifest(tag);
        for (target, digit) in SAMPLE_MANIFEST_TARGETS {
            let asset_name = format!("knapper-{tag}-{target}.tar.gz");
            let expected = digit.to_string().repeat(64);
            assert_eq!(manifest_digest_for(&manifest, &asset_name), Some(expected));
        }
    }

    #[test]
    fn manifest_digest_for_returns_none_for_a_missing_asset() {
        let manifest = sample_manifest("v0.4.0");
        assert_eq!(
            manifest_digest_for(&manifest, "knapper-v0.4.0-riscv64-unknown-linux-gnu.tar.gz"),
            None
        );
    }

    #[test]
    fn manifest_digest_for_rejects_malformed_lines() {
        let asset_name = "knapper-v0.4.0-aarch64-apple-darwin.tar.gz";

        let missing_filename = format!("{}\n", "0".repeat(64));
        assert_eq!(manifest_digest_for(&missing_filename, asset_name), None);

        let extra_field = format!("{}  {asset_name}  extra\n", "0".repeat(64));
        assert_eq!(manifest_digest_for(&extra_field, asset_name), None);

        let short_digest = format!("{}  {asset_name}\n", "0".repeat(63));
        assert_eq!(manifest_digest_for(&short_digest, asset_name), None);

        let non_hex_digest = format!("{}g  {asset_name}\n", "0".repeat(63));
        assert_eq!(manifest_digest_for(&non_hex_digest, asset_name), None);
    }

    #[test]
    fn release_attestation_policy_matches_the_requested_tag(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let policy = release_attestation_policy("v1.2.3")?;

        assert_eq!(policy.source.repository.owner(), "combinatrix-ai");
        assert_eq!(policy.source.repository.name(), "knapper");
        assert_eq!(policy.source.repository.owner_id(), Some(139_831_903));
        assert_eq!(
            policy.source.repository.repository_id(),
            Some(1_314_722_218)
        );
        assert_eq!(
            policy.source.git_ref,
            RefPolicy::Exact("refs/tags/v1.2.3".to_owned())
        );
        assert!(policy.source.commit.is_none());

        assert_eq!(policy.signer.repository.owner(), "combinatrix-ai");
        assert_eq!(policy.signer.repository.name(), "knapper");
        assert_eq!(policy.signer.repository.owner_id(), None);
        assert_eq!(policy.signer.repository.repository_id(), None);
        assert_eq!(policy.signer.path.as_str(), ".github/workflows/release.yml");
        assert_eq!(
            policy.signer.revision,
            WorkflowRevisionPolicy::Ref("refs/tags/v1.2.3".to_owned())
        );
        Ok(())
    }

    #[test]
    fn public_good_checkpoint_origin_policy_is_accepted_by_verifier(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let trust_store = TrustStore::embedded_public_good()?;
        let checkpoint_origin_policy = public_good_checkpoint_origin_policy(&trust_store)?;
        Verifier::builder()
            .trust_store(trust_store)
            .github_policy(release_attestation_policy("v1.2.3")?)
            .checkpoint_origin_policy(checkpoint_origin_policy)
            .build()?;
        Ok(())
    }
}
