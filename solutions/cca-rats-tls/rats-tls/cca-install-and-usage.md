# RATS-TLS CCA 安装与使用手册

本文用于在真实 Arm CCA 环境中安装补丁、编译 RATS-TLS，并完成单向证明、双向证明和 baseline 校验。

文中的服务端是“被证明方”，客户端是“验证方”。单向证明只要求服务端具备 CCA 能力；双向证明要求两端都具备 CCA 能力。

## 1. 准备机器和网络

### 1.1 单向证明

准备两台 Linux 机器：

- CCA 服务端：AArch64，能够通过 Linux TSM configfs 生成 CCA evidence。
- 验证客户端：能够连接服务端 TCP 端口。客户端可以不是 CCA 机器。

记录服务端 IPv4 地址。以下示例使用 `192.0.2.10`，执行时请替换为真实地址。

确认客户端能够访问服务端：

```sh
ping -c 3 192.0.2.10
```

如果服务端启用了防火墙，放行测试端口：

```sh
sudo ufw allow 1234/tcp
```

系统没有使用 UFW 时，不需要执行该命令，但必须保证 TCP 1234 没有被其他防火墙拦截。

### 1.2 双向证明

双向证明时，客户端也会生成 CCA evidence。因此客户端和服务端都必须是 AArch64 CCA 机器，并且两端都必须通过第 2 节的检查。

## 2. 检查 CCA TSM 接口

在所有需要生成 evidence 的机器上执行：

```sh
uname -m
sudo mountpoint -q /sys/kernel/config || sudo mount -t configfs none /sys/kernel/config
sudo test -w /sys/kernel/config/tsm/report/report0/inblob
sudo test -r /sys/kernel/config/tsm/report/report0/outblob
sudo test -r /sys/kernel/config/tsm/report/report0/auxblob
```

成功标准：

- `uname -m` 输出 `aarch64`。
- 三条 `test` 命令均无输出，且返回码为 `0`。

可执行下面的命令检查上一条命令的返回码：

```sh
echo $?
```

如果 `/sys/kernel/config/tsm/report/report0` 不存在，仅挂载 configfs 不能创建 CCA report 接口。需要检查内核是否启用了 TSM，以及 CCA guest 驱动是否正常加载。

## 3. 安装编译依赖

下面以 Debian/Ubuntu 为例。QCBOR 和 t_cose 安装在 `/usr/local`，两端需要编译或运行 verifier 时均应安装。

### 3.1 安装系统包

```sh
sudo apt-get update
sudo apt-get install -y \
  build-essential \
  cmake \
  git \
  pkg-config \
  autoconf \
  libtool \
  libssl-dev \
  libcbor-dev \
  uthash-dev \
  python3
```

`uthash-dev` 是编译 `cca-client` 的必需依赖。

### 3.2 安装 QCBOR

本手册使用 QCBOR `v1.6.1`：

```sh
cd /tmp
git clone --branch v1.6.1 --depth 1 \
  https://github.com/laurencelundblade/QCBOR.git
cmake -S QCBOR -B QCBOR/build \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=ON
cmake --build QCBOR/build --parallel
sudo cmake --install QCBOR/build
sudo ldconfig
```

检查安装结果：

```sh
test -f /usr/local/include/qcbor/qcbor.h
find /usr/local/lib /usr/local/lib64 -maxdepth 1 \
  -name 'libqcbor.so*' -o -name 'libqcbor.a'
```

第一条命令应返回 `0`；第二条命令应至少显示一个 QCBOR 库文件。

### 3.3 安装 t_cose

本手册使用 t_cose `v1.2.0`，并选择 OpenSSL crypto provider：

```sh
cd /tmp
git clone --branch v1.2.0 --depth 1 \
  https://github.com/laurencelundblade/t_cose.git
cmake -S t_cose -B t_cose/build \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=ON \
  -DCRYPTO_PROVIDER=OpenSSL
cmake --build t_cose/build --parallel
sudo cmake --install t_cose/build
sudo ldconfig
```

检查安装结果：

```sh
test -f /usr/local/include/t_cose/t_cose_sign1_verify.h
find /usr/local/lib /usr/local/lib64 -maxdepth 1 \
  -name 'libt_cose.so*' -o -name 'libt_cose.a'
```

