# 模糊拼音 + 简拼 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 实现全拼输入法的模糊拼音（z/zh, s/sh, n/l 等混淆）和自动简拼/首字全拼混输（nhao→nihao→你好, nh→你好）。

**Architecture:** 
- 模糊拼音：FuzzyNormalizer 已实现 13 条规则，只需接入管线并加配置
- 简拼：新增 AbbreviationNormalizer，检测纯简拼段（所有段都是单字母），展开首段为所有可能音节
- Normalizer trait 增加 `normalize_all` 方法（接收全部段），保持向后兼容

**Tech Stack:** Rust 2024, cheime-pipeline, cheime-config

## 全局约束

- `#![forbid(unsafe_code)]` 在 cheime-pipeline 和 cheime-config
- 使用 `parking_lot::Mutex`
- 所有新功能必须有单元测试和集成测试
- 不破坏现有 339 个测试

---

## 现状分析

### 管线架构
```
按键 → Processor → Segmentor → [Normalizer] → Translator → Filter → Ranker → 候选
```

### 关键发现
1. **FuzzyNormalizer** 已实现 13 条规则（zh↔z, ch↔c, sh↔s, n↔l, f↔h, ang↔an, eng↔en, ing↔in），但 **factory 硬编码 `normalizer: None`**
2. **简拼完全缺失**，DictTranslator 对单字母段做前缀查询（"n"→所有 n 开头的词），但多段简拼（如 "nh"）无法匹配
3. **CodeNormalizer trait** 只有 `normalize(&self, segment: &CodeSegment) -> Vec<CodeSegment>`，无法看到全部段

### 简拼问题详解

| 输入 | Segmentor 输出 | Translator 行为 | 结果 |
|------|---------------|----------------|------|
| `nhao` | `["n", "hao"]` | `query_prefix("n hao")` | ✅ 匹配 "ni hao"→你好 |
| `nh` | `["n", "h"]` | `query_prefix("n h")` | ❌ 不匹配 "ni hao" |
| `nhm` | `["n", "h", "m"]` | `query_prefix("n h m")` | ❌ 不匹配 "ni hao ma" |

**根因：** "n h" 不是 "ni hao" 的前缀（"n " ≠ "ni "）。

**解决方案：** 纯简拼时，展开首段单字母为所有可能音节：
- "nh" → 变体 ["ni h", "na h", "ne h", ...] → 各自前缀查询
- "ni h" 是 "ni hao" 的前缀 → ✅ 匹配

---

## 文件结构

```
cheime-core/crates/
├── cheime-config/src/schema.rs          — 新增 FuzzyConfig
├── cheime-pipeline/src/
│   ├── normalizer.rs                    — 修改 trait + 新增 AbbreviationNormalizer
│   ├── factory.rs                       — 接入 FuzzyNormalizer + AbbreviationNormalizer
│   └── lib.rs                           — 修改 ComposablePipeline 调用 normalize_all
└── cheime-pipeline/tests/stress_tests.rs — 新增模糊拼音和简拼集成测试
```

---

## Task 1: 扩展 CodeNormalizer trait

**Files:**
- Modify: `cheime-core/crates/cheime-pipeline/src/normalizer.rs`
- Modify: `cheime-core/crates/cheime-pipeline/src/lib.rs`

**Interfaces:**
- Produces: `CodeNormalizer::normalize_all(&self, segments: &[CodeSegment]) -> Vec<CodeSegment>`

**设计：** trait 增加 `normalize_all` 方法，默认实现调用 `normalize` 逐段处理。AbbreviationNormalizer 覆写此方法以实现跨段逻辑。

- [ ] **Step 1: 修改 CodeNormalizer trait**

在 `normalizer.rs` 中，给 trait 添加新方法：

```rust
pub trait CodeNormalizer: Send + Sync {
    fn name(&self) -> &str;
    fn normalize(&self, segment: &CodeSegment) -> Vec<CodeSegment>;
    
    /// Normalize all segments together (cross-segment logic like abbreviation).
    /// Default: per-segment normalize.
    fn normalize_all(&self, segments: &[CodeSegment]) -> Vec<CodeSegment> {
        segments.iter().flat_map(|s| self.normalize(s)).collect()
    }
}
```

- [ ] **Step 2: 修改 ComposablePipeline 调用 normalize_all**

