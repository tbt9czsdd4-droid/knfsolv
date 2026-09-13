//! Opt-in wall-clock and Linux resource checks; run alone in release mode.
#![cfg(target_os = "linux")]

use knfsolv::{
    coaching::{AnalysisKey, AnalysisRequest, ComputationWorker},
    dice::Dice,
    game::{JobPurpose, JobToken},
    policy::Policy,
    solver::{Method, Settings},
    state::{Phase, Position},
};
use std::time::{Duration, Instant};

fn threads() -> usize {
    std::fs::read_dir("/proc/self/task").unwrap().count()
}

fn rss_kib() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
#[ignore = "two-minute Linux worker soak; run separately in release mode"]
fn continuous_worker_soak() {
    let original_threads = threads();
    let worker = ComputationWorker::new(|| {});
    let mut position = Position::new(4);
    position.phase = Phase::Decision {
        dice: Dice::from_faces(&[1, 2, 3, 4, 6]).unwrap(),
        rerolls_left: 2,
    };
    for (serial, method) in [(1, Method::Rollout), (2, Method::Mcts)] {
        let key = AnalysisKey {
            token: JobToken {
                revision: serial,
                actor: 0,
                purpose: JobPurpose::Coaching,
            },
            serial,
        };
        worker
            .submit(AnalysisRequest {
                key,
                position: position.clone(),
                settings: Settings {
                    method,
                    policy: Policy::Greedy,
                    max_nodes: 128,
                    simulations: 20_000,
                    ..Settings::default()
                },
                continuous: true,
                paused: false,
                baseline_only: false,
            })
            .unwrap();
        let start = Instant::now();
        let mut samples = 0;
        let mut warm_rss = None;
        let mut peak_rss = 0;
        let mut reports = 0;
        while start.elapsed() < Duration::from_secs(60) {
            // Deliberately consume slower than the publication interval.
            std::thread::sleep(Duration::from_millis(750));
            assert_eq!(threads(), original_threads + 1);
            assert!(worker.buffered_reports() <= 2);
            for update in worker.take_updates() {
                assert_eq!(update.key, key);
                let report = update.report.unwrap();
                assert!(!update.finished);
                assert!(report.samples >= samples);
                assert!(report.nodes <= 128);
                samples = report.samples;
                reports += 1;
            }
            if start.elapsed() >= Duration::from_secs(10) {
                let rss = rss_kib();
                let baseline = *warm_rss.get_or_insert(rss);
                peak_rss = peak_rss.max(rss);
                assert!(
                    rss <= baseline + 32 * 1024,
                    "RSS grew over 32 MiB after warmup"
                );
            }
        }
        assert!(samples > 100);
        assert!(reports > 10);
        println!(
            "{method:?}: {samples} samples, {reports} reports, warm RSS {} KiB, peak {peak_rss} KiB, one worker thread",
            warm_rss.unwrap()
        );
        worker.clear();
    }
    let start = Instant::now();
    drop(worker);
    // pthread join synchronizes Rust execution; /proc can briefly retain the
    // terminating kernel task while its final teardown completes.
    while threads() != original_threads && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(threads(), original_threads);
}
