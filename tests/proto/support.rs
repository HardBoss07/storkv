use bytes::BytesMut;

pub(super) fn frame_from_body(payload_type: u8, payload: &[u8]) -> BytesMut {
    let mut body = BytesMut::new();
    body.extend_from_slice(storkv_proto::PROTOCOL_MAGIC);
    body.extend_from_slice(&storkv_proto::PROTOCOL_VERSION.to_be_bytes());
    body.extend_from_slice(&7u64.to_le_bytes());
    body.extend_from_slice(&[payload_type]);
    body.extend_from_slice(payload);

    let mut framed = BytesMut::new();
    framed.extend_from_slice(&(body.len() as u32).to_be_bytes());
    framed.extend_from_slice(&body);
    framed
}

pub(super) fn value_with_tag(tag: u8, payload: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(payload.len() + 1);
    output.push(tag);
    output.extend_from_slice(payload);
    output
}
