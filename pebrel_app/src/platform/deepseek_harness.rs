//! DeepSeek Harness operating-system integration.
//!
//! Desktop discovery, process control and installer launching live behind this
//! platform boundary so Agent/Hook code stays free of OS-specific branches.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::Platform;

pub(crate) fn registered_install_directory() -> Option<PathBuf> {
    if Platform::current() != Platform::Windows {
        return None;
    }
    for root in [
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ] {
        let output = Command::new("reg.exe").args(["query", root, "/s"]).output().ok()?;
        if !output.status.success() {
            continue;
        }
        let raw = String::from_utf8_lossy(&output.stdout);
        let mut key_matches = false;
        let mut install_location = None;
        for line in raw.lines() {
            let line = line.trim();
            if line.starts_with("HKEY_") {
                if key_matches {
                    if let Some(path) = install_location.take() {
                        return Some(path);
                    }
                }
                key_matches = false;
                install_location = None;
            } else if let Some((name, value)) = line.split_once("REG_SZ") {
                let name = name.trim();
                let value = value.trim();
                if name.eq_ignore_ascii_case("DisplayName")
                    && value.eq_ignore_ascii_case("DeepSeek Harness")
                {
                    key_matches = true;
                } else if name.eq_ignore_ascii_case("InstallLocation") && !value.is_empty() {
                    install_location = Some(PathBuf::from(value));
                }
            }
        }
        if key_matches {
            if let Some(path) = install_location {
                return Some(path);
            }
        }
    }
    None
}

pub(crate) fn desktop_running() -> bool {
    match Platform::current() {
        Platform::Windows => Command::new("tasklist.exe")
            .args(["/FI", "IMAGENAME eq DeepSeek Harness.exe", "/FO", "CSV", "/NH"])
            .output()
            .is_ok_and(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout)
                        .to_ascii_lowercase()
                        .contains("deepseek harness.exe")
            }),
        Platform::MacOS | Platform::Linux => Command::new("pgrep")
            .args(["-f", "DeepSeek Harness"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success()),
    }
}

pub(crate) fn run_dsh(cli: &Path, arguments: &[&str]) -> io::Result<()> {
    if Platform::current() == Platform::Windows {
        let extension_is_cmd = cli
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
            });
        if extension_is_cmd {
            // Windows command scripts must be invoked through cmd.exe. The only
            // interpolated values are absolute filesystem paths, quoted as one argv.
            let mut command_line = format!("\"\"{}\"", cli.display());
            for argument in arguments {
                command_line.push(' ');
                command_line.push_str(&format!("\"{}\"", argument.replace('"', "\"\"")));
            }
            command_line.push('"');
            let status = Command::new("cmd.exe")
                .args(["/D", "/S", "/C"])
                .arg(command_line)
                .status()?;
            return if status.success() {
                Ok(())
            } else {
                Err(io::Error::other(format!("dsh exited with {status}")))
            };
        }
    }
    let status = Command::new(cli).args(arguments).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("dsh exited with {status}")))
    }
}

fn verify_windows_installer(installer: &Path) -> io::Result<()> {
    // An official download URL and an MZ header do not authenticate an executable.
    // Require a valid Authenticode signature from DeepSeek before launching it.
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$signature = Get-AuthenticodeSignature -LiteralPath $env:PEBREL_INSTALLER; if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(?i)DeepSeek') { exit 1 }",
        ])
        .env("PEBREL_INSTALLER", installer)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(
            "The downloaded installer does not have a valid DeepSeek Authenticode signature; it was not run.",
        ))
    }
}

pub(crate) fn install_desktop(installer_url: &str, directory: &Path) -> io::Result<()> {
    match Platform::current() {
        Platform::Windows => install_windows_desktop(installer_url, directory),
        Platform::MacOS | Platform::Linux => {
            open_download_page(installer_url)?;
            Err(io::Error::other(
                "Automatic DeepSeek Harness installation is currently supported on Windows; the official release page was opened.",
            ))
        }
    }
}

fn install_windows_desktop(installer_url: &str, directory: &Path) -> io::Result<()> {
    use std::io::{Read as _, Write as _};

    std::fs::create_dir_all(directory)?;
    let partial = directory.join("deepseek-harness-windows-x64.exe.part");
    let installer = directory.join("deepseek-harness-windows-x64.exe");
    let download = (|| -> io::Result<()> {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(900)))
            .build();
        let agent: ureq::Agent = config.into();
        let mut response = agent
            .get(installer_url)
            .call()
            .map_err(|error| io::Error::other(format!(
                "Could not download the official DeepSeek Harness installer: {error}"
            )))?;
        let mut file = std::fs::File::create(&partial)?;
        let reader = response.body_mut().as_reader();
        let mut limited = reader.take(1_000_000_001);
        std::io::copy(&mut limited, &mut file)?;
        file.flush()?;
        file.sync_all()?;

        let size = file.metadata()?.len();
        if !(1_000_000..=1_000_000_000).contains(&size) {
            return Err(io::Error::other(format!(
                "The downloaded installer has an unexpected size ({size} bytes); it was not run."
            )));
        }
        let mut signature = [0u8; 2];
        std::fs::File::open(&partial)?.read_exact(&mut signature)?;
        if signature != *b"MZ" {
            return Err(io::Error::other(
                "The downloaded file is not a Windows executable; it was not run."
            ));
        }
        drop(file);
        verify_windows_installer(&partial)?;
        if installer.exists() {
            std::fs::remove_file(&installer)?;
        }
        std::fs::rename(&partial, &installer)?;
        Ok(())
    })();
    if let Err(error) = download {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    let status = Command::new(installer).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("DeepSeek Harness setup exited with {status}.")))
    }
}

pub(crate) fn open_download_page(url: &str) -> io::Result<()> {
    let mut command = match Platform::current() {
        Platform::Windows => {
            let mut command = Command::new("cmd.exe");
            command.args(["/C", "start", "", url]);
            command
        },
        Platform::MacOS => Command::new("open"),
        Platform::Linux => Command::new("xdg-open"),
    };
    if Platform::current() != Platform::Windows {
        command.arg(url);
    }
    command.spawn().map(|_| ())
}
