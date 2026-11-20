# Windows 构建（无需在本机安装 Rust/Cargo）

本项目新增了手动 GitHub Actions 工作流：`.github/workflows/build-windows-custom.yml`。
它会在 GitHub 的 Windows x64 构建机上编译 GPUI 版本，并生成便携 ZIP 和安装程序 EXE；不会创建 GitHub Release。

## 一次性准备

1. 在 GitHub 上 Fork `Kuddev/pebrel`，或使用你自己已有的 Pebrel 仓库。
2. 用 GitHub Desktop 克隆你自己的仓库。
3. 将本压缩包内 `pebrel-main` 文件夹里的全部内容复制到克隆目录，覆盖同名文件。不要把 `pebrel-main` 再套成仓库里的子文件夹。
4. 在 GitHub Desktop 中检查变更，提交并推送到你自己的分支。

## 生成 Windows 软件包

1. 打开你自己的 GitHub 仓库，进入 **Actions**。
2. 选择 **Build Windows package (custom)**。
3. 点击 **Run workflow**，选择刚才推送的分支。
4. 版本标签保留默认值 `2.5.0` 即可，然后启动工作流。
5. 等待工作流完成。打开本次运行页面底部的 **Artifacts**，下载 `pebrel-windows-x64`。

下载的构建产物包含：

- `Pebrel-v2.5.0-windows-x64.zip`：便携版，解压后直接运行 `pebrel.exe`，不要拆散目录中的 `runtime` 文件。
- `Pebrel-v2.5.0-windows-x64-setup.exe`：安装程序，双击按提示安装。

Actions 构建产物默认保留 14 天，请及时下载保存。此工作流生成的程序未做代码签名，Windows 首次运行时可能出现 SmartScreen 提示；请确认下载来源是你自己的仓库和对应构建记录。

## 本机要求

你的 Windows 电脑不需要安装 Rust、Cargo、Visual Studio C++ Build Tools 或 Inno Setup；这些构建依赖由 GitHub Windows runner 负责。
