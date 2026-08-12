# ratls-rust

[中文](README.md)

ratls-rust is a Rust implementation of RATS-TLS, currently focused on CCA. It
places attestation evidence in the TLS certificate so that both peers can check
the other's identity and runtime environment while establishing TLS. The project
provides a Rust API, a C shared library, and runnable client/server samples.

## Layout

```text
ratls-rust/
├── ratls-api/                     # reusable RATS-TLS core library
│   ├── include/
│   │   └── ratls_api.h            # public C header for the shared library
│   └── src/
│       ├── api/                   # Rust API: init, negotiate, transmit, receive
│       ├── attesters/cca/         # CCA evidence collection through Linux TSM
│       ├── verifiers/cca/         # CCA token, certificate-chain, and claims checks
│       ├── core/                  # evidence, certificates, and DICE/CBOR claims
│       ├── crypto_wrappers/       # OpenSSL crypto wrapper
│       ├── tls_wrappers/          # OpenSSL TLS wrapper
│       └── ffi.rs                 # exported C ABI
│
├── ratls-sample/                  # runnable sample programs
│   └── src/
│       ├── bin/
│       │   ├── cca-client.rs      # sample client
│       │   └── cca-server.rs      # sample server
│       └── common/                # framing, IMA, CCEL, event logs, and policy checks
│
├── Cargo.toml                     # Rust workspace configuration
└── README.md                      # Chinese documentation
```

The main outputs are:

```text
target/debug/libratls_api.so
target/debug/cca-client
target/debug/cca-server
```

## Build and commands

Requirements:

- Rust 1.75+
- OpenSSL 3.5+
- `gcc` or `clang`
- `pkg-config`

Build from the repository root:

```sh
cargo build
cargo build --release
```

Release artifacts:

```sh
target/release/libratls_api.so
target/release/cca-client
target/release/cca-server
```

The current sample command reference follows.

### cca-server

```text
Usage: cca-server [OPTIONS]

Options:
  -i, --ip <IP>                  [default: 0.0.0.0]
  -p, --port <PORT>              [default: 1234]
  -1, --once
  -m, --mutual
      --ima-log <IMA_LOG>
                                  [default: /sys/kernel/security/ima/binary_runtime_measurements]
      --ccel-table <CCEL_TABLE>  [default: /sys/firmware/acpi/tables/CCEL]
      --event-log <EVENT_LOG>
                                  [default: /sys/firmware/acpi/tables/data/CCEL]
      --rootfs-key <ROOTFS_KEY>  [default: /root/rootfs_key.bin]
      --rim <RIM>
  -P, --platform <PLATFORM>
      --max-key <MAX_KEY>        [default: 65536]
      --cert-algo <CERT_ALGO>    [default: ecc256] [possible values: rsa3072, ecc256]
      --issuer-private-key <PEM_FILE>
      --issuer-certificate-chain <PEM_FILE>
      --subject-alt-name <TYPE:VALUE>
      --verify-peer-certificate
      --use-system-ca
      --trusted-ca-chain <PEM_FILE>
      --expected-peer-name <DNS_OR_IP>
  -l, --log-level <LOG_LEVEL>    [default: error]
                                  [debug, info, warn, error, fatal, none]
  -h, --help
```

### cca-client

```text
Usage: cca-client [OPTIONS]

Options:
  -i, --ip <IP>                  [default: 127.0.0.1]
  -p, --port <PORT>              [default: 1234]
  -M, --message <MESSAGE>        [default: "hello CCA"]
      --message-file <MESSAGE_FILE>
  -m, --mutual
  -I, --ima-log
  -g, --bootlog
  -f, --firmware <FIRMWARE>
  -d, --digest <DIGEST>
      --rim <RIM>
  -P, --platform <PLATFORM>
  -k, --fdekey <FDE_KEY>
      --max-log <MAX_LOG>        [default: 10485760]
      --cert-algo <CERT_ALGO>    [default: ecc256] [possible values: rsa3072, ecc256]
      --issuer-private-key <PEM_FILE>
      --issuer-certificate-chain <PEM_FILE>
      --subject-alt-name <TYPE:VALUE>
      --verify-peer-certificate
      --use-system-ca
      --trusted-ca-chain <PEM_FILE>
      --expected-peer-name <DNS_OR_IP>
  -l, --log-level <LOG_LEVEL>    [default: error]
                                  [debug, info, warn, error, fatal, none]
  -h, --help
```

