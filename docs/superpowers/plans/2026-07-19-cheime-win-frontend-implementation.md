# CheIME Windows Frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build the Windows frontend for CheIME MVP: TSF TIP DLL (x64+x86), engine host exe (x64), GDI candidate window, and installer CLI tool.

**Architecture:** `cheime-tip-core` provides shared Windows platform utilities (named pipe I/O, channel dispatch, GDI candidate window). `cheime-tip` is the COM DLL loaded by TSF. `cheime-engine-host` is the user-level engine process. `cheime-installer` registers/unregisters the TIP.

## Global Constraints

- Work only in `D:/coding/cheime/cheime-win`.
- `cheime-core` is pinned via Git submodule; do not modify it.
- All Windows-specific types via `windows` crate 0.58.
- TIP DLL must be loadable into arbitrary host processes — minimal dependencies, no panic across COM boundary.
- Engine host uses `cheime-core` crates via submodule paths.
- `#![forbid(unsafe_code)]` where possible; COM FFI may require limited unsafe wrappers.
- TDD: red, green, refactor, commit.
- Quality gate: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.

---

### Task 1: Named pipe I/O wrapper (cheime-tip-core)

**Files:**
- Create: `crates/cheime-tip-core/src/pipe.rs`
- Modify: `crates/cheime-tip-core/src/lib.rs`

Implement a non-blocking, channel-based pipe I/O adapter:

```rust
pub struct PipeWriter {
    handle: OwnedHandle,  // windows::Win32::Foundation::HANDLE
}

pub struct PipeReader {
    handle: OwnedHandle,
}

impl PipeWriter {
    pub fn new(handle: OwnedHandle) -> Self;
    /// Write a length-prefixed framed message via MessageCodec
    pub fn write_message<M: Serialize>(&self, codec: &MessageCodec, msg: &M) -> Result<(), PipeError>;
}

impl PipeReader {
    pub fn new(handle: OwnedHandle) -> Self;
    /// Read bytes into an internal buffer, attempt to parse a frame
    /// Returns None if more data needed, Some(payload) if a complete frame is ready
    pub fn try_read_frame(&mut self, max_size: usize) -> Result<Option<Vec<u8>>, PipeError>;
}

pub enum PipeError {
    Io(String), Wire(WireError),
    Disconnected, BufferTooSmall,
}
```

PipeReader uses an internal `Vec<u8>` read buffer. On each call, it reads available bytes from the pipe via `ReadFile`, appends to buffer, then calls `FramedReader::read_frame` to see if a complete frame arrived. Returns the payload bytes (copied) if so, or None to signal "need more data".

The key design principle: these are pure synchronous wrappers — the caller (I/O thread or main thread) owns the read loop.

Tests: Use Windows anonymous pipes or in-memory byte vectors.

**Commit:** `feat: add named pipe I/O adapter`

---

### Task 2: Channel dispatch types (cheime-tip-core)

**Files:**
- Create: `crates/cheime-tip-core/src/channel.rs`

Define the bounded mpsc channel for TSF→I/O thread communication:

```rust
use std::sync::mpsc;

pub struct TipChannel {
    /// TSF callbacks push FrontendMessages here (bounded)
    sender: mpsc::SyncSender<FrontendMessage>,
    /// I/O thread receives from here
    receiver: Option<mpsc::Receiver<FrontendMessage>>,
}

impl TipChannel {
    pub fn new(bound: usize) -> Self;
    /// Try to send; returns Err if queue full (caller should handle backpressure)
    pub fn try_send(&self, msg: FrontendMessage) -> Result<(), TrySendError<FrontendMessage>>;
    /// Take the receiver end (handed to I/O thread)
    pub fn take_receiver(&mut self) -> Option<mpsc::Receiver<FrontendMessage>>;
}

pub enum DispatchMessage {
    Snapshot(CandidateSnapshot),
    PlatformAction(PlatformAction),
    SessionStatus(String),     // connection state notification
}
```

Tests:
1. Send/receive round-trip through bounded channel
2. Queue full returns error
3. Multiple messages in order

**Commit:** `feat: add channel dispatch types`

---

### Task 3: GDI candidate window (cheime-tip-core)

**Files:**
- Create: `crates/cheime-tip-core/src/candidate_window.rs`

Pure Win32/GDI candidate rendering window:

```rust
pub struct CandidateWindow {
    hwnd: HWND,
    current_snapshot: Option<CandidateSnapshot>,
    metrics: WindowMetrics,
}

struct WindowMetrics {
    line_height: i32,
    font: HFONT,
    width: i32,
    height: i32,
}

impl CandidateWindow {
    pub fn create() -> Result<Self, WindowError>;
    pub fn update_snapshot(&mut self, snapshot: CandidateSnapshot);
    pub fn show_at(&mut self, x: i32, y: i32);
    pub fn hide(&mut self);
    pub fn hwnd(&self) -> HWND;
    pub fn destroy(self);
}
```

Window class name: `"CheIME_Candidate_Window"` (registered in `create()`).

