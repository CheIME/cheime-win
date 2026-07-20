# CheIME TIP 安全 COM 重建与隔离验证实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将会导致宿主进程崩溃的 CheIME TSF TIP 重建为内存安全、COM identity 合规的多接口对象，并在纯内存测试与隔离子进程探针通过后，仅允许在 Windows Sandbox/可回滚 VM 中执行注册测试。

**Architecture:** 一个 `Box<ComTip>` 同时拥有全部嵌入式接口 header、唯一引用计数和 apartment-bound runtime state。每个接口回调通过固定 `offset_of!` 从该接口 header 恢复 owner；所有 `QueryInterface`、`AddRef`、`Release` 最终转发到同一个 owner，并以 primary header 作为 canonical `IUnknown`。所有 ABI 直接采用 `windows 0.58` 生成的 vtable 类型，不再手写 TSF vtable 布局。

**Tech Stack:** Rust 2024、Rust 1.85、`windows = 0.58`、Windows TSF/COM、PowerShell、独立 probe 进程、Windows Sandbox/可回滚 VM。

## Global Constraints

- 禁止在宿主 Windows 上运行 `regsvr32`、`DllRegisterServer`、installer install、`cheime-registered-probe` 或 `cheime-profile-probe`。
- 禁止在宿主 Windows 上重新添加、选择或激活 CheIME profile。
- 所有 TSF vtable 必须使用 `windows 0.58` 生成类型；禁止重新声明手写 TSF vtable struct。
- 一个 TIP 实例只能有一个 owner allocation、一个引用计数和一个 canonical `IUnknown` identity。
- `Box::from_raw` 只能接收最初由 `Box::into_raw(ComTip::new())` 返回的 owner 起始地址。
- secondary interface 的 `this` 必须经对应的 `offset_of!(ComTip, field)` 恢复 owner，禁止直接 cast 为 `*mut ComTip`。
- 所有成功的 `QueryInterface` 恰好执行一次 `AddRef`；失败必须清空输出参数。
- `CreateInstance` 和 `DllGetClassObject` 均采用“创建者引用 → QI → 释放创建者引用”，成功时只向调用者交接一个引用。
- COM FFI 回调不得让 Rust panic 穿过 ABI 边界；错误线程、重入借用和无效参数必须返回 HRESULT。
- `ITfThreadMgr` 等 apartment-bound COM 接口必须保存 owned typed wrapper，不得持久保存调用方提供的裸指针。
- Level 1 和 Level 2 全部通过之前不得进入任何注册测试；Level 3 只能在 Windows Sandbox 或有快照的 VM 中执行。

---

## File Structure

- `crates/cheime-tip/src/tsf_interfaces.rs` — 单一 `ComTip` allocation、接口 header、owner recovery、统一 IUnknown、TSF 回调和生成 vtable。当前文件已有安全重写雏形；本计划先锁定其不变量，再按职责拆分。
- `crates/cheime-tip/src/runtime.rs` — STA owner thread、activation/focus generation、owned TSF resources、rollback/deactivation state machine。
- `crates/cheime-tip/src/class_factory.rs` — 使用生成的 `IClassFactory_Vtbl`，负责 aggregation 检查和 TIP 创建引用交接。
- `crates/cheime-tip/src/dll_exports.rs` — DLL exports、factory 引用交接、注册数据生成；测试不得调用注册写入。
- `crates/cheime-tip/src/exports.rs` — live object count 与 server lock count，二者独立。
- `crates/cheime-tip/src/lib.rs` — 模块边界；完成拆分后只导出 DLL 所需模块。
- `crates/cheime-probe/main.rs` — 无注册、独立进程 `LoadLibraryW` probe；允许在宿主运行。
- `crates/cheime-probe/registered.rs` — 系统 COM 注册 probe；仅允许 guest VM/Sandbox。
- `crates/cheime-probe/profile.rs` — process-scoped profile activation probe；仅允许 guest VM/Sandbox。
- `crates/cheime-probe/fake_thread_mgr.rs` — 后续创建，用于在独立进程中测试非空 `ActivateEx`，不依赖系统 TSF 注册。
- `scripts/verify-tip-safe.ps1` — 后续创建，只执行 Level 1 + Level 2，不注册。
- `scripts/verify-tip-guest.ps1` — 后续创建，顶部强制 guest opt-in，仅执行 Level 3。
- `DEPLOY.md` — 增加宿主禁用警告和 guest-only 恢复门禁。

