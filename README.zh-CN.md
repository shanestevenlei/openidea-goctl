# OpenIDEA Goctl

[English](README.md)

面向 [go-zero](https://github.com/zeromicro/go-zero) 开发的 OpenIDEA 插件，提供 `.api` 语言支持、go-zero DSL 诊断与类型导航、`.tpl` 模板高亮，以及 `goctl` 代码生成命令。

## 功能

- 编辑 `.api` 文件：语法高亮、结构大纲、代码折叠、代码片段和新建文件模板。
- 在编辑器中提示常见 go-zero DSL 问题，包括 syntax 版本缺失或无效、重复声明和路由、未解析类型、导入循环，以及导入文件的 syntax 版本不匹配。
- 从路由或字段中的类型跳转到定义，也支持跨 `import` 的 `.api` 文件。可用“查找用法”或定义行的 gutter 标记定位引用。
- 通过 `goctl api go` 生成 Go 服务代码，通过 `goctl rpc protoc` 生成 zRPC 服务代码。
- 高亮 go-zero `.tpl` 文件中的 Go 模板动作，并折叠模板代码块。
- 将当前安装的 goctl 版本自带模板初始化到自选目录。
- 通过 OpenIDEA“新建文件”菜单创建 `.api` 和 `.proto` 文件。

## 兼容性与依赖

- 需要支持插件 API **1.13 或更高版本**的 OpenIDEA。本包使用 plugin API 1.13 SDK 构建，并声明 `engines.openidea = "^1.13"`。
- 生成代码和初始化模板时，需要在 OpenIDEA 进程的 `PATH` 中找到 `goctl`。zRPC 生成还需要项目所用的工具，例如 `protoc` 和 Go protobuf 插件。
- 只有从源码构建插件时才需要 Rust 1.96+ 和 `wasm32-wasip2` 编译目标。

`.oiplugin` 包含 WebAssembly 组件，不绑定特定操作系统或 CPU 架构。每台开发机器仍需单独安装 `goctl` 及项目所需工具链。

## 安装

1. 获取 `.oiplugin` 安装包。本版本尚未发布打包产物，请先从源码构建（见 [从源码构建与校验](#从源码构建与校验)），会得到 `dist/shanestevenlei.goctl-0.1.0-local.oiplugin`。
2. 在 OpenIDEA 中打开 **Settings → Plugins → 齿轮 → Install Plugin from Package File…**。
3. 选择安装包；若 IDE 提示重新加载，按提示操作。

如果要把已安装的同版本 `0.1.0` 换成本次重打包版本，请先卸载原插件再安装。开发期间，如果已经通过目录链接加载了本插件，也请先解除链接，避免插件 ID 冲突。

## 使用

### 从 `.api` 生成 Go 代码

在编辑器中打开或在项目树中选中 `.api` 文件，然后运行 **Goctl: Generate Go from API**。按提示选择输出目录。若项目树中选中了文件，命令会优先使用该文件。命令输出和退出状态会显示在 **goctl** Output 面板。

### 从 `.proto` 生成 zRPC 服务

打开或选中 `.proto` 文件，运行 **Goctl: Generate zRPC from Proto**。默认将 `--go_out` 和 `--go-grpc_out` 写入 `<proto目录>/pb`，将 `--zrpc_out` 写入 proto 所在目录。命令也会应用已配置的命名风格和模板来源。

### 初始化自定义 goctl 模板

从命令面板运行 **Goctl: Copy Builtin Templates to Folder**，选择 goctl home 目录。插件会调用当前安装的 goctl 执行 `goctl template init --home <目录>`。模板的版本和分类目录由 goctl 自己决定；已经存在的模板文件会保留。要在代码生成时使用这些模板，请把 **Template source** 设为 `local`，并把 **Template home** 设为所选目录。

### 新建文件

通过 OpenIDEA 的 **New File** 流程选择以下模板：

- **go-zero API File**：包含元数据、请求/响应类型和 service 的 API 骨架。
- **go-zero API File (empty)**：只包含 syntax 和元数据的精简骨架。
- **go-zero gRPC Proto File**：可用于 `goctl rpc protoc` 的 proto3 service 骨架。

### 类型跳转与查找引用

在路由或字段的类型名上使用 ⌘B 或 Ctrl-click 可跳到类型定义，包括 slice、pointer 和 array 的元素类型；被导入的 API 文件也会参与解析。如果存在多个匹配定义，OpenIDEA 会列出可选目标。在定义上使用 **Find Usages** 可列出引用；点击定义行的 gutter 标记可从引用位置中选择跳转目标。

### `.api` 诊断

编辑 `.api` 文件时会显示 DSL 诊断。**goctl version for API checks** 设置控制依赖版本的规则；例如，goctl 1.6 及以上版本会检查是否缺少 `syntax = "vN"`。版本设置留空时会跳过版本相关规则，其他版本无关的检查仍然运行。

## 设置

打开 **Settings → Plugins → OpenIDEA Goctl**：

| 设置项 | 作用 |
| --- | --- |
| Naming style | 命名风格：`gozero`（默认）、`goZero`、`go_zero`、`GoZero` 或 `GOZERO`。 |
| Template source | 模板来源：`builtin`、`local` 或 `git`。 |
| Local template directory | 模板来源为 `local` 时使用的 home 目录。 |
| Git template repository / branch | 模板来源为 `git` 时使用的仓库和可选分支。 |
| goctl version for API checks | 版本相关 `.api` 诊断使用的 goctl 版本，例如 `1.6.4`。 |

插件通过 OpenIDEA 调用程序名 `goctl`，没有任意可执行文件路径设置。OpenIDEA 会请求 `process:goctl` 能力，因为 goctl 在宿主机器上运行，并可能执行项目代码或写入生成文件。

## 从源码构建与校验

仓库中的 `.sdk/` 包含 OpenIDEA plugin API 1.13 SDK。`openidea-plugin` 构建、校验和打包 CLI 属于 OpenIDEA 源码仓库，所以新克隆本仓库后，还需要单独获取一份 OpenIDEA 源码，或者安装 `openidea-plugin` CLI。

如果已安装 `openidea-plugin`：

```sh
rustup target add wasm32-wasip2
openidea-plugin build .
openidea-plugin check .
openidea-plugin pack . --out dist/shanestevenlei.goctl-0.1.0-local.oiplugin
openidea-plugin check dist/shanestevenlei.goctl-0.1.0-local.oiplugin
```

也可以用 OpenIDEA 源码仓库运行 CLI。将 `OPENIDEA_DIR` 设为 OpenIDEA 仓库路径。macOS/Linux 示例：

```sh
export OPENIDEA_DIR=/path/to/openidea
rustup target add wasm32-wasip2
cargo run --manifest-path "$OPENIDEA_DIR/Cargo.toml" -p ide-host --bin openidea-plugin -- build .
cargo run --manifest-path "$OPENIDEA_DIR/Cargo.toml" -p ide-host --bin openidea-plugin -- check .
cargo run --manifest-path "$OPENIDEA_DIR/Cargo.toml" -p ide-host --bin openidea-plugin -- pack . --out dist/shanestevenlei.goctl-0.1.0-local.oiplugin
```

Windows PowerShell 环境变量写法：

```powershell
$env:OPENIDEA_DIR = 'C:\path\to\openidea'
cargo run --manifest-path "$env:OPENIDEA_DIR/Cargo.toml" -p ide-host --bin openidea-plugin -- build .
```

本地开发时，运行 `openidea-plugin link .` 可将当前仓库链接到 OpenIDEA。重新构建后刷新 **Settings → Plugins**。发布前，应根据计划支持的最旧 OpenIDEA 版本核验 SDK 和 `engines.openidea` 声明。

## 发布

插件 ID 为 `shanestevenlei.goctl`，属于社区发布者前缀 `shanestevenlei`；`openidea.*` 保留给由 OpenIDEA 官方构建和签名的插件。

当前发布包**暂不签名**：OpenIDEA 只接受来自已知发布者的签名，未知 publisher 的签名包会在安装时被拒绝（`unknown publisher "shanestevenlei"`）。未签名包可以从磁盘安装，能力授权对话框会给出警告：

```sh
openidea-plugin pack . --out dist/shanestevenlei.goctl-0.1.0.oiplugin
```

等 `shanestevenlei` 及其公钥登记进社区索引后即可签名，届时使用发布者自己的密钥：

```sh
openidea-plugin pack . --key <发布者密钥文件> --publisher shanestevenlei --out dist/shanestevenlei.goctl-0.1.0.oiplugin
```

从插件市场安装时，OpenIDEA 会用登记在 `shanestevenlei` 名下的公钥校验该签名。登记方式是向 [l0ng-ai/openidea-plugins](https://github.com/l0ng-ai/openidea-plugins) 提交 pull request，添加发布者公钥以及带 SHA-256 的包地址。

## 许可证

本项目采用 [MIT License](LICENSE)。