`--firmware` requires `--bootlog`, and `--digest` requires `--ima-log`.
Server-side `--rim` and `--platform` verify client evidence and therefore
require `--mutual`.

## Sample operations

The sample first makes an ordinary TCP connection, then negotiates RATS-TLS on
top of it. The server always collects local CCA evidence. The client verifies
the server by default. With `--mutual`, both sides collect and verify evidence.

The server needs an environment that can collect CCA evidence through the Linux
TSM report interface. Start it normally or handle one connection and exit:

```sh
cargo run -p ratls-sample --bin cca-server
cargo run -p ratls-sample --bin cca-server -- --once
```

Set a listen address and port when needed:

```sh
cargo run -p ratls-sample --bin cca-server -- \
  --ip 0.0.0.0 --port 1234 --once
```

Then run the client:

```sh
cargo run -p ratls-sample --bin cca-client -- --ip <server-ip> --port 1234 --message 'hello CCA' --log-level debug
```

For mutual attestation, the client needs a TSM interface as well:

```sh
# server
cargo run -p ratls-sample --bin cca-server -- --once --mutual

# client
cargo run -p ratls-sample --bin cca-client -- --ip <server-ip> --mutual
```

The sample exposes local CA issuance and peer verification through
`--issuer-private-key`, `--issuer-certificate-chain`, repeatable
`--subject-alt-name`, `--verify-peer-certificate`, `--use-system-ca`,
`--trusted-ca-chain`, and `--expected-peer-name`. For example:

```sh
# server: dynamically sign its RA-TLS leaf with a test CA
cargo run -p ratls-sample --bin cca-server -- \
  --once \
  --issuer-private-key /tmp/ratls-test-ca.key \
  --issuer-certificate-chain /tmp/ratls-test-ca.crt \
  --subject-alt-name DNS:server.test

# client: trust that CA and validate the DNS SAN
cargo run -p ratls-sample --bin cca-client -- \
  --ip <server-ip> \
  --verify-peer-certificate \
  --trusted-ca-chain /tmp/ratls-test-ca.crt \
  --expected-peer-name server.test
```

Without CCA/TSM, you can still build the project, run tests, and check the C
ABI. You cannot perform a real end-to-end CCA attestation exchange.

### Optional checks and transfers

The server defaults to these paths:

- IMA binary log: `/sys/kernel/security/ima/binary_runtime_measurements`
- CCEL table: `/sys/firmware/acpi/tables/CCEL`
- CCA event log: `/sys/firmware/acpi/tables/data/CCEL`
- received rootfs key: `/root/rootfs_key.bin`

Override them as required:

```sh
cargo run -p ratls-sample --bin cca-server -- \
  --once \
  --ima-log <ima-log-path> \
  --ccel-table <ccel-table-path> \
  --event-log <event-log-path> \
  --rootfs-key <output-key-path>
```

Some useful client commands:

```sh
# Request and verify the IMA log
cargo run -p ratls-sample --bin cca-client -- --ip <server-ip> --ima-log --digest <baseline.json>

# Request the boot log; --firmware requires --bootlog
cargo run -p ratls-sample --bin cca-client -- --ip <server-ip> --bootlog --firmware <firmware-baseline.json>

# Check Realm Initial Measurement and the platform policy
cargo run -p ratls-sample --bin cca-client -- --ip <server-ip> --rim <hex-rim> --platform <platform-policy.json>

# Send a rootfs key through the negotiated TLS channel
cargo run -p ratls-sample --bin cca-client -- --ip <server-ip> --fdekey <key-file>
```

