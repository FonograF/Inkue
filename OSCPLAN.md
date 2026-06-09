# OSC Implementation Plan — WinCue

## Design decisions (from design session)

| Decision | Choice |
|---|---|
| Scope | Send Cue + Receive listener |
| Messages per cue | Multiple (`Vec<OscMessage>`) |
| Argument types | `int32`, `float32`, `string`, `bool` |
| Send targets | Named OSC Patches (workspace-level, in `.wincue`) |
| Receive commands | GO, Stop All, Hard Stop, Go To Cue (fire), Select Cue (playhead only), Pause, Resume, Stop cue by number |
| Receive address scheme | Custom `/wincue/` prefix |
| Receive security | IP allowlist (default = accept all); port default `53001` |
| Receive config location | Machine-level Preferences |
| Rust OSC library | `rosc` |
| Send timing | All messages simultaneous at GO |
| Inspector UI | Single "Messages" tab |
| Receive feedback | Flashing activity dot in transport bar |
| OSC Patches UI | Workspace Settings alongside Output Patches |
| Go To Cue behavior | `/go` fires + `/select` moves playhead only |

---

## OSC receive address scheme

| Address | Action |
|---|---|
| `/wincue/go` | Advance playhead and fire GO |
| `/wincue/stop` | Stop all running cues (soft fade) |
| `/wincue/hardstop` | Hard stop all |
| `/wincue/pause` | Pause all running cues |
| `/wincue/resume` | Resume all paused cues |
| `/wincue/cue/{number}/go` | Jump playhead to cue number and fire |
| `/wincue/cue/{number}/select` | Move playhead to cue number, do not fire |
| `/wincue/cue/{number}/stop` | Stop specific cue by number |

---

## Implementation steps

### Step 1 — Add `rosc` dependency

- `src-tauri/Cargo.toml`: add `rosc = "0.10"`

---

### Step 2 — OSC data types (`cue/osc_types.rs`, new file)

```rust
pub enum OscArg { Int(i32), Float(f32), Str(String), Bool(bool) }
pub struct OscMessage { pub patch_id: Uuid, pub address: String, pub args: Vec<OscArg> }
```

- `Serialize`/`Deserialize` for `.wincue` persistence
- Mirror types to TypeScript in `lib/types.ts`

---

### Step 3 — OSC Patch model

**Rust — `engine/osc_patch.rs` (new file)**
```rust
pub struct OscPatch { pub id: Uuid, pub name: String, pub ip: String, pub port: u16 }
```

**Workspace integration — `show/workspace.rs`**
- Add `osc_patches: Vec<OscPatch>` field (serde default = empty vec)
- Inject into `CueContext` alongside `output_patches`

**`CueContext` — `cue/context.rs`**
- Add `osc_patches: Arc<Vec<OscPatch>>`
- Add `resolve_osc_patch(id: Uuid) -> Option<&OscPatch>` helper

**TypeScript — `lib/types.ts`**
- Add `OscPatch`, `OscArg`, `OscMessage` types

---

### Step 4 — OSC Send Cue (`cue/osc_cue.rs`, new file)

Implements the full `Cue` trait:
- `go()`: iterate `messages`, resolve each `patch_id` via `ctx.resolve_osc_patch()`, encode with `rosc`, send via `UdpSocket::bind("0.0.0.0:0")` + `send_to`. Emit `ActionStarted` + `ActionCompleted` immediately.
- `duration()` → `Some(Duration::ZERO)`
- `is_action_started()` → `true` always
- `serialize()` / `CueFactory::from_json()` for `.wincue` roundtrip
- Register in `CueRegistry` (`cue/registry.rs`)

---

### Step 5 — OSC Receive server (`engine/osc_server.rs`, new file)

```rust
pub struct OscServer { /* handle to background thread */ }

impl OscServer {
    pub fn start(config: OscReceiveConfig, app_handle: AppHandle) -> Self { ... }
    pub fn reconfigure(&self, config: OscReceiveConfig) { ... }
    pub fn stop(&self) { ... }
}
```

- Spawns one `std::thread` that binds a `UdpSocket` on the configured port
- Loop: `recv_from` → check IP allowlist → decode with `rosc::decode_udp` → dispatch to `handle_osc_message()`
- `handle_osc_message()` matches address patterns, sends a Tauri `wincue://osc-command` event to the frontend — the frontend calls the same `invoke()` commands as keyboard shortcuts
- Config changes: send a shutdown signal via `AtomicBool` + dummy packet to unblock `recv_from`, then restart with new config
- Emits `wincue://osc-activity` event (empty payload) for the transport bar indicator on every acted-upon message

---

### Step 6 — Machine-level Preferences for receive

**`preferences.rs`**
```rust
pub struct OscReceiveConfig {
    pub enabled: bool,
    pub port: u16,           // default 53001
    pub allowed_ips: Vec<String>,  // empty = accept all
}
```
- Add to `Preferences` struct (serde default)

**`commands/preferences_cmds.rs`**
- `get_osc_config() -> OscReceiveConfig`
- `set_osc_config(config: OscReceiveConfig)` — saves + reconfigures `OscServer`

**`state/app_state.rs`**
- Add `osc_server: Arc<OscServer>`
- Start server at app init in `lib.rs`

---

### Step 7 — Tauri commands for OSC Patches

