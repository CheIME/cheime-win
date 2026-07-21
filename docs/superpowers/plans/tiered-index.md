# Tiered Index Plan (Revised)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep ~10MB in memory for top-N entries per code, place the rest in a sorted binary disk index. Cut 2.45M-entry memory from ~150MB to ~12MB with <100µs cold lookup.

**Architecture:** Hot index = sorted `Vec<(String, Vec<&InMemEntry>)>` (continuous, ~2MB codes + ~8MB entries). Cold index = two-level sorted binary file: Level 1 sorted by code string (not hash) for prefix binary search, Level 2 block table with text offsets into a shared u16-length-prefixed UTF-8 pool. mmap isolated in `cheime-tidx` (separate crate, allows `unsafe_code`); `cheime-dictionary` stays `forbid(unsafe_code)`.

**Tech Stack:** Rust 2024, `rmp-serde`, SHA-256, `memmap2` (mmap crate).

## Global Constraints

- `cheime-dictionary` crate MUST remain `#![forbid(unsafe_code)]`
- mmap/writer logic lives in `cheime-tidx` with `#![allow(unsafe_code)]`
- Prefix query by string comparison, NOT hash — existing `query_prefix` behavior preserved
- Hot entries per code: configurable via `SchemaConfig` (default 5)
- Backward compatible: `index_mode: memory` (default) keeps current behavior

---

## File Structure

```
cheime-core/crates/
├── cheime-dictionary/          (safe, no unsafe)
│   └── src/
│       ├── index.rs            → add CompiledIndex::Tiered(TieredIndex) variant
│       ├── tiered.rs           → TieredIndex: hot Vec + Arc<Tidex>
│       └── cache.rs            → build_hot_cold_fragment()
├── cheime-tidx/                (NEW, allows unsafe_code)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs              → re-exports
│       ├── format.rs           → TidexHeader + write/read structs
│       └── reader.rs           → TidexReader: mmap + binary search + block scan
├── cheime-config/
│   └── src/schema.rs           → add index_mode + hot_entries_per_code fields
└── cheime-pipeline/
    └── src/translator.rs       → TieredIndex::query/query_prefix impl
```

### Design: Why Sorted Array, Not Hash

Original plan used `fnv1a(code)` hash for Level-1 index. This breaks `query_prefix`: there's no way to find all codes starting with `"ni"` in a hash table without full scan.

Correct approach: Level-1 is a **sorted array of code strings**. Binary search finds the start of a prefix range (`"ni"` ≤ code < `"ni\u{FFFF}"`), then iterate forward until code no longer starts with prefix. This is O(log N + K) like BTreeMap but with no tree pointer overhead (~200KB vs ~4MB for 100K codes).

---

## Task Decomposition

### Task 1: Tidex Disk Format (cheime-tidx)

**Files:**
- Create: `cheime-core/crates/cheime-tidx/Cargo.toml`
- Create: `cheime-core/crates/cheime-tidx/src/lib.rs`
- Create: `cheime-core/crates/cheime-tidx/src/format.rs`
- Create: `cheime-core/crates/cheime-tidx/src/reader.rs`

**Interfaces:**
- Produces: `TidexHeader`, `TidexWriter`, `TidexReader`

- [ ] **Step 1: Create crate skeleton**

```toml
# Cargo.toml
[package]
name = "cheime-tidx"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
memmap2 = "0.9"
thiserror.workspace = true
```

```rust
// lib.rs
#![allow(unsafe_code)]
pub mod format;
pub mod reader;
```

- [ ] **Step 2: Write header and writer format**

```rust
// format.rs
pub const TIDX_MAGIC: [u8; 4] = *b"TIDX";
pub const TIDX_VERSION: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TidexHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub code_count: u32,       // number of unique codes (Level-1 entries)
    pub entry_count: u32,      // total entries across all codes (Level-2 entries)
    pub text_pool_size: u32,   // bytes in text pool
    pub code_idx_off: u32,     // byte offset to Level-1 array
    pub block_tbl_off: u32,    // byte offset to Level-2 block table
    pub text_pool_off: u32,    // byte offset to text pool
}
// 7 × 4 = 28 bytes + 4 magic = 32 bytes

/// Level 1: sorted array of (code_len_u16, code_utf8, first_block_idx_u32).
/// Packed: [u16 code_len][u8; code_len][u32 first_block_idx] per entry.
/// Binary searched by code string for exact + prefix queries.

/// Level 2: flat array of (text_pool_offset_u32, weight_i32).
/// 8 bytes per entry, ordered by (code, weight_desc).
/// start, count derived from Level-1 first_block_idx[i]..first_block_idx[i+1].

/// Text pool: [u16 len][utf8 bytes] repeating. Total: text_pool_size bytes.

pub fn write_tidex(
    path: &std::path::Path,
    code_entries: &[(&str, &[(String, Option<i64>)])],
) -> Result<(), std::io::Error> {
    // 1. Compute text pool — dedup texts, assign offsets
    // 2. Write header (placeholder offsets, fill after writing)
    // 3. Write Level-1: for each code, write len+code+first_block_idx
    // 4. Write Level-2: for each entry, write text_off+weight
    // 5. Write text pool
    // 6. Seek back to header, write final offsets
    unimplemented!()
}
```