With `--bootlog`, the client replays event-log registries 1 and 2 and compares
the results with the cryptographically verified `cca_realm_rem0` and
`cca_realm_rem1` evidence claims. Firmware state extraction and `--firmware`
baseline verification run only after that comparison succeeds.

For all flags and defaults:

```sh
cargo run -p ratls-sample --bin cca-server -- --help
cargo run -p ratls-sample --bin cca-client -- --help
```

## Use the C shared library

The public header is `ratls-api/include/ratls_api.h`. Keep it paired with the
`libratls_api.so` from the same build: `ratls_conf_t` and callback evidence
structures have a fixed ABI layout.

```sh
cargo build --release -p ratls-api
cc -std=c11 -Wall -Wextra app.c \
  -Iratls-api/include -Ltarget/release -lratls_api \
  -o app
```

Make sure the dynamic loader can find `libratls_api.so` at runtime:

```sh
export LD_LIBRARY_PATH=target/release:$LD_LIBRARY_PATH
./app
```

Pass `ratls_negotiate_fd` a connected or accepted TCP socket. It duplicates the
descriptor, so the caller still owns and closes the original. `ratls_transmit`
and `ratls_receive` are a TLS byte stream, not a message protocol; add framing
in your application.

```c
#include "ratls_api.h"

ratls_conf_t conf;
ratls_conf_init(&conf);

/* One-way client: verify the server without producing local evidence. */
conf.server = 0;
conf.attester_type = RATLS_ATTESTER_NONE;
conf.verifier_type = RATLS_VERIFIER_CCA;
```

By default the library generates a leaf key and dynamically self-signs the
RA-TLS certificate. To have a CA sign the dynamic certificate, supply an
unencrypted PEM CA key and its PEM issuer chain:

```c
conf.certificate.issuer_private_key =
    (ratls_buffer_t){issuer_key_pem, issuer_key_pem_len};
conf.certificate.issuer_certificate_chain =
    (ratls_buffer_t){issuer_chain_pem, issuer_chain_pem_len};
```

Both fields must be present or both must be empty. Enable standard TLS
certificate verification independently:

```c
conf.tls_verify.verify_peer_certificate = 1;
conf.tls_verify.use_system_ca = 1;
conf.tls_verify.trusted_ca_chain =
    (ratls_buffer_t){private_ca_pem, private_ca_pem_len};
conf.tls_verify.expected_peer_name = "server.example.com"; /* optional */
```

System roots and the custom CA bundle are combined. If system roots are
disabled, a non-empty custom bundle is required. A server can enable peer TLS
certificate verification only with mutual TLS, in which case OpenSSL validates
the client certificate before the existing RA verification runs.
When peer trust verification is disabled, CA trust anchors and peer-name
checking are skipped, but certificate signatures, validity, constraints, key
usage, role EKU, and the existing RA evidence bindings are still verified.
This feature does not enable CRL or OCSP revocation checks. Loading the system
CA paths only adds system trust anchors and does not imply automatic revocation
checking.

Both `rats_tls_init()` and the C `ratls_init()` validate input limits before
use. The main limits are 64 KiB for the issuer key, 1 MiB for the issuer chain,
4 MiB for the custom trust bundle, 64 SAN entries, 64 custom claims, 64 KiB per
claim value, and 256 KiB for all claim values. Corresponding `RATLS_MAX_*`
macros are available in the C header. Invalid leaf algorithms, reserved or
duplicate claim names, malformed SANs and peer DNS names, and oversized inputs
are rejected during initialization.

