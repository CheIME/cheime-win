# Rime-Style Composition and Learning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build graph-based incomplete-Pinyin lookup, Rime-style partial candidate selection, phrase construction, and ten-second rollback-aware user learning.

**Architecture:** `PinyinSegmentor` produces an offset-preserving DAG, and a new decoder combines dictionary/user lexemes into full and prefix candidates without losing selection metadata. `Session` owns confirmed-versus-active composition state, while a learning service stages only successfully applied novel phrases and the Windows TIP can cancel a staged phrase after a guarded Backspace.

**Tech Stack:** Rust 1.85.0, edition 2024, Cargo resolver 3, serde/MessagePack protocol, SQLite via rusqlite, parking_lot, Windows TSF.

## Global Constraints

- Core remains platform-independent; no TSF, COM, HWND, or Windows types enter `cheime-core`.
- Core crates keep `#![forbid(unsafe_code)]`; Windows unsafe code stays inside `cheime-tip`.
- Composition is cleared only after `PlatformActionOutcome::Applied`.
- The static dictionary and user dictionary preserve deterministic ordering.
- No neural model, cloud sync, or cross-commit phrase encoder is added.
- Tests use inline dictionaries except for the final real-dictionary acceptance check.
- The dirty independent checkout at `D:/coding/cheime/cheime-core` is not edited.
- Production code is written only after its focused test has been observed failing.

## File Map

### Core submodule

- `cheime-core/crates/cheime-pipeline/src/segmentation.rs`: graph values shared by segmentor, normalizer, and decoder.
- `cheime-core/crates/cheime-pipeline/src/segmentor.rs`: trie-driven graph construction.
- `cheime-core/crates/cheime-pipeline/src/normalizer.rs`: span-preserving graph variants.
- `cheime-core/crates/cheime-dictionary/src/index.rs`: weighted exact/prefix lexicon records for memory indexes.
- `cheime-core/crates/cheime-dictionary/src/tiered.rs`: matching weighted records for tiered indexes.
- `cheime-core/crates/cheime-pipeline/src/decoder.rs`: bounded deterministic word-graph search.
- `cheime-core/crates/cheime-pipeline/src/translator.rs`: dictionary/user lexicon adapters; remove text-only concatenation fallback.
- `cheime-core/crates/cheime-pipeline/src/lib.rs`: resolved candidate pipeline contract and learning hooks.
- `cheime-core/crates/cheime-pipeline/src/filter.rs`: metadata-preserving deduplication.
- `cheime-core/crates/cheime-pipeline/src/ranker.rs`: metadata-preserving ordering.
- `cheime-core/crates/cheime-pipeline/src/factory.rs`: decoder, user lexicon, and learning-service assembly.
- `cheime-core/crates/cheime-config/src/schema.rs`: completion and sentence decoder flags.
- `cheime-core/config/schemas/base.yaml`: default QuanPin decoder behavior.
- `cheime-core/crates/cheime-session/src/state.rs`: confirmed-prefix composition and two-phase commit records.
- `cheime-core/crates/cheime-model/src/input.rs`: `CommitToken` and rollback protocol value.
- `cheime-core/crates/cheime-model/src/lib.rs`: re-export new model values and increment protocol version.
- `cheime-core/crates/cheime-protocol/src/lib.rs`: `RollbackLearning` frontend message.
- `cheime-core/crates/cheime-user-data/src/event.rs`: action-addressable pending phrase learning.
- `cheime-core/crates/cheime-user-data/src/lib.rs`: re-export pending-learning values.

### Windows parent

- `crates/cheime-engine-host/src/server.rs`: pass the learning service to each session and run expiry.
- `crates/cheime-engine-host/src/session_runner.rs`: protocol-level rollback integration tests.
- `crates/cheime-tip/src/rollback_guard.rs`: pure rollback-guard state machine.
- `crates/cheime-tip/src/key_handler.rs`: Backspace admission with an armed rollback guard.
- `crates/cheime-tip/src/tsf_interfaces.rs`: arm/disarm guard and route guarded Backspace.
- `crates/cheime-tip/src/edit_session.rs`: validated TSF Backspace-equivalent edit and rollback notification.
- `crates/cheime-tip/src/candidate_window.rs`: pass rollback-guard storage to action edit sessions.
- `crates/cheime-tip/src/io_thread.rs`: prepare and validate the new frontend message.

---

### Task 1: Offset-Preserving Pinyin Segmentation Graph

**Files:**

- Create: `cheime-core/crates/cheime-pipeline/src/segmentation.rs`
- Modify: `cheime-core/crates/cheime-pipeline/src/lib.rs`
- Modify: `cheime-core/crates/cheime-pipeline/src/segmentor.rs`
- Modify: `cheime-core/crates/cheime-pipeline/src/normalizer.rs`
- Modify: `cheime-core/crates/cheime-pipeline/src/factory.rs`
- Test: inline tests in `segmentor.rs` and `normalizer.rs`

