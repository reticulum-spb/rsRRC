use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rand::RngCore;
use serde_cbor::Value;
use sha2::{Digest, Sha256};

use crate::constants::*;

pub type Map = BTreeMap<Value, Value>;

#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    pub fields: Map,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceDescriptor {
    pub id: Vec<u8>,
    pub kind: String,
    pub size: usize,
    pub sha256: Option<[u8; 32]>,
    pub encoding: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Capabilities {
    pub resource_envelope: bool,
    pub action: bool,
    pub direct_notice: bool,
    pub room_state: bool,
    pub user_list: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Limits {
    pub max_nick_bytes: Option<usize>,
    pub max_room_name_bytes: Option<usize>,
    pub max_message_bytes: Option<usize>,
    pub max_rooms_per_session: Option<usize>,
    pub messages_per_minute: Option<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Welcome {
    pub hub_name: Option<String>,
    pub version: Option<String>,
    pub capabilities: Capabilities,
    pub limits: Limits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomInfo {
    pub name: String,
    pub topic: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RoomState {
    pub registered: bool,
    pub modes: String,
    pub topic: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserInfo {
    pub nick: Option<String>,
    pub identity: String,
    pub operator: bool,
    pub voiced: bool,
}

static WHO_ENTRY: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)(?:^|,\s)(?:(?P<hash>[0-9a-f]{32})|(?P<nick>.+?)\s\((?P<prefix>[0-9a-f]{12})\))",
    )
    .expect("valid WHO response regex")
});

fn key(value: i128) -> Value {
    Value::Integer(value)
}

impl Envelope {
    pub fn new(message_type: u64, source: &[u8]) -> Self {
        let mut id = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut id);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i128;
        let mut fields = Map::new();
        fields.insert(key(K_V), Value::Integer(VERSION as i128));
        fields.insert(key(K_T), Value::Integer(message_type as i128));
        fields.insert(key(K_ID), Value::Bytes(id.to_vec()));
        fields.insert(key(K_TS), Value::Integer(timestamp));
        fields.insert(key(K_SRC), Value::Bytes(source.to_vec()));
        Self { fields }
    }

    pub fn hello(source: &[u8; 16], nick: Option<&str>) -> Self {
        let mut envelope = Self::new(T_HELLO, source);
        let mut capabilities = Map::new();
        capabilities.insert(Value::Integer(CAP_RESOURCE_ENVELOPE), Value::Bool(true));
        capabilities.insert(Value::Integer(CAP_ACTION), Value::Bool(true));
        capabilities.insert(Value::Integer(CAP_DIRECT_NOTICE), Value::Bool(true));
        capabilities.insert(Value::Integer(CAP_ROOM_STATE), Value::Bool(true));
        capabilities.insert(Value::Integer(CAP_USER_LIST), Value::Bool(true));
        let mut body = Map::new();
        body.insert(Value::Integer(B_HELLO_CAPS), Value::Map(capabilities));
        envelope.set(K_BODY, Value::Map(body));
        if let Some(nick) = nick.filter(|value| valid_text(value, 32)) {
            envelope.set(K_NICK, Value::Text(nick.to_string()));
        }
        envelope
    }

    pub fn join(source: &[u8; 16], room: &str, key_value: Option<&str>) -> Option<Self> {
        let room = normalize_room(room, 64)?;
        let mut envelope = Self::new(T_JOIN, source);
        envelope.set(K_ROOM, Value::Text(room));
        if let Some(key_value) = key_value.filter(|value| valid_text(value, 256)) {
            envelope.set(K_BODY, Value::Text(key_value.to_string()));
        }
        Some(envelope)
    }

    pub fn part(source: &[u8; 16], room: &str) -> Option<Self> {
        let room = normalize_room(room, 64)?;
        let mut envelope = Self::new(T_PART, source);
        envelope.set(K_ROOM, Value::Text(room));
        Some(envelope)
    }

    pub fn message(source: &[u8; 16], room: &str, text: &str, action: bool) -> Option<Self> {
        let room = normalize_room(room, 64)?;
        valid_text(text, 16 * 1024).then_some(())?;
        let mut envelope = Self::new(if action { T_ACTION } else { T_MSG }, source);
        envelope.set(K_ROOM, Value::Text(room));
        envelope.set(K_BODY, Value::Text(text.to_string()));
        Some(envelope)
    }

    pub fn command(source: &[u8; 16], room: Option<&str>, command: &str) -> Option<Self> {
        if !command.starts_with('/') || !valid_text(command, 16 * 1024) {
            return None;
        }
        let room = match room {
            Some(value) => Some(normalize_room(value, 64)?),
            None => None,
        };
        let mut envelope = Self::new(T_MSG, source);
        if let Some(room) = room {
            envelope.set(K_ROOM, Value::Text(room));
        }
        envelope.set(K_BODY, Value::Text(command.to_string()));
        Some(envelope)
    }

    pub fn pong(source: &[u8; 16], ping: &Self) -> Self {
        let mut envelope = Self::new(T_PONG, source);
        if let Some(body) = ping.get(K_BODY) {
            envelope.set(K_BODY, body.clone());
        }
        envelope
    }

    pub fn ping(source: &[u8; 16], nonce: u64) -> Self {
        let mut envelope = Self::new(T_PING, source);
        envelope.set(K_BODY, Value::Integer(nonce.into()));
        envelope
    }

    pub fn resource(
        source: &[u8; 16],
        room: Option<&str>,
        kind: &str,
        data: &[u8],
        encoding: Option<&str>,
    ) -> Option<Self> {
        if !valid_text(kind, 32)
            || encoding.is_some_and(|value| !valid_text(value, 32))
            || data.is_empty()
        {
            return None;
        }
        let room = match room {
            Some(value) => Some(normalize_room(value, 64)?),
            None => None,
        };
        let mut resource_id = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut resource_id);
        let mut body = Map::new();
        body.insert(Value::Integer(B_RES_ID), Value::Bytes(resource_id.to_vec()));
        body.insert(Value::Integer(B_RES_KIND), Value::Text(kind.to_string()));
        body.insert(
            Value::Integer(B_RES_SIZE),
            Value::Integer(data.len() as i128),
        );
        body.insert(
            Value::Integer(B_RES_SHA256),
            Value::Bytes(Sha256::digest(data).to_vec()),
        );
        if let Some(encoding) = encoding {
            body.insert(
                Value::Integer(B_RES_ENCODING),
                Value::Text(encoding.to_string()),
            );
        }
        let mut envelope = Self::new(T_RESOURCE_ENVELOPE, source);
        if let Some(room) = room {
            envelope.set(K_ROOM, Value::Text(room));
        }
        envelope.set(K_BODY, Value::Map(body));
        Some(envelope)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value: Value = serde_cbor::from_slice(bytes).context("invalid CBOR")?;
        let Value::Map(fields) = value else {
            bail!("envelope must be a CBOR map");
        };
        let envelope = Self { fields };
        envelope.validate()?;
        Ok(envelope)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        serde_cbor::to_vec(&Value::Map(self.fields.clone())).context("CBOR encoding failed")
    }

    pub fn validate(&self) -> Result<()> {
        for field in self.fields.keys() {
            if !matches!(field, Value::Integer(value) if *value >= 0) {
                bail!("envelope keys must be unsigned integers");
            }
        }
        for required in [K_V, K_T, K_ID, K_TS, K_SRC] {
            if !self.fields.contains_key(&key(required)) {
                bail!("missing envelope key {required}");
            }
        }
        if self.integer(K_V) != Some(VERSION) {
            bail!("unsupported protocol version");
        }
        if self.integer(K_T).is_none() {
            bail!("message type must be an unsigned integer");
        }
        if self.bytes(K_ID).is_none() {
            bail!("message id must be bytes");
        }
        if self.unsigned(K_TS).is_none() {
            bail!("timestamp must be unsigned");
        }
        if self.bytes(K_SRC).is_none() {
            bail!("sender identity must be bytes");
        }
        if self
            .get(K_ROOM)
            .is_some_and(|value| !matches!(value, Value::Text(_)))
        {
            bail!("room name must be a string");
        }
        if self
            .get(K_NICK)
            .is_some_and(|value| !matches!(value, Value::Text(_)))
        {
            bail!("nickname must be a string");
        }
        if self
            .get(K_DST)
            .is_some_and(|value| !matches!(value, Value::Bytes(_)))
        {
            bail!("destination identity must be bytes");
        }
        Ok(())
    }

    pub fn get(&self, field: i128) -> Option<&Value> {
        self.fields.get(&key(field))
    }

    pub fn integer(&self, field: i128) -> Option<u64> {
        self.unsigned(field).and_then(|value| value.try_into().ok())
    }

    pub fn message_type(&self) -> Option<u64> {
        self.integer(K_T)
    }

    pub fn timestamp_ms(&self) -> Option<u64> {
        self.integer(K_TS)
    }

    pub fn source(&self) -> Option<[u8; 16]> {
        self.bytes(K_SRC)?.try_into().ok()
    }

    pub fn set_source(&mut self, source: &[u8; 16]) {
        self.set(K_SRC, Value::Bytes(source.to_vec()));
    }

    pub fn room(&self) -> Option<&str> {
        self.text(K_ROOM)
    }

    pub fn nick(&self) -> Option<&str> {
        self.text(K_NICK)
    }

    pub fn set_nick(&mut self, nick: &str) {
        self.set(K_NICK, Value::Text(nick.to_string()));
    }

    pub fn body_text(&self) -> Option<&str> {
        self.text(K_BODY)
    }

    pub fn set_room_state(&mut self, state: &RoomState) {
        let mut body = Map::new();
        body.insert(
            Value::Integer(B_ROOM_REGISTERED),
            Value::Bool(state.registered),
        );
        body.insert(
            Value::Integer(B_ROOM_MODES),
            Value::Text(state.modes.clone()),
        );
        if let Some(topic) = &state.topic {
            body.insert(Value::Integer(B_ROOM_TOPIC), Value::Text(topic.to_string()));
        }
        self.set(K_ROOM_STATE, Value::Map(body));
    }

    pub fn room_state(&self) -> Option<RoomState> {
        let body = self.map(K_ROOM_STATE)?;
        let registered = match map_get(body, B_ROOM_REGISTERED) {
            Some(Value::Bool(value)) => *value,
            _ => false,
        };
        let modes = map_text(body, B_ROOM_MODES).unwrap_or("(none)").to_string();
        let topic = map_text(body, B_ROOM_TOPIC).map(str::to_string);
        Some(RoomState {
            registered,
            modes,
            topic,
        })
    }

    pub fn set_user_list(&mut self, users: &[UserInfo]) {
        let users = users
            .iter()
            .map(|user| {
                let mut entry = Map::new();
                entry.insert(
                    Value::Integer(B_USER_IDENTITY),
                    Value::Text(user.identity.clone()),
                );
                if let Some(nick) = &user.nick {
                    entry.insert(Value::Integer(B_USER_NICK), Value::Text(nick.clone()));
                }
                entry.insert(Value::Integer(B_USER_OPERATOR), Value::Bool(user.operator));
                entry.insert(Value::Integer(B_USER_VOICED), Value::Bool(user.voiced));
                Value::Map(entry)
            })
            .collect();
        self.set(K_USER_LIST, Value::Array(users));
    }

    pub fn user_list(&self) -> Option<Vec<UserInfo>> {
        let Value::Array(users) = self.get(K_USER_LIST)? else {
            return None;
        };
        Some(
            users
                .iter()
                .filter_map(|value| {
                    let Value::Map(entry) = value else {
                        return None;
                    };
                    Some(UserInfo {
                        nick: map_text(entry, B_USER_NICK).map(str::to_string),
                        identity: map_text(entry, B_USER_IDENTITY)?.to_ascii_lowercase(),
                        operator: map_bool(entry, B_USER_OPERATOR),
                        voiced: map_bool(entry, B_USER_VOICED),
                    })
                })
                .collect(),
        )
    }

    pub fn welcome_hub_name(&self) -> Option<&str> {
        match map_get(self.map(K_BODY)?, B_WELCOME_HUB)? {
            Value::Text(value) => Some(value),
            _ => None,
        }
    }

    pub fn welcome(&self) -> Option<Welcome> {
        if self.message_type() != Some(T_WELCOME) {
            return None;
        }
        let body = self.map(K_BODY)?;
        let capabilities = match map_get(body, B_WELCOME_CAPS) {
            Some(Value::Map(values)) => Capabilities {
                resource_envelope: map_bool(values, CAP_RESOURCE_ENVELOPE),
                action: map_bool(values, CAP_ACTION),
                direct_notice: map_bool(values, CAP_DIRECT_NOTICE),
                room_state: map_bool(values, CAP_ROOM_STATE),
                user_list: map_bool(values, CAP_USER_LIST),
            },
            _ => Capabilities::default(),
        };
        let limits = match map_get(body, B_WELCOME_LIMITS) {
            Some(Value::Map(values)) => Limits {
                max_nick_bytes: map_usize(values, 0),
                max_room_name_bytes: map_usize(values, 1),
                max_message_bytes: map_usize(values, 2),
                max_rooms_per_session: map_usize(values, 3),
                messages_per_minute: map_usize(values, 4),
            },
            _ => Limits::default(),
        };
        Some(Welcome {
            hub_name: map_text(body, B_WELCOME_HUB).map(str::to_string),
            version: map_text(body, B_WELCOME_VER).map(str::to_string),
            capabilities,
            limits,
        })
    }

    pub fn resource_descriptor(&self) -> Option<ResourceDescriptor> {
        if self.message_type() != Some(T_RESOURCE_ENVELOPE) {
            return None;
        }
        let body = self.map(K_BODY)?;
        let id = match map_get(body, B_RES_ID)? {
            Value::Bytes(value) if !value.is_empty() => value.clone(),
            _ => return None,
        };
        let kind = match map_get(body, B_RES_KIND)? {
            Value::Text(value) if !value.is_empty() => value.clone(),
            _ => return None,
        };
        let size = match map_get(body, B_RES_SIZE)? {
            Value::Integer(value) if *value >= 0 => usize::try_from(*value).ok()?,
            _ => return None,
        };
        let sha256 = match map_get(body, B_RES_SHA256) {
            Some(Value::Bytes(value)) => Some(value.as_slice().try_into().ok()?),
            None => None,
            _ => return None,
        };
        let encoding = match map_get(body, B_RES_ENCODING) {
            Some(Value::Text(value)) => Some(value.clone()),
            None => None,
            _ => return None,
        };
        Some(ResourceDescriptor {
            id,
            kind,
            size,
            sha256,
            encoding,
        })
    }

    pub fn unsigned(&self, field: i128) -> Option<u128> {
        match self.get(field)? {
            Value::Integer(value) if *value >= 0 => Some(*value as u128),
            _ => None,
        }
    }

    pub fn text(&self, field: i128) -> Option<&str> {
        match self.get(field)? {
            Value::Text(value) => Some(value),
            _ => None,
        }
    }

    pub fn bytes(&self, field: i128) -> Option<&[u8]> {
        match self.get(field)? {
            Value::Bytes(value) => Some(value),
            _ => None,
        }
    }

    pub fn map(&self, field: i128) -> Option<&Map> {
        match self.get(field)? {
            Value::Map(value) => Some(value),
            _ => None,
        }
    }

    pub fn set(&mut self, field: i128, value: Value) {
        self.fields.insert(key(field), value);
    }

    pub fn remove(&mut self, field: i128) {
        self.fields.remove(&key(field));
    }
}

