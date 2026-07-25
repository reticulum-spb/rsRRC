# rsRRC

Shared Rust implementation of the Reticulum Relay Chat (RRC) version 1 wire
protocol.

This crate owns protocol constants, integer-key CBOR envelopes, validation,
capabilities, and common message builders. It intentionally contains no
Reticulum transport, server routing, persistence, or user interface code.

It is used by the adjacent `rsRRCD` hub daemon and `rsNomadNet` client.

The crate also defines optional structured `K_ROOM_STATE` and `K_USER_LIST`
extensions. Hubs can attach room metadata to JOINED or NOTICE envelopes and
member metadata to WHO replies without changing their standard RRC v1 bodies.
`CAP_ROOM_STATE` and `CAP_USER_LIST` advertise support in HELLO and WELCOME;
older peers can safely ignore the additional integer-key fields.
