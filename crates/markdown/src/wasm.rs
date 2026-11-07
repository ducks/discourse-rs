//! The composer preview's entry points, for wasm32-unknown-unknown with no
//! bindings generator: strings cross as UTF-8 in this module's memory. The
//! page allocates a buffer (`alloc`), writes into it, and calls `configure`
//! with the site's RenderSettings as JSON once, then `preview` with the
//! post's markdown, or `rich_editor_tree` for the rich editor's node
//! tree. Each returns 0 on success, 1 on failure, and leaves its text (the
//! cooked HTML, the tree's JSON, or the error) at
//! `output_ptr`/`output_len`.

use std::cell::RefCell;

use crate::render::context::Lookups;
use crate::render::{RenderSettings, render};

thread_local! {
    static SETTINGS: RefCell<Option<RenderSettings>> = const { RefCell::new(None) };
    static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// A buffer of `len` bytes for the page to write into.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// Frees a buffer from `alloc`.
///
/// # Safety
/// `ptr` and `len` are what `alloc` returned and was asked for.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    drop(unsafe { Vec::from_raw_parts(ptr, 0, len) });
}

/// The bytes the page wrote.
///
/// # Safety
/// `ptr` and `len` are a buffer from `alloc` holding `len` written bytes.
unsafe fn input(ptr: *const u8, len: usize) -> String {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    String::from_utf8_lossy(bytes).into_owned()
}

fn output(text: String, ok: bool) -> i32 {
    OUTPUT.with(|o| *o.borrow_mut() = text.into_bytes());
    if ok { 0 } else { 1 }
}

#[unsafe(no_mangle)]
pub extern "C" fn output_ptr() -> *const u8 {
    OUTPUT.with(|o| o.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn output_len() -> usize {
    OUTPUT.with(|o| o.borrow().len())
}

/// Takes the site's RenderSettings as JSON.
///
/// # Safety
/// `ptr` and `len` are a buffer from `alloc` holding `len` written bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn configure(ptr: *const u8, len: usize) -> i32 {
    let json = unsafe { input(ptr, len) };
    let settings = match serde_json::from_str::<RenderSettings>(&json) {
        Ok(settings) => settings,
        Err(e) => return output(format!("settings: {e}"), false),
    };
    match settings.compiled() {
        Ok(settings) => {
            SETTINGS.with(|s| *s.borrow_mut() = Some(settings));
            output(String::new(), true)
        }
        Err(e) => output(e.to_string(), false),
    }
}

/// Renders the markdown as a post cooks, without what the server looks up
/// (avatars in quotes, hashtags, upload urls), as Ember's preview does
/// before its decorators run.
///
/// # Safety
/// `ptr` and `len` are a buffer from `alloc` holding `len` written bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn preview(ptr: *const u8, len: usize) -> i32 {
    let raw = unsafe { input(ptr, len) };
    SETTINGS.with(|s| match s.borrow().as_ref() {
        None => output("not configured".into(), false),
        Some(settings) => match render(&raw, settings, Lookups::default()) {
            Ok((html, _)) => output(html, true),
            Err(e) => output(e.to_string(), false),
        },
    })
}

/// The rich editor's node tree for the markdown, as JSON
/// (docs/rich-editor.md), parsed with the configured settings less the
/// typographer, onebox, watched words and censored.
///
/// # Safety
/// `ptr` and `len` are a buffer from `alloc` holding `len` written bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rich_editor_tree(ptr: *const u8, len: usize) -> i32 {
    let raw = unsafe { input(ptr, len) };
    SETTINGS.with(|s| match s.borrow().as_ref() {
        None => output("not configured".into(), false),
        Some(settings) => match crate::render::rich_editor_tree(&raw, settings) {
            Ok(tree) => output(tree.to_string(), true),
            Err(e) => output(e.to_string(), false),
        },
    })
}

/// The configured settings' answer to the rich editor's engine question
/// (render::editor): `f` takes the request's text.
fn engine_call(
    request: &str,
    f: impl Fn(&str, &RenderSettings) -> Result<serde_json::Value, String>,
) -> i32 {
    SETTINGS.with(|s| match s.borrow().as_ref() {
        None => output("not configured".into(), false),
        Some(settings) => match f(request, settings) {
            Ok(value) => output(value.to_string(), true),
            Err(e) => output(e, false),
        },
    })
}

/// `getLinkify().match(text)`, as JSON.
///
/// # Safety
/// `ptr` and `len` are a buffer from `alloc` holding `len` written bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn linkify_match(ptr: *const u8, len: usize) -> i32 {
    engine_call(&unsafe { input(ptr, len) }, |text, settings| {
        crate::render::editor::link_matches(text, settings).map_err(|e| e.to_string())
    })
}

/// `getLinkify().matchAtStart(text)`, as JSON.
///
/// # Safety
/// `ptr` and `len` are a buffer from `alloc` holding `len` written bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn linkify_match_at_start(ptr: *const u8, len: usize) -> i32 {
    engine_call(&unsafe { input(ptr, len) }, |text, settings| {
        crate::render::editor::link_at_start(text, settings).map_err(|e| e.to_string())
    })
}

/// The allow list's tags, as a JSON array (the request is ignored).
///
/// # Safety
/// `ptr` and `len` are a buffer from `alloc` holding `len` written bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn html_allow_list(ptr: *const u8, len: usize) -> i32 {
    engine_call(&unsafe { input(ptr, len) }, |_, settings| {
        Ok(crate::render::editor::html_tags(settings))
    })
}

/// Whether the allow list keeps an attribute: the request is the JSON
/// `[tag, name, value]`, the answer true, false or null (no such tag).
///
/// # Safety
/// `ptr` and `len` are a buffer from `alloc` holding `len` written bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn html_attr_allowed(ptr: *const u8, len: usize) -> i32 {
    engine_call(&unsafe { input(ptr, len) }, |request, settings| {
        let (tag, name, value): (String, String, String) =
            serde_json::from_str(request).map_err(|e| format!("html_attr_allowed: {e}"))?;
        Ok(crate::render::editor::html_attr_allowed(
            &tag, &name, &value, settings,
        ))
    })
}

/// An emoji code's image as JSON `{url, custom}`, or null.
///
/// # Safety
/// `ptr` and `len` are a buffer from `alloc` holding `len` written bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn emoji_image(ptr: *const u8, len: usize) -> i32 {
    engine_call(&unsafe { input(ptr, len) }, |code, settings| {
        Ok(crate::render::editor::emoji(code, settings))
    })
}

/// The emoticons and their emoji names, as a JSON object (the request is
/// ignored).
///
/// # Safety
/// `ptr` and `len` are a buffer from `alloc` holding `len` written bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn emoji_translations(ptr: *const u8, len: usize) -> i32 {
    engine_call(&unsafe { input(ptr, len) }, |_, _| {
        Ok(crate::render::editor::emoji_translations())
    })
}

/// lib/text's `emojiUnescape(text)`, as a JSON string.
///
/// # Safety
/// `ptr` and `len` are a buffer from `alloc` holding `len` written bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn emoji_unescape(ptr: *const u8, len: usize) -> i32 {
    engine_call(&unsafe { input(ptr, len) }, |text, settings| {
        Ok(crate::render::editor::emoji_unescape(text, settings))
    })
}