pub fn normalize_nick(value: &str, max_bytes: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > max_bytes || value.contains(['\n', '\r', '\0']) {
        None
    } else {
        Some(value.to_string())
    }
}

pub fn normalize_room(value: &str, max_bytes: usize) -> Option<String> {
    let value = value.trim().trim_start_matches('#').to_ascii_lowercase();
    if value.is_empty() || value.len() > max_bytes || value.contains([' ', '\n', '\r', '\0']) {
        None
    } else {
        Some(value)
    }
}

fn valid_text(value: &str, max_bytes: usize) -> bool {
    let value = value.trim();
    !value.is_empty() && value.len() <= max_bytes && !value.contains(['\0', '\r'])
}

pub fn map_get(map: &Map, field: i128) -> Option<&Value> {
    map.get(&key(field))
}

pub fn parse_room_list_notice(text: &str) -> Option<Vec<RoomInfo>> {
    let stripped = text.trim();
    if stripped == "No public rooms registered" {
        return Some(Vec::new());
    }
    let mut lines = text.lines();
    if !lines
        .next()?
        .trim_start()
        .starts_with("Registered public rooms")
    {
        return None;
    }
    let mut rooms = lines
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (name, topic) = line
                .split_once(" - ")
                .map_or((line, None), |(name, topic)| {
                    (
                        name,
                        (!topic.trim().is_empty()).then(|| topic.trim().into()),
                    )
                });
            Some(RoomInfo {
                name: name.trim().trim_start_matches('#').to_ascii_lowercase(),
                topic,
            })
        })
        .collect::<Vec<_>>();
    rooms.sort_by(|left, right| left.name.cmp(&right.name));
    Some(rooms)
}

