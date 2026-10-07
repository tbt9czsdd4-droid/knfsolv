# iPhone web app plan

Status: Phases 0–4 done (2026-10-07; Pages deployment postponed; the family test of Phase 4 is still open); Phase 5 in progress (step 1 of 5 done). This replaces the earlier draft (commit `edcef33`).
Sources: [the brainstorm](iphone-web-app-brainstorm.md), code inspection, and measurements taken on 2026-10-04 (section 3).

## 1. Goal

Family and friends open a link on their iPhone and add the app to the Home Screen. At the table, they copy the paper score sheets into it, tap in the current dice, and see the best move with its chance of winning. The app is in German, works offline, and needs no account or setup. The phone does all the computing; GitHub Pages only serves static files. Development stays on Linux. The desktop app, CLI, and TUI keep working as they do today, apart from a few shared improvements noted in D3 and D5.

### Decisions already made

| Question | Answer | What it means for the plan |
| --- | --- | --- |
| Main use on the phone | **Analyze positions** | The position editor gets the phone-first design. Local games and games against the computer stay available and reuse the same phone components, with less dedicated polish. |
| Audience | **Me + family/friends** | No jargon, no setup, no manual saving. Installing must be explained inside the app. Nothing may require reading documentation. |
| Language | **German** | The web app is German. The desktop app stays English (decision D6). |
| Hosting | **GitHub Pages** | Static deployment from GitHub Actions. The free plan requires a public repository; see D8 for a way to keep the source private. Deployment is postponed for now (D8). |
| App name | **Kniffel-Coach** | Home Screen name and page title; the Pages path would be `/kniffel-coach/`. |

## 2. What users get

### Installing (once)

1. Patrick sends the link, or the QR code from the help page.
2. In Safari, a banner explains: *„Tippe auf Teilen ⬆︎ und dann ‚Zum Home-Bildschirm‘. Dann startet die App im Vollbild und funktioniert auch offline.“*
3. From then on, the Home Screen icon opens the app full screen, with or without internet.

### First start

The first start shows a one-screen welcome, then asks *„Wer spielt mit?“*: the number of players (1–8), listed in playing order starting with whoever began. Names are optional and default to „Ich“, „Spieler 2“, and so on. The analyzer then opens with empty sheets.

- **One player:** the advice maximizes points. Today every move shows a 100 % win chance and only the engine's expected-score tie-break picks the move. The phone UI should select the expected-score objective explicitly and show „Ø Endpunkte“ instead.
- **Several players:** the advice maximizes the chance of winning. This is the engine's strength.

### Every later use

The app opens exactly where it was left: same players, same sheets.

1. **Update the sheets.** Tap a cell, then tap its value: two taps per entry (see the picker below).
2. **Choose who is rolling and which roll this is.** The app works out whose turn it is from the number of entries, as the editor already does. Tap **1. / 2. / 3. Wurf**.
3. **Enter the dice.** Tap five faces on the 1–6 palette. Advice appears at once and refines for a few seconds.
4. **Optional: „Übernehmen“.**
   - For a reroll tip, the kept dice are locked and the roll advances, so only the new dice need entering.
   - For a scoring tip, the points are entered and the next player is up.

   With this, the app's sheets stay in sync with the paper block without switching to a separate “play” mode.

### Phone analyzer screen (portrait, about 390 points wide)

```text
┌──────────────────────────────────────────┐
│ ☰  Kniffel-Coach               ↶    ↷    │  menu · undo · redo (44-pt targets)
├──────────────────────────────────────────┤
│ Anna ist dran           [1.][2.][3.] Wurf│
│  [6] [6] [6] [2] [3]                     │  tap a die: select / keep
│  (1)(2)(3)(4)(5)(6)          [⌫]         │  face palette
├──────────────────────────────────────────┤
│ ★ Behalte 6 6 6 · würfle 2 neu           │  recommendation card
│   Gewinnchance 58 % · Ø 241 Punkte       │
│   [ Übernehmen ]       Andere Züge ▾     │
│   ✓ fertig · knappe Sache: 2. Zug fast   │
│     gleich gut                           │
├──────────────────────────────────────────┤
│              Ich    Anna    Ben          │  tap a name: name, Zusatz-Kniffel
│ 1er           3      –       2           │  tap a cell: picker
│ 2er           –      6       –           │
│ …                                        │
│ Bonus      ab 63:  48/63   …             │
│ Endsumme     112     98     120          │
└──────────────────────────────────────────┘
```

- **One vertical scroll** holds dice, advice, and sheet; the top bar stays fixed.
- **Three or four players fit**, depending on Schriftgröße, using abbreviated row names (1er … 6er, 3er-Pasch, 4er-Pasch, Full H., Kl. Str., Gr. Str., Kniffel, Chance). With more players, the player columns scroll sideways while the row names stay put.
- **The layout must still work at about 310 points wide.** A 390-point iPhone shows that much with Schriftgröße Groß (egui zoom factor 1.25).

### Score picker

