# Rime-Style Composition and Learning Design

**Date:** 2026-07-24

**Status:** Proposed

## Goal

Improve CheIME's Pinyin lookup and learning so that:

- incomplete Pinyin can produce completed candidates, including `nih` → `你好`;
- ambiguous Pinyin segmentation does not depend on a single greedy split;
- a user can select one segment at a time while the remaining Pinyin stays active;
- a phrase absent from the static dictionary can be composed from smaller dictionary entries;
- the composed phrase becomes a user-dictionary candidate after a successful TSF commit;
- deleting the newly committed phrase within ten seconds cancels that learning.

The interaction follows the common Rime-style model: the engine may offer a
whole-sentence candidate and shorter prefix candidates; selecting a prefix
confirms only that segment, while the remaining input continues composing.

## Repository Scope

The implementation spans two repositories:

- `cheime-win/cheime-core`: platform-independent segmentation, decoding,
  session state, protocol values, and user learning;
- `cheime-win`: engine-host timers and Windows TIP rollback handling.

The independent `D:/coding/cheime/cheime-core` checkout is not edited during
this work because it contains unrelated uncommitted changes. Core changes are
implemented and committed in the `cheime-win/cheime-core` submodule, then the
parent repository records that submodule revision.

## Existing Limitations

The current `PinyinSegmentor` returns one greedy leftmost-longest sequence.
The current `DictTranslator` then performs one exact or prefix lookup. When a
multi-segment phrase is missing, it concatenates only the first result of each
segment. This loses:

- alternative segmentation paths;
- the raw-input range consumed by a candidate;
- the dictionary elements that formed a sentence;
- enough provenance to learn the selected phrase correctly.

`Session` stores only display candidates and treats every candidate selection
as a full commit. It cannot confirm a prefix and continue composing.

`UserStore` has a pending-learning vector, but its current behavior confirms
the previous item on the next commit. It has no ten-second expiry and the
Windows engine host never stages learning after a TSF `Applied` result.

The Windows TIP passes Backspace through when no composition exists. The
engine therefore cannot observe deletion of newly committed text.

## Behavioral Contract

### Incomplete Pinyin

An unfinished final syllable is a valid graph edge if it is a prefix of at
least one legal Pinyin syllable. It is marked `Incomplete`; it is not silently
rewritten into a complete syllable.

For `nih`, the graph contains:

```text
0 --"ni"/Complete--> 2 --"h"/Incomplete--> 3
```

The decoder may complete `h` to `hao`, query the canonical code `ni hao`, and
return `你好`. Completion candidates rank below an otherwise equivalent exact
candidate.

Invalid internal fragments do not trigger an unbounded dictionary scan. If no
valid edge can advance from a byte position, the segmentor emits one
`Raw` edge so that input remains editable and can fall back to raw display.

### Whole-Sentence and Prefix Candidates

The candidate list contains:

1. complete sentence candidates that consume all active input;
2. exact dictionary words that consume a proper prefix and can be selected for
   manual composition;
3. completion candidates;
4. a raw-input fallback.

Selecting a complete sentence commits it. Selecting a prefix appends the
candidate to the confirmed portion of the composition and refreshes
candidates for the remaining raw input.

Digit keys, Space, and candidate-window clicks use the same selection
transition. Enter keeps the current CheIME behavior and commits raw Pinyin.

### Backspace While Composing

Backspace has two composition behaviors:

- if unconfirmed raw input remains, delete its previous input character;
- if the caret is at the beginning of the unconfirmed part and at least one
  segment is confirmed, reopen the most recently confirmed segment and make
  its original Pinyin editable again.

Reopening a segment removes it from the future learning record.

### Learning

Learning is considered only after the corresponding TSF `Commit` action
returns `Applied`. A rejected action never changes user data.

The commit record contains:

- committed text;
- canonical, space-separated Pinyin;
- schema identity;
- the selected lexeme sequence;
- whether the complete text was already an exact dictionary phrase;
- a commit token containing session, epoch, and action identity.

Existing dictionary candidates update normal user frequency. A newly composed
phrase is staged as a pending learn and becomes queryable only after its
ten-second rollback window expires.

### Ten-Second Rollback Window

The Windows TIP arms a rollback guard after it successfully applies a commit.
The guard contains the action ID, focused TSF context identity, a collapsed
range at the end of the committed text, and a monotonic deadline.

The guard remains valid only while:

- the same TSF context is focused;
- the current selection is still collapsed at the recorded end position;
- no non-modifier text/navigation key has been processed;
- fewer than ten seconds have elapsed.

If Backspace arrives while the guard is valid, the TIP handles that Backspace
through a TSF edit session, performs the ordinary deletion, and sends an
idempotent learning-rollback message for the action ID. The first such deletion
cancels the whole pending phrase so that a phrase being corrected is not
learned. Subsequent Backspaces behave normally.

