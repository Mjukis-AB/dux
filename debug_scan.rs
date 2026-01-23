// Run with: cargo run --example debug_scan -- /path/to/scan
// Add to dux-core/Cargo.toml: [[example]] name = "debug_scan" path = "../debug_scan.rs"

use dux_core::{ScanConfig, ScanMessage, Scanner};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    println!("Scanning: {:?}", path);

    let scanner = Scanner::new(ScanConfig::default());
    let (rx, handle) = scanner.scan(path);

    let start = Instant::now();
    let mut last_print = Instant::now();
    let mut last_files = 0u64;
    let mut last_path: Option<PathBuf> = None;
    let mut stuck_count = 0;

    for msg in rx {
        match msg {
            ScanMessage::Progress(p) => {
                let now = Instant::now();
                let elapsed = now.duration_since(start);

                // Check if we're stuck (same file count for multiple updates)
                if p.files_scanned == last_files {
                    stuck_count += 1;
                } else {
                    stuck_count = 0;
                }
                last_files = p.files_scanned;

                // Print every second or if stuck
                if now.duration_since(last_print) > Duration::from_secs(1) || stuck_count > 5 {
                    let path_changed = last_path.as_ref() != p.current_path.as_ref();
                    println!(
                        "[{:>6.1}s] files={:<8} dirs={:<8} bytes={:<12} errors={:<4} stuck={} path_changed={} path={:?}",
                        elapsed.as_secs_f64(),
                        p.files_scanned,
                        p.dirs_scanned,
                        p.bytes_scanned,
                        p.errors,
                        stuck_count,
                        path_changed,
                        p.current_path
                            .as_ref()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default()
                    );
                    last_print = now;
                    last_path = p.current_path.clone();
                }
            }
            ScanMessage::Finalizing => {
                println!("[{:>6.1}s] FINALIZING...", start.elapsed().as_secs_f64());
            }
            ScanMessage::Completed => {
                println!("[{:>6.1}s] COMPLETED", start.elapsed().as_secs_f64());
            }
            ScanMessage::Cancelled => {
                println!("[{:>6.1}s] CANCELLED", start.elapsed().as_secs_f64());
            }
            ScanMessage::Error(e) => {
                println!("[{:>6.1}s] ERROR: {}", start.elapsed().as_secs_f64(), e);
            }
            _ => {}
        }
    }

    let tree = handle.join().unwrap();
    println!(
        "\nFinal: {} nodes, {} total size",
        tree.len(),
        dux_core::format_size(tree.total_size())
    );
}
