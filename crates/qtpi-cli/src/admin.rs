use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;

const START_MARKER: &str = "# >>> qtpi >>>";
const END_MARKER: &str = "# <<< qtpi <<<";
const LEGACY_START_MARKER: &str = "# >>> 2cp >>>";
const LEGACY_END_MARKER: &str = "# <<< 2cp <<<";
const EMBEDDED_ZSH_HOOK: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../shell/zsh/qtpi.zsh"
));

#[derive(Clone, Debug)]
pub struct PathOverrides {
    pub bin_path: Option<PathBuf>,
    pub hook_path: Option<PathBuf>,
    pub rc_file: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallPaths {
    pub bin_path: PathBuf,
    pub hook_path: PathBuf,
    pub rc_file: PathBuf,
    pub cache_dir: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DoctorReport {
    pub shell: String,
    pub overall_status: CheckStatus,
    pub bin_path: PathBuf,
    pub hook_path: PathBuf,
    pub rc_file: PathBuf,
    pub cache_dir: PathBuf,
    pub checks: Vec<DoctorCheck>,
}

pub fn resolve_install_paths(overrides: &PathOverrides) -> Result<InstallPaths> {
    let home_dir = home_dir()?;
    let config_dir = config_dir(&home_dir);
    let cache_dir = cache_dir(&home_dir);
    let bin_path = match &overrides.bin_path {
        Some(path) => expand_tilde(path, &home_dir),
        None => env::current_exe().context("failed to determine current executable path")?,
    };
    let hook_path = overrides
        .hook_path
        .as_ref()
        .map(|path| expand_tilde(path, &home_dir))
        .unwrap_or_else(|| config_dir.join("qtpi").join("zsh").join("qtpi.zsh"));
    let rc_file = overrides
        .rc_file
        .as_ref()
        .map(|path| expand_tilde(path, &home_dir))
        .unwrap_or_else(|| home_dir.join(".zshrc"));

    Ok(InstallPaths {
        bin_path,
        hook_path,
        rc_file,
        cache_dir: cache_dir.join("qtpi"),
    })
}

pub fn print_shell_hook(paths: &InstallPaths) -> String {
    managed_block(&paths.bin_path, &paths.hook_path)
}

pub fn install(paths: &InstallPaths, force: bool) -> Result<()> {
    ensure_bin_path(&paths.bin_path)?;
    write_hook_file(&paths.hook_path, force)?;
    remove_legacy_hook_file(paths)?;
    let mut rc_contents = read_optional_file(&paths.rc_file)?;
    rc_contents = upsert_managed_block(
        &rc_contents,
        &managed_block(&paths.bin_path, &paths.hook_path),
    )?;
    write_atomic(&paths.rc_file, rc_contents.as_bytes())?;
    fs::create_dir_all(&paths.cache_dir).with_context(|| {
        format!(
            "failed to create cache directory at {}",
            paths.cache_dir.display()
        )
    })?;
    Ok(())
}

pub fn uninstall(paths: &InstallPaths) -> Result<()> {
    let rc_contents = read_optional_file(&paths.rc_file)?;
    let updated = remove_managed_block(&rc_contents)?;
    if updated != rc_contents {
        write_atomic(&paths.rc_file, updated.as_bytes())?;
    }

    match fs::remove_file(&paths.hook_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to remove hook file at {}",
                    paths.hook_path.display()
                )
            });
        }
    }

    remove_legacy_hook_file(paths)?;

    Ok(())
}

pub fn run_doctor(paths: &InstallPaths) -> DoctorReport {
    let checks = vec![
        bin_check(&paths.bin_path),
        hook_check(&paths.hook_path),
        rc_file_check(&paths.rc_file),
        zsh_check(),
        overlay_check(),
        suggest_check(&paths.bin_path),
    ];

    let overall_status = checks
        .iter()
        .map(|check| check.status)
        .max_by_key(|status| match status {
            CheckStatus::Ok => 0,
            CheckStatus::Warning => 1,
            CheckStatus::Error => 2,
        })
        .unwrap_or(CheckStatus::Ok);

    DoctorReport {
        shell: "zsh".to_string(),
        overall_status,
        bin_path: paths.bin_path.clone(),
        hook_path: paths.hook_path.clone(),
        rc_file: paths.rc_file.clone(),
        cache_dir: paths.cache_dir.clone(),
        checks,
    }
}

