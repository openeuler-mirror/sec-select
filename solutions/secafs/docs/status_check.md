# 状态检查

排查顺序，按依赖关系从底往上查：

```
openGauss (5433)  →  daemon (socket)  →  gateway (18789)  →  bridge (8090)  →  SSH 转发  →  浏览器
      ↑                    ↑                    ↑                 ↑               ↑
   1.1-1.4              2.1-2.5              2.3-2.5           3.1-3.6         4.1-4.2
```

哪一步 `[FAIL]`，就先修哪一步，再往上查。

下面按步骤给出**检查命令**和**预期输出**。

## 步骤 1：openGauss 容器

### 1.1 容器是否在跑

```bash
docker ps --filter "publish=5433" --format "table {{.Names}}\t{{.Status}}"
```

**预期**：看到一行，`Status` 为 `Up ... (healthy)`。
- `Up ... (health: starting)` → 还在初始化，等 30s 再查
- `Up ... (unhealthy)` → 容器在跑但 DB 起不来，看日志 `docker logs <容器名>`
- **无输出** → 容器没起，需要启动

### 1.2 端口监听

```bash
ss -ltnp | grep 5433
```

**预期**：至少一行 `LISTEN ... 0.0.0.0:5433 ...`。

### 1.3 数据库 `secafs` 存在且可连

```bash
docker exec $(docker ps --filter "publish=5433" -q | head -1) \
  bash -c 'export LD_LIBRARY_PATH=/usr/local/opengauss/lib && /usr/local/opengauss/bin/gsql -d secafs -p 5432 -U secafs -W "Secafs!123" -c "SELECT 1"'
```

**预期**：
```
 ?column?
----------
        1
(1 row)
```

- 报 `database "secafs" does not exist` → 手动 `CREATE DATABASE secafs OWNER secafs;`（用 omm 用户）
- 报密码错 → 确认 `GS_PASSWORD`，或试 `omm` 用户

### 1.4 表结构是否初始化（可选，更深一层）

```bash
docker exec $(docker ps --filter "publish=5433" -q | head -1) \
  bash -c 'export LD_LIBRARY_PATH=/usr/local/opengauss/lib && /usr/local/opengauss/bin/gsql -d secafs -p 5432 -U secafs -W "Secafs!123" -c "\dt"'
```

**预期**：列出若干表（`volumes`、`inodes`、`state` 等）。空列表也行——daemon 启动时会自动建表（`initialize_schema` 是幂等的）。

---

## 步骤 2：daemon + gateway + bridge（run-stack.sh）

### 2.1 进程是否在跑

```bash
ps -ef | grep -E "run-stack|secafs serve|openclaw gateway" | grep -v grep
```

**预期**：看到三到四行左右：
- `bash run-stack.sh`
- `secafs serve api --socket ...`
- `node ... gateway run --force`
- `node bridge.mjs`

### 2.2 secafs daemon socket 存在

```bash
ls -la /root/.secafs/run/secafs.sock
```

**预期**：`srwxr-xr-x ... /root/.secafs/run/secafs.sock`（类型 `s` = socket）。
- `No such file` → daemon 没起来，看日志

### 2.3 gateway 端口监听

```bash
ss -ltnp | grep 18789
```

**预期**：`LISTEN ... 127.0.0.1:18789 ...`（注意 gateway 默认只绑 loopback）。

### 2.4 gateway 是否就绪

```bash
grep "\[gateway\] ready" /tmp/secafs-stack.log
```

**预期**：一行匹配，类似 `... [gateway] ready`。
- **无输出** → 还没就绪或启动失败，看日志

### 2.5 日志检查

```bash
# 启动日志（看 daemon socket up + gateway ready）
tail -30 /tmp/secafs-stack.log

# 只看错误
grep -iE "error|fatal|panic|ENOENT" /tmp/secafs-stack.log | tail -20
```

**预期关键字**：
- `[run-stack] daemon socket up` ← daemon 起来
- `[run-stack] starting Console bridge on :8090…` ← bridge 起来（如端口未被占用）
- `[secafs-chat] plugin registered` ← 插件加载
- `[gateway] ready` ← 完全就绪

**常见错误**：
- `database "secafs" does not exist` → 回到步骤 1.3 修
- `Permission denied` → 检查 `secafs` 二进制权限、init.sql 权限
- `Address already in use` → 上次的进程没杀干净，`pkill -f "openclaw gateway run"`

### 2.6 daemon 实际能响应 RPC（可选，深度验证）

