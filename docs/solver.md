# Solver implementation

The engine is a finite stochastic decision process with explicit opponent policies. Winning utility depends on all sheets, extra bonuses, active player, dice, and turns remaining. Expected-score optimization is a separate objective.

Computation controls are available in the TUI with `B` (or `:budget`). Sessions save `time_ms`, `simulations`, `max_nodes`, and `max_depth`. CLI analysis accepts `--time-ms`, `--simulations`, `--max-nodes`, and `--max-depth`. A zero time or depth means unlimited; zero samples or nodes is invalid. `max_depth` defaults to zero for existing sessions.

Depth limits only MCTS perspective-player decision nodes. The current action consumes one decision; chance outcomes and opponent moves consume none. Both training and frozen-policy evaluation use the baseline beyond the limit and still complete every simulated match. For finite limits, tree keys include decision depth to avoid combining states with different remaining search horizons. Exact search is never cut off and labeled exact by this setting; it retains its separate time/node limits.

## Exact calculations

Dice are six face counts. There are 252 complete five-dice histograms and 462 histograms of sizes zero through five. An outcome `x` of `m` dice has probability `m! / product(x_i!) / 6^m`. Tables enumerate every retained subset and its transitions; histogram probabilities are not uniform.

`solver/turn.rs` calculates the best action for all rolls and 0–2 remaining rerolls, given a value for each scoring action. It first calculates scoring values, then backs up rerolls using exact probability-weighted sums. The result is exact relative to that scoring continuation.

`solver/endgame.rs` memoizes whole positions. The perspective player maximizes terminal utility; opponents execute the declared observable-state policy. Chance nodes sum exact probabilities. A node, cancellation, or time limit aborts the exact query without labeling partial values exact. Completed positions return terminal metrics immediately. Exact search uses `f64` and a 1e-12 comparison tolerance.

The optional solitaire DP stores expected *additional* score at turn boundaries keyed by the open mask, capped upper subtotal, and additional-Kniffel eligibility. Transitions add category and bonus increments once. Completed scalar entries are reusable across sheets with the same key. Policy tables are reconstructed when all successors are covered. Partial precomputation can be resumed; interrupted root values are never stored as exact.

## Approximate multiplayer search

Full-game simulation advances every player in cyclic order until all sheets are finished. `opportunity` now selects **continuation_v2**; the retained v1 policy is exposed as `legacy_opportunity` for Standard computer opponents, training, and reproducible evaluation. Each legal scoring action gets `immediate score/bonus increment + V(next sheet)`. A single exact within-turn DP considers all scoring categories together, so future rolls can change the target. Keeping a pair does not commit to its upper row.

`policy/continuation.rs` approximates V from empirically fitted category-score distributions, separated by additional-Kniffel eligibility. It sums remaining category means, convolves the remaining upper distributions to estimate the probability of reaching 63, and estimates further extra awards when eligible. Already-earned totals/bonuses are not added again. This compact marginal model assumes independence of remaining upper outcomes and does not fully model competition/timing between future categories or future eligibility transitions. It is a sheet-dependent expected-score approximation, not an exact full-game value function or neural network.

For the last two open rows, successors are terminal or single-row states, so continuation is solved exactly using the existing turn DP, including bonus thresholds and extra eligibility. Optional exact solitaire cache coverage supersedes the approximation for larger sheets. Deterministic decisions are cached by open mask, capped upper subtotal and eligibility; each evaluator has bounded FIFO caches for decision tables and singleton values. Sampling caps each cache at `min(max_nodes, 8192)` entries; other evaluators default to 8,192. Optional loaded solitaire tables are fixed, read-only data. Cache eviction changes performance, not policy.

The bundled `data/continuation-v2.json` was fitted with 5,000 completed games per iteration, two policy-evaluation/improvement iterations, seed 42. Iteration one evaluates legacy v1; later iterations evaluate the previous fitted continuation policy. Rare eligibility/category groups borrow pooled observations. Training and held-out evaluation seeds are separate. Models are version/rules/shape validated and saved atomically. Reproduce with `knfsolv train-baseline --games 5000 --iterations 2 --seed 42 --output continuation-model.json`; use `analyze --baseline-model continuation-model.json` to load it. Missing/incompatible custom models fall back to the bundled model with a report note. No training runs at application startup.

