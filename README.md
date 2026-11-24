# report-client

报表打印客户端(GPUI Kit 版),移植自 `report-printer-electron`。

## 运行

```bash
# macOS 需要完整 Xcode(Metal);工具链见 rust-toolchain.toml(gpui 需要 beta)
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
cargo run
```

## 接口(127.0.0.1:18119,与 Electron 版一致)

- HTTP:`GET /health` `/printers` `/jobs`,`POST /print`
- WebSocket `/ws`:`printReport`、`health`

## 打印原理

Rust 没有内置 HTML 渲染,流程为:按打印机配置生成 HTML → 本机 Chrome/Edge/Chromium 无头渲染为 PDF → 提交系统打印队列 → 轮询队列确认完成。

- macOS / Linux:`lp`、`lpstat`
- Windows:PowerShell 枚举打印机,`SumatraPDF.exe` 静默打印
- 环境变量:`REPORT_CLIENT_BROWSER` 指定浏览器路径,`REPORT_CLIENT_SUMATRA` 指定 SumatraPDF 路径

配置与状态保存在系统配置目录的 `report-client/` 下。

## 打包与图标

- macOS:`./scripts/bundle-macos.sh` 生成 `target/bundle/Report Client.app`(含 Dock 图标)。
- Windows:在 Windows 上安装 Rust 和 Visual Studio C++ 构建工具（含 Windows SDK），使用 PowerShell 执行：

  ```powershell
  .\scripts\bundle-windows.ps1
  # 可选：将已有的 SumatraPDF 可执行文件一起打包
  .\scripts\bundle-windows.ps1 -SumatraPath "C:\path\to\SumatraPDF.exe"
  ```

  输出为 `target/bundle/Report-Client-<版本>-<目标架构>.zip`，其中 exe 通过 `build.rs` 嵌入图标。ZIP 不是安装程序，也未签名。打印需要目标机器安装 Edge 或 Chrome，以及 SumatraPDF；后者可以通过上述参数随包提供，或者在目标机器安装或使用 `REPORT_CLIENT_SUMATRA` 指定路径。

将项目推送到 GitHub 后，Build packages 工作流会在推送、PR 和手动运行时分别打包 Windows 和 macOS。可在 Actions 对应运行的 Artifacts 中下载 `report-client-windows`、`report-client-macos-arm64`（Apple Silicon）和 `report-client-macos-x64`（Intel）。Windows 包不包含 SumatraPDF；macOS 包未签名或公证。