Focus changes, cursor or selection movement, other text input, expiry, or a
failed range check disarm the guard without changing learning.

## Core Architecture

### Segmentation Graph

Replace the linear `Vec<CodeSegment>` contract with:

```rust
pub struct InputSpan {
    pub start: usize,
    pub end: usize,
}

pub enum SyllableKind {
    Complete,
    Incomplete,
    Raw,
}

pub struct SyllableEdge {
    pub span: InputSpan,
    pub raw: String,
    pub canonical: String,
    pub kind: SyllableKind,
}

pub struct SegmentationGraph {
    pub input_len: usize,
    pub outgoing: Vec<Vec<SyllableEdge>>,
}
```

Offsets are UTF-8 byte offsets because Pinyin input is ASCII and Rust string
slicing then remains explicit and cheap. The graph is acyclic: every edge has
`end > start`.

`Segmentor::segment` returns `SegmentationGraph`. `PinyinSegmentor` walks the
syllable trie from every reachable input position rather than taking only the
longest match. Apostrophes form hard boundaries and are not included in the
canonical code.

The normalizer transforms graph edges while preserving their input spans.
Fuzzy Pinyin and abbreviation variants therefore do not erase the relationship
between displayed input and consumed input.

### Lexicon Matches

Dictionary lookup returns internal lexicon records before converting them to
display candidates:

```rust
pub struct LexiconMatch {
    pub text: String,
    pub canonical_code: String,
    pub weight: i64,
    pub source: String,
    pub completion: bool,
}
```

Both memory and tiered indexes expose equivalent exact and prefix lookup
semantics. The index layer preserves weights and canonical codes; candidate IDs
are assigned only after decoding and ranking, preventing translator-local ID
collisions.

### Word-Graph Decoder

The decoder expands reachable segmentation edges and exact dictionary matches
into a word graph. An incomplete final edge may use prefix lookup. Static and
user dictionaries participate through the same match interface.

Every decoder hypothesis contains:

- current raw-input offset;
- accumulated text;
- canonical syllables;
- selected lexemes;
- accumulated score;
- whether any completion was used.

Initial limits are constants, not new configuration:

- at most 32 live hypotheses per graph vertex;
- at most 100 resolved candidates before menu pagination;
- at most 8 homographs for one code span.

Scoring is deterministic:

1. dictionary weight;
2. user frequency bonus;
3. exact-match bonus over completion;
4. fewer-lexeme bonus when other scores tie;
5. lexical text order as the final stable tie-breaker.

This phase does not add a neural model or an n-gram language-model file. The
decoder boundary allows a later ranker to replace the simple score without
changing session semantics.

When no whole phrase exists, single-character or shorter-word edges keep the
graph reachable. Their best paths form sentence candidates. Manual prefix
candidates let the user choose a lower-ranked character and continue, so a
phrase absent from the static dictionary is still constructible.

### Resolved Candidates

Display candidates remain protocol-friendly, but `PipelineUpdate` carries
internal selection metadata:

```rust
pub struct ResolvedCandidate {
    pub display: Candidate,
    pub consumed: InputSpan,
    pub canonical_code: String,
    pub lexemes: Vec<SelectedLexeme>,
    pub complete: bool,
    pub exact_phrase: bool,
}
```

Filters and rankers operate on `ResolvedCandidate` so metadata cannot become
detached from the displayed text. The final pipeline stage assigns stable,
unique candidate IDs.

## Session State

Replace the single composition string with:

```rust
pub struct CompositionState {
    pub raw: String,
    pub active_start: usize,
    pub confirmed: Vec<ConfirmedSegment>,
}

pub struct ConfirmedSegment {
    pub text: String,
    pub raw_span: InputSpan,
    pub canonical_code: String,
    pub lexemes: Vec<SelectedLexeme>,
}
```

`Session` retains the full `ResolvedCandidate` list while snapshots expose only
their display values.

Candidate selection performs one of two transitions:

- `candidate.complete == false`: append a confirmed segment, advance
  `active_start`, decode the remainder, update preedit, and do not emit a
  platform commit;
- `candidate.complete == true`: concatenate confirmed text and candidate text,
  create a pending commit record, and emit the normal two-phase TSF commit.

The preedit sent to the TIP is confirmed Chinese text followed by remaining raw
Pinyin. Its cursor is at the end. The session still clears composition only
after `PlatformActionOutcome::Applied`.

## Learning Lifecycle

`InputPipeline` gains lifecycle hooks with default no-op implementations so
test pipelines and non-learning pipelines remain simple:

```rust
fn commit_applied(&self, record: CommitRecord, now: MonotonicTime);
fn rollback_learning(&self, token: CommitToken, now: MonotonicTime);
fn confirm_expired_learning(&self, now: MonotonicTime);
```

`ComposablePipeline` forwards these hooks to a dedicated learning service that
owns the shared `UserStore`. Translators only query user data; they do not
decide commit lifecycle.