如果 CMake 找不到 QCBOR，重新配置 t_cose：

```sh
cmake -S /tmp/t_cose -B /tmp/t_cose/build \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=ON \
  -DCRYPTO_PROVIDER=OpenSSL \
  -DQCBOR_INCLUDE_DIR=/usr/local/include \
  -DQCBOR_LIBRARY=/usr/local/lib/libqcbor.so
```

如果 QCBOR 实际安装在 `/usr/local/lib64`，将最后一个路径改为 `/usr/local/lib64/libqcbor.so`。

## 4. 下载指定版本的 RATS-TLS

补丁只能应用到以下上游提交：

```text
40f7b78403d75d13b1a372c769b2600f62b02692
samples: Clean up banner info
```

先进入保存本补丁文件的目录，再记录其绝对路径：

```sh
cd /path/to/cca-rats-tls/rats-tls
PATCH_DIR="$(pwd)"
```

下载上游代码：

```sh
mkdir -p /tmp/rats-tls-cca-work
cd /tmp/rats-tls-cca-work
git clone https://github.com/inclavare-containers/rats-tls.git
cd rats-tls
git switch --detach 40f7b78403d75d13b1a372c769b2600f62b02692
git rev-parse HEAD
```

最后一条命令必须输出：

```text
40f7b78403d75d13b1a372c769b2600f62b02692
```

## 5. 应用两个补丁

必须按顺序执行：

```sh
cd /tmp/rats-tls-cca-work/rats-tls
git am \
  "$PATCH_DIR/0001-cca-verifier-and-attester.patch" \
  "$PATCH_DIR/0002-cca-verifier-and-attester-cli-command.patch"
```

检查结果：

```sh
git log --oneline -3
test -f src/attesters/cca/collect_evidence.c
test -f src/verifiers/cca/verify_evidence.c
test -f samples/cca-client/client.c
test -f samples/cca-server/server.c
test -f scripts/cca_collect_baselines.py
```

`git log` 顶部应依次出现 `cca verifier and attester cli command` 和 `cca verifier and attester`。五条 `test` 命令都应返回 `0`。

不要把补丁应用到其他提交。

## 6. 编译、安装并检查

在已应用补丁的 RATS-TLS 根目录执行：

```sh
cd /tmp/rats-tls-cca-work/rats-tls
cmake -S . -B build-cca \
  -DCMAKE_BUILD_TYPE=Release \
  -DRATS_TLS_BUILD_TYPE=release \
  -DRATS_TLS_BUILD_MODE=cca
cmake --build build-cca --parallel
sudo cmake --install build-cca
sudo ldconfig
```

检查 CCA 插件和 sample：

```sh
test -r /usr/local/lib/rats-tls/attesters/libattester_cca.so
test -r /usr/local/lib/rats-tls/verifiers/libverifier_cca.so
test -x /usr/share/rats-tls/samples/cca-server
test -x /usr/share/rats-tls/samples/cca-client
```

检查动态库是否缺失：

```sh
ldd /usr/local/lib/rats-tls/verifiers/libverifier_cca.so | grep 'not found'
ldd /usr/share/rats-tls/samples/cca-client | grep 'not found'
```

成功时，两条 `grep` 命令都没有输出。

如果出现 `libqcbor.so`、`libt_cose.so` 或 `librats_tls.so` 找不到，添加动态库目录：

```sh
echo '/usr/local/lib' |
  sudo tee /etc/ld.so.conf.d/rats-tls-cca.conf
echo '/usr/local/lib64' |
  sudo tee -a /etc/ld.so.conf.d/rats-tls-cca.conf
echo '/usr/local/lib/rats-tls' |
  sudo tee -a /etc/ld.so.conf.d/rats-tls-cca.conf
sudo ldconfig
```

再次执行 `ldd` 检查，直到不再出现 `not found`。

## 7. 启动服务端

在 CCA 服务端执行：

```sh
sudo /usr/share/rats-tls/samples/cca-server \
  --ip 0.0.0.0 \
  --port 1234 \
  --once \
  --log-level debug
```

服务端需要访问 TSM report 文件，所以本示例使用 `sudo`。

成功启动时应看到：

