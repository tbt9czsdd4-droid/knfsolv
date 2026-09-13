# GUI redesign checkpoints

## Stage 1 — Physical play interface

Implemented on September 12, 2026. Launch with `cargo run --release -- gui`, or load an existing session with `cargo run --release -- gui --load examples/middle.json`.

The toolbar contains physical-play/edit-position mode, the active player, Undo/Redo, File, and Settings. Dice and advice remain visible while score sheets scroll at 800×600 and larger. Advice moves beside the play area in wide windows; small windows have a scrollable fallback. Position and model controls are hidden in Settings. Filled scores can only be edited in Edit position, which disables playing moves.

Five palette clicks or number keys enter a roll automatically. Retained dice stay locked in their selected slots; rerolls need only the new faces. Clicking a completed die toggles Keep and selects it for correction. The palette replaces the selected face. Backspace clears the last editable die. Completed-roll corrections use a 250 ms debounce; pending corrections block scoring and analysis and immediately invalidate old worker results. Completing a roll or a correction group creates one undo step. Settings edits preserve partial input. Slot order and retained selections are restored by in-memory undo/redo; JSON continues to store the engine's unordered dice representation, so loading can reorder slots.

An available score cell commits its displayed points and advances in one click, including zero. Play suggestion directly applies the advised scoring/reroll action. Advice stars, Keep labels, Locked labels, and the correction outline have distinct meanings. Confidence and selection explanations remain visible, with the complete numerical report available in expanded details. Normal-play analysis follows the active player; position editing retains the configured analysis perspective.

Only discarding unsaved work prompts for confirmation. Incomplete dice drafts cannot be saved; Save asks the player to complete or reset them. A complete correction is committed before saving. Undo first resets any unfinished draft, then works through committed turns/settings on subsequent clicks.

### Verification and limits

Automated GUI coverage includes pointer events for palette entry, zero scoring and Undo; five number-key entry; locked duplicate dice; backspace; settings during partial entry; debounced correction groups and stale-result guards; one-click suggestions; and wide/compact advice layout. Rendering checks cover 360×300, 600×450, 800×600, 1180×900, and 1600×1100, including eight-player sheets and settings/edit/unsaved windows. Both light and dark themes retain opaque backgrounds.

The release application was launched in a native X11 window. Five directed number-key events entered `6 5 4 3 2` without confirmation; captures at 800×600 and 1180×900 verified the entered slot order, prospective scores, and stacked/side advice layouts. This does not establish complete native mouse gameplay or accessibility coverage. At this checkpoint analysis was manually started and bounded; Stage 3 below replaces that workflow.

## Stage 2 — Game controller and session metadata

Implemented on September 12, 2026. `src/game.rs` owns the session, physical-roll drafts, keep selection, correction debounce, actual digital rolls, event history, undo boundaries, and computer scheduling. It is available without the `gui` feature. The GUI reads state and issues controller commands; scoring and turn legality still come from the existing `Position` and rules APIs.

Launch with `cargo run --release -- gui`. New games still start with physical dice. To try digital play, open Settings, choose **Human dice → Digital**, close Settings, and press Roll. Keep selections use the same die buttons. Digital rerolls generate only unkept faces and commit the choice and outcome as one undoable operation. Physical entry and the Stage 1 interaction model remain available.