- [ ] **Step 3: Write reader with binary search and prefix scan**

```rust
// reader.rs
use memmap2::Mmap;
use std::path::Path;

pub struct TidexReader {
    _mmap: Mmap,              // keep mmap alive
    data: &'static [u8],      // ptr into mmap
    code_count: u32,
    entry_count: u32,
    code_idx_base: usize,
    block_tbl_base: usize,
    text_pool_base: usize,
}

impl TidexReader {
    pub fn open(path: &Path) -> Result<Self, TidexError> { /* mmap + validate header */ }

    /// Binary search for exact code match. Returns block (start_idx, count).
    fn find_code(&self, code: &str) -> Option<(u32, u32)> { unimplemented!() }

    /// Find the range of codes matching `prefix`. Returns iterator of (code, entries_iter).
    fn find_prefix_range(&self, prefix: &str) -> PrefixRange { unimplemented!() }

    /// Read an entry from Level-2: (text String, weight i32).
    fn read_entry(&self, idx: u32) -> (String, i32) { unimplemented!() }

    /// Exact query: read all entries for a code.
    pub fn query(&self, code: &str) -> Vec<(String, i32)> { unimplemented!() }

    /// Prefix query: scan all codes matching prefix, collect top `limit` by weight.
    pub fn query_prefix(&self, prefix: &str, limit: usize) -> Vec<(String, i32)> {
        unimplemented!()
    }
}
```

- [ ] **Step 4: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn roundtrip_exact_query() {
        // Write codes: "ni"→[("你",100), ("呢",90)], "ni hao"→[("你好",100)]
        // Read back: query("ni") → [("你",100), ("呢",90)]
        //             query("ni hao") → [("你好",100)]
    }

    #[test]
    fn prefix_query_n_matches_ni_and_ni_hao() {
        // Write codes "ni", "ni hao", "na"
        // query_prefix("ni", 10) → all entries under "ni" + "ni hao", no "na"
    }

    #[test]
    fn empty_code_returns_empty() { /* ... */ }
}
```

- [ ] **Step 5: Add to workspace**

```toml
# cheime-core/Cargo.toml
[workspace]
members = [..., "crates/cheime-tidx"]

[workspace.dependencies]
memmap2 = "0.9"
```

- [ ] **Step 6: Build and test**

```bash
cd cheime-core && cargo test -p cheime-tidx
```
Expected: 3+ tests PASS

- [ ] **Step 7: Commit**

```bash
git add crates/cheime-tidx/ Cargo.toml
git commit -m "feat: cheime-tidx — sorted binary disk index with mmap reader"
```

---

### Task 2: TieredIndex (hot Vec + cold Tidex)

**Files:**
- Create: `cheime-core/crates/cheime-dictionary/src/tiered.rs`
- Modify: `cheime-core/crates/cheime-dictionary/src/index.rs` (add variant)

**Interfaces:**
- Consumes: `cheime_tidx::TidexReader`
- Produces: `TieredIndex { hot: Vec<...>, cold: Arc<TidexReader> }`
- Produces: `CompiledIndex::Tiered(TieredIndex)` enum variant

- [ ] **Step 1: Write TieredIndex struct**

```rust
// tiered.rs
use std::sync::Arc;
use cheime_tidx::TidexReader;

/// In-memory hot entry: code → top-N entries as flat strings (no heap indirection).
#[derive(Clone, Debug)]
pub struct HotEntry {
    pub text: String,     // inline, no extra heap
    pub weight: i32,
}

