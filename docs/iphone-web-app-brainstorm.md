# iPhone support through a web app — brainstorming

Status: exploratory notes for future planning, not an implementation specification.

## General direction

A web app is a practical way to run Knfsolv on an iPhone while developing and building entirely on Linux. This looks like a moderate port: much of the existing code could stay, but it needs more than a different compiler command.

A native iOS app is not completely ruled out without owning a Mac, since a remote Mac could handle builds. However, Apple's native toolchain requires macOS. A web app avoids that requirement.

The existing Rust UI framework, egui/eframe, supports browser rendering. Much of the interface could be reused, with the rules and solver compiled to WebAssembly (Wasm), rather than rewritten in JavaScript.

```text
Linux development machine
    Rust → WebAssembly + JavaScript glue + HTML
                       ↓
                  Static web hosting
                       ↓
                Safari on iPhone
             Interface + solver run locally
```

The hosting server would deliver files; the phone would perform the calculations. The development PC would not need to stay running.

## Likely areas of work

| Part | Likely work |
| --- | --- |
| Rules, score sheets, turn inference | Mostly reusable |
| Solver algorithms | Mostly reusable; adapt timing and platform dependencies |
| Desktop interface | Add browser startup and improve touch layouts |
| Background analysis | Replace the native thread with a browser Web Worker |
| Saving and preferences | Use browser storage and file import/export instead of filesystem paths |
| Score editing | Add touch-friendly legal-value selection; phones have no hover/wheel |

Background analysis is likely the biggest technical piece. The current worker uses a Rust thread, shared state, and synchronization. Ordinary Rust threads and filesystem access do not work on the standard browser Wasm target.

A Web Worker could run the solver and exchange messages with the interface: analyze a position, pause, or report updated results. The solver already supports incremental work slices, which would help the worker yield to receive cancellations and updated positions. Long searches should not run on the browser's UI thread, where they would block tapping and scrolling.

## Possible build workflow

After adding a browser entry point and separating desktop-only dependencies, a typical Linux workflow could use Trunk:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk

trunk serve          # Local development
trunk build --release
```

Trunk handles Rust compilation and JavaScript glue generation, producing a `dist/` directory of website files for static hosting. These commands describe a possible workflow after the port; the current repository is not ready to build this way.

## Installation and offline use

A web manifest and offline caching could make this a Progressive Web App (PWA). On the iPhone, the user would open its URL in Safari and choose **Add to Home Screen**.

With the necessary files cached, the app could also work offline, including local solver analysis.

## Rough effort and tradeoffs

Initial estimates from code inspection, not an attempted port:

- Basic browser version: several focused development days.
- Comfortable iPhone version with responsive analysis, touch controls, saving, offline support, and device testing: roughly one to three weeks.

Phone performance needs measuring. The same algorithms can run locally, but continuous analysis may need smaller default budgets for responsiveness and battery use.

The suggested direction is to reuse the Rust engine and egui interface, get them working in the browser, then add PWA and touch polish. A server-hosted solver remains an alternative if phone performance is insufficient, at the cost of requiring an internet connection and an ongoing server to maintain.

## References

- [Apple Xcode requirements](https://developer.apple.com/xcode/system-requirements)
- [eframe web support](https://docs.rs/eframe/latest/wasm32-unknown-unknown/eframe/)
- [Rust browser Wasm target limitations](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html)
- [Trunk](https://github.com/trunk-rs/trunk)
- [Add a web app to the iPhone Home Screen](https://support.apple.com/en-mide/guide/iphone/iphea86e5236/ios)
- [PWA offline caching](https://developer.mozilla.org/en-US/docs/Web/Progressive_web_apps/Guides/Caching)