```text
CCA RA-TLS server listening on 0.0.0.0:1234
```

另开终端检查监听端口：

```sh
sudo ss -ltnp | grep ':1234'
```

`--once` 表示处理一个连接后退出。移除该参数后，服务端会继续接受连接。

## 8. 完成单向 CCA 证明

在客户端执行，将 IP 替换为服务端真实 IPv4 地址：

```sh
/usr/share/rats-tls/samples/cca-client \
  --ip 192.0.2.10 \
  --port 1234 \
  --message 'hello-cca' \
  --log-level debug
```

客户端会验证服务端 device certificate chain、Platform/Realm token 签名、Realm challenge 和 RAK binding，然后完成 echo 测试。

成功标准：

```text
CCA server: hello-cca
CCA log and baseline verification passed
```

服务端应看到：

```text
CCA client: hello-cca
```

如果两端在同一台 CCA 机器上，可将客户端 IP 改为 `127.0.0.1`。

## 9. 完成双向 CCA 证明

服务端和客户端都必须通过第 2 节的 TSM 检查。

在服务端执行：

```sh
sudo /usr/share/rats-tls/samples/cca-server \
  --ip 0.0.0.0 \
  --port 1234 \
  --once \
  --mutual \
  --log-level debug
```

在客户端执行：

```sh
sudo /usr/share/rats-tls/samples/cca-client \
  --ip 192.0.2.10 \
  --port 1234 \
  --message 'hello-mtls-cca' \
  --mutual \
  --log-level debug
```

客户端使用 `sudo` 是因为双向证明时客户端也要访问 TSM report 文件并生成 evidence。

双向证明成功时，客户端仍应显示 echo 内容和 `CCA log and baseline verification passed`。任意一端的 CCA evidence 校验失败，TLS 握手都会失败。

## 10. 配置证明策略

客户端可用下列参数约束服务端 claims；服务端仅在同时使用 `--mutual` 时，才用这些参数约束客户端 claims。

| 参数 | 作用 | 限制 |
| --- | --- | --- |
| `--realm-profile TEXT` | 精确匹配 Realm profile | 文本必须与 claim 完全一致 |
| `--platform-profile TEXT` | 精确匹配 Platform profile | 文本必须与 claim 完全一致 |
| `--rim HEX` | 匹配 Realm initial measurement | 偶数个十六进制字符，1～64 字节 |
| `--implementation-id HEX` | 匹配 Platform implementation ID | 64 个十六进制字符，即 32 字节 |
| `--instance-id HEX` | 匹配 Platform instance ID | 66 个十六进制字符，即 33 字节 |

先运行一次第 8 节的 debug 命令，从客户端输出中记录对应 claim，再将其作为预期值：

```sh
/usr/share/rats-tls/samples/cca-client \
  --ip 192.0.2.10 \
  --rim 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
  --implementation-id 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
  --instance-id 010123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
  --message 'policy-check'
```

示例值只是长度示范，不能原样用于实际验证。必须替换为可信设备上采集并审核过的值。

## 11. 生成和使用 baseline

baseline 必须从已经确认可信、软件版本正确的 CCA 服务端采集。不要从待验证设备生成 baseline 后立即用它验证同一设备。

### 11.1 准备日志文件

服务端默认使用：

```text
IMA:      /sys/kernel/security/ima/binary_runtime_measurements
CCEL:     /sys/firmware/acpi/tables/CCEL
Boot log: /sys/firmware/acpi/tables/data/CCEL
```

文件位于其他位置时，在启动服务端时覆盖路径：

```sh
sudo /usr/share/rats-tls/samples/cca-server \
  --ip 0.0.0.0 \
  --port 1234 \
  --ima-log /path/to/binary_runtime_measurements \
  --ccel-table /path/to/CCEL \
  --boot-log /path/to/CCEL-event-log \
  --log-level info
```

注意：服务端参数是 `--boot-log`，客户端参数是 `--bootlog`，两者拼写不同。

### 11.2 IMA 日志解析

只解析 IMA binary log，不应用 digest baseline：

```sh
/usr/share/rats-tls/samples/cca-client \
  --ip 192.0.2.10 \
  --ima-log \
  --message 'ima-parse'
```

成功时应看到：