/// Hot index: sorted codes, each with top-N entries in memory.
/// Sorted by code string for binary-search prefix lookup.
pub struct TieredIndex {
    /// Sorted array: (code, Vec<HotEntry>). Codes sorted ascending.
    hot: Vec<(String, Vec<HotEntry>)>,
    /// Cold disk index: mmap-backed, handles all remaining entries.
    cold: Arc<TidexReader>,
    hot_entries_per_code: usize,
}
```

- [ ] **Step 2: Implement exact query** (hot first, cold fallback)

```rust
impl TieredIndex {
    pub fn query(&self, code: &str) -> Vec<Candidate> {
        // 1. Binary search hot codes for exact code
        if let Ok(idx) = self.hot.binary_search_by(|(c, _)| c.as_str().cmp(code)) {
            let entries = &self.hot[idx].1;
            return self.hot_entries_to_candidates(entries, code);
        }
        // 2. Cold fallback
        let cold_entries = self.cold.query(code);
        self.cold_entries_to_candidates(&cold_entries)
    }

    pub fn query_prefix(&self, prefix: &str, limit: usize) -> Vec<Candidate> {
        // 1. Binary search hot for start of prefix range
        let start = self.hot.partition_point(|(c, _)| c.as_str() < prefix);
        let mut results = Vec::new();

        // 2. Collect hot entries within prefix range
        for (_, entries) in &self.hot[start..] {
            // collect up to limit
        }

        // 3. If not enough, query cold
        if results.len() < limit {
            let cold_entries = self.cold.query_prefix(prefix, limit - results.len());
            // merge
        }
        results.truncate(limit);
        results
    }
}
```

- [ ] **Step 3: Update CompiledIndex to enum**

```rust
// index.rs
pub enum CompiledIndex {
    Memory(MemoryIndex),
    Tiered(Arc<TieredIndex>),
}