---

### Task 1: 锁定单一 allocation 与 owner recovery 不变量

**Files:**
- Modify: `crates/cheime-tip/src/tsf_interfaces.rs:38-228`
- Test: `crates/cheime-tip/src/tsf_interfaces.rs` module tests

**Interfaces:**
- Produces: `ComTip::new() -> Box<ComTip>`
- Produces: `owner_from_primary/key/thread_mgr/composition/display(*mut c_void) -> *mut ComTip`
- Produces: `query_owner`, `add_ref_owner`, `release_owner`

- [ ] **Step 1: 写/补齐 owner recovery 失败测试**

测试必须构造一个真实 `Box<ComTip>`，分别取得五个 header 地址并断言恢复到同一 owner：

```rust
#[test]
fn every_interface_header_recovers_the_same_owner() {
    let _guard = test_counter_guard();
    reset_counts();
    let owner = Box::into_raw(ComTip::new());
    let expected = owner;

    let primary = unsafe { std::ptr::addr_of_mut!((*owner).primary).cast() };
    let key = unsafe { std::ptr::addr_of_mut!((*owner).key).cast() };
    let thread_mgr = unsafe { std::ptr::addr_of_mut!((*owner).thread_mgr).cast() };
    let composition = unsafe { std::ptr::addr_of_mut!((*owner).composition).cast() };
    let display = unsafe { std::ptr::addr_of_mut!((*owner).display).cast() };

    assert_eq!(unsafe { owner_from_primary(primary) }, expected);
    assert_eq!(unsafe { owner_from_key(key) }, expected);
    assert_eq!(unsafe { owner_from_thread_mgr(thread_mgr) }, expected);
    assert_eq!(unsafe { owner_from_composition(composition) }, expected);
    assert_eq!(unsafe { owner_from_display(display) }, expected);
    assert_eq!(unsafe { release_owner(owner) }, 0);
    assert_eq!(live_object_count(), 0);
}
```

同时加入：

```rust
#[test]
fn primary_header_is_at_owner_offset_zero() {
    assert_eq!(std::mem::offset_of!(ComTip, primary), 0);
}
```

- [ ] **Step 2: 运行测试确认旧错误模型会失败或当前雏形已覆盖回归**

Run:

```powershell
cargo test -p cheime-tip every_interface_header_recovers_the_same_owner -- --exact
cargo test -p cheime-tip primary_header_is_at_owner_offset_zero -- --exact
```

Expected: 在旧 `InterfaceWrapper`/多解引用实现上 FAIL；若当前未提交安全雏形已使其 PASS，记录为 characterization test，并通过 `git diff` 证明错误实现已被替换，不把“立即 PASS”伪称为本轮 TDD RED。

- [ ] **Step 3: 保留最小安全 owner recovery 实现**

实现必须保持：

```rust
unsafe fn owner_at_offset(this: *mut c_void, offset: usize) -> *mut ComTip {
    unsafe { this.cast::<u8>().sub(offset).cast::<ComTip>() }
}

pub unsafe fn owner_from_key(this: *mut c_void) -> *mut ComTip {
    unsafe { owner_at_offset(this, std::mem::offset_of!(ComTip, key)) }
}
```

其他 header 使用相同模式；不得读取 `*(this as *mut *mut ComTip)`。

- [ ] **Step 4: 加入最后释放者测试**

