//! The map: every test, and for each workspace file its functions and which
//! tests ran each one. Built from a directory of recordings and tables,
//! which may hold the recordings of several CI shares.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::record::{Recording, Table};

#[derive(Serialize, Deserialize)]
pub struct Map {
    pub tests: Vec<Test>,
    pub files: BTreeMap<String, File>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Test {
    pub binary: String,
    pub name: String,
    pub seconds: f64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct File {
    /// Every function's line range, run or not.
    pub functions: Vec<(u32, u32)>,
    /// The functions that ran, with the indexes of the tests that ran them.
    pub ranges: Vec<(u32, u32, Vec<u32>)>,
}

/// `list` is `cargo nextest list --message-format json` output, for the
/// nextest id of each binary recorded by file name.
pub fn build(dir: &str, out: &str, list: Option<&str>) -> Result<()> {
    let dir = Path::new(dir);
    let binary_ids = match list {
        Some(list) => binary_ids(list)?,
        None => BTreeMap::new(),
    };
    // Each CI share's files sit in a directory of their own: the same table
    // comes from several, and process ids repeat across runners.
    let mut paths = Vec::new();
    files_under(dir, &mut paths)?;
    let file_name = |path: &Path| {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };

    let mut functions: BTreeMap<String, BTreeSet<(u32, u32)>> = BTreeMap::new();
    let mut tables_read = BTreeSet::new();
    for path in &paths {
        let in_tables = path
            .parent()
            .is_some_and(|parent| parent.ends_with("tables"));
        if !(in_tables && file_name(path).ends_with(".json"))
            || !tables_read.insert(file_name(path))
        {
            continue;
        }
        let table: Table = serde_json::from_slice(&fs::read(path)?)
            .map_err(|err| format!("{}: {err}", path.display()))?;
        for (file, first, last) in table.functions.into_values() {
            functions.entry(file).or_default().insert((first, last));
        }
    }

    let mut tests: Vec<Test> = Vec::new();
    let mut ids: BTreeMap<(String, String), u32> = BTreeMap::new();
    let mut ran: BTreeMap<String, BTreeMap<(u32, u32), BTreeSet<u32>>> = BTreeMap::new();
    for path in &paths {
        let name = file_name(path);
        if !(name.starts_with("recording-") && name.ends_with(".jsonl")) {
            continue;
        }
        for line in BufReader::new(fs::File::open(path)?).lines() {
            let mut recording: Recording =
                serde_json::from_str(&line?).map_err(|err| format!("{}: {err}", path.display()))?;
            if let Some(id) = binary_ids.get(&recording.binary) {
                recording.binary = id.clone();
            }
            let key = (recording.binary.clone(), recording.test.clone());
            let id = *ids.entry(key).or_insert_with(|| {
                tests.push(Test {
                    binary: recording.binary,
                    name: recording.test,
                    seconds: 0.0,
                });
                (tests.len() - 1) as u32
            });
            tests[id as usize].seconds += recording.seconds;
            for (file, ranges) in recording.files {
                let file_ran = ran.entry(file).or_default();
                for range in ranges {
                    file_ran.entry(range).or_default().insert(id);
                }
            }
        }
    }
    if tests.is_empty() {
        return Err(format!("no recordings in {}", dir.display()).into());
    }

    let mut files = BTreeMap::new();
    for path in functions
        .keys()
        .chain(ran.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
    {
        let file = File {
            functions: functions
                .get(&path)
                .map(|f| f.iter().copied().collect())
                .unwrap_or_default(),
            ranges: ran
                .get(&path)
                .map(|r| {
                    r.iter()
                        .map(|(&(s, e), ids)| (s, e, ids.iter().copied().collect()))
                        .collect()
                })
                .unwrap_or_default(),
        };
        files.insert(path, file);
    }
    let map = Map { tests, files };
    let mut gz = GzEncoder::new(fs::File::create(out)?, Compression::default());
    gz.write_all(&serde_json::to_vec(&map)?)?;
    gz.finish()?;
    eprintln!(
        "build-map: {} tests, {} files -> {out}",
        map.tests.len(),
        map.files.len()
    );
    Ok(())
}

fn files_under(dir: &Path, paths: &mut Vec<std::path::PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            files_under(&path, paths)?;
        } else {
            paths.push(path);
        }
    }
    Ok(())
}

/// Binary file name -> nextest binary id.
fn binary_ids(list: &str) -> Result<BTreeMap<String, String>> {
    #[derive(Deserialize)]
    struct List {
        #[serde(rename = "rust-suites")]
        suites: BTreeMap<String, Suite>,
    }
    #[derive(Deserialize)]
    struct Suite {
        #[serde(rename = "binary-path")]
        binary_path: String,
    }
    let list: List =
        serde_json::from_slice(&fs::read(list)?).map_err(|err| format!("{list}: {err}"))?;
    Ok(list
        .suites
        .into_iter()
        .filter_map(|(id, suite)| {
            let name = Path::new(&suite.binary_path)
                .file_name()?
                .to_string_lossy()
                .into_owned();
            Some((name, id))
        })
        .collect())
}

impl Map {
    pub fn load(path: &str) -> Result<Map> {
        let mut json = Vec::new();
        GzDecoder::new(fs::File::open(path)?).read_to_end(&mut json)?;
        Ok(serde_json::from_slice(&json)?)
    }
}