```text
┌──────────────────────────────────┐
│ Anna · Dreierpasch               │
│ [ 18 mit aktuellem Wurf ]        │  only when it applies
│ [ Offen ]   [ Streichen (0) ]    │
│  5   6   7   8   9  10           │
│ 11  12  13  14  15  16           │  legal values only
│ 17  18  19  20  21  22           │  (rules::category_scores)
│ 23  24  25  26  27  28           │
│ 29  30                           │
│              [ Abbrechen ]       │
└──────────────────────────────────┘
```

- One tap applies the value and closes the picker; **Undo** fixes mistakes. There is no extra Apply step.
- The wheel isn't needed: it doesn't exist on touch screens. Typing a value isn't needed either: illegal values can't be chosen.
- The desktop should get the same picker on click, replacing the typing dialog and keeping the mouse wheel. That leaves one code path and benefits iPad (D5).

### Advice wording (German, plain)

| Engine result | Phone text |
| --- | --- |
| Reroll, keeping dice | „Behalte 6 6 6 · würfle 2 neu“ (with small dice) |
| Reroll everything | „Alle 5 Würfel neu würfeln“ |
| Score | „Dreierpasch eintragen: 18 Punkte“ (+ „inkl. 50 Zusatzpunkte“) |
| Score zero | „Kniffel streichen (0 Punkte)“ |
| Before the roll | „Prognose vor dem Wurf: Gewinnchance 42 %“ |
| Shared-first utility | „Gewinnchance 58 %“; win/tie split only in the details |
| Expected final score | „Ø 241 Punkte“ |
| Exact result | „✓ exakt berechnet“ |
| Estimate, limit reached | „✓ fertig · 6 400 Partien simuliert“ + **Länger rechnen** |
| Best two within 1 percentage point | „knappe Sache: Andere Züge sind fast gleich gut“ |
| Provisional baseline | „vorläufig, rechnet noch …“ |

The engine's labels („Tree depth“, „validating“, intervals, models, budgets) move into a collapsed **Technische Details** section. Numbers use German formatting (`58,3 %`).

### Other screens

- **☰ menu:**
  - Neue Stellung (gleiche Spieler)
  - Spieler ändern …
  - Spielen: gegen den Computer / mit App-Würfeln
  - Darstellung: Hell / Dunkel / System
  - Schriftgröße: Normal / Groß
  - Rechenzeit: Schnell / Gründlich / Ohne Limit
  - Stellung teilen / öffnen
  - Hilfe & Regeln
  - Über die App (version, „App teilen“, diagnostics)
  - Erweitert (the existing engine settings)
- **Play modes** (local game and computer opponent) reuse the dice panel, advice card, and sheet. In those modes, tapping an available cell scores it, as on the desktop.
- **Inconsistent sheets** should help instead of blocking:
  - The message reads: „Die Anzahl der Einträge passt nicht zur Reihenfolge – fehlt irgendwo ein Eintrag?“
  - The players whose counts don't fit are highlighted.
  - A way forward is offered: „Trotzdem analysieren – wer ist dran?“, with one button per player. Choosing one switches to the existing hypothetical-position mode.

## 3. What we already know

The checks below ran in a scratch copy, so the repository is unchanged.

**Compilation.** The engine library compiles for `wasm32-unknown-unknown` after four small changes:
- enable `getrandom`'s `js` feature;
- make `clap`, `crossterm`, `ratatui`, and `tempfile` native-only, deriving `clap::ValueEnum` via `cfg_attr`;
- gate `mod tui`;
- gate the file-based `save_json`.

With those changes, the **entire egui GUI compiles too**. The only error was the missing `eframe::run_native` on the web.

**Runtime blockers.** These compile but fail when run in a browser:

| Code | Problem in the browser |
| --- | --- |
| `coaching::ComputationWorker` spawns a `std::thread` | Fails, and `Desktop::new` would panic at startup |
| `std::time::Instant::now()` (solver, game, preferences, GUI) | Panics |
| Session/preferences file I/O, `preferences::path()` env vars | Not available |
| `catch_unwind` around solver work | A panic stops the whole Wasm instance; there is nothing to catch |

**Speed.** Fixed 5,000-sample coaching runs (`AnalysisSession`, 40 ms slices, bundled continuation model):

| Position | Native release | Wasm (WASI build on Node 22 / V8) | Wasm relative speed |
| --- | --- | --- | --- |
| Opening `1 1 2 5 5`, 2 players | 2.12 s | 2.70 s | 0.79× |
| `examples/middle.json` | 0.13 s | 0.25 s | 0.52× (short run, JIT warm-up dominates) |
| Opening, 4 players | 3.09 s | 3.86 s | 0.80× |

- Recommendations were identical across native and Wasm, so the engine stays deterministic.
- With 8 ms slices, the worst slice took 8.16 ms. Individual solver steps are small, so cooperative scheduling works.
- **On the iPhone** (16 Pro, iOS 27, `?speed` test, solver in a Web Worker): 1,898 games per second over HTTPS, the same as WebKit on the development PC (1,823) and native code. Speed is not a blocker.
- **Only over HTTPS.** iOS 27 renders plain-HTTP pages from LAN addresses in a hardened mode without the JavaScript and WebAssembly compiler (loopback is exempt). Over `http://<pc>:8080` the same phone managed 51 games per second, and every check was 15–45 times slower. GitHub Pages serves HTTPS; for development, `web/serve-lan.sh` serves the build with a self-signed certificate.

