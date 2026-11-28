//! Redis source reader for migration.
use std::error::Error;
use std::future::Future;
use std::pin::Pin;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use super::{Record, encode};

#[derive(Debug)]
enum Reply {
    Status,
    Error(String),
    Integer(i64),
    Bulk(Option<Vec<u8>>),
    Array(Option<Vec<Reply>>),
}

pub(super) async fn read_redis(url: &str) -> Result<Vec<Record>, Box<dyn Error>> {
    let address = redis_address(url);
    let stream = TcpStream::connect(&address).await?;
    let (read_half, mut writer) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    send_command(&mut writer, &[b"SELECT", b"0"]).await?;
    expect_status(&read_reply(&mut reader).await?)?;

    let mut records = Vec::new();
    let mut cursor = b"0".to_vec();
    loop {
        send_command(
            &mut writer,
            &[b"SCAN", cursor.as_slice(), b"COUNT", b"1000"],
        )
        .await?;
        let reply = read_reply(&mut reader).await?;
        let Reply::Array(Some(parts)) = reply else {
            return Err("redis SCAN returned an unexpected reply".into());
        };
        let mut parts = parts.into_iter();
        let next_cursor = match parts.next() {
            Some(Reply::Bulk(Some(cursor))) => cursor,
            _ => return Err("redis SCAN returned an invalid cursor".into()),
        };
        let keys = match parts.next() {
            Some(Reply::Array(Some(keys))) => keys,
            _ => return Err("redis SCAN returned an invalid key list".into()),
        };
        cursor = next_cursor;
        for key in keys {
            let Reply::Bulk(Some(key)) = key else {
                continue;
            };
            let Some(record) = fetch_record(&mut writer, &mut reader, key).await? else {
                continue;
            };
            records.push(record);
        }
        if cursor == b"0" {
            break;
        }
    }
    Ok(records)
}

fn redis_address(url: &str) -> String {
    let address = url.strip_prefix("redis://").unwrap_or(url);
    let address = address.split('/').next().unwrap_or(address);
    if address.contains(':') {
        address.to_string()
    } else {
        format!("{address}:6379")
    }
}

async fn fetch_record<W, R>(
    writer: &mut W,
    reader: &mut R,
    key: Vec<u8>,
) -> Result<Option<Record>, Box<dyn Error>>
where
    W: AsyncWriteExt + Unpin,
    R: AsyncBufRead + Unpin,
{
    send_command(writer, &[b"GET", key.as_slice()]).await?;
    let value = match read_reply(reader).await? {
        Reply::Bulk(Some(value)) => value,
        Reply::Bulk(None) => return Ok(None),
        Reply::Error(error) => return Err(error.into()),
        _ => return Err("redis GET returned an unexpected reply".into()),
    };
    send_command(writer, &[b"PTTL", key.as_slice()]).await?;
    let ttl = match read_reply(reader).await? {
        Reply::Integer(ttl) => ttl,
        _ => -1,
    };
    let expires_at_ms = if ttl > 0 {
        storkv_core::TimestampMs::now()
            .get()
            .saturating_add(u64::try_from(ttl).unwrap_or(0))
    } else {
        0
    };
    Ok(Some(Record {
        key_b64: encode(&key),
        value_b64: encode(&value),
        expires_at_ms,
    }))
}

async fn send_command<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    arguments: &[&[u8]],
) -> Result<(), Box<dyn Error>> {
    let mut encoded = format!("*{}\r\n", arguments.len()).into_bytes();
    for argument in arguments {
        encoded.extend_from_slice(format!("${}\r\n", argument.len()).as_bytes());
        encoded.extend_from_slice(argument);
        encoded.extend_from_slice(b"\r\n");
    }
    writer.write_all(&encoded).await?;
    Ok(())
}

type ReplyFuture<'a> = Pin<Box<dyn Future<Output = Result<Reply, Box<dyn Error>>> + 'a>>;

fn read_reply<'a, R: AsyncBufRead + Unpin>(reader: &'a mut R) -> ReplyFuture<'a> {
    Box::pin(read_reply_inner(reader))
}

async fn read_reply_inner<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Reply, Box<dyn Error>> {
    let mut line = Vec::new();
    if reader.read_until(b'\n', &mut line).await? == 0 {
        return Err("redis connection closed".into());
    }
    if line.ends_with(b"\r\n") {
        line.truncate(line.len() - 2);
    }
    let Some((&prefix, rest)) = line.split_first() else {
        return Err("empty redis reply".into());
    };
    match prefix {
        b'+' => Ok(Reply::Status),
        b'-' => Ok(Reply::Error(String::from_utf8_lossy(rest).to_string())),
        b':' => Ok(Reply::Integer(parse_i64(rest)?)),
        b'$' => read_bulk(reader, rest).await,
        b'*' => read_array(reader, rest).await,
        _ => Err(format!("unsupported redis reply prefix: {}", char::from(prefix)).into()),
    }
}

async fn read_bulk<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    length: &[u8],
) -> Result<Reply, Box<dyn Error>> {
    let length = parse_i64(length)?;
    if length < 0 {
        return Ok(Reply::Bulk(None));
    }
    let length = usize::try_from(length)?;
    let mut data = vec![0u8; length + 2];
    reader.read_exact(&mut data).await?;
    data.truncate(length);
    Ok(Reply::Bulk(Some(data)))
}

async fn read_array<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    count: &[u8],
) -> Result<Reply, Box<dyn Error>> {
    let count = parse_i64(count)?;
    if count < 0 {
        return Ok(Reply::Array(None));
    }
    let mut values = Vec::with_capacity(usize::try_from(count)?);
    for _ in 0..count {
        values.push(read_reply(reader).await?);
    }
    Ok(Reply::Array(Some(values)))
}

fn expect_status(reply: &Reply) -> Result<(), Box<dyn Error>> {
    match reply {
        Reply::Status => Ok(()),
        Reply::Error(error) => Err(error.clone().into()),
        _ => Err("expected a redis status reply".into()),
    }
}

fn parse_i64(bytes: &[u8]) -> Result<i64, Box<dyn Error>> {
    Ok(std::str::from_utf8(bytes)?.parse()?)
}