```text
CCA IMA log verification passed
```

IMA baseline 每行格式为：

```text
<sha1|sha256> <hex-digest> <path>
```

示例：

```text
sha256 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef /usr/bin/example
```

限制：

- 仅支持 `sha1` 和 `sha256`。
- SHA-1 digest 必须是 40 个十六进制字符。
- SHA-256 digest 必须是 64 个十六进制字符。
- path 非空，最长 4096 字节。
- 最多 100000 条 baseline。
- 空行和以 `#` 开头的行会被忽略。

使用 baseline：

```sh
/usr/share/rats-tls/samples/cca-client \
  --ip 192.0.2.10 \
  --digest /path/to/ima-baseline.txt \
  --message 'ima-baseline-check'
```

`--digest` 会自动启用 IMA 日志请求和解析，不必再写 `--ima-log`。

### 11.3 生成 platform baseline

先启动可信 CCA 服务端，再在已应用补丁的源码目录执行：

```sh
cd /tmp/rats-tls-cca-work/rats-tls
mkdir -p /tmp/cca-baseline
python3 scripts/cca_collect_baselines.py \
  --client /usr/share/rats-tls/samples/cca-client \
  --ip 192.0.2.10 \
  --port 1234 \
  --workdir /tmp/cca-baseline \
  --platform-out /tmp/cca-baseline/platform-baseline.json \
  --firmware-out /tmp/cca-baseline/firmware-events.txt
```

成功时应看到：

```text
wrote /tmp/cca-baseline/platform-baseline.json (... platform components)
wrote /tmp/cca-baseline/firmware-events.txt (... firmware events)
```

检查 platform baseline：

```sh
python3 -m json.tool /tmp/cca-baseline/platform-baseline.json
```

把生成文件复制到验证客户端后执行：

```sh
/usr/share/rats-tls/samples/cca-client \
  --ip 192.0.2.10 \
  --platform /tmp/cca-baseline/platform-baseline.json \
  --message 'platform-check'
```

成功时应看到：

```text
CCA platform JSON baseline verification passed
```

platform JSON 最大 16 MiB，最多 100000 个组件。每个组件必须包含名称、measurement、版本和 hash algorithm；`signer_id` 可选。

脚本生成的字段名 `firware_name` 和 `firware_version` 保留了当前 sample 的拼写。校验器同时接受拼写正确的 `firmware_name` 和 `firmware_version`。

字段值 `-`、`*` 或空字符串表示通配。安全策略中不建议对 `measurement` 使用通配值。

### 11.4 重放 boot log

先启动可信 CCA 服务端，再在客户端执行：

```sh
mkdir -p /tmp/cca-bootlog
cd /tmp/cca-bootlog
/usr/share/rats-tls/samples/cca-client \
  --ip 192.0.2.10 \
  --bootlog \
  --message 'bootlog-replay' \
  --log-level info
```

`--bootlog` 会请求 CCEL table 和 event log，重放 measurement registry 1、2，并与握手中已验证的 Realm REM 比较。

成功标准：

```text
boot log replay matched CCA REM0
boot log replay matched CCA REM1
CCA boot log verification passed
```

该命令还会在当前目录保存：

```text
ccel.bin
event_log.bin
boot_log.bin
```

只需要保存二进制文件、不打印每条事件时，使用：

```sh
/usr/share/rats-tls/samples/cca-client \
  --ip 192.0.2.10 \
  --dump-bootlog \
  --message 'dump-bootlog'
```

### 11.5 生成 firmware 事件摘要

`cca_collect_baselines.py` 的 `--firmware-out` 输出事件摘要清单，不是 `--firmware` 可直接读取的 JSON。

从已有 `boot_log.bin` 生成摘要：

```sh
cd /tmp/rats-tls-cca-work/rats-tls
python3 scripts/cca_collect_baselines.py \
  --skip-client \
  --boot-log /tmp/cca-bootlog/boot_log.bin \
  --firmware-out /tmp/cca-baseline/firmware-events.txt
```

查看 SHA-256 事件：

```sh
less /tmp/cca-baseline/firmware-events.txt
```

每行格式为：

```text
<registry> <sha256-hex> [event description]
```

先筛选常见固件路径：