在 `lib.rs:164-166`，将：
```rust
let variants: Vec<CodeSegment> = if let Some(n) = &self.normalizer {
    segments.iter().flat_map(|s| n.normalize(s)).collect()
} else { segments };
```
改为：
```rust
let variants: Vec<CodeSegment> = if let Some(n) = &self.normalizer {
    n.normalize_all(&segments)
} else { segments };
```

- [ ] **Step 3: 运行现有测试**

```bash
cd D:/coding/cheime/cheime-win/cheime-core && cargo test -p cheime-pipeline
```
Expected: 全部通过（默认实现保持行为不变）

- [ ] **Step 4: Commit**

```bash
git add crates/cheime-pipeline/src/normalizer.rs crates/cheime-pipeline/src/lib.rs
git commit -m "feat: add normalize_all to CodeNormalizer trait"
```

---

## Task 2: AbbreviationNormalizer 实现

**Files:**
- Modify: `cheime-core/crates/cheime-pipeline/src/normalizer.rs`

**Interfaces:**
- Consumes: `PINYIN_SYLLABLES` (from segmentor.rs, 412 syllables)
- Produces: `AbbreviationNormalizer` struct implementing `CodeNormalizer`

**设计：** 当所有段都是单字母时，展开首段为所有可能的拼音首字母音节。只展开首段（保持展开数 ≤40），后续段保持原样供前缀匹配。

- [ ] **Step 1: 导入 PINYIN_SYLLABLES**

在 `normalizer.rs` 顶部添加：
```rust
use crate::segmentor::PINYIN_SYLLABLES;
```

在 `segmentor.rs` 中，将 `PINYIN_SYLLABLES` 从 `fn new()` 内的局部常量改为 `pub(crate)` 模块级常量：

```rust
pub(crate) const PINYIN_SYLLABLES: &[&str] = &[
    "a", "ai", "an", "ang", "ao",
    "ba", "bai", "ban", "bang", "bao", "bei", "ben", "beng", "bi", "bian", "biao", "bie", "bin", "bing", "bo", "bu",
    // ... 全部 412 个音节
];
```

- [ ] **Step 2: 实现 AbbreviationNormalizer**

```rust
/// Expands the first single-letter segment to all possible pinyin syllables
/// starting with that letter. Only activates when ALL segments are single letters
/// (pure abbreviation input like "nh", "nhm").
pub struct AbbreviationNormalizer {
    /// Precomputed: letter → all syllables starting with that letter
    by_initial: HashMap<char, Vec<String>>,
}

impl AbbreviationNormalizer {
    pub fn new() -> Self {
        let mut by_initial: HashMap<char, Vec<String>> = HashMap::new();
        for &syl in PINYIN_SYLLABLES {
            if let Some(first) = syl.chars().next() {
                by_initial.entry(first).or_default().push(syl.to_string());
            }
        }
        Self { by_initial }
    }
}

impl CodeNormalizer for AbbreviationNormalizer {
    fn name(&self) -> &str { "abbreviation" }

    fn normalize(&self, segment: &CodeSegment) -> Vec<CodeSegment> {
        // Per-segment: not abbreviation context, passthrough
        vec![segment.clone()]
    }

    fn normalize_all(&self, segments: &[CodeSegment]) -> Vec<CodeSegment> {
        // Only activate for pure abbreviation: all segments are single letters
        if segments.len() < 2 || !segments.iter().all(|s| s.code.len() == 1) {
            return segments.iter().flat_map(|s| self.normalize(s)).collect();
        }

        // Expand first segment to all possible syllables
        let first_letter = segments[0].code.chars().next().unwrap();
        let expansions = match self.by_initial.get(&first_letter) {
            Some(v) => v,
            None => return segments.iter().flat_map(|s| self.normalize(s)).collect(),
        };

        let mut variants = Vec::new();
        for expanded in expansions {
            let mut variant = Vec::with_capacity(segments.len());
            variant.push(CodeSegment {
                code: expanded.clone(),
                tag: format!("{}-abbrev", segments[0].tag),
            });
            // Keep remaining segments as-is (prefix matching will handle)
            for s in &segments[1..] {
                variant.push(s.clone());
            }
            variants.extend(variant);
        }
        variants
    }
}
```

- [ ] **Step 3: 写单元测试**