从每个 secondary interface 获取引用，释放 primary 创建者引用，最后由 secondary `Release` 完成唯一一次 `Box::from_raw(owner)`；使用 drop counter 和 `live_object_count()` 断言只析构一次。

- [ ] **Step 5: 运行包测试**

```powershell
cargo test -p cheime-tip
```

Expected: 0 failed；不得加载 release DLL 或修改注册表。

- [ ] **Step 6: Commit**

```powershell
git add crates/cheime-tip/src/tsf_interfaces.rs
git commit -m "fix: use one allocation for TIP COM interfaces"
```

---

### Task 2: 锁定完整 QueryInterface matrix 和 canonical IUnknown

**Files:**
- Modify: `crates/cheime-tip/src/tsf_interfaces.rs:95-228`
- Test: `crates/cheime-tip/src/tsf_interfaces.rs` module tests

**Interfaces:**
- Consumes: Task 1 owner recovery helpers
- Produces: 从任意已知接口到任意已知接口的稳定 QI；所有 `QI(IUnknown)` 返回 primary header

- [ ] **Step 1: 写完整 7×7 QI matrix 测试**

已知 IID 集合：

```rust
let iids = [
    IUnknown::IID,
    IID_TIP,
    IID_TIP_EX,
    IID_KEY,
    IID_TM,
    IID_COMP,
    IID_DA,
];
```

对每个 source interface 调用其 vtable `QueryInterface` 到每个 target IID，要求 `S_OK`、非空；对每个结果再 QI `IUnknown`，所有指针必须等于 primary 地址。每个新增引用都必须对应 Release。

- [ ] **Step 2: 写失败输出测试**

```rust
#[test]
fn unknown_iid_clears_output_without_changing_refcount() {
    let owner = Box::into_raw(ComTip::new());
    let before = unsafe { (*owner).ref_count.load(Ordering::Relaxed) };
    let unknown = GUID::from_u128(0xDEADBEEF_0000_0000_0000_000000000000);
    let mut out = std::ptr::dangling_mut();
    assert_eq!(unsafe { query_owner(owner, &unknown, &mut out) }, E_NOINTERFACE);
    assert!(out.is_null());
    assert_eq!(unsafe { (*owner).ref_count.load(Ordering::Relaxed) }, before);
    assert_eq!(unsafe { release_owner(owner) }, 0);
}
```

并覆盖 `out == null`、`iid == null`、`owner == null` 返回 `E_POINTER`。

- [ ] **Step 3: 运行失败测试**

```powershell
cargo test -p cheime-tip query_interface_matrix -- --exact
cargo test -p cheime-tip unknown_iid_clears_output_without_changing_refcount -- --exact
```

Expected: 缺少对称 QI、identity 或输出清理时 FAIL。

- [ ] **Step 4: 实现统一 QI**

所有 interface thunk 必须只恢复 owner 后调用：

```rust
unsafe fn query_owner(
    owner: *mut ComTip,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if out.is_null() { return E_POINTER; }
    unsafe { *out = null_mut() };
    if owner.is_null() || iid.is_null() { return E_POINTER; }

    match unsafe { ComTip::interface(owner, &*iid) } {
        Some(interface) => {
            unsafe { add_ref_owner(owner) };
            unsafe { *out = interface };
            S_OK
        }
        None => E_NOINTERFACE,
    }
}
```

`IUnknown`、`IID_TIP`、`IID_TIP_EX` 必须映射到 `owner.primary`。

- [ ] **Step 5: 使用 `windows::core::Interface` 验证 cast/clone/drop**

从 raw primary 构造 `ITfTextInputProcessorEx`，依次 `.cast::<ITfKeyEventSink>()`、`.cast::<IUnknown>()`、`.clone()`、drop，最终断言 live object count 回到初值。

- [ ] **Step 6: Verify and commit**

