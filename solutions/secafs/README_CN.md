<p align="center">
  <h1 align="center">SecAFS</h1>
</p>

<p align="center">
  安全智能体文件系统（Secure Agent Filesystem）—— 一个面向 AI 智能体、支持事务、可审计、可回滚的文件系统，由 openGauss（兼容 PostgreSQL）提供支撑。
</p>

---

> **⚠️ 警告：** 本软件处于 BETA 阶段，仍可能存在缺陷和非预期行为。使用生产数据时请务必谨慎，并确保已做好备份。

## 🎯 SecAFS 是什么？

SecAFS 是一个专为 AI 智能体设计的文件系统，由 **openGauss** 提供支撑 —— 它是一个兼容 PostgreSQL 的数据库，也是默认后端。正如传统文件系统为应用程序提供文件和目录抽象一样，SecAFS 为 AI 智能体提供其所需的存储抽象，并带来只有成熟数据库才能提供的事务保证、可审计性和回滚能力。由于 openGauss 使用 PostgreSQL 线缆协议，标准 PostgreSQL 同样可用 —— 任何可以传入 `opengauss://` 的地方都可以传入 `postgres://` URL。

SecAFS 仓库由以下部分组成：

* **[OpenClaw 集成](integrations/openclaw)** —— 使用 SecAFS 的旗舰方式：一个即插即用的智能体网关集成，为每一次对话提供其专属的、由 SecAFS 支撑并通过 FUSE 挂载的工作区（详见下文）。
* **SDK** —— [TypeScript](sdk/typescript)、[Python](sdk/python) 和 [Rust](sdk/rust) 库，用于以编程方式访问文件系统。
* **[CLI](MANUAL.md)** —— 挂载 SecAFS（Linux 上使用 FUSE，macOS 上使用 NFS）、以写时复制（copy-on-write）方式运行沙箱命令，并从终端访问文件。

## 🔌 在 OpenClaw（及其他智能体网关）中使用 SecAFS

SecAFS 的设计目标是作为工作文件系统位于智能体运行时*之下*。首个集成面向 **[OpenClaw](integrations/openclaw)** 发布，相同的设计无需改动 SecAFS 核心即可扩展到其他智能体网关（例如 Hermes 风格的网关）。

**集成带来了什么**

* **每次对话拥有真实的工作区。** 每个聊天会话都获得其专属的、由 openGauss 支撑并通过 FUSE 挂载的目录，智能体在该挂载点*内部*运行 —— 因此它读取和写入的一切都会被捕获，具备事务性且可审计，没有任何内容会逃逸到宿主机。
* **回滚融入交互循环。** 逐消息和逐回合的检查点让你可以将一次对话的文件系统回退到任意更早的时间点 —— 在不丢失会话其余内容的前提下撤销智能体的错误。
* **会话导出 / 导入。** 将整个对话（工作区 + 历史记录）快照为可移植的归档文件，并在别处恢复。

**为什么插件方式很重要**

* **对宿主网关零改动。** SecAFS 以*外部插件*的形式发布，通过网关的公开插件接口加载。OpenClaw 检出保持**上游原样（pristine upstream）** —— 它可以无合并冲突地跟踪新版本发布，因为所有 SecAFS 相关代码都位于 [`integrations/openclaw/`](integrations/openclaw) 中。
* **只依赖稳定的公开契约。** 该集成仅依赖两样东西：网关的线缆协议及其插件 SDK。智能体的工作目录通过一个上游支持的机制被重定向到 SecAFS 挂载点 —— 没有核心补丁，没有私有钩子。
* **天然与网关无关。** 由于其耦合是标准网关协议 + 插件 SDK（而非分叉），同一个 SecAFS 守护进程和后端可以随着新的智能体网关和运行时的出现而为它们提供支撑 —— 今天是 OpenClaw，明天是其他（Hermes 风格的网关乃至更多）。