```rust
#[test]
fn abbreviation_expands_first_letter_only() {
    let norm = AbbreviationNormalizer::new();
    let segments = vec![
        CodeSegment { code: "n".into(), tag: "pinyin".into() },
        CodeSegment { code: "h".into(), tag: "pinyin".into() },
    ];
    let variants = norm.normalize_all(&segments);
    // Should expand "n" to "ni", "na", "ne", "nai", "nan", etc.
    // Each variant is [expanded_syllable, "h"]
    assert!(variants.len() > 10);
    // Check first variant structure
    assert!(variants[0].code.len() > 1); // expanded syllable
    // Check that remaining segments are preserved
    // (variants come in pairs: [expanded, "h"])
}

#[test]
fn abbreviation_does_not_activate_for_mixed_input() {
    let norm = AbbreviationNormalizer::new();
    let segments = vec![
        CodeSegment { code: "n".into(), tag: "pinyin".into() },
        CodeSegment { code: "hao".into(), tag: "pinyin".into() },
    ];
    let variants = norm.normalize_all(&segments);
    // Not all single letters → passthrough
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0].code, "n");
    assert_eq!(variants[1].code, "hao");
}

#[test]
fn abbreviation_single_segment_passthrough() {
    let norm = AbbreviationNormalizer::new();
    let segments = vec![
        CodeSegment { code: "n".into(), tag: "pinyin".into() },
    ];
    let variants = norm.normalize_all(&segments);
    assert_eq!(variants.len(), 1);
}
```

- [ ] **Step 4: 运行测试**

```bash
cd D:/coding/cheime/cheime-win/cheime-core && cargo test -p cheime-pipeline
```

- [ ] **Step 5: Commit**

```bash
git add crates/cheime-pipeline/src/normalizer.rs crates/cheime-pipeline/src/segmentor.rs
git commit -m "feat: AbbreviationNormalizer — pure abbreviation expansion"
```

---

## Task 3: 配置接入 — FuzzyConfig + 组合 Normalizer

**Files:**
- Modify: `cheime-core/crates/cheime-config/src/schema.rs`
- Modify: `cheime-core/crates/cheime-pipeline/src/factory.rs`

**Interfaces:**
- Consumes: `EngineConfig.fuzzy_pinyin` config
- Produces: `PipelineFactory::build()` 创建组合 Normalizer（Fuzzy + Abbreviation）

- [ ] **Step 1: 添加 FuzzyConfig 到 schema.rs**

```rust
// 在 EngineConfig 中添加：
#[serde(default, skip_serializing_if = "Option::is_none")]
pub fuzzy_pinyin: Option<FuzzyPinyinConfig>,

// 新结构体：
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FuzzyPinyinConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Specific rules to enable. Empty = all standard rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<String>,
}
```

- [ ] **Step 2: 修改 factory.rs 接入 Normalizer**

在 `PipelineFactory::build()` 中，将 `None` 替换为实际的 normalizer：

```rust
pub fn build(...) -> Result<ComposablePipeline, BuildError> {
    let normalizer = Self::build_normalizer(&config.engine);
    let mut p = ComposablePipeline::new(
        Self::build_processor(config)?,
        Self::build_segmentor(&config.engine)?,
        normalizer,  // was: None
        Self::build_translators(...)?,
        Self::build_filters(...)?,
        Self::build_ranker(),
    );
    // ...
}

fn build_normalizer(e: &EngineConfig) -> Option<Box<dyn crate::normalizer::CodeNormalizer>> {
    use crate::normalizer::{FuzzyNormalizer, AbbreviationNormalizer, CompositeNormalizer};
    
    let mut normalizers: Vec<Box<dyn crate::normalizer::CodeNormalizer>> = Vec::new();
    
    // Abbreviation normalizer (always on for pinyin segmentor)
    if e.segmentors.iter().any(|s| matches!(s, SegmentorConfig::PinyinSyllable)) {
        normalizers.push(Box::new(AbbreviationNormalizer::new()));
    }
    
    // Fuzzy normalizer (configurable)
    if let Some(ref fuzzy) = e.fuzzy_pinyin {
        if fuzzy.enabled {
            if fuzzy.rules.is_empty() {
                normalizers.push(Box::new(FuzzyNormalizer::standard()));
            } else {
                normalizers.push(Box::new(FuzzyNormalizer::from_rules(&fuzzy.rules)));
            }
        }
    }
    
    match normalizers.len() {
        0 => None,
        1 => Some(normalizers.into_iter().next().unwrap()),
        _ => Some(Box::new(CompositeNormalizer::new(normalizers))),
    }
}
```

