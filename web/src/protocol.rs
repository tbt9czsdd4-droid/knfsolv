//! Messages between the page and the analysis worker (`src/bin/worker.rs`). Both sides are
//! Rust and exchange JSON strings, so 64-bit seeds and keys never pass through JavaScript
//! numbers.
use knfsolv::coaching::{AnalysisKey, AnalysisRequest, AnalysisUpdate};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Changes whenever the messages do; a page never talks to a worker of another version.
pub const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub enum ToWorker {
    Submit(Box<AnalysisRequest>),
    Controls {
        key: AnalysisKey,
        continuous: bool,
        paused: bool,
    },
    Clear,
    /// Timing checks and solver games for `?speed`, answered with `Line`s and `SpeedDone`.
    SpeedTest {
        games: u64,
    },
    /// Debug: panics in the worker to exercise the restart.
    Crash,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum FromWorker {
    Ready {
        version: u32,
    },
    Update(Box<AnalysisUpdate>),
    /// Sent by the panic hook just before the worker becomes unusable.
    Panicked(String),
    Line(String),
    SpeedDone,
}

pub fn encode(message: &impl Serialize) -> String {
    serde_json::to_string(message).expect("messages serialize")
}
pub fn decode<T: DeserializeOwned>(json: &str) -> serde_json::Result<T> {
    serde_json::from_str(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use knfsolv::{
        coaching::{AnalysisKey, AnalysisRequest, AnalysisUpdate},
        game::{JobPurpose, JobToken},
        solver::{self, Cancel, Settings},
        state::Position,
    };

    fn key() -> AnalysisKey {
        AnalysisKey {
            token: JobToken {
                revision: u64::MAX,
                actor: 0,
                purpose: JobPurpose::Coaching,
            },
            serial: u64::MAX - 1,
        }
    }

    #[test]
    fn ready_carries_the_protocol_version() {
        let json = encode(&FromWorker::Ready { version: VERSION });
        assert!(matches!(
            decode(&json).unwrap(),
            FromWorker::Ready { version: VERSION }
        ));
    }

    #[test]
    fn requests_keep_64_bit_keys_and_seeds() {
        let settings = Settings {
            seed: u64::MAX,
            ..Settings::default()
        };
        let request = AnalysisRequest {
            key: key(),
            position: Position::new(2),
            settings: settings.clone(),
            continuous: true,
            paused: false,
            baseline_only: false,
        };
        let json = encode(&ToWorker::Submit(Box::new(request)));
        let ToWorker::Submit(decoded) = decode(&json).unwrap() else {
            panic!("not a submit: {json}");
        };
        assert_eq!(decoded.key, key());
        assert_eq!(decoded.settings, settings);
        assert_eq!(decoded.position, Position::new(2));
        assert_eq!(encode(&ToWorker::Submit(decoded)), json);
    }

    #[test]
    fn updates_round_trip_reports_exactly() {
        let mut settings = Settings {
            simulations: 40,
            time_ms: 0,
            ..Settings::default()
        };
        settings.seed = u64::MAX;
        let report = solver::analyze(&Position::new(1), &settings, Cancel::default()).unwrap();
        let update = AnalysisUpdate {
            key: key(),
            report: Ok(report),
            finished: true,
            paused: false,
        };
        let json = encode(&FromWorker::Update(Box::new(update)));
        let FromWorker::Update(decoded) = decode(&json).unwrap() else {
            panic!("not an update: {json}");
        };
        assert_eq!(decoded.key, key());
        assert_eq!(decoded.report.as_ref().unwrap().settings.seed, u64::MAX);
        // Floats survive unchanged (serde_json's float_roundtrip), so re-encoding is identical.
        assert_eq!(encode(&FromWorker::Update(decoded)), json);
        let failed = encode(&FromWorker::Update(Box::new(AnalysisUpdate {
            key: key(),
            report: Err("Computation worker failed during search".into()),
            finished: true,
            paused: false,
        })));
        let FromWorker::Update(decoded) = decode(&failed).unwrap() else {
            panic!("not an update: {failed}");
        };
        assert!(decoded.report.is_err());
    }
}