```powershell
cargo test -p cheime-tip

git add crates/cheime-tip/src/tsf_interfaces.rs
git commit -m "fix: enforce COM identity across TIP interfaces"
```

---

### Task 3: 使用 windows 0.58 生成 vtable 锁定 ABI

**Files:**
- Modify: `crates/cheime-tip/src/tsf_interfaces.rs:230-703`
- Test: `crates/cheime-tip/src/tsf_interfaces.rs` ABI tests

**Interfaces:**
- Produces: `ITfTextInputProcessorEx_Vtbl`, `ITfKeyEventSink_Vtbl`, `ITfThreadMgrEventSink_Vtbl`, `ITfCompositionSink_Vtbl`, `ITfDisplayAttributeProvider_Vtbl`

- [ ] **Step 1: 添加编译型 ABI 测试**

```rust
#[test]
fn display_provider_uses_generated_three_argument_abi() {
    let get: unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT = get_display;

    let _vtbl = ITfDisplayAttributeProvider_Vtbl {
        base__: unknown_vtbl(display_qi, display_add_ref, display_release),
        EnumDisplayAttributeInfo: enum_display,
        GetDisplayAttributeInfo: get,
    };
}
```

另为 ThreadMgr sink 五个回调、Key sink `*mut BOOL`、TIPEx base nesting 添加同类编译断言。

- [ ] **Step 2: 运行 ABI 测试**

```powershell
cargo test -p cheime-tip display_provider_uses_generated_three_argument_abi -- --exact
```

Expected: 若仍是四参数函数，编译失败；修正后 PASS。

- [ ] **Step 3: 删除所有自定义 TSF vtable struct 和重复 IID 常量来源**

IID 只从生成接口取得：

```rust
pub const IID_TIP: GUID = ITfTextInputProcessor::IID;
pub const IID_TIP_EX: GUID = ITfTextInputProcessorEx::IID;
```

vtable 直接初始化生成类型，禁止 `TIPVtbl`、`KeySinkVtbl`、`DisplayAttrVtbl`。

- [ ] **Step 4: FFI panic 边界审计**

所有回调不得使用会 panic 的 `borrow_mut()`、`unwrap()`、数组越界或非法枚举构造。runtime access 使用 `try_borrow_mut()`；失败返回 `E_UNEXPECTED`。

- [ ] **Step 5: x64 与 x86 编译门禁**

```powershell
cargo test -p cheime-tip
rustup target add i686-pc-windows-msvc
cargo test -p cheime-tip --target i686-pc-windows-msvc --no-run
```

Expected: 两者编译成功。若缺少本机 x86 linker/toolchain，记录为 BLOCKED，不得跳过后进入 Level 3。

- [ ] **Step 6: Commit**

```powershell
git add crates/cheime-tip/src/tsf_interfaces.rs
git commit -m "fix: bind TIP vtables to generated Windows ABI"
```

---

### Task 4: 修正 ClassFactory、DLL 引用交接与 unload 计数

**Files:**
- Modify: `crates/cheime-tip/src/class_factory.rs`
- Modify: `crates/cheime-tip/src/dll_exports.rs:140-170`
- Modify: `crates/cheime-tip/src/exports.rs`
- Test: same module tests

**Interfaces:**
- Produces: `ClassFactory::new() -> Box<ClassFactory>`
- Produces: `create_instance(iid, out) -> HRESULT`
- Produces: `DllCanUnloadNow() -> HRESULT`

- [ ] **Step 1: 写 factory failure/aggregation 测试**

覆盖：

- `ppv == null` → `E_POINTER`
- `riid == null` → `E_POINTER` 且清空 out
- `outer != null` → `CLASS_E_NOAGGREGATION` 且清空 out
- unknown IID → `E_NOINTERFACE`、无 live object 泄漏

- [ ] **Step 2: 写 creator-reference handoff 测试**

`DllGetClassObject` 成功后 factory refcount 对调用者表现为一个引用；drop factory 后 live object count 回零。`CreateInstance` 同理。