pub fn render_doctor_text(report: &DoctorReport) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "overall: {}\n",
        status_label(report.overall_status)
    ));
    output.push_str(&format!("shell: {}\n", report.shell));
    output.push_str(&format!("bin_path: {}\n", report.bin_path.display()));
    output.push_str(&format!("hook_path: {}\n", report.hook_path.display()));
    output.push_str(&format!("rc_file: {}\n", report.rc_file.display()));
    output.push_str(&format!("cache_dir: {}\n", report.cache_dir.display()));
    for check in &report.checks {
        output.push_str(&format!(
            "{}: {} ({})\n",
            check.name,
            status_label(check.status),
            check.detail
        ));
    }
    output
}

fn status_label(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Ok => "ok",
        CheckStatus::Warning => "warning",
        CheckStatus::Error => "error",
    }
}

fn bin_check(path: &Path) -> DoctorCheck {
    if path.is_file() {
        DoctorCheck {
            name: "binary".to_string(),
            status: CheckStatus::Ok,
            detail: format!("found executable target at {}", path.display()),
        }
    } else {
        DoctorCheck {
            name: "binary".to_string(),
            status: CheckStatus::Error,
            detail: format!("missing binary at {}", path.display()),
        }
    }
}

fn hook_check(path: &Path) -> DoctorCheck {
    if path.is_file() {
        DoctorCheck {
            name: "hook_file".to_string(),
            status: CheckStatus::Ok,
            detail: format!("found hook file at {}", path.display()),
        }
    } else {
        DoctorCheck {
            name: "hook_file".to_string(),
            status: CheckStatus::Warning,
            detail: format!("missing hook file at {}", path.display()),
        }
    }
}