**Interfaces:**

- Produces: `InputSpan`, `SyllableKind`, `SyllableEdge`, `SegmentationGraph`.
- Produces: `Segmentor::segment(&self, composition: &str) -> SegmentationGraph`.
- Produces: `CodeNormalizer::normalize_graph(&self, graph: &SegmentationGraph) -> SegmentationGraph`.

- [ ] **Step 1: Add failing graph tests**

Add tests that express the public graph behavior:

```rust
#[test]
fn nih_keeps_complete_ni_and_incomplete_h() {
    let graph = PinyinSegmentor::new().segment("nih");
    assert!(graph.edges_from(0).iter().any(|e|
        e.span == InputSpan::new(0, 2) &&
        e.canonical == "ni" &&
        e.kind == SyllableKind::Complete));
    assert!(graph.edges_from(2).iter().any(|e|
        e.span == InputSpan::new(2, 3) &&
        e.canonical == "h" &&
        e.kind == SyllableKind::Incomplete));
}

#[test]
fn xianshi_retains_ambiguous_first_edges() {
    let graph = PinyinSegmentor::new().segment("xianshi");
    let first: Vec<_> = graph.edges_from(0).iter()
        .filter(|edge| edge.kind == SyllableKind::Complete)
        .map(|edge| edge.canonical.as_str())
        .collect();
    assert!(first.contains(&"xi"));
    assert!(first.contains(&"xian"));
}

#[test]
fn invalid_fragment_advances_as_raw() {
    let graph = PinyinSegmentor::new().segment("ni1");
    assert!(graph.edges_from(2).iter().any(|edge|
        edge.span == InputSpan::new(2, 3) &&
        edge.kind == SyllableKind::Raw));
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```powershell
cargo test -p cheime-pipeline segmentor::tests -- --nocapture
```

Expected: compilation fails because the graph types and `edges_from` do not
exist.

- [ ] **Step 3: Implement graph values and trie traversal**

Create the graph module with validated forward edges:

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InputSpan {
    pub start: usize,
    pub end: usize,
}

impl InputSpan {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyllableKind {
    Complete,
    Incomplete,
    Raw,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyllableEdge {
    pub span: InputSpan,
    pub raw: String,
    pub canonical: String,
    pub kind: SyllableKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SegmentationGraph {
    input_len: usize,
    outgoing: Vec<Vec<SyllableEdge>>,
}

impl SegmentationGraph {
    pub fn new(input_len: usize) -> Self {
        Self {
            input_len,
            outgoing: vec![Vec::new(); input_len + 1],
        }
    }

    pub fn add_edge(&mut self, edge: SyllableEdge) {
        assert!(edge.span.start < edge.span.end);
        assert!(edge.span.end <= self.input_len);
        self.outgoing[edge.span.start].push(edge);
    }

    pub fn input_len(&self) -> usize {
        self.input_len
    }

    pub fn edges_from(&self, offset: usize) -> &[SyllableEdge] {
        self.outgoing.get(offset).map(Vec::as_slice).unwrap_or(&[])
    }
}
```

Update the trie traversal so every terminal syllable creates a `Complete`
edge, an unfinished suffix at the end creates one `Incomplete` edge, and a
position with no advancing edge receives one-byte `Raw` fallback. Sort each
edge list by `(end, canonical, kind)` for deterministic traversal.

- [ ] **Step 4: Preserve spans through normalizers**

Replace linear `normalize_all` with `normalize_graph`. For each source edge,
clone its span/raw/kind while substituting normalized canonical codes. Dedup
variants by `(span, canonical, kind)` and sort them deterministically.

- [ ] **Step 5: Run graph and pipeline tests and verify GREEN**

Run:

```powershell
cargo test -p cheime-pipeline segmentor::tests -- --nocapture
cargo test -p cheime-pipeline normalizer::tests -- --nocapture
```

Expected: all focused tests pass.

- [ ] **Step 6: Commit**

```powershell
git add crates/cheime-pipeline/src/segmentation.rs crates/cheime-pipeline/src/lib.rs crates/cheime-pipeline/src/segmentor.rs crates/cheime-pipeline/src/normalizer.rs crates/cheime-pipeline/src/factory.rs
git commit -m "refactor: model pinyin as a segmentation graph"
```

---

### Task 2: Weighted Lexicon Lookup and Word-Graph Decoder

**Files:**

