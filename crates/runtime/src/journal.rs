use crate::TrieStorage;
use fluentbase_poseidon::Hashable;
use fluentbase_types::{Address, Bytes, ExitCode, B256};
use halo2curves::bn256::Fr;
use hashbrown::HashMap;
use std::mem::take;

enum JournalEvent {
    ItemChanged {
        key: [u8; 32],
        value: Vec<[u8; 32]>,
        flags: u32,
        prev_state: Option<usize>,
    },
    ItemRemoved {
        key: [u8; 32],
        prev_state: Option<usize>,
    },
}

impl JournalEvent {
    fn key(&self) -> &[u8; 32] {
        match self {
            JournalEvent::ItemChanged { key, .. } => key,
            JournalEvent::ItemRemoved { key, .. } => key,
        }
    }

    fn value(&self) -> Option<(Vec<[u8; 32]>, u32)> {
        match self {
            JournalEvent::ItemChanged { value, flags, .. } => Some((value.clone(), *flags)),
            JournalEvent::ItemRemoved { .. } => None,
        }
    }

    fn prev_state(&self) -> Option<usize> {
        match self {
            JournalEvent::ItemChanged { prev_state, .. } => *prev_state,
            JournalEvent::ItemRemoved { prev_state, .. } => *prev_state,
        }
    }
}

pub struct JournalCheckpoint(pub u32, pub u32);

impl Into<(u32, u32)> for JournalCheckpoint {
    fn into(self) -> (u32, u32) {
        (self.0, self.1)
    }
}

impl JournalCheckpoint {
    fn state(&self) -> usize {
        self.0 as usize
    }

    fn logs(&self) -> usize {
        self.1 as usize
    }
}

pub struct JournalLog {
    address: Address,
    topics: Vec<B256>,
    data: Bytes,
}

pub trait IJournaledTrie {
    fn checkpoint(&mut self) -> JournalCheckpoint;
    fn get(&self, key: &[u8; 32]) -> Option<(Vec<[u8; 32]>, bool)>;
    fn update(&mut self, key: &[u8; 32], value: &Vec<[u8; 32]>, flags: u32);
    fn store(&mut self, address: &Address, slot: &[u8; 32], value: &[u8; 32]);
    fn load(&mut self, address: &Address, slot: &[u8; 32]) -> Option<([u8; 32], bool)>;
    fn remove(&mut self, key: &[u8; 32]);
    fn compute_root(&self) -> [u8; 32];
    fn emit_log(&mut self, address: Address, topics: Vec<B256>, data: Bytes);
    fn commit(&mut self) -> Result<([u8; 32], Vec<JournalLog>), ExitCode>;
    fn rollback(&mut self, checkpoint: JournalCheckpoint);
}

pub struct JournaledTrie<'a, DB: TrieStorage> {
    storage: &'a mut DB,
    state: HashMap<[u8; 32], usize>,
    logs: Vec<JournalLog>,
    journal: Vec<JournalEvent>,
    root: [u8; 32],
    committed: usize,
}

impl<'a, DB: TrieStorage + 'a> JournaledTrie<'a, DB> {
    const DOMAIN: Fr = Fr::zero();

    pub fn new(storage: &'a mut DB) -> Self {
        let root = storage.compute_root();
        Self {
            storage,
            state: HashMap::new(),
            logs: Vec::new(),
            journal: Vec::new(),
            root,
            committed: 0,
        }
    }

    pub fn compress_value(val: &[u8; 32]) -> Fr {
        let mut bytes32 = [0u8; 32];
        bytes32[0..16].copy_from_slice(&val[0..16]);
        let val1 = Fr::from_bytes(&bytes32).unwrap();
        bytes32[0..16].copy_from_slice(&val[16..]);
        let val2 = Fr::from_bytes(&bytes32).unwrap();
        let hasher = Fr::hasher();
        hasher.hash([val1, val2], Self::DOMAIN)
    }

    pub fn storage_key(address: &Address, slot: &[u8; 32]) -> [u8; 32] {
        // storage key is `p(address, p(slot_0, slot_1, d), d)`
        let address = {
            let mut bytes32 = [0u8; 32];
            bytes32[0..20].copy_from_slice(address.as_slice());
            Fr::from_bytes(&bytes32).unwrap()
        };
        let slot = Self::compress_value(slot);
        let hasher = Fr::hasher();
        let key = hasher.hash([address, slot], Self::DOMAIN);
        key.to_bytes()
    }
}

impl<'a, DB: TrieStorage> IJournaledTrie for JournaledTrie<'a, DB> {
    fn checkpoint(&mut self) -> JournalCheckpoint {
        JournalCheckpoint(self.journal.len() as u32, 0)
    }

