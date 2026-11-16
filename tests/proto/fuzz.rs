use bytes::BytesMut;
use storkv_proto::Message;

#[test]
fn test_deterministic_fuzz_never_panics() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for case in 0..2_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let length = (state % 256) as usize;
        let mut bytes = Vec::with_capacity(length);
        for index in 0..length {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            bytes.push((state >> (index % 8 * 8)) as u8);
        }
        let mut buffer = BytesMut::from(bytes.as_slice());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = Message::decode(&mut buffer);
        }));
        assert!(result.is_ok(), "decoder panicked on fuzz case {case}");
    }
}
