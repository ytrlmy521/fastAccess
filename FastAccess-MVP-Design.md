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

数据来源包括两条互补的事件流：

- Explorer 完成文件夹导航时发出的 Shell COM 事件，用于精确记录运行期间访问过的文件夹。
- Windows Shell 的 `FOLDERID_Recent`，用于获取系统维护的最近文件快捷方式。

MVP 不承诺全磁盘搜索、内容搜索，或恢复 FastAccess 启动之前未被 Windows
记录的文件夹访问。产品承诺是：

> 快速访问 FastAccess 运行期间通过 Explorer 访问的文件夹，以及 Windows
> Recent 中可识别的最近文件。

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
    ▲              ▲
    │              │
Explorer Events   Windows Recent (*.lnk)
```

Explorer 监听器使用 `NavigateComplete2` 与 `DShellWindowsEvents` 接收系统回调，
不使用轮询。Collector 获取 Recent 路径、扫描和解析 `.lnk`、生成
`RecentItem`，两条数据流按目标路径去重并按观察时间排序。搜索只访问最多
500 条的内存快照，不访问磁盘、网络或目标文件元数据。

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

`observed_at_ms` 是 FastAccess 用于排序的观察时间。Explorer 文件夹事件采用
事件到达时间，Windows Recent 项目采用 `.lnk` 文件修改时间；两者都不等同于
NTFS Last Access Time。

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

缓存采用带 `schema_version` 的 JSON。写入先生成 `cache.json.tmp`，执行 flush
和磁盘同步，再以替换语义移动到 `cache.json`，避免产生半文件。所有写入由单一
后台线程执行，250ms 内的连续更新会合并，最迟 1 秒落盘，通知队列容量为 1，
避免快速导航造成线程和 I/O 放大。

## 8. 线程模型

- UI 线程：Slint、输入、内存搜索、列表更新。
- Recent Worker 线程：按请求扫描 Recent、解析 `.lnk`；容量为 1 的队列合并重复刷新。
- Cache Writer 线程：合并保存请求并执行原子缓存替换。
- Explorer 事件线程：STA COM 消息循环，仅在窗口注册、关闭或导航完成时工作。
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