`WM_PAINT` handler:
1. `BeginPaint` / `EndPaint`
2. Select `HFONT` into DC
3. Draw preedit line with `ExtTextOutW` (with underline via display attribute)
4. Draw each candidate row: `序号. 文本 [注释]`
5. Highlight current selection with `FillRect` + `COLOR_HIGHLIGHT`
6. Draw page indicator at bottom

`WM_USER_SNAPSHOT` (custom message registered via `RegisterWindowMessageW`):
- Deserialize `CandidateSnapshot`
- Update internal state
- `InvalidateRect` to trigger repaint

Candidate selection via mouse click:
- `WM_LBUTTONDOWN` → hit-test row → produce callback

Layout calculation:
- Font: `SystemParametersInfo(SPI_GETNONCLIENTMETRICS)` → `lfMessageFont`
- Line height: `TEXTMETRIC.tmHeight + tmExternalLeading + 2px`
- Width: max(text_widths) + 序号线(24px) + padding(16px)

Tests: Test font selection, layout calculation, and snapshot data holder with mock HWND (unit-testable parts only; actual window creation requires message pump).

**Commit:** `feat: add GDI candidate window rendering`

---

### Task 4: Engine host — named pipe server (cheime-engine-host)

**Files:**
- Create: `crates/cheime-engine-host/src/server.rs`
- Modify: `crates/cheime-engine-host/src/main.rs`

Named pipe server that listens for TIP connections:

```rust
pub struct EngineServer {
    pipe_path: String, // "\\\\.\\pipe\\cheime-engine"
}

pub struct ClientConnection {
    client_id: ClientInstanceId,
    reader: PipeReader,
    writer: PipeWriter,
    session: Session<BuiltinPipeline>, // placeholder — real pipeline comes later
}
```

Connection flow:
1. `CreateNamedPipeW` on `\\.\pipe\cheime-engine`
2. `ConnectNamedPipe` (blocking accept)
3. Send `ServerHello` via MessageCodec (raw framed write, no session yet)
4. Read `ClientHello` within 5-second timeout
5. Version check → `HelloAck` or `HelloRejected` + disconnect
6. On ack: create dedicated pipe `\\.\pipe\cheime-engine.{client_id}`
7. Spawn reader thread per client

Tests: Test handshake message encode/decode (no actual pipe needed — use `cheime-wire` to verify framing).

**Commit:** `feat: add engine host named pipe server`

---

### Task 5: TIP DLL — COM exports and registration (cheime-tip)

**Files:**
- Modify: `crates/cheime-tip/src/lib.rs`
- Create: `crates/cheime-tip/src/exports.rs`
- Create: `crates/cheime-tip/cheime-tip.def`

Implement the 4 standard COM DLL exports:

```rust
// These are #[no_mangle] pub unsafe extern "stdcall" exports
DllRegisterServer   // Write CLSID + InprocServer32 + ThreadingModel to HKCU
DllUnregisterServer // Remove above keys
DllGetClassObject   // Create class factory for our CLSID, return via ppv
DllCanUnloadNow     // Return S_OK if no outstanding objects
```

DEF file:
```
EXPORTS
    DllRegisterServer   @1
    DllUnregisterServer @2
    DllGetClassObject   @3
    DllCanUnloadNow     @4
```

The CLSID is a fixed GUID defined in the crate. Registration writes to `HKCU\Software\Classes\CLSID\{{...}}`.

Tests: Verify registry key generation logic, GUID formatting.

**Commit:** `feat: add TIP DLL COM exports and registration`

---

### Task 6: TIP DLL — IClassFactory and TIP skeleton (cheime-tip)

**Files:**
- Create: `crates/cheime-tip/src/class_factory.rs`
- Create: `crates/cheime-tip/src/tip.rs`

Implement `IClassFactory` via hand-written COM vtable:

```rust
// IClassFactory methods:
//   QueryInterface, AddRef, Release (IUnknown)
//   CreateInstance(riid, ppv) -> create TIP instance
//   LockServer(lock)
```

`CreateInstance` spawns a new TIP instance. The TIP struct holds:
- Reference count
- Thread manager reference
- Client ID
- Pipe connection state
- Channel sender/receiver
- I/O thread handle
- Candidate window (delayed creation)

TSF interfaces to stub (this task: declare the vtable types; implement IUnknown methods correctly):

```rust
// The TIP object implements these IIDs:
// ITfTextInputProcessorEx  — ActivateEx, Deactivate
// ITfKeyEventSink          — OnTestKeyDown, OnKeyDown, OnKeyUp
// ITfCompositionSink       — OnCompositionTerminated
// ITfThreadMgrEventSink    — OnInitDocumentMgr, OnUninitDocumentMgr, OnSetFocus, OnPushContext
```

Tests: Reference counting (AddRef/Release), QueryInterface for known and unknown IIDs.

**Commit:** `feat: add TIP class factory and skeleton`

---

### Task 7: TIP DLL — TSF key event handling (cheime-tip)

**Files:**
- Modify: `crates/cheime-tip/src/tip.rs`

Implement `ITfKeyEventSink`:

