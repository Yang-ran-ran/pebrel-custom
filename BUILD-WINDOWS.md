# Windows x64 构建与发布

本仓库使用 `.github/workflows/build-windows-custom.yml` 构建 Pebrel 的 Windows x64 版本。

## CI 与 Windows 构建如何分工

- Pull Request 和相关源码更新只执行 Ubuntu 预检：Cargo workspace metadata、DeepSeek Harness bridge 的 Node 测试，以及变更 Rust 文件的固定版本 rustfmt 检查。
- **Windows runner 不会因 Pull Request 或普通 push 自动启动。** 只有手动执行发布工作流才会占用 Windows 构建额度。
- Windows 发布工作流会先完成 Windows workspace/GPUI 测试，再编译 GPUI app 与 runtime helper，校验便携包/安装器并进行安装、启动文件和卸载 smoke test，最后上传 ZIP 与安装器 EXE。
- `package_version` 留空时从 `pebrel_app/Cargo.toml` 读取；填写时必须与 manifest 完全一致。当前源码 manifest 版本为 `2.5.0`，不要因为 GitHub Release 标签不同而手动改成另一版本。

## 手动生成软件包

1. 在 GitHub 仓库打开 **Actions**。
2. 选择 **Build Windows x64 release packages**。
3. 选择 **Run workflow**，选择要发布的分支。
4. 版本输入框可以留空，自动读取 manifest；只有版本已在源码中更新并同步时才填写。
5. 运行成功后，在工作流页面的 **Artifacts** 下载 `pebrel-windows-x64`。

产物包括：

- `Pebrel-v<version>-windows-x64.zip`：便携版，解压后运行 `pebrel.exe`，请保留 `runtime` 子目录。
- `Pebrel-v<version>-windows-x64-setup.exe`：Inno Setup 安装程序。

构建产物保留 30 天。当前工作流不会自动创建 GitHub Release。程序未做代码签名，首次运行可能显示 Windows SmartScreen 提示；请核对下载仓库与 Actions 运行记录。

## 本机要求

无需在本机安装 Rust、Cargo、Visual Studio C++ Build Tools 或 Inno Setup；这些构建工具由 GitHub runner 提供。