MCTS expands perspective-player decisions across rerolls and future turns. UCT chooses actions; chance is sampled correctly; opponents follow fixed declared policies. Newly reached tree leaves use the baseline. The node cap bounds the number of stored decision nodes. The learned policy is frozen before evaluation; nodes with fewer than 32 visits per action retain the baseline. Other future nodes require an exploratory-return improvement larger than `R*sqrt(2*ln(K)/min(n_candidate,n_baseline))` before departing from it. This is a stability heuristic for nonstationary training returns, not a confidence interval. Reported alternatives use fresh complete-game simulations, and training rewards are never presented as fixed-policy confidence intervals.

Every legal root action initially receives eight validation samples. The bounded CLI/TUI wrapper issues no recommendation until every action has at least one sample. Incremental coaching shows a clearly labeled provisional baseline even before coverage, with insufficient confidence. Covered but under-sampled comparisons also return a provisional baseline. After initial coverage, validation allocates fresh paired rounds to the two highest estimates, two highest upper bounds, the baseline, and a rotating explorer (duplicates removed). Selection uses only previous outcomes. No action is permanently eliminated; the explorer revisits all actions. The baseline is included in every adaptive round. Both rollout and MCTS validation use this allocation; a simulation budget counts every completed game, including baseline comparisons and MCTS training.

Ranking and contender allocation use the highest **paired improvement estimate**: candidate-minus-baseline differences from shared-round outcomes, rather than unpaired means with different sample sets. Raw utility and expected score break ties. Raw outcome metrics remain available; they need not be monotonic in this paired ranking. This does not certify optimality. **Conservative advice is the default**: `conservative_advice: true` retains the baseline unless the sampled leader's paired improvement interval is wholly positive. It is explicitly labeled provisional when unresolved, not presented as evidence that baseline is better. Turning this off (`--sampled-advice` or `advice sampled`) suggests the paired sampled leader with a low-confidence warning. Verification found severe low-budget full-game regressions when automatically following noisy leaders; that is why the earlier proposal to default to the sampled leader was not shipped. The old uncalibrated root paired-error guard is removed. Reports separately expose `best_action`, `sampled_best_action`, `baseline_action`, `confidence`, `selection_reason`, and `baseline_comparison` (mean utility gain, interval, completed pair count); each alternative also exposes its own paired comparison. Conservative fallback never hides the sampled leader. Differences are in objective units: fractional shared-first/strict-first utility, or points for expected-score optimization.

ChaCha8 derives a separate seed per simulation and a separate stream per player, turn, and roll number. Alternatives share the same sample-index streams for paired comparisons, so using more dice now cannot shift the randomness of later turns. Opponent draws cannot shift the perspective player's stream. Policies cannot access future RNG state. Identical deterministic budgets/seeds yield identical action estimates; wall-clock budgets naturally depend on hardware. Paired streams are not a claim that each root alternative has independent samples from other alternatives; interval coverage uses a union bound and does not require that independence.

The reported utility intervals use bounded rewards and a union bound across actions and sample counts (`alpha / (K*n*(n+1))`). Shared-first utility lies in [0,1]; expected-score intervals use a conservative [0,2000] bound valid for supported rules. Paired candidate-minus-baseline differences lie in [-R,R] and use Hoeffding width with range 2R; only complete rounds contribute pairs. Each family (action intervals or paired intervals) has its own simultaneous, time-uniform 95% coverage under the frozen policies. They remain conservative under adaptive allocation/stopping and selected-leader reporting. Neither overlapping bounds nor failure to beat baseline establish that baseline is better. The expected-score bounds can be especially wide. Simulation error, continuation-policy bias, and opponent-model mismatch are separate issues.

## Incremental desktop coaching

`solver::AnalysisSession::new(position, settings)` owns the search. `advance(WorkSlice)` performs bounded work; `snapshot()` reads its current report. `set_paused` retains state, and `set_continuous(false)` applies the saved time/sample stopping rules. `WorkSlice::samples(n)` provides a deterministic completed-game boundary for offline callers and tests. `solver::analyze` remains the bounded interface used by CLI/TUI callers.

Desktop Auto uses fixed-policy rollouts, with one exact attempt when at most three categories remain across the table. The exact attempt is capped by the work slice, the remaining limited budget, and 500 ms (the worker uses 40 ms slices). Success stops computation; incomplete Auto continues with the unchanged fixed continuation policy. Explicit Exact stops after its bounded attempt, reporting incomplete if necessary. Cancellation before an exact result permits a retry on resume; partially explored exact trees are not retained. CLI/TUI Auto retains its existing exact-then-MCTS dispatch.

