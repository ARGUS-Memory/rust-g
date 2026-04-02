//! Drop-in replacement for `ordered_float::OrderedFloat<f32>`.
//! Provides Hash/Eq/Ord for f32 via `total_cmp`, with transparent serde.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct OrderedFloat(pub f32);

impl OrderedFloat {
    #[inline]
    pub fn into_inner(self) -> f32 {
        self.0
    }
}

impl From<f32> for OrderedFloat {
    #[inline]
    fn from(v: f32) -> Self {
        Self(v)
    }
}

impl PartialEq for OrderedFloat {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for OrderedFloat {}

impl std::hash::Hash for OrderedFloat {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl Ord for OrderedFloat {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for OrderedFloat {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