impl CompiledIndex {
    pub fn query(&self, code: &str) -> Vec<Candidate> {
        match self {
            Self::Memory(m) => m.query(code),
            Self::Tiered(t) => t.query(code),
        }
    }
    pub fn query_prefix(&self, prefix: &str, limit: usize) -> Vec<Candidate> {
        match self {
            Self::Memory(m) => m.query_prefix(prefix, limit),
            Self::Tiered(t) => t.query_prefix(prefix, limit),
        }
    }
}
```

- [ ] **Step 4: Make existing MemoryIndex keep current behavior**

Move current `CompiledIndex` fields and methods into `MemoryIndex` (internal rename, no API break).

- [ ] **Step 5: Write tests**

```rust
#[test]
fn tiered_hot_hit_returns_exact_results() { /* hot contains "ni" → returns hot entries */ }
#[test]
fn tiered_cold_fallback_when_hot_empty() { /* hot has "ni" but not "za" → cold returns "za" entries */ }
#[test]
fn tiered_prefix_scans_hot_and_cold() { /* prefix "n" → scans hot + cold, merged by weight */ }
```

- [ ] **Step 6: Run all tests**

```bash
cd cheime-core && cargo test -p cheime-dictionary
```
Expected: ALL existing tests PASS, new tests PASS

- [ ] **Step 7: Commit**

```bash
git add crates/cheime-dictionary/src/
git commit -m "feat: TieredIndex — hot Vec + cold mmap, CompiledIndex enum"
```

---

### Task 3: Config wiring (SchemaConfig + DictCache build)

**Files:**
- Modify: `cheime-core/crates/cheime-config/src/schema.rs`
- Modify: `cheime-core/crates/cheime-dictionary/src/cache.rs`

**Interfaces:**
- Consumes: `TieredIndex`, `cheime_tidx::write_tidex`
- Produces: `DictCache::load_or_build` now also produces `*.tidx` file when `index_mode: tiered`

- [ ] **Step 1: Add config fields**

```rust
// schema.rs — EngineConfig or a new DictionaryConfig
#[serde(deny_unknown_fields)]
pub struct DictionaryConfig {
    #[serde(default)]
    pub index_mode: IndexMode,
    #[serde(default = "default_hot_entries")]
    pub hot_entries_per_code: usize,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexMode {
    #[default]
    Memory,
    Tiered,
}

fn default_hot_entries() -> usize { 5 }
```

- [ ] **Step 2: Add build_tiered to DictCache**

```rust
// cache.rs
impl DictCache {
    /// Build a tiered index: hot entries kept in memory,
    /// cold entries written to a `*.tidx` binary file.
    pub fn build_tiered(
        &self,
        files: &[PathBuf],
        dict_name: &str,
        columns: &[DictColumn],
        generation: DeploymentGeneration,
        hot_entries_per_code: usize,
    ) -> Result<TieredIndex, CacheError> {
        // 1. Load/parse all files (same as load_or_build)
        // 2. Merge entries by code, sort by weight desc
        // 3. Split each code's entries: top N → hot, rest → cold
        // 4. Write cold entries → <cache_dir>/<dict_name>/<hash>.tidx
        // 5. Build TieredIndex { hot, cold: Arc::new(TidexReader::open(tidx_path)) }
    }
}
```

- [ ] **Step 3: Wire into PipelineFactory**

Update `PipelineFactory::build` to check `DictionaryConfig::index_mode`, call `build_tiered` when tiered.

- [ ] **Step 4: Run full workspace tests**

```bash
cd cheime-core && cargo test --workspace
```
Expected: ALL tests PASS (existing memory mode unchanged)

- [ ] **Step 5: Commit**

```bash
git add crates/cheime-config/src/schema.rs crates/cheime-dictionary/src/cache.rs
git commit -m "feat: config-driven tiered index mode"
```

---

### Task 4: Benchmarking

**Files:**
- Create: `cheime-core/crates/cheime-dictionary/benches/tiered_bench.rs`

- [ ] **Step 1: Write memory benchmark**

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};
// Build tiered index from 2.45M entries
// Measure: resident set size (RSS) via /proc/self/status or platform equivalent
// Compare: MemoryIndex RSS vs TieredIndex RSS
```

- [ ] **Step 2: Write latency benchmark**

```rust
fn bench_tiered_query(c: &mut Criterion) {
    // query: hot hit vs cold miss vs prefix
    c.bench_function("tiered/query_hot", |b| { /* ... */ });
    c.bench_function("tiered/query_cold", |b| { /* ... */ });
    c.bench_function("tiered/query_prefix", |b| { /* ... */ });
}
```

- [ ] **Step 3: Run benchmarks**

```bash
cd cheime-core && cargo bench --bench tiered_bench
```

- [ ] **Step 4: Commit**

```bash
git add crates/cheime-dictionary/benches/tiered_bench.rs
git commit -m "bench: tiered index memory + latency"
```

---

### Task 5: Integration test (real dicts)

**Files:**
- Create: `cheime-core/crates/cheime-dictionary/tests/tiered_integration.rs`

- [ ] **Step 1: End-to-end test with real rime_ice dicts**

```rust
#[test]
fn tiered_mode_loads_all_245m_entries() {
    // Load all 8 dict files via DictCache::build_tiered()
    // Verify: total_entries == 2,453,057
    // Verify: query("ni") returns expected top candidates
    // Verify: query_prefix("n", 10) returns candidates from multiple codes
}
```

- [ ] **Step 2: Roundtrip: build tiered → query → same results as memory mode**

```rust
#[test]
fn tiered_and_memory_produce_same_candidates() {
    // Build both MemoryIndex and TieredIndex from same entries
    // For 100 random codes: assert_eq!(memory.query(c), tiered.query(c))
    // For 20 random prefixes: assert_eq!(memory.query_prefix(p, 10), tiered.query_prefix(p, 10))
}
```

- [ ] **Step 3: Run integration tests**

```bash
cd cheime-core && cargo test --test tiered_integration
```

- [ ] **Step 4: Commit**

```bash
git add crates/cheime-dictionary/tests/tiered_integration.rs
git commit -m "test: tiered index integration — full dicts roundtrip"
```

---

## Acceptance Criteria

| Criteria | Target | Verification |
|---|---|---|
| Memory (2.45M entries) | < 15MB RSS | `cargo bench --bench tiered_bench` + OS memory stats |
| Hot query latency | < 5µs | benchmark |
| Cold query latency | < 100µs | benchmark |
| Prefix query latency | < 1ms | benchmark |
| Build time | < 5s | integration test timing |
| Disk size | < 60MB | `ls -l *.tidx` |
| Exact query correctness | 100% match memory mode | `tiered_and_memory_produce_same_candidates` |
| Prefix query correctness | 100% match memory mode | `tiered_and_memory_produce_same_candidates` |
| Backward compat | `index_mode: memory` unchanged | existing tests pass |
| `forbid(unsafe_code)` in core | no unsafe in cheime-dictionary | `grep -r "unsafe" crates/cheime-dictionary/` empty |
