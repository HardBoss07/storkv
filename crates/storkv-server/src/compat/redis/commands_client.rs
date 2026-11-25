//! Redis connection commands: AUTH, CLIENT, and COMMAND.

use bytes::Bytes;
use subtle::ConstantTimeEq;

use super::RedisSession;
use super::codec::Resp;
use super::helpers::{COMMAND_SPECS, CommandSpec, upper, wrong_arity};

const WRONG_PASS: &str = "WRONGPASS invalid username-password pair or user is disabled.";
const NO_PASSWORD: &str =
    "ERR AUTH <password> called without any password configured for the default user.";
const INVALID_CLIENT_NAME: &str =
    "ERR Client names cannot contain spaces, newlines or special characters.";

impl RedisSession {
    pub(super) fn auth(&mut self, args: &[Bytes]) -> Resp {
        let (username, password) = match args {
            [_, password] => (None, password),
            [_, username, password] => (Some(username), password),
            _ => return wrong_arity("auth"),
        };
        match self.verify_credentials(username, password) {
            Ok(()) => {
                self.authenticated = true;
                Resp::simple("OK")
            }
            Err(response) => response,
        }
    }

    /// Check credentials without changing connection state.
    pub(super) fn verify_credentials(
        &self,
        username: Option<&Bytes>,
        password: &[u8],
    ) -> Result<(), Resp> {
        let Some(configured) = self.server.redis_password() else {
            return Err(Resp::error(NO_PASSWORD));
        };
        let username_ok = username.is_none_or(|name| name.as_ref() == b"default");
        if username_ok && password_matches(configured, password) {
            Ok(())
        } else {
            Err(Resp::error(WRONG_PASS))
        }
    }

    pub(super) fn client_command(&mut self, args: &[Bytes]) -> Resp {
        let Some(subcommand) = args.get(1) else {
            return wrong_arity("client");
        };
        match upper(subcommand).as_str() {
            "ID" if args.len() == 2 => Resp::Integer(self.client_id as i64),
            "GETNAME" if args.len() == 2 => match &self.client_name {
                Some(name) => Resp::bulk(name.clone()),
                None => Resp::null(),
            },
            "SETNAME" if args.len() == 3 => {
                let name = &args[2];
                if let Err(response) = validate_client_name(name) {
                    return response;
                }
                self.client_name = Some(String::from_utf8_lossy(name).into_owned());
                Resp::simple("OK")
            }
            "ID" | "GETNAME" | "SETNAME" => wrong_arity("client"),
            other => Resp::error(format!(
                "ERR Unknown CLIENT subcommand or wrong number of arguments for '{other}'"
            )),
        }
    }

    pub(super) fn command_meta(&self, args: &[Bytes]) -> Resp {
        match args.get(1) {
            None => Resp::array(COMMAND_SPECS.iter().map(command_entry).collect()),
            Some(subcommand) if upper(subcommand) == "COUNT" && args.len() == 2 => {
                Resp::Integer(COMMAND_SPECS.len() as i64)
            }
            Some(subcommand) => Resp::error(format!(
                "ERR Unknown COMMAND subcommand or wrong number of arguments for '{}'",
                String::from_utf8_lossy(subcommand)
            )),
        }
    }
}

/// Reject a client name that holds a space or a control character.
pub(super) fn validate_client_name(name: &[u8]) -> Result<(), Resp> {
    if name.iter().any(|byte| *byte <= b' ' || *byte == 0x7F) {
        Err(Resp::error(INVALID_CLIENT_NAME))
    } else {
        Ok(())
    }
}

fn password_matches(configured: &str, provided: &[u8]) -> bool {
    configured.as_bytes().ct_eq(provided).into()
}

fn command_entry(spec: &CommandSpec) -> Resp {
    Resp::array(vec![
        Resp::bulk(spec.name.to_ascii_lowercase()),
        Resp::Integer(i64::from(spec.arity)),
        Resp::array(Vec::new()),
        Resp::Integer(i64::from(spec.first_key)),
        Resp::Integer(i64::from(spec.last_key)),
        Resp::Integer(i64::from(spec.step)),
    ])
}
