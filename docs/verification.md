# Verification results


## October 8: five of a kind as a Full House

The family's house rule now holds in the engine: five of a kind scores 25 in Full House, whether or not Kniffel is open, and a Full House can no longer be crossed out with five of a kind (rules version `schmidt_pdf_2024_house_v2`, [rules.md](rules.md)). Positions saved under `schmidt_pdf_2024_standard_v1` load as current positions; v1 models and solitaire caches are rejected and rebuilt.

The bundled continuation model was retrained with the original parameters (`train-baseline --games 5000 --iterations 2 --seed 42`, about 2 s). Its final-iteration Full House histogram for turns without extra-Kniffel eligibility moved from 1,069 zeros and 2,615 × 25 to 1,036 and 2,707. `five_of_a_kind_is_a_full_house_in_play_and_advice` checks scoring, the forbidden cross-out, and that the exact solver writes the Full House (25) with 2 2 2 2 2, Kniffel crossed out, and only Full House and Chance open, where the old rule forced a cross-out. `positions_saved_under_the_previous_rules_load_as_current` checks the upgrade. The workspace tests pass.

Measurements below this entry, including the preset strength and advice-mode comparisons, were taken under the v1 rule and were not rerun; the rule changes few decisions (mostly five of a kind with Kniffel already crossed out), so their conclusions are expected to hold.

## October 6: advice modes in full games

Should coaching recommend the move with the highest displayed win chance instead of the conservative baseline? `compare_advice_modes_in_full_games` in `tests/quality.rs` plays each game once per advice mode with the same dice for every player, turn, and roll. The analyzed player follows coaching exactly as the app runs it (`AnalysisSession`, Auto: a bounded exact attempt in small endgames, otherwise fixed-policy rollouts) with 5,000 games per decision, about the phone's three seconds. The other player uses the bundled continuation-v2 baseline; the analyzed seat rotates. All three modes read the same report, which has identical alternatives in both advice settings.

Two players, 300 games (±: approximate 95% interval of the paired difference):

| Advice mode | Mean score | Shared-first credit | Difference from conservative |
| --- | ---: | ---: | --- |
| Conservative (default) | 227.64 | .442 | — |
| Paired sampled leader (`advice sampled`) | 212.73 | .397 | score −14.91 ± 5.57, credit −.045 ± .053 |
| Highest displayed objective value | 173.67 | .205 | score −53.97 ± 6.00, credit −.237 ± .059 |

Following the highest displayed estimate is much worse: with 20–30 candidate moves and a few hundred games each, the maximum of noisy raw estimates is often a lucky, rarely sampled move. The paired leader is also worse at this budget. The conservative default stays. Reproduce with `KNFSOLV_ADVICE_GAMES=300 cargo test --release --test quality compare_advice_modes -- --ignored --nocapture` (about 20 minutes on 16 threads; `KNFSOLV_ADVICE_PLAYERS` and `KNFSOLV_ADVICE_SAMPLES` change the setup).

The example that prompted this, `2 2 3 4 5` on an empty two-player sheet: the baseline keeps `2 2` with two rerolls left and `2 3 4 5` with one. With two rerolls left and 100,000 rollouts, keeping `2 3 4 5` leads by a paired +.029 shared-first utility, but its conservative interval [−.027, +.086] still includes zero, so the baseline remains the advice. The conservative rule is strict enough that it rarely departs from the baseline within a phone's budget, even where the baseline is weak.

## September 13 persistent preferences and engine controls

The routine suite passes **113 tests** with GUI support and **77** without it.
The three statistical evaluations and the Linux worker soak remain explicitly
ignored by routine runs. Formatting, Clippy with warnings denied in both feature
configurations, and the release build pass.

New isolated-directory tests cover preference round-trips, the 300 ms debounce,
rescheduling, exit flushing, unchanged files and reverted changes, corrupt and
unsupported files, failed writes, and Retry. Fresh desktop instances verify that
global advice overrides loaded session settings, preserves perspective, normalizes
per-seat defaults, survives Undo/Redo, and does not dirty a saved game. GUI saves
omit global advice and retain computer search configuration. Existing CLI/TUI
round-trip and legacy-session tests continue to pass.

Search tests cover backward-compatible Report JSON, perspective-only decision
depth, opponent/chance exclusion, MCTS training versus frozen validation, baseline
rollout exclusion, pause/resume, exact completion/incompletion, and Auto fallback.
GUI tests cover the Engine shortcut during waiting and computer turns, progress
labels, stale-result rejection, and both themes at compact and wide sizes.

