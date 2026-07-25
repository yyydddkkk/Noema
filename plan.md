# Noema Linux 初版开发计划

状态：Draft 0.6（M4 实现完成，真实云模型验收待显式启用）
项目目录：`/home/yang/noema`
暂定中文名：意态
核心实现语言：Rust

当前实现进度（2026-07-25）：M0 至 M3 已完成，M4 的本地实现和离线协议验收
已完成，最终真实云模型兼容性验收尚未执行。除内存闭环外，现已有统一的
Executor backend 事务接口、真实 Rust 测试 Workload、Process backend、HTTP
健康检查、SIGTERM 停止与进程回收、原子 generation/event JSON 快照，以及
受限 Docker Compose 场景。容器内已验证启动、回滚、崩溃恢复和跨容器重启
恢复。M4 新增了有界 Noema Contract、Rust 类型生成的 reply schema、严格
Gateway、确定性 mock provider 和默认不联网的 OpenAI Responses provider；
离线容器不访问 Docker socket、不访问外网，并以只读 rootfs 和非 root 用户运行。

## 1. 项目定义

Noema Linux 是一个以“目标状态”而不是“命令”作为主要控制接口的 Linux。

云端 AI 不直接生成 Shell 命令，也不直接修改系统；它读取经过筛选且带证据引用的系统状态，提交 System Intent Representation（SIR）。本地 Rust 控制平面验证 SIR，将其编译为确定性的执行计划，在隔离环境中执行、验证，最后提交新的系统 generation 或回滚。

核心数据流：

```text
用户目标
  -> 云端模型
  -> Intent SIR
  -> 本地验证器与 Planner
  -> Execution IR
  -> Executor / Reconciler
  -> Linux 机制
  -> Evidence IR
  -> 新 generation 或回滚
```

一句话原则：

> AI proposes; Noema verifies, executes, and records.

## 2. 系统边界

Noema v0 不修改 Linux 内核。它首先重做 Linux 内核之上的系统控制层，并复用现有内核提供的进程、文件系统、网络、cgroups、namespaces 和设备能力。

### Noema 负责

- 系统对象模型与目标状态
- SIR 的定义、验证和版本管理
- 计划生成与依赖分析
- Workload 生命周期
- generation、事务、提交与回滚
- observed state 的采集
- Evidence IR 与事件记录
- 云模型上下文的筛选和脱敏
- 云模型连接与协议适配
- 传统 Linux Workload 的兼容边界

### Linux 内核负责

- 进程和线程
- 虚拟内存
- 文件系统和块设备
- TCP/IP 和网络设备
- cgroups、namespaces、pidfd 等隔离机制
- 硬件驱动

### v0 明确不做

- 自定义或 fork Linux 内核
- 多 agent 调度
- 本地模型推理
- GUI 或聊天界面
- 通用包管理器
- 真实硬件安装器
- 完整的传统 Linux 软件兼容
- 重做驱动、网络栈或文件系统
- 让 AI 直接执行任意 Shell
- 在开发宿主机上进行系统级变更

## 3. 系统原则

1. **State, not commands**：AI 描述目标状态，不描述 Shell 操作步骤。
2. **AI proposes; the system decides**：AI 只能提交提案，本地确定性核心负责裁决。
3. **Every mutation creates a generation**：任何已提交变更都产生新 generation。
4. **Facts require evidence**：Observed 事实必须来自执行器、内核事件或可信探针。
5. **Invalid proposals have zero side effects**：未通过验证的 SIR 不得产生副作用。
6. **Irreversible effects cross a barrier**：不可逆副作用必须显式声明并在可逆验证完成后执行。
7. **Shell is compatibility, not control**：Shell 只能存在于调试或 legacy compatibility 环境。
8. **Cloud intelligence is replaceable**：Noema 不绑定单一模型厂商，系统状态不依赖云端会话保存。
9. **Offline means stable**：断网时已有 Workload 和本地状态收敛继续工作，只暂停新的智能决策。
10. **The host is never the test target**：开发阶段所有系统变更只发生在模拟器、容器或虚拟机内。

## 4. 三种核心表示

### 4.1 Intent SIR

由云端 AI 提交，表达目标、约束和允许的副作用，不包含任意代码或 Shell 字符串。

最小字段：

```rust
struct IntentSir {
    sir_version: SirVersion,
    proposal_id: ProposalId,
    base_generation: GenerationId,
    mutations: Vec<Mutation>,
    constraints: Vec<Constraint>,
    effect_policy: EffectPolicy,
}
```

### 4.2 Execution IR

由本地 Planner 生成，包含确定性的步骤、依赖、前置条件、验证和失败处理。云端模型无权直接提交 Execution IR。

```rust
struct ExecutionIr {
    transaction_id: TransactionId,
    base_generation: GenerationId,
    steps: Vec<ExecutionStep>,
    invariants: Vec<InvariantCheck>,
    failure_policy: FailurePolicy,
}
```

### 4.3 Evidence IR

由本地 Executor 和 Observer 生成，记录实际观察和状态变化。AI 的判断不能直接写入 Evidence IR。

