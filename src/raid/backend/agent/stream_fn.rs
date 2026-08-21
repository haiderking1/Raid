use std::sync::{Mutex, OnceLock};

use super::types::StreamFn;

static DEFAULT_STREAM_FN: OnceLock<Mutex<Option<StreamFn>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<StreamFn>> {
    DEFAULT_STREAM_FN.get_or_init(|| Mutex::new(None))
}

pub fn set_default_stream_fn(stream_fn: Option<StreamFn>) {
    *slot().lock().expect("stream fn lock") = stream_fn;
}

pub fn get_default_stream_fn() -> StreamFn {
    slot()
        .lock()
        .expect("stream fn lock")
        .clone()
        .expect("No default stream function configured. Pass streamFn explicitly or call set_default_stream_fn().")
}
