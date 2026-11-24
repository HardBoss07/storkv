//! Reply formatting helpers for Memcached ASCII responses.

use storkv_core::StorkvError;

const MILLIS_PER_SECOND: u64 = 1_000;
const MAX_RELATIVE_EXPTIME_SECS: u64 = 30 * 24 * 60 * 60;

pub(super) fn silence_if(noreply: bool, response: Vec<u8>) -> Vec<u8> {
    if noreply { Vec::new() } else { response }
}

pub(super) fn with_crlf(response: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(response.len() + 2);
    output.extend_from_slice(response);
    output.extend_from_slice(b"\r\n");
    output
}

pub(super) fn server_error(error: StorkvError) -> Vec<u8> {
    match error {
        StorkvError::NotPrimary => b"SERVER_ERROR replica is read-only\r\n".to_vec(),
        other => format!("SERVER_ERROR {other}\r\n").into_bytes(),
    }
}

pub(super) fn expiry_from_exptime(exptime: u32) -> u64 {
    if exptime == 0 {
        return 0;
    }
    let seconds = u64::from(exptime);
    if seconds > MAX_RELATIVE_EXPTIME_SECS {
        seconds.saturating_mul(MILLIS_PER_SECOND)
    } else {
        storkv_core::TimestampMs::now()
            .get()
            .saturating_add(seconds.saturating_mul(MILLIS_PER_SECOND))
    }
}