Native X11 release captures with isolated preferences were inspected in dark and
light themes at 1180×900, 800×600, and 360×300. The depth label, recommendation,
and play action fit at 800×600; the smallest viewport uses the existing full-area
scrolling layout. Temporary screenshots and automation live under
`/tmp/knfsolv-preferences-native/` and `/tmp/knfsolv-preferences-native.py`.
Fresh native processes reload the isolated theme and engine configuration correctly.
After accounting for initial window focus, native pointer events opened the Engine
tab directly and changed the theme. Both dark-to-light and light-to-dark changes
were automatically saved and retained across clean exits and restarts. Pause
persistence is covered by GUI/fresh-instance tests.
The native captures use an explicitly loaded legacy example in position-editing
mode, where play actions are intentionally disabled. Pointer activation of normal
play actions remains covered by the existing GUI tests.

## September 12 GUI Stage 6 release checkpoint

The final release suite passes **97 routine tests**; the terminal-only debug suite passes **72**. Formatting, Clippy with warnings denied in both feature configurations, and the release build pass. The Linux resource soak is explicitly ignored by routine runs, as are the three statistical evaluations.

Commands run:

```sh
cargo test --offline --release --quiet
cargo test --offline --no-default-features --quiet
cargo test --offline --release --test release_checks -- --ignored --nocapture
cargo test --offline --release --test quality -- --ignored --nocapture
cargo clippy --offline --all-targets -- -D warnings
cargo clippy --offline --no-default-features --all-targets -- -D warnings
cargo fmt --check
cargo build --offline --release
```

The new Linux soak runs four-player continuous analysis for 60 seconds per method with the Greedy baseline, a 128-node limit, and a slow reader polling every 750 ms. Continuation-cache eviction and evidence preservation are covered separately by the deterministic analysis tests. It asserts monotonic completed evidence, at most two buffered reports, one worker thread, no more than 32 MiB RSS growth after ten seconds of warmup, and worker shutdown within two seconds. The first run exposed a test-observation race: Linux `/proc/self/task` can briefly retain a terminating task after Rust joins it. The assertion now polls within the shutdown deadline instead of requiring instantaneous kernel cleanup. This changes the test only.

The corrected **120.01-second soak passed**. Rollout completed 3,045,632 samples with 80 consumed reports; resident memory was 4,996 KiB after warmup and peaked at 5,012 KiB. MCTS completed 2,553,216 samples with 80 consumed reports; resident memory was 5,356 KiB after warmup and peaked at 5,364 KiB. Both retained exactly one additional worker thread and stayed within the node/report limits. The worker exited within the shutdown deadline and the process returned to its original thread count. Counts and RSS are environment-dependent observations, not performance guarantees.

Native X11 release-window checks exercised Tab focus and Enter activation from Home into setup, five-key automatic physical entry, and Right/Space selection and retention. Captures show the textual Keep indicator separately from advice. Both dark and light 800×600 gameplay keep current dice and advice visible. Native wheel scrolling at 360×300 reaches the final score categories and totals. Complete native local physical/digital and human/computer physical/digital matches were already exercised at the Stage 4–5 checkpoints below; application sources have not changed during Stage 6. Existing event tests cover mouse entry, locked rerolls, corrections, scoring/Undo, themes, focus, and compact/wide layouts.

All **three statistical evaluations passed** in 701.65 seconds using their default configurations. The older endgame check ran 80 fixture/seed pairs per method (Rollout and MCTS), each reporting mean regret 0.06250268 and maximum 0.25 against exact references. Its full-game smoke evaluation completed 12 two-player games and 12 three-player games at 2,048 samples per decision: mean shared-first utility/score were 0.6667/256.33 and 0.2500/212.33 respectively. These small exploratory samples do not establish search strength. The separate held-out continuation comparison also completed its default 300 games per player count (two, three, and four).

The difficulty ordering gate again reproduced the Stage 5 paired results below. No Custom search calibration was added. Native verification covers Linux X11 only; screen-reader behavior and other native platforms remain unverified. A two-minute soak does not establish indefinite resource stability. Exact attempts retain their documented bounded, non-resumable limitation.

## September 12 GUI Stage 5 checkpoint

Computer play passes **97 routine tests** with GUI support and **72** without it (25 GUI tests, six worker tests, and 19 controller integration tests, alongside existing solver/TUI coverage). Formatting, Clippy with warnings denied for both feature configurations, and the release build pass. The two older statistical evaluations remain ignored by routine runs. The new difficulty acceptance test was run explicitly in release mode.