For explicit MCTS coaching, training finishes at the first of half the configured simulation budget or 2,000 completed training games, and half the configured positive time budget or 500 ms. With no configured time limit, the training cap is still 500 ms. The tree then freezes permanently; later validation never retrains or pools outcomes from different policies. An in-flight training game interrupted by the training-time cap contributes no counted sample. Depth limits still bound perspective-player tree decisions; the node cap bounds retained tree nodes. Fixed rollout has no search tree to truncate by depth and finishes each simulated match using the declared policies.

The sampler owns completed statistics, adaptive allocation, stream counters, unfinished paired rounds, in-flight simulated positions/RNG streams, evaluator caches, and any frozen tree. Yielding or pausing does not restart a sampled game or count it twice. Only completed rounds contribute paired comparisons. Fixed-seed sliced and uninterrupted runs agree at the same completed-work boundary when wall-clock cutoffs are disabled. MCTS training cutoffs based on elapsed time can freeze different policies on different machines; deterministic comparison tests use sample-bounded training.

Continuous mode ignores the overall time/sample stopping limits; node/cache/depth limits still apply. Limited mode defaults to two seconds / 20,000 completed games. Time includes initialization and active search work, excluding pauses and time spent waiting behind higher-priority jobs. Switching modes preserves evidence and the frozen policy; changing position, actor, objective, policy, seed, or search settings replaces the analysis. Search evidence is memory-only; saving stores advisor preferences, and loading starts a fresh search.

`coaching::ComputationWorker` owns one joinable thread. It retains at most one replaceable request and report for each of computer decisions and coaching, with computer decisions taking priority. Preempted coaching retains its owned session. Idle, paused, or completed workers wait on a condition variable. Work checks cancellation between game steps and publishes snapshots approximately every 250 ms, plus an initial provisional baseline and control/completion updates. A policy-evaluation step can overrun its time slice; the UI never waits for those steps during input handling.

Every result carries the controller revision, actor, job purpose, and request serial. Both the worker and GUI reject superseded results. Undo/load/new game clear pending work; dice corrections invalidate it immediately. Only complete, valid human decisions start normal-play coaching. Position editing uses the chosen perspective. Computer baseline jobs use the same worker with `baseline_only`, returning a legal declared-policy action without sampled games. Custom computer jobs run limited incremental MCTS. Provisional snapshots may update the thinking display; only a finished report tagged with the current computer-decision key can queue an action. Genuine failure or a finished report without advice submits a new baseline job on the worker and displays the fallback reason. Cancellation/stale reports are discarded. If even the baseline fails, computer play pauses with an explanation.

The controller queues each chosen computer action for a 400 ms presentation delay; Fast play removes that delay without changing the decision policy or restarting an active search. A GUI tick polls deadlines, generates actual rolls, submits decisions, and applies queued actions; it never sleeps. Selected keeps and scoring choices are visible before application. Pausing, editing, Undo, load/new game, navigation away, or opening settings/file popups invalidates or suspends executable work. No queued worker result is serialized. Resuming/reloading computes a fresh decision from the current recorded roll.

Worker initialization and search catch ordinary Rust panics as failures so the persistent worker can report them and attempt a legal fallback. The continuation evaluator’s 64 KiB bonus table is heap-owned to avoid excessive debug-build stack copies when moving owned searches through error handling. This changes storage, not policy values.

## Saved sessions

See the loadable [examples](../examples/middle.json). A session has `position`, `settings`, and optional `desktop` game metadata. A position contains schema/rules versions, player names and sheets, `turn_order`, `active_player`, `phase`, and `hypothetical`. Each sheet has exactly 13 `cells` and an `extra_kniffels` count. Dice counts follow faces 1–6; `[0,1,0,0,0,4]` means `2 6 6 6 6`.

Phases are tagged JSON objects:

```json
{"type":"before_first_roll"}
{"type":"decision","dice":[0,1,0,0,0,4],"rerolls_left":1}
{"type":"awaiting_roll","kept":[0,0,0,0,0,4],"rerolls_after":0}
{"type":"finished"}
```

An empty `settings` object uses defaults. Settings include `perspective`, `objective` (`shared_first`, `strict_first`, `expected_score`), `method` (`auto`, `exact`, `rollout`, `mcts`), default `policy`, optional `opponent_policies` with one entry per player, `time_ms`, `simulations`, `seed`, `max_nodes`, `max_depth`, optional `cache` and `baseline_model` paths, and `conservative_advice` (default true). Old sessions load with bundled continuation evaluation and conservative advice. The per-player policy also supplies that player's baseline when it is the perspective player. Paths are relative to the working directory.

