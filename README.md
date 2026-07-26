# rsRRC

Shared Rust implementation of the Reticulum Relay Chat (RRC) version 1 wire
protocol.

This crate owns protocol constants, integer-key CBOR envelopes, validation,
capabilities, and common message builders. It intentionally contains no
Reticulum transport, server routing, persistence, or user interface code.

It is used directly by the adjacent `rsRRCD` hub daemon and `rsRRC-client`
library. `rsNomadNet` consumes the protocol through `rsRRC-client`, keeping
transport and reconnect behavior out of its web application layer.

The crate also defines optional structured `K_ROOM_STATE` and `K_USER_LIST`
extensions. Hubs can attach room metadata to JOINED or NOTICE envelopes and
member metadata to WHO replies without changing their standard RRC v1 bodies.
`CAP_ROOM_STATE` and `CAP_USER_LIST` advertise support in HELLO and WELCOME;
older peers can safely ignore the additional integer-key fields.

The exact capability and CBOR field layout is documented in
[`EXTENSIONS.md`](EXTENSIONS.md).

## Ecosystem role

`rsRRC` is the transport-independent layer of the Rust RRC stack:

```text
rsRRCD ───────┐
              ├── rsRRC
rsNomadNet ─ rsRRC-client ─┘
```

This boundary allows bots and other clients to reuse the same validated
envelopes without depending on rsNomadNet or duplicating protocol constants.