- [ ] **Step 3: 运行测试确认失败点**

```powershell
cargo test -p cheime-tip class_factory -- --nocapture
cargo test -p cheime-tip dll_exports -- --nocapture
```

- [ ] **Step 4: 实现固定交接模板**

```rust
let factory = Box::into_raw(ClassFactory::new()); // ref 1
let hr = unsafe { ClassFactory::query_interface(factory, riid, ppv) }; // ref 2 on success
unsafe { ClassFactory::release(factory) }; // success ref 1, failure ref 0
hr
```

TIP 创建使用同一模式。禁止 QI 成功后保留 creator ref。

- [ ] **Step 5: 分离 live objects 与 server locks**

`DllCanUnloadNow` 仅在：

```rust
live_object_count() == 0 && server_lock_count() == 0
```

时返回 `S_OK`。`LockServer(FALSE)` 在零计数时不得 underflow。

- [ ] **Step 6: Verify and commit**

```powershell
cargo test -p cheime-tip
cargo clippy -p cheime-tip --all-targets -- -D warnings

git add crates/cheime-tip/src/class_factory.rs crates/cheime-tip/src/dll_exports.rs crates/cheime-tip/src/exports.rs
git commit -m "fix: balance TIP COM factory lifetimes"
```

---

### Task 5: 完成 apartment-bound activation 生命周期

**Files:**
- Modify: `crates/cheime-tip/src/runtime.rs`
- Modify: `crates/cheime-tip/src/tsf_interfaces.rs:246-439`
- Test: runtime and interface module tests

**Interfaces:**
- Consumes: safe `ComTip` and embedded sink headers
- Produces: owned `ActivationResources`
- Produces: `begin_activation`, `complete_activation`, `abort_activation`, `begin_deactivation`

- [ ] **Step 1: 写 wrong-thread、reentrant 和 stale-token 测试**

要求错误线程和 held `RefCell` borrow 均返回拒绝，不 panic；stale activation/focus token 必须把 owned resource 原样返回，不能在 borrow 中 drop。

- [ ] **Step 2: 写 rollback 顺序测试**

事件必须是：

```text
unadvise thread sink
unadvise key sink
release focus/resources
release source/keystroke/thread manager
```

用测试资源的 `Drop` 日志精确断言顺序。

- [ ] **Step 3: 实现 borrowed → owned COM 转换**

激活参数不能用 `from_raw` 接管；必须从 borrowed pointer clone 出 owned `ITfThreadMgr`，然后通过 `.cast()` 得到 `ITfKeystrokeMgr` 和 `ITfSource`。

- [ ] **Step 4: 外部 COM 调用全部移出 `RefCell` borrow**

流程：state 生成 token → 释放 borrow → QI/advise/GetFocus → 重新 borrow 验证 token → commit。任一步失败逆序 unadvise 并释放 owned wrapper。

- [ ] **Step 5: Deactivate 先失效 state 再调用外部 COM**

state 内关闭 key admission、递增 generation、`take()` 全部 owned resources；退出 borrow 后 unadvise，最后 drop wrappers。重复 Deactivate 返回 `S_OK`。

- [ ] **Step 6: Verify and commit**

```powershell
cargo test -p cheime-tip runtime -- --nocapture
cargo test -p cheime-tip

git add crates/cheime-tip/src/runtime.rs crates/cheime-tip/src/tsf_interfaces.rs
git commit -m "fix: make TIP activation rollback-safe"
```

---

### Task 6: 拆分大型接口文件并完成 Level 1 门禁

**Files:**
- Create: `crates/cheime-tip/src/com_object.rs`
- Create: `crates/cheime-tip/src/interfaces/mod.rs`
- Create: `crates/cheime-tip/src/interfaces/text_input_processor.rs`
- Create: `crates/cheime-tip/src/interfaces/key_event_sink.rs`
- Create: `crates/cheime-tip/src/interfaces/thread_mgr_event_sink.rs`
- Create: `crates/cheime-tip/src/interfaces/composition_sink.rs`
- Create: `crates/cheime-tip/src/interfaces/display_attribute_provider.rs`
- Modify: `crates/cheime-tip/src/lib.rs`
- Remove after migration: `crates/cheime-tip/src/tsf_interfaces.rs`