- [ ] **Step 3: 实现 CompositeNormalizer**

在 `normalizer.rs` 中添加组合 normalizer：

```rust
/// Chains multiple normalizers: first expands abbreviations, then fuzzy variants.
pub struct CompositeNormalizer {
    normalizers: Vec<Box<dyn CodeNormalizer>>,
}

impl CompositeNormalizer {
    pub fn new(normalizers: Vec<Box<dyn CodeNormalizer>>) -> Self {
        Self { normalizers }
    }
}

impl CodeNormalizer for CompositeNormalizer {
    fn name(&self) -> &str { "composite" }

    fn normalize(&self, segment: &CodeSegment) -> Vec<CodeSegment> {
        let mut current = vec![segment.clone()];
        for norm in &self.normalizers {
            current = current.iter().flat_map(|s| norm.normalize(s)).collect();
        }
        current
    }

    fn normalize_all(&self, segments: &[CodeSegment]) -> Vec<CodeSegment> {
        let mut current: Vec<CodeSegment> = segments.to_vec();
        for norm in &self.normalizers {
            current = norm.normalize_all(&current);
        }
        current
    }
}
```

- [ ] **Step 4: 实现 FuzzyNormalizer::from_rules**

```rust
impl FuzzyNormalizer {
    pub fn from_rules(rule_names: &[String]) -> Self {
        let all = Self::standard();
        let rules: Vec<FuzzyRule> = all.rules.into_iter()
            .filter(|r| rule_names.iter().any(|name| {
                // Match "zh_z" or "zh↔z" format
                name == &format!("{}_{}", r.from, r.to) || 
                name == &format!("{}↔{}", r.from, r.to)
            }))
            .collect();
        Self { rules }
    }
}
```

- [ ] **Step 5: 写配置解析测试**

```rust
#[test]
fn parse_fuzzy_pinyin_config() {
    let yaml = r#"
engine:
  segmentors:
    - type: pinyin_syllable
  fuzzy_pinyin:
    enabled: true
    rules: ["zh_z", "n_l"]
"#;
    let config: SchemaConfig = serde_yaml::from_str(yaml).unwrap();
    let fuzzy = config.engine.fuzzy_pinyin.unwrap();
    assert!(fuzzy.enabled);
    assert_eq!(fuzzy.rules, vec!["zh_z", "n_l"]);
}

#[test]
fn parse_fuzzy_pinyin_all_rules() {
    let yaml = r#"
engine:
  segmentors:
    - type: pinyin_syllable
  fuzzy_pinyin:
    enabled: true
"#;
    let config: SchemaConfig = serde_yaml::from_str(yaml).unwrap();
    let fuzzy = config.engine.fuzzy_pinyin.unwrap();
    assert!(fuzzy.enabled);
    assert!(fuzzy.rules.is_empty()); // empty = all
}
```

- [ ] **Step 6: 运行全量测试**

```bash
cd D:/coding/cheime/cheime-win/cheime-core && cargo test --workspace
```

- [ ] **Step 7: Commit**

```bash
git add crates/cheime-config/src/schema.rs crates/cheime-pipeline/src/factory.rs crates/cheime-pipeline/src/normalizer.rs
git commit -m "feat: wire FuzzyNormalizer + AbbreviationNormalizer into pipeline"
```

---

## Task 4: 集成测试 — 端到端模糊拼音 + 简拼

**Files:**
- Modify: `cheime-core/crates/cheime-pipeline/tests/stress_tests.rs`

- [ ] **Step 1: 模糊拼音集成测试**

```rust
#[test]
fn fuzzy_zh_matches_z_in_pipeline() {
    // Type "zongguo" (fuzzy for "zhongguo") → should match 中国
    let dict = real_dict();
    let config = SchemaConfig {
        engine: EngineConfig {
            segmentors: vec![SegmentorConfig::PinyinSyllable],
            fuzzy_pinyin: Some(FuzzyPinyinConfig { enabled: true, rules: vec![] }),
            ..Default::default()
        },
        ..Default::default()
    };
    let p = PipelineFactory::build(&config, None, Some(dict.clone()), None).unwrap();
    
    let mut candidates = Vec::new();
    for ch in "zongguo".chars() {
        let events = p.handle_key(KeyEvent { key: Key::Char(ch), state: KeyState::empty() });
        for ev in &events {
            if let OutputEvent::Candidates(c) = ev {
                candidates = c.clone();
            }
        }
    }
    assert!(candidates.iter().any(|c| c.text == "中国"), 
        "fuzzy z/zh should produce 中国 for 'zongguo'");
}
```

