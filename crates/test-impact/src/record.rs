//! Recording, as cargo's target runner over a build with
//! `-C instrument-coverage`: each test runs with a profile of its own, and
//! the workspace functions that ran are written as line ranges.
//!
//! Turning one test's profile into source ranges with `llvm-cov export`
//! takes seconds and hundreds of megabytes on the integration binary. So
//! that's done once per binary, without a profile, into a table of every
//! function's range; a test needs only `llvm-profdata merge --sparse`,
//! which lists the functions that ran, looked up in the table.
//!
//! Environment: TEST_IMPACT_DIR (where recordings and tables go),
//! TEST_IMPACT_LLVM_BIN (the toolchain's llvm-tools), TEST_IMPACT_ROOT (the
//! workspace as the binary was compiled, by default the nearest directory up
//! with a Cargo.lock).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::Result;

/// One test's recording: a line of `recording-<pid>.jsonl`.
#[derive(Serialize, Deserialize)]
pub struct Recording {
    pub binary: String,
    pub test: String,
    pub seconds: f64,
    /// Workspace file -> line ranges of the functions that ran in it.
    pub files: BTreeMap<String, BTreeSet<(u32, u32)>>,
}

/// A binary's workspace functions: mangled name -> (file, first line, last
/// line). Generic functions have an entry per instantiation, all with the
/// same range.
#[derive(Serialize, Deserialize)]
pub struct Table {
    pub functions: HashMap<String, (String, u32, u32)>,
}

pub fn runner(binary: &str, args: &[String]) -> Result<u8> {
    let dir = PathBuf::from(env("TEST_IMPACT_DIR")?);
    let profiles = dir.join("profiles");
    fs::create_dir_all(&profiles)?;
    let profile = profiles.join(format!("{}.profraw", std::process::id()));

    // A run of one test is `--exact <name>`; listing records nothing. The
    // binary is its file name until build-map finds its nextest id (newer
    // nextest sets NEXTEST_BINARY_ID and NEXTEST_TEST_NAME).
    let exact = args
        .iter()
        .position(|arg| arg == "--exact")
        .and_then(|i| args.get(i + 1));
    let test = match (
        std::env::var("NEXTEST_BINARY_ID"),
        std::env::var("NEXTEST_TEST_NAME"),
    ) {
        (Ok(binary_id), Ok(name)) => Some((binary_id, name)),
        _ => exact.map(|name| (binary_name(Path::new(binary)), name.clone())),
    };

    let started = Instant::now();
    let status = Command::new(binary)
        .args(args)
        .env("LLVM_PROFILE_FILE", &profile)
        .status()?;
    let seconds = started.elapsed().as_secs_f64();
    let code = exit_code(status);

    if let Some((binary_id, name)) = test {
        let recorded = record(&dir, Path::new(binary), &profile).map(|files| Recording {
            binary: binary_id,
            test: name,
            seconds: (seconds * 100.0).round() / 100.0,
            files,
        });
        match recorded {
            Ok(recording) => append(&dir, &recording)?,
            // A test that passes but isn't recorded would silently drop out
            // of the map: logged for the recording job to fail on, since
            // a failed test looks the same to nextest.
            Err(err) => {
                let _ = fs::remove_file(&profile);
                let message = format!("recording {binary} {args:?}: {err}");
                let mut log = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dir.join("errors.log"))?;
                writeln!(log, "{message}")?;
                return Err(message.into());
            }
        }
    }
    let _ = fs::remove_file(&profile);
    Ok(code)
}

fn record(
    dir: &Path,
    binary: &Path,
    profile: &Path,
) -> Result<BTreeMap<String, BTreeSet<(u32, u32)>>> {
    let table = Table::for_binary(dir, binary)?;
    let output = Command::new(llvm_tool("llvm-profdata")?)
        .args(["merge", "--sparse", "--text", "-o", "-"])
        .arg(profile)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    let mut files: BTreeMap<String, BTreeSet<(u32, u32)>> = BTreeMap::new();
    for name in functions_run(&String::from_utf8_lossy(&output.stdout)) {
        if let Some((file, first, last)) = table.functions.get(name) {
            files
                .entry(file.clone())
                .or_default()
                .insert((*first, *last));
        }
    }
    Ok(files)
}

