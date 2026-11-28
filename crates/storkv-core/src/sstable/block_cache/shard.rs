//! One cache shard with an index-based doubly linked LRU list.

use std::collections::HashMap;

use bytes::Bytes;

use super::BlockKey;

/// One arena node. Links index other nodes in the same shard.
struct Node {
    key: BlockKey,
    value: Bytes,
    prev: Option<usize>,
    next: Option<usize>,
}

/// Shard with an index-based doubly linked LRU list.
pub(super) struct Shard {
    nodes: Vec<Node>,
    index: HashMap<BlockKey, usize>,
    free: Vec<usize>,
    head: Option<usize>,
    tail: Option<usize>,
    pub(super) bytes: usize,
    max_bytes: usize,
}

impl Shard {
    pub(super) fn new(max_bytes: usize) -> Self {
        Self {
            nodes: Vec::new(),
            index: HashMap::new(),
            free: Vec::new(),
            head: None,
            tail: None,
            bytes: 0,
            max_bytes,
        }
    }

    fn unlink(&mut self, index: usize) {
        let (prev, next) = {
            let node = &self.nodes[index];
            (node.prev, node.next)
        };
        match prev {
            Some(prev) => self.nodes[prev].next = next,
            None => self.head = next,
        }
        match next {
            Some(next) => self.nodes[next].prev = prev,
            None => self.tail = prev,
        }
        self.nodes[index].prev = None;
        self.nodes[index].next = None;
    }

    fn push_front(&mut self, index: usize) {
        self.nodes[index].prev = None;
        self.nodes[index].next = self.head;
        match self.head {
            Some(head) => self.nodes[head].prev = Some(index),
            None => self.tail = Some(index),
        }
        self.head = Some(index);
    }

    fn touch(&mut self, index: usize) {
        if self.head == Some(index) {
            return;
        }
        self.unlink(index);
        self.push_front(index);
    }

    fn remove(&mut self, index: usize) {
        self.unlink(index);
        let key = self.nodes[index].key;
        self.bytes -= self.nodes[index].value.len();
        self.index.remove(&key);
        self.free.push(index);
    }

    pub(super) fn get(&mut self, key: &BlockKey) -> Option<Bytes> {
        let index = *self.index.get(key)?;
        self.touch(index);
        Some(self.nodes[index].value.clone())
    }

    pub(super) fn put(&mut self, key: BlockKey, value: Bytes) {
        if self.max_bytes == 0 || value.len() > self.max_bytes {
            return;
        }
        if let Some(&index) = self.index.get(&key) {
            self.bytes -= self.nodes[index].value.len();
            self.bytes += value.len();
            self.nodes[index].value = value;
            self.touch(index);
        } else {
            let index = match self.free.pop() {
                Some(index) => index,
                None => {
                    self.nodes.push(Node {
                        key,
                        value: Bytes::new(),
                        prev: None,
                        next: None,
                    });
                    self.nodes.len() - 1
                }
            };
            self.nodes[index] = Node {
                key,
                value,
                prev: None,
                next: None,
            };
            self.index.insert(key, index);
            self.bytes += self.nodes[index].value.len();
            self.push_front(index);
        }
        while self.bytes > self.max_bytes {
            let Some(tail) = self.tail else {
                break;
            };
            self.remove(tail);
        }
    }

    fn invalidate(&mut self, key: &BlockKey) {
        if let Some(index) = self.index.get(key).copied() {
            self.remove(index);
        }
    }

    pub(super) fn invalidate_sst(&mut self, scope_id: u32, sst_id: u64) {
        let keys: Vec<BlockKey> = self
            .index
            .keys()
            .filter(|key| key.scope_id == scope_id && key.sst_id == sst_id)
            .copied()
            .collect();
        for key in keys {
            self.invalidate(&key);
        }
    }
}