fn rc_file_check(path: &Path) -> DoctorCheck {
    match fs::read_to_string(path) {
        Ok(contents) => {
            if contents.contains(START_MARKER) && contents.contains(END_MARKER) {
                DoctorCheck {
                    name: "rc_file".to_string(),
                    status: CheckStatus::Ok,
                    detail: format!("managed qtpi block present in {}", path.display()),
                }
            } else {
                DoctorCheck {
                    name: "rc_file".to_string(),
                    status: CheckStatus::Warning,
                    detail: format!("managed qtpi block missing from {}", path.display()),
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => DoctorCheck {
            name: "rc_file".to_string(),
            status: CheckStatus::Warning,
            detail: format!("rc file not found at {}", path.display()),
        },
        Err(error) => DoctorCheck {
            name: "rc_file".to_string(),
            status: CheckStatus::Error,
            detail: format!("failed to read {}: {error}", path.display()),
        },
    }
}

fn zsh_check() -> DoctorCheck {
    match Command::new("zsh").arg("--version").output() {
        Ok(output) if output.status.success() => DoctorCheck {
            name: "zsh".to_string(),
            status: CheckStatus::Ok,
            detail: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        },
        Ok(output) => DoctorCheck {
            name: "zsh".to_string(),
            status: CheckStatus::Error,
            detail: format!("zsh returned non-zero status {}", output.status),
        },
        Err(error) => DoctorCheck {
            name: "zsh".to_string(),
            status: CheckStatus::Error,
            detail: format!("failed to execute zsh: {error}"),
        },
    }
}

fn overlay_check() -> DoctorCheck {
    if matches!(env::var("TERM"), Ok(term) if term == "dumb") {
        return DoctorCheck {
            name: "overlay".to_string(),
            status: CheckStatus::Warning,
            detail: "TERM is dumb; dropdown overlay will stay disabled".to_string(),
        };
    }

    let script = "zmodload zsh/terminfo 2>/dev/null || exit 11; for capability in sc rc cup el; do [[ -n ${terminfo[$capability]-} ]] || exit 12; done";
    match Command::new("zsh").arg("-lc").arg(script).status() {
        Ok(status) if status.success() => DoctorCheck {
            name: "overlay".to_string(),
            status: CheckStatus::Ok,
            detail: "zsh terminfo reports required overlay capabilities".to_string(),
        },
        Ok(status) => DoctorCheck {
            name: "overlay".to_string(),
            status: CheckStatus::Warning,
            detail: format!("zsh terminfo capability probe failed with status {status}"),
        },
        Err(error) => DoctorCheck {
            name: "overlay".to_string(),
            status: CheckStatus::Warning,
            detail: format!("failed to run overlay capability probe: {error}"),
        },
    }
}

fn suggest_check(bin_path: &Path) -> DoctorCheck {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match Command::new(bin_path)
        .arg("suggest")
        .arg("--shell")
        .arg("zsh")
        .arg("--buffer")
        .arg("git ")
        .arg("--cursor")
        .arg("4")
        .arg("--cursor-units")
        .arg("chars")
        .arg("--cwd")
        .arg(&cwd)
        .arg("--format")
        .arg("json")
        .output()
    {
        Ok(output) if output.status.success() => {
            let body = String::from_utf8_lossy(&output.stdout);
            if body.contains("\"status\":\"ok\"") && body.contains("\"suggestions\":[") {
                DoctorCheck {
                    name: "suggest".to_string(),
                    status: CheckStatus::Ok,
                    detail: "static suggest sanity check succeeded for `git `".to_string(),
                }
            } else {
                DoctorCheck {
                    name: "suggest".to_string(),
                    status: CheckStatus::Warning,
                    detail: "suggest command returned an unexpected payload".to_string(),
                }
            }
        }
        Ok(output) => DoctorCheck {
            name: "suggest".to_string(),
            status: CheckStatus::Error,
            detail: format!("suggest command failed with status {}", output.status),
        },
        Err(error) => DoctorCheck {
            name: "suggest".to_string(),
            status: CheckStatus::Error,
            detail: format!("failed to execute suggest sanity check: {error}"),
        },
    }
}

fn ensure_bin_path(path: &Path) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }

    bail!("binary path does not exist: {}", path.display())
}

fn write_hook_file(path: &Path, force: bool) -> Result<()> {
    if path.exists() && !force {
        let existing = fs::read_to_string(path)
            .with_context(|| format!("failed to read existing hook file at {}", path.display()))?;
        if existing == EMBEDDED_ZSH_HOOK {
            return Ok(());
        }
    }

    write_atomic(path, EMBEDDED_ZSH_HOOK.as_bytes())
}

fn remove_legacy_hook_file(paths: &InstallPaths) -> Result<()> {
    let legacy_hook_path = paths
        .hook_path
        .parent()
        .and_then(Path::parent)
        .map(|config_root| config_root.join("2cp").join("zsh").join("twocp.zsh"));

    let Some(legacy_hook_path) = legacy_hook_path else {
        return Ok(());
    };

    if legacy_hook_path == paths.hook_path {
        return Ok(());
    }

    match fs::remove_file(&legacy_hook_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to remove legacy hook file at {}",
                legacy_hook_path.display()
            )
        }),
    }
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create parent directory {}", parent.display()))?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_path = parent.join(format!(".{}.tmp-{nanos}", file_name_or_default(path)));
    fs::write(&temp_path, contents)
        .with_context(|| format!("failed to write temporary file {}", temp_path.display()))?;
    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to atomically move {} into place at {}",
            temp_path.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn file_name_or_default(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("qtpi")
        .to_string()
}