**Interfaces:**
- Preserves all public crate-internal signatures from Tasks 1–5

- [ ] **Step 1: 先运行完整基线测试**

```powershell
cargo test -p cheime-tip
```

记录准确测试数量。

- [ ] **Step 2: 仅按职责移动代码，不改变行为**

`com_object.rs` 只含 allocation/headers/owner/IUnknown/create；每个 interface 文件只含其 generated vtable 和回调；`interfaces/mod.rs` 统一 re-export IID 与 HRESULT。

- [ ] **Step 3: 每次移动一个模块后运行测试**

```powershell
cargo test -p cheime-tip
```

任何失败先恢复等价行为，不在拆分任务中增加新功能。

- [ ] **Step 4: 静态回归搜索**

以下搜索必须无命中，或命中只存在于明确的 owner helper：

```powershell
rg "InterfaceWrapper|cast::<\*const CheimeTip>|cast::<\*mut CheimeTip>|struct TIPVtbl|struct KeySinkVtbl|struct DisplayAttrVtbl" crates/cheime-tip/src
rg "Box::from_raw\(this" crates/cheime-tip/src
```

- [ ] **Step 5: Level 1 全门禁**

```powershell
cargo fmt --all -- --check
cargo clippy -p cheime-tip --all-targets -- -D warnings
cargo test -p cheime-tip
cargo test -p cheime-tip --target i686-pc-windows-msvc --no-run
cargo build -p cheime-tip --release
```

Expected: 所有命令 exit 0；不执行 DLL 加载或注册。

- [ ] **Step 6: Commit**

```powershell
git add crates/cheime-tip/src
git commit -m "refactor: isolate TIP COM interface boundaries"
```

---

### Task 7: 加固无注册隔离子进程 probe（Level 2）

**Files:**
- Modify: `crates/cheime-probe/main.rs`
- Modify: `crates/cheime-probe/Cargo.toml`
- Create: `scripts/verify-tip-safe.ps1`

**Interfaces:**
- Consumes: release `cheime_tip.dll`
- Produces: 独立进程 exit code；输出精确 DLL 路径和 SHA-256

- [ ] **Step 1: 写 probe 自测试/父进程测试**

添加 `--self-test-failure`，让子进程在指定阶段返回非零；测试父脚本必须能识别失败且不继续。

- [ ] **Step 2: 将 probe 分为显式阶段**

顺序固定为：fingerprint → LoadLibrary → exports → factory → aggregation rejection → CreateInstance TIPEx → 7×7 QI → canonical IUnknown → AddRef/Release → inert callbacks → null Activate/ActivateEx 返回 E_POINTER → release TIP/factory → DllCanUnloadNow S_OK → FreeLibrary。

- [ ] **Step 3: 禁止 probe 调用注册 exports**

不得 `GetProcAddress` 或调用 `DllRegisterServer`/`DllUnregisterServer`。probe 只加载用户明确指定或 release 目录中的 DLL。

- [ ] **Step 4: 编写安全验证脚本**

`scripts/verify-tip-safe.ps1` 内容必须只包含：

```powershell
$ErrorActionPreference = 'Stop'
cargo fmt --all -- --check
cargo clippy -p cheime-tip --all-targets -- -D warnings
cargo test -p cheime-tip
cargo build -p cheime-tip -p cheime-probe --release
& "$PSScriptRoot\..\target\release\cheime-probe.exe"
if ($LASTEXITCODE -ne 0) { throw "cheime-probe failed: $LASTEXITCODE" }
```