**Exact endgames.** A two-open-category endgame is solved exactly in about 150 ms natively (33,701 nodes). Under a 40 ms budget, it comes back `incomplete`. Today the coaching worker caps each exact attempt at one 40 ms slice, so such endgames fall back to estimates even on the desktop. On a phone, the exact attempt needs roughly 0.2–0.4 s. That is a strong argument for a background worker (D3).

**Download size.** GUI plus engine is 5.4 MB raw and **1.9 MB gzipped**, measured before `wasm-opt`, so the shipped size will be smaller. The engine alone (for the worker) is about 0.66 MB raw. It is a one-time download, after which the app is cached offline.

## 4. Decisions, with pros and cons

### D1 — Web app (PWA), not a native iOS app or a server

| Option | Pros | Cons |
| --- | --- | --- |
| **PWA, solver on the phone** ✅ | No Mac or App Store; one link for the family; free hosting; instant updates; offline; same Rust code | Installation is manual (Share → Zum Home-Bildschirm); iOS PWA limits (no background work, storage separate from Safari) |
| Native iOS app | Native feel, App Store install | Needs macOS/Xcode (or a rented Mac) and $99/year to give it to family; free signing expires after 7 days; review process |
| Server-hosted solver | Works on any device | Needs a running server, always online, upkeep; unnecessary because the phone is fast enough (section 3) |

### D2 — Reuse the egui interface; don't rewrite it in HTML

| Option | Pros | Cons |
| --- | --- | --- |
| **egui in the browser** ✅ | Reuses `gui.rs` (4,600 lines), the game controller, and 41 headless GUI tests. One UI for desktop and phone. Phone layouts can be tested natively with `cargo test`. Fastest path. | Canvas rendering: no VoiceOver, no iOS text-size setting (an in-app *Schriftgröße* compensates), iOS keyboard handling is imperfect, the look is not native iOS |
| HTML/TypeScript UI + Wasm engine | Native scrolling, keyboard, accessibility; smaller download | Rewrite of the whole GUI and controller wiring; two UIs to maintain; roughly two to three times the effort |

**Kill criterion:** Phase 0 tests egui on a real iPhone. If tapping, scrolling, or name entry turn out unreliable, only the analyzer screen gets rebuilt in HTML. The engine, worker, storage, and PWA work in this plan stays valid either way.

### D3 — Run the solver in a Web Worker

| Option | Pros | Cons |
| --- | --- | --- |
| **Dedicated Web Worker** ✅ | UI never blocks. Exact endgame attempts can use their full ~0.5 s budget. A solver panic only kills the worker, which restarts automatically. Full CPU throughput. | A second small Wasm binary, a JSON message protocol, restart handling |
| Slices on the UI thread | Simplest; measured slices stay tight (8.16 ms worst) | Exact endgames capped to a few ms (worse answers in exactly the positions people ask about); any panic freezes the app; roughly 50–70 % throughput; scrolling competes with analysis |
| Wasm threads (SharedArrayBuffer) | Closest to today's code | Needs special COOP/COEP headers that GitHub Pages cannot send, plus a nightly toolchain |
| Server | — | See D1 |

**Approach:**
- Extract the scheduling logic from `coaching::run` into a platform-neutral core: two job slots, priority, publish every 250 ms. The native thread and the web worker both drive it, so desktop behavior stays as it is and is covered by the existing worker tests.
- Phase 0 uses the UI-thread variant as a throwaway stepping stone.
- While at it, let the worker give the exact attempt up to 500 ms instead of one slice. This fixes the endgame fallback on desktop too.

### D4 — Save automatically (localStorage), no files

| Option | Pros | Cons |
| --- | --- | --- |
| **Autosave to `localStorage`** in the existing session JSON ✅ | Nothing to learn; survives force-quit, reboot, and updates; synchronous API fits egui; data is a few KB | 5 MB limit (irrelevant); Safari storage may be cleared for sites not visited for a while; Home Screen apps are treated separately |
| IndexedDB | Larger, transactional | Async API with more code and more failure states, for no benefit at this data size |
| eframe's built-in persistence | Little code | Saves every 30 s in its own format, with less control over timing and errors |
| Manual Save/Load as on desktop | Already exists | File paths make no sense on iOS; family users would lose positions |

**Details:**
- Save each committed change about 300 ms later, and immediately when the page becomes hidden. iOS kills backgrounded apps without warning.
- Open the last position on start.
- Partial dice entry and undo history stay in memory, as on the desktop.
- **Stellung teilen / öffnen** uses the same JSON as desktop saves, so positions move between the PC and the phone. Export goes through the iOS share sheet, falling back to a download. Import uses the Files picker; files over 1 MiB are rejected, and the file is validated before it replaces anything.
- Replacing a position that has entries asks once: „Aktuelle Stellung verwerfen?“
- Two things to tell users:
  - Safari and the Home Screen app have **separate storage**, so a position entered in Safari does not appear in the installed app.
  - The app should be used from the Home Screen.

### D5 — A dedicated phone layout below 600 points wide

- Below 600 points, the app uses the layout in section 2. At 600 points and wider, today's layouts stay unchanged.
- That covers iPad and wide desktop windows, which get the shared touch improvements:
  - the score picker;
  - no information that only appears on hover;
  - 44-point targets for primary controls.