    fn get(&self, key: &[u8; 32]) -> Option<(Vec<[u8; 32]>, bool)> {
        match self.state.get(key) {
            Some(index) => self
                .journal
                .get(*index)
                .unwrap()
                .value()
                .map(|v| v.0)
                .map(|v| (v, false)),
            None => self.storage.get(key).map(|v| (v, true)),
        }
    }

    fn update(&mut self, key: &[u8; 32], value: &Vec<[u8; 32]>, flags: u32) {
        let pos = self.journal.len();
        self.journal.push(JournalEvent::ItemChanged {
            key: *key,
            value: value.clone(),
            flags,
            prev_state: self.state.get(key).copied(),
        });
        self.state.insert(*key, pos);
    }

    fn store(&mut self, address: &Address, slot: &[u8; 32], value: &[u8; 32]) {
        let storage_key = Self::storage_key(address, slot);
        self.update(&storage_key, &vec![*value], 1);
    }

    fn load(&mut self, address: &Address, slot: &[u8; 32]) -> Option<([u8; 32], bool)> {
        let storage_key = Self::storage_key(address, slot);
        let (values, is_cold) = self.get(&storage_key)?;
        assert_eq!(
            values.len(),
            1,
            "not proper journal usage, storage must have only one element"
        );
        Some((values[0], is_cold))
    }

    fn remove(&mut self, key: &[u8; 32]) {
        let pos = self.journal.len();
        self.journal.push(JournalEvent::ItemRemoved {
            key: *key,
            prev_state: self.state.get(key).copied(),
        });
        self.state.insert(*key, pos);
    }

    fn compute_root(&self) -> [u8; 32] {
        self.storage.compute_root()
    }

    fn emit_log(&mut self, address: Address, topics: Vec<B256>, data: Bytes) {
        self.logs.push(JournalLog {
            address,
            topics,
            data,
        });
    }

    fn commit(&mut self) -> Result<([u8; 32], Vec<JournalLog>), ExitCode> {
        if self.committed >= self.journal.len() {
            panic!("nothing to commit")
        }
        for (key, value) in self
            .journal
            .iter()
            .skip(self.committed)
            .map(|v| (*v.key(), v.value()))
            .collect::<HashMap<_, _>>()
            .into_iter()
        {
            match value {
                Some((value, flags)) => {
                    self.storage.update(&key[..], flags, &value)?;
                }
                None => {
                    self.storage.remove(&key[..])?;
                }
            }
        }
        self.journal.clear();
        self.state.clear();
        let logs = take(&mut self.logs);
        self.committed = 0;
        self.root = self.storage.compute_root();
        Ok((self.root, logs))
    }

    fn rollback(&mut self, checkpoint: JournalCheckpoint) {
        if checkpoint.state() < self.committed {
            panic!("reverting already committed changes is not allowed")
        }
        self.journal
            .iter()
            .rev()
            .take(self.journal.len() - checkpoint.state())
            .for_each(|v| match v.prev_state() {
                Some(prev_state) => {
                    self.state.insert(*v.key(), prev_state);
                }
                None => {
                    self.state.remove(v.key());
                }
            });
        self.journal.truncate(checkpoint.state());
        self.logs.truncate(checkpoint.logs());
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        journal::{IJournaledTrie, JournaledTrie},
        zktrie::ZkTrieStateDb,
        TrieStorage,
    };
    use fluentbase_types::{address, InMemoryAccountDb};

    macro_rules! bytes32 {
        ($val:expr) => {{
            let mut word: [u8; 32] = [0; 32];
            if $val.len() > 32 {
                word.copy_from_slice(&$val.as_bytes()[0..32]);
            } else {
                word[0..$val.len()].copy_from_slice($val.as_bytes());
            }
            word
        }};
    }

    fn calc_trie_root(values: Vec<([u8; 32], Vec<[u8; 32]>, u32)>) -> [u8; 32] {
        let mut db = InMemoryAccountDb::default();
        let mut zktrie = ZkTrieStateDb::new_empty(&mut db);
        values
            .iter()
            .for_each(|(key, value, flags)| zktrie.update(&key[..], *flags, value).unwrap());
        zktrie.compute_root()
    }

