# iPhone web app implementation plan

## Summary and defaults

Build a static, installable web app that reuses Knfsolv’s Rust engine and egui interface. The phone runs all calculations locally; hosting only serves application files. Development and builds run on Linux.

The first release includes local games, computer opponents, position analysis, physical/digital dice, JSON import/export, automatic recovery, and offline use.

Chosen defaults:

- Keep the current eframe version and WebGL renderer.
- Use Trunk to build separate interface and solver-worker Wasm binaries.
- Default browser coaching to **Limited: 1 second, 5,000 simulations, 10,000 nodes**. Keep Continuous available explicitly.
- Use the bundled continuation model. External model/cache files, training, cloud synchronization, accounts, and native iOS packaging are outside this release.
- Preserve existing desktop and CLI/TUI behavior.

Replace the exploratory content in `docs/iphone-web-app-brainstorm.md` with this implementation specification and milestone checklist.

## 1. Make the shared application browser-compatible

- Separate platform operations from shared rules, controller, solver, and presentation. Keep native CLI/TUI dependencies, filesystem access, thread creation, and desktop startup behind native target guards; conditionally derive Clap traits.
- Add browser startup through `eframe::WebRunner`, plus an HTML loading/error screen. Report unsupported rendering, failed downloads, and initialization errors with a retry action.
- Use `web-time` for shared timing and enable the browser randomness backend compatible with the existing dependency versions. Preserve serialized gameplay RNG state and deterministic Redo.
- Extract validated session decoding and serialization from filesystem methods. Browser exports retain the existing session JSON format, including its `desktop` metadata.
- Hide filesystem-path settings in browser builds. Imported native sessions use bundled browser models; explain ignored external model/cache paths before replacement.
- Add Trunk configuration for the interface and dedicated worker, with separate feature sets so the worker excludes graphical dependencies. Trunk supports worker assets directly. [Trunk asset documentation](https://github.com/trunk-rs/trunk/blob/main/guide/src/assets/index.md)

**Milestone:** Linux builds both Wasm targets; the browser opens the shared interface and completes a digital roll without invoking unsupported native APIs.

## 2. Run computation in a dedicated Web Worker

Preserve the existing worker interface—submit, controls, clear, and take updates—with native and browser implementations.

- Define serializable commands for initialization, submission, pause/resume, clearing, and report acknowledgement; responses carry readiness, progress, completion, or errors.
- Include build/protocol versions and a worker-instance identifier alongside the existing analysis key, revision, actor, and job purpose. Reject incompatible workers and stale responses.
- Send Rust-serialized JSON strings through `postMessage`; JavaScript transports them without parsing numeric fields, preserving 64-bit seeds and identifiers.
- Retain the existing two replaceable job slots and computer-decision priority. Keep one unacknowledged progress message per purpose, coalescing newer progress to prevent an accumulating message queue.
- Run initialization, baseline selection, exact attempts, and sampled search inside the worker. Advance searches in approximately 20 ms slices, yielding through timer tasks between slices; publish progress at most every 200 ms, plus completion.
- Preserve evidence across normal pause/resume. Existing exact searches remain bounded, single attempts; incomplete results keep their current honest labeling.
- Measure initialization and individual search steps: slice limits cannot interrupt a synchronous step. While visible, a five-second failure to acknowledge a control command triggers worker termination and an explicit retry state.
- Worker failure disables executable advice and pauses computer play while leaving manual play usable. Existing search failures may use the declared baseline inside a healthy worker; cancellation never produces a fallback move.
- On page hiding, pause computation and block computer actions. On return, reconcile worker state before accepting results and restart computer presentation delays. After full recovery, computer play waits for explicit Resume.

**Milestone:** Analysis never runs on the interface thread, and Undo, replacement, pause, or backgrounding cannot apply an obsolete move.

## 3. Add browser saving and iPhone controls

**Storage and recovery**

- Use IndexedDB for separate versioned preference and recovery records. Save each valid committed game revision promptly; debounce preferences by the existing 300 ms interval.
- Serialize writes and show “Saved on this device” only after the latest applicable transaction succeeds. Include gameplay RNG and computer configuration.
- Keep the last valid checkpoint during partial dice entry or inconsistent position editing. Display “Draft changes not saved”; commit completed corrections through the existing controller before export.
- On startup, offer **Resume saved game** or **New game**. Recovery restores the committed session with fresh analysis and empty undo/redo history.
- Protect the recovery record with a revision comparison inside its write transaction. A conflicting second tab stops automatic writes and offers to load the newer checkpoint or explicitly replace it; keep the current in-memory game available for export.
- Preserve corrupt or future-version records until explicit reset. Storage denial, quota failure, or transaction errors leave play available and show Retry and Export actions.
- Treat browser storage as device-local recovery; explain that clearing site data removes it.

**Import and export**

- Replace filesystem paths with **Import JSON** and **Export JSON**, using a file input and downloadable Blob.
- Reject files larger than 1 MiB before reading; parse and validate before changing the active session. Cancelled selection and invalid files leave the current game untouched.
- Guard asynchronous import completion against intervening game changes, and require confirmation before replacing active progress.
- Export prepared bytes from a direct user action. Report that a download was requested without claiming the browser saved it successfully.

**Phone interface**

- Below 600 logical pixels, use a single-column layout: turn/dice controls, compact advice, then the selected player’s sheet. Provide a player selector and a separate totals overview.
- Use touch targets of at least 44×44 logical pixels. Wrap toolbars and the dice palette; fit dialogs within the visible viewport.
- In position editing, tapping a cell opens an **Open / Zero / legal score** picker with Apply and Cancel. Preserve desktop wheel editing.
- Keep one-tap gameplay scoring and accessible Undo. Ensure dragging to scroll cannot also score or select a die.
- Handle safe-area insets, portrait/landscape changes, browser chrome, and the software keyboard. Essential explanations must be readable without hover.

**Milestone:** A complete game and an edited position can be created, recovered, imported, and exported using touch alone.

## 4. Package offline use and static hosting

- Add a web manifest, application icons, standalone display configuration, and brief iPhone Home Screen instructions.
- Generate a release-specific precache list containing every required HTML, JavaScript, Wasm, worker, font, and icon asset. Cache installation must succeed completely before showing “Available offline.”
- Use a service worker scoped to the deployment directory. Resolve all assets relative to the configured base path so both root hosting and `/knfsolv/` work.
- Keep updates waiting while an existing version is open; display “Update available—reopen to install.” Avoid forced activation or mid-game reloads. Retain caches needed by active clients.
- Keep recovery storage separate from application caches. An interrupted update must leave the previous offline version usable. Service workers require a secure context, so production hosting uses HTTPS. [Service worker lifecycle documentation](https://developer.mozilla.org/en-US/docs/Web/API/Service_Worker_API/Using_Service_Workers)
- Disable service-worker registration during normal development.
- Produce a deployable `dist/` and a manually triggered GitHub Pages workflow for the existing repository. Document Linux setup, development, release builds, and hosting configuration.

**Milestone:** After one successful online preparation, the installed app cold-starts in airplane mode and completes a game with analysis.

## 5. Verification and release criteria

- Run formatting, native tests and Clippy with and without GUI support, Wasm checks for both binaries, and a release Trunk build.
- Test worker serialization, large integer round-trips, job priority, pause/resume evidence, message coalescing, crashes, and stale responses after every invalidating action.
- Compare native/Wasm scoring, legal moves, RNG replay, and fixed-work solver fixtures; allow appropriate floating-point tolerance for estimates.
- Test imports, storage failures, conflicting tabs, corrupt records, incomplete drafts, reload recovery, and paused computer recovery.
- Automate browser checks in Chromium and WebKit for narrow layouts, touch input, storage, worker failure, offline startup, subpath hosting, and interrupted/versioned updates.
- Verify on a real iPhone in Safari and Home Screen mode: keyboard interaction, Files import/export, rotation, screen locking, app termination, airplane mode, and returning during computer analysis.
- Require responsive controls during analysis, no stale computer moves, no silently overwritten recovery, and no app-wide horizontal scrolling at 320–430 logical pixels.
- Run a 30-minute continuous-analysis soak and repeated position replacements; check that memory and message queues do not grow continually.
- Record tested device/browser versions, timing measurements, and accessibility limitations in the verification documentation. Linux WebKit automation alone does not complete the iPhone acceptance gate.