- [ ] **Step 2: 简拼集成测试**

```rust
#[test]
fn abbreviation_nh_produces_nihao_candidates() {
    // Type "nh" (abbreviation for "nihao") → should match 你好
    let dict = real_dict();
    let config = SchemaConfig {
        engine: EngineConfig {
            segmentors: vec![SegmentorConfig::PinyinSyllable],
            ..Default::default()
        },
        ..Default::default()
    };
    let p = PipelineFactory::build(&config, None, Some(dict.clone()), None).unwrap();
    
    let mut candidates = Vec::new();
    for ch in "nh".chars() {
        let events = p.handle_key(KeyEvent { key: Key::Char(ch), state: KeyState::empty() });
        for ev in &events {
            if let OutputEvent::Candidates(c) = ev {
                candidates = c.clone();
            }
        }
    }
    assert!(candidates.iter().any(|c| c.text == "你好"),
        "abbreviation 'nh' should produce 你好");
}

#[test]
fn abbreviation_nhm_produces_nihao_candidates() {
    // "nhm" → abbreviation → should match 你好吗 (ni hao ma)
    let dict = real_dict();
    let config = SchemaConfig {
        engine: EngineConfig {
            segmentors: vec![SegmentorConfig::PinyinSyllable],
            ..Default::default()
        },
        ..Default::default()
    };
    let p = PipelineFactory::build(&config, None, Some(dict.clone()), None).unwrap();
    
    let mut candidates = Vec::new();
    for ch in "nhm".chars() {
        let events = p.handle_key(KeyEvent { key: Key::Char(ch), state: KeyState::empty() });
        for ev in &events {
            if let OutputEvent::Candidates(c) = ev {
                candidates = c.clone();
            }
        }
    }
    assert!(candidates.iter().any(|c| c.text.contains("好")),
        "abbreviation 'nhm' should produce candidates containing 好");
}
```

- [ ] **Step 3: 模糊拼音 + 简拼组合测试**

```rust
#[test]
fn fuzzy_plus_abbreviation_zg_matches_zhongguo() {
    // "zg" = fuzzy(z→zh) + abbreviation → "zhong guo" → 中国
    let dict = real_dict();
    let config = SchemaConfig {
        engine: EngineConfig {
            segmentors: vec![SegmentorConfig::PinyinSyllable],
            fuzzy_pinyin: Some(FuzzyPinyinConfig { enabled: true, rules: vec![] }),
            ..Default::default()
        },
        ..Default::default()
    };
    let p = PipelineFactory::build(&config, None, Some(dict.clone()), None).unwrap();
    
    let mut candidates = Vec::new();
    for ch in "zg".chars() {
        let events = p.handle_key(KeyEvent { key: Key::Char(ch), state: KeyState::empty() });
        for ev in &events {
            if let OutputEvent::Candidates(c) = ev {
                candidates = c.clone();
            }
        }
    }
    assert!(candidates.iter().any(|c| c.text == "中国"),
        "fuzzy+abbreviation 'zg' should produce 中国");
}
```

- [ ] **Step 4: 运行集成测试**

```bash
cd D:/coding/cheime/cheime-win/cheime-core && cargo test -p cheime-pipeline --test stress_tests
```

- [ ] **Step 5: Commit**

```bash
git add crates/cheime-pipeline/tests/stress_tests.rs
git commit -m "test: fuzzy pinyin + abbreviation integration tests"
```

---

## 验收标准

| 标准 | 验证方式 |
|------|---------|
| 模糊拼音 "zongguo" → 中国 | 集成测试 `fuzzy_zh_matches_z_in_pipeline` |
| 简拼 "nh" → 你好 | 集成测试 `abbreviation_nh_produces_nihao_candidates` |
| 简拼 "nhm" → 你好吗 | 集成测试 `abbreviation_nhm_produces_nihao_candidates` |
| 模糊+简拼 "zg" → 中国 | 集成测试 `fuzzy_plus_abbreviation_zg_matches_zhongguo` |
| 配置 `fuzzy_pinyin.enabled: false` 关闭模糊 | 单元测试 |
| 现有 339 测试不破坏 | `cargo test --workspace` |