- Modify: `cheime-core/crates/cheime-dictionary/src/index.rs`
- Modify: `cheime-core/crates/cheime-dictionary/src/tiered.rs`
- Create: `cheime-core/crates/cheime-pipeline/src/decoder.rs`
- Modify: `cheime-core/crates/cheime-pipeline/src/translator.rs`
- Modify: `cheime-core/crates/cheime-pipeline/src/lib.rs`
- Test: inline dictionary and decoder tests

**Interfaces:**

- Consumes: `SegmentationGraph`, `SyllableEdge`, `SyllableKind`.
- Produces: `LexiconEntry { text, code, weight, source, completion }`.
- Produces: `Lexicon::exact(code)` and `Lexicon::prefix(code, limit)`.
- Produces: `Decoder::decode(input, graph, lexicons) -> Vec<ResolvedCandidate>`.

- [ ] **Step 1: Add failing weighted lookup tests**

```rust
#[test]
fn weighted_lookup_preserves_code_weight_and_completion() {
    let idx = MemoryIndex::build(
        vec![DictEntry {
            text: "你好".into(),
            code: "ni hao".into(),
            weight: Some(200),
            stem: None,
        }],
        DeploymentGeneration::new(1),
    );
    let exact = idx.lookup_exact("ni hao");
    assert_eq!(exact[0].code, "ni hao");
    assert_eq!(exact[0].weight, 200);
    assert!(!exact[0].completion);

    let prefix = idx.lookup_prefix("ni h", 10);
    assert_eq!(prefix[0].code, "ni hao");
    assert!(prefix[0].completion);
}
```

- [ ] **Step 2: Run the dictionary test and verify RED**

Run:

```powershell
cargo test -p cheime-dictionary weighted_lookup_preserves -- --nocapture
```

Expected: compilation fails because `lookup_exact`, `lookup_prefix`, and
`LexiconEntry` do not exist.

- [ ] **Step 3: Implement equivalent memory and tiered lookup records**

Add:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LexiconEntry {
    pub text: String,
    pub code: String,
    pub weight: i64,
    pub source: String,
    pub completion: bool,
}
```

Implement exact and prefix methods for `MemoryIndex`, `TieredIndex`, and
`CompiledIndex`. Convert tiered `i32` weights to `i64`. Keep existing
candidate-returning methods temporarily as wrappers so unrelated callers keep
compiling during the refactor.

- [ ] **Step 4: Add failing decoder tests**

Use an inline lexicon fixture:

```rust
#[test]
fn incomplete_nih_decodes_to_nihao() {
    let decoder = decoder(&[
        ("你好", "ni hao", 200),
        ("你", "ni", 100),
        ("好", "hao", 100),
    ]);
    let graph = PinyinSegmentor::new().segment("nih");
    let results = decoder.decode("nih", &graph);
    let candidate = results.iter().find(|c| c.display.text == "你好").unwrap();
    assert!(candidate.complete);
    assert_eq!(candidate.canonical_code, "ni hao");
    assert!(candidate.completion);
}

#[test]
fn missing_phrase_is_composed_from_lexemes() {
    let decoder = decoder(&[
        ("旎", "ni", 90),
        ("皓", "hao", 80),
    ]);
    let graph = PinyinSegmentor::new().segment("nihao");
    let candidate = decoder.decode("nihao", &graph)
        .into_iter()
        .find(|c| c.display.text == "旎皓")
        .unwrap();
    assert!(!candidate.exact_phrase);
    assert_eq!(candidate.lexemes.len(), 2);
}
```

- [ ] **Step 5: Run decoder tests and verify RED**

Run:

```powershell
cargo test -p cheime-pipeline decoder::tests -- --nocapture
```

Expected: compilation fails because `decoder` and resolved-candidate values do
not exist.

- [ ] **Step 6: Implement bounded deterministic decoding**

Define:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedLexeme {
    pub text: String,
    pub canonical_code: String,
    pub weight: i64,
    pub source: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedCandidate {
    pub display: Candidate,
    pub consumed: InputSpan,
    pub canonical_code: String,
    pub lexemes: Vec<SelectedLexeme>,
    pub complete: bool,
    pub exact_phrase: bool,
    pub completion: bool,
    pub score: i64,
}
```

Implement a vertex-indexed beam with `BEAM_WIDTH = 32`,
`MAX_HOMOGRAPHS = 8`, and `MAX_CANDIDATES = 100`. Expand exact dictionary
matches for complete syllable sequences and prefix matches only when the last
edge is incomplete. Add prefix candidates for proper reachable spans. Assign
candidate IDs only after final sort and dedup.

- [ ] **Step 7: Remove the text-only concatenation fallback**

Make `DictTranslator` a lexicon adapter used by `Decoder`. Delete the current
`per_seg[0]` concatenation path. Update user lookup to return weights and
canonical codes through the same adapter.