下方的 **[快速开始](#-快速开始)** 部分展示如何把这套环境跑起来；**[`integrations/openclaw/README.md`](integrations/openclaw/README.md)** 提供完整的架构与配置说明。

## 🧑‍💻 快速开始

### 在 OpenClaw 中（智能体聊天）

运行一个完整的、由 SecAFS 支撑的聊天，其中每次对话都获得其专属的、通过 FUSE 挂载并由 openGauss 支撑的工作区，智能体在其*内部*工作。SecAFS 作为外部插件加载进一个**原样（pristine）** 的 OpenClaw —— 你无需修改 OpenClaw 检出。

使用同级目录布局 `<workspace>/{openclaw, secafs}`，然后：

```bash
WS=<workspace>        # the directory containing openclaw/ and secafs/

# 1. Pristine OpenClaw — zero changes (one-time)
git -C "$WS" clone https://github.com/openclaw/openclaw.git
git -C "$WS/openclaw" checkout v2026.6.8-alpha.1
( cd "$WS/openclaw" && pnpm install && pnpm build )

# 2. Build the SecAFS daemon and the external plugin
( cd "$WS/secafs/cli" && cargo build -p secafs --no-default-features )
( cd "$WS/secafs/integrations/openclaw/plugin" && npm install && npm run build )

# 3. openGauss + the stack (daemon + gateway in one userns) + the browser bridge
( cd "$WS/secafs" && docker compose -f docker-compose.dev.yml --profile opengauss up -d opengauss )
( cd "$WS/secafs/integrations/openclaw/bridge" && bash run-stack.sh & )
( cd "$WS/secafs/integrations/openclaw/bridge" && \
  OPENCLAW_DIR="$WS/openclaw" \
  GATEWAY_TOKEN=$(node -e "console.log(require(require('os').homedir()+'/.openclaw/openclaw.json').gateway.auth.token)") \
  PORT=8090 node bridge.mjs & )

# 4. open the SecAFS Console, start a session, and chat
echo "open http://127.0.0.1:8090"
```

在第 3 步之前，你需要一次性配置 `~/.openclaw/openclaw.json` —— 即模型提供方以及外部插件路径（`plugins.load.paths`）。完整的配置、前置条件（Linux 用户命名空间、一次性的守护进程构建符号链接）以及排障指南都在 **[`integrations/openclaw/README.md`](integrations/openclaw/README.md)** 中；可直接复制粘贴的最简命令序列在 **[`integrations/openclaw/RUNBOOK.md`](integrations/openclaw/RUNBOOK.md)** 中。

### 使用 CLI

在 openGauss 中初始化一个 SecAFS 数据库：

```bash
$ secafs init opengauss://user:pass@localhost/my_agent
Created agent filesystem in openGauss
Database: opengauss://user:pass@localhost/my_agent
```

> 更想用标准 PostgreSQL？换一下协议方案即可：`secafs init postgres://user:pass@localhost/my_agent`。

查看智能体文件系统：

```bash
$ secafs fs opengauss://localhost/my_agent ls
f hello.txt

$ secafs fs opengauss://localhost/my_agent cat hello.txt
hello from agent
```

查看智能体的操作时间线：

```bash
$ secafs timeline opengauss://localhost/my_agent
ID   TOOL                 STATUS       DURATION STARTED
4    execute_code         pending            -- 2024-01-05 09:44:20
3    api_call             error           300ms 2024-01-05 09:44:15
2    read_file            success          50ms 2024-01-05 09:44:10
1    web_search           success        1200ms 2024-01-05 09:43:45
```

使用 FUSE（Linux）或 NFS（macOS）挂载一个 SecAFS 文件系统：

```bash
$ secafs mount opengauss://localhost/my_agent ./mnt
$ echo "hello" > ./mnt/hello.txt
$ cat ./mnt/hello.txt
hello
```

在带有写时复制叠加层的沙箱中运行程序：

```bash
$ secafs run bash
Welcome to SecAFS!

The following directories are writable:
  - /home/user/project (copy-on-write)

Everything else is read-only.
```

阅读 **[用户手册](MANUAL.md)** 以获取完整文档。

### 使用 SDK

在你的项目中安装 SDK：

```bash
npm install secafs-sdk
```

在你的智能体代码中使用它：

```typescript
import { SecAFS } from 'secafs-sdk';

// Connect to openGauss (a postgres:// URL works too)
const agent = await SecAFS.open({ postgresUrl: 'opengauss://localhost/my_agent' });

// Key-Value operations
await agent.kv.set('user:preferences', { theme: 'dark' });
const prefs = await agent.kv.get('user:preferences');

// Filesystem operations
await agent.fs.writeFile('/output/report.pdf', pdfBuffer);
const files = await agent.fs.readdir('/output');

// Tool call tracking
await agent.tools.record(
  'web_search',
  Date.now() / 1000,
  Date.now() / 1000 + 1.5,
  { query: 'AI' },
  { results: [...] }
);
```

## 💡 为什么选择 SecAFS？

SecAFS 为智能体状态管理提供以下优势：

* **沙箱隔离**：所有文件操作都被限定在一个 openGauss 数据库内，与宿主机文件系统隔离。外部污染无法进入，危险操作也无法扩散。
* **可审计性**：每一次文件操作、工具调用和状态变更都会被记录。你可以用 SQL 查询智能体的完整历史，以调试问题、分析行为或满足合规要求。
* **智能体操作回滚**：将多个智能体操作作为单个事务处理。中间变更在任务完成前不可见。支持逐步检查点（`SAVEPOINT`）、快照和回滚。
* **协作隔离**：文件修改具备事务性。多个智能体并发地在文件系统上操作时，通过 openGauss MVCC 彼此隔离。
* **远程部署**：借助成熟、托管的 openGauss（或任何兼容 PostgreSQL 的）基础设施，高效地远程部署智能体环境。
* **可移植性**：将整个智能体的状态导出为可移植的归档文件，用于备份、快照恢复和分发。

## 🔧 SecAFS 的工作原理

SecAFS 是一个通过 SDK 访问的智能体文件系统，它为智能体状态管理提供三个核心接口：

* **文件系统（Filesystem）：** 一个面向文件和目录的类 POSIX 文件系统。
* **键值（Key-Value）：** 一个用于智能体状态和上下文的键值存储。
* **工具调用（Toolcall）：** 一条用于调试和分析的工具调用审计轨迹。

SecAFS 的核心是一个由 openGauss 支撑的存储引擎。智能体所做的一切 —— 它创建的每个文件、它存储的每一份状态、它调用的每个工具 —— 都存于一个具备完整事务保证的 openGauss 数据库中。FUSE/NFS 层将该数据库投射为一棵普通的目录树，因此现有工具和智能体无需改动即可工作，而每一处变更都保持被捕获且可逆。

## 开发用数据库 + 守护进程

在本地开发时，使用随附的 docker-compose 启动一个 openGauss 实例：

```bash
./scripts/dev-up.sh                  # openGauss (default)
export SECAFS_POSTGRES_URL='opengauss://secafs:Secafs%21123@localhost:5433/secafs'
cargo run -p secafs -- serve api
```

更想改用标准 PostgreSQL？

```bash
./scripts/dev-up.sh postgres
export SECAFS_POSTGRES_URL=postgres://secafs:secafs@localhost:5433/secafs
```

停止：

```bash
./scripts/dev-down.sh
```

对于生产部署，请通过 `--pg-url` 或 `$SECAFS_POSTGRES_URL` 提供你自己的 openGauss（或 PostgreSQL）。

## 🌱 起源与差异

SecAFS 的架构源自 **agentfs** —— 一个开源的智能体文件系统框架，它开创性地提出了将数据库暴露为智能体文件系统的理念。SecAFS 保留了这一核心思想，但是一个经过大幅重构的系统，面向生产级智能体基础设施重新设计：

* **以工业级数据库取代嵌入式数据库。** SecAFS 通过线缆协议运行在 **openGauss / PostgreSQL** 之上，而非嵌入式 SQLite 引擎 —— 为智能体文件系统带来 MVCC 并发、托管与远程部署、基于角色的访问控制以及合规级的持久性。
* **回滚与检查点作为一等特性。** 事务性的智能体操作回滚、快照以及逐消息/逐回合的检查点，让你可以将智能体的文件系统回退到任意更早的状态。
* **不同的目标场景。** SecAFS 围绕多对话、多智能体的基础设施设计 —— 每次对话隔离的工作区、通过 MVCC 实现的并发智能体隔离，以及与智能体网关的直接集成（参见 [在 OpenClaw 中使用 SecAFS](#-在-openclaw及其他智能体网关中使用-secafs)）。
* **运维加固。** 会话导出/导入、可修复丢失挂载的挂载守护器（mount-keeper），以及可自愈的数据库连接池，让长时间运行的部署更加稳健。

## 📚 了解更多

- **[OpenClaw 集成](integrations/openclaw/README.md)** —— 运行一个完整的、由 SecAFS 支撑的智能体聊天。
- **[用户手册](MANUAL.md)** —— 使用 SecAFS CLI 和 SDK 的完整指南。
- **[SPEC.md](SPEC.md)** —— 数据库内的模式（schema）与数据模型。

## 📝 许可证

木兰宽松许可证第2版（MulanPSL-2.0）—— 见 http://license.coscl.org.cn/MulanPSL2