不得包含 `regsvr32`、installer、registry cmdlet 或 profile activation。

- [ ] **Step 5: 运行 Level 2**

```powershell
.\scripts\verify-tip-safe.ps1
```

Expected: probe 自身独立退出 0；输出 `DllCanUnloadNow == S_OK` 和 DLL SHA-256。若 probe 崩溃，只记录它的 exit code/WER；不得在宿主激活 profile。

- [ ] **Step 6: Commit**

```powershell
git add crates/cheime-probe/main.rs crates/cheime-probe/Cargo.toml scripts/verify-tip-safe.ps1
git commit -m "test: isolate TIP COM lifecycle in probe process"
```

---

### Task 8: 用安全 fake thread manager 测试非空 ActivateEx

**Files:**
- Create: `crates/cheime-probe/fake_thread_mgr.rs`
- Modify: `crates/cheime-probe/main.rs`
- Modify: `crates/cheime-probe/Cargo.toml`

**Interfaces:**
- Produces: 一个单一 allocation、canonical IUnknown 的 fake，支持 `ITfThreadMgr`、`ITfKeystrokeMgr`、`ITfSource`

- [ ] **Step 1: 先为 fake 编写自身 COM identity/lifetime 测试**

必须覆盖：跨三接口 QI、相同 IUnknown、AddRef/Release、final Release、advise 持有 sink ref、unadvise 释放 sink ref。

- [ ] **Step 2: 实现最小 fake**

fake 只实现 ActivateEx 所需调用：`AdviseKeyEventSink`、`UnadviseKeyEventSink`、`AdviseSink`、`UnadviseSink`、`GetFocus` 返回空。未知操作返回 `E_NOTIMPL`，输出参数清零。

- [ ] **Step 3: 在 probe 子进程调用非空 ActivateEx**

要求：ActivateEx S_OK → fake 记录两个 advise → Deactivate S_OK → fake 记录逆序 unadvise → sink 引用全部释放 → TIP/fake/factory 全部释放 → DllCanUnloadNow S_OK → FreeLibrary。

- [ ] **Step 4: 运行 Level 1 + Level 2**

```powershell
cargo test -p cheime-probe
.\scripts\verify-tip-safe.ps1
```

- [ ] **Step 5: Commit**

```powershell
git add crates/cheime-probe
git commit -m "test: exercise TIP activation with isolated fake TSF"
```

---

### Task 9: 建立 guest-only 注册门禁和恢复说明

**Files:**
- Create: `scripts/verify-tip-guest.ps1`
- Modify: `crates/cheime-probe/registered.rs`
- Modify: `crates/cheime-probe/profile.rs`
- Modify: `DEPLOY.md`

**Interfaces:**
- Consumes: Level 1 + Level 2 已通过的 release artifacts
- Produces: 只允许在明确 guest opt-in 下执行的 Level 3 测试

- [ ] **Step 1: 为 guest 脚本加入硬门禁**

脚本第一段必须要求：

```powershell
if ($env:CHEIME_DISPOSABLE_GUEST -ne '1') {
    throw 'Refusing registration: run only in Windows Sandbox or a revertible VM with CHEIME_DISPOSABLE_GUEST=1.'
}
```

还要把 DLL、probe、日志目录和预注册状态输出到 guest 临时目录。

- [ ] **Step 2: 明确 guest 测试顺序**

只在 guest：注册 → 查询路径 → `cheime-registered-probe` → `cheime-profile-probe` → 卸载 → 验证 CLSID/profile/category 删除 → 保存日志 → 销毁 Sandbox/回滚快照。

- [ ] **Step 3: 防止 registered/profile probe 在宿主误运行**

两个二进制启动时都检查 `CHEIME_DISPOSABLE_GUEST=1`；未设置立即 exit 2，且在 `CoInitializeEx`/`CoCreateInstance` 前退出。

- [ ] **Step 4: 更新 DEPLOY.md**

