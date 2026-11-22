//! DeepSeek Harness Desktop discovery and its independent Desktop-profile bridge.
//!
//! The desktop application owns its runtime. Pebrel only manages the local
//! Desktop profile bundle and launches the installed application; it never
//! mutates the user's global Node/npm installation.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BUNDLE_NAME: &str = "@pebrel/deepseek-harness-bridge";
const DOWNLOAD_PAGE: &str = "https://github.com/deepseek-ai/deepseek-harness/releases/latest";
// Upstream fixed production download documented in apps/desktop/publishing.
const WINDOWS_INSTALLER_URL: &str = "https://download.deepseek.com/desktop/dsh-latest-windows-x64.exe";

const BRIDGE_JS: &str =
    include_str!("../../../../integrations/deepseek-harness-bridge/index.js");
const BRIDGE_PACKAGE: &str =
    include_str!("../../../../integrations/deepseek-harness-bridge/package.json");
const BRIDGE_PATCH: &str =
    include_str!("../../../../integrations/deepseek-harness-bridge/cordis.patch.yml");

#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    pub desktop_executable: Option<PathBuf>,
    pub desktop_installed: bool,
    pub desktop_running: bool,
    pub dsh_cli: Option<PathBuf>,
    pub profile_manifest: Option<PathBuf>,
    pub profile_initialized: bool,
    pub bridge_directory: PathBuf,
    pub hook_installed: bool,
    pub hook_current: bool,
}

pub(crate) fn inspect() -> State {
    let desktop_executable = desktop_executable();
    let dsh_cli = dsh_cli(desktop_executable.as_deref());
    let profile_manifest = profile_dir().map(|path| path.join("package.json"));
    let profile_initialized =
        profile_manifest.as_ref().is_some_and(|path| path.is_file());
    let bridge_directory = bridge_directory();
    let (hook_installed, hook_current) = profile_manifest
        .as_deref()
        .map(|path| inspect_manifest(path, &bridge_directory))
        .unwrap_or_default();

    State {
        desktop_installed: desktop_executable.is_some(),
        desktop_running: desktop_running(),
        desktop_executable,
        dsh_cli,
        profile_manifest,
        profile_initialized,
        bridge_directory,
        hook_installed,
        hook_current,
    }
}

pub(crate) fn profile_manifest() -> Option<PathBuf> {
    profile_dir().map(|path| path.join("package.json"))
}

fn profile_dir() -> Option<PathBuf> {
    let home = std::env::var_os("DSH_HOME")
        .map(PathBuf::from)
        .or_else(|| crate::platform::dirs::home_dir().map(|home| home.join(".dsh")))?;
    Some(home.join("profiles").join("desktop"))
}

fn bridge_directory() -> PathBuf {
    pebrel_settings::settings_dir()
        .join("integrations")
        .join("deepseek-harness-bridge")
}