- [ ] **Step 8: Run focused tests and verify GREEN**

Run:

```powershell
cargo test -p cheime-dictionary --lib
cargo test -p cheime-pipeline decoder::tests -- --nocapture
cargo test -p cheime-pipeline translator::tests -- --nocapture
```

Expected: all focused tests pass.

- [ ] **Step 9: Commit**

```powershell
git add crates/cheime-dictionary/src/index.rs crates/cheime-dictionary/src/tiered.rs crates/cheime-pipeline/src/decoder.rs crates/cheime-pipeline/src/translator.rs crates/cheime-pipeline/src/lib.rs
git commit -m "feat: decode candidates over pinyin word graphs"
```

---

### Task 3: Metadata-Preserving Pipeline and Partial Session Selection

**Files:**

- Modify: `cheime-core/crates/cheime-pipeline/src/lib.rs`
- Modify: `cheime-core/crates/cheime-pipeline/src/filter.rs`
- Modify: `cheime-core/crates/cheime-pipeline/src/ranker.rs`
- Modify: `cheime-core/crates/cheime-pipeline/src/factory.rs`
- Modify: `cheime-core/crates/cheime-session/src/state.rs`
- Test: `cheime-core/crates/cheime-session/src/state.rs`
- Test: `cheime-core/crates/cheime-session/tests/vertical_slice.rs`

**Interfaces:**

- Consumes: `ResolvedCandidate`.
- Produces: `PipelineUpdate { composition, candidates: Vec<ResolvedCandidate>, intent }`.
- Produces: session `CompositionState` with confirmed segments and active raw input.

- [ ] **Step 1: Add failing session tests for partial selection**

Build a deterministic test pipeline with:

```rust
fn prefix_candidate(id: u64) -> ResolvedCandidate {
    ResolvedCandidate {
        display: Candidate::text(CandidateId::new(id), "旎", "test"),
        consumed: InputSpan::new(0, 2),
        canonical_code: "ni".into(),
        lexemes: vec![SelectedLexeme::test("旎", "ni")],
        complete: false,
        exact_phrase: true,
        completion: false,
        score: 90,
    }
}
```

Assert:

```rust
#[test]
fn selecting_prefix_keeps_remaining_input_composing() {
    let mut session = session_with_candidates("nihao", vec![prefix_candidate(1)]);
    let output = session.handle(select_message(1)).unwrap();
    assert!(output.iter().all(|m| !matches!(m,
        EngineMessage::PlatformAction {
            action: PlatformAction { kind: PlatformActionKind::Commit { .. }, .. },
            ..
        })));
    assert_eq!(session.composition_text(), "旎hao");
    assert_eq!(session.active_input(), "hao");
}

#[test]
fn backspace_reopens_last_confirmed_segment() {
    let mut session = partially_confirmed_session("旎", "ni", "hao");
    session.handle(key_message(Key::Backspace)).unwrap();
    assert_eq!(session.composition_text(), "nihao");
    assert!(session.confirmed_segments().is_empty());
}
```

- [ ] **Step 2: Run session tests and verify RED**

Run:

```powershell
cargo test -p cheime-session selecting_prefix -- --nocapture
cargo test -p cheime-session backspace_reopens -- --nocapture
```

Expected: compilation fails because Session has no confirmed-segment state.

- [ ] **Step 3: Refactor filters and rankers around resolved candidates**

Change:

```rust
pub trait Filter: Send + Sync {
    fn name(&self) -> &str;
    fn filter(&self, candidates: Vec<ResolvedCandidate>) -> Vec<ResolvedCandidate>;
}

pub trait Ranker: Send + Sync {
    fn name(&self) -> &str;
    fn rank(&self, candidates: Vec<ResolvedCandidate>) -> Vec<ResolvedCandidate>;
}
```

Deduplicate on displayed text while retaining the highest-scoring full record.
Reassign IDs after all filters and ranking.

- [ ] **Step 4: Implement composition state and partial transitions**

Add:

```rust
#[derive(Clone, Debug, Default)]
struct CompositionState {
    raw: String,
    active_start: usize,
    confirmed: Vec<ConfirmedSegment>,
}

#[derive(Clone, Debug)]
struct ConfirmedSegment {
    text: String,
    raw_span: InputSpan,
    canonical_code: String,
    lexemes: Vec<SelectedLexeme>,
}
```

On a prefix selection, append the confirmed segment, advance `active_start`,
call the pipeline refresh path for the remaining input, and emit only
`SetPreedit` plus a composing snapshot. On a complete selection, concatenate
confirmed and selected text and propose the existing two-phase commit.

Backspace deletes active raw input first. When no active raw character precedes
the caret, pop the last confirmed segment, move `active_start` back to its
start, and refresh.

