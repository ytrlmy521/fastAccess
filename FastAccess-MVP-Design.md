# FastAccess

## 极速文件访问器 MVP 技术设计文档

版本：1.0 MVP Revised  
语言：Rust 2021  
平台：Windows 10 / Windows 11

---

## 1. 项目概述

FastAccess 是一个 Windows 原生轻量级文件快速访问工具。用户通过键盘唤起极简搜索窗口，快速访问 Windows Recent 中可识别的最近文件和文件夹。

核心体验：

> 按快捷键 → 输入关键词 → 回车打开文件

## 2. MVP 边界

数据来源为 Windows Shell 的 `FOLDERID_Recent`，即系统维护的最近项目快捷方式。

MVP 不承诺所有文件访问历史、全磁盘搜索、所有应用最近记录或内容搜索。产品承诺是：

> 快速访问 Windows Recent 中可识别的最近文件和文件夹。

## 3. 性能目标

| 指标 | 目标 |
| --- | --- |
| 快捷键唤起 | P95 < 100ms |
| 搜索响应 | P95 < 10ms |
| 空闲 CPU | 接近 0% |
| 内存 | 目标 < 50MB |

这些指标只在 Windows Release 构建实测后成立，不提前保证。

## 4. 架构

```text
Slint UI
    │
    ▼
In-memory Search Engine
    ▲
    │
Collector Worker
    ▲
    │
Windows Recent (*.lnk)
```

Collector 获取 Recent 路径、扫描和解析 `.lnk`、生成 `RecentItem`、按观察时间排序并发布不可变快照。搜索只访问内存快照，不访问磁盘、网络或目标文件元数据。

## 5. 数据模型

```rust
pub struct RecentItem {
    pub target: PathBuf,
    pub display_name: String,
    pub display_path: String,
    pub search_text: String,
    pub observed_at_ms: u64,
    pub kind: ItemKind,
}
```

`observed_at_ms` 是 FastAccess 用于排序的观察时间，当前实现采用 `.lnk` 文件修改时间，不等同于 NTFS Last Access Time。

## 6. 技术选型

| 领域 | 技术 |
| --- | --- |
| 语言 | Rust 2021 |
| GUI | Slint |
| 搜索 | nucleo-matcher |
| Shortcut | lnk |
| 缓存 | serde / serde_json |
| Windows API | windows-rs（windows-sys） |

## 7. 缓存

缓存位置：

```text
%LOCALAPPDATA%\FastAccess\cache.json
```

缓存采用带 `schema_version` 的 JSON。写入先生成 `cache.json.tmp`，执行 flush 和磁盘同步，再以替换语义移动到 `cache.json`，避免产生半文件。

## 8. 线程模型

- UI 线程：Slint、输入、内存搜索、列表更新。
- Worker 线程：Recent 扫描、`.lnk` 解析、缓存写入。
- Hotkey 线程：Win32 消息循环，收到 `WM_HOTKEY` 后通过 `invoke_from_event_loop` 通知 UI。

## 9. 用户交互

全局快捷键：`Alt + Shift + Space`

| 按键 | 动作 |
| --- | --- |
| `↑` / `↓` | 选择 |
| `Enter` | 打开 |
| `Esc` | 隐藏 |
| 输入文字 | 搜索 |

## 10. MVP 成功标准

用户按快捷键、输入关键词、获得内存搜索结果并按 Enter 打开目标，形成完整闭环。全盘索引、Jump List、内容搜索、标签和插件均留到后续版本。

## 11. 参考资料

- [Microsoft：KNOWNFOLDERID](https://learn.microsoft.com/windows/win32/shell/knownfolderid)
- [Microsoft：SHGetKnownFolderPath](https://learn.microsoft.com/windows/win32/api/shlobj_core/nf-shlobj_core-shgetknownfolderpath)
- [Microsoft：RegisterHotKey](https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-registerhotkey)
- [Slint Rust 文档](https://docs.slint.dev/latest/docs/rust/slint/)
- [`lnk` 0.6.4 文档](https://docs.rs/lnk/0.6.4/lnk/)
- [`nucleo-matcher` 0.3.1 文档](https://docs.rs/nucleo-matcher/0.3.1/nucleo_matcher/)