```sh
grep -Ei 'grub|boot.*efi|vmlinuz|kernel|initramfs' \
  /tmp/cca-baseline/firmware-events.txt
```

如果 EFI image 没有可打印的描述，重新执行第 11.4 节的 `--bootlog --log-level debug` 命令，结合事件类型、digest 和 raw data 确认 GRUB 条目。

根据可信机器的事件内容确认 GRUB EFI image、`grub.cfg`、kernel 和 initramfs 对应的 SHA-256 digest，然后创建 JSON：

```json
{
  "hash_alg": "sha-256",
  "grub": "64个十六进制字符",
  "grub.cfg": "64个十六进制字符",
  "kernels": [
    {
      "version": "允许的内核版本",
      "kernel": "64个十六进制字符",
      "initramfs": "64个十六进制字符"
    }
  ]
}
```

验证 JSON 格式：

```sh
python3 -m json.tool /tmp/cca-baseline/firmware-baseline.json
```

### 11.6 使用 firmware baseline

```sh
/usr/share/rats-tls/samples/cca-client \
  --ip 192.0.2.10 \
  --firmware /tmp/cca-baseline/firmware-baseline.json \
  --message 'firmware-check' \
  --log-level info
```

`--firmware` 会自动请求并重放 boot log，不必同时写 `--bootlog`。

成功标准：

```text
boot log replay matched CCA REM0
boot log replay matched CCA REM1
CCA firmware JSON baseline verification passed
CCA boot log verification passed
```

firmware JSON 最大 16 MiB，只接受 `hash_alg` 为 `sha-256`。`grub`、`grub.cfg` 和 `kernels` 都必须存在，digest 必须是 64 个十六进制字符。

## 12. CLI 参数速查

### 12.1 cca-server

| 参数 | 作用和限制 |
| --- | --- |
| `--ip/-i IPv4` | 监听 IPv4；默认 `0.0.0.0`，不接受主机名和 IPv6 |
| `--port/-p PORT` | TCP 端口；范围 1～65535，默认 1234 |
| `--mutual/-m` | 开启双向 CCA 证明 |
| `--once/-1` | 处理一个连接后退出 |
| `--ima-log PATH` | 覆盖 IMA binary log 路径 |
| `--ccel-table PATH` | 覆盖 CCEL ACPI table 路径 |
| `--boot-log PATH` | 覆盖 CCA event log 路径 |
| `--log-level/-l LEVEL` | `debug`、`info`、`warn`、`error`、`fatal` 或 `off` |
| `--help/-h` | 显示帮助 |

服务端还接受第 10 节的五个策略参数，但只有开启 `--mutual`、验证客户端 evidence 时才有意义。

### 12.2 cca-client

| 参数 | 作用和限制 |
| --- | --- |
| `--ip/-i IPv4` | 服务端 IPv4；默认 `127.0.0.1`，不接受主机名和 IPv6 |
| `--port/-p PORT` | TCP 端口；范围 1～65535，默认 1234 |
| `--message/-M TEXT` | echo 消息；1～4096 字节 |
| `--ima-log/-I` | 请求并解析 IMA binary log |
| `--digest/-d PATH` | 使用 IMA digest baseline；自动启用 IMA |
| `--bootlog/-g` | 请求、打印、保存并重放 boot log |
| `--dump-bootlog` | 保存并重放 boot log，但不打印每条事件 |
| `--platform/-P PATH` | 使用 platform component JSON baseline |
| `--firmware/-f PATH` | 使用 firmware JSON baseline；自动启用 boot log |
| `--mutual/-m` | 开启双向 CCA 证明 |
| `--log-level/-l LEVEL` | `debug`、`info`、`warn`、`error`、`fatal` 或 `off` |
| `--help/-h` | 显示帮助 |

客户端还接受第 10 节的五个策略参数，用于约束服务端 CCA claims。

## 13. 常见错误

### 13.1 `failed to pre-init cca attester`

原因：本端需要生成 evidence，但 TSM report 接口不存在或权限不足。

处理：

```sh
sudo mountpoint -q /sys/kernel/config || sudo mount -t configfs none /sys/kernel/config
sudo ls -l /sys/kernel/config/tsm/report/report0
```

