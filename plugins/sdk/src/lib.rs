//! The guest side of discourse-rs' WebAssembly plugin ABI
//! (docs/plugin-wasm.md): JSON over linear memory.
//!
//! A plugin is a `cdylib` built for `wasm32-unknown-unknown` that names
//! its phases with [`phases!`]:
//!
//! ```ignore
//! fn after_serialize(input: discourse_plugin::Input) -> Result<serde_json::Value, String> { ... }
//! discourse_plugin::phases! { "topic_list.after_serialize" => after_serialize }
//! ```
//!
//! and reaches the host through the functions here, each a synchronous
//! call into the host that the host answers before the guest continues.

use serde::Deserialize;
use serde_json::{Value, json};

/// A phase's input: the payload its pipeline defines, and who is asking.
#[derive(Debug, Deserialize)]
pub struct Input {
    pub payload: Value,
    pub guardian: Guardian,
}

#[derive(Debug, Deserialize)]
pub struct Guardian {
    pub user_id: Option<i64>,
    pub staff: bool,
    pub admin: bool,
}

#[link(wasm_import_module = "discourse")]
unsafe extern "C" {
    #[link_name = "rows"]
    fn host_rows(ptr: *const u8, len: usize) -> u64;
    #[link_name = "core_rows"]
    fn host_core_rows(ptr: *const u8, len: usize) -> u64;
    #[link_name = "setting"]
    fn host_setting(ptr: *const u8, len: usize) -> u64;
    #[link_name = "t"]
    fn host_t(ptr: *const u8, len: usize) -> u64;
}

/// Memory for `len` bytes the host writes into: a phase's input or a host
/// call's answer, handed back with [`take`].
pub fn alloc(len: u32) -> u32 {
    let buf = vec![0u8; len as usize].into_boxed_slice();
    Box::into_raw(buf) as *mut u8 as u32
}

/// The bytes at a packed `ptr << 32 | len` that [`alloc`] made.
fn take(packed: u64) -> Vec<u8> {
    let (ptr, len) = ((packed >> 32) as usize, (packed & 0xffff_ffff) as usize);
    // SAFETY: the host wrote `len` bytes into memory `alloc(len)` returned.
    unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr as *mut u8, len)) }.into_vec()
}

/// Leaks `bytes` and packs where they are, for the host to read. The
/// instance is dropped after the call, its memory with it.
fn give(bytes: Vec<u8>) -> u64 {
    let len = bytes.len() as u64;
    let ptr = Box::into_raw(bytes.into_boxed_slice()) as *mut u8 as usize as u64;
    (ptr << 32) | len
}

/// A phase export's body: decode, run, encode `{"ok"}` or `{"error"}`.
pub fn run(ptr: u32, len: u32, phase: fn(Input) -> Result<Value, String>) -> u64 {
    let bytes = take((u64::from(ptr) << 32) | u64::from(len));
    let result = serde_json::from_slice::<Input>(&bytes)
        .map_err(|e| format!("input: {e}"))
        .and_then(phase);
    let out = match result {
        Ok(ok) => json!({ "ok": ok }),
        Err(error) => json!({ "error": error }),
    };
    give(serde_json::to_vec(&out).unwrap_or_default())
}

fn call(f: unsafe extern "C" fn(*const u8, usize) -> u64, request: &[u8]) -> Vec<u8> {
    // SAFETY: the host reads `request` and answers with memory from alloc.
    take(unsafe { f(request.as_ptr(), request.len()) })
}

fn rows_from(
    f: unsafe extern "C" fn(*const u8, usize) -> u64,
    table: &str,
    column: &str,
    ids: &[i64],
) -> Vec<Value> {
    if ids.is_empty() {
        return Vec::new();
    }
    let request = json!({ "table": table, "column": column, "ids": ids });
    let answer = call(f, request.to_string().as_bytes());
    serde_json::from_slice(&answer).unwrap_or_default()
}

/// The rows of a table this plugin owns whose `column` is one of `ids`,
/// with the columns its manifest lists, ordered by `column` then `id`.
pub fn rows(table: &str, column: &str, ids: &[i64]) -> Vec<Value> {
    rows_from(host_rows, table, column, ids)
}

/// The same for a core table, limited to the columns the manifest's
/// `[core_reads]` lists.
pub fn core_rows(table: &str, column: &str, ids: &[i64]) -> Vec<Value> {
    rows_from(host_core_rows, table, column, ids)
}

/// A site setting the manifest lists.
pub fn setting(name: &str) -> Value {
    serde_json::from_slice(&call(host_setting, name.as_bytes())).unwrap_or(Value::Null)
}

/// A server translation (the key itself when there is none).
pub fn t(key: &str) -> String {
    String::from_utf8(call(host_t, key.as_bytes())).unwrap_or_else(|_| key.to_string())
}

/// The plugin's exports: `dp_alloc` and one function per phase, named
/// `<pipeline>.<phase>`.
#[macro_export]
macro_rules! phases {
    ($($name:literal => $f:path),* $(,)?) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn dp_alloc(len: u32) -> u32 {
            $crate::alloc(len)
        }
        $(
            const _: () = {
                #[unsafe(export_name = $name)]
                pub extern "C" fn phase(ptr: u32, len: u32) -> u64 {
                    $crate::run(ptr, len, $f)
                }
            };
        )*
    };
}
