use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use super::env::ToolEnvironment;

static REGISTRATION: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn registration_lock() -> &'static tokio::sync::Mutex<()> {
    REGISTRATION.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub async fn with_file_mutation_queue<T, F, Fut>(
    env: &ToolEnvironment,
    path: &Path,
    operation: F,
) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let registration = registration_lock();
    let _registration_guard = registration.lock().await;
    let key = env.canonical_path(path);
    let queue = mutation_queue_for_key(key);
    drop(_registration_guard);

    let guard = queue.lock().await;
    let result = operation().await;
    drop(guard);
    result
}

fn mutation_queue_for_key(key: PathBuf) -> Arc<tokio::sync::Mutex<()>> {
    fn queues() -> &'static Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>> {
        static QUEUES: OnceLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
            OnceLock::new();
        QUEUES.get_or_init(|| Mutex::new(HashMap::new()))
    }
    let mut queues = queues().lock().expect("mutation queue map");
    queues
        .entry(key)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}