pub fn parse_who_notice(text: &str) -> Option<(String, Vec<UserInfo>)> {
    let rest = text.strip_prefix("members in ")?;
    let (room, entries) = rest.split_once(": ")?;
    let room = room.trim().trim_start_matches('#').to_ascii_lowercase();
    if room.is_empty() {
        return None;
    }
    let users = if entries.trim() == "(none)" {
        Vec::new()
    } else {
        WHO_ENTRY
            .captures_iter(entries)
            .filter_map(|captures| {
                if let Some(hash) = captures.name("hash") {
                    return Some(UserInfo {
                        nick: None,
                        identity: hash.as_str().to_ascii_lowercase(),
                        operator: false,
                        voiced: false,
                    });
                }
                Some(UserInfo {
                    nick: Some(captures.name("nick")?.as_str().trim().to_string()),
                    identity: captures.name("prefix")?.as_str().to_ascii_lowercase(),
                    operator: false,
                    voiced: false,
                })
            })
            .collect()
    };
    Some((room, users))
}

fn map_text(map: &Map, field: i128) -> Option<&str> {
    match map_get(map, field)? {
        Value::Text(value) => Some(value),
        _ => None,
    }
}

fn map_bool(map: &Map, field: i128) -> bool {
    matches!(map_get(map, field), Some(Value::Bool(true)))
}

