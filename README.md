# rsRRC

Shared Rust implementation of the Reticulum Relay Chat (RRC) version 1 wire
protocol.

This crate owns protocol constants, integer-key CBOR envelopes, validation,
capabilities, and common message builders. It intentionally contains no
Reticulum transport, server routing, persistence, or user interface code.

It is used by the adjacent `rsRRCD` hub daemon and `rsNomadNet` client.

The crate also defines an optional `K_ROOM_STATE` extension. Hubs can attach
the registered flag, mode string, and topic to JOINED or NOTICE envelopes
without changing their standard RRC v1 body. Clients that do not understand
the extension can safely ignore the additional integer-key field.
