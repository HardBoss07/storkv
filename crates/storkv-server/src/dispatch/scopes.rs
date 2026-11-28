//! Scope lifecycle request handlers.

use storkv_core::{CompactionStyle, Durability, Retention, ScopeOptions, StorageMode};
use storkv_proto::{
    Response, WireCompactionStyle, WireDurability, WireRetention, WireScopeHandle, WireScopeOptions,
};

use super::conversions::core_error_to_wire;
use crate::server::Server;

impl Server {
    pub(super) fn handle_create_scope(&self, name: &str, options: WireScopeOptions) -> Response {
        let scope_options = ScopeOptions {
            durability: durability_from_wire(options.durability),
            retention: retention_from_wire(options.retention),
            prefix_bloom_len: options.prefix_bloom_len,
            storage_mode: StorageMode::Disk,
            min_ttl: options.min_ttl,
            max_ttl: options.max_ttl,
            allow_permanent: options.allow_permanent,
            active_ttl_tracking: options.active_ttl_tracking,
        };
        match self.db.create_scope(name, scope_options) {
            Ok(handle) => Response::Scope(WireScopeHandle {
                scope_id: handle.scope_id,
                name: handle.name,
            }),
            Err(error) => Response::Error(core_error_to_wire(error)),
        }
    }

    pub(super) fn handle_drop_scope(&self, name: &str) -> Response {
        let scope_id = self.db.inner().scope_names.read().get(name).copied();
        match self.db.drop_scope(name) {
            Ok(()) => {
                if let Some(scope_id) = scope_id {
                    self.stats.record_write(scope_id);
                }
                Response::Empty
            }
            Err(error) => Response::Error(core_error_to_wire(error)),
        }
    }
}

fn durability_from_wire(durability: WireDurability) -> Durability {
    match durability {
        WireDurability::StrictFsync => Durability::StrictFsync,
        WireDurability::Buffered { sync_interval } => Durability::Buffered { sync_interval },
    }
}

fn compaction_style_from_wire(style: WireCompactionStyle) -> CompactionStyle {
    match style {
        WireCompactionStyle::Leveled => CompactionStyle::Leveled,
        WireCompactionStyle::Fifo => CompactionStyle::Fifo,
    }
}

fn retention_from_wire(retention: WireRetention) -> Retention {
    match retention {
        WireRetention::Permanent => Retention::Permanent,
        WireRetention::Ttl {
            default_ttl,
            compaction_style,
        } => Retention::Ttl {
            default_ttl,
            compaction_style: compaction_style_from_wire(compaction_style),
        },
    }
}
