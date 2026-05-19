# Excel 表格查看器 - 运行说明

## 已编译的可执行文件
```
D:\php\umya-spreadsheet-excel\target\debug\umya-spreadsheet-excel.exe
```

## 运行方式

### 方式1：直接运行
双击运行：
```
D:\php\umya-spreadsheet-excel\target\debug\umya-spreadsheet-excel.exe
```

### 方式2：从命令行运行
打开PowerShell或CMD，执行：
```powershell
cd D:\php\umya-spreadsheet-excel
.\target\debug\umya-spreadsheet-excel.exe
```

## 程序功能

### ✅ 核心功能
- **精确渲染**：根据Excel单元格的value值、文本颜色、字体大小、背景颜色进行精确渲染
- **表格布局**：支持单元格合并、行高列宽自适应，符合Excel原始格式
- **交互功能**：滚动、选中高亮、工作表切换
- **性能优化**：虚拟化渲染，1000+行数据流畅运行
- **错误处理**：Excel数据格式异常时给出明确提示

### ✅ 支持的样式
- 字体大小
- 文本颜色
- 背景颜色
- 单元格值
- 单元格公式
- 单元格合并

## 编译说明

如果需要重新编译，请使用单线程编译：

```bash
cargo clean
cargo build --jobs 1
```

如果编译遇到系统级错误（如 `proc-macro2` 构建失败），可能需要：
1. 清理 Rust 缓存：`cargo clean`
2. 禁用增量编译
3. 或者等待系统环境恢复后重试

## 项目依赖

- `umya-spreadsheet = "2.2.0"` - Excel文件读取
- `egui = "0.29"` - GUI框架
- `eframe = "0.29"` - eframe应用框架
- `rfd = "0.14"` - 原生文件对话框