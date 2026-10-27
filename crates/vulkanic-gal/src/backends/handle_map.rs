//! Backend objects keyed by GAL handle.
//!
//! GAL handles carry a dense per-kind slot index and a generation (see
//! `handles.rs`), so backends store objects in a vector indexed by slot and
//! check the generation on access. Lookups are an index and a compare
//! instead of a hash or tree walk; this sits on every draw's path.
//!
//! The GAL may reuse a slot while the backend still holds the previous
//! generation (its destroy waits for a submission to retire), so a slot
//! holds a short list of generations; it is almost always one.

use smallvec::SmallVec;

use crate::handles::Handle;

type Entries<T> = SmallVec<[(Handle, T); 1]>;

pub(super) struct HandleMap<T> {
    slots: Vec<Entries<T>>,
    len: usize,
}

impl<T> Default for HandleMap<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            len: 0,
        }
    }
}

impl<T> HandleMap<T> {
    /// Stores `value` for `handle`, returning the value it replaces.
    pub(super) fn insert(&mut self, handle: Handle, value: T) -> Option<T> {
        let index = handle.index() as usize;
        if self.slots.len() <= index {
            self.slots.resize_with(index + 1, SmallVec::new);
        }
        let entries = &mut self.slots[index];
        if let Some((_, existing)) = entries.iter_mut().find(|(stored, _)| *stored == handle) {
            return Some(std::mem::replace(existing, value));
        }
        entries.push((handle, value));
        self.len += 1;
        None
    }

    pub(super) fn get(&self, handle: Handle) -> Option<&T> {
        self.slots
            .get(handle.index() as usize)?
            .iter()
            .find(|(stored, _)| *stored == handle)
            .map(|(_, value)| value)
    }

    pub(super) fn get_mut(&mut self, handle: Handle) -> Option<&mut T> {
        self.slots
            .get_mut(handle.index() as usize)?
            .iter_mut()
            .find(|(stored, _)| *stored == handle)
            .map(|(_, value)| value)
    }

    pub(super) fn remove(&mut self, handle: Handle) -> Option<T> {
        let entries = self.slots.get_mut(handle.index() as usize)?;
        let position = entries.iter().position(|(stored, _)| *stored == handle)?;
        self.len -= 1;
        Some(entries.remove(position).1)
    }

    // Only the Vulkan backend tears objects down in bulk.
    #[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
    pub(super) fn len(&self) -> usize {
        self.len
    }

    // Only the Vulkan backend tears objects down in bulk.
    #[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }

    // Only the Vulkan backend tears objects down in bulk.
    #[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
    /// Removes every entry, last slot first.
    pub(super) fn drain_reverse(&mut self) -> impl Iterator<Item = (Handle, T)> + '_ {
        self.len = 0;
        self.slots
            .drain(..)
            .rev()
            .flat_map(|entries| entries.into_iter().rev())
    }
}

/// Objects of every handle kind, one `HandleMap` per kind.
#[cfg(feature = "vulkan")]
pub(super) struct KindedHandleMap<T> {
    kinds: Vec<HandleMap<T>>,
}

#[cfg(feature = "vulkan")]
impl<T> Default for KindedHandleMap<T> {
    fn default() -> Self {
        Self { kinds: Vec::new() }
    }
}

#[cfg(feature = "vulkan")]
fn kind_index(handle: Handle) -> usize {
    handle.kind().map_or(0, |kind| kind as usize)
}

#[cfg(feature = "vulkan")]
impl<T> KindedHandleMap<T> {
    pub(super) fn insert(&mut self, handle: Handle, value: T) -> Option<T> {
        let kind = kind_index(handle);
        if self.kinds.len() <= kind {
            self.kinds.resize_with(kind + 1, HandleMap::default);
        }
        self.kinds[kind].insert(handle, value)
    }

    pub(super) fn get(&self, handle: Handle) -> Option<&T> {
        self.kinds.get(kind_index(handle))?.get(handle)
    }

    pub(super) fn get_mut(&mut self, handle: Handle) -> Option<&mut T> {
        self.kinds.get_mut(kind_index(handle))?.get_mut(handle)
    }

    pub(super) fn remove(&mut self, handle: Handle) -> Option<T> {
        self.kinds.get_mut(kind_index(handle))?.remove(handle)
    }

    pub(super) fn len(&self) -> usize {
        self.kinds.iter().map(HandleMap::len).sum()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.kinds.iter().all(HandleMap::is_empty)
    }

    /// Removes every entry for teardown: highest kind first, since objects
    /// depend only on kinds numbered below their own (sets on views, views
    /// on textures, pipelines on layouts).
    pub(super) fn drain_teardown(&mut self) -> impl Iterator<Item = (Handle, T)> + '_ {
        self.kinds
            .iter_mut()
            .rev()
            .flat_map(HandleMap::drain_reverse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handles::HandleKind;

    fn handle(index: u32, generation: u32) -> Handle {
        Handle::new(HandleKind::Buffer, index, generation).expect("valid handle")
    }

    #[test]
    fn stale_generations_never_alias_a_reused_slot() {
        let mut map = HandleMap::default();
        let old = handle(3, 1);
        let new = handle(3, 2);
        assert!(map.insert(old, "old").is_none());
        assert_eq!(map.get(old), Some(&"old"));
        assert_eq!(map.remove(old), Some("old"));
        assert!(map.insert(new, "new").is_none());
        assert_eq!(map.get(old), None);
        assert_eq!(map.remove(old), None);
        assert_eq!(map.get(new), Some(&"new"));
        assert_eq!(map.len(), 1);
        *map.get_mut(new).unwrap() = "changed";
        assert_eq!(map.get(new), Some(&"changed"));
    }

    #[test]
    fn a_reused_slot_keeps_the_generation_awaiting_destroy() {
        let mut map = HandleMap::default();
        let pending = handle(0, 1);
        let reused = handle(0, 2);
        map.insert(pending, "pending");
        map.insert(reused, "reused");
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(pending), Some(&"pending"));
        assert_eq!(map.remove(pending), Some("pending"));
        assert_eq!(map.get(reused), Some(&"reused"));
        assert_eq!(map.len(), 1);
    }

    #[cfg(feature = "vulkan")]
    #[test]
    fn kinded_maps_keep_kinds_apart_and_tear_down_dependents_first() {
        let mut map = KindedHandleMap::default();
        let buffer = handle(0, 1);
        let set = Handle::new(HandleKind::ResourceSet, 0, 1).unwrap();
        let view = Handle::new(HandleKind::TextureView, 0, 1).unwrap();
        map.insert(buffer, "buffer");
        map.insert(set, "set");
        map.insert(view, "view");
        assert_eq!(map.get(set), Some(&"set"));
        assert_eq!(map.len(), 3);
        let order: Vec<_> = map.drain_teardown().map(|(_, value)| value).collect();
        assert_eq!(order, ["set", "view", "buffer"]);
        assert!(map.is_empty());
    }

    #[test]
    fn drain_reverse_empties_last_slot_first() {
        let mut map = HandleMap::default();
        for index in [0, 5, 2] {
            map.insert(handle(index, 1), index);
        }
        let order: Vec<u32> = map.drain_reverse().map(|(_, value)| value).collect();
        assert_eq!(order, [5, 2, 0]);
        assert!(map.is_empty());
        assert_eq!(map.get(handle(5, 1)), None);
    }
}