Computer tests complete eight matches: Easy, Standard, Strong, and a small-budget Custom search, each with physical and digital human dice. They exercise the real persistent worker and legal game-controller loop through final results, human-decision Undo, and recorded Redo. Additional coverage checks preset/custom setup and controls, all 252 dice histograms × three reroll counts for the declared legacy policy, JSON identity and saved per-player models, separate computer/advisor budgets, zero-sample legal baseline jobs, rejection of provisional or wrongly tagged executable results, genuine failure and exhausted-no-advice fallback, 400 ms presentation timing, and stale-result rejection after pause, Undo, load, navigation, and settings. A paused computer turn saves/reloads from its recorded dice and starts a fresh decision. Routine worker tests exposed debug-build stack pressure in the large continuation table; heap ownership fixes it without changing evaluated values.

Commands run include `cargo test --offline --quiet`, `cargo test --offline --no-default-features --quiet`, `cargo clippy --offline --all-targets -- -D warnings`, `cargo clippy --offline --no-default-features --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo build --offline --release`. The complete computer-match GUI test takes about 26 seconds in this debug environment; release policy evaluation is much faster.

### Preset ordering gate

Reproduce with:

```sh
cargo test --offline --release --test quality desktop_difficulty_ordering -- --ignored --nocapture
```

Each adjacent comparison uses **1,000 independent held-out seeds**, each played twice with swapped seats: **2,000 complete two-player matches per comparison**, 4,000 total. Seeds start at `0x5354414745350000`, separate from fitting and prior evaluations. Actual roll streams are matched by seat, completed turn, and roll number across the swapped games. Each pair contributes its average stronger-minus-weaker score and average stronger-player win credit (1 for an outright win, 1/2 for a tie).

| Comparison | Paired mean score advantage (approx. 95% interval) | Stronger preset win credit | Simultaneous 95% win-credit interval |
| --- | ---: | ---: | --- |
| Standard vs Easy | +111.349 ± 2.202 | 0.9900 | [0.9432, 1.0000] |
| Strong vs Standard | +16.297 ± 2.060 | 0.6110 | [0.5642, 0.6578] |

Win-credit intervals use Hoeffding bounds on independent pair averages in [0,1], with alpha 0.025 per comparison (Bonferroni protection across both comparisons). Score intervals are conventional approximate fixed-sample intervals from paired differences. Both mean score differences are positive and both win-credit lower bounds exceed 0.5, so the Easy/Standard/Strong labels pass the agreed gate. No Custom search budget ordering or strength against humans is established. The final release rerun reproduced the same results after the storage change.

### Native interaction

Native X11 checks use a temporary release-window process and directed keyboard/pointer inputs. They inspect the new computer setup at 800×600 and gameplay at 1180×900, including generated computer dice and visible retained/scoring choices. The physical-human/Standard match completed after exercising Pause computer, saving and CLI-validating the paused computer turn, Undo to the preceding human decision, explicit discard of that test Undo while loading the saved turn, Resume computer, and Fast play. It ended **human 220, computer 267**; the result showed the correct winner and the computer’s upper bonus. The fully digital-human/Standard replay repeated the same pause/save/Undo/reload/resume workflow and finished **human 68, computer 250**. Both finished JSON files passed CLI validation, and each human digital turn required an explicit Roll. Native captures show the chosen keep before application. These scripted matches verify interaction and persistence, not playing strength.

