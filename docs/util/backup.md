# `util/backup.rs` 文档

## 1. 模块职责

`src/util/backup.rs` 提供**导入文件快照备份**能力：用户每次经「文件 → 导入」打开文件时，把所选文件复制一份到 `~/.MyExcel/backup/` 做版本化备份。命名规则为 `原文件名_yyyymmddhhmmss.扩展名`（保留原扩展名），备份目录不存在时递归创建。

该模块为**纯工具模块**：不依赖 GUI（egui/eframe）、不持有任何状态，仅对外暴露一个 `pub fn`，便于复用与单元测试。

> **定位与口径**：备份是**附加功能**。函数返回 `io::Result`，失败时由调用方决定处理方式；当前调用方（`viewer.rs::start_async_load`）选择**仅记日志、不阻断导入与加载**——即便备份失败，用户仍能正常打开并解析文件。

## 2. 主要函数

### `backup_imported_file`

```rust
pub fn backup_imported_file(src: &std::path::Path) -> std::io::Result<std::path::PathBuf>
```

把源文件 `src` 复制到 `~/.MyExcel/backup/`，返回备份文件的完整路径；目录创建或文件复制失败时返回底层 `io::Error`。

**参数**

| 参数 | 类型 | 说明 |
|------|------|------|
| `src` | `&Path` | 用户选择并即将导入的源文件路径 |

**返回**

- `Ok(PathBuf)`：备份成功，值为备份文件的完整路径（如 `~/.MyExcel/backup/template_20260625143005.xlsx`）。
- `Err(io::Error)`：目录递归创建（`create_dir_all`）或文件复制（`std::fs::copy`）失败。

**命名示例**：`template.xlsx` → `~/.MyExcel/backup/template_20260625143005.xlsx`；无扩展名文件 → `data_20260625143005`。

## 3. 核心逻辑与数据流

```
backup_imported_file(src)
   │
   ├─ 1. 解析备份目录：dirs::home_dir().join(".MyExcel").join("backup")
   │        （home_dir 为 None 时回退当前目录 "."）
   ├─ 2. std::fs::create_dir_all(&backup_dir)      // 不存在则递归创建
   ├─ 3. 拼装备份文件名：
   │        stem   = src.file_stem()（缺省 "import"）
   │        ts     = util::date::now_timestamp14()  // "yyyymmddhhmmss"
   │        name   = "{stem}_{ts}.{ext}"（无 ext 则 "{stem}_{ts}"）
   └─ 4. std::fs::copy(src, backup_dir.join(name))  // 复制
        ▼
   Ok(backup_path) / Err(io::Error)
```

关键点：① 主目录定位与全项目一致走 `dirs::home_dir()`（回退 `"."`）；② 扩展名用 `Path::extension()`，**保留原扩展名**且缺失时优雅降级（不加 `.ext`）；③ 时间戳取自 [`util::date::now_timestamp14`](./date.md#now_timestamp14)，UTC 口径。

## 4. 依赖关系

- **对外依赖**：`dirs`（`home_dir` 定位主目录）、`std::fs`（`create_dir_all` / `copy`）、`std::path`、`std::io`。
- **内部依赖**：[`util::date::now_timestamp14`](./date.md)（时间戳）。
- **被依赖**：[`gui/viewer.rs`](../gui/viewer.md) 的 `ExcelViewer::start_async_load` 在导入入口调用本函数。

## 5. 与导入流程的关系

```
菜单「文件 → 导入」
   ▼ draw_import_dialog（rfd 文件选择）
返回选中路径 path
   ▼ viewer.rs start_async_load(path, ctx)
   └─ 后台线程（顺序执行，避免阻塞 UI）:
        ├─ backup_imported_file(Path::new(&path))   ◄── 本模块：备份到 ~/.MyExcel/backup/
        │     └─ Err 仅 log::warn! 记日志，不阻断
        └─ ExcelData::load_from_file(path)            // 真正解析加载
```

备份与加载在**同一后台线程内顺序执行**（先备份后加载）：既避免文件复制阻塞 UI，又保证即使用户文件后续解析失败，所选文件仍已被备份，便于回溯原始导入内容。
