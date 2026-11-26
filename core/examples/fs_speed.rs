//! Measures the per-file cost of creating, flushing and renaming many small files,
//! the receiver's work for each small file.
//!
//!   cargo run --release --example fs_speed -- <empty folder> [files] [kb] [threads]

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().unwrap_or_else(|| ".bench/fs".into()));
    let files: usize = args.next().map_or(Ok(2000), |a| a.parse())?;
    let kb: usize = args.next().map_or(Ok(250), |a| a.parse())?;
    let threads: usize = args.next().map_or(Ok(8), |a| a.parse())?;
    let data = vec![7u8; kb * 1000];

    for (label, sync, rename) in [
        ("write only", false, false),
        ("write + rename", false, true),
        ("write + flush to disk", true, false),
        ("write + flush + rename", true, true),
    ] {
        let run = dir.join(label.replace([' ', '+'], ""));
        fs::create_dir_all(&run)?;
        let started = Instant::now();
        std::thread::scope(|scope| {
            for t in 0..threads {
                let (run, data) = (&run, &data);
                scope.spawn(move || {
                    for i in (t..files).step_by(threads) {
                        write_one(run, i, data, sync, rename).unwrap();
                    }
                });
            }
        });
        let secs = started.elapsed().as_secs_f64();
        println!(
            "{label:<24} {files} files in {secs:5.2} s = {:6.2} ms per file, {:6.1} MB/s",
            secs * 1000.0 * threads as f64 / files as f64,
            (files * data.len()) as f64 / secs / 1e6
        );
        fs::remove_dir_all(&run)?;
    }
    Ok(())
}

fn write_one(dir: &Path, i: usize, data: &[u8], sync: bool, rename: bool) -> std::io::Result<()> {
    let part = dir.join(format!("f{i}.part"));
    let mut file = File::create(&part)?;
    file.write_all(data)?;
    if sync {
        file.sync_all()?;
    }
    drop(file);
    if rename {
        fs::rename(&part, dir.join(format!("f{i}.bin")))?;
    }
    Ok(())
}
