use clap::{Parser, Subcommand};
use knfsolv::{
    Result,
    dice::Dice,
    persistence::{Session, save_json},
    policy::Policy,
    solver::{self, Cancel, Method, Objective, solitaire::Cache},
    state::Phase,
};
use std::{path::PathBuf, time::Instant};

#[derive(Parser)]
#[command(
    version,
    about = "Opponent-aware Kniffel solver. No subcommand opens the TUI."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    /// Fit a versioned continuation model by reproducible offline policy evaluation.
    TrainBaseline {
        #[arg(long, default_value_t = 5000)]
        games: u64,
        #[arg(long, default_value_t = 42)]
        seed: u64,
        #[arg(long, default_value_t = 2)]
        iterations: usize,
        #[arg(long, default_value = "continuation-model.json")]
        output: PathBuf,
    },
    /// Open the graphical score sheet (egui desktop application).
    #[cfg(feature = "gui")]
    Gui {
        #[arg(long)]
        load: Option<PathBuf>,
        #[arg(long, default_value_t = 2)]
        players: usize,
    },
    /// Open the interactive score sheet.
    Tui {
        #[arg(long)]
        load: Option<PathBuf>,
        #[arg(long, default_value_t = 2)]
        players: usize,
    },
    /// Analyze a saved session. Player indices are zero-based in JSON/CLI.
    Analyze {
        position: PathBuf,
        #[arg(long)]
        time_ms: Option<u64>,
        #[arg(long)]
        simulations: Option<u64>,
        #[arg(long)]
        seed: Option<u64>,
        #[arg(long, value_enum)]
        method: Option<Method>,
        #[arg(long, value_enum)]
        objective: Option<Objective>,
        #[arg(long, value_enum)]
        policy: Option<Policy>,
        #[arg(long)]
        perspective: Option<usize>,
        #[arg(long)]
        max_nodes: Option<usize>,
        /// Maximum MCTS decisions for the analyzed player; 0 = unlimited. Exact search is unaffected.
        #[arg(long)]
        max_depth: Option<usize>,
        #[arg(long)]
        cache: Option<PathBuf>,
        #[arg(long)]
        baseline_model: Option<PathBuf>,
        #[arg(long, conflicts_with = "sampled_advice")]
        conservative_advice: bool,
        /// Opt into low-confidence paired sampled-leader advice instead of conservative fallback.
        #[arg(long)]
        sampled_advice: bool,
        #[arg(long)]
        json: bool,
    },
    /// Validate local scores, dice, and turn-order consistency.
    Validate { position: PathBuf },
    /// Create a new session or a reproducible example: opening, middle, lead, trail.
    New {
        #[arg(long, default_value_t = 2)]
        players: usize,
        #[arg(long, default_value = "opening")]
        example: String,
        #[arg(long, default_value = "position.json")]
        output: PathBuf,
    },
    /// Build/resume an exact solitaire table; only complete entries are saved.
    Precompute {
        #[arg(long)]
        position: Option<PathBuf>,
        #[arg(long, default_value = "solitaire-cache.json")]
        output: PathBuf,
        #[arg(long, default_value_t = 10000)]
        time_ms: u64,
        #[arg(long, default_value_t = 100000)]
        max_states: usize,
    },
    /// Run a release-build benchmark; optional suite is a JSON list of session paths.
    Benchmark {
        #[arg(long)]
        suite: Option<PathBuf>,
        #[arg(long, default_value_t = 1000)]
        time_ms: u64,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}