- [ ] **Step 5: Preserve rejected-commit state**

Store the full proposed commit record in `PendingEffect`. Clear composition
only for `Applied`. For `Rejected`, remove the pending action but retain raw,
confirmed segments, resolved candidates, page, and highlight.

- [ ] **Step 6: Run session and pipeline tests and verify GREEN**

Run:

```powershell
cargo test -p cheime-session -- --nocapture
cargo test -p cheime-pipeline --lib
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```powershell
git add crates/cheime-pipeline/src/lib.rs crates/cheime-pipeline/src/filter.rs crates/cheime-pipeline/src/ranker.rs crates/cheime-pipeline/src/factory.rs crates/cheime-session/src/state.rs crates/cheime-session/tests/vertical_slice.rs
git commit -m "feat: support partial candidate confirmation"
```

---

### Task 4: Action-Addressable Delayed Phrase Learning

**Files:**

- Modify: `cheime-core/crates/cheime-model/src/input.rs`
- Modify: `cheime-core/crates/cheime-model/src/lib.rs`
- Modify: `cheime-core/crates/cheime-user-data/src/event.rs`
- Modify: `cheime-core/crates/cheime-user-data/src/lib.rs`
- Modify: `cheime-core/crates/cheime-pipeline/src/lib.rs`
- Modify: `cheime-core/crates/cheime-pipeline/src/factory.rs`
- Modify: `cheime-core/crates/cheime-session/src/state.rs`
- Test: inline user-data and session tests

**Interfaces:**

- Produces: `CommitToken { session, epoch, action_id }`.
- Produces: `PendingPhrase`, `stage_phrase`, `cancel_phrase`,
  `confirm_expired`.
- Produces: `LearningService<C: Clock>` and `InputPipeline` lifecycle hooks.

- [ ] **Step 1: Add failing deterministic-time user-data tests**

```rust
#[test]
fn pending_phrase_is_learned_only_at_deadline() {
    let mut store = UserStore::new("test");
    let token = token(1);
    store.stage_phrase(token, phrase("旎皓", "ni hao"), 10_000);
    store.confirm_expired(9_999);
    assert!(store.query("ni hao").is_empty());
    store.confirm_expired(10_000);
    assert_eq!(store.query("ni hao")[0].text, "旎皓");
}

#[test]
fn rollback_before_deadline_never_persists_phrase() {
    let mut store = UserStore::new("test");
    let token = token(2);
    store.stage_phrase(token, phrase("旎皓", "ni hao"), 10_000);
    assert!(store.cancel_phrase(token));
    assert!(!store.cancel_phrase(token));
    store.confirm_expired(20_000);
    assert!(store.query("ni hao").is_empty());
}

#[test]
fn concurrent_session_action_ids_do_not_collide() {
    let mut store = UserStore::new("test");
    store.stage_phrase(token_for_session(1, 1), phrase("甲", "jia"), 10);
    store.stage_phrase(token_for_session(2, 1), phrase("乙", "yi"), 10);
    store.confirm_expired(10);
    assert_eq!(store.query("jia")[0].text, "甲");
    assert_eq!(store.query("yi")[0].text, "乙");
}
```

- [ ] **Step 2: Run user-data tests and verify RED**

Run:

```powershell
cargo test -p cheime-user-data pending_phrase -- --nocapture
cargo test -p cheime-user-data concurrent_session_action -- --nocapture
```

Expected: compilation fails because action-addressable learning APIs do not
exist.

- [ ] **Step 3: Implement commit tokens and pending map**

Add the serializable model:

```rust
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct CommitToken {
    pub session: SessionId,
    pub epoch: SessionEpoch,
    pub action_id: ActionId,
}
```

Replace `pending: Vec<PendingLearn>` with
`HashMap<CommitToken, PendingPhrase>`. Store explicit millisecond deadlines
supplied by the caller. `confirm_expired(now_ms)` drains only entries where
`deadline_ms <= now_ms`, sorts by token before applying events, and is
idempotent.

- [ ] **Step 4: Add failing Applied/Rejected session tests**

```rust
#[test]
fn applied_novel_phrase_stages_learning() {
    let learning = RecordingLearningSink::default();
    let mut session = novel_phrase_session(learning.clone());
    let action = select_complete_phrase(&mut session);
    session.handle(applied(action.id)).unwrap();
    assert_eq!(learning.staged()[0].text, "旎皓");
    assert_eq!(learning.staged()[0].canonical_code, "ni hao");
}