```rust
// OnTestKeyDown: no side effects, return S_OK if CheIME handles this key
//   Rules: CheIME active? Is Chinese mode? Is a-z/Backspace/Escape/Enter/Space/digit?
// OnKeyDown: reuse same decision, push KeyCommand to mpsc channel, return S_OK
// OnKeyUp: return S_OK (no-op in MVP)
```

Permission rules:
- CheIME not activated → S_FALSE (pass through)
- English mode → pass through except Shift/Ctrl+Space (toggle to Chinese)
- Chinese mode → handle a-z, Backspace, Escape, Enter, Space, digits 1-9, +/-, PgUp/PgDn, Up/Down
- All other keys → S_FALSE

Tests: Permission matrix for every key, mode combinations.

**Commit:** `feat: add TSF key event handling`

---

### Task 8: Candidate window integration (cheime-tip)

**Files:**
- Modify: `crates/cheime-tip/src/tip.rs`

Integrate CandidateWindow into the TIP's message loop:

- `ITfTextInputProcessorEx::ActivateEx` → initialize TIP state, connect to engine pipe
- I/O thread reads `CandidateSnapshot` → `PostMessage(hwnd, WM_USER_SNAPSHOT, ...)`
- WindowProc handles snapshot → update CandidateWindow → show at TSF anchor position
- Mouse click on candidate → `UiCommand::SelectCandidate` → push to I/O thread → send to engine
- `ITfCompositionSink::OnCompositionTerminated` → hide candidate window
- `Deactivate` → disconnect pipe, destroy window, join I/O thread

Anchor position: `ITfContextView::GetTextExt` or fall back to caret position via `GetCaretPos` / `GetGUIThreadInfo`.

**Commit:** `feat: integrate candidate window with TIP`

---

### Task 9: Engine host — session integration (cheime-engine-host)

**Files:**
- Modify: `crates/cheime-engine-host/src/main.rs`
- Create: `crates/cheime-engine-host/src/session_runner.rs`

Wire up actual session/pipeline/dictionary:

```rust
// On client connection:
// 1. Load dictionary from %LOCALAPPDATA%\CheIME\data\dicts\
// 2. Create BuiltinPipeline with entries
// 3. Create Session with pipeline
// 4. Handle FrontendMessage → Session::handle → EngineMessage → write to pipe
// 5. Load user data store, hook into Session commit confirmations
// 6. Load Lua extensions if configured
```

Message loop per client:
```rust
loop {
    // Read frame from pipe
    match reader.try_read_frame(65536) {
        Ok(Some(payload)) => {
            let msg: FrontendMessage = codec.decode_frontend(&payload)?;
            let outputs = session.handle(msg)?;
            for out in outputs {
                writer.write_message(&codec, &out)?;
            }
        }
        Ok(None) => continue, // need more data
        Err(PipeError::Disconnected) => break,
        Err(e) => { log::warn!("pipe error: {e}"); break; }
    }
}
```

**Commit:** `feat: integrate session, pipeline and dictionary in engine host`

---

### Task 10: Installer tool (cheime-installer)

**Files:**
- Modify: `crates/cheime-installer/src/main.rs`

Implement the three CLI commands:

```rust
fn cmd_install() -> Result<()> {
    // 1. LoadLibrary("cheime-tip-x64.dll") → call DllRegisterServer
    // 2. LoadLibrary("cheime-tip-x86.dll") → call DllRegisterServer (via syswow64 redirection or explicit path)
    // 3. Register TSF profile via ITfInputProcessorProfileMgr
    // 4. Ensure %LOCALAPPDATA%\CheIME\ directories exist
}

fn cmd_uninstall() -> Result<()> {
    // 1. Unregister TSF profile
    // 2. LoadLibrary → call DllUnregisterServer for both bitnesses
}

fn cmd_status() -> Result<()> {
    // Check registry keys, TSF profile, file layout
    // Print human-readable status
}
```

CLI parsing: simple `std::env::args()` match, no clap dependency needed for MVP.

**Commit:** `feat: add installer register/unregister/status commands`

---

### Task 11: Integration test — local echo pipeline

**Files:**
- Create: `tests/integration/fake_ime_roundtrip.rs`

Since we can't easily run real TSF in CI, create an integration test that exercises the pipe I/O path end-to-end:

1. Start engine-host in background thread (or process)
2. Create pipe connection as "fake TIP"
3. Send KeyCommand → verify CandidateSnapshot response
4. Send SelectCandidate → verify PlatformAction response
5. Send PlatformActionResult → verify state confirmed
6. Test disconnect/reconnect

**Commit:** `test: add integration test for pipe roundtrip`

---

### Task 12: Full quality gate and finalize

- Add `.cargo/config.toml` with x86 target alias
- Add build scripts or PowerShell build helper
- Run full `cargo test --workspace`
- Run `cargo clippy --workspace --all-targets -- -D warnings`
- Verify dependency tree has no unexpected platform leakage
- Update README with build instructions

**Commit:** `chore: finalize Windows frontend quality gate`