开头加入醒目警告：当前宿主禁止部署；列出 2026-07-19 事故签名 `cheime-tip.dll / 0xc000041d / offset 0x384d`；只在 Level 3 通过后另行解除禁令。

- [ ] **Step 5: 仅测试 refusal path**

宿主只允许运行：

```powershell
Remove-Item Env:CHEIME_DISPOSABLE_GUEST -ErrorAction SilentlyContinue
cargo run -p cheime-probe --bin cheime-registered-probe
```

Expected: exit 2，且在任何 COM activation 前拒绝。**不得在宿主测试 opt-in 成功路径。**

- [ ] **Step 6: Commit**

```powershell
git add scripts/verify-tip-guest.ps1 crates/cheime-probe/registered.rs crates/cheime-probe/profile.rs DEPLOY.md
git commit -m "chore: restrict TIP registration tests to disposable guests"
```

---

### Task 10: Windows Sandbox/VM Level 3 验证与宿主解禁决策

**Files:**
- No production code changes unless guest evidence identifies a reproducible defect
- Create in guest artifact bundle: `artifacts/guest-verification/<timestamp>/`

**Interfaces:**
- Consumes: exact DLL SHA-256 from Level 2
- Produces: guest logs、event logs、registry before/after、probe exit codes

- [ ] **Step 1: 创建 VM 快照或启动一次性 Windows Sandbox**

不得使用宿主系统。确认 guest 内设置：

```powershell
$env:CHEIME_DISPOSABLE_GUEST = '1'
```

- [ ] **Step 2: 核对 artifact hash**

guest 内 `Get-FileHash` 必须与 Level 2 输出一致；不一致立即停止。

- [ ] **Step 3: 执行 guest 脚本**

```powershell
.\scripts\verify-tip-guest.ps1
```

Expected: 注册路径为 guest 中的 DLL；registered probe、profile probe 均 exit 0；卸载后相关 registry keys 不存在。

- [ ] **Step 4: 检查 guest Event Log/WER**

查询测试窗口内 Application ID 1000/1001。任何 `cheime-tip.dll` fault、`0xc000041d`、`0xc0000005` 或 probe crash 均判定 Level 3 失败。

- [ ] **Step 5: 销毁 guest/回滚快照**

无论成功失败都不保留注册状态。

- [ ] **Step 6: 决策门**

只有以下全部满足，才可以单独提出“是否在宿主重新注册”的用户确认：

- Level 1 全绿；
- Level 2 完整退出并 FreeLibrary；
- x64/x86 ABI 编译通过；
- guest registered probe 通过；
- guest process-scoped profile probe 通过；
- guest Event Log/WER 无 CheIME fault；
- guest uninstall 后 registry clean。

本任务本身**不执行宿主注册**。

---

## Final Review Checklist

- [ ] 搜索不到旧 `InterfaceWrapper` 和错误 `this` 多解引用模式。
- [ ] 任意接口可成为最后释放者且只 drop owner 一次。
- [ ] 任意接口 `QI(IUnknown)` 返回 primary header 同一地址。
- [ ] QI matrix 对称、静态、失败清空输出。
- [ ] Provider `GetDisplayAttributeInfo` 为生成绑定的三参数 ABI。
- [ ] factory/TIP creator references 均平衡。
- [ ] live object 与 server lock 独立，`DllCanUnloadNow` 可回到 `S_OK`。
- [ ] runtime 不持久保存 borrowed raw `ITfThreadMgr*`。
- [ ] 外部 COM 调用不发生在 `RefCell` borrow 内。
- [ ] failure rollback 与 deactivation unadvise 顺序有测试。
- [ ] Level 2 probe 不访问注册表、不调用注册 exports。
- [ ] registered/profile probes 在无 guest opt-in 时先拒绝再退出。
- [ ] 所有真实注册与 profile activation 只发生在一次性 guest。
- [ ] 未在宿主 Windows 重新注册或激活 CheIME。