**`commands/osc_cmds.rs` (new file)**
- `list_osc_patches(workspace_id) -> Vec<OscPatch>`
- `add_osc_patch(workspace_id, name, ip, port) -> OscPatch`
- `update_osc_patch(workspace_id, patch)`
- `remove_osc_patch(workspace_id, patch_id)`

Register in `lib.rs` invoke handler.

---

### Step 8 — Frontend: OSC Patches management UI

**`components/OscPatches/OscPatchesPanel.tsx` (new file)**
- Table: Name | IP | Port | Delete button
- Add row button at bottom
- Inline-editable cells (same pattern as Output Patches if one exists, otherwise a simple controlled table)
- Wired to `osc_cmds` via `lib/commands.ts`

Mount inside Workspace Settings (wherever Output Patches are managed today — investigate exact location).

---

### Step 9 — Frontend: OSC Send Cue Inspector tab

**`components/Inspector/OscTab.tsx` (new file)**
- List of message rows; each row:
  - Patch selector (dropdown of `OscPatch` names)
  - Address text input
  - Arg list: each arg has a type selector (`int` / `float` / `string` / `bool`) + value input
  - Remove row button
- "Add message" button at bottom
- onChange calls `update_cue` command

**`components/Inspector/InspectorPanel.tsx`**
- Add `OscTab` branch for `CueType.Osc`

---

### Step 10 — Frontend: OSC activity indicator

**`hooks/useTauriEvents.ts`**
- Listen for `wincue://osc-activity` → set a short-lived boolean in transport store

**`stores/transportStore.ts`**
- Add `oscActivityAt: number | null` (timestamp of last OSC message)

**`components/Transport/TransportBar.tsx`**
- Small dot (e.g., 8×8px circle) that turns green for 300ms when `oscActivityAt` is recent, then fades back to grey

---

### Step 11 — Frontend: Preferences modal OSC tab

**`components/Preferences/PreferencesModal.tsx`**
- Add "OSC" tab
- Fields: Enable toggle, Port number input, IP allowlist (textarea, one IP per line)
- onChange calls `set_osc_config`

---

### Step 12 — CueList UI: OSC cue row

**`lib/types.ts`**
- Add `OscCueData` interface and `CueType.Osc` variant

**`components/CueList/CueRow.tsx`**
- Add OSC cue icon/label (no special columns needed — OSC has no duration display)

---

### Step 13 — Tests

**`src-tauri/` (`cargo test`)**
- `OscArg` serialize/deserialize roundtrip
- `OscMessage` serialize/deserialize roundtrip
- `OscCue` serialize/deserialize roundtrip
- IP allowlist filtering logic (unit test on `osc_server` module)
- Address pattern matching in receive dispatch

---

### Step 14 — PROGRESS.md + Cargo.toml version bump

- Add `OscCue` row to cue type status table
- Add `OscServer` + `osc_cmds` rows to module table
- Bump to `0.6.0`

---

## File inventory

### New files
| File | Purpose |
|---|---|
| `src-tauri/src/cue/osc_types.rs` | `OscArg`, `OscMessage` types |
| `src-tauri/src/cue/osc_cue.rs` | `OscCue` + `OscCueFactory` |
| `src-tauri/src/engine/osc_patch.rs` | `OscPatch` struct |
| `src-tauri/src/engine/osc_server.rs` | UDP listener, dispatch, activity event |
| `src-tauri/src/commands/osc_cmds.rs` | Tauri commands for patches + config |
| `src/components/Inspector/OscTab.tsx` | Inspector messages tab |
| `src/components/OscPatches/OscPatchesPanel.tsx` | Workspace OSC patch management |

### Modified files
| File | Change |
|---|---|
| `src-tauri/Cargo.toml` | Add `rosc` dependency |
| `src-tauri/src/cue/mod.rs` | Export `osc_types`, `osc_cue` |
| `src-tauri/src/cue/registry.rs` | Register `OscCueFactory` |
| `src-tauri/src/cue/context.rs` | Add `osc_patches` field + `resolve_osc_patch()` |
| `src-tauri/src/engine/mod.rs` | Export `osc_patch`, `osc_server` |
| `src-tauri/src/show/workspace.rs` | Add `osc_patches: Vec<OscPatch>` |
| `src-tauri/src/preferences.rs` | Add `OscReceiveConfig` |
| `src-tauri/src/state/app_state.rs` | Add `osc_server: Arc<OscServer>` |
| `src-tauri/src/commands/mod.rs` | Export `osc_cmds` |
| `src-tauri/src/commands/preferences_cmds.rs` | Add `get/set_osc_config` |
| `src-tauri/src/lib.rs` | Init `OscServer`; register `osc_cmds` |
| `src/lib/types.ts` | Add `OscPatch`, `OscArg`, `OscMessage`, `OscCueData`, `CueType.Osc` |
| `src/lib/commands.ts` | Add OSC patch + config commands |
| `src/hooks/useTauriEvents.ts` | Listen for `osc-activity` event |
| `src/stores/transportStore.ts` | Add `oscActivityAt` |
| `src/components/Transport/TransportBar.tsx` | Activity dot |
| `src/components/Inspector/InspectorPanel.tsx` | Add `OscTab` branch |
| `src/components/Preferences/PreferencesModal.tsx` | Add OSC tab |
| `src/components/CueList/CueRow.tsx` | OSC cue row label/icon |
| `PROGRESS.md` | Update status table, bump to 0.6.0 |
