# OpenIDEA Goctl

[简体中文](README.zh-CN.md)

An OpenIDEA plugin for [go-zero](https://github.com/zeromicro/go-zero) development. It adds `.api` language support, go-zero DSL diagnostics and navigation, `.tpl` highlighting, and `goctl` code-generation actions.

## Features

- Edit `.api` files with syntax highlighting, outline, folding, snippets, and starter file templates.
- See inline diagnostics for common go-zero DSL issues, including invalid or missing syntax versions, duplicate declarations and routes, unresolved types, import cycles, and mismatched imported syntax versions.
- Navigate from a route or field type to its declaration, including references across imported `.api` files. Use Find Usages or the declaration gutter marker to locate references.
- Generate Go services with `goctl api go` and zRPC services with `goctl rpc protoc`.
- Highlight go-zero `.tpl` files, including Go template actions and foldable template blocks.
- Initialize the installed goctl version's built-in templates in a chosen custom home directory.
- Create starter `.api` and `.proto` files from OpenIDEA's New File menu.

## Compatibility and requirements

- OpenIDEA with plugin API **1.13 or later**. This package is built against plugin API 1.13 and declares `engines.openidea = "^1.13"`.
- `goctl` available on the OpenIDEA process `PATH` for code generation and template initialization. Install any additional tools required by your project, such as `protoc` and the Go protobuf plugins for zRPC generation.
- Rust 1.96+ and the `wasm32-wasip2` target are required only to build the plugin from source.

The `.oiplugin` contains a WebAssembly component and is not tied to an operating system or CPU architecture. The `goctl` executable and its toolchain are installed separately on each development machine.

## Install

1. Obtain the `.oiplugin` package. This version has no packaged release yet, so build it from source (see [Build and validate from source](#build-and-validate-from-source)); that produces `dist/shanestevenlei.goctl-0.1.0-local.oiplugin`.
2. In OpenIDEA, open **Settings → Plugins → gear → Install Plugin from Package File…**.
3. Select the package and reload OpenIDEA if prompted.

When replacing an already installed build with the same `0.1.0` version, uninstall the existing plugin first. During development, remove any linked copy of this plugin before installing the package to avoid duplicate plugin IDs.

## Use

### Generate Go code from an API definition

Open or select a `.api` file in the editor or project tree, then choose **Goctl: Generate Go from API**. Choose an output directory when prompted. The selected project-tree file takes priority if one is selected. Command output and the exit status appear in the **goctl** Output channel.

### Generate a zRPC service from a proto file

Open or select a `.proto` file and choose **Goctl: Generate zRPC from Proto**. By default, `--go_out` and `--go-grpc_out` target `<proto-directory>/pb`; `--zrpc_out` targets the proto directory. The command also applies the configured naming style and template source.

### Initialize custom goctl templates

Choose **Goctl: Copy Builtin Templates to Folder** from the command palette and select a goctl home directory. The plugin runs `goctl template init --home <directory>` using the installed goctl executable. Goctl chooses its own version and category layout and preserves template files that already exist. To use the initialized templates, set **Template source** to `local` and **Template home** to the selected directory.

### Create files

Use OpenIDEA's **New File** flow to create one of the contributed templates:

- **go-zero API File**: an API skeleton with metadata, request/response types, and a service.
- **go-zero API File (empty)**: a minimal syntax and metadata skeleton.
- **go-zero gRPC Proto File**: a proto3 service skeleton for `goctl rpc protoc`.

### Navigate API types and references

Use ⌘B or Ctrl-click on a type name in a route or field to navigate to its declaration, including slice, pointer, and array element types. Imported API files are included. When multiple declarations match, OpenIDEA offers the available targets. Use **Find Usages** on a declaration to list references, or click its gutter marker to choose among referring locations.

### `.api` diagnostics

Diagnostics appear while editing `.api` files. The configured **goctl version for API checks** controls version-dependent rules; for example, the missing `syntax = "vN"` warning is enabled for goctl 1.6 and later. Leave the setting empty to skip version-dependent rules while keeping version-independent checks active.

## Settings

Open **Settings → Plugins → OpenIDEA Goctl**:

| Setting | Purpose |
| --- | --- |
| Naming style | `gozero` (default), `goZero`, `go_zero`, `GoZero`, or `GOZERO`. |
| Template source | `builtin`, `local`, or `git`. |
| Local template directory | Home directory used when the source is `local`. |
| Git template repository / branch | Repository and optional branch used when the source is `git`. |
| goctl version for API checks | Version used by version-dependent `.api` diagnostics, such as `1.6.4`. |

The plugin invokes the program name `goctl` through OpenIDEA; it does not provide a setting for an arbitrary executable path. OpenIDEA asks for the `process:goctl` capability because goctl runs on the host and can execute project-specific code or write generated files.

## Build and validate from source

The repository includes the OpenIDEA plugin SDK 1.13 in `.sdk/`. The `openidea-plugin` build/check/package CLI is part of the OpenIDEA source repository, so a fresh clone of this plugin repository also needs a separate OpenIDEA checkout, or an installed `openidea-plugin` CLI.

With `openidea-plugin` already installed:

```sh
rustup target add wasm32-wasip2
openidea-plugin build .
openidea-plugin check .
openidea-plugin pack . --out dist/shanestevenlei.goctl-0.1.0-local.oiplugin
openidea-plugin check dist/shanestevenlei.goctl-0.1.0-local.oiplugin
```

To run the CLI from an OpenIDEA source checkout, set `OPENIDEA_DIR` to that checkout. For example, on macOS/Linux:

```sh
export OPENIDEA_DIR=/path/to/openidea
rustup target add wasm32-wasip2
cargo run --manifest-path "$OPENIDEA_DIR/Cargo.toml" -p ide-host --bin openidea-plugin -- build .
cargo run --manifest-path "$OPENIDEA_DIR/Cargo.toml" -p ide-host --bin openidea-plugin -- check .
cargo run --manifest-path "$OPENIDEA_DIR/Cargo.toml" -p ide-host --bin openidea-plugin -- pack . --out dist/shanestevenlei.goctl-0.1.0-local.oiplugin
```

Use PowerShell syntax for the environment variable on Windows:

```powershell
$env:OPENIDEA_DIR = 'C:\path\to\openidea'
cargo run --manifest-path "$env:OPENIDEA_DIR/Cargo.toml" -p ide-host --bin openidea-plugin -- build .
```

For local development, `openidea-plugin link .` registers the repository folder with OpenIDEA. Refresh **Settings → Plugins** after rebuilding. Before publishing a release, validate the SDK and `engines.openidea` declaration against the oldest OpenIDEA version the release will support.

## Publishing

The plugin ID is `shanestevenlei.goctl`, so it lives under the community publisher prefix `shanestevenlei`; `openidea.*` is reserved for plugins built and signed by OpenIDEA itself.

Release packages are **unsigned** for now, because OpenIDEA only accepts a signature whose publisher it already knows. Installing an unsigned package from disk works, but the capability dialog carries a warning:

```sh
openidea-plugin pack . --out dist/shanestevenlei.goctl-0.1.0.oiplugin
```

Signing becomes possible once `shanestevenlei` and its public key are registered in the community index. From then on, package with the publisher's own key:

```sh
openidea-plugin pack . --key <publisher-key-file> --publisher shanestevenlei --out dist/shanestevenlei.goctl-0.1.0.oiplugin
```

Marketplace installation verifies that signature against the public keys registered for `shanestevenlei`. Registration is a pull request to [l0ng-ai/openidea-plugins](https://github.com/l0ng-ai/openidea-plugins) that adds the publisher's public key and the package URL with its SHA-256.

## License

This project is licensed under the [MIT License](LICENSE).