`UserStore` replaces `commit_pending`, `undo_last`, and
`confirm_all_pending` with action-addressable operations:

```rust
stage_phrase(token, record, expires_at)
cancel_phrase(token)
confirm_expired(now)
```

The pending map is keyed by `(session_id, epoch, action_id)` so action counters
from concurrent sessions cannot collide. Cancellation and expiry are
idempotent. An expired entry is converted to the existing `LearnWord` event
and persisted through the existing SQLite path.

The engine host runs a small periodic worker against the shared store so an
idle phrase becomes learned after ten seconds. Tests call the same expiry
method with explicit times; production code is the only place that reads a
monotonic clock.

## Protocol and Windows TIP

Add a frontend message carrying the commit token:

```rust
FrontendMessage::RollbackLearning {
    header: MessageHeader,
    token: CommitToken,
}
```

The message is accepted only for the current session and epoch. An unknown,
already-cancelled, or expired action ID is a successful no-op. This makes a
late or duplicate frontend notification harmless.

The protocol version is incremented because both core and TIP are shipped
together and the new frontend variant changes the wire contract.

The TIP adds a small `RollbackGuard` state object. Key-admission logic remains
pure: it receives whether a valid guard is armed and handles Backspace when
either a composition or rollback guard exists.

On guarded Backspace, a TSF edit session:

1. obtains the current selection;
2. verifies that it is collapsed at the saved commit-end range;
3. deletes the preceding user-perceived character using a TSF range;
4. reports `Applied` locally;
5. sends `RollbackLearning` and disarms the guard.

If validation fails, the edit session performs an ordinary Backspace-equivalent
delete at the current selection but does not send rollback feedback. This
prevents swallowing the user's Backspace after a mouse or caret move.

## Error Handling

- A malformed graph edge or backward span is a pipeline error in tests and a
  raw fallback in production assembly.
- Dictionary lookup failure yields no lexicon edge; it does not abort the
  session.
- Beam limits truncate deterministically and never depend on hash-map order.
- A rejected TSF commit retains composition and creates no learning ticket.
- SQLite persistence failure preserves in-memory candidates and reports through
  the existing diagnostic path; it does not block input.
- A rollback for another epoch or session is rejected by header validation.
- A rollback for an unknown action in the correct session is idempotently
  ignored.

## Testing Strategy

### Segmentor

- `nih` produces complete `ni` and incomplete `h` edges.
- `xianshi` retains both valid ambiguous paths.
- apostrophes prevent cross-boundary syllables.
- invalid input advances through a raw edge and never loops.

### Dictionary and Decoder

Use inline dictionaries rather than the 539K-entry fixture:

- incomplete `ni h` resolves exact phrase code `ni hao` to `你好`;
- a missing phrase is composed from single-character entries;
- an alternative prefix character can be selected and the remainder decoded;
- memory and tiered indexes return equivalent match metadata;
- candidate ordering is deterministic under score ties;
- candidate IDs are unique after multiple sources are merged.

### Session

- selecting a prefix does not emit `Commit`;
- the snapshot shows confirmed Chinese plus remaining Pinyin;
- selecting the final segment emits one commit with the full composed text;
- Backspace reopens the last confirmed segment;
- rejected platform actions retain confirmed and active state;
- only `Applied` stages learning.

### User Data

Use explicit synthetic times:

- a new phrase is absent before expiry;
- rollback at 9.999 seconds cancels it;
- expiry at 10 seconds learns it once;
- duplicate rollback and duplicate expiry are no-ops;
- SQLite reopen returns the learned phrase;
- a cancelled phrase is never written.

### Engine Host and Protocol

- successful action acknowledgement stages the exact canonical code;
- rejected acknowledgement does not stage;
- rollback reaches the matching action ticket;
- stale-session rollback is rejected;
- the periodic expiry worker learns idle pending phrases.

### Windows TIP

- key admission handles Backspace with an armed guard and no composition;
- focus change, other text input, navigation, and expiry disarm the guard;
- matching collapsed selection deletes and sends rollback;
- moved selection deletes normally without rollback;
- commit failure does not arm a guard.

### Acceptance Scenarios

1. Type `nih`; `你好` appears.
2. Type a phrase not present as a complete dictionary entry, choose its first
   character, then choose the remaining characters; the whole phrase commits.
3. After ten seconds, type the same Pinyin; the composed phrase appears from
   the user dictionary with learned priority.
4. Repeat the composition, then immediately Backspace within ten seconds; the
   new phrase is not learned.
5. Commit a new phrase, move the caret, and press Backspace; ordinary deletion
   occurs but the learning rollback is not attributed to that phrase.

## Non-Goals

- neural language models;
- cloud synchronization;
- learning phrases assembled across separate, already committed compositions;
- observing arbitrary application-side deletion methods such as toolbar
  actions;
- mirroring changes into the dirty independent core checkout.