若目录仍不存在，检查 CCA guest 内核和 TSM 驱动。

### 13.2 `failed to load ... libverifier_cca.so`

原因：插件或其依赖库未安装，或者动态加载器没有搜索对应目录。

处理：

```sh
sudo ldconfig
ldd /usr/local/lib/rats-tls/verifiers/libverifier_cca.so | grep 'not found'
```

按第 6 节补充 `/etc/ld.so.conf.d/rats-tls-cca.conf`。

### 13.3 `connect failed: Connection refused`

原因：服务端未启动、IP/端口错误，或者 `--once` 已处理过一个连接并退出。

处理：

```sh
sudo ss -ltnp | grep ':1234'
```

重新启动服务端，并确认客户端使用的是服务端 IPv4。

### 13.4 `failed to open ...`

原因：服务端无法读取 IMA、CCEL 或 boot log 文件，或客户端无法读取 baseline。

处理：

```sh
sudo test -r /sys/kernel/security/ima/binary_runtime_measurements
sudo test -r /sys/firmware/acpi/tables/CCEL
sudo test -r /sys/firmware/acpi/tables/data/CCEL
```

路径不同则使用服务端的 `--ima-log`、`--ccel-table`、`--boot-log` 指定真实路径。

### 13.5 `boot log replay does not match CCA REM`

原因：event log 不是本次 evidence 对应的日志、registry 1/2 缺少 SHA-256 digest，或重放值与 token 中 REM 不一致。

处理：

```sh
cd /tmp/cca-bootlog
/usr/share/rats-tls/samples/cca-client \
  --ip 192.0.2.10 \
  --bootlog \
  --message 'replay-debug' \
  --log-level debug
```

确认服务端 `--boot-log` 指向当前系统的真实 CCEL event log，不要使用其他机器或其他启动周期保存的文件。

### 13.6 `invalid IMA baseline`

检查每行是否严格符合：

```text
sha1 40个十六进制字符 路径
sha256 64个十六进制字符 路径
```

不要写 `sha-256`；IMA baseline 只接受 `sha256` 或 `sha1`。

### 13.7 `firmware JSON baseline must contain ...`

确认 JSON 同时包含：

```text
hash_alg
grub
grub.cfg
kernels
```

其中 `hash_alg` 必须是 `sha-256`，`kernels` 至少包含一个对象。

### 13.8 `CCA ... baseline mismatch`

baseline 与当前设备或软件版本不一致。不要直接把当前待测设备的值覆盖到正式 baseline。

应在可信基准设备上重新采集，审核差异，确认属于允许的升级后再更新 baseline。

## 14. 已知限制

- 仅支持真实 AArch64 CCA 环境和 Linux TSM configfs report interface。
- 不实现旧 sample 的 FDE key 传输。
- 客户端和服务端 CLI 只接受 IPv4 字面值，不接受域名和 IPv6。
- IMA 只校验日志结构和可选 digest baseline，不把 IMA replay 结果与 CCA REM 绑定。
- boot log 只用 SHA-256 重放 registry 1、2；缺少对应 SHA-256 digest 时校验失败。
- firmware JSON 校验只支持 SHA-256。
- IMA 日志接收上限为 1 GiB，CCEL table 为 1 MiB，CCA event log 为 5 MiB。
- 当前 firmware 校验仅在日志中出现 kernel/initramfs 事件时比较对应 digest；不要把它当作完整启动链策略。
- evidence 内的 token 和 certificate 长度使用本机 `size_t`，不应视为跨位宽或跨端序的稳定协议。
- sample 会打印 echo 消息和 CCA claims。不要用 sample 发送密钥、口令或其他敏感业务数据。

## 15. 一次性验收清单

完成后逐项确认：

- CCA attester 机器存在可读写的 TSM report interface。
- QCBOR、t_cose 和 RATS-TLS 动态库没有 `not found`。
- 两个补丁按顺序应用到指定提交。
- 单向证明完成，客户端收到相同 echo。
- 双向模式下两端均生成并验证 CCA evidence。
- 启用策略参数后，正确值通过，修改一个十六进制字符后失败。
- boot log replay 同时匹配 REM0 和 REM1。
- platform/IMA/firmware baseline 的正确文件通过，错误 digest 失败。
