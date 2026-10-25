use crate::{Track, Value};
use std::sync::Arc;

/// Sampled property values keyed by caller-defined names. Properties are kept
/// in sorted order in a compact vector, making full-clip sampling allocation
/// free after the shared property names have been interned.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pose {
    values: Vec<(Arc<str>, Value)>,
}

impl Pose {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, property: impl Into<String>, value: Value) {
        self.insert_shared(Arc::from(property.into()), value);
    }

    pub(crate) fn insert_shared(&mut self, property: Arc<str>, value: Value) {
        if let Some((last, _)) = self.values.last()
            && last.as_ref() > property.as_ref()
        {
            match self
                .values
                .binary_search_by(|(name, _)| name.as_ref().cmp(property.as_ref()))
            {
                Ok(index) => self.values[index].1 = value,
                Err(index) => self.values.insert(index, (property, value)),
            }
            return;
        }
        if let Some((last, current)) = self.values.last_mut()
            && last.as_ref() == property.as_ref()
        {
            *current = value;
            return;
        }
        self.values.push((property, value));
    }

    pub(crate) fn fill_from_tracks(&mut self, tracks: &[Track], time: f32) -> bool {
        if self.values.len() == tracks.len()
            && self
                .values
                .iter()
                .zip(tracks)
                .all(|((name, _), track)| name.as_ref() == track.property())
        {
            for ((_, value), track) in self.values.iter_mut().zip(tracks) {
                *value = track.sample(time);
            }
            true
        } else {
            false
        }
    }

    pub fn remove(&mut self, property: &str) -> Option<Value> {
        let index = self
            .values
            .binary_search_by(|(name, _)| name.as_ref().cmp(property))
            .ok()?;
        Some(self.values.remove(index).1)
    }

    pub fn get(&self, property: &str) -> Option<Value> {
        self.values
            .binary_search_by(|(name, _)| name.as_ref().cmp(property))
            .ok()
            .map(|index| self.values[index].1)
    }

    pub fn f32(&self, property: &str) -> Option<f32> {
        self.get(property)?.as_f32()
    }

    pub fn vec3(&self, property: &str) -> Option<[f32; 3]> {
        self.get(property)?.as_vec3()
    }

    pub fn quat(&self, property: &str) -> Option<[f32; 4]> {
        self.get(property)?.as_quat()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, Value)> {
        self.values
            .iter()
            .map(|(name, value)| (name.as_ref(), *value))
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Blends this pose toward `other`. Properties present in only one side are
    /// preserved so layered clips can contribute disjoint channels.
    pub fn blend(&self, other: &Self, weight: f32) -> Self {
        let weight = if weight.is_finite() {
            weight.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut values = Vec::with_capacity(self.values.len() + other.values.len());
        let mut left = 0usize;
        let mut right = 0usize;
        while left < self.values.len() || right < other.values.len() {
            match (self.values.get(left), other.values.get(right)) {
                (Some((left_name, left_value)), Some((right_name, right_value))) => {
                    match left_name.as_ref().cmp(right_name.as_ref()) {
                        std::cmp::Ordering::Less => {
                            values.push((left_name.clone(), *left_value));
                            left += 1;
                        }
                        std::cmp::Ordering::Greater => {
                            values.push((right_name.clone(), *right_value));
                            right += 1;
                        }
                        std::cmp::Ordering::Equal => {
                            values.push((
                                left_name.clone(),
                                left_value
                                    .lerp(*right_value, weight)
                                    .unwrap_or(*right_value),
                            ));
                            left += 1;
                            right += 1;
                        }
                    }
                }
                (Some((name, value)), None) => {
                    values.push((name.clone(), *value));
                    left += 1;
                }
                (None, Some((name, value))) => {
                    values.push((name.clone(), *value));
                    right += 1;
                }
                (None, None) => break,
            }
        }
        Self { values }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Pose {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.values.len()))?;
        for (name, value) in &self.values {
            map.serialize_entry(name.as_ref(), value)?;
        }
        map.end()
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Pose {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let values = std::collections::BTreeMap::<String, Value>::deserialize(deserializer)?;
        Ok(Self {
            values: values
                .into_iter()
                .map(|(name, value)| (Arc::from(name), value))
                .collect(),
        })
    }
}