```rust
struct EvidenceIr {
    transaction_id: TransactionId,
    old_generation: GenerationId,
    new_generation: Option<GenerationId>,
    observations: Vec<Observation>,
    state_changes: Vec<StateChange>,
    invariant_results: Vec<InvariantResult>,
    artifacts: Vec<ArtifactRef>,
}
```

## 5. 状态模型

系统必须区分：

```text
Desired   希望系统成为的状态
Observed  本地可信组件实际观察到的状态
Inferred  AI 或规则根据证据得出的推断
```

v0 只实现一个主要对象：`Workload`。

```rust
struct Workload {
    id: WorkloadId,
    generation: u64,
    artifact: ArtifactRef,
    desired: DesiredWorkloadState,
    observed: ObservedWorkloadState,
    health: HealthSpec,
    restart_policy: RestartPolicy,
}
```

最小状态集合：

```text
Desired:  Absent | Stopped | Running
Observed: Absent | Starting | Running | Stopped | Failed | Unknown
```

## 6. v0 最小闭环

v0 只证明以下闭环成立：

```text
提交 Workload Intent
  -> 验证 SIR
  -> 生成 Execution IR
  -> 创建 candidate generation
  -> 启动测试 Workload
  -> 采集 observed state
  -> 执行健康检查
  -> 生成 Evidence IR
  -> commit 或 rollback
  -> Reconciler 持续维持目标状态
```

第一个演示 Workload 是 Noema 自带的 Rust 测试程序，而不是 nginx、Docker 或第三方服务。它需要支持：

- 启动简单 HTTP 健康端点
- 正常退出
- 模拟崩溃
- 模拟启动超时
- 模拟健康检查失败
- 响应停止信号

## 7. 仓库规划

```text
noema/
├── Cargo.toml
├── README.md
├── plan.md
├── docs/
│   ├── constitution.md
│   ├── architecture.md
│   ├── threat-model.md
│   └── environment.md
├── specs/
│   ├── sir-v0.md
│   ├── object-model-v0.md
│   └── evidence-v0.md
├── crates/
│   ├── noema-ir/
│   ├── noema-state/
│   ├── noema-planner/
│   ├── noema-executor/
│   ├── noema-reconciler/
│   ├── noema-linux/
│   ├── noema-protocol/
│   └── noema-testkit/
├── bins/
│   ├── noemad/
│   ├── noemactl/
│   ├── noema-gateway/
│   └── noema-test-workload/
├── scenarios/
│   ├── workload-start/
│   ├── workload-crash/
│   ├── health-failure/
│   ├── stale-generation/
│   └── rollback/
├── images/
│   ├── initramfs/
│   └── qemu/
├── docker/
│   └── compose.yaml
└── xtask/
```

第一阶段保持 Rust workspace 单仓库。除非出现明确的独立发布边界，否则不拆分仓库和微服务。

## 8. 开发与运行环境

### 8.1 日常开发：Docker

Docker 是快速、可销毁的开发实验室，不是 Noema 的最终运行依赖。

```text
Docker Compose
├── noemad
├── noema-gateway
├── mock-model
└── test workloads
```

要求：

- 默认使用 mock model
- noemad 使用只读容器根文件系统
- `/run` 和 `/tmp` 使用临时文件系统
- Noema 状态保存到独立、明确命名的测试 volume
- 测试网络默认不访问公网
- 不向 noemad 挂载宿主机 `/var/run/docker.sock`
- 不向容器挂载宿主机 `/etc`、`/usr`、`/boot` 或用户主目录
- 清理命令只能删除 Noema 自己明确命名的容器、网络和 volume

### 8.2 操作系统验收：QEMU

Docker 无法可靠验证以下行为，因此阶段性测试必须使用 QEMU：

- Linux 完整启动流程
- initramfs
- noemad 作为真实 PID 1
- 根文件系统与设备初始化
- 系统级 cgroup 和 namespace 初始化
- generation 启动切换
- 启动失败后的自动回滚
- 断电和重启恢复

开发阶段必须保留串口日志和 rescue initramfs。无 Shell 是产品目标，不是早期调试限制。

### 8.3 云模型连接

早期将 gateway 运行在 VM 或容器外：

```text
Cloud Model
  -> noema-gateway（开发主机或独立容器）
  -> noemad（Docker 或 QEMU）
```

API key 不进入测试镜像，不写入日志，不提交到仓库。

## 9. 测试策略

### 9.1 单元测试

- SIR 序列化和版本检查
- 对象状态转换
- Planner 的依赖排序
- generation 创建与提交
- Evidence IR 生成

### 9.2 属性测试和 fuzzing

必须长期维护的性质：

- 非法 SIR 不改变任何状态
- 相同输入产生确定性计划
- 幂等提案重复执行不产生重复对象
- 失败事务不会改变 current generation
- Evidence 记录与实际状态变化一致
- AI 不能通过 Intent SIR 写入 Observed 状态
- 过期 base generation 必须被拒绝或显式重新规划

### 9.3 Docker 场景测试

