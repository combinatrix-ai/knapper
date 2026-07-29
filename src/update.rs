//! Replacing the running binary with the newest GitHub release.
//!
//! knapper ships as a downloaded binary, so unlike a package-manager install
//! there is nothing that would otherwise upgrade it. This is the only code in
//! the program that opens a network connection, and it runs only when the
//! user asks for it by name.

use anyhow::{anyhow, Result};

const OWNER: &str = "combinatrix-ai";
const REPO: &str = "knapper";

pub fn run(check_only: bool, yes: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let bin = if cfg!(windows) {
        "knapper.exe"
    } else {
        "knapper"
    };
    // The release workflow packs the binary under a fixed `knapper/` rather
    // than a directory named for the tag, so this path cannot drift out of
    // step with the version being installed.
    let in_archive = format!("knapper/{bin}");

    let update = self_update::backends::github::Update::configure()
        .repo_owner(OWNER)
        .repo_name(REPO)
        .bin_name(bin)
        .bin_path_in_archive(&in_archive)
        .current_version(current)
        .show_download_progress(true)
        // Replacing a binary is not something to do quietly, so confirm
        // unless --yes says otherwise.
        .no_confirm(yes)
        .build()
        .map_err(releases_failed)?;

    if check_only {
        let latest = update.get_latest_release().map_err(releases_failed)?;
        if self_update::version::bump_is_greater(current, &latest.version).unwrap_or(false) {
            println!("knapper {current} -> {} is available.", latest.version);
            println!("Run `knapper self-update` to install it.");
        } else {
            println!("knapper {current} is up to date.");
        }
        return Ok(());
    }

    match update.update().map_err(|e| anyhow!("update failed: {e}"))? {
        self_update::Status::UpToDate(v) => println!("knapper {v} is up to date."),
        self_update::Status::Updated(v) => {
            println!("Updated to knapper {v}.");
            // The skill documents this binary, so a stale one is worse than
            // none. The new binary carries the new text; register it now.
            crate::skill::register();
        }
    }
    Ok(())
}

fn releases_failed(err: self_update::errors::Error) -> anyhow::Error {
    anyhow!("could not read the releases of {OWNER}/{REPO}: {err}")
}