fn example(n: usize, kind: &str) -> Result<Session> {
    if !(1..=8).contains(&n) {
        return Err(knfsolv::Error::Invalid("Need 1–8 players".into()));
    }
    let mut s = Session::new(n);
    match kind {
        "opening" => {
            s.position.phase = Phase::Decision {
                dice: Dice::from_faces(&[1, 2, 3, 4, 6])?,
                rerolls_left: 2,
            }
        }
        "middle" => {
            for (i, p) in s.position.players.iter_mut().enumerate() {
                for c in 0..6 {
                    p.sheet.cells[c] = Some((c as u16 + 1) * (2 + (i + c) % 3) as u16);
                }
            }
            s.position.phase = Phase::Decision {
                dice: Dice::from_faces(&[2, 3, 3, 3, 6])?,
                rerolls_left: 1,
            };
        }
        "lead" | "trail" => {
            if n != 2 {
                return Err(knfsolv::Error::Invalid(
                    "Lead/trail examples use two players".into(),
                ));
            }
            for p in &mut s.position.players {
                p.sheet.cells = [Some(0); 13];
            }
            s.position.players[0].sheet.cells[12] = None;
            s.position.players[1].sheet.cells[8] = Some(25);
            if kind == "trail" {
                s.position.players[1].sheet.cells[3] = Some(4);
            }
            s.position.turn_order = vec![1, 0];
            s.position.phase = Phase::Decision {
                dice: Dice::from_faces(&[6, 6, 6, 6, 2])?,
                rerolls_left: 1,
            };
        }
        _ => {
            return Err(knfsolv::Error::Invalid(
                "Use opening, middle, lead, or trail".into(),
            ));
        }
    }
    s.validate()?;
    Ok(s)
}
fn run() -> Result<()> {
    match Cli::parse().command {
        Some(Command::TrainBaseline {
            games,
            seed,
            iterations,
            output,
        }) => {
            knfsolv::policy::continuation::train(games, seed, iterations)?.save(&output)?;
            println!("Saved continuation model to {}", output.display());
            Ok(())
        }
        #[cfg(feature = "gui")]
        Some(Command::Gui { load, players }) => {
            if !(1..=8).contains(&players) {
                return Err(knfsolv::Error::Invalid("Need 1–8 players".into()));
            }
            let session = match &load {
                Some(path) => Session::load(path)?,
                None => Session::new(players),
            };
            knfsolv::gui::run(session, load)
        }
        None => knfsolv::tui::run(Session::new(2), None),
        Some(Command::Tui { load, players }) => {
            let session = if let Some(p) = &load {
                Session::load(p)?
            } else {
                if !(1..=8).contains(&players) {
                    return Err(knfsolv::Error::Invalid("Need 1–8 players".into()));
                }
                let s = Session::new(players);
                s.validate()?;
                s
            };
            knfsolv::tui::run(session, load)
        }
        Some(Command::New {
            players,
            example: kind,
            output,
        }) => {
            example(players, &kind)?.save(&output)?;
            println!("Saved {}", output.display());
            Ok(())
        }
        Some(Command::Validate { position }) => {
            let s = Session::load(&position)?;
            println!(
                "Valid {} position: {} players, {} open categories. Local scores checked; historical reachability is not proven.",
                if s.position.hypothetical {
                    "hypothetical"
                } else {
                    "match"
                },
                s.position.players.len(),
                s.position.open()
            );
            Ok(())
        }
        Some(Command::Analyze {
            position,
            time_ms,
            simulations,
            seed,
            method,
            objective,
            policy,
            perspective,
            max_nodes,
            max_depth,
            cache,
            baseline_model,
            conservative_advice,
            sampled_advice,
            json,
        }) => {
            let mut s = Session::load(&position)?;
            if let Some(path) = baseline_model {
                s.settings.baseline_model = Some(path);
            }
            if conservative_advice {
                s.settings.conservative_advice = true;
            }
            if sampled_advice {
                s.settings.conservative_advice = false;
            }
            if let Some(x) = simulations {
                s.settings.simulations = x;
                if time_ms.is_none() {
                    s.settings.time_ms = 0;
                }
            }
            if let Some(x) = time_ms {
                s.settings.time_ms = x;
            }
            if let Some(x) = seed {
                s.settings.seed = x;
            }
            if let Some(x) = method {
                s.settings.method = x;
            }
            if let Some(x) = objective {
                s.settings.objective = x;
            }
            if let Some(x) = policy {
                s.settings.policy = x;
                s.settings.opponent_policies.clear();
            }
            if let Some(x) = perspective {
                s.settings.perspective = x;
            }
            if let Some(x) = max_nodes {
                s.settings.max_nodes = x;
            }
            if let Some(x) = max_depth {
                s.settings.max_depth = x;
            }
            if let Some(x) = cache {
                s.settings.cache = Some(x);
            }
            let report = solver::analyze(&s.position, &s.settings, Cancel::default())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}", report.text());
            }
            Ok(())
        }
        Some(Command::Precompute {
            position,
            output,
            time_ms,
            max_states,
        }) => {
            let s = if let Some(path) = position {
                Session::load(&path)?
            } else {
                Session::new(1)
            };
            let mut cache = if output.exists() {
                Cache::load(&output)?
            } else {
                Cache::default()
            };
            let start = Instant::now();
            let value = cache.precompute(
                &s.position.players[s.settings.perspective].sheet,
                time_ms,
                max_states,
                Cancel::default(),
            )?;
            cache.save(&output)?;
            println!(
                "Saved {} exact turn-boundary values in {} ms to {}. Root expected additional score: {}. Interrupted runs can be resumed.",
                cache.len(),
                start.elapsed().as_millis(),
                output.display(),
                value
                    .map(|v| format!("{v:.6}"))
                    .unwrap_or_else(|| "not solved within budget".into())
            );
            Ok(())
        }
        Some(Command::Benchmark {
            suite,
            time_ms,
            output,
        }) => {
            let sessions = if let Some(path) = suite {
                let paths: Vec<PathBuf> = serde_json::from_reader(std::fs::File::open(path)?)?;
                paths
                    .into_iter()
                    .map(|p| Session::load(&p).map(|s| (p.display().to_string(), s)))
                    .collect::<Result<Vec<_>>>()?
            } else {
                let mut list = vec![];
                for n in [2, 4, 8] {
                    for kind in ["opening", "middle"] {
                        list.push((format!("{kind}-{n}"), example(n, kind)?));
                    }
                }
                for kind in ["lead", "trail"] {
                    list.push((kind.into(), example(2, kind)?));
                }
                list
            };
            let mut results = vec![];
            for (name, mut s) in sessions {
                s.settings.time_ms = time_ms;
                let report = solver::analyze(&s.position, &s.settings, Cancel::default())?;
                eprintln!(
                    "{name}: {} ms, {} samples, {}",
                    report.elapsed_ms, report.samples, report.status
                );
                results.push(serde_json::json!({"name":name,"report":report}));
            }
            if let Some(path) = output {
                save_json(&path, &results)?;
            } else {
                println!("{}", serde_json::to_string_pretty(&results)?);
            }
            Ok(())
        }
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("knfsolv: {e}");
        println!("hello!");
        std::process::exit(1);
    }
}
