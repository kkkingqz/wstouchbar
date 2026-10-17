use super::types::{DisplayInfo, Outgoing};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    num::NonZeroU32,
};
use touchbar_model::{SystemKey, Timestamp, WatchKey};
use touchbar_protocol::DamageRect;
pub const QUEUE_LIMIT: usize = 256;
pub const MAX_CONTACTS: usize = 32;
pub const MAX_CONNECTIONS: usize = 17;
#[derive(Clone, Debug)]
pub(super) struct Contact {
    pub id: NonZeroU32,
    pub x: i32,
    pub y: i32,
}
#[derive(Clone, Debug)]
pub(super) struct Client {
    pub uid: u32,
    pub session: Option<String>,
    pub queue: VecDeque<Outgoing>,
    pub closing: bool,
    pub close_at: Option<Timestamp>,
    pub display: bool,
    pub input_enabled: bool,
    pub enable_after: Option<usize>,
    pub contacts: BTreeMap<u32, Contact>,
    pub next_contact: u64,
    pub pressed: BTreeSet<SystemKey>,
    pub watched: BTreeSet<WatchKey>,
    pub damage: Vec<DamageRect>,
    pub frames: BTreeMap<u32, u64>,
    pub pending_frames: VecDeque<(u32, u64)>,
    pub last_frame: Option<u64>,
    pub info: Option<DisplayInfo>,
}
impl Client {
    pub fn new(uid: u32) -> Self {
        Self {
            uid,
            session: None,
            queue: VecDeque::new(),
            closing: false,
            close_at: None,
            display: false,
            input_enabled: false,
            enable_after: None,
            contacts: BTreeMap::new(),
            next_contact: 1,
            pressed: BTreeSet::new(),
            watched: BTreeSet::new(),
            damage: Vec::new(),
            frames: BTreeMap::new(),
            pending_frames: VecDeque::new(),
            last_frame: None,
            info: None,
        }
    }
}
