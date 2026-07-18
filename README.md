# CheIME Windows Frontend

Windows TSF TIP、engine host、候选窗口渲染和安装工具。

`cheime-core` 作为 Git submodule 引入，本仓库只包含 Windows 平台代码：TSF COM 适配、Named Pipe I/O、GDI 渲染和安装注册。

## 仓库结构

| Crate | 职责 |
|------|------|
| `cheime-tip-core` | 候选窗口 GDI 渲染、信道调度、平台动作应用、Pipe I/O 抽象 |
| `cheime-tip` | TSF TIP COM DLL (x64 + x86)、`DllRegisterServer` |
| `cheime-engine-host` | Engine host exe (x64)、命名管道监听、Session Actor |
| `cheime-installer` | 安装/注册/卸载 CLI 工具 |

## 构建

```powershell
# x64
cargo build --release

# x86 TIP
cargo build --release --target i686-pc-windows-msvc -p cheime-tip
```

## 质量门

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
