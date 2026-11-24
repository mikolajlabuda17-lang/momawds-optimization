# Momawd's Optimization Mod

**Momawd's Optimization Mod** (`momawds_optimization`) is a lightweight, sandboxed **WebAssembly (`.hmod`)** all-in-one **client FPS + world/server TPS** optimization mod for **Horizon `26.3`** (`api = "0.2"`, `side = "both"`).

Because it compiles to a pure WebAssembly module (`mod.wasm`), it runs inside Horizon's zero-trust sandbox with **no native-code warning prompt** and works seamlessly across Windows, Linux, and macOS.

---

## Features

### Client FPS & Frame-Time Optimizations
- **Dynamic Background, Minimized & Idle Menu FPS Governor**: Automatically caps framerate when the game window is unfocused (`5–60 FPS` configurable) and drops to `1 FPS` when minimized to eliminate background GPU power draw and heat. In **Competitive** (`120 FPS`) and **Max FPS** (`60 FPS`) profiles, idle main-menu rendering is also capped.
- **Distance-Based Item Frame 3D Content Culling**: Skips expensive 3D item and map draws inside distant item frames (`48m` Balanced / `32m` Competitive / `18m` Max FPS, scaling down automatically during active `< 45 FPS` pressure) while keeping the item frame block itself visible — ideal for large storage rooms and shopping districts.
- **Adaptive Ambient Swirl & Crowd Name-Tag Relief**: Suppresses redundant ambient beacon swirls and armor-stand crowd name-tag overdraw under heavy load while strictly preserving multiplayer fairness (never reveals hidden effects or hidden name tags, and never hides player name tags).

### World & Server TPS Optimizations *(Off by Default — Opt-In)*
> **Multiplayer Warning:** Both world/server simulation optimizations below (**Lossless XP Orb Clumping** and **Adaptive MSPT & Explosion Guard**) are **disabled (`off`) by default** because they modify experience orb pickup/cooldown and server entity/explosion simulation behavior. Always check multiplayer server rules before enabling them on a shared server.

- **Lossless `u128` Experience Orb Clumping & Instant Pickup** *(default: Off)*: Merges nearby experience orbs of any value into exact saved-NBT `(i16 Value, i32 Count)` representations with **zero XP loss**, collapses clumped stacks on pickup, and removes the vanilla XP pickup delay so mob grinders and furnace arrays stay lag-free.
- **Adaptive Server MSPT Wildlife Pacer** *(default: Off)*: Monitors authoritative server `tick_stats()` (`MSPT`) and `server_has_time()` every tick. When MSPT is healthy, entity ticking is 100% untouched. During MSPT spikes above your preset's budget, ambient wildlife (`bat`, `cod`, `salmon`, `tropical_fish`, `tadpole`, `squid`, `glow_squid`) is deterministically phase-staggered across ticks without touching players, villagers, redstone, minecarts, items, or farm mobs.
- **Chain-Explosion Blast Storm Guard** *(default: Off)*: Tracks explosions per server tick and caps redundant trailing entity blast-exposure walks on the 4th+ simultaneous explosion in a single tick (preventing $O(N_{\text{explosions}} \times N_{\text{entities}})$ server stalls during TNT chain reactions).

### In-Game Settings & Mini-HUD
Configure everything live in **`Options > Mods > Momawd's Optimization Mod > Config`** (localized in English and Polish):
- **Performance Preset**: `Balanced` *(default)* / `Competitive` / `Max FPS`
- **Background FPS Limit**: `5` – `60 FPS`
- **Individual Feature Toggles**: Item-frame culling *(on)*, ambient swirl governor *(on)*, lossless XP clumping *(off by default — check server rules before enabling)*, adaptive MSPT & explosion guard *(off by default — check server rules before enabling)*
- **Performance Mini-HUD**: `Off` / `Compact (FPS + MSPT)` / `Detailed (FPS + 1% Low + MSPT + Optimizations)` with selectable screen corner (`Top left`, `Top right`, `Bottom left`, `Bottom right`), color-coded health indicator, and automatic yield to `F3`.

---

## Rebuilding & Relinking Against the Horizon SDK