pub(crate) fn desktop_executable() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(directory) = crate::platform::deepseek_harness::registered_install_directory() {
        candidates.push(directory.join("DeepSeek Harness.exe"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        candidates.extend([
            local.join("Programs/DeepSeek Harness/DeepSeek Harness.exe"),
            local.join("DeepSeek Harness/DeepSeek Harness.exe"),
        ]);
    }
    for variable in ["ProgramFiles", "ProgramW6432"] {
        if let Some(program_files) = std::env::var_os(variable) {
            candidates.push(
                PathBuf::from(program_files).join("DeepSeek Harness/DeepSeek Harness.exe"),
            );
        }
    }
    if let Some(home) = crate::platform::dirs::home_dir() {
        candidates.push(home.join("Applications/DeepSeek Harness.app/Contents/MacOS/DeepSeek Harness"));
        candidates.push(home.join(".local/bin/deepseek-harness-desktop"));
    }
    candidates.into_iter().find(|path| path.is_file())
}


fn dsh_cli(desktop: Option<&Path>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(executable) = desktop {
        if let Some(root) = executable.parent() {
            candidates.extend([
                root.join("resources/runtime/cli/bin/dsh.cmd"),
                root.join("resources/runtime/cli/bin/dsh"),
                root.join("Contents/Resources/runtime/cli/bin/dsh"),
            ]);
        }
    }
    // Do not fall back to npm-global dsh: upstream explicitly restricts
    // Desktop-profile mutation to the runtime bundled with the Desktop app.
    candidates.into_iter().find(|path| path.is_file())
}

fn desktop_running() -> bool {
    crate::platform::deepseek_harness::desktop_running()
}

fn inspect_manifest(path: &Path, bridge_dir: &Path) -> (bool, bool) {
    let Ok(raw) = std::fs::read_to_string(path) else { return (false, false) };
    let Ok(manifest) = serde_json::from_str::<Value>(&raw) else { return (false, false) };
    let dependency = manifest
        .get("dependencies")
        .and_then(Value::as_object)
        .and_then(|dependencies| dependencies.get(BUNDLE_NAME))
        .is_some();
    let bundle = manifest
        .get("dsh")
        .and_then(|dsh| dsh.get("profile"))
        .and_then(|profile| profile.get("bundles"))
        .and_then(Value::as_array)
        .is_some_and(|bundles| bundles.iter().any(|value| value.as_str() == Some(BUNDLE_NAME)));
    let installed = dependency || bundle;
    let file_matches = |name: &str, expected: &str| {
        std::fs::read_to_string(bridge_dir.join(name))
            .is_ok_and(|actual| actual == expected)
    };
    let current = dependency
        && bundle
        && file_matches("index.js", BRIDGE_JS)
        && file_matches("package.json", BRIDGE_PACKAGE)
        && file_matches("cordis.patch.yml", BRIDGE_PATCH);
    (installed, current)
}

fn write_owned_bundle() -> std::io::Result<()> {
    let directory = bridge_directory();
    std::fs::create_dir_all(&directory)?;
    for (name, content) in [
        ("index.js", BRIDGE_JS),
        ("package.json", BRIDGE_PACKAGE),
        ("cordis.patch.yml", BRIDGE_PATCH),
    ] {
        let path = directory.join(name);
        match std::fs::read_to_string(&path) {
            Ok(existing) if existing == content => continue,
            Ok(_) => {
                return Err(std::io::Error::other(format!(
                    "Pebrel bridge file was edited; preserving it: {}",
                    path.display()
                )));
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
        let temporary = path.with_extension(format!("pebrel-tmp-{}", std::process::id()));
        std::fs::write(&temporary, content)?;
        if let Err(error) = std::fs::rename(&temporary, &path) {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
    }
    Ok(())
}


fn find_uninstaller(executable: &Path) -> Option<PathBuf> {
    const NAMES: &[&str] = &[
        "Uninstall DeepSeek Harness.exe",
        "unins000.exe",
        "uninstall.exe",
    ];
    let parent = executable.parent()?;
    let find_in = |directory: &Path| {
        NAMES.iter().map(|name| directory.join(name)).find(|path| path.is_file())
    };
    find_in(parent).or_else(|| {
        // Only look one level up for an Electron-style version directory.
        // Do not wander into generic parents such as "Programs" or a drive root.
        let version_directory = parent.file_name()?.to_string_lossy().to_ascii_lowercase();
        if !version_directory.starts_with("app-") {
            return None;
        }
        find_in(parent.parent()?)
    })
}

fn run_dsh(cli: &Path, arguments: &[&str]) -> std::io::Result<()> {
    crate::platform::deepseek_harness::run_dsh(cli, arguments)
}

pub(crate) fn install_hook() -> std::io::Result<()> {
    let state = inspect();
    if !state.desktop_installed {
        return Err(std::io::Error::other(
            "DeepSeek Harness Desktop is not installed. Install the desktop application first.",
        ));
    }
    if state.desktop_running {
        return Err(std::io::Error::other(
            "Quit DeepSeek Harness Desktop from its tray menu before installing or removing its profile plugin; closing the window only hides it.",
        ));
    }
    if !state.profile_initialized {
        return Err(std::io::Error::other(
            "Open DeepSeek Harness Desktop once to initialize its desktop profile, then refresh Agents.",
        ));
    }
    let cli = state.dsh_cli.ok_or_else(|| {
        std::io::Error::other("The Desktop-bundled dsh command was not found.")
    })?;
    write_owned_bundle()?;
    let bundle = bridge_directory().to_string_lossy().into_owned();
    run_dsh(&cli, &["plugin", "--profile", "desktop", "add", &bundle])?;
    let state = inspect();
    if !state.hook_installed || !state.hook_current {
        return Err(std::io::Error::other(
            "dsh completed but the DeepSeek Desktop profile did not verify the Pebrel bridge.",
        ));
    }
    Ok(())
}

pub(crate) fn remove_hook() -> std::io::Result<()> {
    let state = inspect();
    if !state.hook_installed {
        return Ok(());
    }
    if state.desktop_running {
        return Err(std::io::Error::other(
            "Quit DeepSeek Harness Desktop before installing or removing its profile plugin.",
        ));
    }
    let cli = state.dsh_cli.ok_or_else(|| {
        std::io::Error::other("The Desktop-bundled dsh command was not found.")
    })?;
    run_dsh(&cli, &["plugin", "--profile", "desktop", "remove", BUNDLE_NAME])?;
    if inspect().hook_installed {
        return Err(std::io::Error::other("The DeepSeek Harness bridge remains in the Desktop profile."));
    }
    match std::fs::remove_dir_all(bridge_directory()) {
        Ok(()) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(error),
    }
    Ok(())
}

pub(crate) fn launch_desktop() -> std::io::Result<()> {
    let state = inspect();
    if state.desktop_running {
        return Err(std::io::Error::other(
            "DeepSeek Harness is already running. Quit it from the tray before launching it through Pebrel.",
        ));
    }
    let executable = state.desktop_executable.ok_or_else(|| {
        std::io::Error::other("DeepSeek Harness Desktop is not installed.")
    })?;
    let helper = super::helper_path();
    let pipe = std::env::var_os(crate::ai_hook::PIPE_ENV)
        .or_else(|| std::env::var_os(crate::ai_hook::LEGACY_PIPE_ENV));
    if state.hook_installed && !state.hook_current {
        return Err(std::io::Error::other(
            "The DeepSeek Harness Hook is not current. Toggle it off and on again before launch.",
        ));
    }
    if state.hook_installed && (helper.is_none() || pipe.is_none()) {
        return Err(std::io::Error::other(
            "Pebrel's private Hook helper or pipe is unavailable; restart Pebrel before launching DeepSeek Harness.",
        ));
    }
    let mut command = Command::new(executable);
    if let Some(helper) = helper {
        command.env(crate::ai_hook::HOOK_EXE_ENV, helper);
    }
    // The pipe is process-wide; the bridge adds session cwd so Pebrel can
    // return each event to the matching Pane even without a per-pane launcher.
    if let Some(pipe) = pipe {
        command.env(crate::ai_hook::PIPE_ENV, pipe);
    }
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    command.spawn()?;
    for _ in 0..40 {
        if desktop_running() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Err(std::io::Error::other(
        "DeepSeek Harness process did not appear after launch.",
    ))
}

pub(crate) fn uninstall_desktop() -> std::io::Result<()> {
    let state = inspect();
    if state.desktop_running {
        return Err(std::io::Error::other("Quit DeepSeek Harness Desktop from its tray menu before uninstalling it; closing the window only hides it."));
    }
    let executable = state.desktop_executable.ok_or_else(|| {
        std::io::Error::other("DeepSeek Harness Desktop is not installed.")
    })?;
    // Locate the uninstaller before changing the profile so a missing setup
    // component does not leave the user's Desktop profile half-uninstalled.
    // Electron/NSIS layouts may keep the executable in a versioned
    // subdirectory while placing the uninstaller higher in the install tree.
    let uninstaller = find_uninstaller(&executable)
        .ok_or_else(|| std::io::Error::other("The DeepSeek Harness uninstaller was not found."))?;
    remove_hook()?;
    let status = Command::new(uninstaller).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "DeepSeek Harness uninstaller exited with {status}."
        )))
    }
}

pub(crate) fn install_desktop() -> std::io::Result<()> {
    crate::platform::deepseek_harness::install_desktop(
        WINDOWS_INSTALLER_URL,
        &pebrel_settings::settings_dir().join("downloads"),
    )
}

pub(crate) fn open_download_page() -> std::io::Result<()> {
    crate::platform::deepseek_harness::open_download_page(DOWNLOAD_PAGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstaller_discovery_walks_up_versioned_install_layouts() {
        let directory = tempfile::tempdir().unwrap();
        let versioned = directory.path().join("app-1.2.3");
        std::fs::create_dir_all(&versioned).unwrap();
        let executable = versioned.join("DeepSeek Harness.exe");
        std::fs::write(&executable, b"fixture").unwrap();
        let uninstaller = directory.path().join("Uninstall DeepSeek Harness.exe");
        std::fs::write(&uninstaller, b"fixture").unwrap();
        assert_eq!(find_uninstaller(&executable), Some(uninstaller));
    }

    #[test]
    fn uninstaller_discovery_does_not_escape_into_generic_parent_directories() {
        let directory = tempfile::tempdir().unwrap();
        let install = directory.path().join("Programs/DeepSeek Harness");
        std::fs::create_dir_all(&install).unwrap();
        let executable = install.join("DeepSeek Harness.exe");
        std::fs::write(&executable, b"fixture").unwrap();
        std::fs::write(directory.path().join("Programs/uninstall.exe"), b"unrelated").unwrap();
        assert_eq!(find_uninstaller(&executable), None);
    }

    #[test]
    fn profile_bridge_presence_requires_both_dependency_and_bundle_layer() {
        let directory = tempfile::tempdir().unwrap();
        let bridge = directory.path().join("bridge");
        std::fs::create_dir_all(&bridge).unwrap();
        std::fs::write(bridge.join("index.js"), BRIDGE_JS).unwrap();
        std::fs::write(bridge.join("package.json"), BRIDGE_PACKAGE).unwrap();
        std::fs::write(bridge.join("cordis.patch.yml"), BRIDGE_PATCH).unwrap();
        let manifest = directory.path().join("package.json");
        let profile = serde_json::json!({
            "dependencies": { "@pebrel/deepseek-harness-bridge": "link:bridge" },
            "dsh": { "profile": { "bundles": ["@deepseek-ai/dsh-base", BUNDLE_NAME] } }
        });
        std::fs::write(&manifest, serde_json::to_vec(&profile).unwrap()).unwrap();
        assert_eq!(inspect_manifest(&manifest, &bridge), (true, true));

        let without_layer = serde_json::json!({
            "dependencies": { "@pebrel/deepseek-harness-bridge": "link:bridge" },
            "dsh": { "profile": { "bundles": ["@deepseek-ai/dsh-base"] } }
        });
        std::fs::write(&manifest, serde_json::to_vec(&without_layer).unwrap()).unwrap();
        assert_eq!(inspect_manifest(&manifest, &bridge), (true, false));
    }
}