fn map_usize(map: &Map, field: i128) -> Option<usize> {
    match map_get(map, field)? {
        Value::Integer(value) if *value >= 0 => usize::try_from(*value).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trip_uses_integer_keys() {
        let mut envelope = Envelope::new(T_MSG, &[7; 16]);
        envelope.set(K_ROOM, Value::Text("test".into()));
        envelope.set(K_BODY, Value::Text("hello".into()));
        let decoded = Envelope::decode(&envelope.encode().unwrap()).unwrap();
        assert_eq!(decoded.integer(K_T), Some(T_MSG));
        assert_eq!(decoded.text(K_ROOM), Some("test"));
    }

    #[test]
    fn rejects_missing_required_fields() {
        let bytes = serde_cbor::to_vec(&Value::Map(Map::new())).unwrap();
        assert!(Envelope::decode(&bytes).is_err());
    }

    #[test]
    fn normalizes_common_client_values() {
        assert_eq!(normalize_nick(" Nomad ", 32).as_deref(), Some("Nomad"));
        assert_eq!(normalize_room("#Rust", 64).as_deref(), Some("rust"));
        assert!(normalize_room("bad room", 64).is_none());
    }

    #[test]
    fn client_builders_produce_server_compatible_envelopes() {
        let hello = Envelope::hello(&[7; 16], Some(" Nomad "));
        assert_eq!(hello.message_type(), Some(T_HELLO));
        assert_eq!(hello.nick(), Some(" Nomad "));
        assert_eq!(hello.source(), Some([7; 16]));

        let join = Envelope::join(&[7; 16], "#Rust", None).unwrap();
        assert_eq!(join.room(), Some("rust"));

        let part = Envelope::part(&[7; 16], "#Rust").unwrap();
        assert_eq!(part.message_type(), Some(T_PART));
        assert_eq!(part.room(), Some("rust"));

        let message = Envelope::message(&[7; 16], "#Rust", "hello", false).unwrap();
        assert_eq!(message.message_type(), Some(T_MSG));
        assert_eq!(message.body_text(), Some("hello"));
    }

    #[test]
    fn parses_resource_descriptor() {
        let mut body = Map::new();
        body.insert(Value::Integer(B_RES_ID), Value::Bytes(vec![1; 8]));
        body.insert(Value::Integer(B_RES_KIND), Value::Text("notice".into()));
        body.insert(Value::Integer(B_RES_SIZE), Value::Integer(42));
        body.insert(Value::Integer(B_RES_SHA256), Value::Bytes(vec![2; 32]));
        body.insert(Value::Integer(B_RES_ENCODING), Value::Text("utf-8".into()));
        let mut envelope = Envelope::new(T_RESOURCE_ENVELOPE, &[7; 16]);
        envelope.set(K_BODY, Value::Map(body));

        let descriptor = envelope.resource_descriptor().unwrap();
        assert_eq!(descriptor.kind, "notice");
        assert_eq!(descriptor.size, 42);
        assert_eq!(descriptor.sha256, Some([2; 32]));
    }

    #[test]
    fn room_state_round_trips_as_an_optional_extension() {
        let state = RoomState {
            registered: true,
            modes: "+mnrt".into(),
            topic: Some("Rust room".into()),
        };
        let mut envelope = Envelope::new(T_JOINED, &[7; 16]);
        envelope.set(K_ROOM, Value::Text("rust".into()));
        envelope.set_room_state(&state);

        let decoded = Envelope::decode(&envelope.encode().unwrap()).unwrap();
        assert_eq!(decoded.room_state(), Some(state));
    }

    #[test]
    fn structured_user_list_round_trips_roles() {
        let users = vec![UserInfo {
            nick: Some("alice".into()),
            identity: "11111111111111111111111111111111".into(),
            operator: true,
            voiced: false,
        }];
        let mut envelope = Envelope::new(T_NOTICE, &[7; 16]);
        envelope.set_user_list(&users);
        let decoded = Envelope::decode(&envelope.encode().unwrap()).unwrap();
        assert_eq!(decoded.user_list(), Some(users));
    }

    #[test]
    fn builds_verifiable_resource_envelope() {
        let data = b"a long bot response";
        let envelope =
            Envelope::resource(&[7; 16], Some("#Bots"), "message", data, Some("utf-8")).unwrap();
        let descriptor = envelope.resource_descriptor().unwrap();
        assert_eq!(envelope.room(), Some("bots"));
        assert_eq!(descriptor.size, data.len());
        assert_eq!(
            descriptor.sha256,
            Some(Sha256::digest(data).as_slice().try_into().unwrap())
        );
    }

    #[test]
    fn parses_welcome_capabilities_and_limits() {
        let capabilities = Map::from([
            (Value::Integer(CAP_RESOURCE_ENVELOPE), Value::Bool(true)),
            (Value::Integer(CAP_ACTION), Value::Bool(true)),
        ]);
        let limits = Map::from([
            (Value::Integer(0), Value::Integer(32)),
            (Value::Integer(2), Value::Integer(16_384)),
        ]);
        let body = Map::from([
            (
                Value::Integer(B_WELCOME_HUB),
                Value::Text("Rust Hub".into()),
            ),
            (Value::Integer(B_WELCOME_VER), Value::Text("0.1.0".into())),
            (Value::Integer(B_WELCOME_CAPS), Value::Map(capabilities)),
            (Value::Integer(B_WELCOME_LIMITS), Value::Map(limits)),
        ]);
        let mut envelope = Envelope::new(T_WELCOME, &[9; 16]);
        envelope.set(K_BODY, Value::Map(body));

        let welcome = envelope.welcome().unwrap();
        assert_eq!(welcome.hub_name.as_deref(), Some("Rust Hub"));
        assert!(welcome.capabilities.resource_envelope);
        assert!(welcome.capabilities.action);
        assert!(!welcome.capabilities.direct_notice);
        assert!(!welcome.capabilities.room_state);
        assert!(!welcome.capabilities.user_list);
        assert_eq!(welcome.limits.max_nick_bytes, Some(32));
        assert_eq!(welcome.limits.max_message_bytes, Some(16_384));
    }

    #[test]
    fn hello_advertises_structured_state_extensions() {
        let hello = Envelope::hello(&[7; 16], None);
        let Some(Value::Map(body)) = hello.get(K_BODY) else {
            panic!("HELLO body is missing");
        };
        let Some(Value::Map(capabilities)) = body.get(&Value::Integer(B_HELLO_CAPS)) else {
            panic!("HELLO capabilities are missing");
        };

        assert_eq!(
            capabilities.get(&Value::Integer(CAP_ROOM_STATE)),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            capabilities.get(&Value::Integer(CAP_USER_LIST)),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn parses_list_and_who_notices() {
        assert_eq!(
            parse_room_list_notice("Registered public rooms:\n  alpha\n  rust - Rust room"),
            Some(vec![
                RoomInfo {
                    name: "alpha".into(),
                    topic: None,
                },
                RoomInfo {
                    name: "rust".into(),
                    topic: Some("Rust room".into()),
                },
            ])
        );
        assert_eq!(
            parse_who_notice(
                "members in rust: alice (0b0b0b0b0b0b), user, alt (161616161616), 22222222222222222222222222222222"
            ),
            Some((
                "rust".into(),
                vec![
                    UserInfo {
                        nick: Some("alice".into()),
                        identity: "0b0b0b0b0b0b".into(),
                        operator: false,
                        voiced: false,
                    },
                    UserInfo {
                        nick: Some("user, alt".into()),
                        identity: "161616161616".into(),
                        operator: false,
                        voiced: false,
                    },
                    UserInfo {
                        nick: None,
                        identity: "22222222222222222222222222222222".into(),
                        operator: false,
                        voiced: false,
                    },
                ],
            ))
        );
        assert_eq!(
            parse_room_list_notice("No public rooms registered"),
            Some(Vec::new())
        );
    }
}