### Pinned Build Inputs
- **Horizon Mod SDK Repository**: [`https://github.com/horizon-rust/horizon`](https://github.com/horizon-rust/horizon)
- **Pinned SDK Commit**: [`f037744a79f9982c47876f49c5a3067279f88706`](https://github.com/horizon-rust/horizon/tree/f037744a79f9982c47876f49c5a3067279f88706) (declared in `Cargo.toml` under `[package.metadata.horizon] sdk`)
- **Pinned Rust Wasm Toolchain**: `1.99.1` (`58cf8f9c35b355199727627d07ae34a4e2846d67`) with `rust-src` and `wasm32-unknown-unknown` target (declared in `horizon-mod.toml` under `[wasm] toolchain`)

### 1. Rebuilding `mod.wasm` and `.hmod` from the Pinned SDK
```bash
# Install the pinned Rust toolchain + wasm32 target + rust-src (for -Zbuild-std)
rustup toolchain install 1.99.1 --profile minimal --component rust-src --target wasm32-unknown-unknown

# Install cargo-horizon-mod from the exact pinned Horizon commit
cargo install --locked \
  --git https://github.com/horizon-rust/horizon \
  --rev f037744a79f9982c47876f49c5a3067279f88706 \
  cargo-horizon-mod

# Run unit tests, compile mod.wasm against the verified pinned SDK, and pack .hmod
cargo test
cargo horizon-mod build --wasm
cargo horizon-mod check target/horizon-mod/momawds_optimization.hmod --kind mod
```
`cargo horizon-mod build --wasm` automatically fetches and minisign-verifies the `f037744a79f9982c47876f49c5a3067279f88706` SDK into `.horizon/sdk/`, compiles `mod.wasm` as a position-independent shared WebAssembly module (`-Crelocation-model=pic`), updates `[wasm] sha512` in `horizon-mod.toml`, and writes `target/horizon-mod/momawds_optimization.hmod`.

### 2. Relinking Against a Modified or Updated Horizon SDK (LGPL-3.0 §4d)
Because `mod.wasm` incorporates compiled code from the Horizon Mod SDK (`LGPL-3.0-or-later`), all Mod source files (`Cargo.toml`, `Cargo.lock`, `horizon-mod.toml`, `src/*.rs`) are bundled inside the `.hmod` archive as well as in this repository so you can modify the SDK and relink `mod.wasm`:

- **Against a local modified checkout of `horizon-rust/horizon`**:
  1. Clone [`https://github.com/horizon-rust/horizon`](https://github.com/horizon-rust/horizon), check out `f037744a79f9982c47876f49c5a3067279f88706` (or a compatible revision), and apply your changes to `crates/horizon-core`, `crates/horizon-mod-api`, `crates/horizon-mod-vocab`, or `crates/horizon-mod-runtime`.
  2. Install `cargo-horizon-mod` from your modified checkout:
     ```bash
     cargo install --locked --path <horizon-checkout>/crates/cargo-horizon-mod
     ```
  3. Either place this mod directory inside your `<horizon-checkout>` workspace and update `horizon-mod-api` in `Cargo.toml` to `{ path = "<relative-path-to-horizon>/crates/horizon-mod-api" }` (detected directly by `cargo-horizon-mod` without downloading a remote SDK), or serve/unpack your modified SDK release directory via `HORIZON_SDK_BASE=<dir>`.
  4. Run `cargo horizon-mod build --wasm` to recompile and relink `mod.wasm` and emit the updated `.hmod`.
- **Against a newer upstream Horizon SDK build**:
  ```bash
  cargo horizon-mod update --horizon <commit-or-nightly>
  cargo horizon-mod build --wasm
  ```
- **Runtime Dynamic Linking**: At game startup, Horizon links `mod.wasm` in memory against the engine's shared `horizon-wasm-std.wasm` (`cargo horizon-mod wasm-std <out.wasm>`).

---

## License & Third-Party Notices

- **Original Mod Code & Assets**: Released under the [MIT License](./LICENSE) (`Copyright (c) 2026 Momawd`).
- **Horizon SDK & Third-Party Code in `mod.wasm`**: The precompiled `mod.wasm` inside the `.hmod` archive includes compiled code from the pinned Horizon Mod SDK ([`LGPL-3.0-or-later`](./LICENSE-LGPL-3.0) / [`GPL-3.0`](./LICENSE-GPL-3.0)), Sun Microsystems `fdlibm` 5.3, the Rust `1.99.1` standard/`compiler_builtins` libraries ([`Apache-2.0 WITH LLVM-exception`](./LICENSE-APACHE-2.0) / `MIT`), and third-party crates (`Cargo.lock`). See [`NOTICE`](./NOTICE), [`LICENSE-LGPL-3.0`](./LICENSE-LGPL-3.0), [`LICENSE-GPL-3.0`](./LICENSE-GPL-3.0), [`LICENSE-APACHE-2.0`](./LICENSE-APACHE-2.0), [`LICENSE-UNICODE-3.0`](./LICENSE-UNICODE-3.0), and [`LICENSE-UNICODE-DFS-2016`](./LICENSE-UNICODE-DFS-2016) (all included in both this repository and the `.hmod` archive).