- Squeezing the desktop layout onto a phone would keep jargon, hover help, and wheel editing, all of which fail on touch screens.

### D6 — German through a translation table; the desktop stays English

| Option | Pros | Cons |
| --- | --- | --- |
| **Small `t("English text")` table with German entries; web defaults to German, desktop to English** ✅ | English source stays readable; the 41 GUI tests that click by English label keep working; desktop unchanged; a language switch becomes almost free | Every UI string goes through `t()`; formatted strings need placeholders |
| Translate strings in place | No lookup layer | Desktop becomes German too; most GUI tests and the README need rewriting |

- `gui.rs` has roughly 250–300 distinct UI strings.
- A test fails when a string used with `t()` has no German entry.
- Category names follow the rulebook: Einser … Sechser, Dreierpasch, Viererpasch, Full House, Kleine/Große Straße, Kniffel, Chance; zero is „streichen“.
- Engine messages that users can actually trigger get German UI wording. Rare internal errors show a German sentence plus the original text under *Technische Details*.

### D7 — Offline and updates: simple and safe

- A small hand-written service worker precaches every file of a build in a cache named after that build's version. It serves cache-first and keeps the old version until the new one is fully cached.
- A new version is used on the next app start. If one is waiting while the app is open, a small banner offers „Neue Version – tippen zum Aktualisieren“.
- Because of autosave, reloading loses nothing. That removes the need for the complex “wait until all tabs close” logic of the earlier draft.
- During development the service worker is off.
- Service workers need HTTPS; GitHub Pages provides it.

### D8 — GitHub Pages deployment

A GitHub Actions workflow builds with `trunk build --release --public-url /<repository>/` and deploys to Pages. The free plan serves Pages only from **public** repositories. The repository currently contains the Schmidt rulebook PDF, which is copyrighted.

| Option | Notes |
| --- | --- |
| **Keep the source private; deploy the built `dist/` to a separate public repository** (recommended) | Only the compiled app is public. Needs one access token as an Actions secret. |
| Make this repository public | Simplest, but first remove the PDF; it would also stay reachable in git history unless history is rewritten |
| GitHub Pro | Pages from the private repository, about $4/month |

The resulting URL contains the GitHub user name. That is fine with a QR code; a custom domain is optional.

**Decision (2026-10-05): postponed.** Phase 1 adds the checks workflow only. Development and phone tests use `web/serve-lan.sh` until one of the options above is chosen; it has to be settled before the family uses the app.

## 5. Architecture

```text
Linux PC ──git push──▶ GitHub Actions ──▶ GitHub Pages (static HTTPS files)
                                                │ first visit, online
                                                ▼
iPhone · Home Screen app
  service worker ── serves the cached app files (offline after first start)
  main thread ──── egui UI (app.wasm) ──JSON messages──▶ Web Worker: solver (worker.wasm)
  localStorage ─── current position + preferences
```

| Area | Change |
| --- | --- |
| `Cargo.toml` | Native-only `clap`/`crossterm`/`ratatui`/`tempfile`; `getrandom` `js` on Wasm; `web-time`; workspace member `web/` |
| `solver/`, `game.rs`, `coaching.rs`, `preferences.rs`, `gui` | `web_time::Instant` (re-exports `std` on native, so the API is unchanged) |
| `coaching.rs` | Extract the scheduler core; add a `Computation` trait. The native thread implementation keeps today's behavior. Serde derives on requests/updates/keys |
| `persistence.rs`, `preferences.rs` | Separate parse/serialize from file access; add a storage trait (files on native, `localStorage` on web) |
| `gui.rs` → `src/gui/` | Split the 4,600-line file: shared state, `phone.rs` layout, widgets (dice, picker), `i18n.rs`. Desktop layouts stay as they are |
| `game.rs` | Editor commands: apply advice („Übernehmen“); keep entered dice when an edit doesn't change who is rolling (today any change of entry counts clears them) |
| `web/` (new crate) | App entry (`eframe::WebRunner`), worker entry, browser glue (storage, share/import, visibility, install hint), `index.html`, CSS (safe areas, no zoom, `100dvh`), `sw.js`, manifest, icons, `Trunk.toml` |
| `.github/workflows/` | Checks and Pages deployment |
| Docs | README section, German user guide, verification notes |

**Worker protocol:**
- Messages are JSON strings serialized by Rust on both ends, so JavaScript never parses the 64-bit seeds and keys.
- Main → worker: `Submit`, `Controls`, `Clear`. Worker → main: `Ready{version}`, `Update`.
- The existing revision/actor/purpose/serial keys keep rejecting stale results.
- The main thread remembers the latest requests. If the worker dies, it starts a new one and resubmits them; after three failures within a minute it offers „Neu laden“.
- When the page is hidden, analysis and computer play pause.

## 6. Phases

Estimates are focused working days. Each phase ends with something testable on the iPhone, because deployment is set up early.

### Phase 0 — Device check (1–2 days)

**Goal:** answer “does egui feel good on an iPhone, and how fast is the solver there?” before investing further.