- Workload 正常启动
- 健康检查成功与失败
- Workload 崩溃后收敛
- noemad 重启后恢复状态
- candidate generation 回滚
- gateway 与 mock model 的协议测试

### 9.4 QEMU 测试

- 镜像可启动
- noemad 可作为 PID 1
- 串口可观察启动状态
- Workload 可运行和恢复
- generation 可跨重启保存
- 无效 generation 不会成为默认启动项

### 9.5 模型兼容性测试

真实云模型测试默认关闭，显式启用时才运行，并限制：

- 最大请求次数
- 最大 token
- 最大费用
- 单场景总超时
- 可发送的数据范围

核心正确性测试不得依赖真实模型的随机输出。

## 10. 里程碑

### M0：项目定义

- 完成项目宪法
- 完成 SIR v0 草案
- 完成 Workload v0 对象定义
- 建立 Rust workspace 和基础 CI

退出条件：核心术语和不变量能够被 Rust 类型表达。

### M1：纯状态核心

- 实现 Desired、Observed、Inferred 的类型边界
- 实现 generation 和 candidate state
- 实现 append-only 事件记录的最小版本

退出条件：无 Linux 副作用即可完成状态提交和回滚测试。

### M2：Simulation backend

- 实现虚拟 Workload
- 实现故障注入
- 实现 Reconciler
- 实现 Evidence IR

退出条件：全部 v0 场景在内存模拟器通过。

### M3：Container backend

- 在 Docker 隔离环境中管理真实子进程
- 实现健康检查、停止、崩溃观察和重启
- 持久化 generation 和事件

退出条件：Docker 中完成第一个端到端演示。

### M4：Gateway 与模型契约

- [x] 定义 Noema Contract
- [x] 生成模型可读的 SIR schema
- [x] 实现 mock provider
- [x] 接入第一个可替换的云模型 provider
- [ ] 使用显式提供的 API key 完成有预算限制的真实模型兼容性验收

退出条件：未专门训练的云模型能根据 contract 提交合法 Workload SIR。

### M5：QEMU 最小系统

- 构建 Linux kernel + initramfs + noemad 镜像
- 串口日志
- rescue 启动路径
- VM 场景测试

退出条件：VM 能启动并运行与 Docker 相同的 Workload 场景。

### M6：noemad 成为 PID 1

- 初始化最小文件系统和伪文件系统
- 正确处理 signal 与僵尸进程
- 初始化 Workload runtime
- 启动失败进入恢复路径

退出条件：不依赖传统 init 完成启动、运行和正常关机。

### M7：第二类系统对象

在 `Volume` 与 `Endpoint` 中选择一个加入对象模型，不同时加入两者。

退出条件：跨对象依赖可以被 Planner、Executor 和 Evidence 正确表达。

### M8：Legacy compatibility prototype

- 定义 legacy workload 边界
- Shell 只能存在于隔离的 legacy 环境
- legacy 环境不能直接修改 Noema host state

退出条件：一个传统 Linux 程序可以运行，但无法绕过 Noema 控制平面。

## 11. 第一个端到端验收场景

```text
1. noemad 从 generation 0 启动
2. 提交创建 workload:hello 的 Intent SIR
3. SIR 验证通过
4. Planner 生成 Execution IR
5. 创建 candidate generation 1
6. 启动 noema-test-workload
7. 健康检查通过
8. 生成 Evidence IR
9. generation 1 成为 current
10. 主动终止 Workload
11. Observer 记录 Failed 或 Stopped
12. Reconciler 恢复 Workload
13. 提交一个必然健康检查失败的变更
14. candidate generation 2 被放弃
15. generation 1 继续保持 current 和健康
```

## 12. 尚未决定的问题

以下问题在相应里程碑前通过原型和测试决定，不提前锁死：

- M3 的原子 JSON 快照何时迁移为 SQLite、纯 Rust 数据库或 append-only store
- 内部协议使用 JSON、CBOR、Protobuf 或其他编码
- Workload 最终采用进程、namespace、OCI artifact 还是 microVM
- generation 覆盖哪些本地持久状态
- 不可逆副作用的具体分级和提交屏障
- Noema Contract 如何针对不同云模型生成
- noemad 是否长期保持单体，何时拆分特权 helper
- 哪些 Linux 能力确实需要内核扩展

决策标准依次是：正确性、可恢复性、可测试性、接口稳定性、性能。

## 13. 当前最近步骤

1. 增加显式启用的真实模型兼容性场景，限制请求数、token、费用和总超时。
2. 用真实云模型完成“根据 contract 创建测试 Workload”的 M4 退出验收。
3. 固化通过验收的模型 snapshot 与 contract fixture，模型升级时重跑评估。
4. 开始 M5：定义 initramfs 中 noemad、状态分区和 rescue 路径的最小边界。
5. 构建第一个只运行 Simulation backend 的 QEMU 启动镜像。

M4 只建立云模型契约，不让模型绕过 Intent SIR；在 M5 完成前不制作真实硬件
安装镜像，在 M6 完成前不把开发版本作为宿主机 PID 1。