    #[test]
    fn test_commit_multiple_values() {
        let mut db = InMemoryAccountDb::default();
        let mut zktrie = ZkTrieStateDb::new_empty(&mut db);
        let mut journal = JournaledTrie::new(&mut zktrie);
        journal.update(&bytes32!("key1"), &vec![bytes32!("val1")], 0);
        journal.update(&bytes32!("key2"), &vec![bytes32!("val2")], 1);
        // just commit all changes w/o revert
        journal.commit().unwrap();
        assert_eq!(
            journal.compute_root(),
            calc_trie_root(vec![
                (bytes32!("key1"), vec![bytes32!("val1")], 0),
                (bytes32!("key2"), vec![bytes32!("val2")], 1),
            ])
        );
        // add third key to the existing trie and commit
        journal.update(&bytes32!("key3"), &vec![bytes32!("val3")], 0);
        journal.commit().unwrap();
        assert_eq!(
            journal.compute_root(),
            calc_trie_root(vec![
                (bytes32!("key1"), vec![bytes32!("val1")], 0),
                (bytes32!("key2"), vec![bytes32!("val2")], 1),
                (bytes32!("key3"), vec![bytes32!("val3")], 0),
            ])
        );
    }

    #[test]
    fn test_commit_and_rollback() {
        let mut db = InMemoryAccountDb::default();
        let mut zktrie = ZkTrieStateDb::new_empty(&mut db);
        let mut journal = JournaledTrie::new(&mut zktrie);
        journal.update(&bytes32!("key1"), &vec![bytes32!("val1")], 0);
        journal.update(&bytes32!("key2"), &vec![bytes32!("val2")], 1);
        // just commit all changes w/o revert
        journal.commit().unwrap();
        assert_eq!(
            journal.compute_root(),
            calc_trie_root(vec![
                (bytes32!("key1"), vec![bytes32!("val1")], 0),
                (bytes32!("key2"), vec![bytes32!("val2")], 1),
            ])
        );
        // add third key to the existing trie and rollback
        let checkpoint = journal.checkpoint();
        journal.update(&bytes32!("key3"), &vec![bytes32!("val3")], 0);
        journal.rollback(checkpoint);
        assert_eq!(journal.state.len(), 2);
        assert_eq!(
            journal.compute_root(),
            calc_trie_root(vec![
                (bytes32!("key1"), vec![bytes32!("val1")], 0),
                (bytes32!("key2"), vec![bytes32!("val2")], 1),
            ])
        );
        // modify the same key and rollback
        let checkpoint = journal.checkpoint();
        journal.update(&bytes32!("key2"), &vec![bytes32!("Hello, World")], 0);
        journal.rollback(checkpoint);
        assert_eq!(journal.state.len(), 2);
        assert_eq!(
            journal.compute_root(),
            calc_trie_root(vec![
                (bytes32!("key1"), vec![bytes32!("val1")], 0),
                (bytes32!("key2"), vec![bytes32!("val2")], 1),
            ])
        );
    }

    #[test]
    fn test_rollback_to_empty() {
        let mut db = InMemoryAccountDb::default();
        let mut zktrie = ZkTrieStateDb::new_empty(&mut db);
        let mut journal = JournaledTrie::new(&mut zktrie);
        let checkpoint = journal.checkpoint();
        journal.update(&bytes32!("key1"), &vec![bytes32!("val1")], 0);
        journal.update(&bytes32!("key2"), &vec![bytes32!("val2")], 1);
        journal.rollback(checkpoint);
        assert_eq!(journal.compute_root(), calc_trie_root(vec![]));
        assert_eq!(journal.state.len(), 0);
        let checkpoint = journal.checkpoint();
        journal.update(&bytes32!("key3"), &vec![bytes32!("val3")], 0);
        journal.update(&bytes32!("key4"), &vec![bytes32!("val4")], 1);
        journal.rollback(checkpoint);
        assert_eq!(journal.compute_root(), calc_trie_root(vec![]));
        assert_eq!(journal.state.len(), 0);
    }

    #[test]
    fn test_storage_store_load() {
        let mut db = InMemoryAccountDb::default();
        let mut zktrie = ZkTrieStateDb::new_empty(&mut db);
        let mut journal = JournaledTrie::new(&mut zktrie);
        let address = address!("0000000000000000000000000000000000000001");
        journal.store(&address, &bytes32!("slot1"), &bytes32!("value1"));
        let (value, is_cold) = journal.load(&address, &bytes32!("slot1")).unwrap();
        assert_eq!(value, bytes32!("value1"));
        // value is warm because we've just loaded it into state
        assert_eq!(is_cold, false);
        journal.commit().unwrap();
        let (value, is_cold) = journal.load(&address, &bytes32!("slot1")).unwrap();
        assert_eq!(value, bytes32!("value1"));
        // value is cold because we committed state before that made it empty
        assert_eq!(is_cold, true);
    }
}
