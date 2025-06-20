// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use notify::event::ModifyKind;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum FileWatcherError {
    #[error("watch error: {0}")]
    WatchError(String),
}

#[derive(Debug)]
pub struct FileWatcher {
    watcher: RecommendedWatcher,
    cancellation_token: CancellationToken,
}

impl Drop for FileWatcher {
    fn drop(&mut self) {
        self.stop_watcher();
    }
}

impl FileWatcher {
    pub fn create_watcher<F>(callback: F) -> Self
    where
        F: Fn(&str) + Send + 'static,
    {
        let (tx, mut rx) = mpsc::channel::<notify::Result<Event>>(10);
        let watcher = notify::recommended_watcher(move |res| {
            // Send file system events to the event channel
            let _ = tx.blocking_send(res);
        })
        .expect("error creating the watcher");
        let fw = FileWatcher {
            watcher,
            cancellation_token: CancellationToken::new(),
        };

        let c_token = fw.cancellation_token.clone();
        tokio::spawn(async move {
            debug!("starting new watcher");
            loop {
                tokio::select! {
                    next = rx.recv() => {
                        match next {
                            Some(res) => {
                                match res {
                                    Ok(event) => {
                                        if let notify::EventKind::Modify(ModifyKind::Data(_)) = event.kind {
                                            if event.paths.is_empty() {
                                                // skip this event, we don't know the associated file
                                                continue;
                                            }
                                            if let Some(p) = event.paths.first().and_then(|p| p.to_str()) {
                                                debug!("detected event {:?}", event);
                                                callback(p);
                                            }
                                        }
                                    }
                                    Err(e) => println!("watch error: {:?}", e),
                                }
                            }
                            None => {
                                debug!("channel closed, stop watcher");
                                break;
                            }
                        }
                    }
                    _ = c_token.cancelled() => {
                        debug!("cancellation token signaled, stop watcher");
                        break;
                    }
                }
            }
        });

        // return the FileWatcher
        fw
    }

    pub fn add_file(&mut self, file_name: &str) -> Result<(), FileWatcherError> {
        match self
            .watcher
            .watch(Path::new(file_name), RecursiveMode::NonRecursive)
        {
            Ok(_) => {
                debug!("start watching file {}", file_name);
                Ok(())
            }
            Err(e) => Err(FileWatcherError::WatchError(e.to_string())),
        }
    }

    pub fn stop_watcher(&self) {
        self.cancellation_token.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time;
    use tracing::info;
    use tracing_test::traced_test;

    use parking_lot::RwLock;
    use std::collections::HashMap;
    use std::fs::{File, OpenOptions};
    use std::io::{Seek, SeekFrom, Write};
    use std::sync::Arc;
    use std::time::Duration;
    use std::{env, fs};

    fn create_file(file_path: &str, content: &str) -> std::io::Result<()> {
        let mut file = File::create(file_path)?;
        file.write_all(content.as_bytes())?;
        Ok(())
    }

    fn modify_file(file_path: &str, new_content: &str) -> std::io::Result<()> {
        let mut file = OpenOptions::new().write(true).open(file_path)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(new_content.as_bytes())?;
        Ok(())
    }

    fn delete_file(file_path: &str) -> std::io::Result<()> {
        fs::remove_file(file_path)?;
        Ok(())
    }

    #[tokio::test]
    #[traced_test]
    async fn test_watcher() {
        let counter_map = Arc::new(RwLock::new(HashMap::<String, u32>::new()));
        let clone_map = Arc::clone(&counter_map);

        // create the watcher
        let mut w = FileWatcher::create_watcher(move |file: &str| {
            info!("modification detected on file {}", file);
            let mut map = clone_map.write();
            match map.get_mut(file) {
                Some(val) => {
                    let x = *val + 1;
                    map.insert(String::from(file), x);
                }
                None => {
                    map.insert(String::from(file), 1);
                }
            }
        });

        // create a new file
        let path = env::current_dir().expect("error reading local path");
        let full_path = path.join("test_file_watcher.txt");
        let full_test_file_name = full_path.to_str().unwrap();
        create_file(full_test_file_name, "CONFIG 1").expect("Failed to create file");

        // add file to watcher
        let res = w.add_file(full_test_file_name);
        assert_eq!(res, Ok(()));

        // modify the file
        modify_file(full_test_file_name, "CONFIG 2").expect("Failed to modify file");
        time::sleep(Duration::from_millis(100)).await;
        {
            let map = counter_map.read();
            let res = map.get(full_test_file_name).expect("file does not exists");
            assert_eq!(*res, 1);
        }

        modify_file(full_test_file_name, "CONFIG 3").expect("Failed to modify file");
        time::sleep(Duration::from_millis(100)).await;
        {
            let map = counter_map.read();
            let res = map.get(full_test_file_name).expect("file does not exists");
            assert_eq!(*res, 2);
        }

        // add other file to watch
        let path = env::current_dir().expect("error reading local path");
        let full_path = path.join("test_file_watcher_2.txt");
        let full_test_file_name_2 = full_path.to_str().unwrap();
        create_file(full_test_file_name_2, "CONFIG 1").expect("Failed to create file");

        // add file to watcher
        let res = w.add_file(full_test_file_name_2);
        assert_eq!(res, Ok(()));

        // modify the file
        modify_file(full_test_file_name_2, "CONFIG 2").expect("Failed to modify file");
        time::sleep(Duration::from_millis(100)).await;
        {
            let map = counter_map.read();
            let res = map.get(full_test_file_name).expect("file does not exists");
            assert_eq!(*res, 2);
            let res = map
                .get(full_test_file_name_2)
                .expect("file does not exists");
            assert_eq!(*res, 1);
        }

        modify_file(full_test_file_name_2, "CONFIG 3").expect("Failed to modify file");
        time::sleep(Duration::from_millis(100)).await;
        {
            let map = counter_map.read();
            let res = map.get(full_test_file_name).expect("file does not exists");
            assert_eq!(*res, 2);
            let res = map
                .get(full_test_file_name_2)
                .expect("file does not exists");
            assert_eq!(*res, 2);
        }

        modify_file(full_test_file_name, "CONFIG 4").expect("Failed to modify file");
        time::sleep(Duration::from_millis(100)).await;
        {
            let map = counter_map.read();
            let res = map.get(full_test_file_name).expect("file does not exists");
            assert_eq!(*res, 3);
            let res = map
                .get(full_test_file_name_2)
                .expect("file does not exists");
            assert_eq!(*res, 2);
        }

        w.stop_watcher();

        delete_file(full_test_file_name).expect("error deleting file");
        delete_file(full_test_file_name_2).expect("error deleting file");

        time::sleep(Duration::from_millis(100)).await;
    }
}