Broad manual screen-reader/navigation audits, extended memory/concurrency profiling, and a final release pass remain Stage 6. See the [Stage 5 handoff](gui-redesign.md#stage-5--computer-opponents).

## September 12 GUI Stage 4 checkpoint

Local-game setup and results pass **89 routine tests** with GUI support and **69** without it, including 20 GUI tests and 17 controller integration tests. Commands run: `cargo test --offline --quiet`, `cargo test --offline --no-default-features --quiet`, `cargo clippy --offline --all-targets -- -D warnings`, and `cargo build --offline --release`. Formatting passes. After the native check prompted a stable-row layout fix, all 20 GUI tests were rerun; the extended complete-game/save-reload test and Clippy also pass. The two statistical quality evaluations remain ignored; no new difficulty or strength claim is made.

New controller tests cover every starting seat for 2–8 players, trimmed/nonblank names and invalid setup, saved configuration, no automatic human digital roll, fresh history, complete physical/digital matches with two/eight players, exact sheet-derived bonuses and totals, all tied winners, and final-score Undo/Redo. GUI tests operate setup controls with pointer events, edit names without entering dice, exercise local/editor/load routes and invalid loads, preserve partial dice while cancelling replacement, and play full two-player physical and digital games using roll/palette/score buttons. They verify stable score-row positions, result review, final Undo/Redo, new-game navigation, and reloading finished desktop games into results with fresh history.

Setup and results render in both themes at 360×300, 800×600, and 1180×900; eight-player setup fits at 800×600, and smaller windows scroll. Existing tests continue to cover standard keyboard navigation, compact/wide play layouts, automatic coaching, stale results, persistence, exact/probability calculations, and TUI behavior.

Native X11 release-window checks completed two full two-player games from the new start screen with no scoring confirmations or advanced settings. The physical game entered five sixes per turn using directed number-key events and scored all 26 categories by pointer, ending in a **220–220 tie** with 50 extra Kniffel points each. The digital game selected Digital dice during setup, explicitly clicked Roll each turn, and scored all categories, ending **52–36** with Player 1 correctly identified as the sole winner. Coaching started automatically and was paused by native pointer input during each replay. Setup captures were inspected at 800×600; gameplay and results at 1180×900. These were scripted native inputs, not a manual accessibility audit. The initial replay exposed shrinking filled rows; fixed row/action-area heights now keep score targets stable, verified in both subsequent native matches and GUI tests.

Computer turns, screen-reader/manual navigation audits, extended concurrency/memory profiling, and held-out difficulty ordering remain later-stage work. See the [Stage 4 handoff](gui-redesign.md#stage-4--local-game-setup-and-results).

## September 12 GUI Stage 3 checkpoint

Continuous coaching passes **81 routine tests** with GUI support and **66** without it. The two statistical quality tests remain ignored; no new difficulty or strength claim is made. Commands run: `cargo test --offline --quiet`, `cargo test --offline --no-default-features --quiet`, `cargo clippy --offline --all-targets -- -D warnings`, and `cargo build --offline --release`; formatting also passes. Clippy also passes with `--no-default-features`.

Nine incremental-search tests compare sliced, paused/resumed, and uninterrupted fixed-seed evidence at identical completed-work boundaries, including an unfinished multiplayer game and paired round. They verify no double counting, paused-time exclusion, extending a limited search without restarting, immediate provisional advice, exact completion/fallback, MCTS training caps and permanent freezing, time-slice/cancellation responsiveness, and bounded cache eviction without changing evidence. Five persistent-worker tests verify priority/preemption, same-thread resumption, latest-request replacement, one buffered report per purpose, invalid requests/errors, and cancellation on drop. The suite includes 15 GUI tests, 14 controller integration tests, and existing exact/probability/persistence/TUI regressions.

Native X11 release-window checks entered `6 5 4 3 2` through five directed number-key events. At 800×600 and 1180×900, dice and automatic advice remained visible, and successive captures showed increasing completed-game counts without manual analysis commands. Native pointer clicks at compact size verified Pause/Resume: two paused captures retained 7,019 games and 2.92 seconds of computation; after Resume the count reached 9,137 games and 3.80 seconds. Full native games, screen-reader behavior, long-duration memory profiling, and difficulty ordering remain release-stage acceptance work. The GUI does not yet run computer turns. See the [Stage 3 handoff](gui-redesign.md#stage-3--continuous-background-coaching).

## September 12 GUI Stage 2 checkpoint

The controller extraction passes **65 routine tests** with the GUI enabled and **52** without it. This includes 14 controller/integration tests and 13 GUI tests. The two statistical quality evaluations remain ignored; no new strength evaluation is claimed. `cargo fmt --check`, `cargo clippy --offline --all-targets -- -D warnings`, and `cargo build --offline --release` pass.

Controller tests finish complete physical and digital local games with two and eight players, plus physical-human and digital-human games against a computer whose legal choices are supplied by the test. They check partial drafts and retained slots, correction debounce, human-decision Undo, recorded-outcome Redo, independent gameplay RNG state, saved RNG continuation, computer-turn save/load, nonblocking 400 ms scheduling, pause/fast play, configuration/revision/actor-purpose guards, and illegal-command rejection. Old sessions still load; new metadata survives a TUI settings edit/save and CLI validation. GUI event tests cover the digital Roll button and Undo/Redo; loading a legacy session clears history and rejects queued advice.

A native X11 release-window smoke check entered `6 5 4 3 2` with five directed key events and captured correct dice, prospective scores, and advice at 800×600 and 1180×900. Native digital-game interaction and full games were not audited at this checkpoint; complete-game checks used the GUI-independent controller. The controller supports computer scheduling, but the desktop does not execute computer turns yet. The persistent worker/continuous coaching and visible opponent loop remain Stages 3 and 5. See the [checkpoint handoff](gui-redesign.md#stage-2--game-controller-and-session-metadata).


## September 12 GUI Stage 1 checkpoint

The physical-play redesign passes **49 routine tests** with the GUI enabled (including 11 GUI tests) and **38** in the terminal-only configuration. Both runs leave the two statistical quality evaluations ignored; no new engine-strength claim is made. `cargo fmt --check`, `cargo clippy --offline --all-targets -- -D warnings`, and `cargo build --offline --release` pass.

GUI tests cover five-click and five-key automatic entry; stable slots and selected duplicate-die locks; reroll entry/backspace; settings changes during partial input; 250 ms correction grouping and undo/redo; pending-draft action/save guards; one-click scoring, including zero, and one-click suggestions; stale reports; save-before-load safety; opaque dark/light backgrounds; and compact/wide rendering from 360×300 through 1600×1100. The 800×600 checks assert that dice, advice, analysis controls, and the sheet heading remain in view. Native inspection caught an inherited horizontal layout in the wide advice panel; this now has a regression assertion.

A native X11 release-window smoke test entered `6 5 4 3 2` via five directed number-key events and captured the resulting GUI at 800×600 and 1180×900. The captures show correct slot order, prospective scores, and stacked/side advice layouts. Complete native mouse gameplay, screen-reader behavior, and the later continuous-analysis/computer-play acceptance criteria remain unverified. See the [checkpoint handoff](gui-redesign.md) for behavior and remaining stages.


## September 10: continuation evaluation and explicit uncertainty

The default-feature suite now has **44 passing routine tests** (11 library, 25 engine integration, eight TUI integration), verified in debug and release. Terminal-only testing passes 38 tests. Formatting and Clippy with warnings denied pass; the full release GUI/CLI binary was rebuilt. The two statistical tests remain ignored by routine runs and were executed separately.

New regression coverage includes:

- Opening `1 1 2 5 5` baseline keeps `5 5`; when only Ones remains it correctly keeps `1 1`. No face-specific preference rule was added. `examples/pairs.json` reproduces the opening position.
- Four two-open-row sheets, including bonus-boundary and additional-Kniffel eligibility cases: all 252 dice histograms and all three reroll counts match exact solitaire values and actions (3,024 comparisons).
- Reproducible model training/save/load, invalid-version rejection, already-earned bonus nonduplication, and old-session defaults.
- Initial root coverage followed by unequal adaptive allocation; rotating exploration never abandons an action. Fixed-seed reproduction and depth-1/fixed-policy equivalence still pass.
- Explicit sampled-leader versus conservative-baseline selection, paired gain intervals, UI commands/history, stale analysis invalidation, and existing graphical background/rendering regressions.

The bundled empirical model was trained with 5,000 games per iteration, two iterations, seed 42. Held-out evaluation used seeds starting at 900000, 300 games per player count, rotated analyzed seats, and matched per-player/turn/roll randomness. Only the analyzed player's policy changes between legacy-v1 and continuation-v2 replays; opponents use legacy v1. These measure **baseline-policy improvement**, not MCTS strength against humans or globally optimal play.

| Players | Legacy mean score | Continuation mean score | Paired score gain (approx. fixed-sample 95% interval) | Shared-first credit, old → new | Upper bonus rate, old → new |
| --- | ---: | ---: | --- | --- | --- |
| 2 | 221.53 | 241.45 | +19.93 ± 4.87 | .477 → .597 | .280 → .583 |
| 3 | 221.25 | 239.33 | +18.09 ± 4.84 | .317 → .433 | .257 → .573 |
| 4 | 224.04 | 244.02 | +19.98 ± 5.24 | .267 → .397 | .257 → .613 |

Reproduce with `cargo test --release --test quality compare_continuation -- --ignored --nocapture`. `KNFSOLV_BASELINE_GAMES` overrides the default 300. These score intervals are conventional approximate fixed-sample intervals, separate from the engine's conservative anytime Hoeffding intervals.

Verification changed one proposed default: automatically following uncertain sampled leaders regressed badly at low budgets. An intermediate raw-ranking implementation scored 190.92/169.75 over 12 games at two/three players; a four-game paired-ranking revision still scored 176.25/139.25. These are small exploratory samples, but were sufficient warning not to ship sampled-leader mode as the default. Conservative advice now defaults on, is explicitly labeled provisional when uncertain, and never conceals the sampled leader. Sampled advice remains opt-in. The old root standard-error heuristic was replaced by bounded, simultaneous time-uniform paired intervals; the future-tree policy also has a stricter stability gate.

The final conservative MCTS smoke evaluation (2,048 samples/decision, four games per player count, rotated analyzed seat against continuation-v2 opponents) scored 244.75 at two players with .500 shared-first credit and 197.25 at three players with .000 credit. The three-player result is poor; four games do not support a reliable strength claim. On the 80 forced-approximate fixture/seed comparisons with deliberately weak Greedy as baseline, both rollout and MCTS had mean regret .06250268 and maximum .25 under conservative mode. This exposes the price of retaining a poor baseline when rigorous bounds are unresolved. Auto solves these small fixtures exactly; explicit sampled advice remains available. The earlier permissive root-selection variants had mean regret .00000268 and maximum .00001072 on these fixtures, but their full-game performance was the reason for retaining a conservative default.

Reproduce that short search check with `KNFSOLV_QUALITY_GAMES=4 cargo test --release --test quality report_endgame_regret_and_match_results -- --ignored --nocapture`; omit the variable for 12 games. The original September 7 benchmark figures below are historical, not current continuation-v2 throughput. The richer baseline costs more per unseen sheet. In one final opening query the two-second default budget completed 3,272 total simulations and recommended the fives with an explicit low-confidence conservative explanation; counts depend on hardware/cache/load. No new comprehensive performance benchmark is claimed. Headless GUI tests ran for this solver update; no new native-window interaction audit was performed.

## Earlier checkpoints

Measured on 2026-09-07 with Rust 1.96.0, Linux x86_64, AMD Ryzen 7 9800X3D (8 cores/16 logical CPUs). Search currently uses one CPU worker. Results are local measurements, not cross-platform guarantees.

Completion check on 2026-09-09: formatting, Clippy with warnings denied, and all 25 routine tests passed again in debug and release builds. The floating-point JSON fix was also verified through the CLI: generate a 2,048-state table, reload and extend it to 4,096 states, then load it for a 1,000-simulation middle-game analysis. All steps succeeded without checksum errors. The leading and trailing exact examples still return shared-first utilities of 1 and 1/4 respectively. Performance and statistical-quality figures below remain the original September 7 measurements; they were not remeasured in this completion check.

## Correctness and interface

The subsequent egui desktop implementation brings the default suite to **36 passing tests**, verified in debug and release; formatting and Clippy with warnings denied pass. The terminal-only feature configuration also passes its 31 tests. Five new GUI unit tests cover actual dice/keeps/rerolls/scoring/undo, unconfirmed draft guards, stale worker results/settings changes, saving before loading without overwriting the target file, and headless rendering at 600×450, 1180×900, and 1600×1100 with eight players and edit/score/unsaved dialogs. The native desktop window successfully started with `examples/middle.json`. This native startup smoke check does not establish full manual mouse or screen-reader coverage.

The September 9 TUI usability update brings the routine suite to **31 passing tests** (23 engine integration, seven TUI integration, one internal unit test), verified in debug and release with Clippy warnings denied. New coverage includes visual dice selection, incomplete-roll rejection, cancellation/undo, locked kept dice, correction without consuming a reroll, Vim navigation, action-menu rendering at three terminal sizes, and scoring preview/confirmation. A real pseudoterminal run loaded the trailing fixture, analyzed, applied the reroll to open the picker, entered the new face, navigated to Chance with G, confirmed scoring, and quit successfully. The earlier counts below describe preceding checkpoints.

The routine suite now contains **27 passing tests**: 23 engine integration tests, three TUI integration tests, and one stale-worker-result unit test. It was run in debug and release builds. Formatting and Clippy with warnings denied also pass. The separate statistical evaluation is ignored by routine tests and was run explicitly.

The computation-settings update adds checks for editing/saving budget controls and enforcing MCTS depth. Depth 1 creates only the root decision node; its independent validation matches fixed-baseline full-game rollouts, while depth 2 expands additional decisions. A CLI check using `--max-depth 1 --max-nodes 100 --simulations 1000` returned one search node and 1,000 samples. Existing sessions without a depth setting still load with unlimited depth.

Coverage includes independent enumeration of all 7,776 ordered dice rolls, histogram multiplicities, all categories and additional-Kniffel rules, upper-bonus boundaries and nonduplication, turn transitions, changing retained dice, validity of generated full matches with 1/2/4/8 players, exact endgames, reproducible sampling, incomplete/cancelled searches, optional-cache validation, and save/load round trips.

The exact Chance fixture returns:

| Position | Advice | Shared-first utility |
| --- | --- | --- |
| `6 6 6 6 2`, one reroll, opponent finished on 25 | Write 26 | 1 |
| Same dice, opponent finished on 29 | Keep four sixes, reroll the 2 | 1/4 |
| Same dice, expected-score objective | Keep four sixes, reroll the 2 | 27.5 expected points |

Another exact fixture keeps the opponent's current total at 25 but changes their last open row between Ones and Chance. With `6 6 6 6 4`, advice changes from scoring 28 to rerolling the 4. The opponent still has to play, so this checks actual future-sheet dependence, not only comparison with a finished target.

The TUI was exercised in an actual pseudoterminal: load the trailing example, analyze, apply the reroll, enter five sixes as the actual result, analyze again, score 30, save to a temporary file, quit, and validate the finished session through the CLI. Rendering tests cover 40×16, 80×24 and 120×40 terminals, including eight players. Tests verify undo/redo and transactional rejection of invalid commands. A cancellation test interrupts an active eight-player search and joins its worker within a 500 ms test bound; the design checks cancellation within each simulated game step.

## Performance

Command: `target/release/knfsolv benchmark --time-ms 1000`, wrapped with `/usr/bin/time -v`. The default 20,000 total-sample cap still applies. Counts include both MCTS training and independent validation games.

| Fixture | Time | Samples | Result |
| --- | ---: | ---: | --- |
| Opening, 2 players | 649 ms | 20,000 | Estimated |
| Middle game, 2 players | 289 ms | 20,000 | Estimated |
| Opening, 4 players | 1,001 ms | 15,397 | Estimated |
| Middle game, 4 players | 555 ms | 20,000 | Estimated |
| Opening, 8 players | 1,000 ms | 7,734 | Estimated |
| Middle game, 8 players | 1,001 ms | 17,852 | Estimated |
| Leading Chance endgame | <1 ms reported | 0 | Exact |
| Trailing Chance endgame | <1 ms reported | 0 | Exact |

Peak process resident memory for this suite was 30,272 KiB, about 29.6 MiB. Wall time was 4.50 seconds. The two Chance searches each explored 261 nonterminal states. A one-millisecond deadline overrun is possible from the current work unit and report construction. A cold opportunity-policy table build is included in the first fixture's time.

A separate 200 ms blank-sheet solitaire-precompute run saved 2,037 completed turn-boundary values and returned after 212 ms including saving. It correctly reported the blank root as unsolved. The one-category Chance table completed with exact expected additional score 23.333333 and loaded successfully for analysis. A larger resume check exposed JSON float-parsing drift in checksum verification; enabling exact float round-tripping fixed it. The multi-entry regression test saves, reloads, extends, and reloads a partial table. No full blank-sheet table-generation time is claimed.

## Statistical quality and limits

Reproduce the evaluation with:

```sh
cargo test --release --test quality -- --ignored --nocapture
```

On four exact endgame fixtures and 20 seeds (80 decisions), with 1,000 total samples per query and Greedy as the declared baseline/opponent policy:

| Method | Mean decision regret | Maximum regret |
| --- | ---: | ---: |
| Fixed rollout | 0.00000268 | 0.00001072 |
| MCTS plus guarded advice | 0.00312768 | 0.25 |

Regret is exact optimal shared-first utility minus the exact value of the recommended action. The MCTS maximum is a real low-budget miss, including the conservative baseline safeguard; this is why the program does not label sampled advice exact. Auto mode solves these small endgames exactly instead of using these forced approximate modes. This small fixture set does not measure opening-game strength.

An exploratory full-game check used 12 games at each player count, rotated the analyzed seat, and gave MCTS 2,048 samples per decision against the opportunity heuristic:

| Players | Mean shared-first utility | Mean analyzed-player score |
| --- | ---: | ---: |
| 2 | 0.6667 | 239.83 |
| 3 | 0.0417 | 202.42 |

These samples are too small to infer a reliable win rate, and do **not** establish that MCTS is stronger than the heuristic. The three-player result is poor. More simulation and a better continuation/opponent model remain worthwhile improvements. The evaluation can be repeated with `KNFSOLV_QUALITY_SAMPLES=20000` to increase the decision budget.

Testing exposed severe overfitting in an earlier low-budget tree policy. The implementation now retains the baseline at under-sampled future nodes, couples future randomness by player/turn/roll, and requires a paired-error improvement before replacing baseline root advice. A regression test covers the very-small-budget fallback. These measures improve robustness; they are not a proof of global optimality.

The deliverable is a functioning, verified rules/probability engine with exact small-position analysis and explicitly approximate multiplayer advice. Human playing strength, full-game equilibrium optimality, and a completed opening solitaire database have not been established.


## Expanded advice panel verification

The move-card update passes `cargo test --offline --quiet` (100 routine tests, four extended tests ignored), `cargo fmt --check`, `cargo clippy --offline --all-targets -- -D warnings`, and `cargo build --offline --release`. The 28 GUI tests include structured-card visibility/order, disclosure retention on report updates, missing estimates, zero scoring, rerolls, tied outcomes, objective-specific intervals, and unclipped recommendation statistics/action at 800×600 and 1180×900 in both themes.

Native X11 screenshots of the release application loaded with `examples/middle.json` were inspected at 1180×900, 800×600, and 360×300 in dark/light themes. The recommended retained dice and all four statistics fit with the play button at compact size. Native clicks expanded Analysis details and all moves; the details correctly identified a conservative baseline differing from the sampled leader. Tab produced visible focus, and wheel scrolling in the small window reached cards and final sheet totals. Captures and the temporary automation script live under `/tmp/advice-*.png` and `/tmp/knfsolv-advice-native.py`; they are local verification artifacts, not repository dependencies. The final provisional-baseline status label and fixed statistic column widths were added after these native captures and are covered by GUI regression tests. Screen-reader behavior and other window systems were not audited for this change.


## Gameplay styling and playable alternatives — September 13, 2026

The final routine suite passes 106 tests, including 34 GUI tests, with four extended checks left ignored. Commands: `cargo test --offline --quiet`, `cargo fmt --check`, `cargo clippy --offline --all-targets -- -D warnings`, and `cargo build --offline --release`. Coverage includes alternative scoring, zero and expanded moves, duplicate physical/digital keeps, Undo/Redo, disabled actions, numeric accessible labels for pip buttons, keyboard activation, the enlarged Roll target, and hidden digital face entry. A reordering test caught and verified the fix for focus following an action rather than its old list position.

Native X11 captures inspected dark/light gameplay at 1180×900, 800×600, and 360×300, including scrolling through cards and final score totals. A native click on an alternative retained exactly its three threes, cleared the other two physical slots, and displayed Locked labels; Undo restored the previous roll. A separate digital session verified the 160×44 Roll button, absent physical picker, generated faces, and the visible recommendation/action at 800×600. Native Tab focus was exercised; headless keyboard tests additionally verify pip activation and action identity after report reordering. The final compact viewport is 180 pixels; the final digital captures include this setting. Full screen-reader behavior and other window systems remain unverified.

Temporary artifacts are `/tmp/play-*.png`, `/tmp/knfsolv-play-native.py`, and `/tmp/knfsolv-digital-native.json`. The test application was closed after the audit. No solver, controller, persistence, or CLI/TUI interfaces changed.


## Before-roll position forecasts — September 13, 2026

The routine suite passes 117 tests (four extended checks remain ignored). Formatting, Clippy with warnings denied, and the release build pass. The new forecast regression was also rerun after extending it to verify that switching Limited to Continuous continues beyond the sample budget while retaining the request, and pausing retains its context.

Coverage includes non-mutating turn inference for one through eight players, all rounds and rotated starting orders, zero scores, inconsistent prefixes, completed games, the supplied Three of a Kind example, atomic turn/dice/history changes, score corrections, hypothetical perspective selection, partial/full dice entry, returning to forecasts, Undo/Redo, paused startup, Limited budgets, Continuous continuation, and stale worker result rejection. Existing local/computer gameplay and global engine preference tests pass. The solver and save schema are unchanged.

Native release captures were inspected in dark/light themes at 1180×900, 800×600, and 360×300 using `examples/before-roll.json`. The larger layouts show the inferred Player 2 and all four forecast statistics with no roll-all move card. The smallest layout uses the existing scrolling play area. Headless rendering also covers all six theme/size combinations. Temporary native artifacts are `/tmp/knfsolv-forecast-native/` and `/tmp/knfsolv-forecast-native.py`. Full screen-reader and other window-system audits were not performed.

## Score-wheel position editing — September 13, 2026

The routine suite passes 123 tests, with four extended checks ignored. `cargo fmt --check`, `cargo clippy --offline --all-targets -- -D warnings`, and `cargo build --offline --release` pass. The score-wheel tests were rerun after adding explicit stale-result rejection and completed-sheet reopening checks.

The rules helper's sorted score options are checked against all attainable dice scores and sheet validation across every category. GUI tests exercise both wheel directions, stop/wrap boundaries, individual Undo/Redo, pixel accumulation and its cell/direction/timeout resets, scroll-event consumption without smoothing-induced edits, modified-wheel exclusion, manual-dialog access and modal protection, inferred turns, dice retention/reset, inconsistent sheets, extra-Kniffel validation, stale worker updates, and reopening a completed sheet. Preference tests cover old-file defaults and persistence; GUI checks confirm the preference does not alter position history or saved context. Local historical editing retains ordinary wheel scrolling.

Native release screenshots were inspected in both themes at 1180×900, 800×600, and 360×300; fixed-width score buttons keep hover targets stable as values change. Headless interaction tests verify scrolling outside score cells, including the compact play area, in all six theme/size combinations. Native synthetic wheel events did not establish a compact scrolling interaction, so that behavior is verified by the automated GUI tests rather than claimed as a native mouse audit. Artifacts are `/tmp/knfsolv-wheel-native/` and `/tmp/knfsolv-wheel-native.py`. No game-file schema or solver behavior changed.
