# Noema 开发环境报告

检查日期：2026-07-25
项目目录：`/home/yang/noema`

## 系统

- 发行版：Ubuntu 26.04 LTS（Resolute Raccoon）
- 内核：Linux 7.0.0-28-generic
- 架构：x86_64
- CPU：Intel Core 5 220H，16 个逻辑 CPU，支持 VT-x
- 项目磁盘可用空间：约 121 GiB

## 已有工具

- Git 2.53.0
- curl
- jq 1.8.1

## 本次安装

- build-essential
- pkg-config 2.5.1
- GCC 15.2.0
- Clang/LLD 21.1.8
- CMake 4.2.3
- rustup 1.27.1
- Docker Engine 29.1.3
- Docker Compose v2 2.40.3
- containerd 2.2.2
- runc 1.4.0
- QEMU 10.2.1
- qemu-img 10.2.1
- OVMF 2025.11

## Rust toolchain

- rustc 1.97.1 stable（2026-07-14）
- cargo 1.97.1
- rustfmt 1.9.0-stable
- clippy 0.1.97
- rust-src 已安装

已安装的 rustup components：

- cargo
- clippy
- rust-docs
- rust-src
- rust-std
- rustc
- rustfmt

## 验证结果

### Rust

在 `/tmp` 创建一次性二进制 crate，并成功执行：

```text
cargo build
cargo test
cargo fmt --check
cargo clippy -- -D warnings
```

结果：全部通过。

### Docker

- Docker 服务状态：active
- Docker client/server 连接：成功
- Docker Compose v2：成功
- `hello-world` 一次性容器：成功运行并自动删除
- 未创建测试 volume
- 未向任何容器挂载 Docker socket

用户 `yang` 已加入 `docker` 组。新的登录会话可以直接使用 Docker；现有登录会话需要退出后重新登录，或重新启动终端会话，才能刷新附加组。

注意：`docker` 组成员可以控制 Docker daemon，实际具备主机级高权限，应只用于可信开发代码。

### QEMU/KVM

- QEMU x86_64：已安装
- QEMU TCG 软件加速：可用
- QEMU KVM 加速：可用
- `/dev/kvm`：存在
- `kvm_intel` 和 `kvm` 模块：已加载
- 当前用户对 `/dev/kvm`：具有读写 ACL
- OVMF UEFI 固件：`/usr/share/OVMF/OVMF_CODE_4M.fd`

`kvm-ok` 结果：KVM acceleration can be used。

## 仓库状态

`/home/yang/noema` 已初始化为 Git 仓库和 Rust workspace，远程仓库为
`yyydddkkk/Noema`。M0 至 M3 的开发均使用独立 `agent/*` 分支和 GitHub PR。

## 未安装

以下组件当前没有明确需求，因此未安装：

- cargo-nextest
- Kubernetes 工具
- Nix
- 本地模型运行时
- Protobuf compiler
- musl cross toolchain
- 容器构建增强插件

## 剩余人工事项

1. 项目正式使用云模型前，以独立 secret 管理方式配置 API key；不要写入仓库。
2. Docker 场景结束后，只使用仓库提供的 Compose project 清理其命名资源。

开发环境已经满足 Noema M0-M5 的基础编译、Docker 测试和 QEMU/KVM 测试需求。