`game::LocalGameConfig` initializes 2–8 named humans on empty sheets, rotates `turn_order` to the selected starting player, and preserves that order in the existing position schema. New computer-game sessions also store per-player baseline policies in `settings`, so CLI/TUI analysis can use the declared opponent without reading desktop metadata. Human digital turns begin in `before_first_roll` and wait for an explicit Roll command. `GameController::results` reports finished totals and all tied winners using the authoritative sheet bonus/total calculations.

Desktop metadata is optional and omitted from legacy/analysis sessions. Such files open in the GUI's Edit position mode; the CLI/TUI continue to analyze `position` and `settings`. Current GUI games add:

```json
"desktop": {
  "version": 1,
  "controllers": ["human", "human"],
  "dice_source": "digital",
  "advisor": {"mode": "continuous", "paused": false},
  "computer": {"policy": "continuation_v2", "search": null, "fast_play": false, "paused": false},
  "rng": {"seed": 1234, "word_position": 0}
}
```

`controllers` must have one entry per player (`human` or `computer`). `dice_source` applies to humans; computer actual rolls always use the gameplay RNG. Omitted advisor/computer/dice-source configuration receives defaults. Computer policy identities are `greedy`, `legacy_opportunity`, and `continuation_v2`; these are stable policy identities displayed as Easy, Standard, and Strong after the held-out Stage 5 comparison. Labels describe relative behavior among these presets. Optional `computer.search` stores a `Settings` object for custom search. Desktop execution uses its time/sample/depth/node/objective/seed/conservative settings, with MCTS and the bundled continuation-v2 baseline. Actor and per-player models are derived from the current game; advisor model/cache paths do not change desktop computer policies. The advisor records continuous/limited mode and pause state; both are honored by desktop coaching. Missing preferences default to continuous and unpaused. Changing only those controls retains the current search and does not add a gameplay undo step.

Desktop version 1 uses a separate ChaCha8 stream seeded from `rng.seed`. For each face it reads u32 words until one is below 4,294,967,292, then maps it to `word % 6 + 1`. `word_position` counts every consumed word, including rejections. Solver seeds, training, and analysis never consume this stream. Saves preserve the next gameplay RNG position. In-memory history records complete generated outcomes; redo restores those outcomes and the saved RNG state without generating new dice. Loading or starting a new game clears undo/redo and the event log; pending worker requests and queued automatic actions are never serialized. Dice slot order is an in-memory presentation detail, so loaded dice may appear sorted.

The TUI keeps editable state separate from search. A worker receives a snapshot and cancellation token. Revision tags discard stale results after any edit. Analysis never enters fictional dice or modifies scores. Position saves use a temporary file in the destination directory and atomic rename. UI snapshots support undo/redo. Page Up/Down scrolls long advice and help.

## Deliberate limits

This is not a solved full-game multiplayer equilibrium or a calibrated model of individual humans. Exact endgame dispatch is currently at the root; the general MCTS tree uses baseline continuation beyond expanded nodes. There is no neural value network, parallel search, or game-theoretic opponent mode. Startup works with the bundled small empirical model; full opening-table precomputation is not assumed to finish interactively. The stronger continuation policy costs more CPU per previously unseen sheet than v1, so a fixed wall-clock budget can yield fewer simulations. Low-budget sampled leaders may differ from the baseline and remain noisy. Strength claims must be based on measured regret or match outcomes, not confidence interval width alone.

### Structured search progress

`Report.progress` is optional and defaults to absent when reading older JSON reports.
It contains `phase` (`mcts_training`, `mcts_validation`, `exact`, or `rollout`)
and `max_decision_depth`. The depth counts decisions by the perspective player,
starting at one for a decision root. Chance outcomes and other players' decisions
never increment it. MCTS records actual tree visits/expansions during training,
excluding the baseline continuation after a leaf or depth/node limit. Validation
retains that maximum after the tree freezes. Exact search records its actual
traversal, including partial attempts; it does not imply that every branch was
solved to that depth. Rollout analysis has no tree depth. Instrumentation draws no
random numbers and does not alter action selection or configured search budgets.

Incremental snapshots preserve progress when paused, reset it for a new search,
and show no progress before an exact attempt starts. Auto's incomplete exact
attempt is described in the report note; the progress field describes the current
fallback search rather than the abandoned attempt.