#[test]
fn rejected_novel_phrase_does_not_stage_learning() {
    let learning = RecordingLearningSink::default();
    let mut session = novel_phrase_session(learning.clone());
    let action = select_complete_phrase(&mut session);
    session.handle(rejected(action.id)).unwrap();
    assert!(learning.staged().is_empty());
}
```

- [ ] **Step 5: Run session tests and verify RED**

Run:

```powershell
cargo test -p cheime-session novel_phrase -- --nocapture
```

Expected: assertions fail because no learning hook runs on action results.

- [ ] **Step 6: Add pipeline learning hooks and stage on Applied**

Extend `InputPipeline` with default no-op hooks:

```rust
fn commit_applied(&self, token: CommitToken, record: CommitRecord) {}
fn rollback_learning(&self, token: CommitToken) {}
```

`ComposablePipeline` owns an optional shared `LearningService` wrapping the
store and a `Clock`. Production uses one process-local `Instant` clock; tests
use `FakeClock::set(now_ms)`. `Session` keeps `CommitRecord` in the matching
pending effect and calls `commit_applied` only after `Applied`. Stage only
records with `exact_phrase == false`; exact entries update normal frequency
without creating a rollback ticket.

- [ ] **Step 7: Run user-data and session tests and verify GREEN**

Run:

```powershell
cargo test -p cheime-user-data --lib
cargo test -p cheime-session --lib
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```powershell
git add crates/cheime-model/src/input.rs crates/cheime-model/src/lib.rs crates/cheime-user-data/src/event.rs crates/cheime-user-data/src/lib.rs crates/cheime-pipeline/src/lib.rs crates/cheime-pipeline/src/factory.rs crates/cheime-session/src/state.rs
git commit -m "feat: delay learning for newly composed phrases"
```

---

### Task 5: Rollback Protocol and Engine-Host Expiry

**Files:**

- Modify: `cheime-core/crates/cheime-model/src/lib.rs`
- Modify: `cheime-core/crates/cheime-protocol/src/lib.rs`
- Modify: `cheime-core/crates/cheime-protocol/src/serde_tests.rs`
- Modify: `cheime-core/crates/cheime-session/src/state.rs`
- Modify: `crates/cheime-engine-host/src/main.rs`
- Modify: `crates/cheime-engine-host/src/server.rs`
- Modify: `crates/cheime-engine-host/src/session_runner.rs`

**Interfaces:**

- Consumes: `CommitToken`, pipeline learning hooks.
- Produces: `FrontendMessage::RollbackLearning`.
- Produces: a periodic `LearningService::confirm_expired()` engine-host worker.

- [ ] **Step 1: Add failing protocol round-trip and session rollback tests**

```rust
#[test]
fn rollback_learning_roundtrips() {
    let message = FrontendMessage::RollbackLearning {
        header: header(),
        token: CommitToken {
            session: SessionId::new(2),
            epoch: SessionEpoch::new(3),
            action_id: ActionId::new(4),
        },
    };
    let bytes = rmp_serde::to_vec_named(&message).unwrap();
    assert_eq!(rmp_serde::from_slice::<FrontendMessage>(&bytes).unwrap(), message);
}

#[test]
fn rollback_is_forwarded_idempotently() {
    let learning = RecordingLearningSink::with_pending(token(1));
    let mut session = session_with_learning(learning.clone());
    session.handle(rollback_message(token(1))).unwrap();
    session.handle(rollback_message(token(1))).unwrap();
    assert_eq!(learning.cancel_attempts(), 2);
    assert_eq!(learning.cancelled(), vec![token(1)]);
}
```

- [ ] **Step 2: Run protocol/session tests and verify RED**

Run:

```powershell
cargo test -p cheime-protocol rollback_learning -- --nocapture
cargo test -p cheime-session rollback_is_forwarded -- --nocapture
```

Expected: compilation fails because the frontend variant is absent.

- [ ] **Step 3: Add the protocol message and bump the protocol version**

Add `RollbackLearning { header, token }` to `FrontendMessage`, include it in
`header()`, message preparation, and session dispatch. Increment
`CORE_PROTOCOL_VERSION` from `1` to `2`. Header validation rejects another
epoch/session but an unknown token in the current session remains a no-op.

- [ ] **Step 4: Add failing expiry-worker unit test**

Extract a single tick:

```rust
#[test]
fn expiry_tick_confirms_shared_store() {
    let clock = Arc::new(FakeClock::new(0));
    let service = LearningService::new(
        Arc::new(Mutex::new(UserStore::new("test"))),
        clock.clone(),
    );
    service.stage_phrase(token(1), phrase("旎皓", "ni hao"));
    clock.set(10_000);
    confirm_expired_tick(&service);
    assert_eq!(service.store().lock().query("ni hao")[0].text, "旎皓");
}
```

- [ ] **Step 5: Implement host expiry worker**

Add:

```rust
fn confirm_expired_tick(service: &LearningService) {
    service.confirm_expired();
}
```