Use `ratls_conf_init` rather than relying on `{0}`: certificate algorithm zero
selects RSA-3072 rather than the default. The usual sequence is
`ratls_conf_init`, `ratls_init`, optional `ratls_set_log_level` and
`ratls_set_verification_callback`, `ratls_negotiate_fd`, any number of
`ratls_transmit`/`ratls_receive` calls, and finally `ratls_cleanup`.

Set `mutual` to a non-zero value for mutual attestation. Certificate algorithms:

- `RATLS_CERT_ALGO_RSA_3072_SHA256` (0)
- `RATLS_CERT_ALGO_ECC_256_SHA256` (1)
- `RATLS_CERT_ALGO_DEFAULT` (3; ECC P-256/SHA-256)

Zero-initialising `cert_algo` selects RSA; it does not select the default.
`RATLS_CERT_ALGO_MAX` (2) is a sentinel and is invalid.

After an error, call `ratls_last_error()` immediately from the same thread. Its
pointer becomes invalid after the next FFI call that updates error state. The
log level is process-wide. A verification callback accepts with a non-zero
return value and rejects with zero. Its evidence, token, JSON, and custom-claim
buffers are borrowed for the duration of that callback only.

Complete, compilable C programs are available in
`ratls-api/examples/c/ratls_client.c` and
`ratls-api/examples/c/ratls_server.c`. The server requires an environment with
the Linux TSM report interface so it can collect real CCA evidence.

## Extending the project

Start by identifying where a new security scenario belongs:

```text
new TEE / evidence format
    ├── attesters/       # collect local evidence
    ├── verifiers/       # verify peer evidence
    └── core/            # evidence, claims, and certificate-extension types

new TLS or crypto implementation
    ├── tls_wrappers/    # TLS negotiation and certificate loading
    └── crypto_wrappers/ # keys, certificates, signatures, and hashes

new application policy
    ├── ratls_set_verification_callback
    └── ratls-sample/    # sample protocol, logs, and policy code
```

### Add an attester and verifier

CCA support lives in:

```text
ratls-api/src/attesters/cca/
ratls-api/src/verifiers/cca/
```

For a new TEE or evidence format, add equivalent modules under
`attesters/<type>/` and `verifiers/<type>/`. The attester collects evidence and
places it in the RA-TLS certificate extension. The verifier extracts the peer
evidence and validates its certificate chain, signature, nonce, and claims.

Register both implementations in their registries so applications can select
them:

```c
ratls_conf_t conf = {
    .attester_type = RATLS_ATTESTER_CCA,
    .verifier_type = RATLS_VERIFIER_CCA,
    .tls_type = "openssl",
    .crypto_type = "openssl",
};
```

### Custom claims and application policy

If the goal is to bind application information to evidence, a new attester is
not necessary. Use `ratls_conf_t.custom_claims` for a workload name, image
digest, configuration hash, or service version. Do not put private keys, tokens,
or rootfs-key material into custom claims.

The verifier checks that evidence is valid; the application decides whether
valid evidence satisfies its own policy. Put that decision in a verification
callback:

```c
int verify_callback(const ratls_verified_evidence_t *evidence, void *user_data) {
    /* Check claims_json or custom_claims. */
    /* Non-zero accepts the peer; zero rejects it. */
    return 1;
}
```

The IMA, RIM, platform-policy, and firmware-baseline checks in `ratls-sample/`
are examples of application policy.

### Add a TLS or crypto wrapper

To replace OpenSSL, implement and register modules under:

```text
ratls-api/src/tls_wrappers/<type>/
ratls-api/src/crypto_wrappers/<type>/
```

Then select them through `.tls_type` and `.crypto_type`. Keep the existing
boundary: the TLS wrapper handles negotiation and certificate loading; the
crypto wrapper handles keys, certificates, signatures, and hashes.

After development, at least run:

```sh
cargo fmt
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```

If the C ABI changes, update the following files as well:

```text
ratls-api/src/ffi.rs
ratls-api/include/ratls_api.h
README.md
README.en.md
```