```bash
# 如果装了 websocat / nc 能连 unix socket
echo '{"jsonrpc":"2.0","id":1,"method":"list"}' | \
  nc -U /root/.secafs/run/secafs.sock
```

有 JSON 响应就说明 daemon 正常。如果没 nc/不会用，跳过——后面 bridge 测试能覆盖。

---

## 步骤 3：bridge（由 run-stack.sh 自动启动）

> bridge 由 `run-stack.sh` 在 `BRIDGE_PORT`（默认 8090）未被占用时自动启动。如需单独重启，见 deploy 文档第 7 步。

### 3.1 进程在跑

```bash
ps -ef | grep "node bridge.mjs" | grep -v grep
```

**预期**：一行 `node bridge.mjs`。

### 3.2 端口监听

```bash
ss -ltnp | grep 8090
```

**预期**：`LISTEN ... 127.0.0.1:8090 ...`（注意是 `127.0.0.1` 不是 `0.0.0.0`）。

### 3.3 HTTP 响应

```bash
curl -sS -o /dev/null -w "HTTP %{http_code}\n" http://127.0.0.1:8090/
```

**预期**：`HTTP 200`。
- `HTTP 404` → 前端目录找不到，`bridge.mjs` 从自身位置推导，确认 `frontend/` 目录与 `bridge.mjs` 同级
- `Connection refused` → bridge 没起或端口没监听

### 3.4 HTML 内容

```bash
curl -sS http://127.0.0.1:8090/ | head -5
```

**预期**：看到 `<!doctype html>` 和 `<title>SecAFS Console</title>`。

### 3.5 日志

```bash
tail -10 /tmp/secafs-bridge.log
```

**预期**：最后一行 `[secafs-bridge] http+ws on http://127.0.0.1:8090 -> gateway ws://127.0.0.1:18789`。

### 3.6 WS 端到端测试（关键，验证 bridge → gateway 通）

```bash
# 如果有 websocat
echo '{"type":"req","id":1,"method":"secafs.status"}' | \
  websocat -1 ws://127.0.0.1:8090
```

**预期**：返回 JSON，`"ok":true` 且 `payload` 有内容。
- `ok:false` + `error` → bridge 连不上 gateway 或 token 不对，看 3.5 日志

---

## 步骤 4：SSH 端口转发

### 4.1 转发进程在跑

终端跑：
```bash
ps -ef | grep "ssh -L 8090" | grep -v grep
```

**预期**：一行 ssh 进程。

### 4.2 本地 8090 通了

终端跑：
```bash
curl -sS -o /dev/null -w "HTTP %{http_code}\n" http://127.0.0.1:8090/
```

**预期**：`HTTP 200`。
- `Connection refused` → ssh 转发断了，重跑 `ssh -L ...`
- 卡住 → ssh 通道在但服务器端 bridge 没起

---

## 步骤 5：浏览器访问 + paired.json

### 5.1 配置文件是否生成

服务器终端跑：
```bash
ls -la ~/.openclaw/devices/paired.json
```

**预期**：文件存在，大小不为 0。
- `No such file` → 还没触发配对，打开浏览器连一次；连了还没有，看 gateway 日志

### 5.2 设备 scope 是否齐全

```bash
node -e 'const d=JSON.parse(require("fs").readFileSync(require("os").homedir()+"/.openclaw/devices/paired.json","utf8"));for(const id in d)console.log(id,"scopes=",d[id].scopes)'
```

**预期**：每个设备的 `scopes` 包含 `operator.admin`。
- 只有 `operator.pairing` → 跑 seed 命令补 scope

### 5.3 gateway 日志看配对事件

```bash
grep -iE "pair|device" /tmp/openclaw/openclaw-*.log | tail -10
```

**预期**：看到 `device paired` 或类似记录。

---

## 从宿主机检查 FUSE 挂载

挂载点位于 `run-stack.sh` 的私有挂载命名空间内，因此宿主机上直接 `ls ~/.secafs/mounts/<id>` 看到的是**空**目录（这是设计如此）—— 即使是 root 也一样。要查看内部，需进入守护进程的命名空间：

```bash
cd "$WS/secafs/integrations/openclaw/bridge"
./secafs-ns.sh ls -la ~/.secafs/mounts/<id>/
./secafs-ns.sh                 # 无参数 → 在命名空间内开交互式 shell
```

要获取*实际挂载了什么*的真相，优先看守护进程自己的视图：`secafs.status` 的 `mountCount`，或 `cat /proc/<daemon-pid>/mounts`（注意：daemon 崩溃后那里可能残留死挂载 —— daemon 的 RPC 列表才是权威）。

---