Start one named background thread for the process, not one per client. Poll at
250 milliseconds and share the same `LearningService` with every pipeline.

- [ ] **Step 6: Run core and host tests and verify GREEN**

Run:

```powershell
cargo test -p cheime-protocol
cargo test -p cheime-session
cargo test -p cheime-engine-host
```

Expected: all tests pass.

- [ ] **Step 7: Commit core protocol changes**

From `cheime-core`:

```powershell
git add crates/cheime-model/src/lib.rs crates/cheime-protocol/src/lib.rs crates/cheime-protocol/src/serde_tests.rs crates/cheime-session/src/state.rs
git commit -m "feat: add learning rollback protocol"
```

- [ ] **Step 8: Commit engine-host changes**

From `cheime-win`:

```powershell
git add crates/cheime-engine-host/src/main.rs crates/cheime-engine-host/src/server.rs crates/cheime-engine-host/src/session_runner.rs
git commit -m "feat: expire pending phrase learning in engine host"
```

---

### Task 6: Windows Rollback Guard and Guarded Backspace

**Files:**

- Create: `crates/cheime-tip/src/rollback_guard.rs`
- Modify: `crates/cheime-tip/src/lib.rs`
- Modify: `crates/cheime-tip/src/key_handler.rs`
- Modify: `crates/cheime-tip/src/tsf_interfaces.rs`
- Modify: `crates/cheime-tip/src/edit_session.rs`
- Modify: `crates/cheime-tip/src/candidate_window.rs`
- Modify: `crates/cheime-tip/src/io_thread.rs`
- Test: inline pure tests plus existing Windows edit-session tests

**Interfaces:**

- Consumes: applied `PlatformActionKind::Commit`, `CommitToken`.
- Produces: pure `RollbackGuard` state transitions.
- Produces: guarded TSF deletion and `RollbackLearning`.

- [ ] **Step 1: Add failing pure rollback-guard tests**

```rust
#[test]
fn guard_is_valid_only_for_same_context_before_deadline() {
    let guard = RollbackGuard::armed(token(1), 42, 10_000);
    assert!(guard.matches(42, 9_999));
    assert!(!guard.matches(41, 9_999));
    assert!(!guard.matches(42, 10_000));
}

#[test]
fn text_or_navigation_disarms_guard() {
    for event in [
        GuardEvent::TextInput,
        GuardEvent::Navigation,
        GuardEvent::FocusChanged,
    ] {
        let mut guard = RollbackGuard::armed(token(1), 42, 10_000);
        guard.observe(event);
        assert!(!guard.is_armed());
    }
}
```

- [ ] **Step 2: Run guard tests and verify RED**

Run:

```powershell
cargo test -p cheime-tip rollback_guard -- --nocapture
```

Expected: compilation fails because the module does not exist.

- [ ] **Step 3: Implement pure guard state**

Create a platform-neutral-in-tests state object:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardEvent {
    TextInput,
    Navigation,
    FocusChanged,
    Expired,
}

