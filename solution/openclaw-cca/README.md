# OpenClaw-CCA

Run the [OpenClaw](https://github.com/nicholasgriffintn/openclaw) LLM gateway inside an ARM CCA (Confidential Compute Architecture) Realm, with remote attestation binding LUKS volume decryption to Realm integrity measurements — only a verified Realm instance can unlock the encrypted volume.

## Overview

OpenClaw-CCA protects all of OpenClaw's persistent data at rest by encrypting it under a LUKS2 volume whose passphrase is never stored on disk. Instead, the passphrase is released by [RBS](https://gitcode.com/openeuler/globaltrustauthority-rbs) (Resource Broker Service) only after the Realm successfully completes remote attestation via [GTA](https://gitcode.com/openeuler/global-trust-authority) (Global Trust Authority).

The encrypted volume holds:

- **Configuration** (`openclaw.json`): API keys, LLM endpoints, tool/skill configuration
- **Long-term memory**: conversation history, user profiles, knowledge base & vector embeddings
- **Session state**: persistent session snapshots, cached intermediate results
- **Runtime metadata**: logs, audit records, skill workspaces

## Trust Chain

```
Realm boot
  └─ extend-rem3 measures critical components (losetup / cryptsetup / rbc-cli / openclaw / shadow hash)
       └─ rbc-cli collect-evidence gathers TEE evidence containing REM3
            └─ RBS verifies evidence against policy
                 └─ Releases passphrase → cryptsetup luksOpen → openclaw starts
```

## Environment

| Item | Description |
|------|-------------|
| OS | openEuler 24.03-SP4 |
| Hardware | Kunpeng 950 |
| TEE | ARM CCA Realm (also supports vCCA) |

## Dependencies

| Project | Role |
|---------|------|
| [GTA](https://gitcode.com/openeuler/global-trust-authority) @ `3f58d58` | Hardware-backed remote attestation — verifies node integrity via cryptographic evidence |
| [RBS](https://gitcode.com/openeuler/globaltrustauthority-rbs) @ `8e3d7f7` | Policy-driven resource distribution — releases secrets only to attested workloads |

## Directory Structure

```
openclaw-cca/
├── src/
│   └── extend_tools/
│       └── attest.c                  # extend-rem3: extend REM3 register via ioctl
├── Makefile                          # Build & install
├── docs/
│   ├── usage_guide.md                # Full deployment guide (Chinese)
│   └── images/demo.png               # Trust chain diagram
├── scripts/
│   ├── openclaw-init.sh              # First-time initialization
│   ├── openclaw-create-volume.sh     # Create LUKS2 encrypted volume
│   ├── openclaw-rbc-unlock.sh        # Unlock volume via RBC attestation
│   ├── gen_policy.py                 # Generate OPA/Rego policy from JWT baseline
│   └── policy_template/
│       ├── cca.rego                  # CCA attestation policy template
│       └── vcca.rego                 # vCCA attestation policy template
├── skills/
│   ├── openclaw-cca-attest/          # Agent skill: CCA attestation workflow
│   └── openclaw-vcca-attest/         # Agent skill: vCCA attestation workflow
└── systemd/
    ├── openclaw-luks-unlock.service  # Auto-unlock encrypted volume at boot
    └── openclaw.service              # OpenClaw gateway service
```

## Quick Start

### Prerequisites

- ARM CCA Realm (or vCCA)
- `rbc-cli` installed at `/usr/bin/rbc-cli`
- `cryptsetup`, `make`, `gcc`, `openssl`, `jq`
- RBS service reachable from the Realm
- OpenClaw binary installed

### Build & Install

```bash
git clone https://gitcode.com/openeuler/sec-select.git
cd solution/openclaw-cca

# Replace with your actual RBS URL
sed -i 's|YOUR_RBS_URL_HERE|<your_rbs_url>|' scripts/openclaw-rbc-unlock.sh

make
sudo make install
```

Installed artifacts:

| Path | Description |
|------|-------------|
| `/usr/local/bin/extend-rem3` | Extend REM3 register with component hashes |
| `/usr/local/sbin/openclaw-rbc-unlock.sh` | RBC attestation & volume unlock |
| `/usr/local/sbin/openclaw-init.sh` | First-time initialization |
| `/usr/local/sbin/openclaw-create-volume.sh` | Create encrypted LUKS2 volume |
| `/etc/systemd/system/openclaw-luks-unlock.service` | Boot-time unlock service |

### Deployment

For the complete step-by-step deployment guide (including policy generation, volume creation, and systemd configuration), see [docs/usage_guide.md](docs/usage_guide.md).

High-level flow:

1. **Initialize**: `sudo openclaw-init.sh` — measures components into REM3, generates baseline JWT
2. **Generate policy**: `python3 scripts/gen_policy.py /tmp/baseline_jwt.txt` — produces an OPA/Rego policy, upload to RBS
3. **Register secret**: Generate a random passphrase, upload to RBS with the policy, record the returned `key_uri`
4. **Create volume**: `sudo openclaw-create-volume.sh <key_uri>` — creates LUKS2 encrypted volume
5. **Configure systemd**: Fill in `KEY_URI`/`DEVICE`/`MOUNT_POINT` in the unlock service
6. **Write API keys**: Edit `/opt/openclaw-data/openclaw.json`
7. **Enable & reboot**: `systemctl enable openclaw-luks-unlock.service && reboot`

## How It Works

### REM3 Measurement

`extend-rem3` (built from `attest.c`) talks to the `/dev/attest` ioctl interface to extend REM3 — an accumulator register in the CCA Realm that can only be extended, never reset (except at Realm reboot). The following components are measured in order:

1. `/usr/local/bin/extend-rem3`
2. `/sbin/losetup`
3. `/usr/sbin/cryptsetup`
4. `/usr/bin/rbc-cli`
5. OpenClaw binary
6. Current user's `/etc/shadow` password hash (binds to the VM instance identity)

### Attestation Flow

1. An ephemeral RSA-4096 key pair is generated
2. `rbc-cli challenge` fetches a nonce from RBS
3. `rbc-cli collect-evidence` gathers TEE evidence (containing REM/RIM values) signed with the attester key
4. `rbc-cli get-resource` submits evidence to RBS; RBS evaluates the OPA/Rego policy against the evidence
5. If the policy matches (all REM/RIM values equal the predefined baseline), RBS releases the LUKS passphrase
6. `cryptsetup luksOpen` unlocks the volume using the passphrase

### CCA vs vCCA

The solution supports both hardware CCA and virtual CCA (vCCA):

| Aspect | CCA | vCCA |
|--------|-----|------|
| Policy template | `cca.rego` | `vcca.rego` |
| JWT field path | `cca.realm_token` | `virt_cca.realm_token` |
| Measurement keys | `cca_rpv`, `cca_rim`, `cca_rem[0-3]` | `vcca_rpv`, `vcca_rim`, `vcca_rem[0-3]` |
| Kernel module | `arm_cca_guest` required | Not required |

### Re-initialization After Updates

Any update to measured components (particularly the OpenClaw binary) changes REM3 values, invalidating the RBS policy. After updating, you must re-run the full initialization flow to generate a new baseline and policy.

## Fault Recovery

If the unlock service fails at boot (e.g., RBS was unreachable), simply restart the service once RBS is available — no need to reboot the Realm:

```bash
systemctl start openclaw-luks-unlock.service
```

The script uses a tmpfs flag (`/run/openclaw-rem3-extended`) to avoid re-extending REM3 on retry, ensuring the measurement stays consistent with the RBS policy.

## License

[Mulan Permissive Software License, Version 2 (MulanPSL-2.0)](http://license.coscl.org.cn/MulanPSL2)