fn read_optional_file(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn managed_block(bin_path: &Path, hook_path: &Path) -> String {
    format!(
        "{START_MARKER}\nexport QTPI_BIN={}\nsource {}\n{END_MARKER}\n",
        shell_quote_path(bin_path),
        shell_quote_path(hook_path)
    )
}

fn shell_quote_path(path: &Path) -> String {
    shell_quote(&path.to_string_lossy())
}

fn shell_quote(value: &str) -> String {
    let escaped = value.replace('\'', "'\\''");
    format!("'{escaped}'")
}

fn upsert_managed_block(contents: &str, block: &str) -> Result<String> {
    if let Some((start, end)) = managed_block_range(contents)? {
        let mut updated = String::new();
        updated.push_str(&contents[..start]);
        updated.push_str(block);
        updated.push_str(&contents[end..]);
        return Ok(updated);
    }

    let mut updated = contents.to_string();
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    if !updated.is_empty() {
        updated.push('\n');
    }
    updated.push_str(block);
    Ok(updated)
}

fn remove_managed_block(contents: &str) -> Result<String> {
    let Some((start, end)) = managed_block_range(contents)? else {
        return Ok(contents.to_string());
    };

    let mut updated = String::new();
    updated.push_str(&contents[..start]);
    updated.push_str(&contents[end..]);
    while updated.contains("\n\n\n") {
        updated = updated.replace("\n\n\n", "\n\n");
    }
    let normalized = updated.trim_start_matches('\n');
    let normalized = normalized.trim_end_matches('\n');
    if normalized.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("{normalized}\n"))
    }
}

fn managed_block_range(contents: &str) -> Result<Option<(usize, usize)>> {
    managed_block_range_for_markers(contents, START_MARKER, END_MARKER)
        .or_else(|| {
            managed_block_range_for_markers(contents, LEGACY_START_MARKER, LEGACY_END_MARKER)
        })
        .transpose()
}

fn managed_block_range_for_markers(
    contents: &str,
    start_marker: &str,
    end_marker: &str,
) -> Option<Result<(usize, usize)>> {
    let start = contents.find(start_marker)?;
    let Some(end_marker_start) = contents[start..]
        .find(end_marker)
        .map(|offset| start + offset)
    else {
        return Some(Err(anyhow!(
            "found managed start marker without matching end marker"
        )));
    };
    let end = line_end_after(contents, end_marker_start + end_marker.len());
    Some(Ok((start, end)))
}

fn line_end_after(contents: &str, start: usize) -> usize {
    contents[start..]
        .find('\n')
        .map(|offset| start + offset + 1)
        .unwrap_or(contents.len())
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("HOME is not set"))
}

fn config_dir(home_dir: &Path) -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                home_dir.join("Library").join("Application Support")
            } else {
                home_dir.join(".config")
            }
        })
}

fn cache_dir(home_dir: &Path) -> PathBuf {
    env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                home_dir.join("Library").join("Caches")
            } else {
                home_dir.join(".cache")
            }
        })
}

