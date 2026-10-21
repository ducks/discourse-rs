//! A JavaScript plugin (plugin.js) run by Boa compiled into the guest
//! (docs/plugin-wasm.md, "JavaScript in the guest"): the same ABI as a
//! Rust guest, with `rows` and `t` as global functions of the script.
//!
//! `wizer.initialize` builds the realm and evaluates the script, so a
//! module pre-initialized with wizer starts with both done; without it,
//! the first phase call does them.

use std::cell::RefCell;

use boa_engine::context::time::FixedClock;
use boa_engine::{
    Context, JsError, JsNativeError, JsResult, JsValue, NativeFunction, Source, js_string,
};
use discourse_plugin::Input;
use serde_json::Value;

discourse_plugin::phases! {
    "topic_list.after_serialize" => after_serialize,
}

const SCRIPT: &str = include_str!("../plugin.js");

thread_local! {
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

fn ids(value: &JsValue, context: &mut Context) -> JsResult<Vec<i64>> {
    let json = value.to_json(context)?.unwrap_or(Value::Null);
    Ok(json
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default())
}

fn string(value: &JsValue, context: &mut Context) -> JsResult<String> {
    Ok(value.to_string(context)?.to_std_string_escaped())
}

/// `rows(table, column, ids)`
fn rows(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    let table = string(&arg(0), context)?;
    let column = string(&arg(1), context)?;
    let ids = ids(&arg(2), context)?;
    let rows = discourse_plugin::rows(&table, &column, &ids);
    JsValue::from_json(&Value::Array(rows), context)
}

/// `t(key)`
fn t(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let key = string(&args.first().cloned().unwrap_or_default(), context)?;
    Ok(JsValue::from(js_string!(discourse_plugin::t(&key))))
}

fn new_context() -> JsResult<Context> {
    // No clock in the guest: Date.now() is the epoch, and StdClock's
    // Instant::now() would trap on wasm32-unknown-unknown.
    let mut context = Context::builder()
        .clock(std::rc::Rc::new(FixedClock::from_millis(0)))
        .build()?;
    context.register_global_callable(js_string!("rows"), 3, NativeFunction::from_fn_ptr(rows))?;
    context.register_global_callable(js_string!("t"), 1, NativeFunction::from_fn_ptr(t))?;
    context.eval(Source::from_bytes(SCRIPT))?;
    Ok(context)
}

/// wizer's pre-initialization hook.
#[unsafe(export_name = "wizer.initialize")]
pub extern "C" fn initialize() {
    CONTEXT.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = new_context().ok();
        }
    });
}

fn call(path: &str, payload: &Value) -> Result<Value, String> {
    initialize();
    CONTEXT.with(|c| {
        let mut c = c.borrow_mut();
        let context = c.as_mut().ok_or("plugin.js did not load")?;
        let fail = |e: JsError| e.to_string();
        let function = context.eval(Source::from_bytes(path)).map_err(fail)?;
        let function = function
            .as_callable()
            .ok_or_else(|| format!("{path} is not a function"))?;
        let arg = JsValue::from_json(payload, context).map_err(fail)?;
        let out = function
            .call(&JsValue::undefined(), &[arg], context)
            .map_err(fail)?;
        out.to_json(context)
            .map_err(fail)?
            .ok_or_else(|| JsError::from(JsNativeError::typ()).to_string())
    })
}

fn after_serialize(input: Input) -> Result<Value, String> {
    call("plugin.topic_list.after_serialize", &input.payload)
}