New GUI games save optional `desktop` metadata: player controllers, human dice source, advisor/computer configuration, and a separate gameplay RNG seed/word position. Old files have no metadata and open in Edit position; choose Play to continue their physical workflow. CLI/TUI analysis still uses the same `position` and `settings`. See [saved-session details](solver.md#saved-sessions).

Undo in a computer game returns to the previous human decision and removes subsequent automatic actions. Redo restores the complete recorded sequence and RNG state. Pausing the computer survives Undo/Redo. Loading and new game clear the in-memory event log and undo/redo history, even when the incoming session equals the current one. Every relevant change invalidates controller job tokens; a token identifies revision, active actor, and coaching/computer purpose.

The controller exposes nonblocking automatic roll/decision/apply jobs, a 400 ms presentation delay, fast play, and pause. Only current computer-decision results can queue computer actions. Cancelled or stale results do nothing. No worker or window is required to exercise these commands. The GUI does not yet run this automatic loop or offer opponent setup: those remain Stage 5, using the worker added in Stage 3. At this checkpoint advisor preferences were stored in preparation for Stage 3.

### Verification and limits

The complete routine suite passes 65 tests with GUI support and 52 without it, including 14 controller/integration tests and 13 GUI tests. Formatting, Clippy with warnings denied, and the release build pass. Controller tests finish two/eight-human physical and digital games, plus physical-human/digital-human versus computer sequences driven by supplied legal decisions. These verify game mechanics, not opponent policy strength. Additional tests cover partial drafts/settings, locked duplicates, debounce, RNG independence from solver work, saved RNG continuation, recorded redo, computer-turn save/load, human-decision Undo, pause/fast play, revision/purpose checks, and illegal-command rejection. GUI pointer tests cover digital Roll/Undo/Redo, and a legacy-load test rejects queued advice while clearing history. A shared-session test changes analysis settings through the TUI, saves without altering desktop metadata, and validates that file through the CLI binary.

Native smoke results are recorded in [verification](verification.md). Full native games, accessible navigation audits, actual opponent policies, and difficulty calibration remain later-stage work. Event and undo histories are intentionally memory-only; loading sorts the engine's dice histogram into slots and starts a new history.

## Stage 3 — Continuous background coaching

Implemented on September 12, 2026. Launch with `cargo run --release -- gui`. Enter five physical faces, or select digital dice in Settings and click Roll. Advice starts automatically for the active human once the roll is valid, first with a clearly provisional baseline, then with updated estimates approximately every 250 ms. Invalid positions and incomplete rolls wait; changes invalidate old results.

The persistent advice panel offers **Pause analysis / Resume analysis** and **Continuous / Limited**. Pausing retains completed evidence, unfinished simulated games, and paired rounds. Limited uses the saved time/sample limits (defaults: two seconds / 20,000 games), excluding paused time. Switching to Continuous extends the same analysis. Position/model/search-setting changes begin a fresh search. Depth and node controls remain in Settings. An exact answer stops automatically.

Default Auto coaching uses fixed-policy rollouts after a short exact attempt in small endgames. Explicit MCTS has a bounded initial training phase, at most 500 ms or 2,000 training games, then freezes the policy for continuing validation. Confidence and provisional-baseline explanations remain explicit. Explicit Exact stops after one bounded attempt and may report incomplete. See [solver details](solver.md#incremental-desktop-coaching) for budgets, cache limits, and the distinction from bounded CLI/TUI Auto.

One persistent computation thread replaces per-request threads. Latest requests and reports replace old ones, with revision/actor/purpose/serial guards. Computer-decision requests have priority over retained coaching; this is worker infrastructure for Stage 5, with no automatic computer moves added here. Advisor mode/pause preferences are saved for desktop games; search evidence stays in memory and loading begins fresh.

### Verification and limits

The full routine suite passes 81 tests with GUI support and 66 without it. Nine incremental-search tests cover deterministic pause/resume, unfinished games and paired rounds, limited-to-continuous extension, paused-time accounting, exact success/fallback, frozen MCTS training, cancellation, and cache eviction without changing policy. Five worker tests cover replacement, priority, retained state, bounded report storage, errors, and shutdown. Fifteen GUI tests include automatic coaching, pause/resume controls, invalid/finished positions, and prior gameplay/layout regressions. Formatting, Clippy, and the release build pass.

Native X11 captures at 800×600 and 1180×900 confirm five-key automatic entry and growing analysis counts with dice and advice visible. Native pointer clicks also verified Pause/Resume: repeated paused captures retained 7,019 games / 2.92 seconds, then Resume advanced to 9,137 games. Full native games and accessibility audits remain Stage 6. The worker checks cancellation between simulation steps; an individual policy calculation can overrun a slice. Exact attempts are bounded but their partial trees are not resumable. No new opponent-strength evaluation or calibrated difficulty claim is included.

## Stage 4 — Local-game setup and results

Launch `cargo run --release -- gui`. The start screen offers Local game, Analyze a position, and Load session. Play computer is present but disabled until Stage 5. `--players N` preselects the setup count; `--load path.json` opens that file directly. Local games support 2–8 humans, defaulting to two. Set names, the starting player, and physical or digital dice before clicking Start local game. Turn order starts at the selected seat and wraps through the listed players. Position analysis retains its separate 1–8-player editor.

`LocalGameConfig` builds a validated session with empty sheets, named players, rotated turn order, desktop metadata, and an independent gameplay seed. Names cannot be blank. Human digital turns always wait for Roll. Five physical faces commit automatically, and scoring remains one click. Inactive players remain in the shared score grid, with horizontal scrolling when needed. The score rows and dice-action area now retain their height as cells fill and turns change.

Finished games show totals, upper subtotals and bonuses, extra Kniffel points, and every tied winner. Results use the existing sheet calculations and appear automatically after the last score. Review score sheets and Show results switch between the summary and complete sheets. Undo restores the final decision; Redo restores results. Loading a finished desktop game also opens its results.

File → New game and the results New game button return to setup without discarding the session. Return to session preserves dice drafts and resumes coaching. Navigating away pauses the existing computation request without changing the saved advisor preference. The unsaved-work prompt appears only when starting a replacement game/editor or loading another file. Cancel keeps the current session; accepted replacement clears history and stale work. A fresh start screen can close without an unsaved-placeholder prompt.

### Verification and limits

The routine suite passes 89 tests with GUI support and 69 without it, including 20 GUI tests and 17 controller integration tests. New coverage includes names and starting-player selection through pointer controls, text-entry isolation, fresh startup, local/editor/load routes, invalid loads, replacement cancellation with partial dice, all starting seats for 2–8 players, saved setup, complete physical/digital matches, bonuses and tied winners, final-score Undo/Redo, and results review. Setup/results rendering covers dark/light themes at 360×300, 800×600, and 1180×900. Eight-player setup fits at 800×600; smaller windows scroll. Existing GUI keyboard, coaching, persistence, exact/probability, and TUI regressions remain passing. Formatting, Clippy, and the release build pass.

Native X11 verification completed two full two-player games from empty sheets through all 26 score clicks: physical entry ended in a correctly displayed 220–220 tie (50 extra Kniffel points each), and digital Roll ended with a single winner (52–36). Setup was inspected at 800×600 and gameplay/results at 1180×900. The native check prompted the stable-row fix, now covered by GUI regression assertions. Details are recorded in [verification](verification.md). Computer play, broad accessibility audits, and difficulty calibration remain later stages. There is still a session-path field rather than an operating-system file picker; setup choices and final sheets use the existing JSON format, while undo history and search evidence remain memory-only.

## Stage 5 — Computer opponents

Launch `cargo run --release -- gui` and choose Play computer. Configure your name, the computer name, who starts, your physical/digital dice, and a difficulty preset. Easy scores immediately with Greedy; Standard uses the retained legacy opportunity policy; Strong uses continuation-v2. Computer rolls always come from the independent gameplay RNG. The start-screen default is Standard. Local games and position analysis remain separate options.

Custom search uses continuation-v2 with bounded MCTS, initially two seconds / 20,000 simulations and conservative advice. Its setup exposes time, simulations, depth, and nodes. It is explicitly uncalibrated, with no promise that larger budgets mean stronger play. Coaching follows the human and uses the declared computer baseline; the custom-search approximation is stated in the advice panel. Preset computer games use bundled continuation evaluation regardless of advisor model/cache paths.

The existing controller now drives the native loop: generate the actual roll, request a worker decision, show chosen keeps/scoring, then apply it. Normal actions have a nonblocking 400 ms presentation delay. Pause computer / Resume computer and Fast play are available in the toolbar. Human digital dice still wait for Roll. Undo returns to the previous human decision and removes subsequent automatic actions; Redo restores recorded outcomes. Save/load during computer turns preserves dice/RNG/configuration and pause state, clears in-memory history, and computes fresh decisions when resumed.

Only finished computer-controller reports carrying the current revision, actor, purpose, and request serial may enqueue moves. Provisional coaching/search reports never execute computer actions. Pause, Undo, load/new game, edit mode, setup navigation, and settings/file popups prevent stale moves. A genuine search error or completed search with no advice requests the legal baseline on the persistent worker and displays why. Cancellation never invokes fallback. A failed baseline pauses the computer. Ordinary worker panics become error reports; the large continuation lookup table is now heap-owned to keep debug worker stacks bounded.

### Verification and limits

Adjacent presets passed the required held-out ordering gate over 1,000 independent paired seeds per comparison, each replayed with swapped seats (2,000 matches per comparison). Standard over Easy: +111.349 mean points and 0.9900 win credit, simultaneous 95% Hoeffding interval [0.9432, 1.0000]. Strong over Standard: +16.297 points and 0.6110 win credit, interval [0.5642, 0.6578]. Both comparisons have positive mean score differences and intervals above parity. Custom search remains uncalibrated. See [verification](verification.md) for reproduction and the precise confidence construction.

The routine suite passes 97 tests with GUI support and 72 without it. It covers complete computer games with all three presets and a small custom search budget, each with physical and digital human dice; legal baseline worker decisions; explicit legacy-policy reproduction; separate coaching/computer settings; provisional-report rejection; failure/budget fallback; 400 ms timing; pause, Undo, load and navigation cancellation; computer-turn save/reload; and prior local-game, solver, persistence, and TUI regressions. Native physical-human and fully digital-human games against Standard both finished successfully after pause, save, Undo, and reload during computer turns; finished files passed CLI validation. Interaction results are recorded in [verification](verification.md).

Remaining limits include broad manual accessibility/navigation audits, extended memory/concurrency profiling, and a final release pass. Computer difficulty is selected during new-game setup. Event history, pending decisions, and accumulated search evidence remain memory-only. No online multiplayer or rules changes are included.

## Stage 6 — release verification

Added an opt-in Linux worker soak in `tests/release_checks.rs`: one minute each of continuous fixed-policy rollout and frozen-policy MCTS, four players, a 128-node cap, and deliberately slow report consumption. It checks monotonic completed samples, bounded report slots, one persistent worker thread, bounded RSS growth after warmup, and shutdown within two seconds. Existing deterministic tests cover exact sample accounting, incomplete paired rounds, pause/resume, cache eviction, training freeze, priority, and stale-result rejection.

Native X11 checks verified Tab/Enter from Home into local setup, five number keys committing dice, and Right/Space retaining a die with a textual Keep indicator. Both dark and light gameplay keep dice and advice visible at 800×600. At 360×300, native wheel scrolling reaches the final categories and totals. The Stage 4 and Stage 5 complete native games remain the full-game interaction evidence; those same application sources passed the final release regression suite.

Launch with `cargo run --release -- gui`. Reproduce the extended check on Linux with `cargo test --offline --release --test release_checks -- --ignored --nocapture`; run it alone because it measures process-wide thread counts. See [verification results](verification.md) for actual commands, counts, and resource measurements.

All six implementation checkpoints are delivered. Remaining verification limits: native tests cover Linux X11, not Wayland, Windows, or macOS; no screen-reader audit was performed. The two-minute soak is a bounded resource check, not proof of indefinite stability. Custom search strength remains uncalibrated, and partially completed exact attempts do not resume their search trees. Standard rules and CLI/TUI session compatibility are unchanged.


## Expanded advice cards

The advice panel renders structured reports as a bordered **Recommended** card followed by three alternatives. **Show all moves / Show fewer** preserves solver order after putting the actual recommendation first, including when the sampled leader differs. The recommendation offers **Play suggestion**; alternatives now offer **Play this move** (see the gameplay refresh below). Scoring titles include category and immediate points (including zero and extra Kniffel points); rerolls show small retained dice and the reroll count.

Every card shows Win, Tie, Lose to one decimal place and expected **final game score**, distinct from immediate points. Win is outright first; Tie is tied for first; Lose is the remaining probability. Hovering the labels explains their meanings. Unavailable metrics say **Estimating…**. The compact controls and status retain paused, provisional baseline, incomplete, and exact-under-model distinctions. **Analysis details**, collapsed initially, contains selection reasoning, models, objective-specific sampling intervals, paired comparisons, counts, budgets, and stop reasons. Shared-first utility intervals are never labeled as win-probability intervals. Disclosures persist across report updates.

Theme-aware cards wrap in narrow windows. At 800×600 the compact advice viewport keeps the recommendation, statistics, and play action below the dice; alternatives scroll underneath. Wide windows retain the side panel. Smaller windows use one vertical scroll for dice, advice, and sheets.

Verification: 100 routine tests pass, including 28 GUI tests. New coverage checks default cards, both disclosures and report replacement, solver ordering with a different sampled leader, missing metrics, ties, scoring/zero entries, retained/empty keeps, percentage arithmetic, objective labels, status distinctions, and clipping in both themes. Formatting, Clippy with warnings denied, and the release build pass. Native X11 inspection covered dark/light 1180×900, 800×600, and 360×300 layouts, pointer disclosures, Tab focus, and scrolling to final sheet totals. These are directed smoke checks, not a screen-reader audit. No solver, persistence, CLI, or TUI behavior changed.


## Gameplay styling and playable alternatives — September 13, 2026

The in-game toolbar, dice area, and score sheet now share rounded, theme-aware surfaces with the advice cards. Session controls, history, and computer controls are visually grouped. The active player and rerolls remaining appear above the dice. A shared egui pip renderer supplies gameplay dice, face-picker buttons, and small retained dice in every engine card. Text states distinguish Keep, Locked, and Advice; hover, keyboard focus, and correction selection have outlines. Pip controls retain accessible numeric labels and keyboard activation.

Roll and Reroll N dice are primary buttons with a minimum 160×44 target. The physical picker is hidden during digital play and available in position editing. The score grid uses stable 26-pixel minimum rows, aligned columns, an active-player header highlight, and stronger totals. Wide windows keep advice beside play. At 800×600, a 180-pixel advice viewport keeps the recommendation and its action visible while leaving space for the score sheet; alternatives and sheets scroll independently. Small windows scroll the whole play area.

Every alternative has a Play this move button, including expanded and unestimated moves. It executes that card's legal action through the existing controller: immediate scoring, generated digital rerolls, or locked physical keeps awaiting new faces. Manual keep selections do not override the card. Undo/Redo, editing/computer-turn restrictions, pending-input guards, and report invalidation remain intact. Globally stable action-based card IDs keep keyboard focus on the same move when estimates reorder the alternatives. The recommended card keeps Play suggestion and its distinct border; report ordering and disclosures are preserved.

Verification adds six GUI tests for alternative scoring/zero/expanded moves, physical and digital keeps with duplicates and Undo/Redo, disabled actions, all six pip patterns with accessible labels and keyboard activation, large Roll targets and picker visibility, and focus across report reordering. Existing complete-game and layout tests remain in the suite. No solver, controller, persistence, or CLI/TUI interfaces changed.
