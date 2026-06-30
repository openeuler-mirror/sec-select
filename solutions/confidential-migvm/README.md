# MigVM AArch64

[English](README.md) | [中文](README_CN.md)

A standalone bare-metal payload for CCA (ARM v9 Realm) and VirtCCA (ARM v8 TrustZone) confidential VM migration.

## Quick Start

```bash
# Build
cargo build --release --target aarch64-unknown-none

# Run on QEMU
bash sh_script/qemu.sh run

# VirtCCA guest mode
cargo build --release --features virtcca --target aarch64-unknown-none
bash sh_script/qemu.sh virtcca

# CCA Realm mode (pending hardware)
cargo build --release --features cca --target aarch64-unknown-none
```

## Features

| Feature | Description |
|---------|-------------|
| `default` | Normal QEMU VM (no confidential computing) |
| `virtcca` | VirtCCA guest mode (ARM v8 TrustZone S-EL2, TSI SMC) |
| `cca` | CCA Realm mode (ARM v9, RSI SMC) |

## Architecture

```
AArch64 boot (exception.S) → MMU init → FDT parse → GICv3 init → Timer init
    → PCIe enum → VirtIO-net + VirtIO-vsock → smoltcp DHCP → TCP 5001/5002
```

## Documentation

- [Port Summary](docs/aarch64_port_summary.md)
- [CODE FLOW](docs/CODE_FLOW.md)
- [VirtIO Plan](docs/virtio_mmio_plan.md)
- [GIC Controller](docs/platform/gic.md)
- [Timer + TRNG](docs/platform/timer_trng.md)
- [Memory / MMU](docs/platform/mm_memory.md)
- [CCA / VirtCCA](docs/platform/cca_virtcca.md)
- [Debug Logs](docs/debug/debug_logs.md)
- [Debug Methods](docs/debug/debug_methods.md)

## Prerequisites

- Rust nightly/aarch64-unknown-none target
- QEMU 8.2+ with `qemu-system-aarch64`
- For VirtCCA: KVM with `kvm-type=cvm` + TMM firmware

## Verification Status

| Environment | DHCP | TCP | Timer | TSI | MMU |
|------------|------|-----|-------|-----|-----|
| KVM + GICv3 | ✅ | ✅ | ✅ 30s | N/A | ✅ |
| VirtCCA | ✅ | ✅ | ✅ | ✅ 11/11 | ✅ |
| CCA Realm | ⚠️ | ⚠️ | ⚠️ | ⚠️ code ready | ⚠️ |

## License

This project is dual-licensed:

- **BSD-2-Clause-Patent** — applies to files retaining the original Intel
  copyright (generic stubs / standard templates: `src/acpi.rs`,
  `src/mm/layout.rs`, `src/mm/heap.rs`).
- **Mulan Permissive Software License v2 (Mulan PSL v2)** — applies to all
  other source files (AArch64-specific implementations) carrying the
  Huawei Technologies copyright.

See the [LICENSE](LICENSE) file for the full text of both licenses. Each
source file declares its applicable license in its header.