**Tasks:**
- Apply the minimal compile changes from section 3.
- Add a web entry point.
- Run analysis on the UI thread (temporary).
- Start `web/serve-lan.sh` and open `https://<pc>:8443` on the iPhone over Wi-Fi. Plain HTTP also loads, but iOS then runs the page without its compiler (section 3).

**Check:**
- Tapping dice, palette, and cells; scrolling the sheet; typing a name; rotation; notch and home-bar areas; readability.
- Run a fixed benchmark position and note games per second.

**Done when:** there is a go/no-go for D2 and measured numbers for the default thinking time.

**Status (2026-10-05): done. Device check on an iPhone 16 Pro (iOS 27); go for D2.**
- `web/` contains the browser entry point. `?bench` opens the fixed opening `1 1 2 5 5` in the analyzer; `?speed` runs timing checks and solver games in a Web Worker without the interface.
- On the web, `coaching::ComputationWorker` advances 8 ms slices whenever the GUI polls, using the same `step` function the native thread now loops over.
- Saving files is unavailable in the browser, and preferences last for the visit. That changes in Phase 3.
- **Works on the iPhone:** loading, light/dark switching, tapping dice, palette, and cells, touch scrolling (also during analysis), rotation, notch and home-bar areas, readability.
- **Speed:** native speed over HTTPS, 35 times slower over plain HTTP (section 3). Development uses `web/serve-lan.sh`.
- **Name entry needed two taps.** egui often focuses a text field only after the tap, too late for iOS to open the keyboard ([egui#4569](https://github.com/emilk/egui/issues/4569)). `web/src/keyboard.rs` focuses eframe's input inside `touchend` instead. Confirmed on the iPhone: one tap opens the keyboard.
- **Start screen is cramped** at phone width (fixed desktop widths); the phone layout in Phase 4 replaces it.
- **Once the app reopened on the home screen** after being in the background: iOS discards background pages. Autosave in Phase 3 restores the position.
- Native tests, Clippy (with and without `gui`), and the Wasm Clippy build pass.

### Phase 1 — Browser build foundation and deployment (2–3 days)

**Tasks:**
- Make the dependency and `Instant` changes properly.
- Add the storage and computation traits, with native implementations unchanged.
- Create the `web/` crate with a loading screen and an error overlay. A panic shows „Etwas ist schiefgelaufen – Neu laden“; the autosaved position survives.
- Add a GitHub Actions workflow:
  - checks: `cargo fmt`, Clippy and tests with and without `gui`, Wasm `cargo check`, `trunk build --release`;
  - deployment to Pages (D8).

**Done when:**
- All existing tests pass and the desktop, CLI, and TUI behave as before.
- The Pages URL opens on the iPhone and completes an analysis.

**Status (2026-10-05): done, except the Pages deployment, which is postponed (D8).**
- `persistence::Storage` holds one JSON document. Natively, `FileStorage` writes the file atomically as before; in the browser, `MemoryStorage` keeps it for the visit until Phase 3. The preference store uses it, which removed the `cfg!(wasm32)` exceptions from `preferences.rs`. `Session::from_json` and `persistence::to_json` read and write sessions without files.
- `coaching::Computation` is the interface's view of analysis. `ComputationWorker` implements it unchanged, and `Desktop` holds a `Box<dyn Computation>` for the Web Worker of Phase 2.
- `web/` shows a loading screen and a German error overlay („Etwas ist schiefgelaufen“, **Neu laden**, *Technische Details*) for panics, start failures, and failed downloads. The page title is Kniffel-Coach.
- `.github/workflows/checks.yml` runs formatting, Clippy and tests with and without `gui`, Clippy for wasm32, and the Trunk release build.
- The release build is 5.3 MB of Wasm, 2.1 MB gzipped, still without `wasm-opt`.

### Phase 2 — Background worker (2–3 days)

**Tasks:**
- Extract the scheduler core (the existing worker tests guard native behavior).
- Add the serde protocol and the worker binary.
- Add the main-thread `Computation` implementation, with restart on crash and pause when hidden.
- Give the exact attempt its full budget.
- Delete the UI-thread stepping stone.

**Done when:**
- The sheet scrolls smoothly during analysis on the iPhone.
- Editing or Undo during analysis never shows old advice.
- The two-open-category endgame from section 3 reports „exakt berechnet“.
- A debug “crash worker” button recovers by itself.

**Status (2026-10-05): done; checked on the iPhone.**
- `coaching::Scheduler` holds requests and their latest reports; `coaching::Runner` advances them one slice at a time. The native `ComputationWorker` drives both on its thread as before, and the UI-thread stepping stone is gone.
- `web/src/bin/worker.rs` runs the same scheduler in a Web Worker and returns to its event loop after every 40 ms slice, so newer messages replace or pause work promptly. It also runs the `?speed` test, so there is one worker binary.
- `web/src/computation.rs` is the page's `Computation`. It keeps its own `Scheduler` as the record of requests and accepted reports, replays the requests into every newly started worker, restarts a crashed worker, and after three crashes within a minute shows „Die Berechnung ist mehrmals abgestürzt“ with **Neu laden**. While the page is hidden, all requests are paused.
- Messages (`web/src/protocol.rs`) are JSON strings with a protocol version; tests check that 64-bit keys and seeds and whole reports survive unchanged.
- The bounded exact attempt now gets up to 500 ms instead of one slice, natively too. `?endgame` (one player, Dreierpasch and Kniffel open) reports „exact under model“ in the browser.
- Headless Firefox at 390×844: analysis runs in the worker; `?debug`'s crash button restarts it twice without losing the advice, and a third crash shows the overlay; hiding the page stops reports until it is visible again.
- On the iPhone: scrolling stays smooth during analysis, edits and Undo never show old advice, `?endgame` is exact, the crash button recovers, and analysis continues after locking the phone.

### Phase 3 — Autosave, share, and open (1–2 days)

**Tasks:**
- Add the `localStorage` session and preference stores (D4); open the last position on start.
- Hide the desktop file menu, path field, and “unsaved changes” prompts on the web.
- Add *Stellung teilen / öffnen*.

**Done when:**
- After force-quitting and reopening, the position is back.
- Export → Files → import round-trips on the iPhone.
- A phone export opens in the desktop app.

**Status (2026-10-05): done.**
- `Desktop::set_browser` switches the interface to browser behavior through the `gui::Browser` trait: *File* offers *Share position* and *Open position* instead of the path field, Save, and Load, and the start screen offers *Open position*. The "Unsaved" marker and the unsaved-changes dialog are gone; replacing a position with sheet entries asks once, "Discard the current position?". (These strings become German with the translation layer in Phase 4.)
- `web/src/autosave.rs` stores `Desktop::autosave()`, the desktop save format, in `localStorage` 300 ms after each change, and at once when the page is hidden or the app panics; the crash overlay then says the position is saved. Partial dice entry and undo history stay in memory. Preferences use `localStorage` too, and the app asks for persistent storage.
- The app reopens the last position. `?bench` and `?endgame` are never saved, so tests cannot replace the family's position.
- `web/src/files.rs` shares through the share sheet when the browser can share files, otherwise as a download (`kniffel-stellung-<date>-<time>.json`); opening uses the file picker, rejects files over 1 MiB, and validates before anything is replaced.
- GUI tests cover browser mode: no paths or unsaved prompts, sharing the desktop format (which the desktop app loads), the confirmation, and rejected files. Headless Firefox at 390×844: the position survives reloads, hiding the page saves a new score at once, the download is the desktop format, and an opened file replaces the position after the confirmation.
- On the iPhone: the position survives force-quitting Safari, and *Share position* opens the share sheet. Not checked on the device yet: saving to Files and opening that file again, and a phone file in the desktop app. Sharing is expected to be rare.

### Phase 4 — Phone analyzer, German from the start (5–7 days)

**Tasks:**
- Add the `t()` translation layer first. Write all new phone strings in German right away.
- Build the phone layout from section 2: top bar, dice panel with the Wurf selector, advice card, compact sheet grid with sideways scrolling for five or more players.
- Add the score picker, used on the desktop as well.
- Add the player sheet: name, Zusatz-Kniffel count, order.
- Add the „Wer ist dran?“ fallback for inconsistent sheets.
- Keep entered dice when an edit doesn't change who is rolling.
- Add „Übernehmen“ in the analyzer.
- Phone defaults:
  - Limited analysis of about 3 s / 20,000 games (tuned with the Phase 0 numbers), with **Länger rechnen** continuing the same search;
  - one player uses the expected-score objective.
- Add the welcome and „Wer spielt mit?“ screens.
- Add *Schriftgröße* via egui's zoom factor.
- Remove hover-only information from the phone screens.
- Check that local and computer games are usable with the shared components.

**Done when:**
- Headless tests at 312×675 (Schriftgröße Groß), 375×667, 390×844, and 430×932 pass. They cover: no page-wide horizontal overflow; dice, advice, and Übernehmen visible without scrolling at 390×844; picker values matching `rules::category_scores`; touch targets of at least 44 points.
- A non-technical family member enters a real three-player position and gets advice without help.

**Status (2026-10-07): done in seven steps, each committed as "Phase 4.N". The headless tests pass; the test with a family member is still to do.**
1. Done: `src/gui/` split (`mod.rs`, `tests.rs`) and the translation layer `gui::i18n` (`t`, `tf`, per-thread language, a test for missing or unused German entries). The web app starts in German; the Phase 3 browser texts are translated.
2. Done: score picker (`src/gui/picker.rs`) on desktop and phone, replacing the typing dialog. It offers exactly `rules::category_scores` (nonzero values in a 44-point grid of six columns that fits 310 points), *Offen*, *Streichen (0)*, and the current roll's points for the rolling player; one tap applies, Undo reverts, the mouse wheel stays. `serve-lan.sh` now also rebuilds on changes to the main crate.
3. Done: controller features. `GameController::apply_advice` is „Übernehmen“ in the position editor (a reroll locks the kept dice, a score moves to the next player; one Undo step). Entered dice stay through edits while the same player rolls. One player gets the expected-score objective. The web app starts with `Preferences::phone()` (limited analysis, 3 s / 20,000 games), and „Länger rechnen“ continues a finished limited search for the current position only.
4. Done: phone layout (`src/gui/phone.rs`) for the play screen below 600 points, in German. A fixed top bar (☰ menu, ↺ ↻) and one vertical scroll: dice with „1./2./3. Wurf“, ★ for advised dice and a 44-point palette; the advice card („Behalte ⚄⚄ · würfle 3 neu“, „Gewinnchance 49 % · Ø 238 Punkte“, *Übernehmen*, *Andere Züge*, status such as „✔ fertig · 5 314 Partien simuliert“ with *Länger rechnen*, „Knappe Sache …“, *Technische Details*); the compact sheet with short row names, bonus progress, and player columns that scroll sideways. The menu offers *Neue Stellung (gleiche Spieler)*, *Neues Spiel …*, *Darstellung*, *Rechenzeit* (Schnell 3 s, Gründlich 15 s, Ohne Limit), sharing, and *Erweitert …*. Checked at 390×844 and 312×675; dice, advice, and *Übernehmen* are visible without scrolling at 390×844.
5. Done: player sheet and „Wer ist dran?“ (`src/gui/players.rs`). In the position editor, tapping a name opens the player sheet: name (selected until changed, so typing replaces "Player 2"), Zusatz-Kniffel −/+ with a check that refuses impossible counts, and the place in the turn order (*Früher* / *Später*); *Fertig* applies all of it as one Undo step. When the entry counts do not fit the turn order, the advice area says „Die Anzahl der Einträge passt nicht zur Reihenfolge – fehlt irgendwo ein Eintrag?“, the likely culprits are marked in the sheet, and „Trotzdem analysieren – wer ist dran?“ offers one button per player, which switches to the hypothetical mode; „Annahme: … ist dran“ then offers *Automatisch bestimmen*.
6. Done: phone start screens (`src/gui/start.rs`) and *Schriftgröße*. Below 600 points the start screen is a one-page welcome (*Stellung analysieren*, *Zurück zur Stellung*, *Stellung öffnen*, local game, game against the computer); the app shows it on first start and from „Neues Spiel …“. „Wer spielt mit?“ asks for 1–8 players in playing order with optional names (empty ones become „Ich“, „Spieler 2“, …; the headline then reads „Ich bin dran“) and opens the analyzer. *Schriftgröße* Normal/Groß (zoom 1.25) is a preference; it is applied only when it changes, so desktop Ctrl +/− zoom stays. The start messages of the analyzer and games are translated. Local and computer games work in the phone layout (entering and keeping dice, „Zug ausführen“, scoring from the sheet, the computer's turns); their setup screens are still the desktop ones, small and English, until Phase 5.
7. Done: phone-size headless tests (`src/gui/tests.rs`). The analyzer with an entered roll and a finished report is rendered in English and German at the four sizes, three players at 312 points and four from 375 on. The tests check that no text or frame reaches past the screen's right edge, that every enabled tap target on screen is at least 44×44 points (scroll bars excluded), that the last player column ends on screen, and that *Übernehmen* and the face palette are visible without scrolling from 390×844 on. At 312×675 the score picker's values and the player sheet fit on screen. The tests turned up three fixes: phone screens no longer make labels selectable (a tap on text selected it, and the labels counted as small targets); *Technische Details* is a 44-point toggle; and the player sheet has a fixed width with a gutter, and its count and place moved into the row headings, so *Früher*/*Später* and −/+ fit side by side in German on 312 points.

**First usable version:** after Phase 4 (about 2½ weeks in), the family can use the online version.

### Phase 5 — German everywhere else (2–3 days)

**Tasks:**
- Translate the remaining reachable screens: home, local/computer setup, results, settings, error messages.
- Write a short German *Hilfe & Regeln* page: how to use it, the house conventions from [rules.md](rules.md) such as additional Kniffels, and the QR code.
- Use German number formatting.

**Done when:** no English is visible in normal flows (only under *Technische Details*), and the missing-translation test passes.

**Status (2026-10-07): in progress, in five steps, each committed as "Phase 5.N" and reported before the next one starts.** "Normal flows" include the desktop layout, which the web app shows from 600 points on (iPad, a phone held sideways).
1. Done: setup and results screens. Below 600 points, „Lokales Spiel“ (2–8 players, optional names, „Wer fängt an?“, *Echte Würfel* / *Die App würfelt*) and „Gegen den Computer“ (your name, who starts, your dice, *Leicht* / *Mittel* / *Stark*) have phone screens in the style of „Wer spielt mit?“. Names typed once carry over between the three setups, and the names of the last local game come back. A finished game shows the standings, best first, with *Neues Spiel*. The desktop setup and results screens and the theme buttons go through `t()`; default names follow the language („Spieler 2“, „Du“), including names saved in English. Text fields on phone screens are 44 points tall, player columns grow up to 90 points when there is room (so „Computer“ fits), and with app dice the advice area says „Würfle, dann gibt es einen Rat.“
2. Desktop layout: toolbar, dice panel, score sheets, advice, dialogs. The confirmation dialogs („Aktuelle Stellung verwerfen?“ when a new game or position replaces one with entries, unsaved changes) open centered, fit 312 points, and have 44-point buttons; today they open at the top left with desktop-sized buttons.
3. Settings (*Erweitert …*) and messages: controller and computer messages, and errors users can trigger (a German sentence, the original under *Technische Details*). On the phone, *Erweitert …* becomes a full-width, scrollable sheet with 44-point controls instead of the 540-point desktop window at the top left.
4. German number formatting everywhere, and a rendering test that looks for English in normal flows.
5. *Hilfe & Regeln* with the QR code; documentation.

### Phase 6 — Install and offline (1–2 days)

**Tasks:**
- Add the manifest (name, standalone display, theme colors) and the `apple-touch-icon` and status-bar meta tags.
- Add a dice icon set and the Safari install banner.
- Add the service worker, with a per-build precache list generated by a Trunk post-build hook, and the update banner.
- Add „App teilen“, which opens the iOS share sheet with the URL.

**Done when:**
- After one online start, the Home Screen app cold-starts in airplane mode and analyzes a position.
- A new deployment shows the update banner and updates without losing the position.

### Phase 7 — iPhone testing, polish, documentation (2–3 days)

**Tasks:**
- Work through the device checklist (section 7).
- Run a 30-minute continuous-analysis check for memory and heat.
- Tune the defaults from measurements.
- Write the README section, the German user guide, and the verification entry, recording device models, iOS versions, timings, and known limitations.

**Total:** about **16–25 focused days (roughly 3–5 weeks)**. This is more than the brainstorm's 1–3 weeks because of the German translation and the redesigned analyzer. A reasonable cut, if needed, is to postpone Phase 5 for screens the family doesn't use.

## 7. Testing

- **Native first.** Phone layouts are ordinary egui code, so `cargo test` renders them headlessly at phone sizes with the existing test harness. This is the main safety net and runs in CI.
- **Shared logic.** The scheduler core, worker protocol round-trips (including large 64-bit values), the editor's „Übernehmen“, and dice retention are covered by native unit tests.
- **Browser build.** CI checks both Wasm binaries and runs a release Trunk build. Before testing on the phone, smoke-test WebKit behavior on Linux with GNOME Web (Epiphany) or Playwright's WebKit build; both use Safari's engine family.
- **Debugging without a Mac.** Safari's Web Inspector needs macOS, so the app brings its own help:
  - *Über die App → Diagnose* shows the version, worker status, storage status, recent errors, and a speed test (games per second on a fixed position).
  - Panics appear on screen with a copyable report.
- **Device checklist (iPhone, Safari and Home Screen):**
  - install from a link;
  - first start;
  - enter a three-player position by touch;
  - advice, Übernehmen, Undo;
  - lock the screen during analysis and return;
  - force-quit and reopen;
  - airplane-mode cold start;
  - update after a deployment;
  - rotate;
  - dark mode;
  - Schriftgröße Groß;
  - share and open a position via Files and AirDrop.

  Test on the oldest family iPhone too.

## 8. Risks

| Risk | Mitigation |
| --- | --- |
| egui feels wrong on iOS (keyboard, scrolling) | Phase 0 go/no-go; fallback: HTML analyzer only (D2); names are optional, so the keyboard is rarely needed |
| Phone slower or hotter than expected | Measured native speed on an iPhone 16 Pro (section 3); limited analysis by default; pause when hidden; Diagnose speed test |
| Page served over plain HTTP runs without the compiler on iOS (35× slower) | HTTPS everywhere: GitHub Pages in production, `web/serve-lan.sh` in development; the speed test warns on insecure pages |
| Safari clears data, or users use the Safari tab instead of the app | Install banner and help text; export as backup; request persistent storage where supported |
| WebGL context lost after the app is backgrounded | Test in Phase 7; eframe recreates it, otherwise reload (position is autosaved) |
| Service worker stuck on an old version | Versioned caches; „Neu laden“ in Diagnose; a self-unregistering `sw.js` can be deployed as a kill switch |
| Desktop regressions from the refactors | Native behavior stays behind the existing 100+ tests; scheduler extraction guarded by the worker tests |
| Missing German strings after later changes | Missing-translation test in CI |
| Public URL and naming | The rulebook PDF must not be published (D8); avoid Schmidt logos; „Kniffel“ in the app name is fine for a private family tool |

## 9. Deliberately not planned

- **Native iOS app** and App Store distribution (D1).
- **Accounts, cloud sync, online multiplayer.** These would need a server and conflict with “no setup”.
- **Multi-tab conflict handling.** The Home Screen app runs as a single instance with its own storage; last write wins in Safari tabs.
- **Custom models, solitaire caches, and training in the browser.** The bundled model is used; the desktop and CLI keep these features.
- **VoiceOver support.** egui draws to a canvas; documented as a limitation.
- **A separate landscape design.** Landscape uses the existing small-window layout.
- **Later ideas:** share a position as a link (`#stellung=…`), a list of recent positions, an iPad-tuned layout, an English switch on the web.

## 10. Open questions

1. **Deployment repository** (postponed on 2026-10-05): keep the source private and deploy to a separate public repository (recommended), make this repository public after removing the rulebook PDF, or use GitHub Pro?
2. **Desktop language:** keep English (recommended), or switch to German as well?
3. **Oldest iPhone/iOS** in the family? The plan targets iOS 16.4 or newer.

Answered: the app name is Kniffel-Coach (2026-10-05).