#[derive(Clone, Debug)]
pub struct RollbackGuard {
    token: Option<CommitToken>,
    context_identity: usize,
    deadline_ms: u64,
}
```

The guard stores no COM interface. The owning TIP state separately stores a
cloned collapsed `ITfRange` anchor while the pure object stores identity and
deadline.

- [ ] **Step 4: Add failing key-admission test**

```rust
#[test]
fn backspace_is_handled_for_rollback_without_composition() {
    assert_eq!(
        check_key(
            InputMode::Chinese,
            true,
            VK_BACK,
            false,
            false,
            false,
            false,
            true,
        ),
        KeyAdmission::Handled,
    );
}
```

Extend `check_key` with `has_rollback_guard: bool`. Update all callers/tests
mechanically.

- [ ] **Step 5: Run admission test and verify RED**

Run:

```powershell
cargo test -p cheime-tip backspace_is_handled_for_rollback -- --nocapture
```

Expected: the old admission function passes Backspace through.

- [ ] **Step 6: Arm guard only after successful commit edit**

When `PlatformActionKind::Commit` succeeds, clone and collapse the resulting
selection range, compute the matching `CommitToken` from the acknowledged
session identity and action, and store both the pure guard and anchor. A failed
commit leaves the guard disarmed.

- [ ] **Step 7: Implement guarded TSF Backspace**

On Backspace with no composition and an armed guard:

1. request a synchronous TSF edit session;
2. compare current collapsed selection with the saved anchor;
3. if equal, shift a cloned range one user-visible character backward and
   replace it with empty text;
4. if not equal, perform the same ordinary previous-character deletion at the
   current selection but set `should_rollback = false`;
5. send `FrontendMessage::RollbackLearning` only when deletion succeeded and
   `should_rollback` is true;
6. disarm in every completion branch.

Use UTF-16-safe `ITfRange::ShiftStart`/`ShiftEnd` operations; do not calculate
byte offsets in TIP code.

- [ ] **Step 8: Run TIP tests and verify GREEN**

Run:

```powershell
cargo test -p cheime-tip
```

Expected: all TIP tests pass.

- [ ] **Step 9: Commit**

```powershell
git add crates/cheime-tip/src/rollback_guard.rs crates/cheime-tip/src/lib.rs crates/cheime-tip/src/key_handler.rs crates/cheime-tip/src/tsf_interfaces.rs crates/cheime-tip/src/edit_session.rs crates/cheime-tip/src/candidate_window.rs crates/cheime-tip/src/io_thread.rs
git commit -m "feat: cancel phrase learning on guarded backspace"
```

---

### Task 7: Acceptance Coverage, Performance Guard, and Integration

**Files:**

- Modify: `cheime-core/crates/cheime-pipeline/tests/stress_tests.rs`
- Modify: `cheime-core/crates/cheime-pipeline/benches/pipeline_bench.rs`
- Modify: `cheime-core/crates/cheime-session/tests/vertical_slice.rs`
- Modify: `cheime-core/crates/cheime-config/src/schema.rs`
- Modify: `cheime-core/config/schemas/base.yaml`
- Modify: `tests/integration/fake_ime_roundtrip.rs`
- Modify: `docs/getting-started.md`
- Update: `cheime-core` submodule pointer in parent repository

**Interfaces:**

- Consumes all previous tasks.
- Produces end-to-end acceptance evidence and a recorded parent/submodule state.

- [ ] **Step 1: Add failing real-dictionary acceptance test**

```rust
#[test]
fn incomplete_nih_produces_nihao() {
    let pipeline = real_pipeline();
    let update = type_text(&pipeline, "nih");
    assert!(update.candidates.iter().any(|candidate|
        candidate.display.text == "你好"));
}
```

- [ ] **Step 2: Add failing constructed-phrase vertical slice**

Use an inline dictionary that intentionally lacks the complete phrase. Send
keys for `nihao`, select a one-syllable prefix, select the remaining syllable,
acknowledge the commit, expire learning, start a fresh composition, and assert
the whole phrase is returned from `user_dict`.

- [ ] **Step 3: Run acceptance tests and verify RED**

Run:

```powershell
cargo test -p cheime-pipeline --test stress_tests incomplete_nih -- --nocapture
cargo test -p cheime-session --test vertical_slice constructed_phrase -- --nocapture
```

Expected: the new tests fail until every integration path is connected.

- [ ] **Step 4: Wire completion, sentence, and user-lexicon configuration**

Modify `DictTranslatorConfig` and `TableTranslatorConfig` so both expose
`enable_completion: bool` and `enable_sentence: bool`, each defaulting to
`true`. Build:

```rust
DecoderOptions {
    enable_completion: config.enable_completion,
    enable_sentence: config.enable_sentence,
}
```

Add `UserLexicon` whenever `user_store.is_some()` before processing configured
static translators; do not limit it to the `out.is_empty()` fallback. Wrap
emoji and passthrough candidates as `ResolvedCandidate` values consuming the
full active input with empty lexeme lists. Add explicit `enable_completion:
true` and `enable_sentence: true` entries to `config/schemas/base.yaml`.

- [ ] **Step 5: Add a decoder benchmark guard**

Benchmark `nih`, `nihao`, `xianshi`, and a 20-syllable sentence. Record
candidate count with `black_box`. The benchmark is informational; the release
gate is that a single `nihao` decode remains below 1 ms on the development
machine.

- [ ] **Step 6: Run full verification**

From `cheime-core`:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

From `cheime-win`:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: every command exits with code 0 and reports no warnings or failures.

- [ ] **Step 7: Update user documentation**

Document:

- incomplete Pinyin completion;
- Space/digit/click partial selection;
- Backspace reopening a confirmed segment;
- new phrases becoming candidates after ten seconds;
- immediate guarded Backspace preventing phrase learning.

- [ ] **Step 8: Commit core acceptance coverage**

From `cheime-core`:

```powershell
git add crates/cheime-pipeline/tests/stress_tests.rs crates/cheime-pipeline/benches/pipeline_bench.rs crates/cheime-session/tests/vertical_slice.rs
git commit -m "test: cover incomplete pinyin and phrase construction"
```

- [ ] **Step 9: Commit parent integration**

From `cheime-win`:

```powershell
git add cheime-core tests/integration/fake_ime_roundtrip.rs docs/getting-started.md
git commit -m "feat: integrate rime-style phrase composition"
```
