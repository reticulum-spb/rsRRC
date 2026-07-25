# RRC v1 optional extensions

These extensions preserve the RRC v1 integer-key CBOR envelope. A peer must
ignore unknown capability IDs, envelope keys, and map keys.

Capabilities are boolean entries in the map at body key `2` in HELLO and
WELCOME:

| Capability | ID | Meaning |
| --- | ---: | --- |
| Resource envelope | `0` | Native Resource payloads announced by message type `50` |
| Action | `1` | Action messages using type `22` |
| Direct notice | `2` | NOTICE with a destination identity in envelope key `8` |
| Room state | `3` | Structured room state in envelope key `9` |
| User list | `4` | Structured WHO result in envelope key `10` |

Absence and an explicit `false` have the same meaning. Room-state and user-list
fields are included only for a receiving peer that advertised them. Native
Resource delivery is likewise selected per recipient. ACTION and direct
NOTICE initiation requires the hub to advertise support in WELCOME. Standard
textual fallbacks remain present when structured fields are used.

## Room state

`K_ROOM_STATE = 9` contains a CBOR map describing the room named by
`K_ROOM = 5`. It can accompany JOINED and room-related NOTICE envelopes.

| Map key | CBOR type | Meaning |
| --- | --- | --- |
| `0` | boolean | Room is registered |
| `1` | text | Current mode string, or `(none)` |
| `2` | text, optional | Topic; absence means no topic |

The extension is advertised as `CAP_ROOM_STATE = 3`.

## User list

`K_USER_LIST = 10` contains an array of maps and accompanies the textual NOTICE
reply to WHO. Each map describes one current room member:

| Map key | CBOR type | Meaning |
| --- | --- | --- |
| `0` | text | Full 16-byte identity hash encoded as 32 lowercase hex characters |
| `1` | text, optional | Nickname |
| `2` | boolean | Room operator |
| `3` | boolean | Voiced member |

The extension is advertised as `CAP_USER_LIST = 4`. Clients must continue to
accept the textual WHO result when the capability or field is absent.

## Compatibility rules

- Do not reinterpret an unknown integer key.
- Do not reject an otherwise valid envelope because it has extra keys.
- Do not require optional fields after reconnect until WELCOME is received.
- A client may parse textual LIST and WHO responses regardless of advertised
  capabilities.
- A hub should tailor optional fields and Resource delivery to each recipient,
  since modern and legacy clients can share a room.