fn expand_tilde(path: &Path, home_dir: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    if value == "~" {
        return home_dir.to_path_buf();
    }
    if let Some(stripped) = value.strip_prefix("~/") {
        return home_dir.join(stripped);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn upsert_managed_block_appends_to_new_rc_file() {
        let block = managed_block(Path::new("/bin/qtpi"), Path::new("/tmp/qtpi.zsh"));
        let updated = upsert_managed_block("", &block).expect("block should append");
        assert_eq!(updated, block);
    }

    #[test]
    fn upsert_managed_block_replaces_existing_block() {
        let original = "\
export PATH=/bin\n\
\n\
# >>> qtpi >>>\n\
export QTPI_BIN='old'\n\
source 'old-hook'\n\
# <<< qtpi <<<\n";
        let replacement = managed_block(Path::new("/new/qtpi"), Path::new("/new/qtpi.zsh"));
        let updated = upsert_managed_block(original, &replacement).expect("block should replace");

        assert!(updated.contains("export PATH=/bin"));
        assert!(updated.contains("/new/qtpi.zsh"));
        assert!(!updated.contains("old-hook"));
    }

    #[test]
    fn remove_managed_block_preserves_unrelated_rc_content() {
        let original = "\
export PATH=/bin\n\
\n\
# >>> qtpi >>>\n\
export QTPI_BIN='old'\n\
source 'old-hook'\n\
# <<< qtpi <<<\n\
\n\
alias gs='git status'\n";
        let updated = remove_managed_block(original).expect("block should remove");

        assert_eq!(updated, "export PATH=/bin\n\nalias gs='git status'\n");
    }

    #[test]
    fn upsert_managed_block_replaces_legacy_block() {
        let original = "\
export PATH=/bin\n\
\n\
# >>> 2cp >>>\n\
export TWOCP_BIN='old'\n\
source 'old-hook'\n\
# <<< 2cp <<<\n";
        let replacement = managed_block(Path::new("/new/qtpi"), Path::new("/new/qtpi.zsh"));
        let updated =
            upsert_managed_block(original, &replacement).expect("legacy block should replace");

        assert!(updated.contains(START_MARKER));
        assert!(!updated.contains(LEGACY_START_MARKER));
        assert!(!updated.contains("old-hook"));
    }

    #[test]
    fn remove_managed_block_removes_legacy_block() {
        let original = "\
export PATH=/bin\n\
\n\
# >>> 2cp >>>\n\
export TWOCP_BIN='old'\n\
source 'old-hook'\n\
# <<< 2cp <<<\n\
\n\
alias gs='git status'\n";
        let updated = remove_managed_block(original).expect("legacy block should remove");

        assert_eq!(updated, "export PATH=/bin\n\nalias gs='git status'\n");
    }

    #[test]
    fn install_and_uninstall_manage_hook_and_rc_file() {
        let tempdir = tempdir().expect("tempdir should exist");
        let home_dir = tempdir.path().join("home");
        let config_dir = home_dir.join(".config");
        let cache_dir = home_dir.join(".cache");
        let bin_path = tempdir.path().join("bin").join("qtpi");
        let rc_file = home_dir.join(".zshrc");
        let hook_path = config_dir.join("qtpi").join("zsh").join("qtpi.zsh");

        fs::create_dir_all(bin_path.parent().expect("bin parent"))
            .expect("bin parent should exist");
        fs::create_dir_all(&config_dir).expect("config dir should exist");
        fs::create_dir_all(&cache_dir).expect("cache dir should exist");
        fs::write(&bin_path, "stub").expect("stub bin should write");
        fs::write(&rc_file, "export PATH=/bin\n").expect("rc file should write");

        let paths = InstallPaths {
            bin_path: bin_path.clone(),
            hook_path: hook_path.clone(),
            rc_file: rc_file.clone(),
            cache_dir: cache_dir.join("qtpi"),
        };
        install(&paths, false).expect("install should succeed");

        let rc_after_install = fs::read_to_string(&rc_file).expect("rc file should read");
        assert!(rc_after_install.contains(START_MARKER));
        assert!(rc_after_install.contains(&hook_path.to_string_lossy().to_string()));
        assert!(hook_path.is_file());
        assert!(paths.cache_dir.is_dir());

        uninstall(&paths).expect("uninstall should succeed");

        let rc_after_uninstall = fs::read_to_string(&rc_file).expect("rc file should still read");
        assert_eq!(rc_after_uninstall, "export PATH=/bin\n");
        assert!(!hook_path.exists());
    }

    #[test]
    fn doctor_reports_missing_install_state() {
        let tempdir = tempdir().expect("tempdir should exist");
        let paths = InstallPaths {
            bin_path: tempdir.path().join("missing-bin"),
            hook_path: tempdir.path().join("missing-hook"),
            rc_file: tempdir.path().join("missing-zshrc"),
            cache_dir: tempdir.path().join("cache"),
        };

        let report = run_doctor(&paths);
        assert_eq!(report.shell, "zsh");
        assert!(report.checks.iter().any(|check| check.name == "binary"));
        assert!(report.checks.iter().any(|check| check.name == "suggest"));
        assert_eq!(report.overall_status, CheckStatus::Error);
    }
}
