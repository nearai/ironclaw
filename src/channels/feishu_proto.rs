//! Feishu WebSocket binary frame format (protobuf).
//!
//! Wire-compatible with the official `pbbp2.proto` used by
//! `larksuite/oapi-sdk-go/ws` and `@larksuiteoapi/node-sdk`.

/// Key-value header inside a [`Frame`].
#[derive(Clone, PartialEq, prost::Message)]
pub struct Header {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

/// Top-level WebSocket binary frame exchanged with Feishu servers.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Frame {
    #[prost(uint64, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, tag = "2")]
    pub log_id: u64,
    #[prost(int32, tag = "3")]
    pub service: i32,
    #[prost(int32, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<Header>,
    #[prost(string, tag = "6")]
    pub payload_encoding: String,
    #[prost(string, tag = "7")]
    pub payload_type: String,
    #[prost(bytes = "vec", tag = "8")]
    pub payload: Vec<u8>,
    #[prost(string, tag = "9")]
    pub log_id_new: String,
}

// Frame.method values
pub const FRAME_CONTROL: i32 = 0;
pub const FRAME_DATA: i32 = 1;

// Header type values
pub const MSG_TYPE_EVENT: &str = "event";
pub const MSG_TYPE_PING: &str = "ping";
pub const MSG_TYPE_PONG: &str = "pong";

// Header keys
pub const HDR_TYPE: &str = "type";
pub const HDR_MESSAGE_ID: &str = "message_id";
pub const HDR_TRACE_ID: &str = "trace_id";
pub const HDR_SUM: &str = "sum";
pub const HDR_SEQ: &str = "seq";
pub const HDR_BIZ_RT: &str = "biz_rt";

impl Frame {
    pub fn header_str(&self, key: &str) -> &str {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
            .unwrap_or("")
    }

    pub fn header_int(&self, key: &str) -> usize {
        self.header_str(key).parse().unwrap_or(0)
    }

    pub fn new_ping(service_id: i32) -> Self {
        Self {
            method: FRAME_CONTROL,
            service: service_id,
            headers: vec![Header {
                key: HDR_TYPE.into(),
                value: MSG_TYPE_PING.into(),
            }],
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn ping_frame_round_trip() {
        let frame = Frame::new_ping(42);
        let bytes = frame.encode_to_vec();
        let decoded = Frame::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.method, FRAME_CONTROL);
        assert_eq!(decoded.service, 42);
        assert_eq!(decoded.header_str(HDR_TYPE), MSG_TYPE_PING);
    }
}