/// The functions with a nonzero counter in `llvm-profdata merge --text`
/// output: a name line, then `# Func Hash:`, `# Num Counters:` and
/// `# Counter Values:` each followed by values, then a blank line.
fn functions_run(text: &str) -> Vec<&str> {
    let mut ran = Vec::new();
    let mut name = None;
    let mut in_counters = false;
    let mut counted = false;
    for line in text.lines() {
        if line.is_empty() {
            name = None;
            in_counters = false;
            counted = false;
        } else if line.starts_with('#') {
            in_counters = line.starts_with("# Counter Values");
        } else if line.starts_with(':') {
            // Profile-wide flags, such as `:ir`.
        } else if name.is_none() {
            name = Some(line);
        } else if in_counters && !counted && line != "0" {
            counted = true;
            ran.extend(name);
        }
    }
    ran
}

fn append(dir: &Path, recording: &Recording) -> Result<()> {
    let path = dir.join(format!("recording-{}.jsonl", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", serde_json::to_string(recording)?)?;
    Ok(())
}

impl Table {
    /// Built by the first test of a binary to get here; the rest wait for
    /// it. Keyed by the binary's file name, which carries cargo's hash.
    pub fn for_binary(dir: &Path, binary: &Path) -> Result<Table> {
        let tables = dir.join("tables");
        fs::create_dir_all(&tables)?;
        let path = tables.join(format!("{}.json", binary_name(binary)));
        let lock = path.with_extension("lock");
        let started = Instant::now();
        loop {
            if path.exists() {
                return Ok(serde_json::from_slice(&fs::read(&path)?)?);
            }
            if fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock)
                .is_ok()
            {
                let built = Table::build(binary);
                let written = built.and_then(|table| {
                    let partial = path.with_extension("partial");
                    fs::write(&partial, serde_json::to_vec(&table)?)?;
                    fs::rename(&partial, &path)?;
                    Ok(table)
                });
                fs::remove_file(&lock)?;
                return written;
            }
            if started.elapsed() > Duration::from_secs(600) {
                return Err(format!("waited 10 minutes on {}", lock.display()).into());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    fn build(binary: &Path) -> Result<Table> {
        // nextest runs a test in its package's directory; paths are the
        // workspace's, as git names them.
        let root = match std::env::var("TEST_IMPACT_ROOT") {
            Ok(root) => PathBuf::from(root),
            Err(_) => {
                let cwd = std::env::current_dir()?;
                cwd.ancestors()
                    .find(|dir| dir.join("Cargo.lock").exists())
                    .map(Path::to_path_buf)
                    .unwrap_or(cwd)
            }
        };
        let prefix = format!("{}/", root.display());
        let output = Command::new(llvm_tool("llvm-cov")?)
            .args(["export", "-empty-profile", "-skip-expansions"])
            .arg("--ignore-filename-regex=/rustc/|/\\.cargo/|/target/|/nix/store/")
            .arg(binary)
            .output()?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
        }
        let export: Export = serde_json::from_slice(&output.stdout)?;
        let mut functions = HashMap::new();
        for data in export.data {
            for function in data.functions {
                let Some(file) = function.filenames.first() else {
                    continue;
                };
                let Some(file) = file.strip_prefix(&prefix) else {
                    continue;
                };
                // A region is [line, column, end line, end column, count,
                // file id, ...]; file 0 is the function's own.
                let lines = function.regions.iter().filter(|region| region[5] == 0);
                let first = lines.clone().map(|region| region[0]).min();
                let last = lines.map(|region| region[2]).max();
                if let (Some(first), Some(last)) = (first, last) {
                    functions.insert(function.name, (file.to_string(), first, last));
                }
            }
        }
        Ok(Table { functions })
    }
}

#[derive(Deserialize)]
struct Export {
    data: Vec<ExportData>,
}

#[derive(Deserialize)]
struct ExportData {
    functions: Vec<ExportFunction>,
}

#[derive(Deserialize)]
struct ExportFunction {
    name: String,
    regions: Vec<Vec<u32>>,
    filenames: Vec<String>,
}

fn binary_name(binary: &Path) -> String {
    binary
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn llvm_tool(name: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(env("TEST_IMPACT_LLVM_BIN")?).join(name))
}

fn env(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| format!("{name} is not set").into())
}

fn exit_code(status: std::process::ExitStatus) -> u8 {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(code), _) => code.clamp(0, 255) as u8,
        // A test killed by a signal (an abort, say) fails like a shell's.
        (None, Some(signal)) => (128 + signal).clamp(0, 255) as u8,
        (None, None) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_functions_with_a_nonzero_counter() {
        let text = "\
:ir
_RNvCs1_5thing3ran
# Func Hash:
42
# Num Counters:
2
# Counter Values:
0
3

_RNvCs1_5thing4idle
# Func Hash:
43
# Num Counters:
1
# Counter Values:
0
";
        assert_eq!(functions_run(text), ["_RNvCs1_5thing3ran"]);
    }
}
